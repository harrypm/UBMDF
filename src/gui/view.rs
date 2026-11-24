//! Clay layout for every screen. Pure function of (App, Input) → elements +
//! queued Actions; no state is mutated here (apply happens after drawing).
use clay_layout::elements::{FloatingAttachToElement, PointerCaptureMode};
use clay_layout::id::Id;
use clay_layout::layout::{Alignment, LayoutAlignmentX as AX, LayoutAlignmentY as AY, LayoutDirection, Padding, Sizing};
use clay_layout::text::{TextConfig, TextElementConfigWrapMode as Wrap};
use clay_layout::{ClayLayoutScope, Color, Declaration};

use super::model::{Filter, Kind};
use super::render::{self, BODY, BODY_B, H1, H2, SMALL, SMALL_B};
use super::theme::Theme;
use super::{Action, App, Arena, CatalogState, Confirm, Input, Level, Tab};

type Ui<'c, 'r> = ClayLayoutScope<'c, 'r, (), ()>;
type Decl<'r> = Declaration<'r, (), ()>;

pub struct Cx<'a> {
    pub th: &'a Theme,
    pub inp: &'a Input,
}

const GROW: Sizing = Sizing::Grow(0.0, f32::MAX);

fn decl<'r>() -> Decl<'r> {
    Declaration::new()
}

fn text<'c, 'r>(ui: &Ui<'c, 'r>, s: &'r str, font: u16, color: Color)
where
    'c: 'r,
{
    ui.text(
        s,
        TextConfig::new()
            .font_id(font)
            .font_size(render::style_px(font))
            .color(color)
            .wrap_mode(Wrap::None)
            .end(),
    );
}

fn wrapped<'c, 'r>(ui: &Ui<'c, 'r>, s: &'r str, font: u16, color: Color)
where
    'c: 'r,
{
    ui.text(
        s,
        TextConfig::new()
            .font_id(font)
            .font_size(render::style_px(font))
            .line_height(render::style_px(font) + 5)
            .color(color)
            .wrap_mode(Wrap::Words)
            .end(),
    );
}

fn spacer<'c, 'r>(ui: &mut Ui<'c, 'r>)
where
    'c: 'r,
{
    ui.with(decl().layout().width(GROW).end(), |_| {});
}

#[derive(Clone, Copy, PartialEq)]
enum Btn {
    Primary,
    Secondary,
    Danger,
}

/// Button: hover/click use last frame's layout (Clay pointer state).
#[allow(clippy::too_many_arguments)]
fn button<'c, 'r>(
    ui: &mut Ui<'c, 'r>,
    id: Id,
    label: &'r str,
    kind: Btn,
    enabled: bool,
    cx: &Cx,
    acts: &mut Vec<Action>,
    on: Action,
) where
    'c: 'r,
{
    let th = cx.th;
    let hov = enabled && ui.pointer_over(id);
    if hov {
        cx.inp.hand.set(true);
        if cx.inp.pressed {
            acts.push(on);
        }
    }
    let (bg, fg, border) = match (enabled, kind) {
        (false, _) => (th.disabled, th.text_dim, th.disabled),
        (true, Btn::Primary) => (if hov { th.accent_hover } else { th.accent }, th.on_accent, th.accent),
        (true, Btn::Danger) => (if hov { th.err } else { th.surface }, if hov { th.on_accent } else { th.err }, th.err),
        (true, Btn::Secondary) => (if hov { th.hover } else { th.surface }, th.text, th.border),
    };
    ui.with(
        decl()
            .id(id)
            .layout()
            .padding(Padding::new(14, 14, 8, 8))
            .child_alignment(Alignment::new(AX::Center, AY::Center))
            .end()
            .background_color(bg)
            .corner_radius()
            .all(6.0)
            .end()
            .border()
            .all_directions(1)
            .color(border)
            .end(),
        |ui| text(ui, label, BODY_B, fg),
    );
}

fn badge<'c, 'r>(ui: &mut Ui<'c, 'r>, label: &'r str, fg: Color, bg: Color)
where
    'c: 'r,
{
    ui.with(
        decl()
            .layout()
            .padding(Padding::new(8, 8, 3, 3))
            .end()
            .background_color(bg)
            .corner_radius()
            .all(9.0)
            .end(),
        |ui| text(ui, label, SMALL_B, fg),
    );
}

fn kind_badge<'c, 'r>(ui: &mut Ui<'c, 'r>, k: Kind, th: &Theme)
where
    'c: 'r,
{
    let (fg, bg) = match k {
        Kind::Flashable => (th.ok, th.badge_ok_bg),
        Kind::Inventory => (th.text_dim, th.badge_dim_bg),
        Kind::Legacy => (th.warn, th.badge_warn_bg),
        Kind::Plaintext => (th.accent, th.badge_info_bg),
    };
    badge(ui, k.label(), fg, bg);
}

fn dot<'c, 'r>(ui: &mut Ui<'c, 'r>, color: Color)
where
    'c: 'r,
{
    ui.with(
        decl()
            .layout()
            .width(Sizing::Fixed(10.0))
            .height(Sizing::Fixed(10.0))
            .end()
            .background_color(color)
            .corner_radius()
            .all(5.0)
            .end(),
        |_| {},
    );
}

/// Bordered surface card, vertical stack.
fn card<'r>(th: &Theme, gap: u16) -> Decl<'r> {
    let mut d = decl();
    d.layout()
        .width(GROW)
        .direction(LayoutDirection::TopToBottom)
        .padding(Padding::all(14))
        .child_gap(gap)
        .end()
        .background_color(th.surface)
        .corner_radius()
        .all(8.0)
        .end()
        .border()
        .all_directions(1)
        .color(th.border)
        .end();
    d
}

fn row<'r>(gap: u16) -> Decl<'r> {
    let mut d = decl();
    d.layout()
        .width(GROW)
        .direction(LayoutDirection::LeftToRight)
        .child_gap(gap)
        .child_alignment(Alignment::new(AX::Left, AY::Center))
        .end();
    d
}

/// Vertical scroll container. Offset comes from Clay's scroll data of the
/// previous frame (Clay clamps it in update_scroll_containers).
fn scroll_offset(ui: &Ui, id: Id) -> clay_layout::math::Vector2 {
    match ui.scroll_container_data(id) {
        Some(sd) if !sd.scrollPosition.is_null() => unsafe {
            let p = *sd.scrollPosition;
            clay_layout::math::Vector2::new(p.x, p.y)
        },
        _ => clay_layout::math::Vector2::new(0.0, 0.0),
    }
}

fn scroll_box<'r>(th: &Theme, id: Id, offset: clay_layout::math::Vector2, gap: u16) -> Decl<'r> {
    let mut d = decl();
    d.id(id)
        .layout()
        .width(GROW)
        .height(GROW)
        .direction(LayoutDirection::TopToBottom)
        .child_gap(gap)
        .end()
        .clip(false, true, offset)
        .background_color(th.surface)
        .corner_radius()
        .all(8.0)
        .end()
        .border()
        .all_directions(1)
        .between_children(1)
        .color(th.border)
        .end();
    d
}

// ── Root ───────────────────────────────────────────────────────────────────

pub fn build<'c, 'r>(
    ui: &mut Ui<'c, 'r>,
    app: &'r App,
    ar: &'r Arena,
    th: &Theme,
    inp: &Input,
    acts: &mut Vec<Action>,
) where
    'c: 'r,
{
    let cx = Cx { th, inp };
    ui.with(
        decl()
            .id(ui.id("root"))
            .layout()
            .width(GROW)
            .height(GROW)
            .direction(LayoutDirection::TopToBottom)
            .end()
            .background_color(th.bg),
        |ui| {
            header(ui, app, &cx, acts);
            tabs(ui, app, ar, &cx, acts);
            ui.with(
                decl()
                    .layout()
                    .width(GROW)
                    .height(GROW)
                    .direction(LayoutDirection::TopToBottom)
                    .padding(Padding::all(12))
                    .child_gap(10)
                    .end(),
                |ui| match app.tab {
                    Tab::Firmware => firmware(ui, app, ar, &cx, acts),
                    Tab::Devices => devices(ui, app, ar, &cx, acts),
                    Tab::Update => update(ui, app, ar, &cx, acts),
                    Tab::Log => log_view(ui, app, ar, &cx, acts),
                },
            );
            progress(ui, app, ar, &cx);
            status_bar(ui, app, ar, &cx);
        },
    );
    if app.confirm.is_some() {
        dialog(ui, app, ar, &cx, acts);
    }
}

fn header<'c, 'r>(ui: &mut Ui<'c, 'r>, app: &'r App, cx: &Cx, acts: &mut Vec<Action>)
where
    'c: 'r,
{
    let th = cx.th;
    ui.with(
        row(12)
            .layout()
            .padding(Padding::new(16, 16, 12, 12))
            .end()
            .background_color(th.surface),
        |ui| {
            ui.with(
                decl().layout().direction(LayoutDirection::TopToBottom).child_gap(2).end(),
                |ui| {
                    text(ui, "UBDMF", H1, th.text);
                    text(ui, concat!("Universal Blackmagic Devices Flasher · v", env!("CARGO_PKG_VERSION")), SMALL, th.text_dim);
                },
            );
            spacer(ui);
            let label = if app.dark { "Light theme" } else { "Dark theme" };
            let id = ui.id("theme_btn");
            button(ui, id, label, Btn::Secondary, true, cx, acts, Action::ToggleTheme);
        },
    );
}

fn tabs<'c, 'r>(ui: &mut Ui<'c, 'r>, app: &'r App, ar: &'r Arena, cx: &Cx, acts: &mut Vec<Action>)
where
    'c: 'r,
{
    let th = cx.th;
    let list: [(Tab, &'r str); 4] = [
        (Tab::Firmware, ar.s(format!("Firmware  {}", app.bundles.len()))),
        (Tab::Devices, ar.s(format!("Devices  {}", app.devices.len()))),
        (Tab::Update, "Update"),
        (Tab::Log, "Log"),
    ];
    ui.with(
        row(0)
            .layout()
            .padding(Padding::new(8, 8, 0, 0))
            .end()
            .background_color(th.surface)
            .border()
            .bottom(1)
            .color(th.border)
            .end(),
        |ui| {
            for (i, (tab, label)) in list.into_iter().enumerate() {
                let id = ui.id_index("tab", i as u32);
                let active = app.tab == tab;
                let hov = ui.pointer_over(id);
                if hov && !active {
                    cx.inp.hand.set(true);
                    if cx.inp.pressed {
                        acts.push(Action::Tab(tab));
                    }
                }
                let mut d = decl();
                d.id(id)
                    .layout()
                    .padding(Padding::new(14, 14, 11, 9))
                    .end()
                    .background_color(if hov && !active { th.hover } else { th.surface })
                    .border()
                    .bottom(3)
                    .color(if active { th.accent } else { th.surface })
                    .end();
                ui.with(&d, |ui| {
                    text(ui, label, BODY_B, if active { th.accent } else { th.text_dim })
                });
            }
        },
    );
}

// ── Firmware ───────────────────────────────────────────────────────────────

fn firmware<'c, 'r>(ui: &mut Ui<'c, 'r>, app: &'r App, ar: &'r Arena, cx: &Cx, acts: &mut Vec<Action>)
where
    'c: 'r,
{
    let th = cx.th;
    if let Some(e) = &app.index_err {
        ui.with(&card(th, 6), |ui| {
            text(ui, "Firmware index could not be read", H2, th.err);
            wrapped(ui, e, SMALL, th.text_dim);
        });
    }
    if app.bundles.is_empty() {
        ui.with(&card(th, 8), |ui| {
            text(ui, "No firmware in the index", H2, th.text);
            wrapped(ui, "Rebuild the index from the Update tab, or run: ubdmf index rebuild", BODY, th.text_dim);
        });
        return;
    }
    let n_flash = app.bundles.iter().filter(|b| b.kind == Kind::Flashable).count();
    let n_inv = app.bundles.len() - n_flash;
    let visible: Vec<usize> = (0..app.bundles.len())
        .filter(|&i| match app.filter {
            Filter::All => true,
            Filter::Flashable => app.bundles[i].kind == Kind::Flashable,
            Filter::Inventory => app.bundles[i].kind != Kind::Flashable,
        })
        .collect();

    // Filter chips
    ui.with(&row(6), |ui| {
        let chips = [
            (Filter::All, ar.s(format!("All {}", app.bundles.len()))),
            (Filter::Flashable, ar.s(format!("Flashable {n_flash}"))),
            (Filter::Inventory, ar.s(format!("Inventories {n_inv}"))),
        ];
        for (i, (f, label)) in chips.into_iter().enumerate() {
            let id = ui.id_index("chip", i as u32);
            let active = app.filter == f;
            let hov = ui.pointer_over(id);
            if hov && !active {
                cx.inp.hand.set(true);
                if cx.inp.pressed {
                    acts.push(Action::Filter(f));
                }
            }
            let mut d = decl();
            d.id(id)
                .layout()
                .padding(Padding::new(12, 12, 6, 6))
                .end()
                .background_color(if active { th.accent } else if hov { th.hover } else { th.surface })
                .corner_radius()
                .all(14.0)
                .end()
                .border()
                .all_directions(1)
                .color(if active { th.accent } else { th.border })
                .end();
            ui.with(&d, |ui| text(ui, label, SMALL_B, if active { th.on_accent } else { th.text }));
        }
        spacer(ui);
        text(ui, "newest first", SMALL, th.text_dim);
    });

    // List
    let list_id = ui.id("fw_list");
    let off = scroll_offset(ui, list_id);
    ui.with(&scroll_box(th, list_id, off, 0), |ui| {
        for &i in &visible {
            let b = &app.bundles[i];
            let id = ui.id_index("fw_row", i as u32);
            let sel = app.selected == Some(i);
            let hov = ui.pointer_over(id);
            if hov {
                cx.inp.hand.set(true);
                if cx.inp.pressed {
                    acts.push(Action::Select(i));
                }
            }
            let bg = if sel { th.selected } else if hov { th.hover } else { th.surface };
            let mut d = row(10);
            d.id(id).layout().padding(Padding::new(14, 14, 9, 9)).end().background_color(bg);
            ui.with(&d, |ui| {
                ui.with(
                    decl().layout().width(GROW).direction(LayoutDirection::TopToBottom).child_gap(3).end(),
                    |ui| {
                        text(ui, &b.version, H2, th.text);
                        text(ui, ar.s(format!("{} · {}", b.device, b.stats)), SMALL, th.text_dim);
                    },
                );
                kind_badge(ui, b.kind, th);
            });
        }
    });

    // Detail card
    if let Some(i) = app.selected
        && let Some(b) = app.bundles.get(i)
    {
        ui.with(&card(th, 6), |ui| {
            ui.with(&row(10), |ui| {
                ui.with(decl().layout().width(GROW).end(), |ui| text(ui, &b.title, H2, th.text));
                kind_badge(ui, b.kind, th);
            });
            text(ui, ar.s(format!("{} · {}", b.device, b.stats)), BODY, th.text_dim);
            if let Some(n) = &b.notes {
                wrapped(ui, n, SMALL, if b.kind == Kind::Legacy || b.caution { th.warn } else { th.text_dim });
            }
            if !b.source.is_empty() {
                wrapped(ui, &b.source, SMALL, th.text_dim);
            }
            let block = if b.kind == Kind::Flashable { app.flash_block_reason() } else { None };
            ui.with(row(8).layout().padding(Padding::new(0, 0, 4, 0)).end(), |ui| {
                if b.kind == Kind::Flashable {
                    let id = ui.id("flash_btn");
                    button(ui, id, "Flash to device", Btn::Primary, block.is_none(), cx, acts, Action::AskFlash(i));
                }
                if let Some(p) = b.manifest.as_ref().or(b.inventory.as_ref()) {
                    let id = ui.id("files_btn");
                    let label = if b.kind == Kind::Flashable { "Show manifest" } else { "Show inventory" };
                    button(ui, id, label, Btn::Secondary, true, cx, acts, Action::OpenPath(p.clone()));
                }
                if let Some(dir) = &b.images_dir {
                    let id = ui.id("images_btn");
                    button(ui, id, "Show images", Btn::Secondary, dir.exists(), cx, acts, Action::OpenPath(dir.clone()));
                }
                if let Some(z) = &b.installer_zip {
                    let id = ui.id("zip_btn");
                    button(ui, id, "Show installer", Btn::Secondary, z.exists(), cx, acts, Action::OpenPath(z.clone()));
                }
            });
            if let Some(reason) = block {
                wrapped(ui, reason, SMALL, th.warn);
            }
        });
    }
}

// ── Devices ────────────────────────────────────────────────────────────────

fn devices<'c, 'r>(ui: &mut Ui<'c, 'r>, app: &'r App, ar: &'r Arena, cx: &Cx, acts: &mut Vec<Action>)
where
    'c: 'r,
{
    let th = cx.th;
    let busy = app.job.is_some();
    if app.devices.is_empty() {
        ui.with(&card(th, 8), |ui| {
            ui.with(&row(10), |ui| {
                dot(ui, if app.polled { th.warn } else { th.text_dim });
                text(
                    ui,
                    if app.polled { "No service-port device detected" } else { "Scanning USB…" },
                    H2,
                    th.text,
                );
            });
            wrapped(ui, "Connect the device's USB service port. The bus is polled every 2 seconds (VID 1edb / PID be13).", BODY, th.text_dim);
            #[cfg(windows)]
            wrapped(ui, "Windows: the service port must be bound to WinUSB (Zadig) to appear here. While it is bound to Blackmagic's DesktopVideoUpdate driver only the official updater can see it.", SMALL, th.text_dim);
            #[cfg(target_os = "linux")]
            wrapped(ui, "Linux: install the udev rule from the README so your user can open the device.", SMALL, th.text_dim);
        });
    }
    for (i, e) in app.devices.iter().enumerate() {
        ui.with(&card(th, 6), |ui| {
            ui.with(&row(10), |ui| {
                dot(ui, th.ok);
                text(ui, e.product.as_deref().unwrap_or("Blackmagic service port"), H2, th.text);
            });
            if let Some(s) = &e.serial {
                text(ui, ar.s(format!("Serial  {s}")), BODY, th.text_dim);
            }
            let mut usb = format!("USB  bus {} · address {}", e.bus_id, e.address);
            if !e.port_chain.is_empty() {
                let chain: Vec<String> = e.port_chain.iter().map(|p| p.to_string()).collect();
                usb.push_str(&format!(" · port {}", chain.join(".")));
            }
            if let Some(sp) = &e.speed {
                usb.push_str(&format!(" · {sp}"));
            }
            text(ui, ar.s(usb), BODY, th.text_dim);
            match &e.driver {
                Some(drv) if drv.to_ascii_lowercase().contains("winusb") => {
                    text(ui, ar.s(format!("Driver  {drv} — ready to flash")), BODY, th.ok)
                }
                Some(drv) => wrapped(ui, ar.s(format!("Driver  {drv} — swap to WinUSB with Zadig to flash")), BODY, th.warn),
                None if cfg!(windows) => text(ui, "Driver  unknown", BODY, th.text_dim),
                None => {}
            }
            ui.with(row(8).layout().padding(Padding::new(0, 0, 6, 0)).end(), |ui| {
                let id = ui.id_index("dump_btn", i as u32);
                button(ui, id, "Dump info to JSON", Btn::Secondary, !busy, cx, acts, Action::Dump);
                let id = ui.id_index("reset_btn", i as u32);
                button(ui, id, "Reset session", Btn::Danger, !busy, cx, acts, Action::AskReset);
            });
        });
    }
    if let Some(p) = &app.last_dump {
        ui.with(&card(th, 6), |ui| {
            text(ui, "Last dump", BODY_B, th.text);
            wrapped(ui, ar.s(p.display().to_string()), SMALL, th.text_dim);
            ui.with(&row(8), |ui| {
                let id = ui.id("dump_show");
                button(ui, id, "Show file", Btn::Secondary, true, cx, acts, Action::OpenPath(p.clone()));
            });
        });
    }
    wrapped(
        ui,
        "Dump reads identity and state only. The service port has no image readback, so a firmware image cannot be pulled off the device.",
        SMALL,
        th.text_dim,
    );
}

// ── Update ─────────────────────────────────────────────────────────────────

fn update<'c, 'r>(ui: &mut Ui<'c, 'r>, app: &'r App, ar: &'r Arena, cx: &Cx, acts: &mut Vec<Action>)
where
    'c: 'r,
{
    let th = cx.th;
    let busy = app.job.is_some();
    let n_flash = app.bundles.iter().filter(|b| b.kind == Kind::Flashable).count();
    ui.with(&card(th, 8), |ui| {
        text(ui, "Blackmagic Desktop Video catalog", H2, th.text);
        wrapped(ui, "Check Blackmagic's public download catalog. Versions not yet indexed can be downloaded, extracted and inventoried here.", BODY, th.text_dim);
        #[cfg(windows)]
        wrapped(ui, "Extraction runs msiexec /a, which needs UBDMF started as administrator. For many versions at once use tools/harvest.ps1.", SMALL, th.text_dim);
        #[cfg(not(windows))]
        wrapped(ui, "Downloading new versions needs Windows (installer extraction uses msiexec). Check catalog and Rebuild index work here; copy a firmware/ folder harvested on Windows next to UBDMF.", SMALL, th.warn);
        text(
            ui,
            ar.s(format!("Index: {} flashable · {} inventories", n_flash, app.bundles.len() - n_flash)),
            BODY,
            th.text,
        );
        ui.with(row(8).layout().padding(Padding::new(0, 0, 4, 0)).end(), |ui| {
            let id = ui.id("cat_btn");
            button(ui, id, "Check catalog", Btn::Primary, !busy, cx, acts, Action::CheckCatalog);
            let id = ui.id("rebuild_btn");
            button(ui, id, "Rebuild index", Btn::Secondary, !busy, cx, acts, Action::RebuildIndex);
        });
        match &app.catalog {
            CatalogState::Idle => {}
            CatalogState::Loading => text(ui, "Contacting Blackmagic…", SMALL, th.text_dim),
            CatalogState::Failed(e) => wrapped(ui, ar.s(format!("Catalog check failed: {e}")), SMALL, th.err),
            CatalogState::Ready(rows) => {
                let missing = rows.iter().filter(|r| !r.indexed).count();
                text(
                    ui,
                    ar.s(format!("{} versions in catalog · {} indexed · {missing} not indexed", rows.len(), rows.len() - missing)),
                    SMALL_B,
                    if missing == 0 { th.ok } else { th.warn },
                );
            }
        }
    });
    if let CatalogState::Ready(rows) = &app.catalog {
        let id = ui.id("cat_list");
        let off = scroll_offset(ui, id);
        ui.with(&scroll_box(th, id, off, 0), |ui| {
            for (i, r) in rows.iter().enumerate() {
                let mut d = row(10);
                d.layout().padding(Padding::new(14, 14, 7, 7)).end();
                ui.with(&d, |ui| {
                    ui.with(decl().layout().width(GROW).end(), |ui| text(ui, &r.version, BODY_B, th.text));
                    if r.indexed {
                        badge(ui, "Indexed", th.ok, th.badge_ok_bg);
                    } else {
                        let id = ui.id_index("fetch_btn", i as u32);
                        button(
                            ui,
                            id,
                            "Download",
                            Btn::Secondary,
                            !busy && cfg!(windows),
                            cx,
                            acts,
                            Action::Fetch { version: r.version.clone(), download_id: r.download_id.clone() },
                        );
                    }
                });
            }
        });
    }
}

// ── Log ────────────────────────────────────────────────────────────────────

fn log_view<'c, 'r>(ui: &mut Ui<'c, 'r>, app: &'r App, ar: &'r Arena, cx: &Cx, acts: &mut Vec<Action>)
where
    'c: 'r,
{
    let th = cx.th;
    let path = crate::logger::log_path();
    ui.with(&row(8), |ui| {
        ui.with(decl().layout().width(GROW).direction(LayoutDirection::TopToBottom).child_gap(2).end(), |ui| {
            text(ui, ar.s(format!("{} recent lines", app.log_lines.len())), BODY_B, th.text);
            text(ui, ar.s(path.display().to_string()), SMALL, th.text_dim);
        });
        let id = ui.id("follow_btn");
        let label = if app.log_follow { "Following" } else { "Follow" };
        let kind = if app.log_follow { Btn::Primary } else { Btn::Secondary };
        button(ui, id, label, kind, true, cx, acts, Action::ToggleFollow);
        let id = ui.id("copy_btn");
        button(ui, id, "Copy", Btn::Secondary, true, cx, acts, Action::CopyLog);
        let id = ui.id("openlog_btn");
        button(ui, id, "Open file", Btn::Secondary, path.exists(), cx, acts, Action::OpenPath(path.clone()));
    });

    let id = ui.id("log_scroll");
    let mut off = scroll_offset(ui, id);
    if let Some(sd) = ui.scroll_container_data(id)
        && !sd.scrollPosition.is_null()
    {
        let max = (sd.contentDimensions.height - sd.scrollContainerDimensions.height).max(0.0);
        let at_bottom = -off.y >= max - 2.0;
        let over = ui.pointer_over(id);
        if over && cx.inp.wheel_y > 0.0 && app.log_follow {
            acts.push(Action::ToggleFollow); // user scrolled up: stop following
        } else if over && cx.inp.wheel_y < 0.0 && at_bottom && !app.log_follow {
            acts.push(Action::ToggleFollow); // back at the bottom: resume
        } else if app.log_follow {
            unsafe { (*sd.scrollPosition).y = -max };
            off.y = -max;
        }
    }
    let mut d = scroll_box(th, id, off, 0);
    d.layout().padding(Padding::new(10, 10, 8, 8)).child_gap(3).end().border().between_children(0).end();
    ui.with(&d, |ui| {
        for line in &app.log_lines {
            let col = if line.contains(" ERROR ") {
                th.err
            } else if line.contains(" WARN ") {
                th.warn
            } else if line.contains("[raylib]") {
                th.text_dim
            } else {
                th.text
            };
            wrapped(ui, line, SMALL, col);
        }
    });
}

// ── Progress / status / dialog ─────────────────────────────────────────────

fn progress<'c, 'r>(ui: &mut Ui<'c, 'r>, app: &'r App, ar: &'r Arena, cx: &Cx)
where
    'c: 'r,
{
    let Some(job) = &app.job else { return };
    let th = cx.th;
    ui.with(
        decl()
            .layout()
            .width(GROW)
            .direction(LayoutDirection::TopToBottom)
            .padding(Padding::new(16, 16, 10, 12))
            .child_gap(6)
            .end()
            .background_color(th.surface)
            .border()
            .top(1)
            .color(th.border)
            .end(),
        |ui| {
            ui.with(&row(8), |ui| {
                ui.with(decl().layout().width(GROW).end(), |ui| text(ui, &job.title, BODY_B, th.text));
                if let Some(f) = job.frac {
                    text(ui, ar.s(format!("{:.0}%", f * 100.0)), BODY_B, th.accent);
                }
            });
            text(ui, &job.phase, SMALL, th.text_dim);
            ui.with(
                decl()
                    .layout()
                    .width(GROW)
                    .height(Sizing::Fixed(8.0))
                    .end()
                    .background_color(th.badge_dim_bg)
                    .corner_radius()
                    .all(4.0)
                    .end(),
                |ui| {
                    if let Some(f) = job.frac
                        && f > 0.0
                    {
                        ui.with(
                            decl()
                                .layout()
                                .width(Sizing::Percent(f.clamp(0.0, 1.0)))
                                .height(GROW)
                                .end()
                                .background_color(th.accent)
                                .corner_radius()
                                .all(4.0)
                                .end(),
                            |_| {},
                        );
                    }
                },
            );
            if !job.detail.is_empty() {
                text(ui, &job.detail, SMALL, th.text_dim);
            }
            if job.critical {
                text(ui, "Do not unplug or power off the device", SMALL_B, th.warn);
            }
        },
    );
}

fn status_bar<'c, 'r>(ui: &mut Ui<'c, 'r>, app: &'r App, ar: &'r Arena, cx: &Cx)
where
    'c: 'r,
{
    let th = cx.th;
    let col = match app.level {
        Level::Info => th.text_dim,
        Level::Ok => th.ok,
        Level::Warn => th.warn,
        Level::Err => th.err,
    };
    ui.with(
        row(10)
            .layout()
            .padding(Padding::new(16, 16, 8, 8))
            .end()
            .background_color(th.surface)
            .border()
            .top(1)
            .color(th.border)
            .end(),
        |ui| {
            ui.with(decl().layout().width(GROW).end().clip(true, false, Default::default()), |ui| {
                text(ui, &app.status, SMALL, col)
            });
            let n = app.devices.len();
            dot(ui, if n > 0 { th.ok } else { th.text_dim });
            let label = match n {
                0 => "No device",
                1 => "1 device",
                _ => ar.s(format!("{n} devices")),
            };
            text(ui, label, SMALL_B, if n > 0 { th.text } else { th.text_dim });
        },
    );
}

fn dialog<'c, 'r>(ui: &mut Ui<'c, 'r>, app: &'r App, ar: &'r Arena, cx: &Cx, acts: &mut Vec<Action>)
where
    'c: 'r,
{
    let th = cx.th;
    let Some(c) = &app.confirm else { return };
    let (title, body, warn, yes): (&'r str, &'r str, &'r str, &'r str) = match c {
        Confirm::Flash { title, device, caution, .. } => (
            "Flash firmware?",
            ar.s(format!("{title} will be written to the connected {device} over the USB service port.")),
            match caution {
                Some(note) => ar.s(format!(
                    "This ERASES and REWRITES the device firmware. Keep the device powered and connected until it finishes.

Caution: {note}"
                )),
                None => "This ERASES and REWRITES the device firmware. Keep the device powered and connected until it finishes.",
            },
            "Flash",
        ),
        Confirm::Reset => (
            "Reset device session?",
            "Sends the reboot command (0x12) to clear a lingering service-port session.",
            "The device will drop off the USB bus for a few seconds.",
            "Reset",
        ),
    };
    let dw = (cx.inp.w - 32.0).min(440.0);
    ui.with(
        decl()
            .id(ui.id("modal"))
            .floating()
            .attach_to(FloatingAttachToElement::Root)
            .z_index(100)
            .pointer_capture_mode(PointerCaptureMode::Capture)
            .end()
            .layout()
            .width(Sizing::Fixed(cx.inp.w))
            .height(Sizing::Fixed(cx.inp.h))
            .child_alignment(Alignment::new(AX::Center, AY::Center))
            .end()
            .background_color(th.overlay),
        |ui| {
            let mut d = card(th, 10);
            d.layout().width(Sizing::Fixed(dw)).padding(Padding::all(20)).end();
            ui.with(&d, |ui| {
                text(ui, title, H2, th.text);
                wrapped(ui, body, BODY, th.text);
                wrapped(ui, warn, SMALL_B, th.warn);
                ui.with(row(8).layout().padding(Padding::new(0, 0, 8, 0)).end(), |ui| {
                    spacer(ui);
                    let id = ui.id("dlg_no");
                    button(ui, id, "Cancel", Btn::Secondary, true, cx, acts, Action::ConfirmNo);
                    let id = ui.id("dlg_yes");
                    let kind = if matches!(c, Confirm::Reset) { Btn::Danger } else { Btn::Primary };
                    button(ui, id, yes, kind, true, cx, acts, Action::ConfirmYes);
                });
            });
        },
    );
}
