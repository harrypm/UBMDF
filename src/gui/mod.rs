//! UBMDF GUI — Clay layout + raylib renderer (same pairing as MISRC GUI).
//!
//! - Resizable portrait window, Qt6-style flat theme, dark/light toggle.
//! - Firmware: scrollable index (flashable first, then newest → oldest),
//!   filter chips, detail card, Flash with confirm dialog.
//! - Devices: live USB poll (2 s), per-device Dump (JSON) / Reset (0x12).
//! - Update: BMD catalog check, per-version download + extract + index,
//!   index rebuild — all on worker threads with progress.
//! - Log: integrated console (raylib + app + worker lines), follow/copy/open.
//!
//! Every device/network operation runs on a worker thread and reports back
//! over an mpsc channel; the UI thread only lays out, draws and applies actions.
//! CLI: `ubmdf gui` or `ubmdf gui --flash <manifest> --yes` (automated testing).
mod model;
mod render;
mod theme;
mod view;

use std::cell::{Cell, UnsafeCell};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use clay_layout::Clay;
use raylib::prelude::*;

use crate::protocol::DeviceEntry;
use model::{Bundle, Filter};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Firmware,
    Devices,
    Update,
    Log,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Ok,
    Warn,
    Err,
}

pub enum Confirm {
    Flash { title: String, device: String, manifest: PathBuf, caution: Option<String> },
    Reset,
}

pub struct Job {
    pub title: String,
    pub phase: String,
    pub frac: Option<f32>,
    pub detail: String,
    /// Flashing must never be interrupted (window close is deferred).
    pub critical: bool,
}

pub struct CatalogRow {
    pub version: String,
    pub download_id: String,
    pub indexed: bool,
}

pub enum CatalogState {
    Idle,
    Loading,
    Ready(Vec<CatalogRow>),
    Failed(String),
}

pub enum Ev {
    Devices(Vec<DeviceEntry>),
    Phase(String),
    Progress { frac: f32, detail: String },
    Catalog(Result<Vec<crate::downloader::DesktopVideoVersion>, String>),
    JobDone { ok: bool, msg: String, reload_index: bool, dump: Option<PathBuf> },
}

#[derive(Clone)]
pub enum Action {
    Tab(Tab),
    ToggleTheme,
    Filter(Filter),
    Select(usize),
    AskFlash(usize),
    AskReset,
    ConfirmYes,
    ConfirmNo,
    Dump,
    OpenPath(PathBuf),
    CheckCatalog,
    Fetch { version: String, download_id: String },
    RebuildIndex,
    CopyLog,
    ToggleFollow,
}

pub struct App {
    pub tab: Tab,
    pub dark: bool,
    pub filter: Filter,
    pub bundles: Vec<Bundle>,
    pub index_err: Option<String>,
    pub selected: Option<usize>,
    pub devices: Vec<DeviceEntry>,
    pub polled: bool,
    pub status: String,
    pub level: Level,
    pub job: Option<Job>,
    pub confirm: Option<Confirm>,
    pub catalog: CatalogState,
    pub last_dump: Option<PathBuf>,
    pub log_follow: bool,
    pub log_lines: Vec<String>,
    pub close_pending: bool,
    conf_path: PathBuf,
}

/// Per-frame input snapshot handed to the view.
pub struct Input {
    pub pressed: bool,
    pub wheel_y: f32,
    pub w: f32,
    pub h: f32,
    /// Set by any hovered, enabled control → pointing-hand cursor.
    pub hand: Cell<bool>,
}

/// Per-frame string arena: Clay render commands borrow text until drawing is
/// finished, so formatted strings must outlive the layout scope. Boxed strs
/// never move when the Vec grows, and clearing needs `&mut`, so no reference
/// handed out by `s()` can outlive its storage.
#[derive(Default)]
pub struct Arena {
    items: UnsafeCell<Vec<Box<str>>>,
}

impl Arena {
    pub fn s(&self, v: String) -> &str {
        let b = v.into_boxed_str();
        let p: *const str = &*b;
        unsafe {
            (*self.items.get()).push(b);
            &*p
        }
    }
    pub fn clear(&mut self) {
        self.items.get_mut().clear();
    }
}

impl App {
    fn set_status(&mut self, level: Level, msg: impl Into<String>) {
        let msg = msg.into();
        match level {
            Level::Err => log::error!("{msg}"),
            Level::Warn => log::warn!("{msg}"),
            _ => log::info!("{msg}"),
        }
        self.status = msg;
        self.level = level;
    }

    fn reload_index(&mut self) {
        let (b, err) = model::load_bundles();
        self.bundles = b;
        self.index_err = err;
        self.selected = None;
        let versions: Vec<String> = self.bundles.iter().map(|b| b.version.clone()).collect();
        if let CatalogState::Ready(rows) = &mut self.catalog {
            for r in rows.iter_mut() {
                r.indexed = versions.contains(&r.version);
            }
        }
    }

    pub fn flash_block_reason(&self) -> Option<&'static str> {
        if self.job.is_some() {
            return Some("Another operation is running");
        }
        if self.devices.is_empty() {
            return Some("Connect a device's USB service port to flash");
        }
        if self.devices.len() > 1 {
            return Some("More than one device connected — leave only the target plugged in");
        }
        if let Some(drv) = &self.devices[0].driver
            && !drv.to_ascii_lowercase().contains("winusb")
        {
            return Some("Service port is not on WinUSB — swap the driver with Zadig first");
        }
        None
    }

    fn start_job(&mut self, title: &str, critical: bool) {
        self.job = Some(Job {
            title: title.to_string(),
            phase: "Starting…".into(),
            frac: None,
            detail: String::new(),
            critical,
        });
        self.set_status(Level::Info, format!("{title}…"));
    }

    fn handle(&mut self, ev: Ev) {
        match ev {
            Ev::Devices(list) => {
                self.devices = list;
                self.polled = true;
            }
            Ev::Phase(p) => {
                log::info!("{p}");
                if let Some(j) = &mut self.job {
                    j.phase = p;
                    j.frac = None;
                    j.detail.clear();
                }
            }
            Ev::Progress { frac, detail } => {
                if let Some(j) = &mut self.job {
                    j.frac = Some(frac.clamp(0.0, 1.0));
                    j.detail = detail;
                }
            }
            Ev::Catalog(res) => {
                self.job = None;
                match res {
                    Ok(list) => {
                        let rows: Vec<CatalogRow> = list
                            .into_iter()
                            .map(|v| CatalogRow {
                                indexed: self.bundles.iter().any(|b| b.version == v.version),
                                version: v.version,
                                download_id: v.download_id,
                            })
                            .collect();
                        let missing = rows.iter().filter(|r| !r.indexed).count();
                        self.set_status(
                            Level::Ok,
                            format!("Catalog: {} versions, {missing} not indexed", rows.len()),
                        );
                        self.catalog = CatalogState::Ready(rows);
                    }
                    Err(e) => {
                        self.set_status(Level::Err, format!("Catalog check failed: {e}"));
                        self.catalog = CatalogState::Failed(e);
                    }
                }
            }
            Ev::JobDone { ok, msg, reload_index, dump } => {
                self.job = None;
                if let Some(p) = dump {
                    self.last_dump = Some(p);
                }
                if reload_index {
                    self.reload_index();
                }
                self.set_status(if ok { Level::Ok } else { Level::Err }, msg);
            }
        }
    }

    fn apply(&mut self, a: Action, tx: &Sender<Ev>, rl: &mut RaylibHandle) {
        // While a dialog is open only its own buttons act.
        if self.confirm.is_some() && !matches!(a, Action::ConfirmYes | Action::ConfirmNo) {
            return;
        }
        match a {
            Action::Tab(t) => self.tab = t,
            Action::ToggleTheme => {
                self.dark = !self.dark;
                let _ = std::fs::write(&self.conf_path, if self.dark { "dark" } else { "light" });
            }
            Action::Filter(f) => {
                self.filter = f;
                self.selected = None;
            }
            Action::Select(i) => {
                self.selected = if self.selected == Some(i) { None } else { Some(i) };
            }
            Action::AskFlash(i) => {
                if let Some(reason) = self.flash_block_reason() {
                    self.set_status(Level::Warn, reason);
                } else if let Some(b) = self.bundles.get(i)
                    && let Some(m) = &b.manifest
                {
                    self.confirm = Some(Confirm::Flash {
                        title: b.title.clone(),
                        device: b.device.clone(),
                        manifest: m.clone(),
                        caution: if b.caution { b.notes.clone() } else { None },
                    });
                }
            }
            Action::AskReset => {
                if self.job.is_none() {
                    self.confirm = Some(Confirm::Reset);
                }
            }
            Action::ConfirmNo => self.confirm = None,
            Action::ConfirmYes => match self.confirm.take() {
                Some(Confirm::Flash { title, manifest, .. }) => {
                    if let Some(reason) = self.flash_block_reason() {
                        self.set_status(Level::Warn, reason);
                    } else {
                        self.start_job(&format!("Flashing {title}"), true);
                        spawn_flash(manifest, tx.clone());
                    }
                }
                Some(Confirm::Reset) => {
                    if self.job.is_none() {
                        self.start_job("Resetting device session", false);
                        spawn_reset(tx.clone());
                    }
                }
                None => {}
            },
            Action::Dump => {
                if self.job.is_none() {
                    self.start_job("Dumping device info", false);
                    spawn_dump(tx.clone());
                }
            }
            Action::OpenPath(p) => open_path(&p),
            Action::CheckCatalog => {
                if self.job.is_none() {
                    self.catalog = CatalogState::Loading;
                    self.start_job("Checking Blackmagic catalog", false);
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        let res = crate::downloader::fetch_catalog()
                            .map(|c| crate::downloader::desktop_video_versions(&c))
                            .map_err(|e| format!("{e:#}"));
                        let _ = tx.send(Ev::Catalog(res));
                    });
                }
            }
            Action::Fetch { version, download_id } => {
                if self.job.is_none() {
                    self.start_job(&format!("Fetching Desktop Video {version}"), false);
                    spawn_fetch(version, download_id, tx.clone());
                }
            }
            Action::RebuildIndex => {
                if self.job.is_none() {
                    self.start_job("Rebuilding firmware index", false);
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        let ev = match crate::index::Index::rebuild() {
                            Ok(i) => Ev::JobDone {
                                ok: true,
                                msg: format!("Index rebuilt: {} bundles", i.bundles.len()),
                                reload_index: true,
                                dump: None,
                            },
                            Err(e) => Ev::JobDone {
                                ok: false,
                                msg: {
                                    log::error!("index rebuild failed: {e:#}");
                                    format!("Index rebuild failed: {e}")
                                },
                                reload_index: false,
                                dump: None,
                            },
                        };
                        let _ = tx.send(ev);
                    });
                }
            }
            Action::CopyLog => {
                let text = crate::logger::snapshot().join("\n");
                match rl.set_clipboard_text(&text) {
                    Ok(()) => self.set_status(Level::Ok, "Log copied to clipboard"),
                    Err(e) => self.set_status(Level::Err, format!("Copy failed: {e}")),
                }
            }
            Action::ToggleFollow => self.log_follow = !self.log_follow,
        }
    }
}

// ── Workers ────────────────────────────────────────────────────────────────

fn spawn_flash(manifest: PathBuf, tx: Sender<Ev>) {
    std::thread::spawn(move || {
        let send = |e: Ev| {
            let _ = tx.send(e);
        };
        let fail = |msg: String| Ev::JobDone { ok: false, msg, reload_index: false, dump: None };
        send(Ev::Phase("Checking image hashes".into()));
        let (m, specs) = match crate::ops::load_specs(&manifest) {
            Ok(v) => v,
            Err(e) => {
                log::error!("flash aborted, image check: {e:#}");
                return send(fail(format!("Flash aborted: image check failed ({e})")));
            }
        };
        send(Ev::Phase("Opening device".into()));
        let mut dev = match crate::protocol::open() {
            Ok(d) => d,
            Err(e) => {
                log::error!("flash aborted, device open: {e:#}");
                return send(fail(format!("Flash aborted: device open failed ({e})")));
            }
        };
        let total: u64 = specs.iter().map(|s| s.data.len() as u64).sum();
        log::info!("flash started: {} ({total} bytes)", m.image.name);
        let mut written: u64 = 0;
        let mut phase_write = false;
        let sizes: Vec<u64> = specs.iter().map(|s| s.data.len() as u64).collect();
        let regions: Vec<u32> = specs.iter().map(|s| s.region_block_addr).collect();
        let res = crate::protocol::flash(&mut dev, &specs, |ev| match ev {
            crate::protocol::FlashEvent::Identity { .. } => {} // logged by protocol::flash
            crate::protocol::FlashEvent::ErasePoll { region, done, expected } => {
                // 0x38 `done` is the absolute block reached (region 192 x 30 ends at 222).
                let n = done.saturating_sub(*region).min(*expected);
                let frac = if *expected > 0 { n as f32 / *expected as f32 } else { 0.0 };
                send(Ev::Progress { frac, detail: format!("Erase {}: {n}/{expected} blocks", part(*region)) });
            }
            crate::protocol::FlashEvent::WriteChunk { component, bytes } => {
                if !phase_write {
                    phase_write = true;
                    send(Ev::Phase("Writing image".into()));
                }
                written += *bytes as u64;
                let before: u64 = sizes[..*component].iter().sum();
                let region = regions.get(*component).copied().unwrap_or(0);
                send(Ev::Progress {
                    frac: if total > 0 { written as f32 / total as f32 } else { 0.0 },
                    detail: format!(
                        "Write {}: {:.1} / {:.1} MB  (total {:.1} / {:.1} MB)",
                        part(region),
                        (written - before) as f64 / 1e6,
                        sizes[*component] as f64 / 1e6,
                        written as f64 / 1e6,
                        total as f64 / 1e6
                    ),
                });
            }
        });
        send(match res {
            Ok(()) => Ev::JobDone {
                ok: true,
                msg: format!(
                    "Flashed {} · not verified (no read-back) · unplug USB, then power-cycle",
                    short_image(&m.image.name)
                ),
                reload_index: false,
                dump: None,
            },
            Err(e) => {
                log::error!("flash failed: {e:#}");
                fail(format!("Flash failed: {e}"))
            }
        });
    });
}

fn spawn_dump(tx: Sender<Ev>) {
    std::thread::spawn(move || {
        let out = crate::ops::dumps_dir().join(format!("ubmdf-dump-{}.json", crate::ops::unix_now()));
        let ev = match crate::ops::dump_device(Some(out)) {
            Ok(p) => Ev::JobDone {
                ok: true,
                msg: format!(
                    "Dump saved: {}",
                    p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
                ),
                reload_index: false,
                dump: Some(p),
            },
            Err(e) => {
                log::error!("dump failed: {e:#}");
                Ev::JobDone { ok: false, msg: format!("Dump failed: {e}"), reload_index: false, dump: None }
            }
        };
        let _ = tx.send(ev);
    });
}

fn spawn_reset(tx: Sender<Ev>) {
    std::thread::spawn(move || {
        let res = (|| -> anyhow::Result<()> {
            let dev = crate::protocol::open()?;
            let (nonce, _) = crate::protocol::read_id(&dev.iface)?;
            log::info!("reset: device open, nonce={}", crate::protocol::hex(&nonce));
            crate::protocol::reboot(&dev.iface)
        })();
        let ev = match res {
            Ok(()) => Ev::JobDone {
                ok: true,
                msg: "Reset sent · the device re-enumerates in a few seconds".into(),
                reload_index: false,
                dump: None,
            },
            Err(e) => Ev::JobDone {
                ok: false,
                msg: {
                    log::error!("reset failed: {e:#}");
                    format!("Reset failed: {e} · power-cycle the unit")
                },
                reload_index: false,
                dump: None,
            },
        };
        let _ = tx.send(ev);
    });
}

fn spawn_fetch(version: String, download_id: String, tx: Sender<Ev>) {
    std::thread::spawn(move || {
        let send = |e: Ev| {
            let _ = tx.send(e);
        };
        let res = (|| -> anyhow::Result<String> {
            let installers = crate::index::firmware_root().join("installers");
            std::fs::create_dir_all(&installers)?;
            let zip = installers.join(format!("Blackmagic_Desktop_Video_Windows_{version}.zip"));
            send(Ev::Phase(format!("Requesting download link for {version}")));
            let url = crate::downloader::get_signed_url(&download_id)?;
            send(Ev::Phase(format!("Downloading Desktop Video {version}")));
            crate::downloader::download_zip(&url, &zip, &mut |w, t| {
                send(Ev::Progress {
                    frac: if t > 0 { w as f32 / t as f32 } else { 0.0 },
                    detail: format!("{:.1} / {:.1} MB", w as f64 / 1e6, t as f64 / 1e6),
                });
            })?;
            send(Ev::Phase("Extracting installer, inventorying firmware".into()));
            let r = crate::extract::extract_installer(&zip, &installers, &version)?;
            send(Ev::Phase("Rebuilding index".into()));
            crate::index::Index::rebuild()?;
            Ok(format!(
                "Desktop Video {version} indexed: {} files, {}",
                r.file_count,
                model::human_bytes(r.total_bytes)
            ))
        })();
        send(match res {
            Ok(msg) => Ev::JobDone { ok: true, msg, reload_index: true, dump: None },
            Err(e) => Ev::JobDone {
                ok: false,
                msg: {
                    log::error!("fetch {version} failed: {e:#}");
                    format!("Fetch {version} failed: {e}")
                },
                reload_index: true,
                dump: None,
            },
        });
    });
}

/// Show a file (selected in the file manager) or open a folder — per platform.
fn open_path(p: &Path) {
    let res = if cfg!(windows) {
        if p.is_file() {
            std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn()
        } else {
            std::process::Command::new("explorer").arg(p).spawn()
        }
    } else if cfg!(target_os = "macos") {
        if p.is_file() {
            std::process::Command::new("open").arg("-R").arg(p).spawn()
        } else {
            std::process::Command::new("open").arg(p).spawn()
        }
    } else {
        let target = if p.is_file() { p.parent().unwrap_or(p) } else { p };
        std::process::Command::new("xdg-open").arg(target).spawn()
    };
    match res {
        Ok(_) => log::info!("opened {}", p.display()),
        Err(e) => log::warn!("could not open {}: {e}", p.display()),
    }
}

// ── Main loop ──────────────────────────────────────────────────────────────

/// Launch options (`ubmdf gui --flash/--yes/--tab/--select`).
#[derive(Default)]
pub struct Opts {
    pub flash: Option<PathBuf>,
    pub yes: bool,
    pub tab: Option<String>,
    pub select: Option<String>,
    /// Start a Blackmagic catalog check on launch (Update tab), as the button does.
    pub check_catalog: bool,
}

pub fn run_gui(opts: Opts) -> anyhow::Result<()> {
    // log_to_rust(): raylib TraceLog → log facade → our logger (ring + file),
    // installed before InitWindow so raylib's init lines reach the Log tab.
    let (mut rl, thread) = raylib::init()
        .size(600, 860)
        .title("UBMDF - Universal Blackmagic Devices Flasher")
        .resizable()
        .msaa_4x()
        .log_to_rust()
        .build();
    rl.set_window_min_size(460, 560);
    rl.set_target_fps(60);
    rl.set_exit_key(None); // Esc must never close the app (mid-flash safety)
    // Window/taskbar icon — embedded so it works from any launch directory and
    // on every platform (temporary art: tools/make-icon.ps1 → assets/icon/).
    // Not on macOS: Cocoa windows have no icons (GLFW only warns, error 65548);
    // the Dock icon comes from the .app bundle's ubmdf.icns.
    #[cfg(not(target_os = "macos"))]
    match Image::load_image_from_mem(".png", include_bytes!("../../assets/icon/ubmdf.png")) {
        Ok(mut icon) => {
            icon.set_format(PixelFormat::PIXELFORMAT_UNCOMPRESSED_R8G8B8A8);
            rl.set_window_icon(&icon);
        }
        Err(e) => log::warn!("window icon not loaded: {e}"),
    }

    let fonts = render::Fonts::load(&mut rl, &thread);
    let mut clay = Clay::new((600.0, 860.0).into());
    let raw = fonts.raw();
    clay.set_measure_text_function(move |text, cfg| render::measure(&raw, text, cfg));

    let conf_path = crate::index::firmware_root()
        .parent()
        .map(|p| p.join("ubmdf-gui.conf"))
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("ubmdf-gui.conf"));
    // Carry the theme over from the pre-rename settings file (<= v1.0.0).
    let old_conf = conf_path.with_file_name("ubdmf-gui.conf");
    if !conf_path.exists() && old_conf.exists() {
        let _ = std::fs::rename(&old_conf, &conf_path);
    }
    let dark = std::fs::read_to_string(&conf_path).map(|s| s.trim() != "light").unwrap_or(true);

    let (tx, rx): (Sender<Ev>, Receiver<Ev>) = channel();
    let mut app = App {
        tab: Tab::Firmware,
        dark,
        filter: Filter::All,
        bundles: Vec::new(),
        index_err: None,
        selected: None,
        devices: Vec::new(),
        polled: false,
        status: "Ready".into(),
        level: Level::Info,
        job: None,
        confirm: None,
        catalog: CatalogState::Idle,
        last_dump: None,
        log_follow: true,
        log_lines: Vec::new(),
        close_pending: false,
        conf_path,
    };
    app.reload_index();
    log::info!("GUI: {} bundles loaded from {}", app.bundles.len(), crate::index::index_path().display());

    // Device poller: enumerate every 2 s, report only on change. Enumeration
    // opens no handles, so it never conflicts with a running flash worker.
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut last: Option<Vec<String>> = None; // None = never polled
            loop {
                let entries = crate::protocol::list_bmd_entries().unwrap_or_default();
                let sig: Vec<String> = entries.iter().map(|e| format!("{}|{}", e.bus_id, e.address)).collect();
                if last.as_ref() != Some(&sig) {
                    if entries.is_empty() {
                        log::info!("device poll: no service-port devices on the bus");
                    }
                    for e in &entries {
                        log::info!(
                            "device poll: {} (bus {} addr {})",
                            e.product.as_deref().unwrap_or("service port"),
                            e.bus_id,
                            e.address
                        );
                    }
                    last = Some(sig);
                    if tx.send(Ev::Devices(entries)).is_err() {
                        return; // GUI closed
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        });
    }

    match opts.tab.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("devices") => app.tab = Tab::Devices,
        Some("update") => app.tab = Tab::Update,
        Some("log") => app.tab = Tab::Log,
        _ => {}
    }
    if let Some(v) = &opts.select {
        // Prefer the flashable entry when a version exists as both kinds.
        app.selected = app.bundles.iter().position(|b| &b.version == v && b.kind == model::Kind::Flashable)
            .or_else(|| app.bundles.iter().position(|b| &b.version == v));
    }
    if opts.check_catalog {
        app.apply(Action::CheckCatalog, &tx, &mut rl);
    }
    if let Some(path) = opts.flash {
        if opts.yes {
            app.start_job(&format!("Flashing {}", path.display()), true);
            spawn_flash(path, tx.clone());
        } else {
            // Name the image from its manifest; fall back to the file path.
            let (title, caution) = match crate::manifest::Manifest::load(&path) {
                Ok((m, _)) => {
                    let caution = m.image.notes.filter(|n| n.contains("NOT yet flashed"));
                    (m.image.name, caution)
                }
                Err(_) => (path.display().to_string(), None),
            };
            app.confirm = Some(Confirm::Flash { title, device: "device".into(), manifest: path, caution });
        }
    }

    let mut arena = Arena::default();
    let mut hand_cursor = false;
    // Idle throttle: full 60 fps while the user interacts or a job runs,
    // 15 fps otherwise (live device/log updates still appear within ~70 ms).
    let mut last_active = std::time::Instant::now();
    let mut fast = true;
    loop {
        if rl.window_should_close() {
            let critical = app.job.as_ref().is_some_and(|j| j.critical);
            if !critical {
                break;
            }
            if !app.close_pending {
                app.close_pending = true;
                app.set_status(Level::Warn, "Flash in progress — the window will close when it finishes");
            }
        }
        while let Ok(ev) = rx.try_recv() {
            app.handle(ev);
        }
        if app.close_pending && app.job.is_none() {
            break;
        }
        if app.tab == Tab::Log {
            app.log_lines = tidy_log(crate::logger::tail(400));
        }

        let (w, h) = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);
        clay.set_layout_dimensions((w, h).into());
        let m = rl.get_mouse_position();
        clay.pointer_state((m.x, m.y).into(), rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT));
        let wheel = rl.get_mouse_wheel_move_v();
        let delta = rl.get_mouse_delta();
        if delta.x != 0.0
            || delta.y != 0.0
            || wheel.y != 0.0
            || rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT)
            || app.job.is_some()
            || rl.is_window_resized()
        {
            last_active = std::time::Instant::now();
        }
        let want_fast = last_active.elapsed() < std::time::Duration::from_millis(500);
        if want_fast != fast {
            fast = want_fast;
            rl.set_target_fps(if fast { 60 } else { 15 });
        }
        clay.update_scroll_containers(false, (wheel.x * 4.0, wheel.y * 4.0).into(), rl.get_frame_time());

        let input = Input {
            pressed: rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT),
            wheel_y: wheel.y,
            w,
            h,
            hand: Cell::new(false),
        };
        let th = if app.dark { theme::DARK } else { theme::LIGHT };
        let mut acts: Vec<Action> = Vec::new();
        {
            let mut d = rl.begin_drawing(&thread);
            d.clear_background(Color::new(th.bg.r as u8, th.bg.g as u8, th.bg.b as u8, 255));
            let mut ui = clay.begin::<(), ()>();
            view::build(&mut ui, &app, &arena, &th, &input, &mut acts);
            render::render(&mut d, &fonts, ui.end());
        }
        arena.clear();

        if input.hand.get() != hand_cursor {
            hand_cursor = input.hand.get();
            rl.set_mouse_cursor(if hand_cursor {
                MouseCursor::MOUSE_CURSOR_POINTING_HAND
            } else {
                MouseCursor::MOUSE_CURSOR_DEFAULT
            });
        }
        for a in acts {
            app.apply(a, &tx, &mut rl);
        }
    }
    Ok(())
}

/// Which image a region holds, for progress text: the FPGA bitstream lives at
/// block 0, the MicroBlaze program at block 192 (UltraStudio 4K Mini layout).
fn part(region: u32) -> &'static str {
    match region {
        0 => "A (FPGA)",
        192 => "B (CPU)",
        _ => "region",
    }
}

/// "UltraStudio 4K Mini - Desktop Video 16.5" -> "DV 16.5" for the status bar.
fn short_image(name: &str) -> String {
    let tail = name.rsplit(" - ").next().unwrap_or(name);
    tail.replace("Desktop Video ", "DV ")
}

/// Log tab readout: drop raylib's INFO chatter (texture/font loads, frame-timer
/// switches; its warnings and errors stay) and shorten the home folder to `~`.
/// The log file and "Copy" keep every line unchanged.
fn tidy_log(lines: Vec<String>) -> Vec<String> {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok().filter(|h| h.len() > 1);
    lines
        .into_iter()
        .filter(|l| !(l.contains(" INFO ") && l.contains("[raylib]")))
        .map(|l| match &home {
            Some(h) => l.replace(h.as_str(), "~"),
            None => l,
        })
        .collect()
}

#[cfg(test)]
mod status_text_tests {
    use super::*;

    #[test]
    fn short_image_names() {
        assert_eq!(short_image("UltraStudio 4K Mini - Desktop Video 16.5"), "DV 16.5");
        assert_eq!(short_image("custom"), "custom");
    }

    #[test]
    fn tidy_log_drops_raylib_info() {
        let out = tidy_log(vec![
            "[1.0] INFO  [raylib] TIMER: Target time per frame".into(),
            "[1.1] WARN  [raylib] GLFW: something".into(),
            "[1.2] INFO  flash started".into(),
        ]);
        assert_eq!(out.len(), 2);
        assert!(out[0].contains("WARN"));
    }

    #[test]
    fn parts() {
        assert_eq!(part(0), "A (FPGA)");
        assert_eq!(part(192), "B (CPU)");
    }
}
