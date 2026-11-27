//! UBMDF Android app entry (NativeActivity).
//!
//! Start-up chain: Android NativeActivity -> `ubmdf_on_create` (named by the
//! manifest's `android.app.func_name`, because a Rust cdylib does not export
//! the C glue's `ANativeActivity_onCreate`) -> raylib's android_native_app_glue
//! -> raylib `android_main` -> `main` below -> the shared UBMDF GUI.
#![cfg(target_os = "android")]

use std::ffi::{c_char, c_int, c_void};

unsafe extern "C" {
    /// android_native_app_glue (compiled into raylib's static library).
    fn ANativeActivity_onCreate(activity: *mut c_void, saved_state: *mut c_void, saved_state_size: usize);
}

/// Entry point named in AndroidManifest.xml (android.app.func_name).
#[unsafe(no_mangle)]
pub extern "C" fn ubmdf_on_create(activity: *mut c_void, saved_state: *mut c_void, saved_state_size: usize) {
    unsafe { ANativeActivity_onCreate(activity, saved_state, saved_state_size) }
}

/// Called by raylib's android_main on the app thread.
#[unsafe(no_mangle)]
pub extern "C" fn main(_argc: c_int, _argv: *const *const c_char) -> c_int {
    ubmdf::android::init_environment();
    ubmdf::logger::init();
    if let Err(e) = ubmdf::bundled::install_missing() {
        log::warn!("bundled firmware not installed: {e:#}");
    }
    if let Err(e) = ubmdf::gui::run_gui(ubmdf::gui::Opts::default()) {
        log::error!("ubmdf: {e:#}");
        return 1;
    }
    0
}
