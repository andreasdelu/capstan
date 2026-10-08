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
            Event::Generated(action) => format!(
                "GENERATED · {} (not OS delivery proof)",
                action_label(action)
            ),
            Event::GenerationFailed(action) => {
                format!("GENERATION FAILED · {}", action_label(action))
            }
        }
    }
}

fn action_label(action: &Action) -> String {
    match action {
        Action::Tap(key) => format!("{} tap", key.label()),
        Action::ModifierDown(modifier) => format!("{} down", modifier.label()),
        Action::ModifierUp(modifier) => format!("{} up", modifier.label()),
    }
}

#[derive(Default)]
pub struct Diagnostics {
    pub recording: bool,
    pub rows: VecDeque<Row>,
}

impl Diagnostics {
    pub fn record(&mut self, timestamp: Duration, event: Event) {
        if !self.recording {
            return;
        }
        if self.rows.len() == CAPACITY {
            self.rows.pop_front();
        }
        self.rows.push_back(Row { timestamp, event });
    }
    pub fn clear(&mut self) {
        self.rows.clear();
    }
    pub fn stop(&mut self) {
        self.recording = false;
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
