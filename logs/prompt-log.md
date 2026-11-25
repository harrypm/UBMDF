# UBDMF — Universal Blackmagic Devices Flasher: prompt log
- Project start: 2026-11-20 (plan 13a385f9, approved after user refinements)
- Continues the validated `bmdflash` engine (C:\Users\Harry\bmdflash) — see
  `Documents\AI Notebook\2026-11-19 BMD cross-platform flashing tool - prompt log.md`
  for the full protocol reverse-engineering history.

## Task input (user)
1. "Make this into a dedicated CLI & GUI tool with an UI listing all the firmware (pre-packed) and devices and there firmware add a dump and an flash option, add a update tap, I want an intergrated log system and terminal readout with status and progress bars for flashing 'UBDMF - Universal Blackmagic Devices Flasher' should be the tool and repo name. Then lets pull the latest 16.5 and also start pulling older versions down to 12.8.x and adding there firmware to the index"
2. Refinements: GUI must use **Clay like MISRC GUI** (confirmed: MISRC GUI = clay.h + raylib, C/CMake), light as possible, clean **Qt6-style aesthetic, simple dark/light mode**; **index all the BMD Software versions**; use an **auto extractor script/routine** for the harvest instead of per-version manual operations (token efficiency).

## Decisions locked at approval
- Repo/tool: UBDMF at C:\Users\Harry\UBDMF, Rust single binary, engine ported from bmdflash (byte-verified protocol: 0x31 begin/0x12 session lifecycle, 0x34→0x35 erase-context order, 0x39+0x35 write latch, arm-gated erase completion, ZLP terminator after each component).
- GUI stack: **clay (Rust bindings) + raylib (rust bindings)** — same library + renderer pair as MISRC GUI. Hand-rolled widget layer; Qt6-style flat theme, dark/light toggle (theme.rs). No egui/eframe.
- Index: UBDMF/firmware/index.json; entries {device, dv_version, kind: flashable|encrypted-inventory, path, components/inventory, sha256s, notes}; 12.9 flagged known-bad (UltraStudio 4K Mini). Catalog metadata (all non-SDK Desktop Video versions) cached from BMD's public API.
- Harvest: `ubdmf update harvest [--from|--to|--all] [--no-zips]` — one unattended command: download → Authenticode verify (CN=Blackmagic Design Pty Ltd) → unzip → msiexec /a sandbox extract → firmware sha256 inventory → index entry; harvest-report.json; failures skip-and-report. Default: all ~70 Windows non-SDK releases (9.5.3→16.5). Local zips imported (12.8.1, 15.0, 16.0).
- Verified facts: BMD catalog API (api/support/us/downloads.json + api/register/us/download/<id>, no registration) lists all versions with stable IDs (confirmed by live fetch this session); MSI admin extract proven on 16.0; dump has no image readback (protocol limitation) — dump = identity/state JSON, honestly.

## Execution log
- U1: cargo init (binary ubdmf), folders logs/docs/drivers/firmware created; protocol.rs + manifest.rs + tests + docs/protocol.md + drivers/ubdmf-winusb.inf ported from bmdflash (INF internals still say "bmdflyash/bmdflash" — cosmetic cleanup pending). New: logger.rs (file logs/ubdmf.log + ring buffer + terminal, idempotent init), main.rs (CLI: list/info/dump/flash/verify/reset; session-close 0x12 on info/dump/verify; dump → ubdmf-dump-<epoch>.json). Deps: nusb, anyhow, clap(derive), indicatif, log, serde(derive), serde_json, sha2.

## Execution log (continued)
- U1 (01:44): cargo init, protocol+manifest ported, logger (file+ring+terminal), CLI list/info/dump/flash/verify/reset. Fixed log crate alloc feature. Info-level filtering (nusb debug spam). CLI smoke: list finds UltraStudio 4K Mini. 6/6 tests, clippy clean.
- U2 (01:50): index.rs (load/save/scan/rebuild), 12.8.1 + 16.0 bundles imported, index rebuild + list + show working. Fixed 3 borrow-checker issues (temporary lifetime, moved value). 8/8 tests.
- U3 (02:00-02:41): downloader.rs (ureq 2.x, catalog fetch → signed URL → zip download), extract.rs (zip → msiexec /a → sha256 inventory). Wired update check/fetch/import. Fixed ureq 3→2 downgrade (API changes), zip 9.x name() Result, sha2 LowerHex. **16.5 proven end-to-end**: downloaded 568 MB, MSI extracted, but msiexec /a required ELEVATION (exit 1603 non-elevated, success elevated — 60 fw files, 251.9 MB). 11/11 tests.
- U5 (03:06): harvest.ps1 written (self-elevating, unattended, all ~70 versions). **LAUNCHED** at 03:06 — elevated process running, 16.4 downloading (16.5 skipped). Expected: hours, ~25-35 GB. Report → firmware/installers/harvest-report.json.

## Execution log (continued, 20-21 Nov)
- U4 (GUI): Multiple iterations fixing raylib-rs API issues. Root cause of ~75 cascading errors: **Rust 2024 edition parses `as i32 <` as generic type args** — all casts parenthesized. Fixed Color struct literal, ureq 2.x API, variable shadowing. Final working GUI: 540×800 portrait, Arimo Bold font (bundled), 4 tabs with working flash workflow (worker thread + mpsc channel + progress bars with % readout). Responsive 3-column firmware list with measured text widths.
- 16.5 firmware flash: Driver swap to BMD DesktopVideoUpdate, then ran the 16.5 updater (extracted from 16.5 zip). Tool found the device, flashed 0→100%, exit 0. Captures on all 3 hubs.
- **DEVICE RECOVERY MILESTONE**: After 16.5 flash + power-cycle, the unit's **Thunderbolt controller came alive** — detected on a 2018 Mac Mini's TB bus (40 Gb/s, firmware 68.3, link controller 1.45.0). However: front panel display blank, not recognized by Desktop Video Setup or vrecord. Likely needs DV 16.5 installed on the Mac Mini for the driver to recognize the device. The TB controller being alive means the main boot firmware IS running — this is a separate subsystem from the main FPGA capture firmware.
- U5 harvest restart (21 Nov 05:06): Relaunched tools/harvest.ps1 — 37 inventories (was 36), processing remaining ~41 versions.
- U6: README.md written covering CLI, GUI, protocol summary, harvest, platform notes, building, and driver development notes.

## Outcome (updated 22 Nov 07:50 — project status)
- **U1-U6 all complete.** Repo: 2 commits (8f378ed baseline, a3e84d2 legacy-layout). Harvest **COMPLETE**: 102/102 catalog versions inventoried, 0 failures — 102 inventory CSVs + 102 installer zips (9.5.3 → 16.5, ~32.6 GB), index holds 104 bundles (102 inventories + 2 flashable: 12.8.1, 16.0). 46 legacy entries (≤ 10.9.7, firmware-in-drivers layout) flagged in notes.
- Validation: GUI build ✓, CLI-only ✓, 11/11 tests both configs ✓, clippy clean ✓, Linux x86-64 ELF (zigbuild, glibc 2.17) ✓, PE GUI-subsystem no-popup + working terminal CLI output ✓.
- Tool behavior: bare `ubdmf.exe` opens the GUI; live device poll (2s, driver badge); integrated Log tab console (raylib bridge); CWD-independent paths (UBDMF_FIRMWARE/UBDMF_LOG overrides).
- Device (UltraStudio 4K Mini): TB controller alive after 16.5 flash (2018 Mac Mini, 40 Gb/s); front panel still blank; not in DV Setup/vrecord — needs DV 16.5 driver on the test Mac. BMD service = fallback.
- **Remaining backlog**: (1) extract 16.5 plaintext bundle from firmware/installers/flash165_USBPcap3.pcap → third flashable image; (2) install DV 16.5 on the Mac Mini and retest device recognition; (3) user GUI smoke-check (Devices tab with unit connected + Log tab) still pending per session rules.
- Restore points: Documents\AI Notebook\restore-points\UBDMF-restore-2026-11-22-{pre,post}-msifix.zip.

## Execution log (24 Nov 2026, "continue development and fix this messy GUI" — Clay port)
- Branch `gui-clay` (from master a057be4; local repo, no remote). User choices (AskUserQuestion): **port to Clay** (as originally locked) and list order **flashable first, then newest**.
- Hard-data diagnosis (screenshot of the old GUI via PrintWindow, scratchpad shot.ps1): only 12 of 104 bundles visible, no scrolling (16.5 + both flashable bundles unreachable); lexical version sort (10.10 before 10.2); `…`/`●` drawn as `?` (raylib LoadFontEx(NULL) = 95 ASCII glyphs only); device column truncated to "(all Desktop ?"; fixed 540×800 window; theme button "D"; Update tab static text; Dump only a CLI hint; GUI flash skipped the sha256 check the CLI does.
- **Clay binding**: crates.io `clay-layout` 0.4.0 (clay.h via cc). Its `raylib-renderer` feature pins raylib 5.5 → conflicts with raylib 6, so only the core is used and `src/gui/render.rs` is our own raylib-6 port of Clay's reference renderer (rect/rounded, border, text via ffi DrawTextEx/MeasureTextEx, scissor).
- New layout `src/gui/{mod,view,model,render,theme}.rs` (old `src/gui.rs` removed); shared `src/ops.rs` (snapshot/dump/close_session/load_specs) now used by both CLI and GUI.
- Features: resizable window (600×860, min 460×560); per-size font atlases (Arimo R/B 14/16/19/24 + Latin-1 + `… — →`); header + horizontal tabs with counts; firmware filter chips + Clay scroll list + detail card (stats from inventory CSV, legacy notes, absolute paths, Flash / Show manifest / Show inventory / Show installer); Flash gating with stated reasons (no device / >1 device / non-WinUSB driver / busy); confirm dialog (floating, pointer capture); Devices cards with Dump (→ `dumps/`) and Reset (confirm); Update tab Check catalog / per-version Download (download→extract→inventory→index with progress) / Rebuild index; Log tab follow (auto-stops on scroll-up) / Copy / Open file; progress panel; close deferred during a flash, Esc disabled; `ubdmf gui --tab/--select` options.
- Pitfalls hit: (1) Clay `clip` on an axis stops children shrinking on it → wrapped text in the detail card overflowed and pushed the badge off; removed clip from cards. (2) `with_styling` children only get `&Scope` (no nested `with`), so scroll offsets come from `scroll_container_data(id).scrollPosition` (previous frame) instead of `scroll_offset()`. (3) Clay multiplies scroll delta ×10 → wheel × 4. (4) Index manifest paths use `\` → normalised + `std::path::absolute` for cross-platform.
- Validation (hard data): screenshots of every state (firmware/select/legacy/devices/update/log/dialog/light) in scratchpad; cargo test 13/13 (GUI) + 11/11 (CLI-only); clippy --all-targets clean both configs; CLI-only build ✓; GUI build last (Arimo string check 2); `ubdmf index list` → 104 bundles. Idle CPU 8.9% → **2.3%** of one core after idle throttle (15 fps idle / 60 fps active), RAM 65 MB.
- **Not yet verified (needs user)**: clicking through tabs/chips/rows/buttons, theme toggle, Check catalog (network), Dump/Reset/Flash with the unit connected, log follow behaviour. Not committed — waiting for user sign-off.

## Execution log (22 Nov ~01:37-02:05, "firmwares are missing" — old-MSI layout + full harvest)
- User: continue, firmwares missing; save current progress.
- **Progress saved first**: first-ever git commit 8f378ed (whole project; .gitignore excludes firmware/ ~30GB, target/, runtime state) + restore zip `Documents\AI Notebook\restore-points\UBDMF-restore-2026-11-22-pre-msifix.zip`.
- Diagnosis (hard data from harvest-report.json): 21 Nov run finished 07:06 with **47/102 failed** — 46 × "no Firmware directory in extract" (versions ≤ 10.9.7 + boundary check) and 1 × download 502 (11.3, zip genuinely missing). 101 zips present → not a download problem.
- **MSI layout discovery** (tools/inspect-msi.ps1, elevated, log in tools/): DV 9.5.3 admin-extract has **NO Firmware directory at all** — 36 dirs, 49 payload files; layout is `Blackmagic Design\Blackmagic Desktop Video\` with Drivers/DriverUSB (.sys drivers = the firmware), .cpl panels, codecs, plugin folders. The separate `Firmware/` + data-*.bin model only exists from **10.9.10 onward** (10.9.10-10.9.12, 10.10, 10.11.x all inventoried fine). 11.3 = only a download failure; it has a normal Firmware dir.
- First inspect-msi.ps1 attempt died at parse (my nested backtick-quote `.Replace()` inside `$()` — parse error "Missing ')' in method call"); rewritten without nested quoting, parse-checked via `Parser::ParseFile` before elevating. Its elevated run hung on locked-MSI cleanup (Windows Installer held files) — killed leftover pwsh by CIM CommandLine match, cleaned %TEMP%\ubdmf-inspect-*.
- **Fixes**: harvest.ps1 — LEGACY fallback: no Firmware dir → sha256 the FULL sandbox payload (relative paths in CSV `file` column), discard sandbox, status `ok-legacy`. extract.rs — same fallback via `inventory_all_payload()` (+ collect_files helper), index unchanged in shape. index.rs — scan_inventories detects legacy CSVs (path separator in first data row) and flags notes "legacy layout (DV <= 10.9.7): firmware ships inside drivers; zip = authoritative copy".
- Validation: build ✓, 11/11 tests ✓, clippy clean ✓.
- **Rerun: COMPLETE — succeeded=47, skipped=55, failed=0. Totals: 102 inventory CSVs, 102 zips, index 104 bundles (102 inventories + 2 flashable 12.8.1/16.0), 46 legacy-flagged. Disk 41 GB free.** 11.3 re-downloaded (334 MB) and inventoried normally — confirming the boundary.
- Legacy per-version reality check from transcript: e.g. 10.6.7 = 125 files, 441 MB payload. Old-version "firmware" = drivers (decklink.sys et al); no data-*.bin to decrypt — matches the protocol-era history (service-port updater + encrypted bins appeared ~10.9.10+).

## Execution log (21 Nov ~06:38-06:50, live device list + integrated console)
- User request: GUI must list actually live detected devices (not the hardcoded "UltraStudio 4K Mini") and show an integrated console readout instead of the external CMD popup.
- **Live devices**:
  - protocol.rs: new `DeviceEntry` struct + `list_bmd_entries()` — structured enumeration (product/serial strings, bus_id, device_address, port_chain, speed, and on Windows the bound driver name). Opens no handles, claims nothing — safe alongside a running flash worker.
  - gui.rs: poller thread enumerates every 2s, reports only on device-set change via `Ev::Devices`; Devices tab reworked (live list, per-device detail line, WinUSB/BMD driver badge, empty-state hints with Zadig note); status bar shows real `● N devices`.
  - Bug found by launch-test evidence (zero poller log lines): initial `last_sig = Vec::new()` compares equal to the empty-bus signature, so the first "no devices" state was never reported. Fixed with `Option<Vec<String>>` sentinel (None = never polled). Re-test: `device poll: no service-port devices on the bus` now logs immediately (device not connected to THE-RIPPER — matches CLI `list`).
- **Integrated console / no CMD popup**:
  - Cargo.toml: raylib `features = ["log"]` → builder `.log_to_rust()` bridges ALL raylib TraceLog into the `log` facade (target `raylib`), landing in the ring buffer + file + Log tab. Verified: `[raylib] SHADER/RLGL/FONT/...` lines in the log file from a live launch.
  - logger.rs: stderr sink made non-panicking (`writeln!` result ignored — GUI-subsystem runs can have no console); raylib-target lines tagged `[raylib]` for readability.
  - main.rs: `#![cfg_attr(all(windows, feature="gui"), windows_subsystem = "windows")]` — GUI builds allocate NO console at all. CLI invocations call `attach_parent_console()`: if stdio was inherited (terminal/pipe/file) it is left untouched; only NULL handles get AttachConsole(parent) + CONOUT$ reopen.
  - **Pitfall found by hard-data testing**: calling AttachConsole unconditionally when the process inherited pipe handles CLOBBERS them — `ubdmf list` through the harness pipe produced zero output (exit 0). Fixed by checking GetStdHandle non-NULL BEFORE attaching; re-verified `--help`/`list` print correctly through pipes, and the PE subsystem byte is 2 (GUI).
  - Bundled font now resolved relative to the repo root (firmware_root().parent()) — a TEMP-CWD launch previously fell back to Segoe (`Arimo-Bold.ttf Failed to open`); re-test shows `[C:\Users\Harry\UBDMF\assets/fonts/Arimo-Bold.ttf] File loaded successfully`.
- Validation: GUI build ✓ (built last), CLI-only ✓, 11/11 tests both configs ✓, clippy clean both ✓, Linux zigbuild fresh (06:48:36Z) ✓, bare-exe launch from TEMP alive with title 'UBDMF' ✓, no console window possible (subsystem 2) ✓. README GUI section updated.
- **Harvest incident (found at 06:52)**: the 05:06 relaunched harvest (PID 10000) was dead with NO harvest-report.json (script only writes it at the end) and NO new zips since 20 Nov 03:21 — it inventoried exactly one version, then sat "alive" but stalled for ~85 min, then died. Root cause identified from the script source: **none of its Invoke-RestMethod/Invoke-WebRequest calls had -TimeoutSec** — a dead TCP connection hangs the batch forever — and all output went to its (now gone) elevated console, so the stall point was untraceable. Disk 63.9 GB free, no orphaned msiexec.
- **harvest.ps1 hardened**: Start-Transcript → tools/harvest-transcript.log (Append) for post-mortem visibility; -TimeoutSec 60 catalog / 120 register / 900 download; post-download existence check. Relaunched detached (self-elevated, one UAC) at ~06:55 — transcript confirms resume behavior: `[37/102] SKIP (inventory exists)` → `[38/102] DV 12.0 downloading...`. Note: catalog now lists 102 non-SDK Windows DV versions (not 78) — ~64 remain ≈ 20-35 GB; disk has headroom but is worth monitoring.
- User smoke-check of the Devices tab with the unit connected + Log tab still pending (per session rules).

## Execution log (21 Nov ~06:30, "exe does not run GUI when launched manually")
- User report: the rebuilt exe doesn't open the GUI on manual launch. Hard-data diagnosis:
  - Binary HAD the GUI compiled in (Arimo-Bold/gui panic strings present in the exe bytes, 6.2 MB, built 06:12Z) — not a stale CLI-only overwrite.
  - Root cause: bare `ubdmf.exe` (no args) → clap requires a subcommand → prints help and exits. Manual/double-click launch = instant console flash, no GUI. The GUI needed the `gui` subcommand.
  - Secondary bug waiting to bite: all paths were CWD-relative, so a double-clicked exe (CWD=target\release\) would show an empty firmware list and scatter logs.
- Fixes:
  - main.rs: no-args launch now opens the GUI directly when the `gui` feature is compiled in (CLI-only builds keep help-printing).
  - index.rs `firmware_root()`: `UBDMF_FIRMWARE` env → `./firmware` if present → walk up from the exe (covers target/release → repo root, up to 5 levels) → CWD-relative fallback. All index/manifest/log/theme-conf consumers now CWD-independent.
  - gui.rs: index load + flash-manifest resolution use `index::firmware_root()`; theme conf anchored beside it.
  - logger.rs: log file anchored beside the resolved firmware root (`UBDMF_LOG` still overrides).
- Verification (hard data): launched bare exe with CWD=%TEMP% — process alive 5s, MainWindowTitle='UBDMF', raylib 6.0/GFLW full init in stdout, NO stray logs\ in TEMP, repo log shows absolute path `C:\Users\Harry\UBDMF\logs\ubdmf.log`. Validation matrix re-run: GUI build ✓, CLI-only ✓, 11/11 tests both configs ✓, clippy clean both ✓, Linux zigbuild rebuilt (fresh 06:34:38Z) ✓. README updated (bare-run behavior + Paths section). **Pitfall for future rounds: `cargo build --release` and `--no-default-features` write the same target\release\ubdmf.exe — whichever ran last wins; always re-run the GUI build last and verify via the Arimo string check.**
- User smoke-check of the window contents still pending (per session rules).

## Execution log (21 Nov, clippy/GUI-optional/cross-compile round)
- **gui.rs brace fix**: the collapsed detail-panel let-chain (gui.rs:373) had a leftover second closing brace from the old nested if-let (error: unexpected closing delimiter at gui.rs:531). Removed; block re-indented. GUI build compiles.
- **downloader.rs**: test module still used pre-rename serde field names (`downloadId` etc.) in struct literals — updated to `download_id`/`release_id`/`download_title` (E0609/E0560).
- **src/bin/gui_test.rs deleted**: early raylib smoke-test binary compiled raylib unconditionally and broke `--no-default-features` builds; superseded by the real GUI.
- **zip crate trimmed** to `default-features = false, features = ["deflate"]` — drops zstd-sys/bzip2 C deps (BMD zips are deflate-only), leaving only `ring` needing a C cross compiler.
- **Linux cross-compile actually verified for the first time** (the earlier "passing" claim was never built — plain `cargo build --target x86_64-unknown-linux-gnu` fails: no cross gcc, no WSL, no package managers on the box):
  - zig 0.15.2 unzipped to `C:\Users\Harry\tools\zig-x86_64-windows-0.15.2` (also 0.14.1/0.17.1 downloaded while isolating the failure; 0.15.2 kept on PATH).
  - Root causes found by direct experiment: (1) zig cannot parse the 4-field rust triple cc-rs injects (`--target=x86_64-unknown-linux-gnu` → "unable to parse target query ... UnknownOperatingSystem" on zig 0.14/0.15/0.17 alike); (2) cc-rs always appends its own --target on top of any -target in CC, so manual `CC='zig cc -target x86_64-linux-gnu.2.17'` can never work.
  - Solution: `cargo install cargo-zigbuild` (0.24.1) + zig on PATH; `cargo zigbuild --release --no-default-features --target x86_64-unknown-linux-gnu` — its generated wrapper .bat handles the flag translation. **Pitfall**: stale `CC_x86_64_unknown_linux_gnu`/`AR_...`/`CARGO_TARGET_..._LINKER` env vars in the persistent pwsh session silently override zigbuild's wrappers (it only sets missing vars) — clear them first.
  - Output verified from raw header bytes: ELF magic 7F 45 4C 46, class 02 (64-bit), data 01 (LSB), e_machine 0x003E (x86-64), 3.7 MB, glibc 2.17 default target.
- **Validation matrix (all hard-verified)**: GUI build ✓, CLI-only build ✓, cargo test 11/11 (both configs) ✓, clippy --all-targets clean (both configs) ✓, Linux ELF ✓. README Building section rewritten with the real procedure.
- Harvest still running (PID 10000, elevated): 38 dv-* inventory dirs.

- 24 Nov follow-up: user: 'text scaling needs to be a bit bigger'. STYLES 14/16/19/24 -> 16/18/21/27 (small/body/H2/H1). Rebuilt; screenshots v4-* checked: no overflow in list, detail card, update card, dialog. GUI reopened for user.
- 24 Nov: user "good commit this" → committed e06a435 (branch gui-clay) + restore point `AI Notebook/restore-points/UBDMF-restore-2026-11-24-gui-clay-e06a435.{zip,md}`; `/dumps` added to .gitignore.
- 24 Nov: temporary icon (user: "ultrastudio with ebu bars on the screen, 1:1"). No Pillow on the box → `tools/make-icon.ps1` (System.Drawing, 1024² render → 512 PNG + PNG-in-ICO 16–256). Pitfall: PowerShell unrolled the function's byte[] → 125-byte ICO; fixed with `return , $bytes` + List[byte[]]. First render had buttons overlapping the knob → moved. Wired: window icon via `Image::load_image_from_mem(include_bytes!)` (all platforms); exe icon via build.rs + winresource (Windows target only, non-fatal). Verified: ExtractAssociatedIcon of the exe = new art; title bar crop shows the icon; clippy clean both, tests 13/11, GUI built last.

## Execution log (24 Nov 2026, "extract all the versions from 10.9.10 onwards")
- Hard data: 56 dv-*-extract dirs (≥ 10.9.10). 11.5 → 16.5 complete (file counts = inventories). **10.9.10 → 11.4.1 (14 versions) only had data-bdaa.bin (9.7 KB)**. 12.1 had 256 files = 46 correct + a leftover `sandbox\` from an msiexec lock (deleting it was blocked by the auto-mode classifier → left for user; the index only reads Firmware\).
- Diagnosis via MSI File table (WindowsInstaller COM, read-only) + 7-Zip: in 10.9.10–11.4.1 the images are PE RCDATA resources of **CheckVersionPCI.exe** (190 MB in 10.9.10 → 239 MB in 11.4.1). 11.5 drops it and ships Firmware\data-rNN.bin.
- **The RCDATA images are plaintext**: entropy 3.3–6.0 (vs ≈ 8.0 for data-r*.bin); 46–51 Xilinx bitstreams (AA995566 sync), 2–4 ELF (`7f454c46`, same family as the plaintext 12.8.1 component_B), 14 other binaries. Resources are unnamed (string table has no names) → device mapping still open.
- New `tools/extract-updater-firmware.ps1`: 7-Zip only (zip → MSI compound → CAB → exe → .rsrc), **no elevation, no msiexec**; skips data-r layouts and already-done versions; writes rc-<id>.bin + resources.csv + source-exe.txt; regenerates the inventory; report → firmware/installers/updater-extract-report.json; log → tools/updater-extract.log. Temp dirs cleaned (verified 0 left).
- Result: 14/14 OK, 0 failed (62–69 images, 179–226 MB per version, ≈ 2.8 GB total; C: 26.5 → 23.8 GB free).
- index.rs: detects resources.csv → note "updater-embedded layout …" (the separator-based legacy check no longer misfires on `CheckVersionPCI-rcdata/` paths). GUI: Kind::Plaintext → "Plaintext images" badge + Show images button.
- Verified: index rebuild = 104 bundles (14 updater-embedded, 46 legacy, 2 flashable); clippy clean both configs; tests 13/11; GUI built last; screenshot v6-plaintext (11.4.1 detail) ✓. Not committed yet.

## Execution log (24 Nov 2026, "commit and continue" — device mapping)
- Committed 021eb1e (updater extraction). 12.1 sandbox left (user didn't answer that question).
- **Hash hit**: `rc-202` in 11.3/11.4/11.4.1 is byte-identical (sha256) to the verified 4K Mini component_B (12.8.1 + 16.0).
- **IDCODE match** (new tools/bitstream_info.py): component_A 12.8.1 + 16.0 → IDCODE 03824093, sync@112. 11.3–11.4.1 `rc-200` has the same IDCODE, sync@112 and exact size 9,491,428 → only 153 bytes differ from 12.8.1 component_A (header bytes 4–7 = build stamp, rest scattered words).
- History: 4K Mini resources first appear in 10.11.4; 10.11.4–11.2 have a different pair (rc-200 2,161,946 B / rc-202 1,375,860 B) → "probable", not bundled (layout unproven). The images are identical within each group.
- Bundle `firmware/ultrastudio4kmini/dv-11.4.1` built with new tools/make_rcdata_bundle.py (geometry from 12.8.1, refuses if larger than the proven region / mismatched across --also versions / existing dir). `ubdmf verify` → both components sha256 OK.
- Manifest gets optional `image.notes` → index notes → GUI amber note + caution text in the Flash dialog; CLI flash prints Notes. "NOT yet flashed on hardware" stated everywhere.
- docs/firmware-map.md: 4K Mini map + IDCODE family table for 11.4.1.
- Verified: clippy clean both configs, tests 13/11, GUI built last, index 105 bundles (3 flashable), screenshots v7-bundle / v7-dialog ✓. **Not flashed** — needs user decision + unit on WinUSB.

## Execution log (24 Nov 2026, "add more flashable options there is still a lot more left")
- **Breakthrough: data-rNN.bin are NOT encrypted.** In 12.8.1 data-r101 I found a 15-byte periodic pattern (impossible under real encryption), and a raw scan found a zlib stream at offset 104. Container = 32-byte prefix + records [BD BD 00 01 header (0x48 USB / 0x30 PCIe) + zlib]. Header: uncompressed size @16, unknown checksum @20 (not crc32/adler/mpeg2/sum32 — integrity covered by zlib's Adler-32 + size), write addr @28, comp index @32, USB VID/PID @36/38.
- Proof: 12.8.1 + 16.0 data-r90.bin → sha256-exact component_A + component_B of the hardware captures (write 0x0 / 0xC00000 = the captured geometry).
- tools/datar.py (record walker by header length/next field): **2,151/2,151 data files in DV 11.5–16.5 parse, 0 failures, 0 size mismatches, 61 target devices** (USB 1edb:xxxx service ports + PCIe 0000:00xx).
- 4K Mini (be13) is in data-r90 in all 42 versions; only 2 builds: 11.5–12.8.1 = 12.8.1 capture, 12.9–16.5 = 16.0 capture. **12.9 (KNOWN-BAD) carries the same images as 16.0** → note updated (the warning stays).
- tools/make_datar_bundles.py: one bundle per version from its installer; geometry from the headers, must match the verified layout; identical images hardlinked (43 bundles = 33 MB). Dry run, then real run: 40 created, 12.8.1/16.0 confirmed identical (independent cross-check of the captures).
- Verified: `ubdmf verify` 43/43 OK; index 145 bundles (43 flashable; caution: 11.4.1 not-yet-flashed, 12.9 known-bad); clippy clean both; tests 13/11; GUI built last; screenshot v8-many ✓.
- Docs: README format section (the old "encrypted" claim corrected), docs/firmware-map.md all-builds table.
- Open: other devices (60 IDs) are now unpackable, but the flash code is hard-wired to PID be13 and the protocol is only proven on the 4K Mini.

## Execution log (24 Nov 2026, local binary + private repo + Actions; debug/sanity pass; v1.0.0)
- Fonts embedded (include_bytes + LoadFontFromMemory), so a lone .exe / AppImage / .app renders. Local binary: dist/ubdmf.exe (gitignored), launched from TEMP → 145 bundles via walk-up.
- Private repo https://github.com/harrypm/UBDMF (gh, harrypm), pushed master + gui-clay (default became gui-clay). Tracked files checked: no firmware/zips/pcaps (firmware/ gitignored).
- .github/workflows/build.yml: windows exe, linux AppImage (ubuntu-22.04, linuxdeploy), macOS universal .app (lipo arm64+x86_64, iconutil, ad-hoc codesign), tests + CLI smoke, release job on v* tags.
- Run 45321390987: windows ✓, **linux ✗ E0609 `driver` field is #[cfg(windows)]** but the GUI reads it unconditionally (macOS cancelled). Fixed: field always present, None off Windows.
- Sanity pass fixes: (1) firmware_root fallback → per-user data dir (%LOCALAPPDATA% / ~/Library/Application Support / XDG) instead of a CWD-relative ./firmware (.app CWD is "/", AppImage mount is read-only) — tested with sandboxed LOCALAPPDATA: log lands in …\UBDMF\logs; (2) AppImage walk-up starts from $APPIMAGE; (3) Update › Download disabled off Windows (msiexec) with reason; (4) version 1.0.0 shown in the GUI header (CARGO_PKG_VERSION); (5) stale "encrypted" doc comment in index.rs; README Downloads/CI + data-folder sections.
- Local: clippy clean both, tests 13/11, CLI then GUI build, `ubdmf --version` = 1.0.0. The 4K Mini is now connected (poll: UltraStudio 4K Mini, addr 26) — nothing was sent to it.

## Execution log (24 Nov 2026, bundle flashable firmware + Git LFS + Linux/macOS)
- User bug report (screenshot, unit connected): Devices card USB line overflowed the card (Windows bus id "PCIROOT(0)#PCI(0102)#…" has no spaces). Fix: address/port/speed on a wrapping line; bus id on its own line, '#' → " › " (breakable); added ‹› to the font glyphs (› first drew as '?'). Re-screenshotted with the real device ✓.
- CI run on e3cc970: windows ✓ linux ✓ (driver fix works), macOS never started: "macos-14-arm64-us-xl not supported". Probe branch (deleted after): macos-latest / macos-15 work → workflow pinned to macos-15.
- Sizes (sha256-deduped): flashable bundles 0.49 GB → 30 MB unique; 56 extract dirs 10.3 GB → 1.65 GB (490 unique); zips 27.6 GB (excluded). **User chose: everything in LFS (~1.7 GB, may use paid quota).**
- .gitattributes: firmware/**/*.bin + *.pcap → LFS. .gitignore: track firmware/ except zips, *-full/, sandbox/, *.msi. Staged 3,338 firmware files; all 3,162 bin/pcap blobs are 131–133-byte LFS pointers (verified via cat-file sizes).
- index.json manifest paths now always '/' (was '\' on Windows → broken on Linux/macOS). Rebuilt: 0 backslashes.
- Feature `bundled-firmware`: build.rs packs firmware/<device>/dv-*/ manifests + blobs/<sha256> (43 bundles, 4 images, 13.2 MB); refuses LFS pointers. src/bundled.rs installs missing bundles on start-up (sha256-checked, manifest written last) + rebuilds the index. Pitfalls: sha2 0.11 digest has no {:x}; zip 9 file_names() yields Result<Cow>.
- Tested: exe 21 MB; in-repo run = no-op (145 bundles); sandboxed LOCALAPPDATA first run → "installed 43 flashable bundles", 43/43 verify OK; second run installs nothing (idempotent). (My first second-run harness deadlocked on redirected pipes — fixed with async reads; not an app bug.)
- CI: each job fetches only the bundle images from LFS (cached .git/lfs keyed on the oid list) and builds/tests with --features bundled-firmware. README: Bundled firmware + Git LFS sections.

- 24 Nov: push 6d4377c (LFS 266 objects, 1.2 GB). CI 45325721040: windows/linux/macos all success; logs: 43 bundles / 4 images embedded on every platform, tests 13/13, smoke 'ubdmf 1.0.0', macOS fat binary x86_64+arm64, .app codesign valid. Tagged v1.0.0 → run 45326244579 success → release https://github.com/harrypm/UBDMF/releases/tag/v1.0.0 (UBDMF-macos-universal.zip 32 MB, ubdmf-windows-x64.exe 21 MB, UBDMF-x86_64.AppImage 16 MB). Downloaded release exe runs (--version 1.0.0) and contains the bundled archive.

## Execution log (24 Nov 2026, "1 & 2": hardlinked first-run install + master default)
- bundled.rs: the first copy of each image (by sha256) is written, later bundles get a hardlink (copy fallback on FAT/exFAT/cross-volume). Sandbox test: "43 bundles … (4 images written, 82 hardlinked)"; apparent 489 MB, actual on disk 30.4 MB (48 unique files by inode); verify 43/43 OK. clippy clean both configs, tests 13/13.
- Note: my v1.0.0 release check had already installed a non-linked copy (466 MB) into the real %LOCALAPPDATA%\UBDMF — left for the user to delete (re-created compactly on next run).

## Execution log (24 Nov 2026, rename UBDMF → UBMDF, v1.0.1)
- User: "fix repo name UBMDF 'Universal Blackmagic Devices Flasher'". Asked (letters differ from the original request): user confirmed **UBMDF**, scope **repo + everything**, then v1.0.1. Entries above this point use the old name (history kept as written).
- Renamed in 19 tracked files + git mv assets/icon/ubmdf.{ico,png}, drivers/ubmdf-winusb.inf: crate/binary `ubmdf`, GUI title/header, CI asset names (ubmdf-windows-x64.exe, UBMDF-x86_64.AppImage, UBMDF-macos-universal.zip, UBMDF.app, bundle id io.github.ubmdf), env vars UBMDF_FIRMWARE / UBMDF_LOG, per-user data folder UBMDF, ubmdf-gui.conf, logs/ubmdf.log, ubmdf-dump-*.json.
- Compatibility: UBDMF_FIRMWARE / UBDMF_LOG still honoured; an existing per-user UBDMF folder is renamed to UBMDF on first run; ubdmf-gui.conf carried over. Sandbox test: old folder migrated, log at …\UBMDF\logs\ubmdf.log, 43 bundles (4 written / 82 hardlinked), `ubmdf 1.0.0`.
- Version 1.0.1 (rename + hardlinked bundled install).
- v1.0.1 tagged on 0cc4ab2 (CI 45354220960 all green: ubmdf 1.0.1, 43 bundles, fat x86_64+arm64) → release run 45355222373.

## Execution log (24 Nov 2026, macOS, Mac mini 2018: unit stays dark after flashing; items 1-5)
- Context: user reports that after UBMDF 1.0.0 flashes (16.5 at 08:09, 12.8.1 at 08:16, 16.5 at 08:22, all "Flash complete") the unit's front panel stays black and it is absent on Thunderbolt. Earlier entries already record a blank panel after the OFFICIAL 16.5 updater.
- Findings: the flash165_USBPcap{1,2,3}.pcap captures contain no write. Hub 3 starts mid-erase (0x38 expected=145, done 93→138); hubs 1 and 2 have no 1edb traffic. component_B is a MicroBlaze ELF (front-panel UI) that runs inside the FPGA. component_A has WBSTAR=0 and TIMER=0 (no fallback image).
- User: "1-5 go ahead":
  1. Erase 145 blocks for component A in dv-12.9 … dv-16.5 (was 144; the official 16.5 updater erases 145). tools/make_datar_bundles.py now always erases the full captured region. 41 manifests + index.json patched in place: an `index rebuild` on this Mac would drop the installer_zip links, because the zips live only on the Windows machine. bundled.rs refreshes an installed manifest when the images are identical and still pass sha256, and never touches one whose images differ. Sandbox test: 145 written, a tampered bundle was skipped with a warning, the second run was a no-op.
  2. protocol::flash is now driven by `planned_ops()`, and every op and device answer is logged. The 0x10 id reads moved inside flash(): the GUI used to skip them while the CLI sent them, so both now send identical traffic. Completion messages say "transfers accepted, contents not verified".
  3. Docs: "hardware-verified capture" → "official-tool capture" for images; protocol.md gains sections on the 16.5 captures, what a complete flash proves, and the probe results; firmware-map.md notes erase size and boot status.
  4. `ubmdf replay-check <pcap> <manifest> [--device BUS:ADDR]` (src/replay.rs): USBPcap (249) and usbmon (220), LCS alignment against planned_ops, byte-exact bulk comparison, and the device-reported 0x38 erase size. On the real flash165_USBPcap3 it reports 1 matched op (the 0x38 polls, expected=145), and hubs 1/2 are rejected as having no BMD traffic.
  5. `ubmdf probe` (src/probe.rs): read-only vendor IN sweep with logging. Found that the device STALLs unless wLength is exact. A full sweep (bRequest 0-255 × 0xC0/0xC1 × 70 lengths, outside and inside a session) answers only 0x10, 0x11 (8 B 890bb734890bb734), 0x20 (2 B 0101), 0x30, 0x38, and 0x33 (2 B 0500, session only). 0x11/0x20/0x33 STALL for any non-zero wValue/wIndex. EP 0x82 times out. No read-back command found. The device answers `info` identically afterwards.
- Build/test: macOS x86_64 (cmake from the nix profile for raylib-sys), cargo test 17/17, clippy clean apart from the existing collapsible-if in gui/mod.rs:313. The bundled-firmware build was tested from a copy of the tree with the 4 LFS images, sha256-checked.
- Not done: no new flash of the unit with the changed bundles (user's call); not committed.

## Execution log (24 Nov 2026, macOS: official updater traced, UBMDF aligned)
- User: flashes give a backlit black screen and still no TB3; "I think something is missing for the flashing part".
- Built tools/usbtrace (IOUSBLib interposer → USBPcap pcap + text). Traced the official macOS DesktopVideoUpdateTool 16.5
  flashing the unit (an ad-hoc re-signed copy, since the hardened runtime blocks injection; re-signing only the exe gives
  "damaged"). Trace: logs/official-trace-2026-11-24/ (pcap gz, sha256 d0149049…).
- replay-check against UBMDF a143c67 → differences: (1) ClearPipeStallBothEnds on both pipes at open; (2) ID order C1 1 B,
  C0 16 B ×2; (3) 0x37 A = (0, **144**), so a143c67's 145 was wrong (the flash165_USBPcap3 `expected=145` doesn't match
  the official sequence); (4) after each ZLP a 0x30 busy-wait (010101 → 010000 ~105 ms) + an extra 0x30.
- Fixed: planned_ops/flash follow the trace exactly (WireOp::ClearHalt, WaitWriteDone), erase = ceil(size)
  (generator, 42 manifests, index, docs); false "done != expected" warning fixed (0x38 done = absolute end block).
  replay-check vs the official trace: 32/32 matched, only the post-reboot 0x10 read is extra. Tests 17/17.
- Outcome of the OFFICIAL flash on the unit: TB link training still times out after a power-cycle (10:10:10).
  The flash procedure is not what keeps Thunderbolt down.
- Button/screen test: 0x12 reboot power-cycles the main board (screen off/on); TB link attempts follow 0x31 sessions; all time out.

## Execution log (25 Nov 2026, macOS: GUI text tightened, README icon + screenshots)
- User: "tighten up bottom text readout info, add image examples to the README and icon at the top".
- Status bar: one short line per result (e.g. "Flashed DV 16.5 · not verified (no read-back) · unplug USB, then power-cycle",
  "Dump saved: <file>", "Reset sent · …"); full error chains go to the log only (log::error! in each worker).
- Progress: erase shows per-part blocks ("Erase B (CPU): 30/30") — was "222/30" and an instant 100% for B because 0x38 `done`
  is absolute; write shows the per-part MB plus the total.
- Firmware detail card: per-image line (A · FPGA · 9.4 MB · erase 144 blocks · @ 0x000000), short notes, paths shown with ~.
  Bundle notes shortened in the generator, 41 manifests and the index. Restored the 12.9 KNOWN-BAD note in the repo index.json
  (lost in a143c67's index patch; runtime rebuilds always had it).
- Log tab: raylib INFO chatter hidden (file + Copy keep everything), home folder shown as ~; macOS skips set_window_icon
  (GLFW 65548 "Regular windows do not have icons on macOS").
- New: `ubmdf gui --check-catalog`. README: icon at the top, 2×2 screenshot grid (docs/images, window-only captures via
  tools/screenshot/winid.c + screencapture -l). Tests 20/20, clippy clean apart from the old gui/mod.rs:313.
- User: "readme is missing snips and has too much info that should be in sub docs", "did you push?". The snips were missing
  on GitHub because nothing after d87a7c2 had been pushed. README cut from 241 to 54 lines: centred icon, title and tagline,
  screenshots, features, download, quick start, a docs table and the licence. Details moved to new docs/cli.md, gui.md,
  building.md, platforms.md, tracing.md and firmware-data.md (protocol.md and firmware-map.md already existed). Dropped a
  stale reference to a nonexistent `ubmdf driver restore-bmd`. "See README" hints in the code now point to docs/platforms.md.
- User: "tag a new release with all the changes" → version 1.0.2 (Cargo.toml + Cargo.lock), tag v1.0.2 on master.
