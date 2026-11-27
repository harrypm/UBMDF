//! Fonts, Clay text measurement and a raylib-6 renderer for Clay render
//! commands. clay-layout's bundled renderer targets raylib 5.5, so this is our
//! own port of Clay's reference raylib renderer, with real TTF fonts.
//!
//! Fonts are rasterised once per (family, size) — every text style draws from
//! an atlas baked at exactly that pixel size, so text is crisp at 1:1 with no
//! scaling blur. The atlas includes Latin-1 + common punctuation (… — → ·),
//! which raylib's default 95-glyph set lacked (they rendered as '?').
use clay_layout::math::{BoundingBox, Dimensions};
use clay_layout::render_commands::{RenderCommand, RenderCommandConfig};
use clay_layout::text::TextConfig;
use raylib::ffi;
use raylib::prelude::*;
use std::ffi::CString;

/// Text styles. The value is the Clay font id; `STYLES[id]` gives (bold, px).
pub const SMALL: u16 = 0;
pub const BODY: u16 = 1;
pub const SMALL_B: u16 = 2;
pub const BODY_B: u16 = 3;
pub const H2: u16 = 4;
pub const H1: u16 = 5;
const STYLES: [(bool, i32); 6] = [(false, 16), (false, 18), (true, 16), (true, 18), (true, 21), (true, 27)];

pub fn style_px(id: u16) -> u16 {
    STYLES[id as usize].1 as u16
}

pub struct Fonts {
    _owned: Vec<Font>,
    raw: Vec<ffi::Font>,
}

impl Fonts {
    /// Arimo is embedded in the binary, so portable builds (AppImage, .app,
    /// a lone .exe) render identically with no assets folder beside them.
    /// `scale` > 1 (Android high-density screens) rasterises each style at
    /// px × scale; text is still laid out and drawn at its logical px size and
    /// the whole frame is zoomed by `scale`, so glyphs land 1:1 on screen.
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread, scale: f32) -> Fonts {
        const REGULAR: &[u8] = include_bytes!("../../assets/fonts/Arimo-Regular.ttf");
        const BOLD: &[u8] = include_bytes!("../../assets/fonts/Arimo-Bold.ttf");
        let mut glyphs: String = (32u8..127).map(|b| b as char).collect();
        glyphs.extend((0xA0u32..=0xFF).filter_map(char::from_u32));
        glyphs.push_str("…—–→←↑↓•·×±µ‘’“”‹›");

        let mut owned = Vec::new();
        for &(bold, px) in STYLES.iter() {
            let data = if bold { BOLD } else { REGULAR };
            let atlas_px = ((px as f32) * scale).round().max(1.0) as i32;
            let font = rl
                .load_font_from_memory(thread, ".ttf", data, atlas_px, Some(&glyphs))
                .unwrap_or_else(|e| panic!("embedded Arimo {px}px failed to load: {e}"));
            owned.push(font);
        }
        let raw = owned.iter().map(|f| *f.as_ref()).collect();
        Fonts { _owned: owned, raw }
    }

    /// Copy of the raw font table for the measure callback (fonts outlive it:
    /// both are dropped at the end of run_gui, after the Clay context).
    pub fn raw(&self) -> Vec<ffi::Font> {
        self.raw.clone()
    }
}

fn cstr(text: &str) -> CString {
    CString::new(text.replace('\0', " ")).unwrap_or_default()
}

pub fn measure(raw: &[ffi::Font], text: &str, cfg: &TextConfig) -> Dimensions {
    let id = (cfg.font_id as usize).min(raw.len() - 1);
    let size = cfg.font_size as f32;
    let c = cstr(text);
    let v = unsafe { ffi::MeasureTextEx(raw[id], c.as_ptr(), size, 0.0) };
    Dimensions::new(v.x, size)
}

fn rl_color(c: clay_layout::Color) -> ffi::Color {
    ffi::Color { r: c.r as u8, g: c.g as u8, b: c.b as u8, a: c.a as u8 }
}

fn rl_rect(b: &BoundingBox) -> ffi::Rectangle {
    ffi::Rectangle { x: b.x.round(), y: b.y.round(), width: b.width.round(), height: b.height.round() }
}

fn roundness(radius: f32, b: &BoundingBox) -> f32 {
    let m = b.width.min(b.height);
    if m <= 0.0 { 0.0 } else { (radius * 2.0 / m).min(1.0) }
}

pub fn render<'a>(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    commands: impl Iterator<Item = RenderCommand<'a, (), ()>>,
    scale: f32,
) {
    // Logical units -> screen pixels. Desktop (scale 1.0) skips the camera so
    // its output is unchanged.
    let zoom = (scale - 1.0).abs() > f32::EPSILON;
    if zoom {
        let cam = ffi::Camera2D {
            offset: ffi::Vector2 { x: 0.0, y: 0.0 },
            target: ffi::Vector2 { x: 0.0, y: 0.0 },
            rotation: 0.0,
            zoom: scale,
        };
        unsafe { ffi::BeginMode2D(cam) };
    }
    for cmd in commands {
        let b = cmd.bounding_box;
        match cmd.config {
            RenderCommandConfig::Rectangle(r) => {
                let rad = r.corner_radii.top_left;
                if rad > 0.0 {
                    d.draw_rectangle_rounded(rl_rect(&b), roundness(rad, &b), 8, rl_color(r.color));
                } else {
                    d.draw_rectangle_rec(rl_rect(&b), rl_color(r.color));
                }
            }
            RenderCommandConfig::Border(br) => {
                let w = &br.width;
                let col = rl_color(br.color);
                let rad = br.corner_radii.top_left;
                let uniform = w.left == w.right && w.left == w.top && w.left == w.bottom && w.left > 0;
                if uniform && rad > 0.0 {
                    d.draw_rectangle_rounded_lines_ex(rl_rect(&b), roundness(rad, &b), 8, w.left as f32, col);
                } else {
                    let (x, y, bw, bh) = (b.x.round() as i32, b.y.round() as i32, b.width.round() as i32, b.height.round() as i32);
                    if w.left > 0 { d.draw_rectangle(x, y, w.left as i32, bh, col); }
                    if w.right > 0 { d.draw_rectangle(x + bw - w.right as i32, y, w.right as i32, bh, col); }
                    if w.top > 0 { d.draw_rectangle(x, y, bw, w.top as i32, col); }
                    if w.bottom > 0 { d.draw_rectangle(x, y + bh - w.bottom as i32, bw, w.bottom as i32, col); }
                }
            }
            RenderCommandConfig::Text(t) => {
                let id = (t.font_id as usize).min(fonts.raw.len() - 1);
                let c = cstr(t.text);
                let pos = ffi::Vector2 { x: b.x.round(), y: b.y.round() };
                unsafe {
                    ffi::DrawTextEx(fonts.raw[id], c.as_ptr(), pos, t.font_size as f32, 0.0, rl_color(t.color));
                }
            }
            // Scissor rectangles are in screen pixels, not camera space.
            RenderCommandConfig::ScissorStart() => unsafe {
                ffi::BeginScissorMode(
                    (b.x * scale).round() as i32,
                    (b.y * scale).round() as i32,
                    (b.width * scale).round() as i32,
                    (b.height * scale).round() as i32,
                );
            },
            RenderCommandConfig::ScissorEnd() => unsafe {
                ffi::EndScissorMode();
            },
            RenderCommandConfig::Image(_) | RenderCommandConfig::Custom(_) | RenderCommandConfig::None() => {}
        }
    }
    if zoom {
        unsafe { ffi::EndMode2D() };
    }
}
