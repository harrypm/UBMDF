//! Android platform glue (NativeActivity via raylib's android_native_app_glue).
//!
//! * App storage: the activity's internal data dir becomes the firmware root
//!   (UBMDF_FIRMWARE), so the bundled firmware, index, logs and dumps live in
//!   app-private storage.
//! * UI scale: screen density / 160 (mdpi), so the desktop layout is legible
//!   on phones and tablets.
//! * USB: Android apps cannot enumerate /dev/bus/usb. Devices are listed and
//!   opened through the Java `UsbManager` (JNI); the connection's file
//!   descriptor is handed to nusb (`Device::from_fd`), so the protocol code is
//!   shared unchanged with the desktop builds.
use std::ffi::{CStr, c_char, c_void};
use std::os::fd::{BorrowedFd, OwnedFd};
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use jni::objects::{GlobalRef, JObject, JObjectArray, JString, JValue};
use jni::{JNIEnv, JavaVM};

use crate::protocol::{DeviceEntry, PID, VID};

#[repr(C)]
struct ANativeActivity {
    callbacks: *mut c_void,
    vm: *mut jni::sys::JavaVM,
    env: *mut c_void,
    clazz: jni::sys::jobject,
    internal_data_path: *const c_char,
    external_data_path: *const c_char,
    sdk_version: i32,
    instance: *mut c_void,
    asset_manager: *mut c_void,
    obb_path: *const c_char,
}

/// Leading fields of `struct android_app` (android_native_app_glue.h).
#[repr(C)]
struct AndroidApp {
    user_data: *mut c_void,
    on_app_cmd: *mut c_void,
    on_input_event: *mut c_void,
    activity: *mut ANativeActivity,
    config: *mut c_void,
}

unsafe extern "C" {
    /// Provided by raylib's rcore_android.c (valid once android_main runs).
    fn GetAndroidApp() -> *mut AndroidApp;
    fn AConfiguration_getDensity(config: *const c_void) -> i32;
}

fn activity() -> Result<&'static ANativeActivity> {
    unsafe {
        let app = GetAndroidApp();
        if app.is_null() || (*app).activity.is_null() {
            bail!("Android activity not available");
        }
        Ok(&*(*app).activity)
    }
}

/// App-private storage (e.g. /data/user/0/io.github.ubmdf/files).
pub fn data_dir() -> Option<PathBuf> {
    let a = activity().ok()?;
    if a.internal_data_path.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(a.internal_data_path) }.to_string_lossy().to_string();
    Some(PathBuf::from(s))
}

/// UI scale from screen density (160 dpi = 1.0), clamped to a sane range.
pub fn ui_scale() -> f32 {
    unsafe {
        let app = GetAndroidApp();
        if app.is_null() || (*app).config.is_null() {
            return 2.0;
        }
        let dpi = AConfiguration_getDensity((*app).config);
        if dpi <= 0 { 2.0 } else { (dpi as f32 / 160.0).clamp(1.0, 4.0) }
    }
}

/// Called once from the Android entry point, before any thread starts.
pub fn init_environment() {
    if let Some(dir) = data_dir() {
        // SAFETY: single-threaded at this point (called first thing in main()).
        unsafe { std::env::set_var("UBMDF_FIRMWARE", dir.join("firmware")) };
    }
}

fn vm() -> Result<JavaVM> {
    let a = activity()?;
    unsafe { JavaVM::from_raw(a.vm) }.context("JavaVM")
}

/// Run `f` with a JNI env for this thread (attached permanently: the device
/// poller thread calls this every 2 s) inside a local reference frame, so
/// per-call local refs are always released. Any pending Java exception is
/// cleared before returning.
fn with_env<T>(f: impl FnOnce(&mut JNIEnv, &JObject) -> Result<T>) -> Result<T> {
    let vm = vm()?;
    let mut env = vm.attach_current_thread_permanently().context("JNI attach")?;
    let ctx = unsafe { JObject::from_raw(activity()?.clazz) };
    env.push_local_frame(64).context("JNI local frame")?;
    let r = f(&mut env, &ctx);
    clear_exc(&mut env);
    // SAFETY: no local refs from inside the frame escape it (results are Rust values).
    let _ = unsafe { env.pop_local_frame(&JObject::null()) };
    r
}

/// Open connections are kept alive here (dropping the Java object would close
/// the descriptor the framework owns; nusb holds its own dup).
static CONNECTIONS: Mutex<Vec<GlobalRef>> = Mutex::new(Vec::new());

/// Clear a pending Java exception (e.g. SecurityException from getSerialNumber
/// before permission is granted) so later JNI calls stay valid.
fn clear_exc(env: &mut JNIEnv) {
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
}

fn opt_string(env: &mut JNIEnv, obj: &JObject, method: &str) -> Option<String> {
    let v = env.call_method(obj, method, "()Ljava/lang/String;", &[]);
    match v.and_then(|v| v.l()) {
        Ok(o) if !o.is_null() => {
            let js = JString::from(o);
            env.get_string(&js).ok().map(|s| s.into())
        }
        _ => {
            clear_exc(env);
            None
        }
    }
}

fn usb_manager<'a>(env: &mut JNIEnv<'a>, ctx: &JObject) -> Result<JObject<'a>> {
    let name = env.new_string("usb")?;
    let m = env
        .call_method(ctx, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&name)])
        .and_then(|v| v.l());
    match m {
        Ok(m) if !m.is_null() => Ok(m),
        _ => {
            clear_exc(env);
            bail!("USB host service not available on this device")
        }
    }
}

/// Service-port devices (VID/PID match) as Java UsbDevice objects.
fn service_ports<'a>(env: &mut JNIEnv<'a>, mgr: &JObject) -> Result<Vec<JObject<'a>>> {
    let map = env.call_method(mgr, "getDeviceList", "()Ljava/util/HashMap;", &[])?.l()?;
    let values = env.call_method(&map, "values", "()Ljava/util/Collection;", &[])?.l()?;
    let arr = JObjectArray::from(env.call_method(&values, "toArray", "()[Ljava/lang/Object;", &[])?.l()?);
    let n = env.get_array_length(&arr)?;
    let mut out = Vec::new();
    for i in 0..n {
        let d = env.get_object_array_element(&arr, i)?;
        let vid = env.call_method(&d, "getVendorId", "()I", &[])?.i()? as u16;
        let pid = env.call_method(&d, "getProductId", "()I", &[])?.i()? as u16;
        if vid == VID && pid == PID {
            out.push(d);
        }
    }
    Ok(out)
}

/// Enumerate service-port devices (no permission needed for the listing).
pub fn list_devices() -> Result<Vec<DeviceEntry>> {
    with_env(|env, ctx| {
        let mgr = usb_manager(env, ctx)?;
        let devs = service_ports(env, &mgr)?;
        let mut out = Vec::new();
        for (i, d) in devs.iter().enumerate() {
            let name = opt_string(env, d, "getDeviceName").unwrap_or_else(|| format!("usb{i}"));
            let product = opt_string(env, d, "getProductName");
            // Serial needs permission on Android 10+ (SecurityException otherwise).
            let has = env
                .call_method(&mgr, "hasPermission", "(Landroid/hardware/usb/UsbDevice;)Z", &[JValue::Object(d)])
                .and_then(|v| v.z())
                .unwrap_or(false);
            clear_exc(env);
            let serial = if has { opt_string(env, d, "getSerialNumber") } else { None };
            out.push(DeviceEntry {
                product,
                serial,
                bus_id: name.clone(),
                address: name.rsplit('/').next().and_then(|n| n.parse().ok()).unwrap_or(0),
                port_chain: Vec::new(),
                speed: None,
                driver: Some(if has { "Android USB (permission granted)".into() } else { "Android USB (permission needed)".into() }),
            });
        }
        Ok(out)
    })
}

/// Open the first service-port device and return a dup of its descriptor.
/// Without permission, Android's permission dialog is requested and an error
/// asks the user to retry after tapping Allow.
pub fn open_service_port() -> Result<OwnedFd> {
    with_env(|env, ctx| {
        let mgr = usb_manager(env, ctx)?;
        let devs = service_ports(env, &mgr)?;
        let Some(dev) = devs.first() else {
            bail!("no Blackmagic service-port device found (USB VID 1edb PID be13) — connect it with a USB OTG cable/adapter");
        };
        let has = env
            .call_method(&mgr, "hasPermission", "(Landroid/hardware/usb/UsbDevice;)Z", &[JValue::Object(dev)])?
            .z()?;
        if !has {
            let action = env.new_string("io.github.ubmdf.USB_PERMISSION")?;
            let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)])?;
            let pkg = env.call_method(ctx, "getPackageName", "()Ljava/lang/String;", &[])?.l()?;
            env.call_method(&intent, "setPackage", "(Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(&pkg)])?;
            // FLAG_MUTABLE (API 31+) with an explicit (package) intent, as Android 14 requires.
            const FLAG_MUTABLE: i32 = 0x0200_0000;
            let pi = env
                .call_static_method(
                    "android/app/PendingIntent",
                    "getBroadcast",
                    "(Landroid/content/Context;ILandroid/content/Intent;I)Landroid/app/PendingIntent;",
                    &[JValue::Object(ctx), JValue::Int(0), JValue::Object(&intent), JValue::Int(FLAG_MUTABLE)],
                )?
                .l()?;
            env.call_method(
                &mgr,
                "requestPermission",
                "(Landroid/hardware/usb/UsbDevice;Landroid/app/PendingIntent;)V",
                &[JValue::Object(dev), JValue::Object(&pi)],
            )?;
            bail!("USB permission requested — tap Allow on the Android prompt, then try again");
        }
        let conn = env
            .call_method(
                &mgr,
                "openDevice",
                "(Landroid/hardware/usb/UsbDevice;)Landroid/hardware/usb/UsbDeviceConnection;",
                &[JValue::Object(dev)],
            )?
            .l()?;
        if conn.is_null() {
            bail!("Android refused to open the USB device");
        }
        let fd = env.call_method(&conn, "getFileDescriptor", "()I", &[])?.i()?;
        if fd < 0 {
            bail!("USB connection has no file descriptor");
        }
        let owned = unsafe { BorrowedFd::borrow_raw(fd) }.try_clone_to_owned().context("dup USB fd")?;
        let keep = env.new_global_ref(&conn)?;
        if let Ok(mut c) = CONNECTIONS.lock() {
            c.clear(); // previous connections are no longer used by nusb
            c.push(keep);
        }
        Ok(owned)
    })
}
