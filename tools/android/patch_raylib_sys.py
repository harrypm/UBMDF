"""Make a patched copy of raylib-sys for Android builds (used by CI).

raylib-sys 6.0.0's build.rs derives the Android API level from the last part
of the Rust target triple ("aarch64-linux-android" -> "android"), which hands
the NDK toolchain an invalid ANDROID_PLATFORM / CMAKE_SYSTEM_VERSION. This
copies the crate cargo resolved and reads the level from $ANDROID_PLATFORM
instead (default "android-26", matching minSdk in android/app/build.gradle.kts).

Its bindgen call also passes clang the bare target triple, which NDK r27+
headers reject ("Unversioned target triples are not supported"); the patch
adds the versioned triple (+ NDK sysroot) for Android targets.

Its link() skips X11 with #[cfg(target_os = "android")], but a build script
is compiled for the HOST, so Android links got -lX11; the patch makes that a
runtime check on the target platform.

Usage:  python tools/android/patch_raylib_sys.py <dest-dir>
Then:   cargo ... --config 'patch.crates-io.raylib-sys.path="<dest-dir>"'
"""
import json
import os
import shutil
import subprocess
import sys

OLD = '''let android_platform = target.split("-").last().expect("fail to parse the android version of the target triple, example:'aarch64-linux-android25'");'''
NEW = '''let android_platform_env = env::var("ANDROID_PLATFORM").unwrap_or_else(|_| "android-26".to_string());
            let android_platform = android_platform_env.as_str();'''


OLD_BG = '.clang_arg(plat)'
NEW_BG = '.clang_arg(plat)\n        .clang_args(android_bindgen_args(&target))'
OLD_X11 = 'println!("cargo:rustc-link-lib=X11");'
NEW_X11 = 'if platform != Platform::Android { println!("cargo:rustc-link-lib=X11"); }'
HELPER = """

/// UBMDF patch: versioned triple + NDK sysroot for bindgen on Android.
#[allow(dead_code)]
fn android_bindgen_args(target: &str) -> Vec<String> {
    if !target.contains("android") {
        return Vec::new();
    }
    let level = env::var("ANDROID_PLATFORM").unwrap_or_else(|_| "android-26".into());
    let level = level.trim_start_matches("android-").to_string();
    let mut args = vec![format!("--target={target}{level}")];
    if let Ok(ndk) = env::var("ANDROID_NDK_HOME") {
        for host in ["linux-x86_64", "darwin-x86_64", "windows-x86_64"] {
            let sysroot = format!("{ndk}/toolchains/llvm/prebuilt/{host}/sysroot");
            if std::path::Path::new(&sysroot).exists() {
                args.push(format!("--sysroot={sysroot}"));
                break;
            }
        }
    }
    args
}
"""


def main():
    dest = os.path.abspath(sys.argv[1])
    meta = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--manifest-path", "android/Cargo.toml"]))
    pkgs = [p for p in meta["packages"] if p["name"] == "raylib-sys"]
    if not pkgs:
        sys.exit("raylib-sys not in the dependency graph")
    src = os.path.dirname(pkgs[0]["manifest_path"])
    if os.path.exists(dest):
        shutil.rmtree(dest)
    shutil.copytree(src, dest)
    build = os.path.join(dest, "build.rs")
    text = open(build, encoding="utf-8").read()
    if OLD not in text:
        sys.exit(f"raylib-sys {pkgs[0]['version']}: expected build.rs line not found — check the patch")
    if text.count(OLD_BG) != 1:
        sys.exit("raylib-sys: bindgen anchor '.clang_arg(plat)' not found exactly once — check the patch")
    if text.count(OLD_X11) != 1:
        sys.exit("raylib-sys: X11 link line not found exactly once — check the patch")
    text = text.replace(OLD, NEW).replace(OLD_BG, NEW_BG).replace(OLD_X11, NEW_X11) + HELPER
    open(build, "w", encoding="utf-8").write(text)
    print(f"patched raylib-sys {pkgs[0]['version']} -> {dest}")


if __name__ == "__main__":
    main()
