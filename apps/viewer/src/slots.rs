//! Themable UI slots: named places in our layout (top bar, outliner, menu buttons...) that can be dressed
//! with a picture from the user's own Victoria 2 install. The assignment lives in
//! `themes/<name>/slots.toml` as `install:`-relative paths; the pictures are decoded at runtime and never
//! copied. A slot without a picture (or with a missing file) keeps the built-in drawing.
//!
//! The slot table sits in the egui context (`get(ctx)`), so any drawing code can reach it without
//! plumbing. The UI maker turns on `record` to collect where each slot was drawn this frame.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32, ColorImage, Context, Id, Mesh, Pos2, Rect, Response, Shape, TextureHandle, TextureOptions, Vec2};

/// (key, name shown to the owner). The order is the UI maker's list order.
pub const SLOTS: &[(&str, &str)] = &[
    ("menu_bg", "Main menu background"),
    ("menu_button", "Main menu buttons"),
    ("topbar_bg", "Top bar background"),
    ("topbar_tab", "Top bar tab buttons"),
    ("flag_frame", "Flag frame (top bar)"),
    ("speed_button", "Pause / speed buttons"),
    ("msglog_bg", "Message log (bottom bar) background"),
    ("mapmode_button", "Map-mode buttons"),
    ("outliner_bg", "Outliner background"),
    ("province_bg", "Province window background"),
    ("window_bg", "Country window frame"),
    ("window_tab", "Country window tab buttons"),
    ("button", "Small buttons (expand, build...)"),
];

pub fn slot_name(key: &str) -> &str {
    SLOTS.iter().find(|s| s.0 == key).map_or(key, |s| s.1)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Fit {
    #[default]
    Stretch,
    Nine,
    Tile,
}

impl Fit {
    pub fn as_str(self) -> &'static str {
        match self {
            Fit::Stretch => "stretch",
            Fit::Nine => "nine",
            Fit::Tile => "tile",
        }
    }
    fn parse(s: &str) -> Fit {
        match s {
            "nine" | "9slice" | "nine_slice" => Fit::Nine,
            "tile" => Fit::Tile,
            _ => Fit::Stretch,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum State {
    Normal,
    Hover,
    Pressed,
}

#[derive(Clone, PartialEq, Debug)]
pub struct SlotArt {
    /// `install:gfx/...` (relative to the install root).
    pub image: String,
    pub fit: Fit,
    /// Left, top, right, bottom inset in source pixels (9-slice).
    pub border: [u32; 4],
    /// Horizontal strip of N frames.
    pub frames: u32,
    pub frame_normal: u32,
    pub frame_hover: u32,
    pub frame_pressed: u32,
}

impl SlotArt {
    pub fn new(image: String) -> Self {
        SlotArt { image, fit: Fit::Stretch, border: [8; 4], frames: 1, frame_normal: 0, frame_hover: 0, frame_pressed: 0 }
    }
}

pub const PREFIX: &str = "install:";

#[derive(Default)]
pub struct Slots {
    pub map: HashMap<String, SlotArt>,
    pub install: Option<PathBuf>,
    cache: HashMap<String, Option<(TextureHandle, [usize; 2])>>,
    /// UI maker: collect (key, rect) of every slot drawn this frame.
    pub record: bool,
    pub hits: Vec<(&'static str, Rect)>,
}

pub type SlotsRef = Arc<Mutex<Slots>>;

fn ctx_id() -> Id {
    Id::new("ui_slots")
}

pub fn get(ctx: &Context) -> SlotsRef {
    ctx.data_mut(|d| d.get_temp_mut_or_insert_with(ctx_id(), || Arc::new(Mutex::new(Slots::default()))).clone())
}

/// Path of `slots.toml` for theme `name`: next to the exe first, then the source tree (`cargo run`).
pub fn find_file(base: &Path, name: &str) -> Option<PathBuf> {
    [base.join("themes").join(name), Path::new(env!("CARGO_MANIFEST_DIR")).join("themes").join(name)]
        .into_iter()
        .map(|d| d.join("slots.toml"))
        .find(|p| p.is_file())
}

/// Load theme `name`'s slots into the context (an empty table if none).
pub fn install_for(ctx: &Context, base: &Path, name: Option<&str>, install: Option<&Path>) {
    let map = name.and_then(|n| find_file(base, n)).and_then(|p| std::fs::read_to_string(p).ok()).map(|s| parse(&s)).unwrap_or_default();
    let s = get(ctx);
    let mut s = s.lock().unwrap();
    s.map = map;
    s.install = install.map(Path::to_path_buf);
    s.cache.clear();
}

pub fn parse(src: &str) -> HashMap<String, SlotArt> {
    let mut out: HashMap<String, SlotArt> = HashMap::new();
    let mut cur: Option<String> = None;
    for line in src.lines() {
        let line = line.split(" #").next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(k) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            cur = Some(k.trim().to_string());
            continue;
        }
        let (Some(sec), Some((k, v))) = (&cur, line.split_once('=')) else { continue };
        let v = v.trim().trim_matches('"');
        let a = out.entry(sec.clone()).or_insert_with(|| SlotArt::new(String::new()));
        let n = || v.parse::<u32>().unwrap_or(0);
        match k.trim() {
            "image" => a.image = v.to_string(),
            "mode" => a.fit = Fit::parse(v),
            "border" => {
                let p: Vec<u32> = v.trim_matches(['[', ']']).split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if p.len() == 4 {
                    a.border = [p[0], p[1], p[2], p[3]];
                }
            }
            "frames" => a.frames = n().max(1),
            "frame_normal" => a.frame_normal = n(),
            "frame_hover" => a.frame_hover = n(),
            "frame_pressed" => a.frame_pressed = n(),
            _ => {}
        }
    }
    out.retain(|_, a| !a.image.is_empty());
    out
}

pub fn write(map: &HashMap<String, SlotArt>) -> String {
    let mut s = String::from(
        "# UI slots for this theme, written by `viewer --ui-maker`.\n\
         # `install:` paths are relative to the user's own Victoria 2 folder and are read at runtime.\n\
         # No game files are stored here.\n",
    );
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort_by_key(|k| SLOTS.iter().position(|s| s.0 == k.as_str()).unwrap_or(usize::MAX));
    for k in keys {
        let a = &map[k];
        let b = a.border;
        s += &format!(
            "\n[{k}]\nimage = \"{}\"\nmode = \"{}\"\nborder = [{}, {}, {}, {}]\nframes = {}\nframe_normal = {}\nframe_hover = {}\nframe_pressed = {}\n",
            a.image, a.fit.as_str(), b[0], b[1], b[2], b[3], a.frames, a.frame_normal, a.frame_hover, a.frame_pressed
        );
    }
    s
}

/// Decode an install picture (or any absolute path) to a texture. Cached by path.
pub fn load_texture(ctx: &Context, path: &Path, name: &str) -> Option<(TextureHandle, [usize; 2])> {
    let img = crate::dds::load_any(path)?;
    let size = [img.width, img.height];
    let tex = ctx.load_texture(name, ColorImage::from_rgba_unmultiplied(size, &img.px), TextureOptions::LINEAR);
    Some((tex, size))
}

impl Slots {
    pub fn resolve(&self, image: &str) -> Option<PathBuf> {
        match image.strip_prefix(PREFIX) {
            Some(rel) => self.install.as_ref().map(|i| i.join(rel)),
            None => Some(PathBuf::from(image)),
        }
    }

    pub fn has(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    fn texture(&mut self, ctx: &Context, image: &str) -> Option<(TextureHandle, [usize; 2])> {
        if let Some(t) = self.cache.get(image) {
            return t.clone();
        }
        let t = self.resolve(image).and_then(|p| load_texture(ctx, &p, &format!("slot:{image}")));
        self.cache.insert(image.to_string(), t.clone());
        t
    }

    /// The slot's picture for `rect`, or `None` (caller keeps its built-in drawing).
    pub fn shape(&mut self, ctx: &Context, key: &'static str, rect: Rect, state: State) -> Option<Shape> {
        if self.record {
            self.hits.push((key, rect));
        }
        let a = self.map.get(key)?.clone();
        let (tex, size) = self.texture(ctx, &a.image)?;
        Some(Shape::mesh(mesh(&tex, size, &a, rect, state)))
    }
}

fn mesh(tex: &TextureHandle, size: [usize; 2], a: &SlotArt, rect: Rect, state: State) -> Mesh {
    let frames = a.frames.max(1);
    let f = match state {
        State::Normal => a.frame_normal,
        State::Hover => a.frame_hover,
        State::Pressed => a.frame_pressed,
    }
    .min(frames - 1);
    let (tw, th) = (size[0] as f32, size[1] as f32);
    let fw = tw / frames as f32;
    let x0 = fw * f as f32;
    let uv = |x: f32, y: f32| Pos2::new(x / tw, y / th);
    let mut m = Mesh::with_texture(tex.id());
    let w = Color32::WHITE;
    match a.fit {
        Fit::Stretch => m.add_rect_with_uv(rect, Rect::from_min_max(uv(x0, 0.0), uv(x0 + fw, th)), w),
        Fit::Tile => {
            let cols = (rect.width() / fw).ceil().min(256.0) as usize;
            let rows = (rect.height() / th).ceil().min(256.0) as usize;
            for r in 0..rows {
                for c in 0..cols {
                    let min = rect.min + Vec2::new(c as f32 * fw, r as f32 * th);
                    let max = Pos2::new((min.x + fw).min(rect.max.x), (min.y + th).min(rect.max.y));
                    let (pw, ph) = (max.x - min.x, max.y - min.y);
                    m.add_rect_with_uv(Rect::from_min_max(min, max), Rect::from_min_max(uv(x0, 0.0), uv(x0 + pw, ph)), w);
                }
            }
        }
        Fit::Nine => {
            let [l, t, r, b] = a.border.map(|v| v as f32);
            let (l, r) = (l.min(fw / 2.0), r.min(fw / 2.0));
            let (t, b) = (t.min(th / 2.0), b.min(th / 2.0));
            // Destination borders: one point per source pixel, shrunk if the slot is small.
            let k = (rect.width() / (l + r).max(1.0)).min(rect.height() / (t + b).max(1.0)).min(1.0);
            let (dl, dr, dt, db) = (l * k, r * k, t * k, b * k);
            let xs = [rect.min.x, rect.min.x + dl, rect.max.x - dr, rect.max.x];
            let ys = [rect.min.y, rect.min.y + dt, rect.max.y - db, rect.max.y];
            let us = [x0, x0 + l, x0 + fw - r, x0 + fw];
            let vs = [0.0, t, th - b, th];
            for j in 0..3 {
                for i in 0..3 {
                    let d = Rect::from_min_max(Pos2::new(xs[i], ys[j]), Pos2::new(xs[i + 1], ys[j + 1]));
                    if d.width() > 0.0 && d.height() > 0.0 {
                        m.add_rect_with_uv(d, Rect::from_min_max(uv(us[i], vs[j]), uv(us[i + 1], vs[j + 1])), w);
                    }
                }
            }
        }
    }
    m
}

/// Reserve a place under the content about to be drawn; fill it with `finish`.
pub fn begin(ui: &egui::Ui) -> egui::layers::ShapeIdx {
    ui.painter().add(Shape::Noop)
}

/// Paint slot `key` over `rect` into the place reserved by `begin` (no-op without art).
pub fn finish(ctx: &Context, layer: egui::LayerId, idx: egui::layers::ShapeIdx, key: &'static str, rect: Rect) {
    let s = get(ctx);
    let shape = s.lock().unwrap().shape(ctx, key, rect, State::Normal);
    if let Some(sh) = shape {
        ctx.layer_painter(layer).set(idx, sh);
    }
}

/// Paint slot `key` directly (on top of whatever is there). Returns whether art was drawn.
pub fn paint(ui: &egui::Ui, key: &'static str, rect: Rect, state: State) -> bool {
    let s = get(ui.ctx());
    let shape = s.lock().unwrap().shape(ui.ctx(), key, rect, state);
    shape.map(|sh| ui.painter().add(sh)).is_some()
}

/// A button dressed by slot `key` when it has art, else the normal egui button.
pub fn button(ui: &mut egui::Ui, key: &'static str, size: Option<Vec2>, selected: bool, enabled: bool, text: impl Into<egui::WidgetText>) -> Response {
    let has = get(ui.ctx()).lock().unwrap().has(key);
    if !has {
        let b = if selected { egui::Button::selectable(true, text) } else { egui::Button::new(text) };
        let r = match size {
            Some(s) => ui.add_enabled_ui(enabled, |ui| ui.add_sized(s, b)).inner,
            None => ui.add_enabled(enabled, b),
        };
        let s = get(ui.ctx());
        let mut s = s.lock().unwrap();
        if s.record {
            s.hits.push((key, r.rect));
        }
        return r;
    }
    let idx = begin(ui);
    let b = egui::Button::new(text).frame(false);
    let r = match size {
        Some(s) => ui.add_enabled_ui(enabled, |ui| ui.add_sized(s, b)).inner,
        None => ui.add_enabled(enabled, b),
    };
    let state = if r.is_pointer_button_down_on() || selected {
        State::Pressed
    } else if r.hovered() {
        State::Hover
    } else {
        State::Normal
    };
    let s = get(ui.ctx());
    let shape = s.lock().unwrap().shape(ui.ctx(), key, r.rect, state);
    if let Some(sh) = shape {
        ui.painter().set(idx, sh);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut m = HashMap::new();
        let mut a = SlotArt::new("install:gfx/interface/x.dds".into());
        a.fit = Fit::Nine;
        a.border = [1, 2, 3, 4];
        a.frames = 3;
        a.frame_hover = 1;
        a.frame_pressed = 2;
        m.insert("topbar_tab".to_string(), a);
        assert_eq!(parse(&write(&m)), m);
    }
}
