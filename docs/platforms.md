# Platform notes

## Windows
The service port needs the **WinUSB** driver (install it with Zadig) for UBMDF to open it. Blackmagic's own `DesktopVideoUpdateTool` needs Blackmagic's `DesktopVideoUpdate` driver instead. To switch, re-run Zadig or bind Blackmagic's INF with `pnputil /add-driver … /install`. The Devices tab shows the bound driver.

## Linux
No driver is needed. A udev rule gives access without root:

```
SUBSYSTEM=="usb", ATTRS{idVendor}=="1edb", ATTRS{idProduct}=="be13", MODE="0660", TAG+="uaccess"
```

## macOS
No driver install is needed, because IOKit gives user-space access. Blackmagic's macOS `DesktopVideoUpdateTool` can flash the service port too. [tracing.md](tracing.md) shows how to trace it.

## Android
`UBMDF-android-arm64.apk` is the same UBMDF app (GUI, protocol and bundled firmware) running as an Android NativeActivity on 64-bit ARM phones and tablets.

**Untested on a device.** The APK is built and checked in CI, but it has not been run on a phone yet.

Requirements and behaviour:

- **USB host (OTG):** the phone or tablet must support it. Connect the UltraStudio service port with a USB OTG cable or adapter. The service port is a USB device; Android provides the host side.
- **Permission:**
  - plugging the unit in offers to open UBMDF, and accepting grants USB access;
  - otherwise UBMDF shows Android's permission prompt the first time you dump, reset or flash. Tap **Allow**, then repeat the action.
- **No drivers or udev rules.** Android hands UBMDF a file descriptor for the device, which goes to the same protocol code as on desktop.
- **Storage:** firmware, the index, logs, dumps and settings live in app-private storage. The 43 bundled firmware versions install there on first run.
- **Interface:** scaled to the screen density, portrait only; drag to scroll lists.
- **Limitations:**
  - **Update** can check the catalog but cannot download and extract installers (that needs Windows, like Linux and macOS);
  - **Show file / folder** buttons only log the path;
  - the APK is signed with a debug key, for sideloading, not the Play Store.

> [!WARNING]
> Phone USB power and OTG adapters vary. A flash must not be interrupted, so use a powered setup you trust and read the [disclaimer](../DISCLAIMER.md) first.

Building: CI job `android` (`.github/workflows/build.yml`):

1. `cargo-ndk` builds `android/` (a `cdylib` around the `ubmdf` library) for arm64-v8a, with raylib-sys patched at build time by `tools/android/patch_raylib_sys.py`.
2. Gradle packages the APK from `android/app`.
