//! Start screen (New game / Load game), laid out like the original's so players find things where they
//! expect them, drawn in our own look (D18): bookmarks and saves top left, the playable-countries table
//! under it, a title top centre, the selected country's card and the players list on the right, the
//! bookmark's story bottom centre, Back bottom left and Observe / Play bottom right (D20). The map in the
//! middle is the normal map view; clicking a province selects its owner.
//!
//! It is a mode of `App` (so the map, camera, flags and labels are shared with the game that follows).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Margin, RichText, TextStyle, Vec2};

use crate::ui::{dim, opt_num, pie, App};

#[derive(Debug, Clone)]
pub struct Entry {
    pub title: String,
    pub date: String,
    /// The save the game is built from (the 1836 bookmark uses the start save).
    pub save: Option<PathBuf>,
    /// The save's player country tag.
    pub tag: Option<String>,
    pub story: String,
    pub bookmark: bool,
}

/// What the player chose.
#[derive(Debug, Clone)]
pub struct Launch {
    pub save: PathBuf,
    pub country: u16,
    pub observe: bool,
}

pub struct StartState {
    pub entries: Vec<Entry>,
    pub saves_only: bool,
    pub sel_entry: usize,
    pub country: Option<u16>,
    /// Culture shares of the selected country (name, population).
    cultures: Vec<(String, f64)>,
    pub launch: Option<Launch>,
    pub back: bool,
}

impl StartState {
    pub fn new(entries: Vec<Entry>, saves_only: bool) -> Self {
        let mut s = StartState { entries, saves_only, sel_entry: 0, country: None, cultures: Vec::new(), launch: None, back: false };
        s.sel_entry = s.visible().first().copied().unwrap_or(0);
        s
    }

    pub fn visible(&self) -> Vec<usize> {
        (0..self.entries.len()).filter(|i| !self.saves_only || !self.entries[*i].bookmark).collect()
    }
}

/// `date="1836.1.2"` and `player="ENG"` from the head of a save (plain-text saves).
pub fn peek_save(path: &Path) -> Option<(String, Option<String>)> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 64 * 1024];
    let n = f.read(&mut buf).ok()?;
    let head = String::from_utf8_lossy(&buf[..n]);
    let field = |k: &str| {
        let i = head.find(&format!("{k}="))? + k.len() + 1;
        let rest = head[i..].trim_start().trim_start_matches('"');
        let end = rest.find(|c: char| c == '"' || c.is_whitespace()).unwrap_or(rest.len());
        Some(rest[..end].to_string())
    };
    Some((field("date")?, field("player")))
}

/// "1836.1.2" -> "2 January 1836".
pub fn pretty_date(d: &str) -> String {
    const M: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let p: Vec<&str> = d.split('.').collect();
    match (p.first(), p.get(1).and_then(|m| m.parse::<usize>().ok()), p.get(2)) {
        (Some(y), Some(m), Some(day)) if (1..=12).contains(&m) => format!("{day} {} {y}", M[m - 1]),
        _ => d.to_string(),
    }
}

impl App {
    fn start_select_country(&mut self, c: u16) {
        let Some(st) = self.start.as_mut() else { return };
        st.country = Some(c);
        let w = self.source.world();
        let mut acc: BTreeMap<u16, f64> = BTreeMap::new();
        for p in w.provinces.iter().flatten().filter(|p| p.owner == Some(c)) {
            for pop in &p.pops {
                *acc.entry(pop.culture).or_default() += pop.size;
            }
        }
        let mut v: Vec<(String, f64)> = acc.into_iter().map(|(k, s)| (w.defs.cultures.cultures[k as usize].name.clone(), s)).collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        st.cultures = v;
    }

    fn start_select_entry(&mut self, i: usize) {
        let Some(st) = self.start.as_mut() else { return };
        st.sel_entry = i;
        let tag = st.entries[i].tag.clone();
        if let Some(c) = tag.and_then(|t| self.source.world().tag_index.get(&t).copied()) {
            self.start_select_country(c);
        }
    }

    pub(crate) fn start_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let th = self.theme;
        let em = ui.text_style_height(&TextStyle::Body);
        // Capture mode: preselect a country (GM_UI_START_TAG, default the top-ranked one).
        if self.start.as_ref().is_some_and(|s| s.country.is_none()) {
            let want = std::env::var("GM_UI_START_TAG").ok().and_then(|t| self.source.world().tag_index.get(&t).copied());
            if let Some(c) = want.or_else(|| self.source.data().ledger.first().map(|r| r.country)) {
                self.start_select_country(c);
            }
        }
        let panel = egui::Frame::new().fill(th.bg_panel.gamma_multiply(0.97)).inner_margin(Margin::same(8)).stroke(egui::Stroke::new(1.0, th.edge));
        egui::Panel::left("start_left").resizable(false).exact_size(em * 21.0).frame(panel).show(ui, |ui| self.start_left(ui, em));
        egui::Panel::right("start_right").resizable(false).exact_size(em * 19.0).frame(panel).show(ui, |ui| self.start_right(ui, em));
        let avail = ui.available_rect_before_wrap();
        egui::CentralPanel::no_frame().show(ui, |ui| self.map_view(ui));
        // A click on the map selects the province's owner.
        if let Some(id) = self.selected.take() {
            if let Some(o) = self.source.world().province(id).and_then(|p| p.owner) {
                self.start_select_country(o);
            }
        }
        let Some(st) = self.start.as_ref() else { return };
        let entry = st.entries.get(st.sel_entry).cloned();
        let year = entry.as_ref().and_then(|e| e.date.split('.').next().map(str::to_string)).unwrap_or_default();
        // Title, top centre.
        let deco = egui::Frame::new().fill(th.bg_deep.gamma_multiply(0.9)).stroke(egui::Stroke::new(1.0, th.gold.gamma_multiply(0.7))).inner_margin(Margin::symmetric(24, 6)).corner_radius(4);
        egui::Area::new(egui::Id::new("start_title")).anchor(Align2::CENTER_TOP, Vec2::new(avail.center().x - ctx.content_rect().center().x, avail.top() + 8.0)).interactable(false).show(&ctx, |ui| {
            deco.show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(format!("The world {year} AD")).family(egui::FontFamily::Name(crate::theme::SERIF.into())).size(em * 2.0).color(th.ink));
                    ui.label(dim(&th, "Click on the map to select a nation"));
                });
            });
        });
        // Story, bottom centre.
        if let Some(e) = &entry {
            // Fits between Back and Observe/Play (two buttons of 8 em each side plus margins).
            let w = (avail.width() - 2.0 * (em * 16.0 + 40.0)).clamp(em * 14.0, em * 40.0);
            egui::Area::new(egui::Id::new("start_story")).anchor(Align2::CENTER_BOTTOM, Vec2::new(avail.center().x - ctx.content_rect().center().x, -8.0)).show(&ctx, |ui| {
                deco.show(ui, |ui| {
                    ui.set_width(w);
                    egui::ScrollArea::vertical().max_height(em * 7.0).show(ui, |ui| {
                        ui.label(RichText::new(&e.story).family(egui::FontFamily::Name(crate::theme::SERIF.into())).size(em * 1.1));
                    });
                });
            });
        }
        // Back, bottom left of the map; Observe and Play bottom right.
        let bsize = Vec2::new(em * 8.0, em * 2.6);
        let can_play = entry.as_ref().is_some_and(|e| e.save.is_some());
        let country = st.country;
        let mut back = false;
        let mut launch: Option<bool> = None;
        egui::Area::new(egui::Id::new("start_back")).fixed_pos(avail.left_bottom() + Vec2::new(10.0, -bsize.y - 10.0)).show(&ctx, |ui| {
            if self.art.menu_button(ui, &th, "Back", bsize, true).clicked() {
                back = true;
            }
        });
        egui::Area::new(egui::Id::new("start_play")).fixed_pos(avail.right_bottom() + Vec2::new(-bsize.x * 2.0 - 20.0, -bsize.y - 10.0)).show(&ctx, |ui| {
            ui.horizontal(|ui| {
                let r = self.art.menu_button(ui, &th, "Observe", bsize, can_play).on_hover_text("Watch the world run without playing a country. You can switch the watched country at any time.");
                if r.clicked() {
                    launch = Some(true);
                }
                let r = self.art.menu_button(ui, &th, "Play", bsize, can_play && country.is_some());
                if r.clicked() {
                    launch = Some(false);
                }
            });
        });
        if !can_play {
            egui::Area::new(egui::Id::new("start_noplay")).anchor(Align2::RIGHT_BOTTOM, Vec2::new(-10.0, -bsize.y - 18.0)).interactable(false).show(&ctx, |ui| {
                ui.label(dim(&th, "No start save found (start/1836.v2)").small());
            });
        }
        let st = self.start.as_mut().unwrap();
        st.back |= back;
        if let (Some(observe), Some(save)) = (launch, entry.and_then(|e| e.save)) {
            let fallback = self.source.data().ledger.first().map_or(0, |r| r.country);
            st.launch = Some(Launch { save, country: country.unwrap_or(fallback), observe });
        }
        self.handle_screenshot(&ctx);
    }

    fn start_left(&mut self, ui: &mut egui::Ui, em: f32) {
        let th = self.theme;
        let Some(st) = self.start.as_ref() else { return };
        let vis = st.visible();
        let sel = st.sel_entry;
        let entries = st.entries.clone();
        let sel_country = st.country;
        ui.label(RichText::new(if st.saves_only { "Saved games" } else { "Bookmarks and saves" }).color(th.gold).strong());
        ui.separator();
        let mut pick_entry = None;
        egui::ScrollArea::vertical().id_salt("entries").max_height(ui.available_height() * 0.38).auto_shrink([false, true]).show(ui, |ui| {
            if vis.is_empty() {
                ui.label(dim(&th, "No saves found.").italics());
            }
            for i in vis {
                let e = &entries[i];
                let r = ui
                    .horizontal(|ui| {
                        match &e.tag {
                            Some(t) => self.flags.show(ui, t, em * 1.6),
                            None => {
                                ui.allocate_space(Vec2::new(em * 2.4, em * 1.6));
                            }
                        }
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let t = RichText::new(&e.title).strong();
                            ui.add(egui::Label::new(if i == sel { t.color(th.gold) } else { t }).truncate());
                            ui.label(dim(&th, pretty_date(&e.date)).small());
                        });
                    })
                    .response
                    .interact(egui::Sense::click());
                if i == sel {
                    ui.painter().rect_stroke(r.rect.expand(2.0), 3, egui::Stroke::new(1.0, th.gold.gamma_multiply(0.6)), egui::StrokeKind::Outside);
                }
                if r.clicked() {
                    pick_entry = Some(i);
                }
                ui.add_space(3.0);
            }
        });
        if let Some(i) = pick_entry {
            self.start_select_entry(i);
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Playable countries").color(th.gold).strong());
        ui.separator();
        let rows = self.source.ranking_rows();
        let mut pick = None;
        egui::ScrollArea::vertical().id_salt("countries").auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("country_table").num_columns(5).striped(true).spacing(Vec2::new(6.0, 3.0)).show(ui, |ui| {
                for h in ["Country", "Rank", "Prestige", "Industry", "Military"] {
                    ui.label(dim(&th, h).small());
                }
                ui.end_row();
                for r in rows.iter() {
                    let c = self.source.country(r.country);
                    let resp = ui
                        .horizontal(|ui| {
                            ui.set_width(em * 9.0);
                            self.flags.show(ui, &c.tag, em * 1.0);
                            let t = RichText::new(&c.name);
                            ui.add(egui::Label::new(if sel_country == Some(r.country) { t.color(th.gold).strong() } else { t }).truncate());
                        })
                        .response
                        .interact(egui::Sense::click());
                    if resp.clicked() {
                        pick = Some(r.country);
                    }
                    ui.label(r.rank.to_string());
                    ui.label(opt_num(r.prestige));
                    ui.label(opt_num(r.industrial));
                    ui.label(opt_num(r.military));
                    ui.end_row();
                }
            });
        });
        if let Some(c) = pick {
            self.start_select_country(c);
            // Centre the map on the country's capital.
            if let (Some(cap), Some(mut cam)) = (self.source.world().country(c).history.capital, self.cam) {
                let map = &self.source.world().map;
                if let Some(&(x, y)) = map.centroid.get(cap as usize) {
                    if let Some((_, (w, h), ..)) = self.last_render {
                        cam.center_on(map, x as f64, y as f64, w, h);
                        self.cam = Some(cam);
                    }
                }
            }
        }
    }

    fn start_right(&mut self, ui: &mut egui::Ui, em: f32) {
        let th = self.theme;
        let Some(st) = self.start.as_ref() else { return };
        let Some(c) = st.country else {
            ui.label(dim(&th, "Select a country on the map or in the table.").italics());
            return;
        };
        let cultures = st.cultures.clone();
        let f = self.source.country(c);
        let s = self.source.country_summary(c);
        let row = self.source.ranking_rows().into_iter().find(|r| r.country == c);
        ui.horizontal(|ui| {
            self.flags.show(ui, &f.tag, em * 3.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(&f.name).family(egui::FontFamily::Name(crate::theme::SERIF.into())).size(em * 1.5).color(th.ink));
                let gp = row.as_ref().is_some_and(|r| r.rank <= 8);
                ui.label(dim(&th, if gp { "Great power" } else { "Nation" }));
                ui.label(dim(&th, s.government.clone().unwrap_or_else(|| "-".into()).replace('_', " ")));
            });
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Rank").color(th.gold).strong());
        ui.separator();
        egui::Grid::new("card_ranks").num_columns(2).show(ui, |ui| {
            for (k, v) in [
                ("Overall", row.as_ref().map_or("-".into(), |r| format!("#{}", r.rank))),
                ("Prestige", opt_num(s.prestige)),
                ("Industry", opt_num(s.industry_score)),
                ("Military", opt_num(s.military_score)),
            ] {
                ui.label(dim(&th, k));
                ui.label(v);
                ui.end_row();
            }
        });
        ui.add_space(4.0);
        let pop: f64 = cultures.iter().map(|c| c.1).sum();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Population").color(th.gold).strong());
            ui.label(wiktoria_render::fmt_int(pop));
        });
        ui.separator();
        ui.horizontal(|ui| {
            let slices: Vec<f64> = cultures.iter().take(7).map(|c| c.1).chain([cultures.iter().skip(7).map(|c| c.1).sum()]).collect();
            pie(ui, &th, em * 2.6, &slices);
            ui.vertical(|ui| {
                for (n, v) in cultures.iter().take(4) {
                    ui.label(RichText::new(format!("{} {:.0}%", n.replace('_', " "), 100.0 * v / pop.max(1.0))).small());
                }
            });
        });
        ui.add_space(4.0);
        let names = |v: &[u16]| if v.is_empty() { "-".to_string() } else { v.iter().map(|c| self.source.country(*c).name).collect::<Vec<_>>().join(", ") };
        egui::Grid::new("card_dip").num_columns(2).show(ui, |ui| {
            ui.label(dim(&th, "Wars"));
            ui.label(match s.at_war { Some(true) => "at war", _ => "none" });
            ui.end_row();
            ui.label(dim(&th, "Allies"));
            ui.add(egui::Label::new(names(&s.allies)).wrap());
            ui.end_row();
            ui.label(dim(&th, "Overlord"));
            ui.label(s.overlord.map_or("-".into(), |o| self.source.country(o).name));
            ui.end_row();
        });
        ui.add_space(10.0);
        ui.label(RichText::new("Players: 1").color(th.gold).strong());
        ui.separator();
        ui.horizontal(|ui| {
            self.flags.show(ui, &f.tag, em * 1.2);
            ui.label(format!("Player ({})", f.name));
        });
        ui.label(dim(&th, "Play takes this country; Observe watches the world, and the watched country can be switched in game.").small());
    }
}
