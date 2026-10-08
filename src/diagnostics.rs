//! Opt-in Caps-only diagnostics. No text, other keycodes, disk I/O, or export.
use std::collections::VecDeque;
use std::time::Duration;

use super::{Action, HoldModifier, TapKey};

pub const CAPACITY: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    PhysicalDown {
        timeout_ms: u64,
        tap: TapKey,
        modifier: HoldModifier,
    },
    PhysicalUp {
        elapsed_ms: u128,
        timeout_ms: u128,
        decision: Decision,
    },
    Chord,
    CapsSuppressed,
    Cancelled,
    Generated(Action),
    GenerationFailed(Action),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Tap,
    Chord,
    Hold,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub timestamp: Duration,
    pub event: Event,
    // Recording boundaries prevent a paused/clipped sequence from being paired.
    pub segment: u64,
}

impl Row {
    pub fn label(&self) -> String {
        match &self.event {
            Event::PhysicalDown {
                timeout_ms,
                tap,
                modifier,
            } => format!(
                "Caps down · tap {} / held {} · timeout {timeout_ms} ms",
                tap.label(),
                modifier.label()
            ),
            Event::PhysicalUp {
                elapsed_ms,
                timeout_ms,
                decision,
            } => format!(
                "Caps up · {elapsed_ms} ms · {} · timeout {timeout_ms} ms",
                match decision {
                    Decision::Tap => "tap",
                    Decision::Chord => "chord",
                    Decision::Hold => "long hold",
                }
            ),
            Event::Chord => "Chord (other input omitted)".into(),
            Event::CapsSuppressed => "Processed Caps transition suppressed".into(),
            Event::Cancelled => "Gesture cancelled · modifier release attempted".into(),
            Event::Generated(action) => format!("GENERATED · {}", action_label(action)),
            Event::GenerationFailed(action) => {
                format!("GENERATION FAILED · {}", action_label(action))
            }
        }
    }
}

pub fn action_label(action: &Action) -> String {
    match action {
        Action::Tap(key) => format!("{} tap", key.label()),
        Action::ModifierDown(modifier) => format!("{} down", modifier.label()),
        Action::ModifierUp(modifier) => format!("{} up", modifier.label()),
    }
}

/// A sequence projection, never a timestamp join: HID and callback clocks have
/// different reference points. Only typed down/up boundaries establish pairing.
#[derive(Debug)]
pub struct Gesture {
    pub mapping: Option<(TapKey, HoldModifier, u64)>,
    pub elapsed_ms: Option<u128>,
    pub result: &'static str,
    pub output: Vec<String>,
    pub ended: bool,
    pub incomplete: bool,
}

impl Gesture {
    fn partial() -> Self {
        Self {
            mapping: None,
            elapsed_ms: None,
            result: "Partial",
            output: vec![],
            ended: false,
            incomplete: true,
        }
    }
    pub fn label(&self) -> String {
        let duration = self
            .elapsed_ms
            .map(|ms| format!("{ms} ms"))
            .unwrap_or_else(|| "duration unavailable".into());
        let mapping = self
            .mapping
            .map(|(tap, held, timeout)| {
                format!(
                    "tap {} / held {} · cutoff {timeout} ms",
                    tap.label(),
                    held.label()
                )
            })
            .unwrap_or_else(|| "mapping unavailable".into());
        let output = if self.incomplete || self.mapping.is_none() {
            "details unavailable (partial capture)".into()
        } else if self.output.is_empty() {
            "output details unavailable".into()
        } else {
            self.output.join(", ")
        };
        format!("{} · {duration} · {mapping} · {output}", self.result)
    }
}

pub fn gestures(rows: &[Row]) -> Vec<Gesture> {
    let mut gestures = vec![];
    let mut current: Option<Gesture> = None;
    let mut segment = None;
    for row in rows {
        if segment != Some(row.segment) {
            if let Some(mut previous) = current.take() {
                if !previous.ended {
                    previous.incomplete = true;
                }
                gestures.push(previous);
            }
            segment = Some(row.segment);
        }
        match &row.event {
            Event::PhysicalDown {
                timeout_ms,
                tap,
                modifier,
            } => {
                if let Some(mut previous) = current.take() {
                    if !previous.ended {
                        previous.incomplete = true;
                    }
                    gestures.push(previous);
                }
                current = Some(Gesture {
                    mapping: Some((*tap, *modifier, *timeout_ms)),
                    elapsed_ms: None,
                    result: "Held",
                    output: vec![],
                    ended: false,
                    incomplete: false,
                });
            }
            Event::PhysicalUp {
                elapsed_ms,
                decision,
                ..
            } => {
                if current.as_ref().is_some_and(|g| g.ended) {
                    gestures.push(current.take().unwrap());
                }
                let gesture = current.get_or_insert_with(Gesture::partial);
                gesture.elapsed_ms = Some(*elapsed_ms);
                gesture.result = match decision {
                    Decision::Tap => "Tap",
                    Decision::Hold => "Hold",
                    Decision::Chord => "Chord",
                };
                gesture.ended = true;
            }
            Event::Cancelled => {
                if current.as_ref().is_some_and(|g| g.ended) {
                    gestures.push(current.take().unwrap());
                }
                let gesture = current.get_or_insert_with(Gesture::partial);
                gesture.result = "Cancelled";
                gesture.ended = true;
            }
            Event::Chord => {
                if current.as_ref().is_some_and(|g| g.ended) {
                    gestures.push(current.take().unwrap());
                }
                current.get_or_insert_with(Gesture::partial).result = "Chord";
            }
            Event::Generated(action) | Event::GenerationFailed(action) => {
                let gesture = current.get_or_insert_with(Gesture::partial);
                // Don't attribute an evicted or mid-recording output to a guessed press.
                if gesture.mapping.is_some() {
                    gesture.output.push(format!(
                        "{}{}",
                        action_label(action),
                        if matches!(row.event, Event::GenerationFailed(_)) {
                            " failed"
                        } else {
                            ""
                        }
                    ));
                }
            }
            Event::CapsSuppressed => {} // Raw evidence only, not a gesture boundary.
        }
    }
    if let Some(mut gesture) = current {
        if !gesture.ended {
            gesture.incomplete = true;
        }
        gestures.push(gesture);
    }
    gestures
}

#[derive(Default)]
pub struct Diagnostics {
    pub recording: bool,
    pub rows: VecDeque<Row>,
    segment: u64,
}

impl Diagnostics {
    pub fn set_recording(&mut self, recording: bool) {
        if self.recording != recording {
            self.segment = self.segment.wrapping_add(1);
        }
        self.recording = recording;
    }

    pub fn record(&mut self, timestamp: Duration, event: Event) {
        if !self.recording {
            return;
        }
        if self.rows.len() == CAPACITY {
            self.rows.pop_front();
        }
        self.rows.push_back(Row {
            timestamp,
            event,
            segment: self.segment,
        });
    }
    pub fn clear(&mut self) {
        self.segment = self.segment.wrapping_add(1);
        self.rows.clear();
    }
    pub fn stop(&mut self) {
        self.set_recording(false);
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn down(tap: TapKey, held: HoldModifier, cutoff: u64) -> Event {
        Event::PhysicalDown {
            timeout_ms: cutoff,
            tap,
            modifier: held,
        }
    }
    fn up(duration: u128, cutoff: u128, decision: Decision) -> Event {
        Event::PhysicalUp {
            elapsed_ms: duration,
            timeout_ms: cutoff,
            decision,
        }
    }
    fn add(d: &mut Diagnostics, event: Event) {
        // Deliberately alternate clock reference points, preserving insertion order.
        let time = if matches!(event, Event::PhysicalDown { .. } | Event::PhysicalUp { .. }) {
            46610
        } else {
            1118
        };
        d.record(Duration::from_secs(time), event);
    }
    #[test]
    fn projection_pairs_sequence_not_clocks_and_keeps_snapshots_and_failures() {
        let mut d = Diagnostics::default();
        d.set_recording(true);
        for (decision, duration) in [
            (Decision::Tap, 100),
            (Decision::Chord, 80),
            (Decision::Hold, 900),
        ] {
            add(&mut d, down(TapKey::Tab, HoldModifier::Shift, 300));
            add(
                &mut d,
                Event::Generated(Action::ModifierDown(HoldModifier::Shift)),
            );
            if decision == Decision::Chord {
                add(&mut d, Event::Chord);
            }
            add(&mut d, Event::CapsSuppressed);
            add(&mut d, up(duration, 300, decision));
            add(
                &mut d,
                Event::Generated(Action::ModifierUp(HoldModifier::Shift)),
            );
            if decision == Decision::Tap {
                add(&mut d, Event::GenerationFailed(Action::Tap(TapKey::Tab)));
            }
        }
        add(&mut d, down(TapKey::Escape, HoldModifier::Control, 500));
        add(&mut d, Event::Cancelled);
        add(
            &mut d,
            Event::Generated(Action::ModifierUp(HoldModifier::Control)),
        );
        let g = gestures(&d.rows.iter().cloned().collect::<Vec<_>>());
        assert_eq!(
            g.iter().map(|g| g.result).collect::<Vec<_>>(),
            ["Tap", "Chord", "Hold", "Cancelled"]
        );
        assert_eq!(g[0].elapsed_ms, Some(100));
        assert!(g[0].label().contains("Tab tap failed"));
        assert!(g[0].label().contains("cutoff 300 ms"));
        assert!(g[3].label().contains("cutoff 500 ms"));
        assert!(!g[3].label().contains("GENERATED"));
    }
    #[test]
    fn actual_state_snapshot_repeated_presses_and_recording_mid_hold() {
        let mut state = super::super::State::default();
        state.diagnostics.set_recording(true);
        let start = Duration::from_secs(10);
        state.press(start, Duration::from_millis(300));
        assert!(state.press(start, Duration::from_millis(300)).is_none());
        let changed = super::super::config::Config {
            tap_key: TapKey::Tab,
            hold_modifier: HoldModifier::Shift,
            escape_timeout_ms: 500,
            remapping_enabled: true,
            ..Default::default()
        };
        state.configure(&changed);
        state.release(start + Duration::from_millis(100));
        let snapshot = |state: &super::super::State| {
            gestures(&state.diagnostics.rows.iter().cloned().collect::<Vec<_>>())
        };
        assert_eq!(snapshot(&state).len(), 1);
        assert_eq!(
            snapshot(&state)[0].mapping,
            Some((TapKey::Escape, HoldModifier::Control, 300))
        );
        state.diagnostics.clear();
        state.diagnostics.set_recording(false);
        state.press(start, Duration::from_millis(500));
        state.diagnostics.set_recording(true);
        state.release(start + Duration::from_millis(80));
        assert!(snapshot(&state)[0].mapping.is_none());
        for i in 0..120 {
            let start = Duration::from_secs(20 + i);
            state.press(start, Duration::from_millis(500));
            state.release(start + Duration::from_millis(50));
        }
        assert_eq!(state.diagnostics.rows.len(), CAPACITY);
        assert_eq!(snapshot(&state).len(), CAPACITY / 2);
        assert!(
            snapshot(&state)
                .iter()
                .all(|g| g.result == "Tap" && !g.incomplete)
        );
    }
    #[test]
    fn partial_capture_pause_clear_and_eviction_never_invent_pairing() {
        let mut d = Diagnostics::default();
        d.set_recording(true);
        add(&mut d, Event::Generated(Action::ControlDown));
        add(&mut d, up(90, 300, Decision::Tap));
        let snapshot = |d: &Diagnostics| gestures(&d.rows.iter().cloned().collect::<Vec<_>>());
        assert!(snapshot(&d)[0].label().contains("details unavailable"));
        assert!(snapshot(&d)[0].output.is_empty());
        d.clear();
        add(&mut d, down(TapKey::Escape, HoldModifier::Control, 300));
        d.set_recording(false);
        d.set_recording(true);
        add(&mut d, up(100, 300, Decision::Tap));
        assert_eq!(snapshot(&d).len(), 2);
        assert!(
            snapshot(&d)
                .iter()
                .all(|g| g.label().contains("details unavailable"))
        );
        d.clear();
        add(&mut d, down(TapKey::Escape, HoldModifier::Control, 300));
        for _ in 0..CAPACITY {
            add(&mut d, Event::CapsSuppressed);
        }
        add(&mut d, up(100, 300, Decision::Tap));
        assert_eq!(snapshot(&d).len(), 1);
        assert!(snapshot(&d)[0].label().contains("mapping unavailable"));
        d.clear();
        add(&mut d, down(TapKey::Escape, HoldModifier::Control, 300));
        add(&mut d, down(TapKey::Tab, HoldModifier::Shift, 500));
        add(&mut d, up(100, 500, Decision::Tap));
        assert_eq!(snapshot(&d).len(), 2);
        assert!(snapshot(&d)[0].incomplete);
        assert!(!snapshot(&d)[1].incomplete);
    }
    #[test]
    fn opt_in_pause_clear_stop_and_bounded_fifo() {
        let mut d = Diagnostics::default();
        d.record(Duration::ZERO, Event::Chord);
        assert!(d.rows.is_empty());
        d.recording = true;
        for i in 0..250 {
            d.record(Duration::from_millis(i), Event::Chord);
        }
        assert_eq!(d.rows.len(), CAPACITY);
        assert_eq!(d.rows.front().unwrap().timestamp, Duration::from_millis(50));
        d.recording = false;
        d.record(Duration::ZERO, Event::CapsSuppressed);
        assert_eq!(d.rows.len(), CAPACITY);
        d.clear();
        assert!(!d.recording);
        d.recording = true;
        d.record(Duration::ZERO, Event::Chord);
        d.stop();
        assert!(!d.recording && d.rows.is_empty());
    }
}
