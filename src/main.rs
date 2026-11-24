//! UBMDF — Universal Blackmagic Devices Flasher.
//! Engine: reverse-engineered Blackmagic service-port protocol (docs/protocol.md),
//! hardware-validated and byte-verified against the official DesktopVideoUpdateTool.

// Windows GUI builds use the windows subsystem: no console window is allocated
// at all (the GUI's Log tab is the integrated console). CLI invocations launched
// from a terminal re-attach the parent console in main() so output still works.
#![cfg_attr(all(windows, feature = "gui"), windows_subsystem = "windows")]

#[cfg(feature = "bundled-firmware")]
mod bundled;
mod downloader;
mod extract;
#[cfg(feature = "gui")]
mod gui;
mod index;
mod logger;
mod manifest;
mod ops;
mod protocol;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use manifest::Manifest;
use ops::{load_specs, read_device_snapshot};
use protocol::FlashEvent;

#[derive(Parser)]
#[command(
    name = "ubmdf",
    version,
    about = "UBMDF - Universal Blackmagic Devices Flasher (service-port protocol, see docs/protocol.md)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List connected Blackmagic service-port devices (VID 1edb / PID be13)
    List,
    /// Read device identity, state and flash geometry (read-only; session closed cleanly)
    Info,
    /// Dump device identity/state to a JSON file (read-only; session closed cleanly)
    Dump {
        /// Output file (default: ubmdf-dump-<epoch-secs>.json in the cwd)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Flash an image bundle described by a manifest JSON
    Flash {
        manifest: PathBuf,
        /// Skip the interactive confirmation prompt
        #[arg(long)]
        yes: bool,
        /// Handshake and plan only; nothing is written to the device
        #[arg(long)]
        dry_run: bool,
    },
    /// Verify manifest component hashes and read the device state
    Verify {
        manifest: PathBuf,
    },
    /// Send the reboot command (0x12) to clear a lingering session
    Reset,
    /// Firmware index operations
    Index {
        #[command(subcommand)]
        cmd: IndexCmd,
    },
    /// Download/extract/update firmware from Blackmagic
    Update {
        #[command(subcommand)]
        cmd: UpdateCmd,
    },
    /// Launch the GUI (raylib, Qt6-style dark/light)
    Gui {
        /// Auto-start flashing this manifest (for automated testing)
        #[arg(long)]
        flash: Option<PathBuf>,
        /// Skip confirmation when used with --flash
        #[arg(long)]
        yes: bool,
        /// Open on this tab: firmware, devices, update or log
        #[arg(long)]
        tab: Option<String>,
        /// Pre-select the firmware entry with this Desktop Video version
        #[arg(long)]
        select: Option<String>,
    },
}

#[derive(Subcommand)]
enum UpdateCmd {
    /// List all Desktop Video versions available from Blackmagic
    Check,
    /// Download a specific version, sandbox-extract, inventory its firmware
    Fetch {
        version: String,
    },
    /// Import a local installer zip (extract + inventory + index)
    Import {
        path: PathBuf,
    },
}

#[derive(Subcommand)]
enum IndexCmd {
    /// List all firmware bundles in the index
    List,
    /// Show details for a specific bundle
    Show { id: String },
    /// Rescan firmware/ and rebuild the index
    Rebuild,
}

fn main() {
    // Windows GUI-subsystem builds start with no console. When launched from a
    // terminal with CLI arguments, re-attach the parent console FIRST (before
    // any output) so CLI output and the clap help still appear. GUI mode keeps
    // no console — the GUI's Log tab is the integrated readout, no CMD popup.
    #[cfg(all(windows, feature = "gui"))]
    {
        let gui_mode = std::env::args().len() == 1
            || std::env::args().nth(1).is_some_and(|a| a == "gui");
        if !gui_mode {
            attach_parent_console();
        }
    }
    logger::init();
    #[cfg(feature = "bundled-firmware")]
    if let Err(e) = bundled::install_missing() {
        log::warn!("bundled firmware not installed: {e:#}");
    }
    // Bare launch (no arguments) → open the GUI when it is compiled in.
    // Matches desktop expectations when the exe is double-clicked or run bare;
    // CLI-only builds keep the clap help-printing behavior.
    #[cfg(feature = "gui")]
    if std::env::args().len() == 1 {
        // No console exists in this mode — errors go to the file log only.
        if let Err(e) = gui::run_gui(gui::Opts::default()) {
            log::error!("ubmdf: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(e) = run(Cli::parse()) {
        log::error!("ubmdf: {e:#}");
        // stderr may not exist (GUI-subsystem run without console) — never panic.
        let _ = writeln!(std::io::stderr(), "ubmdf: error: {e:#}");
        std::process::exit(1);
    }
}

/// Windows GUI-subsystem process started from a terminal with CLI arguments:
/// make terminal output work exactly like a console-subsystem build.
/// Order matters:
/// 1. If stdio was inherited (terminal, pipe, or file redirection) it already
///    works — return WITHOUT attaching, because AttachConsole would clobber
///    the inherited handles and send output to the console instead of the pipe.
/// 2. Otherwise attach the parent's console (when one exists) and re-open only
///    the missing handles to CONOUT$.
#[cfg(all(windows, feature = "gui"))]
fn attach_parent_console() {
    use std::os::windows::ffi::OsStrExt as _;
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5; // (u32)(-11)
    const STD_ERROR_HANDLE: u32 = 0xFFFF_FFF4; // (u32)(-12)
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_WRITE: u32 = 0x2;
    const OPEN_EXISTING: u32 = 3;
    unsafe extern "system" {
        fn AttachConsole(dw_process_id: u32) -> i32;
        fn GetStdHandle(n_std_handle: u32) -> *mut core::ffi::c_void;
        fn SetStdHandle(n_std_handle: u32, handle: *mut core::ffi::c_void) -> i32;
        fn CreateFileW(
            lp_filename: *const u16,
            dw_desired_access: u32,
            dw_share_mode: u32,
            lp_security_attributes: *mut core::ffi::c_void,
            dw_creation_disposition: u32,
            dw_flags_and_attributes: u32,
            h_template_file: *mut core::ffi::c_void,
        ) -> *mut core::ffi::c_void;
    }
    unsafe {
        let have_out = !GetStdHandle(STD_OUTPUT_HANDLE).is_null();
        let have_err = !GetStdHandle(STD_ERROR_HANDLE).is_null();
        if have_out && have_err {
            return; // inherited stdio (terminal/pipe/file) — do not touch it
        }
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return; // no parent console (double-click etc.) — nothing to do
        }
        let reopen = |std_id: u32, name: &str| {
            if !GetStdHandle(std_id).is_null() {
                return; // inherited handle (console, pipe or file) — keep it
            }
            let wide: Vec<u16> = std::ffi::OsStr::new(name)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let h = CreateFileW(
                wide.as_ptr(),
                GENERIC_WRITE,
                FILE_SHARE_WRITE,
                core::ptr::null_mut(),
                OPEN_EXISTING,
                0,
                core::ptr::null_mut(),
            );
            if h as isize != -1 {
                SetStdHandle(std_id, h);
            }
        };
        reopen(STD_OUTPUT_HANDLE, "CONOUT$");
        reopen(STD_ERROR_HANDLE, "CONOUT$");
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::List => cmd_list(),
        Cmd::Info => cmd_info(),
        Cmd::Dump { out } => cmd_dump(out),
        Cmd::Flash {
            manifest,
            yes,
            dry_run,
        } => cmd_flash(&manifest, yes, dry_run),
        Cmd::Verify { manifest } => cmd_verify(&manifest),
        Cmd::Reset => cmd_reset(),
        Cmd::Index { cmd } => match cmd {
            IndexCmd::List => cmd_index_list(),
            IndexCmd::Show { id } => cmd_index_show(&id),
            IndexCmd::Rebuild => cmd_index_rebuild(),
        },
        Cmd::Update { cmd } => match cmd {
            UpdateCmd::Check => cmd_update_check(),
            UpdateCmd::Fetch { version } => cmd_update_fetch(&version),
            UpdateCmd::Import { path } => cmd_update_import(&path),
        },
        Cmd::Gui { flash, yes, tab, select } => {
            #[cfg(feature = "gui")]
            {
                // GUI mode may have no console — errors go to the file log only.
                if let Err(e) = gui::run_gui(gui::Opts { flash, yes, tab, select }) {
                    log::error!("ubmdf: {e:#}");
                    std::process::exit(1);
                }
                Ok(())
            }
            #[cfg(not(feature = "gui"))]
            {
                let _ = (flash, yes, tab, select); // silence unused warnings
                anyhow::bail!("GUI not compiled in. Build with --features gui to enable the raylib GUI. CLI-only build: cargo build --no-default-features")
            }
        }
    }
}

fn cmd_update_check() -> Result<()> {
    let catalog = downloader::fetch_catalog()?;
    let versions = downloader::desktop_video_versions(&catalog);
    let idx = index::Index::load()?;

    println!("Blackmagic Desktop Video versions available (Windows, non-SDK):");
    println!("{:10} {:8} {:10}", "VERSION", "HAVE", "DOWNLOAD ID");
    for v in &versions {
        let have = idx
            .bundles
            .iter()
            .any(|b| b.dv_version == v.version)
            || index::firmware_root()
                .join("installers")
                .join(format!("Blackmagic_Desktop_Video_Windows_{}.zip", v.version))
                .exists();
        println!(
            "{:10} {:8} {}",
            v.version,
            if have { "yes" } else { "" },
            v.download_id
        );
    }
    println!("\n{} version(s) in catalog", versions.len());
    log::info!("update check: {} DV versions in catalog", versions.len());
    Ok(())
}

fn cmd_update_fetch(version: &str) -> Result<()> {
    let catalog = downloader::fetch_catalog()?;
    let dv = downloader::find_version(&catalog, version)?;

    let firmware_dir = index::firmware_root();
    let installers_dir = firmware_dir.join("installers");
    std::fs::create_dir_all(&installers_dir)?;

    let zip_path = installers_dir.join(format!("Blackmagic_Desktop_Video_Windows_{version}.zip"));

    // Download
    println!("Downloading Desktop Video {version}...");
    log::info!("fetching DV {version} (id: {})", dv.download_id);
    let signed_url = downloader::get_signed_url(&dv.download_id)?;
    let pb = indicatif::ProgressBar::new_spinner();
    pb.set_style(
        indicatif::ProgressStyle::with_template("[{elapsed_precise}] downloading {msg}").unwrap(),
    );
    let bytes = downloader::download_zip(&signed_url, &zip_path, &mut |written, total| {
        pb.set_message(format!("{:.1} MB / {:.1} MB", written as f64 / 1e6, total as f64 / 1e6));
        pb.tick();
    })?;
    pb.finish_and_clear();
    println!("Downloaded {} bytes", bytes);

    // Extract + inventory
    println!("Extracting and inventorying firmware...");
    let result = extract::extract_installer(&zip_path, &installers_dir, version)?;
    println!(
        "Firmware inventory: {} files, {:.1} MB",
        result.file_count,
        result.total_bytes as f64 / 1e6
    );

    // Rebuild index
    let idx = index::Index::rebuild()?;
    println!("Index rebuilt: {} bundle(s)", idx.bundles.len());
    log::info!("DV {version} fetched and indexed ({count} firmware files)", count = result.file_count);
    Ok(())
}

fn cmd_update_import(path: &Path) -> Result<()> {
    let firmware_dir = index::firmware_root();
    let installers_dir = firmware_dir.join("installers");
    std::fs::create_dir_all(&installers_dir)?;

    // Copy the zip to the installers dir for provenance
    let zip_path = installers_dir.join(
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "imported.zip".to_string()),
    );
    if path != zip_path {
        std::fs::copy(path, &zip_path)?;
    }

    println!("Importing {}...", zip_path.display());
    let result = extract::import_local_zip(&zip_path, &installers_dir)?;
    println!(
        "Firmware inventory: {} files, {:.1} MB",
        result.file_count,
        result.total_bytes as f64 / 1e6
    );

    let idx = index::Index::rebuild()?;
    println!("Index rebuilt: {} bundle(s)", idx.bundles.len());
    log::info!("local zip imported and indexed");
    Ok(())
}

fn cmd_index_list() -> Result<()> {
    let idx = index::Index::load()?;
    if idx.bundles.is_empty() {
        println!("Index is empty. Run `ubmdf index rebuild` to scan firmware/.");
        return Ok(());
    }
    println!("{:40} {:10} {:20} {:8} NOTES", "ID", "VERSION", "DEVICE", "KIND");
    for b in &idx.bundles {
        println!(
            "{:40} {:10} {:20} {:8} {}",
            b.id,
            b.dv_version,
            b.device,
            b.kind,
            b.notes.as_deref().unwrap_or("")
        );
    }
    println!("\n{} bundle(s) in index", idx.bundles.len());
    Ok(())
}

fn cmd_index_show(id: &str) -> Result<()> {
    let idx = index::Index::load()?;
    let b = idx
        .find(id)
        .with_context(|| format!("bundle '{}' not found (run `ubmdf index list`)", id))?;
    println!("ID:         {}", b.id);
    println!("Device:     {}", b.device);
    println!("Version:    {}", b.dv_version);
    println!("Kind:       {}", b.kind);
    if let Some(m) = &b.manifest {
        println!("Manifest:   {}", m);
    }
    if let Some(i) = &b.inventory {
        println!("Inventory:  {}", i);
    }
    if let Some(z) = &b.installer_zip {
        println!("Installer:  {}", z);
    }
    for (i, c) in b.components.iter().enumerate() {
        println!(
            "  component {i}: {} ({} bytes, sha256 {}...)",
            c.file,
            c.bytes,
            &c.sha256[..16.min(c.sha256.len())]
        );
        if let Some(addr) = c.region_block_addr {
            println!("    region {} blocks {}, write addr {:#010x}",
                addr, c.block_count.unwrap_or(0), c.write_addr.unwrap_or(0));
        }
    }
    if let Some(n) = &b.notes {
        println!("Notes:      {}", n);
    }
    Ok(())
}

fn cmd_index_rebuild() -> Result<()> {
    let idx = index::Index::rebuild()?;
    println!("Index rebuilt: {} bundle(s)", idx.bundles.len());
    for b in &idx.bundles {
        println!("  {} ({})", b.id, b.kind);
    }
    log::info!("index rebuilt: {} bundles", idx.bundles.len());
    Ok(())
}

/// CLI wrapper: close the session (0x12) and print the outcome.
fn report_close(dev: &protocol::BmdDevice) {
    match ops::close_session(dev) {
        Ok(()) => println!("session closed cleanly (0x12)"),
        Err(e) => println!("warning: {e:#}"),
    }
}

fn cmd_list() -> Result<()> {
    let found = protocol::list_bmd()?;
    if found.is_empty() {
        println!("No Blackmagic service-port devices found (VID 1edb PID be13).");
        println!(
            "Hints: on Windows the device must be bound to WinUSB (Zadig; see README); \
             on Linux check udev permissions."
        );
        log::info!("device list: none found");
    } else {
        for f in &found {
            println!("Blackmagic service port: {f}");
            log::info!("device list: {f}");
        }
    }
    Ok(())
}

fn cmd_info() -> Result<()> {
    let mut dev = protocol::open()?;
    log::info!("device open for info");
    let snap = read_device_snapshot(&mut dev)?;

    println!("device nonce (0x10): {}", snap.nonce);
    println!("device id byte (0x10): {:#04x}", snap.id_byte);
    println!("state (0x30): {}", snap.state);
    match snap.geometry {
        Some(fields) => println!(
            "geometry (0x35): fields = {:?}, block size = {} bytes",
            fields,
            fields[3]
        ),
        None => println!("geometry (0x35): unavailable (requires active erase context)"),
    }
    match &snap.progress {
        Some(p) => println!(
            "progress (0x38): region={} expected={} done={} busy={}",
            p.region, p.expected, p.done, p.busy
        ),
        None => println!("progress (0x38): unavailable"),
    }

    report_close(&dev);
    Ok(())
}

fn cmd_dump(out: Option<PathBuf>) -> Result<()> {
    let path = ops::dump_device(out)?;
    println!("device dump written: {}", path.display());
    Ok(())
}

fn cmd_flash(manifest: &Path, yes: bool, dry_run: bool) -> Result<()> {
    let (m, specs) = load_specs(manifest)?;

    println!("Image: {}", m.image.name);
    println!("Source: {}", m.image.source);
    if let Some(n) = &m.image.notes {
        println!("Notes:  {n}");
    }
    let total: u64 = specs.iter().map(|s| s.data.len() as u64).sum();
    for (i, s) in specs.iter().enumerate() {
        println!(
            "  component {i}: {} bytes -> region {} ({} blocks), write addr {:#010x}",
            s.data.len(),
            s.region_block_addr,
            s.block_count,
            s.write_addr
        );
    }
    println!("Total image bytes: {total}");

    if dry_run {
        let dev = protocol::open()?;
        let (nonce, one) = protocol::read_id(&dev.iface)?;
        println!(
            "dry-run handshake OK: nonce={}, id byte={:#04x}",
            protocol::hex(&nonce),
            one
        );
        println!("dry run: no erase/write commands were sent.");
        log::info!("dry-run handshake OK ({}), no writes", m.image.name);
        return Ok(());
    }

    if !yes {
        print!(
            "\nThis will ERASE and REWRITE the firmware on the connected device.\n\
             The device must stay powered and connected for the whole write.\n\
             Type 'yes' to continue: "
        );
        std::io::stdout().flush()?;
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if line.trim() != "yes" {
            bail!("aborted by user");
        }
    }

    let mut dev = protocol::open()?;
    let (nonce, one) = protocol::read_id(&dev.iface)?;
    println!(
        "device open: nonce={}, id byte={:#04x}",
        protocol::hex(&nonce),
        one
    );
    log::info!("flash started: {} ({} bytes)", m.image.name, total);

    let total_len: u64 = specs.iter().map(|s| s.data.len() as u64).sum();
    let write_pb = ProgressBar::new(total_len);
    write_pb.set_style(
        ProgressStyle::with_template(
            "[{elapsed_precise}] {bar:40.cyan/blue} {bytes}/{total_bytes} ({bytes_per_sec})",
        )
        .unwrap()
        .progress_chars("=>-"),
    );
    let erase_pb = ProgressBar::new_spinner();
    erase_pb.set_style(
        ProgressStyle::with_template("[{elapsed_precise}] erasing region {msg}").unwrap(),
    );

    let mut current_component = usize::MAX;
    protocol::flash(&mut dev, &specs, |ev| match ev {
        FlashEvent::ErasePoll {
            region,
            done,
            expected,
        } => {
            erase_pb.set_message(format!("{region}: {done}/{expected} blocks"));
            erase_pb.tick();
        }
        FlashEvent::WriteChunk { component, bytes } => {
            if *component != current_component {
                current_component = *component;
                erase_pb.finish_and_clear();
                write_pb.set_position(0);
            }
            write_pb.inc(*bytes as u64);
        }
    })?;

    write_pb.finish_and_clear();
    log::info!("flash complete: {}", m.image.name);
    println!("Flash complete.");
    println!("Please unplug the USB cable from your device.");
    Ok(())
}

fn cmd_verify(manifest: &Path) -> Result<()> {
    let (m, base) = Manifest::load(manifest)?;
    let files = m.verify_files(&base)?;
    println!("Manifest: {}", m.image.name);
    for (c, p) in m.components.iter().zip(files) {
        println!("  OK {} ({} bytes, sha256 verified)", p.display(), c.bytes);
    }
    match protocol::open() {
        Ok(mut dev) => {
            let snap = read_device_snapshot(&mut dev)?;
            println!("Device state (0x30): {}", snap.state);
            if let Some(fields) = snap.geometry {
                println!("Device geometry (0x35): {:?} (block size {})", fields, fields[3]);
            }
            println!("Note: no image readback over the service port; hash + state checks only.");
            report_close(&dev);
        }
        Err(e) => {
            println!("Device not openable ({e:#}); file hashes verified only.");
        }
    }
    Ok(())
}

fn cmd_reset() -> Result<()> {
    let dev = protocol::open()?;
    let (nonce, _one) = protocol::read_id(&dev.iface)?;
    println!("device open: nonce={}", protocol::hex(&nonce));
    let state = protocol::read_state(&dev.iface)?;
    println!("state before reset (0x30): {}", protocol::hex(&state));
    println!("sending reboot (0x12)...");
    match protocol::reboot(&dev.iface) {
        Ok(()) => {
            println!("reboot accepted. The device may drop off the bus briefly; after a few seconds re-run `ubmdf info`.");
            log::info!("reset (0x12) accepted");
        }
        Err(e) => {
            println!("reboot refused ({e}). Session state unclear - power-cycle the unit if problems persist.");
            log::warn!("reset (0x12) refused: {e:#}");
        }
    }
    Ok(())
}
