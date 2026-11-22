//! UBDMF GUI — portrait 540×800, bundled Arimo Bold, working flash workflow.
//! Integrated console: raylib's TraceLog is bridged into the `log` facade
//! (`log_to_rust()`) so the Log tab shows raylib + app + device-poll lines —
//! no external console window is needed (windows-subsystem build).
//! Devices tab is live: a poller thread enumerates the USB bus every 2s.
//! CLI: `ubdmf gui` or `ubdmf gui --flash <manifest> --yes` (automated testing).
use raylib::prelude::*;
use std::sync::mpsc::{Receiver, Sender, channel};

fn c(r: u8, g: u8, b: u8, a: u8) -> Color { Color { r, g, b, a } }

const W: i32 = 540;     // wider for comfortable 3-column layout
const H: i32 = 800;
const SW: i32 = 110;    // sidebar
const SH: i32 = 36;     // status bar
const TH: i32 = 56;     // title bar
const PAD: i32 = 14;
const LH: i32 = 46;     // row height
// Bigger, clearer font sizes
const FT: f32 = 32.0;   // title
const FB: f32 = 24.0;   // body / list
const FS: f32 = 18.0;   // small / status
const FL: f32 = 22.0;   // tab labels

pub fn run_gui(auto_flash: Option<std::path::PathBuf>, auto_yes: bool) -> anyhow::Result<()> {
    // log_to_rust(): raylib TraceLog → log facade → our logger (ring + file).
    // Installed before InitWindow so raylib's own init lines land in the Log tab.
    let (mut rl, thread) = raylib::init()
        .size(W, H)
        .title("UBDMF")
        .log_to_rust()
        .build();
    rl.set_target_fps(60);

    // ── Load font: bundled Arimo (Helvetica-compatible, cross-platform) first,
    // resolved relative to the repo/exe root so it works from any launch CWD,
    // then Windows system fallbacks. ──
    let arimo = |name: &str| -> String {
        crate::index::firmware_root()
            .parent()
            .map(|p| p.join("assets/fonts").join(name))
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::PathBuf::from("assets/fonts").join(name))
            .to_string_lossy()
            .to_string()
    };
    let font = rl.load_font_ex(&thread, &arimo("Arimo-Bold.ttf"), 32, None)
        .or_else(|_| rl.load_font_ex(&thread, &arimo("Arimo-Regular.ttf"), 32, None))
        .or_else(|_| rl.load_font_ex(&thread, "C:\\Windows\\Fonts\\segoeuib.ttf", 32, None))
        .or_else(|_| rl.load_font_ex(&thread, "C:\\Windows\\Fonts\\arialbd.ttf", 32, None))
        .unwrap_or_else(|_| {
            rl.load_font_ex(&thread, "C:\\Windows\\Fonts\\segoeui.ttf", 32, None)
                .unwrap_or_else(|_| panic!("no font found — bundle Arimo in assets/fonts/"))
        });
    // Set bilinear filtering for smooth scaling
    // Note: bilinear filtering can be set on the font's texture if needed
    // font.texture.set_filter(&thread, TextureFilter::TEXTURE_FILTER_BILINEAR);

    // Theme colors
    let bg      = c(30, 30, 34, 255);
    let fg      = c(230, 230, 234, 255);
    let dim     = c(170, 170, 178, 255);
    let panel   = c(42, 42, 48, 255);
    let phov    = c(52, 52, 60, 255);
    let border  = c(64, 64, 70, 255);
    let accent  = c(60, 130, 230, 255);
    let ahov    = c(80, 150, 245, 255);
    let atext   = c(255, 255, 255, 255);
    let success = c(80, 200, 120, 255);
    let warn    = c(240, 180, 60, 255);
    let sel_bg  = c(50, 70, 110, 255);
    let sel_fg  = c(240, 240, 244, 255);

    let mut tab: usize = 0;
    // Theme conf lives beside the firmware root (repo root) so it survives
    // launches from any directory — exe dir, double-click, or repo root.
    let conf_path = crate::index::firmware_root()
        .parent()
        .map(|p| p.join("ubdmf-gui.conf"))
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::PathBuf::from("ubdmf-gui.conf"));
    let mut dark = std::fs::read_to_string(&conf_path)
        .map(|s| !s.trim().eq("light"))
        .unwrap_or(true);

    // Simple state
    let mut selected: Option<usize> = None;
    let mut status = String::from("Ready");
    let mut confirm: Option<String> = None;
    let mut log_scroll: f32 = 0.0;

    // Flash progress state (driven by worker thread events)
    #[derive(Default, Clone)]
    struct FlashState {
        active: bool,
        phase: String,          // "erasing region X" or "writing component N"
        progress: f32,         // 0.0–1.0
        total_bytes: u64,
        written_bytes: u64,
    }
    let mut flash_state = FlashState::default();

    // Worker event channel
    #[derive(Debug, Clone)]
    enum Ev {
        Log(String),
        /// Live device list from the 2s bus poller (replaces the whole list).
        Devices(Vec<crate::protocol::DeviceEntry>),
        Erase { region: u32, done: u32, expected: u32 },
        Write { component: usize, bytes: u64, total: u64 },
        Done,
        Fail(String),
    }
    let (ev_tx, ev_rx): (Sender<Ev>, Receiver<Ev>) = channel();
    let mut worker_busy = false;
    let mut flash_manifest: Option<String> = None; // set when Flash button clicked, used on confirm

    // Flash worker: runs in a thread, sends progress events through the channel
    let spawn_flash = |manifest_path: &str, tx: Sender<Ev>| {
        let manifest_path = manifest_path.to_string(); // own it for the thread
        std::thread::spawn(move || {
            let send = |e: Ev| { let _ = tx.send(e); };
            send(Ev::Log("Opening device...".into()));

            // Load manifest
            let manifest_text = match std::fs::read_to_string(&manifest_path) {
                Ok(t) => t,
                Err(e) => { send(Ev::Fail(format!("manifest read: {e}"))); return; }
            };
            let manifest: crate::manifest::Manifest = match serde_json::from_str(&manifest_text) {
                Ok(m) => m,
                Err(e) => { send(Ev::Fail(format!("manifest parse: {e}"))); return; }
            };

            // Resolve component file paths (relative to manifest's parent)
            let base = std::path::Path::new(&manifest_path)
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default();

            // Verify + load components
            let mut specs = Vec::new();
            for c in &manifest.components {
                let p = base.join(&c.file);
                let data = match std::fs::read(&p) {
                    Ok(d) => d,
                    Err(e) => { send(Ev::Fail(format!("component read {}: {e}", c.file))); return; }
                };
                specs.push(crate::protocol::ComponentSpec {
                    region_block_addr: c.region_block_addr,
                    block_count: c.block_count,
                    write_addr: c.write_addr,
                    data,
                });
            }

            // Open device
            let mut dev = match crate::protocol::open() {
                Ok(d) => d,
                Err(e) => { send(Ev::Fail(format!("device open: {e:#}"))); return; }
            };

            send(Ev::Log(format!("Flashing {}...", manifest.image.name)));

            // Calculate total bytes across all components
            let total_bytes: u64 = specs.iter().map(|s| s.data.len() as u64).sum();

            // Track cumulative bytes written
            let mut written: u64 = 0;
            let mut last_component = 0;

            // Run the flash with event callback
            let result = crate::protocol::flash(&mut dev, &specs, |ev| {
                match ev {
                    crate::protocol::FlashEvent::ErasePoll { region, done, expected } => {
                        send(Ev::Erase { region: *region, done: *done, expected: *expected });
                    }
                    crate::protocol::FlashEvent::WriteChunk { component, bytes } => {
                        if *component != last_component {
                            // New component — reset counter
                            last_component = *component;
                        }
                        written += *bytes as u64;
                        send(Ev::Write { component: *component, bytes: written, total: total_bytes });
                    }
                }
            });

            match result {
                Ok(()) => send(Ev::Done),
                Err(e) => send(Ev::Fail(format!("{e:#}"))),
            }
        })
    };
    // (version, device, kind, flashable, manifest_path)
    let mut bundles: Vec<(String, String, String, bool, Option<String>)> = Vec::new();
    // Live device list — driven by the poller thread below.
    let mut devices: Vec<crate::protocol::DeviceEntry> = Vec::new();

    // Device poller: enumerate the bus every 2s (hotplug-safe) and report only
    // when the device set changes. Enumeration opens no handles and claims
    // nothing, so it never conflicts with a running flash worker.
    {
        let tx = ev_tx.clone();
        std::thread::spawn(move || {
            // None = never polled: the first poll ALWAYS reports (including the
            // empty-bus case — a plain empty Vec would compare equal to an empty
            // signature and the initial state would never be logged or sent).
            let mut last_sig: Option<Vec<String>> = None;
            loop {
                let entries = crate::protocol::list_bmd_entries().unwrap_or_default();
                let sig: Vec<String> = entries
                    .iter()
                    .map(|e| format!("{}|{}", e.bus_id, e.address))
                    .collect();
                if last_sig.as_ref() != Some(&sig) {
                    if entries.is_empty() {
                        log::info!("device poll: no service-port devices on the bus");
                    } else {
                        for e in &entries {
                            log::info!(
                                "device poll: {} (bus {} addr {})",
                                e.product.as_deref().unwrap_or("service port"),
                                e.bus_id,
                                e.address
                            );
                        }
                    }
                    last_sig = Some(sig);
                    let _ = tx.send(Ev::Devices(entries));
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        });
    }

    // Load data from index (path resolved via index::firmware_root — CWD independent)
    if let Ok(text) = std::fs::read_to_string(crate::index::index_path())
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(arr) = v["bundles"].as_array()
    {
        for b in arr {
            let ver = b["dv_version"].as_str().unwrap_or("?").to_string();
            let dev = b["device"].as_str().unwrap_or("?").to_string();
            let kind = b["kind"].as_str().unwrap_or("?").to_string();
            let flash = kind == "flashable";
            let manifest = b["manifest"].as_str().map(|s| s.to_string());
            bundles.push((ver, dev, kind, flash, manifest));
        }
    }
    // Auto-flash for CLI testing: `ubdmf gui --flash <manifest> --yes`
    if let Some(path) = auto_flash {
        let path_str = path.to_string_lossy().to_string();
        if auto_yes {
            worker_busy = true;
            status = String::from("Auto-flash started");
            spawn_flash(&path_str, ev_tx.clone());
        } else {
            confirm = Some(format!("Flash {}?", path.display()));
            flash_manifest = Some(path_str);
        }
    }

    // Helper: draw text with font at integer position
    macro_rules! txt {
        ($d:expr, $t:expr, $x:expr, $y:expr, $s:expr, $col:expr) => {
            $d.draw_text_ex(&font, $t, Vector2 { x: $x as f32, y: $y as f32 }, $s, 1.0, $col)
        };
    }

    while !rl.window_should_close() {
        let mouse = rl.get_mouse_position();
        let clicked = rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT);
        let wheel = rl.get_mouse_wheel_move();

        // Poll worker + device-poller events
        while let Ok(ev) = ev_rx.try_recv() {
            match ev {
                Ev::Log(msg) => { status = msg; }
                Ev::Devices(list) => { devices = list; }
                Ev::Erase { region, done, expected } => {
                    flash_state.active = true;
                    flash_state.phase = format!("Erasing region {}", region);
                    flash_state.progress = if expected > 0 { done as f32 / expected as f32 } else { 0.0 };
                }
                Ev::Write { component, bytes, total } => {
                    flash_state.active = true;
                    flash_state.phase = format!("Writing component {}", component);
                    flash_state.written_bytes = bytes;
                    if total > 0 { flash_state.total_bytes = total; }
                    if flash_state.total_bytes > 0 {
                        flash_state.progress = bytes as f32 / flash_state.total_bytes as f32;
                    }
                }
                Ev::Done => {
                    flash_state.active = false;
                    worker_busy = false;
                    status = String::from("Flash complete — unplug USB");
                }
                Ev::Fail(e) => {
                    flash_state.active = false;
                    worker_busy = false;
                    status = format!("Flash error: {}", e);
                }
            }
        }

        let mut d = rl.begin_drawing(&thread);

        // Theme switch
        let (bg, fg, dim, panel, phov, border, accent, ahov, atext, success, warn, sel_bg, sel_fg) = if dark {
            (bg, fg, dim, panel, phov, border, accent, ahov, atext, success, warn, sel_bg, sel_fg)
        } else {
            (c(245,245,248,255), c(30,30,36,255), c(120,120,130,255),
             c(255,255,255,255), c(235,235,240,255), c(200,200,208,255),
             c(30,100,200,255), c(50,120,220,255), c(255,255,255,255),
             c(40,160,80,255), c(200,140,20,255), c(200,215,240,255), c(20,20,30,255))
        };

        d.clear_background(bg);

        // ── Title bar ──
        d.draw_rectangle(0, 0, W, TH, panel);
        txt!(d, "UBDMF", PAD, 12, FT, fg);
        let tl = if dark { "D" } else { "L" };
        let tw = 40;
        let tx = W - tw - PAD;
        let thv = (mouse.x as i32) >= tx && (mouse.x as i32) < tx + tw && (mouse.y as i32) < TH;
        d.draw_rectangle(tx, 6, tw, TH - 12, if thv { phov } else { border });
        txt!(d, tl, tx + 12, 12, FB, fg);
        if thv && clicked {
            dark = !dark;
            let _ = std::fs::write(&conf_path, if dark { "dark" } else { "light" });
        }

        // ── Sidebar (vertical tabs) ──
        let tabs = ["Firmware", "Devices", "Update", "Log"];
        d.draw_rectangle(0, TH, SW, H - TH - SH, panel);
        d.draw_line(SW, TH, SW, H - SH, border);

        for (i, name) in tabs.iter().enumerate() {
            let tab_h = 64;
            let ty = TH + i as i32 * tab_h;
            let act = tab == i;
            let hov = (mouse.x as i32) < SW && (mouse.y as i32) >= ty && (mouse.y as i32) < ty + tab_h;
            d.draw_rectangle(0, ty, SW, tab_h, if act { accent } else if hov { phov } else { panel });
            let tc = if act { atext } else { fg };
            let cy_tab = ty + (tab_h - FL as i32) / 2;
            txt!(d, name, 12, cy_tab, FL, tc);
            if hov && clicked { tab = i; }
        }

        // ── Content ──
        let cx = SW;
        let cw = W - SW;
        let cy = TH;
        let ch = H - TH - SH;

        match tab {
            0 => {
                // ── Firmware ──
                if bundles.is_empty() {
                    txt!(d, "No firmware in index.", cx + PAD, cy + PAD, FB, dim);
                    txt!(d, "Run: ubdmf index rebuild", cx + PAD, cy + PAD + LH, FS, dim);
                } else {
                    // Responsive 3-column layout:
                    //   VERSION (measured) | DEVICE (flex, truncated to fit) | KIND (measured, right-aligned)
                    // Column positions are computed from actual text widths — no fixed offsets, no overflow.
                    let mut ry = cy + LH;

                    // Compute column boundaries from the widest version and kind text
                    let mut ver_w = 0;
                    let mut kind_w = 0;
                    for (ver, _, kind, _, _) in bundles.iter() {
                        let vw = font.measure_text(ver, FB, 1.0).x as i32 + PAD * 2;
                        if vw > ver_w { ver_w = vw; }
                        let kw = font.measure_text(kind, FS, 1.0).x as i32 + PAD;
                        if kw > kind_w { kind_w = kw; }
                    }
                    // Device column: the flexible space between version and kind
                    let dev_x = cx + ver_w;
                    let dev_w = cw - ver_w - kind_w - PAD * 2;

                    // Header
                    d.draw_rectangle(cx, cy, cw, LH, panel);
                    txt!(d, "VERSION", cx + PAD, cy + 10, FS, dim);
                    txt!(d, "DEVICE", dev_x, cy + 10, FS, dim);
                    let khw = font.measure_text("KIND", FS, 1.0).x as i32;
                    txt!(d, "KIND", cx + cw - PAD - khw, cy + 10, FS, dim);

                    for (i, (ver, dev, kind, flash, _)) in bundles.iter().enumerate() {
                        if ry + LH > cy + ch - 100 { break; }
                        let sel = selected == Some(i);
                        let hov = (mouse.x as i32) >= cx && (mouse.x as i32) < cx + cw && (mouse.y as i32) >= ry && (mouse.y as i32) < ry + LH;
                        d.draw_rectangle(cx, ry, cw, LH, if sel { sel_bg } else if hov { phov } else if i % 2 == 0 { panel } else { bg });

                        // Vertically-centered text positions:
                        // FB (24px) text: (LH - 24) / 2 = 11px from top
                        // FS (18px) text: (LH - 18) / 2 = 14px from top
                        let v_c = ry + 11;   // version (24px) center
                        let s_c = ry + 14;   // device/kind (18px) center

                        // Col 1: Version (bold, left)
                        let tc = if sel { sel_fg } else { fg };
                        txt!(d, ver, cx + PAD, v_c, FB, tc);

                        // Col 2: Device (flex, centered)
                        let dc = if sel { sel_fg } else { dim };
                        if dev_w > 30 {
                            let full_w = font.measure_text(dev, FS, 1.0).x as i32;
                            if full_w <= dev_w {
                                txt!(d, dev, dev_x, s_c, FS, dc);
                            } else {
                                let mut lo = 0; let mut hi = dev.len();
                                let mut best = 0;
                                while lo <= hi {
                                    let mid = (lo + hi) / 2;
                                    let candidate = format!("{}…", &dev[..mid]);
                                    let w = font.measure_text(&candidate, FS, 1.0).x as i32;
                                    if w <= dev_w { best = mid; lo = mid + 1; } else { hi = mid - 1; }
                                }
                                let trunc = format!("{}…", &dev[..best]);
                                txt!(d, &trunc, dev_x, s_c, FS, dc);
                            }
                        }

                        // Col 3: Kind (colored, right-aligned)
                        let kc = if *flash { success } else { dim };
                        let kw2 = font.measure_text(kind, FS, 1.0).x as i32;
                        txt!(d, kind, cx + cw - PAD - kw2, s_c, FS, kc);

                        if hov && clicked { selected = Some(i); }
                        ry += LH;
                    }

                    // Detail panel (compact 96px, tight layout)
                    if let Some(idx) = selected
                        && let Some((ver, _dev, kind, flash, manifest)) = bundles.get(idx)
                    {
                        let dh = 96;
                        let dy = cy + ch - dh;
                        d.draw_rectangle(cx, dy, cw, dh, panel);
                        d.draw_line(cx, dy, cx + cw, dy, border);

                        // Version (big, left) + kind badge (right)
                        txt!(d, ver, cx + PAD, dy + 8, FB, fg);
                        let kw3 = font.measure_text(kind, FS, 1.0).x as i32;
                        txt!(d, kind, cx + cw - PAD - kw3, dy + 12, FS, if *flash { success } else { dim });

                        // Flash button (vertically centered, compact)
                        if *flash && !worker_busy {
                            let bw = 110; let bh = 44;
                            let bx = cx + cw - bw - PAD; let by = dy + 40;
                            let hov = (mouse.x as i32) >= bx && (mouse.x as i32) < bx + bw && (mouse.y as i32) >= by && (mouse.y as i32) < by + bh;
                            d.draw_rectangle(bx, by, bw, bh, if hov { ahov } else { accent });
                            let fw2 = font.measure_text("Flash", FB, 1.0).x as i32;
                            txt!(d, "Flash", bx + (bw - fw2) / 2, by + (bh - FB as i32) / 2, FB, atext);
                            if hov && clicked {
                                confirm = Some(format!("Flash {}?", ver));
                                flash_manifest = manifest.clone();
                            }
                        }
                    }

                    // Confirm modal
                    if let Some(msg) = confirm.clone() {
                        let mw = cw - 40; let mh = 160;
                        let mx = cx + 20; let my = cy + 60;
                        d.draw_rectangle(cx, cy, cw, ch, Color { r: 0, g: 0, b: 0, a: 170 });
                        d.draw_rectangle(mx, my, mw, mh, panel);
                        d.draw_rectangle_lines(mx, my, mw, mh, border);
                        txt!(d, &msg, mx + PAD, my + 14, FB, fg);
                        txt!(d, "ERASES + REWRITES firmware", mx + PAD, my + 44, FS, warn);

                        let bw = 80; let bh = 36;
                        let yx = mx + mw / 2 - bw - 8; let nx = mx + mw / 2 + 8;
                        let by = my + mh - bh - 12;
                        let yh = (mouse.x as i32) >= yx && (mouse.x as i32) < yx + bw && (mouse.y as i32) >= by && (mouse.y as i32) < by + bh;
                        let nh = (mouse.x as i32) >= nx && (mouse.x as i32) < nx + bw && (mouse.y as i32) >= by && (mouse.y as i32) < by + bh;
                        d.draw_rectangle(yx, by, bw, bh, if yh { ahov } else { accent });
                        txt!(d, "Flash", yx + 16, by + 8, FB, atext);
                        d.draw_rectangle(nx, by, bw, bh, if nh { phov } else { border });
                        txt!(d, "Cancel", nx + 12, by + 8, FB, fg);
                        if (yh || nh) && clicked {
                            confirm = None;
                            if yh
                                && !worker_busy
                                && let Some(mp) = flash_manifest.take()
                            {
                                // Resolve relative to the firmware root (CWD independent)
                                let full = crate::index::firmware_root().join(&mp);
                                let path = if full.exists() { full.to_string_lossy().to_string() } else { mp };
                                worker_busy = true;
                                status = String::from("Starting flash...");
                                spawn_flash(&path, ev_tx.clone());
                            }
                        }
                    }
                }
            }
            1 => {
                // ── Devices (live: bus polled every 2s by the poller thread) ──
                if devices.is_empty() {
                    txt!(d, "No service-port devices found", cx + PAD, cy + PAD, FB, warn);
                    txt!(d, "VID 1edb / PID be13 - bus polled every 2s", cx + PAD, cy + PAD + 32, FS, dim);
                    let hy = cy + PAD + LH * 2;
                    #[cfg(windows)]
                    {
                        txt!(d, "Windows: the service port must be bound to WinUSB", cx + PAD, hy, FS, dim);
                        txt!(d, "(Zadig). If it is bound to BMD's", cx + PAD, hy + 24, FS, dim);
                        txt!(d, "DesktopVideoUpdate driver it will not", cx + PAD, hy + 48, FS, dim);
                        txt!(d, "appear here - see README (drivers).", cx + PAD, hy + 72, FS, dim);
                    }
                    #[cfg(not(windows))]
                    {
                        txt!(d, "Linux: check the udev rule in the README.", cx + PAD, hy, FS, dim);
                    }
                } else {
                    let hdr = format!("Connected ({}):", devices.len());
                    txt!(d, &hdr, cx + PAD, cy + PAD, FB, fg);
                    for (i, e) in devices.iter().enumerate() {
                        let y = cy + PAD + LH + 64 * i as i32;
                        let name = e.product.clone().unwrap_or_else(|| String::from("Blackmagic service port"));
                        txt!(d, "\u{25CF}", cx + PAD, y, FB, success);
                        let bw2 = font.measure_text("\u{25CF}", FB, 1.0).x as i32;
                        txt!(d, &name, cx + PAD + bw2 + 10, y, FB, fg);

                        // Detail line: bus / address / port chain / speed / serial
                        let mut detail = format!("bus {}  addr {}", e.bus_id, e.address);
                        if !e.port_chain.is_empty() {
                            let chain = e.port_chain.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(".");
                            detail.push_str(&format!("  port {chain}"));
                        }
                        if let Some(sp) = &e.speed {
                            detail.push_str(&format!("  {sp}"));
                        }
                        if let Some(s) = &e.serial {
                            detail.push_str(&format!("  serial {s}"));
                        }
                        txt!(d, &detail, cx + PAD + 14, y + 32, FS, dim);

                        // Driver badge (Windows): WinUSB = flashable, else Zadig swap needed
                        #[cfg(windows)]
                        if let Some(drv) = &e.driver {
                            let winusb = drv.to_ascii_lowercase().contains("winusb");
                            let dc2 = if winusb { success } else { warn };
                            let dl = if winusb {
                                format!("driver: {drv} (flashable)")
                            } else {
                                format!("driver: {drv} (Zadig swap to flash)")
                            };
                            let dlw = font.measure_text(&dl, FS, 1.0).x as i32;
                            txt!(d, &dl, cx + cw - PAD - dlw, y, FS, dc2);
                        }
                    }
                }
                txt!(d, "Dump: ubdmf dump -> JSON (no image readback)", cx + PAD, cy + ch - LH, FS, dim);
            }
            2 => {
                // ── Update ──
                txt!(d, "BMD Catalog", cx + PAD, cy + PAD, FB, fg);
                txt!(d, "CLI: ubdmf update check", cx + PAD, cy + PAD + LH, FS, dim);
                let inv = bundles.iter().filter(|(_,_,_,f,_)| !f).count();
                let fls = bundles.iter().filter(|(_,_,_,f,_)| *f).count();
                let s = format!("Flashable: {} | Inventories: {}", fls, inv);
                txt!(d, &s, cx + PAD, cy + PAD + LH * 3, FB, fg);
                txt!(d, "Batch: pwsh tools/harvest.ps1", cx + PAD, cy + PAD + LH * 5, FS, accent);
            }
            3 => {
                // ── Log ──
                d.draw_rectangle(cx, cy, cw, ch, panel);
                let vis = (ch / LH) as usize;
                let lines = crate::logger::tail(200);
                let tot = lines.len();
                log_scroll += wheel * LH as f32;
                log_scroll = log_scroll.clamp(0.0, ((tot.saturating_sub(vis)) as f32 * LH as f32).max(0.0));
                let skip = (log_scroll / LH as f32) as usize;
                let mut ly = cy;
                let mc = ((cw - PAD * 2) / (FS as i32 / 2)) as usize;
                for line in lines.iter().skip(skip).take(vis) {
                    let lc = if line.contains("ERROR") || line.contains("error") { c(235,90,90,255) }
                             else if line.contains("WARN") { warn } else { dim };
                    let dp = if line.len() > mc { &line[..mc] } else { line.as_str() };
                    txt!(d, dp, cx + PAD, ly, FS, lc);
                    ly += LH;
                }
            }
            _ => {}
        }

        // ── Flash progress (3-line clean layout above status bar) ──
        if flash_state.active {
            let ph = 72;                           // progress area height
            let py = H - SH - ph;
            d.draw_rectangle(0, py, W, ph, panel);
            d.draw_line(0, py, W, py, border);

            let pct = (flash_state.progress * 100.0) as i32;

            // Line 1: phase text (left) + % (right-aligned, big)
            txt!(d, &flash_state.phase, PAD, py + 4, FS, fg);
            let pct_str = format!("{}%", pct);
            let pw = font.measure_text(&pct_str, FB, 1.0).x as i32;
            txt!(d, &pct_str, W - PAD - pw, py + 2, FB, accent);

            // Line 2: progress bar (16px, clean, full width minus padding)
            let bar_y = py + 28;
            let bar_h = 16;
            let bar_w = W - PAD * 2;
            d.draw_rectangle(PAD, bar_y, bar_w, bar_h, border);
            let fill_w = (bar_w as f32 * flash_state.progress.clamp(0.0, 1.0)) as i32;
            d.draw_rectangle(PAD, bar_y, fill_w, bar_h, accent);

            // Line 3: byte readout (left, below the bar)
            let info = if flash_state.total_bytes > 0 {
                format!("{:.1} / {:.1} MB", 
                    flash_state.written_bytes as f64 / 1e6,
                    flash_state.total_bytes as f64 / 1e6)
            } else {
                String::from("")
            };
            if !info.is_empty() {
                txt!(d, &info, PAD, py + 48, FS, dim);
            }
        }

        // ── Status bar ──
        let sy = H - SH;
        d.draw_rectangle(0, sy, W, SH, panel);
        d.draw_line(0, sy, W, sy, border);
        let sc = if status.contains("error") { c(235,90,90,255) } else { dim };
        let scy = sy + (SH - FS as i32) / 2;
        txt!(d, &status, PAD, scy, FS, sc);
        if !devices.is_empty() {
            let n = devices.len();
            let dl = format!("\u{25CF} {n} device{}", if n == 1 { "" } else { "s" });
            let dw = font.measure_text(&dl, FS, 1.0).x as i32;
            txt!(d, &dl, W - PAD - dw, scy, FS, success);
        }
    }
    Ok(())
}
