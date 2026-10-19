//! Background blur behind a transparent window, like WezTerm's `macos_window_background_blur`.
//! macOS has no public call for it; this uses the same private CoreGraphics one.

/// Blur the window behind `cc` by `radius` (0..=100). Does nothing off macOS.
#[cfg(target_os = "macos")]
pub(super) fn apply(cc: &eframe::CreationContext<'_>, radius: u32) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::{c_char, c_void};
    #[link(name = "objc")]
    extern "C" {
        fn objc_msgSend();
        fn sel_registerName(name: *const c_char) -> *mut c_void;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGSMainConnectionID() -> u32;
        fn CGSSetWindowBackgroundBlurRadius(connection: u32, window: i64, radius: i32) -> i32;
    }
    let Ok(handle) = cc.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    // SAFETY: `ns_view` is the live NSView of this window; `window` and `windowNumber` are plain
    // Objective-C getters called with the matching function signatures.
    unsafe {
        let send_ptr: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let send_int: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i64 =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let window = send_ptr(
            appkit.ns_view.as_ptr(),
            sel_registerName(c"window".as_ptr()),
        );
        if window.is_null() {
            return;
        }
        let number = send_int(window, sel_registerName(c"windowNumber".as_ptr()));
        CGSSetWindowBackgroundBlurRadius(CGSMainConnectionID(), number, radius as i32);
    }
}
#[cfg(not(target_os = "macos"))]
pub(super) fn apply(_: &eframe::CreationContext<'_>, _: u32) {}
