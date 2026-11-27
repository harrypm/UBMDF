"""Make a patched copy of raylib-sys for Android builds (used by CI).

raylib-sys 6.0.0's build.rs derives the Android API level from the last part
of the Rust target triple ("aarch64-linux-android" -> "android"), which hands
the NDK toolchain an invalid ANDROID_PLATFORM / CMAKE_SYSTEM_VERSION. This
copies the crate cargo resolved and reads the level from $ANDROID_PLATFORM
instead (default "android-26", matching minSdk in android/app/build.gradle.kts).

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
    open(build, "w", encoding="utf-8").write(text.replace(OLD, NEW))
    print(f"patched raylib-sys {pkgs[0]['version']} -> {dest}")


if __name__ == "__main__":
    main()
