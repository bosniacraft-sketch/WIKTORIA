//! Moddable look (D19): colours, sizes and an optional font come from `theme.toml` next to the exe,
//! images come from a `ui_art/` folder next to the exe, and a `themes/<name>/` folder can override
//! single files (including `theme.toml` keys). Everything falls back to the built-in palette or to
//! "no image" when a file is missing. Nothing from the Victoria 2 install is read or written here.
//!
//! Images are never used to size widgets: callers pass a slot size derived from the text height and
//! `image_in_slot` fits the picture into it (D18).
//!
//! The main menu's background and buttons go through `Art::menu_background` and `Art::menu_button` only,
//! so another look can be plugged in there.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, ColorImage, Context, Sense, TextureHandle, TextureOptions, Vec2};

/// Font family holding the serif face (title, headings, map labels).
pub const SERIF: &str = "serif";

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub ink: Color32,
    pub ink_dim: Color32,
    pub gold: Color32,
    pub bg_deep: Color32,
    pub bg_panel: Color32,
    pub bg_widget: Color32,
    pub bg_hover: Color32,
    pub bg_active: Color32,
    pub edge: Color32,
    pub selection: Color32,
    pub good: Color32,
    pub bad: Color32,
    /// Map country names: letters and their halo.
    pub label_ink: Color32,
    pub label_halo: Color32,
    /// Body text size in points (the other styles scale with it).
    pub font_size: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            ink: Color32::from_rgb(226, 214, 188),
            ink_dim: Color32::from_rgb(158, 147, 124),
            gold: Color32::from_rgb(201, 160, 84),
            bg_deep: Color32::from_rgb(22, 19, 16),
            bg_panel: Color32::from_rgb(34, 29, 24),
            bg_widget: Color32::from_rgb(52, 45, 37),
            bg_hover: Color32::from_rgb(72, 62, 49),
            bg_active: Color32::from_rgb(98, 81, 55),
            edge: Color32::from_rgb(88, 75, 56),
            selection: Color32::from_rgb(120, 92, 44),
            good: Color32::from_rgb(120, 160, 100),
            bad: Color32::from_rgb(190, 100, 80),
            label_ink: Color32::from_rgb(28, 22, 16),
            label_halo: Color32::from_rgb(236, 226, 200),
            font_size: 14.0,
        }
    }
}

fn parse_color(v: &str) -> Option<Color32> {
    let v = v.trim().trim_matches('"');
    if let Some(h) = v.strip_prefix('#') {
        if h.len() == 6 {
            let n = u32::from_str_radix(h, 16).ok()?;
            return Some(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8));
        }
        return None;
    }
    let p: Vec<u8> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    (p.len() == 3).then(|| Color32::from_rgb(p[0], p[1], p[2]))
}

/// Flat `key = value` parser (sections and comments are ignored). Enough for colours and sizes.
pub fn parse_kv(src: &str, out: &mut HashMap<String, String>) {
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let v = v.split(" #").next().unwrap_or(v).trim().trim_matches('"');
            out.insert(k.trim().to_string(), v.to_string());
        }
    }
}

/// The selected theme's name: GM_UI_THEME, else the settings value.
pub fn theme_name(name: Option<&str>) -> Option<String> {
    std::env::var("GM_UI_THEME").ok().or_else(|| name.map(str::to_string)).filter(|n| !n.is_empty())
}

pub struct Art {
    /// Search order: `themes/<name>/`, then `ui_art/` next to the exe, then the source tree's `ui_art/`.
    dirs: Vec<PathBuf>,
    pub font_file: Option<PathBuf>,
    cache: HashMap<String, Option<TextureHandle>>,
}

impl Art {
    /// `name` (from the settings file) picks `themes/<name>/`; GM_UI_THEME wins over it.
    pub fn load(base: &Path, name: Option<&str>) -> (Theme, Art) {
        let mut kv = HashMap::new();
        if let Ok(s) = std::fs::read_to_string(base.join("theme.toml")) {
            parse_kv(&s, &mut kv);
        }
        let name = theme_name(name).or_else(|| kv.get("theme").cloned());
        let mut dirs = Vec::new();
        if let Some(n) = name.filter(|n| !n.is_empty()) {
            let d = base.join("themes").join(n);
            if let Ok(s) = std::fs::read_to_string(d.join("theme.toml")) {
                parse_kv(&s, &mut kv);
            }
            dirs.push(d);
        }
        dirs.push(base.join("ui_art"));
        // The default art kept in the source tree (for `cargo run`, where the exe sits in target/).
        dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("ui_art"));
        let mut t = Theme::default();
        for (key, slot) in [
            ("ink", &mut t.ink),
            ("ink_dim", &mut t.ink_dim),
            ("gold", &mut t.gold),
            ("bg_deep", &mut t.bg_deep),
            ("bg_panel", &mut t.bg_panel),
            ("bg_widget", &mut t.bg_widget),
            ("bg_hover", &mut t.bg_hover),
            ("bg_active", &mut t.bg_active),
            ("edge", &mut t.edge),
            ("selection", &mut t.selection),
            ("good", &mut t.good),
            ("bad", &mut t.bad),
            ("label_ink", &mut t.label_ink),
            ("label_halo", &mut t.label_halo),
        ] {
            if let Some(c) = kv.get(key).and_then(|v| parse_color(v)) {
                *slot = c;
            }
        }
        if let Some(s) = kv.get("font_size").and_then(|v| v.parse::<f32>().ok()) {
            t.font_size = s.clamp(8.0, 40.0);
        }
        let mut art = Art { dirs, font_file: None, cache: HashMap::new() };
        if let Some(f) = kv.get("font") {
            art.font_file = art.find(f);
        }
        (t, art)
    }

    fn find(&self, file: &str) -> Option<PathBuf> {
        self.dirs.iter().map(|d| d.join(file)).find(|p| p.is_file())
    }

    /// The serif face: `fonts/serif.ttf` from a theme, else our bundled EB Garamond (SIL OFL 1.1,
    /// licence in `ui_art/fonts/OFL.txt`).
    pub fn serif_font(&self) -> Vec<u8> {
        self.find("fonts/serif.ttf")
            .and_then(|p| std::fs::read(p).ok())
            .unwrap_or_else(|| include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui_art/fonts/EBGaramond.ttf")).to_vec())
    }

    /// Our own image `ui_art/<name>.png` (or the theme's override). `None` when absent.
    pub fn get(&mut self, ctx: &Context, name: &str) -> Option<TextureHandle> {
        if let Some(t) = self.cache.get(name) {
            return t.clone();
        }
        let tex = self.find(&format!("{name}.png")).and_then(|p| {
            let img = image::open(p).ok()?.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            Some(ctx.load_texture(format!("art_{name}"), ColorImage::from_rgba_unmultiplied(size, img.as_raw()), TextureOptions::LINEAR))
        });
        self.cache.insert(name.to_string(), tex.clone());
        tex
    }

    /// Menu backdrop (main menu, loading screen). A theme's `menu_background.png` fills the screen
    /// (cropped to keep its aspect ratio); otherwise a dark gradient with a faint silhouette of the
    /// install's land, if it has been loaded.
    pub fn menu_background(&mut self, painter: &egui::Painter, rect: egui::Rect, t: &Theme, silhouette: Option<&TextureHandle>) {
        let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let slot = crate::slots::get(painter.ctx());
        let shape = slot.lock().unwrap().shape(painter.ctx(), "menu_bg", rect, crate::slots::State::Normal);
        if let Some(sh) = shape {
            painter.add(sh);
            return;
        }
        if let Some(tex) = self.get(painter.ctx(), "menu_background") {
            let s = tex.size_vec2();
            let scale = (rect.width() / s.x).max(rect.height() / s.y);
            let r = egui::Rect::from_center_size(rect.center(), s * scale);
            painter.with_clip_rect(rect).image(tex.id(), r, full, Color32::WHITE);
            return;
        }
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(rect.left_top(), t.bg_panel);
        mesh.colored_vertex(rect.right_top(), t.bg_panel);
        mesh.colored_vertex(rect.right_bottom(), t.bg_deep);
        mesh.colored_vertex(rect.left_bottom(), t.bg_deep);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(egui::Shape::mesh(mesh));
        if let Some(s) = silhouette {
            let size = s.size_vec2();
            // Cover the whole window (keeps the aspect ratio, centred; the overflow is clipped).
            let scale = (rect.width() / size.x).max(rect.height() / size.y);
            let r = egui::Rect::from_center_size(rect.center(), size * scale);
            painter.with_clip_rect(rect).image(s.id(), r, full, t.gold.gamma_multiply(0.14));
        }
    }

    /// A menu button filling `size`. Text is sized from the slot, never from art.
    pub fn menu_button(&mut self, ui: &mut egui::Ui, t: &Theme, text: &str, size: Vec2, enabled: bool) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
        let hovered = enabled && resp.hovered();
        let state = if resp.is_pointer_button_down_on() {
            crate::slots::State::Pressed
        } else if hovered {
            crate::slots::State::Hover
        } else {
            crate::slots::State::Normal
        };
        if crate::slots::paint(ui, "menu_button", rect, state) {
            let col = if enabled { t.ink } else { t.ink_dim.gamma_multiply(0.6) };
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::new(size.y * 0.52, egui::FontFamily::Name(SERIF.into())), col);
            return resp;
        }
        let fill = if !enabled {
            t.bg_panel.gamma_multiply(0.85)
        } else if hovered {
            t.bg_hover
        } else {
            t.bg_widget
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            egui::CornerRadius::same(4),
            fill.gamma_multiply(0.94),
            egui::Stroke::new(if hovered { 1.5 } else { 1.0 }, if hovered { t.gold } else { t.edge }),
            egui::StrokeKind::Inside,
        );
        let col = if enabled { t.ink } else { t.ink_dim.gamma_multiply(0.6) };
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::new(size.y * 0.52, egui::FontFamily::Name(SERIF.into())), col);
        resp
    }
}

/// The only way images are drawn: the picture is scaled to fit `slot` and keeps its aspect ratio.
/// The slot comes from the layout (text-height units), never from the picture's pixel size.
pub fn image_in_slot(ui: &mut egui::Ui, tex: &TextureHandle, slot: Vec2) {
    let (rect, _) = ui.allocate_exact_size(slot, Sense::hover());
    ui.put(rect, egui::Image::new(tex).fit_to_exact_size(slot));
}
