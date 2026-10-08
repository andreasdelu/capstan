#![allow(unexpected_cfgs)] // objc 0.2 macros check a legacy cargo-clippy cfg.

use std::cell::{Cell, RefCell};
use std::ffi::CStr;
use std::sync::OnceLock;

thread_local! {
    static STATUS_ITEM: Cell<id> = const { Cell::new(nil) };
    static MENU_STATUS: RefCell<Option<Box<dyn Fn() -> MenuStatus>>> = const { RefCell::new(None) };
}

// AppKit objects are only accessed on the application's main thread.
pub fn set_menu_bar_visible(visible: bool) {
    STATUS_ITEM.with(|item| {
        let item = item.get();
        if !item.is_null() {
            unsafe {
                let _: () = msg_send![item, setVisible: if visible { YES } else { NO }];
            }
        }
    });
}

use cocoa::base::{id, nil};
use cocoa::foundation::NSString;
use objc::declare::ClassDecl;
use objc::runtime::{BOOL, Class, NO, Object, Sel, YES};
use objc::{class, msg_send, sel, sel_impl};

#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightPostEventAccess() -> bool;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDCheckAccess(request_type: u32) -> u32;
    fn IOServiceMatching(name: *const i8) -> *mut std::ffi::c_void;
    fn IOServiceGetMatchingService(port: u32, matching: *mut std::ffi::c_void) -> u32;
    fn IOServiceOpen(service: u32, task: u32, kind: u32, connection: *mut u32) -> i32;
    fn IOServiceClose(connection: u32) -> i32;
    fn IOObjectRelease(object: u32) -> i32;
    fn IOHIDGetModifierLockState(connection: u32, selector: i32, state: *mut bool) -> i32;
    fn IOHIDSetModifierLockState(connection: u32, selector: i32, state: bool) -> i32;
}

unsafe extern "C" {
    static mach_task_self_: u32;
    fn mach_timebase_info(info: *mut Timebase) -> i32;
    fn mach_absolute_time() -> u64;
}

#[repr(C)]
struct Timebase {
    numer: u32,
    denom: u32,
}

pub fn monotonic_timestamp() -> std::time::Duration {
    hid_timestamp(unsafe { mach_absolute_time() })
}

pub fn hid_timestamp(ticks: u64) -> std::time::Duration {
    static TIMEBASE: OnceLock<Timebase> = OnceLock::new();
    let info = TIMEBASE.get_or_init(|| {
        let mut info = Timebase { numer: 0, denom: 0 };
        let result = unsafe { mach_timebase_info(&mut info) };
        assert!(result == 0 && info.denom != 0, "mach timebase unavailable");
        info
    });
    ticks_to_duration(ticks, info.numer, info.denom)
}

fn ticks_to_duration(ticks: u64, numer: u32, denom: u32) -> std::time::Duration {
    let nanos = (ticks as u128 * numer as u128 / denom as u128).min(u64::MAX as u128) as u64;
    std::time::Duration::from_nanos(nanos)
}

#[cfg(test)]
mod timestamp_tests {
    use super::*;

    #[test]
    fn converts_non_unit_timebase_without_intermediate_overflow() {
        assert_eq!(
            ticks_to_duration(72_000_000, 125, 3),
            std::time::Duration::from_secs(3)
        );
        assert_eq!(
            ticks_to_duration(u64::MAX, 125, 3),
            std::time::Duration::from_nanos(u64::MAX)
        );
    }
}

// A parameter connection, not an exclusive keyboard grab. macOS owns the lock
// state before the event tap runs; dropping a CGEvent cannot undo that state.
pub struct CapsLock(u32);

impl CapsLock {
    pub fn open() -> Result<Self, String> {
        unsafe {
            let matching = IOServiceMatching(c"IOHIDSystem".as_ptr());
            if matching.is_null() {
                return Err("Could not match IOHIDSystem".into());
            }
            let service = IOServiceGetMatchingService(0, matching);
            if service == 0 {
                return Err("Could not find IOHIDSystem".into());
            }
            let mut connection = 0;
            let result = IOServiceOpen(service, mach_task_self_, 1, &mut connection);
            IOObjectRelease(service);
            if result != 0 {
                return Err(format!("Could not open Caps Lock control ({result:#x})"));
            }
            Ok(Self(connection))
        }
    }

    pub fn clear(&self) -> Result<(), String> {
        let mut locked = false;
        let mut result = unsafe { IOHIDGetModifierLockState(self.0, 1, &mut locked) };
        if result == 0 && locked {
            result = unsafe { IOHIDSetModifierLockState(self.0, 1, false) };
        }
        if result == 0 {
            Ok(())
        } else {
            Err(format!("Could not clear native Caps Lock ({result:#x})"))
        }
    }
}

impl Drop for CapsLock {
    fn drop(&mut self) {
        unsafe {
            IOServiceClose(self.0);
        }
    }
}

#[derive(Clone, Copy)]
pub struct Permissions {
    pub input_monitoring: bool,
    pub accessibility: bool,
}

impl Permissions {
    pub fn ready(self) -> bool {
        self.input_monitoring && self.accessibility
    }
}

pub fn permissions() -> Permissions {
    unsafe {
        Permissions {
            input_monitoring: IOHIDCheckAccess(1) == 0,
            accessibility: CGPreflightPostEventAccess(),
        }
    }
}

pub enum PrivacyPane {
    InputMonitoring,
    Accessibility,
}

pub fn open_privacy_pane(pane: PrivacyPane) -> Result<(), String> {
    let url = match pane {
        PrivacyPane::InputMonitoring => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent"
        }
        PrivacyPane::Accessibility => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
    };
    unsafe {
        let text = NSString::alloc(nil).init_str(url);
        let url: id = msg_send![class!(NSURL), URLWithString: text];
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let opened: BOOL = msg_send![workspace, openURL: url];
        if opened == NO {
            return Err("Could not open System Settings".into());
        }
    }
    Ok(())
}

pub fn bundled() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            let contents = exe.parent()?.parent()?;
            (contents.file_name()? == "Contents" && contents.join("Info.plist").is_file())
                .then_some(())
        })
        .is_some()
}

pub fn remapping_enabled() -> bool {
    unsafe {
        let defaults: id = msg_send![class!(NSUserDefaults), standardUserDefaults];
        let key = NSString::alloc(nil).init_str("remappingEnabled");
        let value: id = msg_send![defaults, objectForKey: key];
        if value.is_null() {
            true
        } else {
            msg_send![defaults, boolForKey: key]
        }
    }
}

pub fn login_enabled() -> bool {
    if !bundled() {
        return false;
    }
    unsafe {
        let service: id = msg_send![class!(SMAppService), mainAppService];
        let status: isize = msg_send![service, status];
        status == 1
    }
}

pub fn set_login_enabled(enabled: bool) -> Result<(), String> {
    if !bundled() {
        return Err("Launch at Login requires the bundled .app, not cargo run".into());
    }
    unsafe {
        let service: id = msg_send![class!(SMAppService), mainAppService];
        let mut error: id = nil;
        let ok: BOOL = if enabled {
            msg_send![service, registerAndReturnError: &mut error]
        } else {
            msg_send![service, unregisterAndReturnError: &mut error]
        };
        if ok == NO {
            if !error.is_null() {
                let description: id = msg_send![error, localizedDescription];
                let text: *const i8 = msg_send![description, UTF8String];
                if !text.is_null() {
                    return Err(CStr::from_ptr(text).to_string_lossy().into_owned());
                }
            }
            return Err("Could not update macOS Login Items".into());
        }
    }
    if enabled && !login_enabled() {
        return Err("Approve Capstan in System Settings → General → Login Items".into());
    }
    Ok(())
}

pub fn hide_settings() {
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, hide: nil];
        let changed: BOOL = msg_send![app, setActivationPolicy: 1isize];
        if changed == NO {
            eprintln!("Could not hide Capstan from the Dock");
        }
    }
}

extern "C" fn open_settings(_target: &Object, _selector: Sel, _sender: id) {
    show_settings();
}

// GPUI gates on_reopen on AppKit's hasVisibleWindows flag. An accessory app
// must restore its Dock presence even when macOS considers a window visible.
// Replace this single-application process's GPUI delegate-class reopen method,
// not the delegate object or other lifecycle callbacks. Install during GPUI's
// finish-launch callback, after the delegate and activation policy are set.
pub fn install_reopen_handler() {
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let delegate: id = msg_send![app, delegate];
        assert!(
            !delegate.is_null(),
            "Capstan requires the launched GPUI delegate"
        );
        let method = (*delegate)
            .class()
            .instance_method(sel!(applicationShouldHandleReopen:hasVisibleWindows:))
            .expect("GPUI delegate must implement application reopen");
        let implementation = std::mem::transmute::<
            extern "C" fn(&Object, Sel, id, BOOL) -> BOOL,
            objc::runtime::Imp,
        >(handle_reopen);
        objc::runtime::method_setImplementation(method as *const _ as *mut _, implementation);
    }
}

extern "C" fn handle_reopen(_: &Object, _: Sel, _: id, _: BOOL) -> BOOL {
    show_settings();
    // Capstan has already handled window ordering and activation.
    NO
}

pub fn show_settings() {
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let changed: BOOL = msg_send![app, setActivationPolicy: 0isize];
        if changed == NO {
            eprintln!("Could not show Capstan in the Dock");
        }
        let _: () = msg_send![app, unhide: nil];
        let windows: id = msg_send![app, windows];
        let count: usize = msg_send![windows, count];
        for index in 0..count {
            let window: id = msg_send![windows, objectAtIndex: index];
            let _: () = msg_send![window, makeKeyAndOrderFront: nil];
        }
        let _: () = msg_send![app, activateIgnoringOtherApps: YES];
    }
}

pub struct MenuStatus {
    pub input_available: bool,
    pub enabled: bool,
    pub timeout_ms: u64,
    pub login: bool,
}

impl MenuStatus {
    fn labels(&self) -> [String; 3] {
        [
            if !self.input_available {
                "Remapping: input unavailable"
            } else if self.enabled {
                "Remapping: On"
            } else {
                "Remapping: Off"
            }
            .into(),
            if self.input_available {
                format!("Tap timeout: {} ms", self.timeout_ms)
            } else {
                "Tap timeout: unavailable".into()
            },
            format!("Launch at login: {}", if self.login { "On" } else { "Off" }),
        ]
    }
}

extern "C" fn refresh_menu(_: &Object, _: Sel, menu: id) {
    MENU_STATUS.with(|provider| {
        if let Some(provider) = provider.borrow().as_ref() {
            for (index, label) in provider().labels().iter().enumerate() {
                unsafe {
                    let item: id = msg_send![menu, itemAtIndex: index];
                    let label = NSString::alloc(nil).init_str(label);
                    let _: () = msg_send![item, setTitle: label];
                    let _: () = msg_send![label, release];
                }
            }
        }
    });
}

fn menu_target_class() -> &'static Class {
    static CLASS: OnceLock<&'static Class> = OnceLock::new();
    CLASS.get_or_init(|| {
        let mut decl =
            ClassDecl::new("CapsTapMenuTarget", class!(NSObject)).expect("menu target class");
        unsafe {
            decl.add_method(
                sel!(menuNeedsUpdate:),
                refresh_menu as extern "C" fn(&Object, Sel, id),
            );
            decl.add_method(
                sel!(openSettings:),
                open_settings as extern "C" fn(&Object, Sel, id),
            );
        }
        decl.register()
    })
}

#[cfg(test)]
mod menu_tests {
    use super::*;
    #[test]
    fn status_reports_enabled_not_merely_running() {
        let mut status = MenuStatus {
            input_available: true,
            enabled: false,
            timeout_ms: 400,
            login: false,
        };
        assert_eq!(
            status.labels(),
            [
                "Remapping: Off",
                "Tap timeout: 400 ms",
                "Launch at login: Off"
            ]
        );
        status.enabled = true;
        status.login = true;
        assert_eq!(status.labels()[0], "Remapping: On");
        assert_eq!(status.labels()[2], "Launch at login: On");
        status.input_available = false;
        assert_eq!(status.labels()[0], "Remapping: input unavailable");
        assert_eq!(status.labels()[1], "Tap timeout: unavailable");
    }
}

// Small native status menu; the settings themselves are GPUI.
pub fn install_status_menu(status: impl 'static + Fn() -> MenuStatus) {
    MENU_STATUS.with(|provider| *provider.borrow_mut() = Some(Box::new(status)));
    unsafe {
        let bar: id = msg_send![class!(NSStatusBar), systemStatusBar];
        let item: id = msg_send![bar, statusItemWithLength: -1.0f64];
        // Keep our own reference for the lifetime of the process.
        let _: id = msg_send![item, retain];
        STATUS_ITEM.with(|slot| slot.set(item));
        let button: id = msg_send![item, button];
        let bytes = include_bytes!("../assets/MenuBarIcon.png");
        let data: id = msg_send![class!(NSData), dataWithBytes: bytes.as_ptr() length: bytes.len()];
        let image: id = msg_send![class!(NSImage), alloc];
        let image: id = msg_send![image, initWithData: data];
        if !image.is_null() {
            let _: () = msg_send![image, setSize: cocoa::foundation::NSSize::new(18., 18.)];
            let _: () = msg_send![image, setTemplate: YES];
            let _: () = msg_send![button, setImage: image];
            let _: () = msg_send![image, release];
        }
        let label = NSString::alloc(nil).init_str("Capstan");
        let _: () = msg_send![button, setToolTip: label];
        let _: () = msg_send![button, setAccessibilityLabel: label];

        let menu: id = msg_send![class!(NSMenu), new];
        let empty = NSString::alloc(nil).init_str("");
        for _ in 0..3 {
            let row: id = msg_send![class!(NSMenuItem), alloc];
            let row: id =
                msg_send![row, initWithTitle: empty action: sel!(nothing:) keyEquivalent: empty];
            let _: () = msg_send![row, setEnabled: NO];
            let _: () = msg_send![menu, addItem: row];
            let _: () = msg_send![row, release];
        }
        let separator: id = msg_send![class!(NSMenuItem), separatorItem];
        let _: () = msg_send![menu, addItem: separator];

        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let open = NSString::alloc(nil).init_str("Open Settings");
        let open_item: id = msg_send![class!(NSMenuItem), alloc];
        let open_item: id = msg_send![open_item, initWithTitle: open action: sel!(openSettings:) keyEquivalent: empty];
        // NSMenuItem's target is not retained; keep this one target until process exit.
        let target: id = msg_send![menu_target_class(), new];
        let _: () = msg_send![menu, setDelegate: target];
        refresh_menu(&*target, sel!(menuNeedsUpdate:), menu);
        let _: () = msg_send![open_item, setTarget: target];
        let _: () = msg_send![menu, addItem: open_item];

        let quit = NSString::alloc(nil).init_str("Quit Capstan");
        let quit_item: id = msg_send![class!(NSMenuItem), alloc];
        let quit_item: id =
            msg_send![quit_item, initWithTitle: quit action: sel!(terminate:) keyEquivalent: empty];
        let _: () = msg_send![quit_item, setTarget: app];
        let _: () = msg_send![menu, addItem: quit_item];
        let _: () = msg_send![item, setMenu: menu];
        // GPUI opens Settings as a regular app. Closing it switches to Accessory.
    }
}
