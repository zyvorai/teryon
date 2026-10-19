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
/// Round the corners of the window's content by `radius` points (needs a transparent window).
#[cfg(target_os = "macos")]
pub(super) fn round_corners(cc: &eframe::CreationContext<'_>, radius: f32) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::{c_char, c_void};
    #[link(name = "objc")]
    extern "C" {
        fn objc_msgSend();
        fn sel_registerName(name: *const c_char) -> *mut c_void;
    }
    let Ok(handle) = cc.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    // SAFETY: `ns_view` is the live NSView of this window; each call below uses the Objective-C
    // method's real signature (object, selector, then one flag or one double).
    unsafe {
        let send_ptr: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let send_bool: unsafe extern "C" fn(*mut c_void, *mut c_void, i8) =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let send_f64: unsafe extern "C" fn(*mut c_void, *mut c_void, f64) =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let view = appkit.ns_view.as_ptr();
        send_bool(view, sel_registerName(c"setWantsLayer:".as_ptr()), 1);
        let layer = send_ptr(view, sel_registerName(c"layer".as_ptr()));
        if layer.is_null() {
            return;
        }
        send_f64(
            layer,
            sel_registerName(c"setCornerRadius:".as_ptr()),
            f64::from(radius),
        );
        send_bool(layer, sel_registerName(c"setMasksToBounds:".as_ptr()), 1);
    }
}
#[cfg(not(target_os = "macos"))]
pub(super) fn round_corners(_: &eframe::CreationContext<'_>, _: f32) {}
#[cfg(not(target_os = "macos"))]
pub(super) fn apply(_: &eframe::CreationContext<'_>, _: u32) {}
