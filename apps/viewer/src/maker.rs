//! UI maker (`viewer --ui-maker`, or "UI maker" on the main menu): the owner dresses our UI slots with
//! pictures from his own Victoria 2 install. Left: the install's gfx/**/*.dds|tga with thumbnails.
//! Centre: our real game UI over a static world, every slot outlined and named; click one to select it.
//! Right: the selected slot's picture and options. Saved to `themes/original/slots.toml` as `install:`
//! paths. Nothing from the install is copied anywhere.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use eframe::egui::{self, Color32, ColorImage, RichText, TextureHandle, TextureOptions, Vec2};

use crate::dds::Rgba;
use crate::slots::{self, Fit, SlotArt, SLOTS};
use crate::ui::App;

pub const THEME: &str = "original";
const THUMB: usize = 72;

pub struct Maker {
    install: Option<PathBuf>,
    base: PathBuf,
    pub app: Option<Box<App>>,
    files: Vec<String>,
    filter: String,
    thumbs: HashMap<String, Option<TextureHandle>>,
    asked: HashSet<String>,
    req: Sender<(PathBuf, String)>,
    got: Receiver<(String, Option<Rgba>)>,
    selected: Option<&'static str>,
    undo: Vec<(String, Option<SlotArt>)>,
    menu_preview: bool,
    show_outlines: bool,
    msg: String,
    menu_art: Option<crate::theme::Art>,
    pub back: bool,
}

fn scan(install: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![install.join("gfx")];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dds") || x.eq_ignore_ascii_case("tga")) {
                if let Ok(rel) = p.strip_prefix(install) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    out.sort_by_key(|s| s.to_lowercase());
    out
}

/// Nearest-neighbour shrink so the longest side is at most `max`.
fn shrink(img: Rgba, max: usize) -> Rgba {
    let k = (img.width.max(img.height) as f32 / max as f32).max(1.0);
    if k <= 1.0 {
        return img;
    }
    let (w, h) = (((img.width as f32 / k) as usize).max(1), ((img.height as f32 / k) as usize).max(1));
    let mut px = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = (((x as f32 * k) as usize).min(img.width - 1), ((y as f32 * k) as usize).min(img.height - 1));
            px[(y * w + x) * 4..][..4].copy_from_slice(&img.px[(sy * img.width + sx) * 4..][..4]);
        }
    }
    Rgba { width: w, height: h, px }
}

fn thumb_worker() -> (Sender<(PathBuf, String)>, Receiver<(String, Option<Rgba>)>) {
    let (req, rx) = channel::<(PathBuf, String)>();
    let (tx, got) = channel();
    std::thread::Builder::new()
        .name("thumbs".into())
        .spawn(move || {
            while let Ok((p, rel)) = rx.recv() {
                let img = crate::dds::load_any(&p).map(|i| shrink(i, THUMB * 2));
                if tx.send((rel, img)).is_err() {
                    break;
                }
            }
        })
        .expect("spawn thumbs thread");
    (req, got)
}

/// Where `slots.toml` is written: the source tree's `themes/original/` when running from it (so it can be
/// committed), else `themes/original/` next to the exe.
pub fn save_path(base: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = if src.is_dir() { src.join("themes").join(THEME) } else { base.join("themes").join(THEME) };
    dir.join("slots.toml")
}

impl Maker {
    pub fn new(ctx: &egui::Context, install: Option<PathBuf>, base: PathBuf) -> Self {
        slots::install_for(ctx, &base, Some(THEME), install.as_deref());
        let (req, got) = thumb_worker();
        let files = install.as_deref().map(scan).unwrap_or_default();
        Maker {
            install,
            base,
            app: None,
            files,
            filter: String::new(),
            thumbs: HashMap::new(),
            asked: HashSet::new(),
            req,
            got,
            selected: Some("topbar_bg"),
            undo: Vec::new(),
            menu_preview: false,
            show_outlines: true,
            msg: String::new(),
            menu_art: None,
            back: false,
        }
    }

    pub fn needs_world(&self) -> bool {
        self.app.is_none() && self.install.is_some()
    }

    pub fn install(&self) -> Option<&Path> {
        self.install.as_deref()
    }

    fn set_install(&mut self, ctx: &egui::Context, p: PathBuf) {
        self.files = scan(&p);
        slots::get(ctx).lock().unwrap().install = Some(p.clone());
        self.install = Some(p);
        self.thumbs.clear();
        self.asked.clear();
    }

    fn assign(&mut self, ctx: &egui::Context, key: &str, art: Option<SlotArt>) {
        let s = slots::get(ctx);
        let mut s = s.lock().unwrap();
        let prev = match art {
            Some(a) => s.map.insert(key.to_string(), a),
            None => s.map.remove(key),
        };
        self.undo.push((key.to_string(), prev));
    }

    fn save(&mut self, ctx: &egui::Context) {
        let text = slots::write(&slots::get(ctx).lock().unwrap().map);
        let p = save_path(&self.base);
        let r = p.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&p, text));
        self.msg = match r {
            Ok(()) => format!("Saved {}", p.display()),
            Err(e) => format!("Save failed: {e}"),
        };
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        // Thumbnails arriving from the worker.
        while let Ok((rel, img)) = self.got.try_recv() {
            let tex = img.map(|i| ctx.load_texture(format!("thumb:{rel}"), ColorImage::from_rgba_unmultiplied([i.width, i.height], &i.px), TextureOptions::LINEAR));
            self.thumbs.insert(rel, tex);
        }
        if self.asked.len() > self.thumbs.len() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        egui::Panel::left("maker_files").resizable(true).default_size(320.0).show(ui, |ui| self.file_list(ui));
        egui::Panel::right("maker_slot").resizable(false).exact_size(300.0).show(ui, |ui| self.inspector(ui));
        egui::Panel::top("maker_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("UI maker").strong());
                ui.separator();
                ui.selectable_value(&mut self.menu_preview, false, "Game screen");
                ui.selectable_value(&mut self.menu_preview, true, "Main menu");
                ui.checkbox(&mut self.show_outlines, "Outlines");
                ui.separator();
                if ui.button("Save").clicked() {
                    self.save(&ctx);
                }
                if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo")).clicked() {
                    if let Some((k, prev)) = self.undo.pop() {
                        let s = slots::get(&ctx);
                        let mut s = s.lock().unwrap();
                        match prev {
                            Some(a) => s.map.insert(k, a),
                            None => s.map.remove(&k),
                        };
                    }
                }
                if ui.button("Back").clicked() {
                    self.back = true;
                }
                ui.label(RichText::new(&self.msg).small());
            });
        });
        // Preview with slot recording on.
        let slots_ref = slots::get(&ctx);
        {
            let mut s = slots_ref.lock().unwrap();
            s.record = true;
            s.hits.clear();
        }
        let area = ui.available_rect_before_wrap();
        if self.menu_preview {
            self.menu_mock(ui, area);
        } else if let Some(app) = &mut self.app {
            app.preview_window();
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
            child.set_clip_rect(area);
            app.frame_ui(&mut child);
        } else {
            ui.centered_and_justified(|ui| {
                if self.install.is_some() {
                    ui.spinner();
                } else {
                    ui.label("Pick the Victoria 2 folder on the left.");
                }
            });
        }
        let hits = {
            let mut s = slots_ref.lock().unwrap();
            s.record = false;
            std::mem::take(&mut s.hits)
        };
        self.overlay(&ctx, area, &hits);
    }

    fn menu_mock(&mut self, ui: &mut egui::Ui, area: egui::Rect) {
        let painter = ui.painter_at(area);
        let th = crate::theme::Theme::default();
        let base = self.base.clone();
        let art = self.menu_art.get_or_insert_with(|| crate::theme::Art::load(&base, Some(THEME)).1);
        art.menu_background(&painter, area, &th, None);
        let em = ui.text_style_height(&egui::TextStyle::Body);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_center_size(area.center(), Vec2::new(em * 16.0, em * 14.0))));
        for l in ["New game", "Load game", "Settings", "Exit"] {
            art.menu_button(&mut child, &th, l, Vec2::new(em * 16.0, em * 2.8), true);
            child.add_space(em * 0.6);
        }
    }

    /// Outline and name every slot drawn this frame; a click (on the foreground) selects the smallest one
    /// under the pointer.
    fn overlay(&mut self, ctx: &egui::Context, area: egui::Rect, hits: &[(&'static str, egui::Rect)]) {
        let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("maker_overlay"));
        let painter = ctx.layer_painter(layer).with_clip_rect(area);
        let mut labelled = HashSet::new();
        if self.show_outlines {
            for (k, r) in hits {
                let sel = self.selected == Some(*k);
                let col = if sel { Color32::from_rgb(255, 210, 60) } else { Color32::from_rgba_unmultiplied(80, 200, 255, 160) };
                painter.rect_stroke(*r, 0.0, egui::Stroke::new(if sel { 2.5 } else { 1.0 }, col), egui::StrokeKind::Inside);
                if labelled.insert(*k) {
                    let font = egui::FontId::proportional(11.0);
                    let g = painter.layout_no_wrap(slots::slot_name(k).to_string(), font, Color32::BLACK);
                    let bg = egui::Rect::from_min_size(r.min, g.size() + Vec2::new(6.0, 2.0));
                    painter.rect_filled(bg, 2.0, col);
                    painter.galley(r.min + Vec2::new(3.0, 1.0), g, Color32::BLACK);
                }
            }
        }
        // Plain click selects the slot under the pointer (Alt+click only uses the UI).
        let pos = ctx.input(|i| if i.pointer.primary_clicked() && !i.modifiers.alt { i.pointer.interact_pos() } else { None });
        if let Some(p) = pos.filter(|p| area.contains(*p)) {
            if let Some((k, _)) = hits.iter().filter(|(_, r)| r.contains(p)).min_by(|a, b| a.1.area().total_cmp(&b.1.area())) {
                self.selected = Some(k);
            }
        }
    }

    fn file_list(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.heading("Install pictures");
        match &self.install {
            Some(p) => {
                ui.label(RichText::new(p.display().to_string()).small());
            }
            None => {
                ui.label("Victoria 2 folder not found.");
            }
        }
        if ui.button("Pick Victoria 2 folder...").clicked() {
            if let Some(p) = rfd::FileDialog::new().set_title("Victoria 2 folder (the one with gfx/)").pick_folder() {
                self.set_install(&ctx, p);
            }
        }
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Search gfx/... (e.g. topbar)"));
        let f = self.filter.to_lowercase();
        let list: Vec<String> = self.files.iter().filter(|s| f.is_empty() || s.to_lowercase().contains(&f)).cloned().collect();
        ui.label(RichText::new(format!("{} files. Click one to assign it to the selected slot.", list.len())).small());
        ui.separator();
        let row_h = THUMB as f32 + 6.0;
        let current = self.selected.and_then(|k| slots::get(&ctx).lock().unwrap().map.get(k).map(|a| a.image.clone()));
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_h, list.len(), |ui, range| {
            for rel in &list[range] {
                if !self.asked.contains(rel) {
                    if let Some(i) = &self.install {
                        let _ = self.req.send((i.join(rel), rel.clone()));
                    }
                    self.asked.insert(rel.clone());
                }
                let value = format!("{}{rel}", slots::PREFIX);
                let is_cur = current.as_deref() == Some(value.as_str());
                let resp = ui
                    .horizontal(|ui| {
                        ui.set_height(row_h - 4.0);
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(THUMB as f32), egui::Sense::hover());
                        ui.painter().rect_filled(r, 2.0, Color32::from_gray(40));
                        if let Some(Some(t)) = self.thumbs.get(rel) {
                            let s = t.size_vec2();
                            let k = (THUMB as f32 / s.x).min(THUMB as f32 / s.y);
                            let ir = egui::Rect::from_center_size(r.center(), s * k);
                            ui.painter().image(t.id(), ir, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        let dims = self.thumbs.get(rel).map_or(String::new(), |t| if t.is_none() { "cannot decode".into() } else { String::new() });
                        ui.vertical(|ui| {
                            ui.add(egui::Label::new(RichText::new(rel.rsplit('/').next().unwrap_or(rel)).strong()).truncate());
                            ui.add(egui::Label::new(RichText::new(rel).small()).truncate());
                            if !dims.is_empty() {
                                ui.label(RichText::new(dims).small().color(Color32::LIGHT_RED));
                            }
                        });
                    })
                    .response
                    .interact(egui::Sense::click());
                if is_cur {
                    ui.painter().rect_stroke(resp.rect, 2.0, egui::Stroke::new(2.0, Color32::from_rgb(255, 210, 60)), egui::StrokeKind::Inside);
                } else if resp.hovered() {
                    ui.painter().rect_stroke(resp.rect, 2.0, egui::Stroke::new(1.0, Color32::GRAY), egui::StrokeKind::Inside);
                }
                if resp.clicked() {
                    if let Some(k) = self.selected {
                        let old = slots::get(&ctx).lock().unwrap().map.get(k).cloned();
                        // Keep the slot's options when swapping pictures.
                        let mut a = old.unwrap_or_else(|| SlotArt::new(String::new()));
                        a.image = value.clone();
                        self.assign(&ctx, k, Some(a));
                    }
                }
            }
        });
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.heading("Slots");
        let assigned: HashSet<String> = slots::get(&ctx).lock().unwrap().map.keys().cloned().collect();
        egui::ScrollArea::vertical().id_salt("slot_list").max_height(260.0).show(ui, |ui| {
            for (k, name) in SLOTS {
                let mark = if assigned.contains(*k) { "\u{25cf} " } else { "\u{25cb} " };
                if ui.selectable_label(self.selected == Some(*k), format!("{mark}{name}")).clicked() {
                    self.selected = Some(k);
                }
            }
        });
        ui.separator();
        let Some(k) = self.selected else {
            ui.label("Click a slot in the preview.");
            return;
        };
        ui.label(RichText::new(slots::slot_name(k)).strong());
        ui.label(RichText::new(format!("key: {k}")).small());
        let Some(mut a) = slots::get(&ctx).lock().unwrap().map.get(k).cloned() else {
            ui.label("No picture: built-in drawing. Click a picture on the left.");
            return;
        };
        ui.add(egui::Label::new(RichText::new(&a.image).small()).wrap());
        let before = a.clone();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut a.fit, Fit::Stretch, "Stretch");
            ui.selectable_value(&mut a.fit, Fit::Nine, "9-slice");
            ui.selectable_value(&mut a.fit, Fit::Tile, "Tile");
        });
        if a.fit == Fit::Nine {
            for (i, l) in ["Left", "Top", "Right", "Bottom"].iter().enumerate() {
                ui.add(egui::Slider::new(&mut a.border[i], 0..=128).text(*l));
            }
        }
        ui.add(egui::Slider::new(&mut a.frames, 1..=16).text("Frames in strip"));
        let maxf = a.frames - 1;
        ui.add_enabled(maxf > 0, egui::Slider::new(&mut a.frame_normal, 0..=maxf).text("Normal frame"));
        ui.add_enabled(maxf > 0, egui::Slider::new(&mut a.frame_hover, 0..=maxf).text("Hover frame"));
        ui.add_enabled(maxf > 0, egui::Slider::new(&mut a.frame_pressed, 0..=maxf).text("Pressed frame"));
        if a != before {
            // Slider drags change in place (no undo entry per pixel); assignments and clears are undoable.
            slots::get(&ctx).lock().unwrap().map.insert(k.to_string(), a);
        }
        ui.add_space(6.0);
        if ui.button("Clear slot").clicked() {
            self.assign(&ctx, k, None);
        }
    }
}
