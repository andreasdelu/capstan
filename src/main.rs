#[cfg(not(target_os = "macos"))]
compile_error!("caps-tap requires macOS");

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

mod config;
mod diagnostics;
mod viewer;
use diagnostics::{Decision, Diagnostics, Event};
mod mac;
mod settings;
use std::time::Duration;

const CAPS: i64 = 57;
use config::{HoldModifier, TapKey};
const LEFT_MOUSE_DOWN: u32 = 1;
const RIGHT_MOUSE_DOWN: u32 = 3;
const OTHER_MOUSE_DOWN: u32 = 25;
const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const FLAGS_CHANGED: u32 = 12;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xffff_fffe;
const TAP_DISABLED_BY_USER_INPUT: u32 = 0xffff_ffff;
const KEYCODE_FIELD: u32 = 9;
const SOURCE_USER_DATA_FIELD: u32 = 42;
const CAPS_LOCK_FLAG: u64 = 1 << 16;
#[cfg(test)]
const CONTROL_FLAG: u64 = 1 << 18;
const SYNTHETIC_TAG: i64 = 0x0043_4150_5354_4150;
#[cfg(test)]
const ALONE_TIMEOUT: Duration = Duration::from_millis(300);
const KEYBOARD_USAGE_PAGE: u32 = 0x07;
const CAPS_USAGE: u32 = 0x39;
const GENERIC_DESKTOP_USAGE_PAGE: i32 = 0x01;
const KEYBOARD_DEVICE_USAGE: i32 = 0x06;
const UTF8_ENCODING: u32 = 0x0800_0100;

type Ref = *mut c_void;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        location: u32,
        placement: u32,
        options: u32,
        mask: u64,
        callback: unsafe extern "C" fn(Ref, u32, Ref, Ref) -> Ref,
        user_info: Ref,
    ) -> Ref;
    fn CGEventTapEnable(tap: Ref, enable: bool);
    fn CGEventGetIntegerValueField(event: Ref, field: u32) -> i64;
    fn CGEventSetIntegerValueField(event: Ref, field: u32, value: i64);
    fn CGEventGetFlags(event: Ref) -> u64;
    fn CGEventGetTimestamp(event: Ref) -> u64;
    fn CGEventSetFlags(event: Ref, flags: u64);
    fn CGEventCreateKeyboardEvent(source: Ref, keycode: u16, down: bool) -> Ref;
    fn CGEventSetType(event: Ref, event_type: u32);
    fn CGEventPost(location: u32, event: Ref);
    fn CGEventTapPostEvent(proxy: Ref, event: Ref);
    fn CFRelease(value: Ref);
    fn CFStringCreateWithCString(allocator: Ref, text: *const i8, encoding: u32) -> Ref;
    fn CFNumberCreate(allocator: Ref, kind: isize, value: *const c_void) -> Ref;
    fn CFDictionaryCreate(
        allocator: Ref,
        keys: *const Ref,
        values: *const Ref,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> Ref;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
    fn CFMachPortCreateRunLoopSource(allocator: Ref, port: Ref, order: isize) -> Ref;
    fn CFRunLoopGetCurrent() -> Ref;
    fn CFRunLoopAddSource(run_loop: Ref, source: Ref, mode: Ref);
    static kCFRunLoopCommonModes: Ref;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDManagerCreate(allocator: Ref, options: u32) -> Ref;
    fn IOHIDManagerSetDeviceMatching(manager: Ref, matching: Ref);
    fn IOHIDManagerRegisterInputValueCallback(
        manager: Ref,
        callback: unsafe extern "C" fn(Ref, i32, Ref, Ref),
        context: Ref,
    );
    fn IOHIDManagerScheduleWithRunLoop(manager: Ref, run_loop: Ref, mode: Ref);
    fn IOHIDManagerOpen(manager: Ref, options: u32) -> i32;
    fn IOHIDValueGetElement(value: Ref) -> Ref;
    fn IOHIDValueGetIntegerValue(value: Ref) -> isize;
    fn IOHIDValueGetTimeStamp(value: Ref) -> u64;
    fn IOHIDElementGetUsagePage(element: Ref) -> u32;
    fn IOHIDElementGetUsage(element: Ref) -> u32;
}

#[derive(Default)]
struct Context {
    state: Mutex<State>,
    tap: AtomicPtr<c_void>,
    enabled: AtomicBool,
    caps_lock: OnceLock<mac::CapsLock>,
    escape_timeout_ms: AtomicU64,
}

#[derive(Default)]
struct State {
    pressed_at: Option<Duration>,
    chorded: bool,
    timeout: Duration,
    config: config::Config,
    tap_key: TapKey,
    hold_modifier: HoldModifier,
    diagnostics: Diagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Tap(TapKey),
    ModifierDown(HoldModifier),
    ModifierUp(HoldModifier),
}

#[cfg(test)]
impl Action {
    #[allow(non_upper_case_globals)]
    const ControlDown: Self = Self::ModifierDown(HoldModifier::Control);
    #[allow(non_upper_case_globals)]
    const ControlUp: Self = Self::ModifierUp(HoldModifier::Control);
    #[allow(non_upper_case_globals)]
    const Escape: Self = Self::Tap(TapKey::Escape);
}

impl State {
    fn configure(&mut self, config: &config::Config) -> Option<Action> {
        let action = if !config.remapping_enabled {
            self.cancel()
        } else {
            None
        };
        self.config = config.clone();
        action
    }

    fn press(&mut self, now: Duration, timeout: Duration) -> Option<Action> {
        if self.pressed_at.is_some() {
            return None;
        }
        self.pressed_at = Some(now);
        self.timeout = timeout;
        self.chorded = false;
        self.tap_key = self.config.tap_key;
        self.hold_modifier = self.config.hold_modifier;
        self.diagnostics.record(
            now,
            Event::PhysicalDown {
                timeout_ms: timeout.as_millis() as u64,
                tap: self.tap_key,
                modifier: self.hold_modifier,
            },
        );
        Some(Action::ModifierDown(self.hold_modifier))
    }

    fn chord(&mut self) {
        if self.pressed_at.is_some() {
            self.chorded = true;
        }
    }

    fn release(&mut self, now: Duration) -> Vec<Action> {
        let Some(started) = self.pressed_at.take() else {
            return vec![];
        };
        let duration = now.saturating_sub(started);
        let timeout = self.timeout;
        let alone = !self.chorded && duration < timeout;
        self.diagnostics.record(
            now,
            Event::PhysicalUp {
                elapsed_ms: duration.as_millis(),
                timeout_ms: timeout.as_millis(),
                decision: if self.chorded {
                    Decision::Chord
                } else if alone {
                    Decision::Tap
                } else {
                    Decision::Hold
                },
            },
        );
        self.chorded = false;
        let mut actions = vec![Action::ModifierUp(self.hold_modifier)];
        if alone {
            actions.push(Action::Tap(self.tap_key));
        }
        actions
    }

    fn cancel(&mut self) -> Option<Action> {
        self.pressed_at
            .take()
            .map(|_| Action::ModifierUp(self.hold_modifier))
    }
}

// HID events tell us when the physical key is released. CGEventTap does not.
unsafe extern "C" fn hid_callback(context: Ref, result: i32, _sender: Ref, value: Ref) {
    if result != 0 || value.is_null() {
        return;
    }
    let element = unsafe { IOHIDValueGetElement(value) };
    if unsafe { IOHIDElementGetUsagePage(element) } != KEYBOARD_USAGE_PAGE
        || unsafe { IOHIDElementGetUsage(element) } != CAPS_USAGE
    {
        return;
    }

    let context = unsafe { &*(context as *const Context) };
    let down = unsafe { IOHIDValueGetIntegerValue(value) } != 0;
    let timestamp = mac::hid_timestamp(unsafe { IOHIDValueGetTimeStamp(value) });
    let mut state = context.state.lock().unwrap_or_else(|e| e.into_inner());
    if !context.enabled.load(Ordering::Acquire) || !state.config.remapping_enabled {
        return;
    }
    if down {
        let timeout = Duration::from_millis(state.config.escape_timeout_ms);
        if let Some(action) = state.press(timestamp, timeout) {
            unsafe {
                emit_recorded(&mut state, timestamp, action, None);
            }
        }
    } else {
        context.clear_caps_lock();
        for action in state.release(timestamp) {
            unsafe {
                emit_recorded(&mut state, timestamp, action, None);
            }
        }
    }
}

// Synthetic HID events pass through our event tap again; tag and ignore them.
unsafe fn emit_recorded(
    state: &mut State,
    timestamp: Duration,
    action: Action,
    proxy: Option<Ref>,
) {
    let success = unsafe { emit(action, proxy) };
    state.diagnostics.record(
        timestamp,
        if success {
            Event::Generated(action)
        } else {
            Event::GenerationFailed(action)
        },
    );
}

unsafe fn emit(action: Action, proxy: Option<Ref>) -> bool {
    let mut success = true;
    let tap = matches!(action, Action::Tap(_));
    let (key, flags, kind) = match action {
        Action::Tap(key) => (key.keycode(), 0, KEY_DOWN),
        Action::ModifierDown(modifier) => (modifier.keycode(), modifier.flag(), FLAGS_CHANGED),
        Action::ModifierUp(modifier) => (modifier.keycode(), 0, FLAGS_CHANGED),
    };
    for down in if tap { &[true, false][..] } else { &[true][..] } {
        let event = unsafe { CGEventCreateKeyboardEvent(ptr::null_mut(), key, *down) };
        if event.is_null() {
            success = false;
            continue;
        }
        unsafe {
            CGEventSetType(event, if tap && !down { KEY_UP } else { kind });
            CGEventSetFlags(event, flags);
            CGEventSetIntegerValueField(event, SOURCE_USER_DATA_FIELD, SYNTHETIC_TAG);
            if let Some(proxy) = proxy {
                CGEventTapPostEvent(proxy, event);
            } else {
                CGEventPost(0, event);
            }
            CFRelease(event);
        }
    }
    success
}

unsafe extern "C" fn callback(proxy: Ref, kind: u32, event: Ref, context: Ref) -> Ref {
    let context = unsafe { &*(context as *const Context) };
    if kind == TAP_DISABLED_BY_TIMEOUT || kind == TAP_DISABLED_BY_USER_INPUT {
        eprintln!("event tap disabled ({kind}); restarting it (check for missed keys)");
        let mut state = context.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(action) = state.cancel() {
            let timestamp = mac::monotonic_timestamp();
            state.diagnostics.record(timestamp, Event::Cancelled);
            unsafe {
                emit_recorded(&mut state, timestamp, action, None);
            }
        }
        unsafe {
            CGEventTapEnable(context.tap.load(Ordering::Relaxed), true);
        }
        return event;
    }
    if unsafe { CGEventGetIntegerValueField(event, SOURCE_USER_DATA_FIELD) } == SYNTHETIC_TAG {
        return event;
    }
    if !context.enabled.load(Ordering::Acquire) {
        return event;
    }
    let key = unsafe { CGEventGetIntegerValueField(event, KEYCODE_FIELD) };
    // macOS reports Caps state transitions, not physical key-up. HID callback owns the state.
    if kind == FLAGS_CHANGED && key == CAPS {
        context
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .diagnostics
            .record(
                Duration::from_nanos(unsafe { CGEventGetTimestamp(event) }),
                Event::CapsSuppressed,
            );
        context.clear_caps_lock();
        return ptr::null_mut();
    }
    // Filtering Caps events alone does not remove the native lock bit from later input.
    unsafe {
        CGEventSetFlags(event, remapped_flags(CGEventGetFlags(event), None));
    }
    if matches!(
        kind,
        KEY_DOWN | KEY_UP | FLAGS_CHANGED | LEFT_MOUSE_DOWN | RIGHT_MOUSE_DOWN | OTHER_MOUSE_DOWN
    ) {
        let mut state = context.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.pressed_at.is_some() {
            // Key-up itself is not a new chord; key-down and modifier changes are.
            if cancels_escape(kind, key, false) {
                if !state.chorded {
                    state.diagnostics.record(
                        Duration::from_nanos(unsafe { CGEventGetTimestamp(event) }),
                        Event::Chord,
                    );
                }
                state.chord();
            }
            unsafe {
                CGEventSetFlags(
                    event,
                    remapped_flags(CGEventGetFlags(event), Some(state.hold_modifier)),
                );
            }
        }
    }
    let _ = proxy;
    event
}

fn cancels_escape(kind: u32, key: i64, synthetic: bool) -> bool {
    if synthetic {
        return false;
    }
    match kind {
        // Only physical modifier keycodes identify a chord. State-only flags
        // notifications (255 in the live Caps trace) are not another key press.
        FLAGS_CHANGED => matches!(key, 54..=56 | 58..=63),
        KEY_DOWN | LEFT_MOUSE_DOWN | RIGHT_MOUSE_DOWN | OTHER_MOUSE_DOWN => true,
        _ => false,
    }
}

fn remapped_flags(flags: u64, modifier: Option<HoldModifier>) -> u64 {
    (flags & !CAPS_LOCK_FLAG) | modifier.map_or(0, HoldModifier::flag)
}

fn keyboard_matching() -> Option<Ref> {
    let names = [c"DeviceUsagePage", c"DeviceUsage"];
    let usages = [GENERIC_DESKTOP_USAGE_PAGE, KEYBOARD_DEVICE_USAGE];
    let mut keys = [ptr::null_mut(); 2];
    let mut values = [ptr::null_mut(); 2];
    for i in 0..2 {
        keys[i] =
            unsafe { CFStringCreateWithCString(ptr::null_mut(), names[i].as_ptr(), UTF8_ENCODING) };
        values[i] = unsafe { CFNumberCreate(ptr::null_mut(), 3, &usages[i] as *const i32 as Ref) };
    }
    let matching = if keys.iter().chain(values.iter()).all(|v| !v.is_null()) {
        unsafe {
            CFDictionaryCreate(
                ptr::null_mut(),
                keys.as_ptr(),
                values.as_ptr(),
                2,
                &raw const kCFTypeDictionaryKeyCallBacks as *const c_void,
                &raw const kCFTypeDictionaryValueCallBacks as *const c_void,
            )
        }
    } else {
        ptr::null_mut()
    };
    for value in keys.into_iter().chain(values) {
        if !value.is_null() {
            unsafe {
                CFRelease(value);
            }
        }
    }
    if matching.is_null() {
        None
    } else {
        Some(matching)
    }
}

impl Context {
    fn apply_config(&self, config: &config::Config) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(action) = state.configure(config) {
            let timestamp = mac::monotonic_timestamp();
            state.diagnostics.record(timestamp, Event::Cancelled);
            unsafe {
                emit_recorded(&mut state, timestamp, action, None);
            }
        }
        if config.remapping_enabled {
            self.clear_caps_lock();
        }
        self.escape_timeout_ms
            .store(config.escape_timeout_ms, Ordering::Release);
        self.enabled
            .store(config.remapping_enabled, Ordering::Release);
    }

    fn clear_caps_lock(&self) {
        if let Some(lock) = self.caps_lock.get()
            && let Err(error) = lock.clear()
        {
            eprintln!("{error}");
        }
    }
}

fn start_input(context: &Context) -> Result<(), String> {
    // Native Caps state changes below the CGEvent tap, so explicitly reset it too.
    let caps_lock = mac::CapsLock::open()?;
    let _ = context.caps_lock.set(caps_lock);
    let run_loop = unsafe { CFRunLoopGetCurrent() };
    let hid = unsafe { IOHIDManagerCreate(ptr::null_mut(), 0) };
    if hid.is_null() {
        return Err("Could not create HID manager".into());
    }
    let matching = keyboard_matching().ok_or("Could not create keyboard matching dictionary")?;
    unsafe {
        IOHIDManagerSetDeviceMatching(hid, matching);
        CFRelease(matching);
        IOHIDManagerRegisterInputValueCallback(hid, hid_callback, context as *const Context as Ref);
        IOHIDManagerScheduleWithRunLoop(hid, run_loop, kCFRunLoopCommonModes);
    }
    let result = unsafe { IOHIDManagerOpen(hid, 0) };
    if result != 0 {
        return Err(if result as u32 == 0xe000_02c5 {
            "Keyboard is in use by another remapper (0xe00002c5)".into()
        } else {
            format!("Could not open keyboard ({result:#x}); check Input Monitoring")
        });
    }

    // Tap suppresses the processed Caps event; HID supplies the actual down/up pair.
    let mask = (1 << KEY_DOWN)
        | (1 << KEY_UP)
        | (1 << FLAGS_CHANGED)
        | (1 << LEFT_MOUSE_DOWN)
        | (1 << RIGHT_MOUSE_DOWN)
        | (1 << OTHER_MOUSE_DOWN);
    let tap =
        unsafe { CGEventTapCreate(0, 0, 0, mask, callback, context as *const Context as Ref) };
    if tap.is_null() {
        return Err("Could not create event tap; check Input Monitoring and Accessibility".into());
    }
    context.tap.store(tap, Ordering::Relaxed);
    let source = unsafe { CFMachPortCreateRunLoopSource(ptr::null_mut(), tap, 0) };
    if source.is_null() {
        return Err("Could not create event-tap run-loop source".into());
    }
    unsafe {
        CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
        CFRelease(source);
        // The run loop retains the tap source; HID manager and context live until app exit.
    }
    // Settings activates input only after configuration has loaded successfully.
    Ok(())
}

fn main() {
    settings::run(Arc::new(Context::default()));
}

#[cfg(test)]
#[allow(dead_code)]
pub fn check_settings_ui() {
    settings::check_ui();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remapping_removes_caps_lock_but_preserves_other_modifiers() {
        let shift = 1 << 17;
        let command = 1 << 20;
        assert_eq!(
            remapped_flags(CAPS_LOCK_FLAG | shift | command, None),
            shift | command
        );
        assert_eq!(
            remapped_flags(CAPS_LOCK_FLAG | shift, Some(HoldModifier::Control)),
            shift | CONTROL_FLAG
        );
        assert_eq!(
            remapped_flags(CAPS_LOCK_FLAG | CONTROL_FLAG, None),
            CONTROL_FLAG
        );
    }

    #[test]
    fn mapping_snapshot_and_disable_release_the_original_modifier() {
        for modifier in HoldModifier::ALL {
            for tap in TapKey::ALL {
                let mut state = State::default();
                state.configure(&config::Config {
                    hold_modifier: modifier,
                    tap_key: tap,
                    ..config::Config::default()
                });
                assert_eq!(
                    state.press(Duration::ZERO, ALONE_TIMEOUT),
                    Some(Action::ModifierDown(modifier))
                );
                state.configure(&config::Config::default());
                assert_eq!(
                    state.release(Duration::from_millis(100)),
                    vec![Action::ModifierUp(modifier), Action::Tap(tap)]
                );
                state.configure(&config::Config {
                    hold_modifier: modifier,
                    tap_key: tap,
                    ..config::Config::default()
                });
                state.press(Duration::ZERO, ALONE_TIMEOUT);
                assert_eq!(
                    state.configure(&config::Config {
                        remapping_enabled: false,
                        ..config::Config::default()
                    }),
                    Some(Action::ModifierUp(modifier))
                );
                assert!(state.release(Duration::from_millis(100)).is_empty());
                assert_eq!(state.cancel(), None);
                assert_eq!(
                    remapped_flags(CAPS_LOCK_FLAG | CONTROL_FLAG, Some(modifier)),
                    CONTROL_FLAG | modifier.flag()
                );
            }
        }
    }

    #[test]
    fn diagnostics_follow_physical_timing_and_do_not_change_actions() {
        let mut state = State::default();
        state.diagnostics.recording = true;
        state.press(Duration::from_secs(10), ALONE_TIMEOUT);
        state.configure(&config::Config {
            escape_timeout_ms: 1000,
            ..config::Config::default()
        });
        assert_eq!(
            state.release(Duration::from_millis(10317)),
            vec![Action::ControlUp]
        );
        assert_eq!(
            state.diagnostics.rows.back().unwrap().event,
            Event::PhysicalUp {
                elapsed_ms: 317,
                timeout_ms: 300,
                decision: Decision::Hold
            }
        );
        state.diagnostics.recording = false;
        let count = state.diagnostics.rows.len();
        state.press(Duration::from_secs(11), ALONE_TIMEOUT);
        assert_eq!(
            state.release(Duration::from_millis(11100)),
            vec![Action::ControlUp, Action::Escape]
        );
        assert_eq!(state.diagnostics.rows.len(), count);
    }

    #[test]
    fn quick_tap_is_control_then_escape() {
        let now = Duration::ZERO;
        let mut s = State::default();
        assert_eq!(s.press(now, ALONE_TIMEOUT), Some(Action::ControlDown));
        assert_eq!(
            s.release(now + Duration::from_millis(100)),
            vec![Action::ControlUp, Action::Escape]
        );
        assert!(s.release(now).is_empty());
    }

    #[test]
    fn hold_releases_control_without_escape() {
        let now = Duration::ZERO;
        let mut s = State::default();
        assert_eq!(s.press(now, ALONE_TIMEOUT), Some(Action::ControlDown));
        assert_eq!(
            s.release(now + Duration::from_millis(300)),
            vec![Action::ControlUp]
        );
    }

    #[test]
    fn chord_releases_control_without_escape() {
        let now = Duration::ZERO;
        let mut s = State::default();
        s.press(now, ALONE_TIMEOUT);
        s.chord();
        assert_eq!(
            s.release(now + Duration::from_millis(10)),
            vec![Action::ControlUp]
        );
    }

    #[test]
    fn event_duration_uses_the_exact_timeout_cutoff() {
        // Pure duration policy test, not proof of native callback delivery.
        let pressed = Duration::from_secs(10);
        let mut s = State::default();
        s.press(pressed, ALONE_TIMEOUT);
        assert_eq!(
            s.release(pressed + ALONE_TIMEOUT - Duration::from_nanos(1)),
            vec![Action::ControlUp, Action::Escape]
        );
        s.press(pressed, ALONE_TIMEOUT);
        assert_eq!(s.release(pressed + ALONE_TIMEOUT), vec![Action::ControlUp]);
    }

    #[test]
    fn timeout_changes_apply_to_the_next_press_not_an_active_hold() {
        let input = Context::default();
        input.apply_config(&config::Config {
            escape_timeout_ms: 500,
            ..config::Config::default()
        });
        input.state.lock().unwrap().press(
            Duration::ZERO,
            Duration::from_millis(input.escape_timeout_ms.load(Ordering::Acquire)),
        );
        input.apply_config(&config::Config::default());
        let mut state = input.state.lock().unwrap();
        assert_eq!(state.press(Duration::from_millis(100), ALONE_TIMEOUT), None);
        assert_eq!(
            state.release(Duration::from_millis(400)),
            vec![Action::ControlUp, Action::Escape]
        );
        state.press(
            Duration::from_secs(1),
            Duration::from_millis(input.escape_timeout_ms.load(Ordering::Acquire)),
        );
        assert_eq!(
            state.release(Duration::from_millis(1400)),
            vec![Action::ControlUp]
        );
    }

    #[test]
    fn modifier_state_notification_does_not_cancel_an_alone_tap() {
        let mut state = State::default();
        state.press(Duration::ZERO, Duration::from_millis(400));
        if cancels_escape(FLAGS_CHANGED, 255, false) {
            state.chord();
        }
        assert_eq!(
            state.release(Duration::from_millis(317)),
            vec![Action::ControlUp, Action::Escape]
        );
        for key in [54, 55, 56, 58, 59, 60, 61, 62, 63] {
            assert!(cancels_escape(FLAGS_CHANGED, key, false));
        }
    }

    #[test]
    fn chord_policy_ignores_key_up_caps_transitions_and_synthetic_input() {
        for kind in [
            KEY_DOWN,
            FLAGS_CHANGED,
            LEFT_MOUSE_DOWN,
            RIGHT_MOUSE_DOWN,
            OTHER_MOUSE_DOWN,
        ] {
            assert!(cancels_escape(kind, 56, false));
            assert!(!cancels_escape(kind, 56, true));
        }
        assert!(!cancels_escape(KEY_UP, 0, false));
        assert!(!cancels_escape(FLAGS_CHANGED, CAPS, false));
    }

    #[test]
    fn repeated_down_and_cancel_do_not_stick() {
        let now = Duration::ZERO;
        let mut s = State::default();
        s.press(now, ALONE_TIMEOUT);
        assert_eq!(s.press(now, ALONE_TIMEOUT), None);
        assert_eq!(s.cancel(), Some(Action::ControlUp));
        assert_eq!(s.cancel(), None);
        assert!(s.release(now + Duration::from_millis(100)).is_empty());
        assert_eq!(
            s.press(now + Duration::from_secs(1), ALONE_TIMEOUT),
            Some(Action::ControlDown)
        );
        assert_eq!(
            s.release(now + Duration::from_millis(1100)),
            vec![Action::ControlUp, Action::Escape]
        );
    }
}
