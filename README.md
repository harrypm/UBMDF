# UBDMF — Universal Blackmagic Devices Flasher

Cross-platform CLI + GUI tool for flashing firmware to Blackmagic capture devices via the USB service port. The protocol was reverse-engineered from official `DesktopVideoUpdateTool` traffic captures and hardware-validated with byte-verified flashes.

## What it does
- Lists connected Blackmagic service-port devices
- Reads device identity, state, and flash geometry
- Flashes firmware bundles (plaintext capture-derived images)
- Dumps device info to JSON
- Manages a firmware index (flashable bundles + encrypted inventories)
- Downloads and inventories official Desktop Video installers from BMD's catalog
- Batch-harvests all firmware versions unattended

## CLI

Running the exe with **no arguments** opens the GUI in GUI builds (so double-clicking works); CLI-only builds print help.

```
ubdmf                                # no args → GUI (GUI builds)
ubdmf list                          # find service-port devices
ubdmf info                          # read device identity/state (read-only)
ubdmf dump                          # dump device info to JSON
ubdmf flash <manifest.json> --yes   # flash a firmware bundle
ubdmf verify <manifest.json>        # verify bundle hashes + device state
ubdmf reset                         # send 0x12 to clear lingering sessions
ubdmf index list                    # list all indexed firmware
ubdmf index show <id>               # show bundle details
ubdmf index rebuild                 # rescan firmware/ and rebuild index
ubdmf update check                 # list BMD catalog versions
ubdmf update fetch <version>       # download + extract + inventory a version
ubdmf update import <path>         # import a local installer zip
ubdmf gui                          # launch the GUI
ubdmf gui --flash <manifest> --yes  # auto-flash (for testing)
```

## GUI
```
ubdmf gui
```
- **Portrait layout** (540×800, Android-friendly)
- **Arimo Bold** font (Helvetica-compatible, bundled for cross-platform)
- **Dark/Light mode** toggle (Qt6-style flat theme)
- **Firmware tab**: indexed bundles list, detail panel, Flash button
- **Devices tab**: **live** device detection — a poller thread enumerates the USB bus every 2s (hotplug-safe); each entry shows product, serial, bus/address, port chain, speed, and the bound driver (**WinUSB = flashable**, otherwise Zadig swap needed)
- **Update tab**: BMD catalog info
- **Log tab**: the **integrated console** — raylib init/GL lines, app events, device-poll and flash progress, with mouse-wheel scrolling
- **Flash progress bar**: erase phase + write phase with % readout
- Flash runs on a **worker thread** with mpsc event channel
- **No console popup**: GUI builds use the Windows GUI subsystem — no CMD window appears when the exe is launched or double-clicked. CLI commands from a terminal still print normally (the parent console is re-attached when needed)

## Firmware Index
`firmware/index.json` tracks two kinds of entries:
- **flashable**: plaintext capture-derived bundles (manifest + components)
- **encrypted-inventory**: SHA-256 inventories of encrypted `data-rNN.bin` files from official installers

## Harvest (batch firmware collection)
```
pwsh -File tools/harvest.ps1                    # all versions
pwsh -File tools/harvest.ps1 -From 16.5 -To 12.8  # version range
pwsh -File tools/harvest.ps1 -NoZips             # discard zips after inventory
```
Self-elevates (one UAC prompt), downloads → extracts → inventories → indexes each version. Produces `harvest-report.json`. Local installer zips are imported when available.

## Platform Notes

### Windows
The service port needs WinUSB (via Zadig) for UBDMF/bmdflash access. Official DesktopVideoUpdateTool needs BMD's DesktopVideoUpdate driver. Swap between them with `ubdmf driver restore-bmd` / re-run Zadig.

### Linux
No driver needed. udev rule for access:
```
SUBSYSTEM=="usb", ATTRS{idVendor}=="1edb", ATTRS{idProduct}=="be13", MODE="0660", TAG+="uaccess"
```

### macOS
No driver install required (IOKit gives user-space access). Vendor-class devices may need `set_configuration` — handled automatically.

## Paths (CWD-independent)
The tool works from any working directory (double-click, shortcut, script):
- `firmware/` root: `UBDMF_FIRMWARE` env override → `./firmware` if present → walk up from the exe (`target/release` → repo root)
- Log file: `UBDMF_LOG` env override → `logs/ubdmf.log` beside the resolved firmware root
- GUI theme conf (`ubdmf-gui.conf`) lives beside the resolved firmware root too

## Protocol Summary
The service-port protocol uses vendor control transfers on EP0 plus bulk OUT on EP 0x01. Key commands:
- `0x31` begin session (must be terminated with `0x12` to avoid lingering sessions)
- `0x34` start erase (followed by `0x35` erase-context read)
- `0x37` region select: {LE u32 addr in 64K blocks, LE u32 block count}
- `0x39` write address: {BE u32 byte address} (must be followed by `0x35` latch)
- Bulk OUT: image data in ≤32 KiB chunks
- **ZLP terminator**: explicit zero-length packet after each component's final short packet
- `0x32` commit
- `0x12` reboot

Full protocol documentation: `docs/protocol.md`

## Building
The GUI is a default cargo feature; build without it for a headless CLI-only binary (smaller, no raylib).
```
cargo build --release                              # Windows GUI + CLI (raylib)
cargo build --release --no-default-features       # CLI-only (no raylib)
cargo test [--no-default-features]                # 11 tests, both configs
cargo clippy --release --all-targets [--no-default-features]
```

### Linux cross-compile from Windows (verified)
`ring` (TLS for the downloader) needs a C cross compiler. Use zig + cargo-zigbuild:
```
# one-time setup (no admin needed)
zig 0.15.2 → unzip to e.g. C:\tools\zig-x86_64-windows-0.15.2, add to PATH
cargo install cargo-zigbuild

# build (CLI-only; glibc 2.17 target for broad compatibility)
cargo zigbuild --release --no-default-features --target x86_64-unknown-linux-gnu
→ target/x86_64-unknown-linux-gnu/release/ubdmf  (64-bit x86-64 ELF)
```
Notes:
- Plain `cargo build --target x86_64-unknown-linux-gnu` fails: no `x86_64-linux-gnu-gcc` on PATH, and `zig cc` cannot parse the `--target=x86_64-unknown-linux-gnu` flag cc-rs injects — cargo-zigbuild's wrapper scripts handle the flag translation.
- If `CC_x86_64_unknown_linux_gnu` etc. are set in your shell, cargo-zigbuild will not override them — clear stale values first.
- Don't run the zigbuild command with the GUI feature; raylib needs a native Linux toolchain to cross-build.

## Driver development note
The on-disk `Firmware/data-rNN.bin` files in Desktop Video installers are **encrypted**; the official tool decrypts them in software. UBDMF works with **plaintext** capture-derived bundles. To obtain a plaintext bundle for a new version, run the official updater under USBPcap capture and use `tools/extract-from-capture.ps1`.

## License
Tool code: MIT. Firmware images are Blackmagic Design property — local archival and flashing of images obtained via official installers only.
