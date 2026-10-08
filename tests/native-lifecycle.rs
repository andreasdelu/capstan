#![allow(unexpected_cfgs)] // objc 0.2 legacy macro cfgs.
// Real AppKit delegate regression check, without input taps, settings writes,
// Login Items, or touching the user's running app. Runs on its own main thread.
#[allow(dead_code, unused_imports)]
#[path = "../src/mac.rs"]
mod mac;

use cocoa::base::{id, nil};
use cocoa::foundation::{NSPoint, NSRect, NSSize, NSString};
use objc::runtime::{BOOL, NO, YES};
use objc::{class, msg_send, sel, sel_impl};

fn main() {
    let application = gpui_kit::application();
    application.on_reopen(|_| mac::show_settings());
    application.run(|cx| unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let delegate: id = msg_send![app, delegate];
        let window: id = msg_send![class!(NSWindow), alloc];
        let window: id = msg_send![window,
            initWithContentRect: NSRect::new(NSPoint::new(0., 0.), NSSize::new(100., 100.))
            styleMask: 1usize backing: 2usize defer: NO];
        let _: () = msg_send![window, setReleasedWhenClosed: NO];
        let title = cocoa::foundation::NSString::alloc(nil).init_str("Capstan");
        let _: () = msg_send![window, setTitle: title];
        let viewer: id = msg_send![class!(NSWindow), alloc];
        let viewer: id = msg_send![viewer,
            initWithContentRect: NSRect::new(NSPoint::new(0., 0.), NSSize::new(100., 100.))
            styleMask: 1usize backing: 2usize defer: NO];
        let _: () = msg_send![viewer, setReleasedWhenClosed: NO];
        let title = cocoa::foundation::NSString::alloc(nil).init_str("Capstan Event Viewer");
        let _: () = msg_send![viewer, setTitle: title];
        let _: () = msg_send![viewer, orderOut: nil];
        // This is the upstream failure seam: visible-window reopen never calls
        // Application::on_reopen, leaving the application in accessory mode.
        let _: BOOL = msg_send![app, setActivationPolicy: 1isize];
        let _: () = msg_send![delegate, applicationShouldHandleReopen: app hasVisibleWindows: YES];
        let before: isize = msg_send![app, activationPolicy];
        assert_eq!(before, 1, "upstream visible-window callback must reproduce the skipped recovery");

        mac::install_reopen_handler();
        for visible in [YES, NO] {
            let _: () = msg_send![window, orderOut: nil];
            let before: BOOL = msg_send![window, isVisible];
            assert_eq!(before, NO, "each case must begin with an ordered-out window");
            mac::hide_settings();
            let hidden: isize = msg_send![app, activationPolicy];
            assert_eq!(hidden, 1);
            let handled: BOOL = msg_send![delegate, applicationShouldHandleReopen: app hasVisibleWindows: visible];
            assert_eq!(handled, NO, "Capstan must suppress default reopen handling");
            let policy: isize = msg_send![app, activationPolicy];
            let window_visible: BOOL = msg_send![window, isVisible];

            assert_eq!(policy, 0, "reopen must restore regular Dock activation policy");
            assert_eq!(window_visible, YES, "reopen must order settings into view");
            let viewer_visible: BOOL = msg_send![viewer, isVisible];
            assert_eq!(viewer_visible, NO, "settings reopen must not resurrect the viewer");

        }
        // An open viewer belongs to the current session, unlike the dismissed
        // viewer above. Application hide/unhide must preserve it on recovery.
        let _: () = msg_send![window, orderFront: nil];
        let _: () = msg_send![viewer, orderFront: nil];
        let settings_before: BOOL = msg_send![window, isVisible];
        let viewer_before: BOOL = msg_send![viewer, isVisible];
        assert_eq!(settings_before, YES);
        assert_eq!(viewer_before, YES);
        mac::hide_settings();
        let hidden: isize = msg_send![app, activationPolicy];
        assert_eq!(hidden, 1);
        let handled: BOOL = msg_send![delegate, applicationShouldHandleReopen: app hasVisibleWindows: YES];
        assert_eq!(handled, NO);
        let policy: isize = msg_send![app, activationPolicy];
        let settings_after: BOOL = msg_send![window, isVisible];
        let viewer_after: BOOL = msg_send![viewer, isVisible];
        assert_eq!(policy, 0);
        assert_eq!(settings_after, YES, "reopen restores settings with an open viewer");
        assert_eq!(viewer_after, YES, "reopen preserves the still-open viewer session");
        let _: () = msg_send![viewer, orderOut: nil];
        let _: () = msg_send![window, orderOut: nil];
        let _: () = msg_send![window, release];
        let _: () = msg_send![viewer, release];
        println!("Native reopen: visible and hidden window paths restore Dock policy and window visibility");
        cx.quit();
    });
}
