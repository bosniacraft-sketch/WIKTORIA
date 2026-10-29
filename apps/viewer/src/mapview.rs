//! Map view and screenshot capture (an extra impl block of `App`).

use eframe::egui::{self, Color32, ColorImage, Context, Margin, Sense, TextureOptions, Vec2};
use wiktoria_render::{pick, render, Camera, RenderOptions};

use crate::ui::App;

impl App {
    pub(crate) fn map_view(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let ppp = ui.ctx().pixels_per_point();
        let (w, h) = (((rect.width() * ppp) as usize).clamp(16, 4096), ((rect.height() * ppp) as usize).clamp(16, 4096));
        let world = self.source.world();
        let map = &world.map;

        let mut cam = match self.cam {
            Some(c) => c,
            None => {
                let mut c = Camera::fit(map, w, h);
                if let Some(b) = world.start_bookmark() {
                    c.zoom = 0.75 * (h as f64 / 900.0).max(0.5);
                    c.center_on(map, b.camera.0, map.height as f64 - b.camera.1, w, h);
                }
                // Capture mode: GM_UI_ZOOM_MAP=<factor> scales the initial map zoom around the view centre.
                if let Some(f) = std::env::var("GM_UI_ZOOM_MAP").ok().and_then(|s| s.parse::<f64>().ok()) {
                    c.zoom_at(map, w as f64 / 2.0, h as f64 / 2.0, f, h);
                }
                c
            }
        };
        let to_px = |p: egui::Pos2| ((p.x - rect.min.x) as f64 * ppp as f64, (p.y - rect.min.y) as f64 * ppp as f64);

        if resp.dragged() {
            let d = resp.drag_delta();
            cam.pan(map, (d.x * ppp) as f64, (d.y * ppp) as f64, h);
        }
        if resp.hovered() {
            let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                if let Some(p) = resp.hover_pos() {
                    let (sx, sy) = to_px(p);
                    cam.zoom_at(map, sx, sy, (scroll as f64 * 0.0025).exp(), h);
                }
            }
        }
        self.hover = resp.hover_pos().and_then(|p| {
            let (sx, sy) = to_px(p);
            pick(map, &cam, sx, sy)
        });
        let clicked = if resp.clicked() { self.hover } else { None };
        self.cam = Some(cam);

        let key = (self.mode, self.source.revision());
        if key != self.palette_key {
            self.palette = self.source.palette(self.mode);
            self.palette_key = key;
        }
        let state = (cam, (w, h), self.borders, self.selected, key);
        if self.last_render.as_ref() != Some(&state) || self.tex.is_none() {
            let mut buf = vec![0u32; w * h];
            render(&self.source.world().map, &self.palette, &cam, &RenderOptions { borders: self.borders, highlight: self.selected }, &mut buf, w, h);
            let pixels = buf.into_iter().map(|c| Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)).collect();
            let img = ColorImage::new([w, h], pixels);
            match &mut self.tex {
                Some(t) => t.set(img, TextureOptions::NEAREST),
                None => self.tex = Some(ui.ctx().load_texture("map", img, TextureOptions::NEAREST)),
            }
            self.last_render = Some(state);
        }
        if let Some(t) = &self.tex {
            ui.painter().image(t.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        // Country names along each country's shape (political mode). Rebuilt only when ownership changes.
        if self.show_labels && self.mode == crate::watch::ViewMode::Political {
            let src = &self.source;
            // Laid out for the visible map area (the central panel rect, so never under side panels).
            let view = crate::labels::View { rect, cam_x: cam.x as f32, cam_y: cam.y as f32, scale: cam.zoom as f32 / ppp };
            self.labels.update(src.world(), &self.palette.owner_key, view, |c| src.country(c).name);
            let painter = ui.painter_at(rect);
            crate::labels::draw(&painter, &self.labels.labels, self.theme.label_ink, self.theme.label_halo);
        }
        if let Some(id) = self.hover {
            if let Some(p) = self.source.world().province(id) {
                let owner = self.source.province(id).and_then(|f| f.owner).map(|o| o.name).unwrap_or_else(|| "unowned".into());
                let text = if p.is_sea { format!("{} (sea)", p.name) } else { format!("{}  |  {}", p.name, owner) };
                let (bg, edge) = (self.theme.bg_panel, self.theme.edge);
                egui::Area::new(egui::Id::new("hover_tip")).fixed_pos(rect.right_top() + Vec2::new(-260.0, 8.0)).interactable(false).show(ui.ctx(), |ui| {
                    egui::Frame::new().fill(bg.gamma_multiply(0.92)).stroke(egui::Stroke::new(1.0, edge)).inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                        ui.add(egui::Label::new(text).extend());
                    });
                });
            }
        }
        if let Some(id) = clicked {
            self.select(id);
        }
        let _ = Sense::hover();
    }

    pub(crate) fn handle_screenshot(&mut self, ctx: &Context) {
        let Some(path) = self.shot_path.clone() else { return };
        self.frame += 1;
        if !self.source.capture_ready() {
            self.frame = 0;
        }
        if self.frame >= 10 && !self.shot_requested {
            self.shot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            if let Some(dir) = std::path::Path::new(&path).parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match image::save_buffer(&path, img.as_raw(), img.width() as u32, img.height() as u32, image::ColorType::Rgba8) {
                Ok(()) => println!("screenshot written to {path}"),
                Err(e) => eprintln!("screenshot failed: {e}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            ctx.request_repaint();
        }
    }
}
