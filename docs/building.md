# Building and releases

The GUI is a default cargo feature. Build without it for a headless CLI-only binary (smaller, no raylib).

```
cargo build --release                              # GUI + CLI (raylib; needs cmake)
cargo build --release --no-default-features        # CLI only
cargo build --release --features bundled-firmware  # release build with every flashable bundle embedded
cargo test [--no-default-features]
cargo clippy --release --all-targets [--no-default-features]
```

`bundled-firmware` needs the real images, not LFS pointers:

```
git lfs pull --include="firmware/*/dv-*/**" --exclude="firmware/installers/**"
```

## Bundled firmware
Release builds carry every flashable bundle inside the binary: 43 UltraStudio 4K Mini versions over 4 distinct images, 13 MB packed.

On start-up:
- **Missing bundles** are unpacked, sha256-checked and indexed.
- **An installed bundle** keeps its images. Its manifest is refreshed only when the bundled one has the same images but newer metadata, such as corrected erase geometry.

## CI and downloads
GitHub Actions (`.github/workflows/build.yml`) builds and tests every push:

| Platform | Artifact |
|---|---|
| Windows | `ubmdf-windows-x64.exe` |
| Linux | `UBMDF-x86_64.AppImage` (built on Ubuntu 22.04) |
| macOS | `UBMDF-macos-universal.zip`: universal `UBMDF.app`, arm64 + x86_64; ad-hoc signed, not notarized, so right-click → Open on first launch |

Pushing a `v*` tag publishes all three as a GitHub release.

## Linux cross-compile from Windows (verified)
`ring` (TLS for the downloader) needs a C cross compiler. Use zig with cargo-zigbuild:

```
# one-time setup (no admin needed)
zig 0.15.2 → unzip to e.g. C:\tools\zig-x86_64-windows-0.15.2, add to PATH
cargo install cargo-zigbuild

# build (CLI-only; glibc 2.17 target for broad compatibility)
cargo zigbuild --release --no-default-features --target x86_64-unknown-linux-gnu
→ target/x86_64-unknown-linux-gnu/release/ubmdf  (64-bit x86-64 ELF)
```

- Plain `cargo build --target x86_64-unknown-linux-gnu` fails, for two reasons: there's no `x86_64-linux-gnu-gcc` on PATH, and `zig cc` can't parse the `--target=` flag that cc-rs injects. cargo-zigbuild's wrapper scripts translate it.
- Clear any stale `CC_x86_64_unknown_linux_gnu`-style variables first; cargo-zigbuild won't override them.
- Don't cross-build with the GUI feature, because raylib needs a native Linux toolchain.
