//! macOS application menu setup.
//!
//! eframe/winit apps don't install an **Edit** menu, so the standard
//! Cut/Copy/Paste/Select-All key equivalents are never delivered down the
//! responder chain. That doesn't matter for pure-egui content (egui handles its
//! own clipboard), but the native Web Page View embeds a `WKWebView` child whose
//! copy/paste rely on macOS routing `cmd+C`/`cmd+V` to it via `performKeyEquivalent:`.
//!
//! Installing an Edit menu whose items target `nil` (the default) makes the
//! responder chain deliver `copy:`/`paste:`/etc. to whichever view is first
//! responder — i.e. the focused webview — so selecting text and pressing
//! `cmd+C` works.
#![allow(unsafe_code)] // Thin Objective-C bridge to build an NSMenu; see module docs.

#[cfg(target_os = "macos")]
pub fn install_edit_menu() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe { install_edit_menu_impl() });
}

#[cfg(not(target_os = "macos"))]
pub fn install_edit_menu() {}

#[cfg(target_os = "macos")]
unsafe fn install_edit_menu_impl() {
    use objc2::runtime::{AnyObject, Sel};
    use objc2::{class, msg_send, sel};
    use std::ffi::CString;

    // All objects below are added to the app's main menu, which lives for the whole
    // process, so the menu retains them; we intentionally don't balance the +1 from
    // alloc/init. NSStrings are autoreleased and only need to outlive the calls here.
    unsafe fn ns_string(s: &str) -> *mut AnyObject {
        let cstr = CString::new(s).unwrap_or_default();
        unsafe { msg_send![class!(NSString), stringWithUTF8String: cstr.as_ptr()] }
    }

    unsafe fn add_item(menu: *mut AnyObject, title: &str, action: Sel, key: &str) {
        unsafe {
            let alloc: *mut AnyObject = msg_send![class!(NSMenuItem), alloc];
            let item: *mut AnyObject = msg_send![
                alloc,
                initWithTitle: ns_string(title),
                action: action,
                keyEquivalent: ns_string(key)
            ];
            let _: () = msg_send![menu, addItem: item];
        }
    }

    unsafe {
        // The app (and main thread) already exist by the time the first frame runs.
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return;
        }

        let mut main_menu: *mut AnyObject = msg_send![app, mainMenu];
        if main_menu.is_null() {
            let alloc: *mut AnyObject = msg_send![class!(NSMenu), alloc];
            main_menu = msg_send![alloc, init];
            let _: () = msg_send![app, setMainMenu: main_menu];
        }

        let edit_alloc: *mut AnyObject = msg_send![class!(NSMenu), alloc];
        let edit_menu: *mut AnyObject = msg_send![edit_alloc, initWithTitle: ns_string("Edit")];

        add_item(edit_menu, "Undo", sel!(undo:), "z");
        add_item(edit_menu, "Redo", sel!(redo:), "Z");
        add_item(edit_menu, "Cut", sel!(cut:), "x");
        add_item(edit_menu, "Copy", sel!(copy:), "c");
        add_item(edit_menu, "Paste", sel!(paste:), "v");
        add_item(edit_menu, "Select All", sel!(selectAll:), "a");

        let item_alloc: *mut AnyObject = msg_send![class!(NSMenuItem), alloc];
        let edit_item: *mut AnyObject = msg_send![item_alloc, init];
        let _: () = msg_send![edit_item, setSubmenu: edit_menu];
        let _: () = msg_send![main_menu, addItem: edit_item];
    }
}
