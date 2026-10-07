#![allow(unexpected_cfgs)] // objc 0.2 macros check a legacy cargo-clippy cfg.

use std::ffi::CStr;
use std::sync::OnceLock;

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
}

#[repr(C)]
struct Timebase {
    numer: u32,
    denom: u32,
}

pub fn hid_timestamp(ticks: u64) -> std::time::Duration {
    static TIMEBASE: OnceLock<Timebase> = OnceLock::new();
    let info = TIMEBASE.get_or_init(|| {
        let mut info = Timebase { numer: 0, denom: 0 };
        let result = unsafe { mach_timebase_info(&mut info) };
        assert!(result == 0 && info.denom != 0, "mach timebase unavailable");
        info
    });
    let nanos =
        (ticks as u128 * info.numer as u128 / info.denom as u128).min(u64::MAX as u128) as u64;
    std::time::Duration::from_nanos(nanos)
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

pub fn save_remapping_enabled(enabled: bool) {
    unsafe {
        let defaults: id = msg_send![class!(NSUserDefaults), standardUserDefaults];
        let key = NSString::alloc(nil).init_str("remappingEnabled");
        let _: () = msg_send![defaults, setBool: enabled forKey: key];
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
        return Err("Approve Caps Tap in System Settings → General → Login Items".into());
    }
    Ok(())
}

pub fn hide_settings() {
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, hide: nil];
        let changed: BOOL = msg_send![app, setActivationPolicy: 1isize];
        if changed == NO {
            eprintln!("Could not hide Caps Tap from the Dock");
        }
    }
}

extern "C" fn open_settings(_target: &Object, _selector: Sel, _sender: id) {
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let changed: BOOL = msg_send![app, setActivationPolicy: 0isize];
        if changed == NO {
            eprintln!("Could not show Caps Tap in the Dock");
        }
        let _: () = msg_send![app, unhide: nil];
        let _: () = msg_send![app, activateIgnoringOtherApps: YES];
    }
}

fn menu_target_class() -> &'static Class {
    static CLASS: OnceLock<&'static Class> = OnceLock::new();
    CLASS.get_or_init(|| {
        let mut decl =
            ClassDecl::new("CapsTapMenuTarget", class!(NSObject)).expect("menu target class");
        unsafe {
            decl.add_method(
                sel!(openSettings:),
                open_settings as extern "C" fn(&Object, Sel, id),
            );
        }
        decl.register()
    })
}

// Small native status menu; the settings themselves are GPUI.
pub fn install_status_menu(running: bool) {
    unsafe {
        let bar: id = msg_send![class!(NSStatusBar), systemStatusBar];
        let item: id = msg_send![bar, statusItemWithLength: -1.0f64];
        // Keep our own reference for the lifetime of the process.
        let _: id = msg_send![item, retain];
        let button: id = msg_send![item, button];
        let title = NSString::alloc(nil).init_str("Caps");
        let _: () = msg_send![button, setTitle: title];

        let menu: id = msg_send![class!(NSMenu), new];
        let label = NSString::alloc(nil).init_str(if running {
            "Caps Tap is running"
        } else {
            "Caps Tap: input unavailable"
        });
        let empty = NSString::alloc(nil).init_str("");
        let running: id = msg_send![class!(NSMenuItem), alloc];
        let running: id =
            msg_send![running, initWithTitle: label action: sel!(nothing:) keyEquivalent: empty];
        let _: () = msg_send![running, setEnabled: NO];
        let _: () = msg_send![menu, addItem: running];

        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let open = NSString::alloc(nil).init_str("Open Settings");
        let open_item: id = msg_send![class!(NSMenuItem), alloc];
        let open_item: id = msg_send![open_item, initWithTitle: open action: sel!(openSettings:) keyEquivalent: empty];
        // NSMenuItem's target is not retained; keep this one target until process exit.
        let target: id = msg_send![menu_target_class(), new];
        let _: () = msg_send![open_item, setTarget: target];
        let _: () = msg_send![menu, addItem: open_item];

        let quit = NSString::alloc(nil).init_str("Quit Caps Tap");
        let quit_item: id = msg_send![class!(NSMenuItem), alloc];
        let quit_item: id =
            msg_send![quit_item, initWithTitle: quit action: sel!(terminate:) keyEquivalent: empty];
        let _: () = msg_send![quit_item, setTarget: app];
        let _: () = msg_send![menu, addItem: quit_item];
        let _: () = msg_send![item, setMenu: menu];
        // GPUI opens Settings as a regular app. Closing it switches to Accessory.
    }
}
