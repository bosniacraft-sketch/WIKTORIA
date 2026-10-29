//! egui front end laid out per docs/UI-LAYOUT.md: top bar, province window bottom left, outliner right,
//! message log bottom, centred country windows, pan/zoom map.
//! Our own look (D18/D19): colours and art come from `theme.rs`, never from the game's art.
//! Flags are the one exception to "no images": they are read from the install at runtime, and like every
//! image they are fitted into a slot sized from the text height (`image_in_slot`).

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui::{
    self, Align2, Color32, ColorImage, Context, Margin, RichText, TextStyle, TextureHandle, TextureOptions, Vec2,
};
use wiktoria_render::{fmt_int, Camera};

use crate::keys::{Action, Bindings};
use crate::slots;

/// Run `f` and paint slot `key` under what it drew. `full` = the whole space given (panels), else just
/// what the content used (windows). `pad` grows the rect over the frame margin.
pub(crate) fn dressed<R>(ui: &mut egui::Ui, key: &'static str, full: bool, pad: f32, f: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let idx = slots::begin(ui);
    let r = f(ui);
    let rect = if full { ui.max_rect() } else { ui.min_rect() };
    slots::finish(ui.ctx(), ui.layer_id(), idx, key, rect.expand(pad));
    r
}
use crate::theme::{image_in_slot, Art, Theme};
use crate::watch::{CountryFacts, CountrySummary, OutlinerKind, ProvinceFacts, ViewMode, WatchSource};

/// Country windows plus the ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Win {
    Production,
    Budget,
    Technology,
    Politics,
    Population,
    Trade,
    Diplomacy,
    Military,
    Ledger,
}

impl Win {
    pub const TOP_BAR: [Win; 8] =
        [Win::Production, Win::Budget, Win::Technology, Win::Politics, Win::Population, Win::Trade, Win::Diplomacy, Win::Military];

    pub fn label(self) -> &'static str {
        match self {
            Win::Production => "Production",
            Win::Budget => "Budget",
            Win::Technology => "Technology",
            Win::Politics => "Politics",
            Win::Population => "Population",
            Win::Trade => "Trade",
            Win::Diplomacy => "Diplomacy",
            Win::Military => "Military",
            Win::Ledger => "Ledger",
        }
    }

    fn parse(s: &str) -> Option<Win> {
        let s = s.trim().to_lowercase();
        Win::TOP_BAR.iter().chain([&Win::Ledger]).copied().find(|w| w.label().to_lowercase() == s)
    }
}

const PIE_COLORS: [Color32; 8] = [
    Color32::from_rgb(176, 126, 62),
    Color32::from_rgb(98, 130, 112),
    Color32::from_rgb(150, 82, 74),
    Color32::from_rgb(110, 110, 156),
    Color32::from_rgb(168, 154, 92),
    Color32::from_rgb(86, 138, 150),
    Color32::from_rgb(140, 100, 134),
    Color32::from_rgb(120, 120, 120),
];

pub(crate) fn apply_style(ctx: &Context, t: &Theme) {
    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(t.ink);
    v.panel_fill = t.bg_panel;
    v.window_fill = t.bg_panel;
    v.window_stroke = egui::Stroke::new(1.0, t.edge);
    v.extreme_bg_color = t.bg_deep;
    v.faint_bg_color = t.bg_panel.gamma_multiply(1.15);
    v.hyperlink_color = t.gold;
    v.selection.bg_fill = t.selection;
    v.selection.stroke = egui::Stroke::new(1.0, t.ink);
    let r = egui::CornerRadius::same(3);
    for (w, fill) in [
        (&mut v.widgets.noninteractive, t.bg_panel),
        (&mut v.widgets.inactive, t.bg_widget),
        (&mut v.widgets.hovered, t.bg_hover),
        (&mut v.widgets.active, t.bg_active),
        (&mut v.widgets.open, t.bg_widget),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.corner_radius = r;
        w.bg_stroke = egui::Stroke::new(1.0, t.edge);
        w.fg_stroke = egui::Stroke::new(1.0, t.ink);
    }
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 5.0);
        s.spacing.button_padding = Vec2::new(10.0, 4.0);
        let b = t.font_size;
        for (ts, size) in [
            (TextStyle::Small, b * 0.8),
            (TextStyle::Body, b),
            (TextStyle::Button, b),
            (TextStyle::Monospace, b),
            (TextStyle::Heading, b * 1.35),
        ] {
            if let Some(f) = s.text_styles.get_mut(&ts) {
                f.size = size;
            }
        }
    });
}

/// Fonts: egui's defaults, the theme's body font if any, and the serif family (title, map labels).
pub(crate) fn install_fonts(ctx: &Context, art: &Art) {
    let mut fonts = egui::FontDefinitions::default();
    if let Some(bytes) = art.font_file.as_ref().and_then(|p| std::fs::read(p).ok()) {
        fonts.font_data.insert("theme_font".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
        fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "theme_font".into());
    }
    fonts.font_data.insert(crate::theme::SERIF.into(), std::sync::Arc::new(egui::FontData::from_owned(art.serif_font())));
    let mut serif = vec![crate::theme::SERIF.to_string()];
    serif.extend(fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default());
    fonts.families.insert(egui::FontFamily::Name(crate::theme::SERIF.into()), serif);
    ctx.set_fonts(fonts);
}

#[allow(dead_code)]
fn load_font(ctx: &Context, art: &Art) {
    let Some(p) = &art.font_file else { return };
    let Ok(bytes) = std::fs::read(p) else { return };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("theme_font".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
    for fam in [egui::FontFamily::Proportional] {
        fonts.families.entry(fam).or_default().insert(0, "theme_font".into());
    }
    ctx.set_fonts(fonts);
}

/// Flag textures read from `<install>/gfx/flags/<TAG>.tga` on first use.
pub(crate) struct Flags {
    dir: PathBuf,
    cache: HashMap<String, Option<TextureHandle>>,
}

impl Flags {
    fn get(&mut self, ctx: &Context, tag: &str) -> Option<TextureHandle> {
        self.cache
            .entry(tag.to_string())
            .or_insert_with(|| {
                let img = image::open(self.dir.join(format!("{tag}.tga"))).ok()?.to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let ci = ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                Some(ctx.load_texture(format!("flag_{tag}"), ci, TextureOptions::LINEAR))
            })
            .clone()
    }

    /// Draw the flag fitted into a slot `height` tall (3:2 slot). The picture's own size is never read.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, tag: &str, height: f32) {
        let ctx = ui.ctx().clone();
        let slot = Vec2::new(height * 1.5, height);
        match self.get(&ctx, tag) {
            Some(t) => image_in_slot(ui, &t, slot),
            None => {
                ui.allocate_space(slot);
            }
        }
    }
}

pub struct App {
    pub(crate) source: Box<dyn WatchSource>,
    pub(crate) flags: Flags,
    /// Watching without playing (D20); the watched country can be switched at any time.
    pub(crate) observer: bool,
    pub(crate) show_labels: bool,
    pub(crate) labels: crate::labels::Labels,
    pub(crate) theme: Theme,
    pub(crate) art: Art,
    /// Start screen state while the App shows the start screen instead of the game.
    pub(crate) start: Option<crate::start::StartState>,
    keys: Bindings,
    pub(crate) cam: Option<Camera>,
    pub(crate) mode: ViewMode,
    pub(crate) palette: wiktoria_render::Palette,
    pub(crate) palette_key: (ViewMode, u64),
    pub(crate) borders: bool,
    pub(crate) selected: Option<u16>,
    pub(crate) hover: Option<u16>,
    /// The country the player watches (top left block, outliner, log).
    pub(crate) player: u16,
    /// Open windows, last = topmost.
    open: Vec<Win>,
    tabs: HashMap<Win, usize>,
    dip_filter: String,
    dip_sel: Option<u16>,
    outliner_open: bool,
    paused: bool,
    speed: u8,
    pub(crate) ui_zoom: f32,
    pub(crate) tex: Option<TextureHandle>,
    pub(crate) last_render: Option<(Camera, (usize, usize), bool, Option<u16>, (ViewMode, u64))>,
    pub(crate) shot_path: Option<String>,
    pub(crate) frame: u32,
    pub(crate) shot_requested: bool,
}

pub(crate) fn opt_num(v: Option<f64>) -> String {
    v.map_or("-".into(), |x| fmt_int(x))
}

pub(crate) fn dim(t: &Theme, s: impl Into<String>) -> RichText {
    RichText::new(s.into()).color(t.ink_dim)
}

fn placeholder(ui: &mut egui::Ui, t: &Theme, what: &str) {
    ui.label(dim(t, format!("{what}: not simulated yet")).italics());
}

fn section(ui: &mut egui::Ui, t: &Theme, title: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(title).color(t.gold).strong());
    ui.separator();
}

/// A pie drawn with plain triangles. Empty or zero slices draw an outlined ring.
pub(crate) fn pie(ui: &mut egui::Ui, t: &Theme, radius: f32, slices: &[f64]) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(radius * 2.0), egui::Sense::hover());
    let c = rect.center();
    let total: f64 = slices.iter().sum();
    let painter = ui.painter();
    if total <= 0.0 {
        painter.circle_stroke(c, radius, egui::Stroke::new(1.0, t.edge));
        return;
    }
    let mut mesh = egui::Mesh::default();
    let mut a0 = -std::f32::consts::FRAC_PI_2;
    for (i, s) in slices.iter().enumerate() {
        let sweep = (*s / total) as f32 * std::f32::consts::TAU;
        if sweep <= 0.0 {
            continue;
        }
        let col = PIE_COLORS[i % PIE_COLORS.len()];
        let steps = ((sweep / std::f32::consts::TAU) * 48.0).ceil().max(1.0) as usize;
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(c, col);
        for k in 0..=steps {
            let a = a0 + sweep * k as f32 / steps as f32;
            mesh.colored_vertex(c + Vec2::angled(a) * radius, col);
        }
        for k in 0..steps as u32 {
            mesh.add_triangle(base, base + 1 + k, base + 2 + k);
        }
        a0 += sweep;
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.circle_stroke(c, radius, egui::Stroke::new(1.0, t.edge));
}

fn tab_strip(ui: &mut egui::Ui, cur: &mut usize, labels: &[&str]) {
    ui.horizontal_wrapped(|ui| {
        for (i, l) in labels.iter().enumerate() {
            if slots::button(ui, "window_tab", None, *cur == i, true, *l).clicked() {
                *cur = i;
            }
        }
    });
    ui.separator();
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, source: Box<dyn WatchSource>, install: PathBuf, base: PathBuf) -> Self {
        Self::with_ctx(&cc.egui_ctx, source, install, base, None)
    }

    pub fn with_ctx(ctx: &Context, source: Box<dyn WatchSource>, install: PathBuf, base: PathBuf, theme_name: Option<&str>) -> Self {
        let (theme, art) = Art::load(&base, theme_name);
        slots::install_for(ctx, &base, crate::theme::theme_name(theme_name).as_deref(), Some(&install));
        apply_style(ctx, &theme);
        install_fonts(ctx, &art);
        let ui_zoom = std::env::var("GM_UI_ZOOM").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
        let ui_zoom = if std::env::var("GM_UI_ZOOM").is_ok() { ui_zoom } else { ctx.zoom_factor() };
        ctx.set_zoom_factor(ui_zoom);
        let mode = ViewMode::Political;
        let palette = source.palette(mode);
        let palette_key = (mode, source.revision());
        let selected = std::env::var("GM_UI_SELECT").ok().and_then(|s| s.parse().ok());
        let player = source.data().ledger.first().map(|r| r.country).unwrap_or(0);
        let open: Vec<Win> = std::env::var("GM_UI_OPEN").ok().map(|s| s.split(',').filter_map(Win::parse).collect()).unwrap_or_default();
        let mut app = App {
            source,
            flags: Flags { dir: install.join("gfx").join("flags"), cache: HashMap::new() },
            observer: false,
            show_labels: true,
            labels: Default::default(),
            theme,
            art,
            start: None,
            keys: Bindings::default(),
            cam: None,
            mode,
            palette,
            palette_key,
            borders: true,
            selected,
            hover: None,
            player,
            open,
            tabs: HashMap::new(),
            dip_filter: "All".into(),
            dip_sel: None,
            outliner_open: true,
            paused: true,
            speed: 1,
            ui_zoom,
            tex: None,
            last_render: None,
            shot_path: std::env::var("GM_UI_SHOT").ok(),
            frame: 0,
            shot_requested: false,
        };
        if let Some(id) = app.selected {
            app.select(id);
        }
        app
    }

    pub(crate) fn select(&mut self, id: u16) {
        self.selected = Some(id);
    }

    /// "Watch country": switch the watched country without restarting.
    fn watch_picker(&mut self, ui: &mut egui::Ui) {
        let rows = self.source.ranking_rows();
        let mut pick = self.player;
        egui::ComboBox::from_id_salt("watch_country").selected_text("Watch...").height(400.0).show_ui(ui, |ui| {
            for r in &rows {
                let name = self.source.country(r.country).name;
                ui.selectable_value(&mut pick, r.country, format!("{}  {}", r.rank, name));
            }
        });
        self.player = pick;
    }

    /// UI maker preview: open a country window so its frame and tabs are visible.
    pub(crate) fn preview_window(&mut self) {
        if self.open.is_empty() {
            self.open.push(Win::Budget);
        }
    }

    fn toggle(&mut self, w: Win) {
        if let Some(i) = self.open.iter().position(|x| *x == w) {
            self.open.remove(i);
        } else {
            self.open.push(w);
        }
    }

    fn handle_keys(&mut self, ctx: &Context) {
        for a in self.keys.pressed(ctx) {
            match a {
                Action::Window(w) => self.toggle(w),
                Action::MapMode(i) => {
                    if let Some(m) = ViewMode::ALL.get(i).filter(|m| self.source.supports(**m)) {
                        self.mode = *m;
                    }
                }
                Action::Pause => self.paused = !self.paused,
                Action::SpeedUp => self.speed = (self.speed + 1).min(5),
                Action::SpeedDown => self.speed = self.speed.saturating_sub(1).max(1),
                Action::CloseTop => {
                    self.open.pop();
                }
                Action::LedgerPrev => self.bump_tab(Win::Ledger, -1, 4),
                Action::LedgerNext => self.bump_tab(Win::Ledger, 1, 4),
            }
        }
    }

    fn bump_tab(&mut self, w: Win, d: i32, n: usize) {
        if self.open.contains(&w) {
            let t = self.tabs.entry(w).or_insert(0);
            *t = (*t as i32 + d).rem_euclid(n as i32) as usize;
        }
    }

    /// One short live line per top-bar button (placeholders until a simulation provides the data).
    fn summary_line(w: Win, s: &CountrySummary) -> String {
        match w {
            Win::Budget => match (s.treasury, s.daily_balance) {
                (Some(t), Some(b)) => format!("{} ({:+.0}/day)", fmt_int(t), b),
                (Some(t), None) => fmt_int(t),
                _ => "treasury n/a".into(),
            },
            Win::Technology => s.research.clone().unwrap_or_else(|| "no research data".into()),
            Win::Politics => s.ruling_party.clone().unwrap_or_else(|| "-".into()),
            Win::Population => match (s.population, s.pop_growth) {
                (Some(p), Some(g)) => format!("{} ({:+.1}%)", fmt_int(p), g * 100.0),
                (Some(p), None) => fmt_int(p),
                _ => "-".into(),
            },
            Win::Trade => s.trade.clone().unwrap_or_else(|| "-".into()),
            Win::Diplomacy => match s.at_war {
                Some(true) => "At war".into(),
                _ => "At peace".into(),
            },
            Win::Military => match (s.army_size, s.navy_size) {
                (Some(a), Some(n)) => format!("{} army, {} navy", fmt_int(a), fmt_int(n)),
                _ => "army and navy n/a".into(),
            },
            Win::Production => "factories n/a".into(),
            Win::Ledger => String::new(),
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let th = self.theme;
        let em = ui.text_style_height(&TextStyle::Body);
        let date = self.source.date();
        let f = self.source.country(self.player);
        let s = self.source.country_summary(self.player);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let fp = ui.cursor().min;
                    self.flags.show(ui, &f.tag, em * 2.2);
                    let fr = egui::Rect::from_min_size(fp, Vec2::new(em * 3.3, em * 2.2)).expand(3.0);
                    slots::paint(ui, "flag_frame", fr, slots::State::Normal);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&f.name).strong());
                            if self.observer {
                                ui.label(dim(&th, "(observing)").small());
                            }
                            self.watch_picker(ui);
                        });
                        ui.label(
                            dim(&th, format!("Rank {}   Prestige {}", s.rank.map_or("-".into(), |r| format!("#{r}")), opt_num(s.prestige)))
                                .small(),
                        );
                        ui.label(dim(&th, format!("Industry {}   Military {}", opt_num(s.industry_score), opt_num(s.military_score))).small());
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)).color(th.gold).strong());
                    if let Some(st) = self.source.sim_status() {
                        ui.label(dim(&th, st).small());
                    }
                    if slots::button(ui, "speed_button", None, self.paused, true, "Pause").clicked() {
                        self.paused = !self.paused;
                    }
                    for sp in 1..=5u8 {
                        if slots::button(ui, "speed_button", None, !self.paused && self.speed == sp, true, sp.to_string()).clicked() {
                            self.paused = false;
                            self.speed = sp;
                        }
                    }
                });
            });
            ui.separator();
            for w in Win::TOP_BAR {
                ui.vertical(|ui| {
                    ui.set_width(em * 9.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let open = self.open.contains(&w);
                    let hint = self.keys.hint(Action::Window(w)).unwrap_or_default();
                    let b = slots::button(
                        ui,
                        "topbar_tab",
                        Some(Vec2::new(em * 9.0, em * 1.6)),
                        open,
                        true,
                        RichText::new(format!("{}  {}", w.label(), hint)).strong(),
                    );
                    if b.clicked() {
                        self.toggle(w);
                    }
                    ui.add(egui::Label::new(dim(&th, Self::summary_line(w, &s)).small()).truncate());
                });
            }
        });
    }

    fn bottom_bar(&mut self, ui: &mut egui::Ui) {
        let th = self.theme;
        let msgs = self.source.messages(5);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_min_width(ui.available_width() * 0.55);
                ui.label(RichText::new("Messages").color(th.gold).small());
                for m in msgs.iter().rev() {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{:04}-{:02}-{:02}  {}", m.date.year, m.date.month, m.date.day, m.text)).small(),
                        )
                        .truncate(),
                    );
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        for (i, m) in ViewMode::ALL.iter().enumerate() {
                            let key = self.keys.hint(Action::MapMode(i)).unwrap_or_default();
                            let r = slots::button(ui, "mapmode_button", None, self.mode == *m, self.source.supports(*m), format!("{} {}", m.label(), key));
                            let r = if self.source.supports(*m) { r } else { r.on_disabled_hover_text("Needs the combined day loop") };
                            if r.clicked() {
                                self.mode = *m;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.borders, "Borders");
                        ui.checkbox(&mut self.show_labels, "Names").on_hover_text("Country names on the political map");
                        ui.add(egui::Slider::new(&mut self.ui_zoom, 0.75..=2.5).text("UI zoom").fixed_decimals(2));
                    });
                });
            });
        });
        if (self.ui_zoom - ui.ctx().zoom_factor()).abs() > 0.001 && !ui.ctx().input(|i| i.pointer.any_down()) {
            ui.ctx().set_zoom_factor(self.ui_zoom);
        }
    }

    fn outliner(&mut self, ui: &mut egui::Ui) {
        let th = self.theme;
        let entries = self.source.outliner();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Outliner").color(th.gold).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(">").on_hover_text("Collapse").clicked() {
                    self.outliner_open = false;
                }
            });
        });
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(ui.available_width());
            for (title, kind) in [("Armies", OutlinerKind::Army), ("Navies", OutlinerKind::Navy), ("Influence", OutlinerKind::Influence)] {
                let list: Vec<_> = entries.iter().filter(|e| e.kind == kind).collect();
                egui::CollapsingHeader::new(format!("{title} ({})", list.len())).default_open(true).show(ui, |ui| {
                    if list.is_empty() {
                        placeholder(ui, &th, "none yet");
                    }
                    for e in list {
                        ui.add(egui::Label::new(&e.name).truncate());
                        ui.add(egui::Label::new(dim(&th, &e.detail).small()).wrap());
                        ui.add_space(2.0);
                    }
                });
            }
        });
    }

    fn country_line(&mut self, ui: &mut egui::Ui, label: &str, c: &Option<CountryFacts>, em: f32) {
        ui.label(dim(&self.theme, label));
        match c {
            Some(c) => {
                ui.horizontal(|ui| {
                    self.flags.show(ui, &c.tag, em * 1.1);
                    ui.label(&c.name);
                });
            }
            None => {
                ui.label("-");
            }
        }
        ui.end_row();
    }

    fn owner_block(&mut self, ui: &mut egui::Ui, o: &CountryFacts, em: f32) {
        let th = self.theme;
        let s = self.source.country_summary(o.idx);
        section(ui, &th, "Owner");
        ui.horizontal(|ui| {
            self.flags.show(ui, &o.tag, em * 2.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(&o.name).strong());
                ui.label(dim(&th, format!("Rank {}   Score {}", s.rank.map_or("-".into(), |r| format!("#{r}")), opt_num(s.prestige))).small());
            });
        });
        let names = |v: &[u16], src: &dyn WatchSource| -> String {
            if v.is_empty() { "-".into() } else { v.iter().map(|c| src.country(*c).name).collect::<Vec<_>>().join(", ") }
        };
        egui::Grid::new("owner_rows").num_columns(2).show(ui, |ui| {
            for (k, v) in [
                ("Government", s.government.clone().unwrap_or_else(|| "-".into())),
                ("Ruling party", s.ruling_party.clone().unwrap_or_else(|| "-".into())),
                ("Sphere", s.sphere_leader.map_or("-".into(), |c| self.source.country(c).name)),
                ("Overlord", s.overlord.map_or("-".into(), |c| self.source.country(c).name)),
                ("Allies", names(&s.allies, &*self.source)),
                ("Wars", match s.at_war { Some(true) => "at war".into(), _ => "none".into() }),
            ] {
                ui.label(dim(&th, k));
                ui.label(v);
                ui.end_row();
            }
        });
        if ui.button("Go to diplomacy").clicked() {
            self.dip_sel = Some(o.idx);
            if !self.open.contains(&Win::Diplomacy) {
                self.open.push(Win::Diplomacy);
            }
        }
    }

    fn province_panel(&mut self, ui: &mut egui::Ui, f: &ProvinceFacts) {
        let th = self.theme;
        let em = ui.text_style_height(&TextStyle::Body);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(&f.name);
                ui.label(dim(&th, format!("{}  #{}", f.state.as_deref().unwrap_or("-"), f.id)).small());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if let Some(o) = &f.owner {
                    self.flags.show(ui, &o.tag, em * 2.0);
                }
                // Our own terrain picture if the art folder has one (`terrain_<name>.png`), else nothing.
                if let Some(t) = f.terrain.as_ref().and_then(|t| self.art.get(ui.ctx(), &format!("terrain_{t}"))) {
                    image_in_slot(ui, &t, Vec2::new(em * 4.0, em * 2.6));
                }
            });
        });
        if f.is_sea {
            ui.label("Sea zone");
            return;
        }
        ui.add(egui::ProgressBar::new((f.life_rating.unwrap_or(0.0) / 100.0) as f32).text(format!("Life rating {}", opt_num(f.life_rating))).fill(th.good.gamma_multiply(0.7)));
        egui::Grid::new("prov_facts").num_columns(2).show(ui, |ui| {
            for (k, v) in [
                ("RGO", format!("{}  ({})", f.rgo_good.as_deref().unwrap_or("-"), opt_num(f.rgo_output))),
                ("Income", opt_num(f.income)),
                ("Population", fmt_int(f.total_pop)),
                ("Literacy", format!("{:.0}%", f.literacy * 100.0)),
                ("Militancy", format!("{:.1}", f.militancy)),
                ("Supply limit", opt_num(f.supply_limit)),
            ] {
                ui.label(dim(&th, k));
                ui.label(v);
                ui.end_row();
            }
            self.country_line(ui, "Controller", &f.controller, em);
        });
        ui.horizontal(|ui| {
            ui.label(dim(&th, "Cores"));
            ui.spacing_mut().item_spacing.x = 3.0;
            for tag in f.cores.iter().take(10) {
                self.flags.show(ui, tag, em * 1.0);
            }
            if f.cores.len() > 10 {
                ui.label(dim(&th, format!("+{}", f.cores.len() - 10)).small());
            }
        });
        section(ui, &th, "Workforce, ideology, culture");
        ui.horizontal(|ui| {
            let sizes: Vec<f64> = f.pop_types.iter().map(|x| x.1).collect();
            let ideo: Vec<f64> = f.ideologies.iter().map(|x| x.1).collect();
            let cult: Vec<f64> = f.cultures.iter().map(|x| x.1).collect();
            for (title, data, names) in [
                ("Workforce", &sizes, f.pop_types.iter().map(|x| x.0.clone()).collect::<Vec<_>>()),
                ("Ideology", &ideo, f.ideologies.iter().map(|x| x.0.clone()).collect()),
                ("Culture", &cult, f.cultures.iter().map(|x| x.0.clone()).collect()),
            ] {
                ui.vertical(|ui| {
                    pie(ui, &th, em * 2.0, data);
                    ui.label(dim(&th, title).small());
                    let top = names.first().cloned().unwrap_or_else(|| "n/a".into());
                    ui.add(egui::Label::new(RichText::new(top).small()).truncate());
                });
            }
        });
        section(ui, &th, "Buildings");
        egui::Grid::new("prov_buildings").num_columns(3).show(ui, |ui| {
            let mut rows: Vec<(String, u32)> =
                vec![("Fort".into(), f.fort), ("Naval base".into(), f.naval_base), ("Railroad".into(), f.railroad)];
            rows.extend(f.buildings.iter().cloned());
            for (k, v) in rows {
                ui.label(dim(&th, k));
                ui.label(format!("lv {v}"));
                slots::button(ui, "button", None, false, false, RichText::new("Expand").small());
                ui.end_row();
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if f.colonial {
                slots::button(ui, "button", None, false, false, "Colonise");
                slots::button(ui, "button", None, false, false, "Protectorate");
            } else {
                slots::button(ui, "button", None, false, false, "Build army");
                slots::button(ui, "button", None, false, false, "Build navy");
            }
        });
        if let Some(o) = f.owner.clone().filter(|o| o.idx != self.player) {
            self.owner_block(ui, &o, em);
        }
    }

    // ---------------------------------------------------------------- country windows

    fn window_body(&mut self, ui: &mut egui::Ui, w: Win) {
        let th = self.theme;
        let mut tab = *self.tabs.get(&w).unwrap_or(&0);
        let s = self.source.country_summary(self.player);
        match w {
            Win::Production => {
                tab_strip(ui, &mut tab, &["Factories", "Foreign investment", "Production", "Projects"]);
                match tab {
                    0 => {
                        section(ui, &th, "Factories per state");
                        placeholder(ui, &th, "slots, workers and profit");
                        ui.add_enabled(false, egui::Button::new("Build factory"));
                    }
                    1 => {
                        section(ui, &th, "Foreign investment");
                        placeholder(ui, &th, "investments");
                    }
                    2 => {
                        for g in ["Military goods", "Raw goods", "Industrial goods", "Consumer goods"] {
                            section(ui, &th, g);
                            placeholder(ui, &th, "output per good");
                        }
                    }
                    _ => {
                        section(ui, &th, "Projects");
                        placeholder(ui, &th, "progress and goods still needed");
                    }
                }
            }
            Win::Budget => {
                ui.columns(2, |c| {
                    section(&mut c[0], &th, "Daily income");
                    for k in ["Poor tax", "Middle tax", "Rich tax", "Gold"] {
                        c[0].horizontal(|ui| {
                            ui.label(dim(&th, k));
                            ui.add_enabled(false, egui::Slider::new(&mut 0.0f32, 0.0..=1.0).show_value(false));
                        });
                    }
                    section(&mut c[0], &th, "Funds");
                    for (k, v) in [("National bank", None), ("Total funds", s.treasury), ("Debt", None)] {
                        c[0].label(format!("{k}: {}", opt_num(v)));
                    }
                    c[0].label(dim(&th, "Loans: none"));
                    section(&mut c[1], &th, "Daily expenses");
                    for k in ["National stockpile", "Education", "Administration", "Social spending", "Military spending"] {
                        c[1].horizontal(|ui| {
                            ui.label(dim(&th, k));
                            ui.add_enabled(false, egui::Slider::new(&mut 0.0f32, 0.0..=1.0).show_value(false));
                        });
                    }
                    section(&mut c[1], &th, "Balance");
                    for (k, v) in [("Tariffs", None), ("Diplomatic balance", None), ("Projected daily balance", s.daily_balance)] {
                        c[1].label(format!("{k}: {}", opt_num(v)));
                    }
                });
            }
            Win::Technology => {
                tab_strip(ui, &mut tab, &["Army", "Navy", "Commerce", "Culture", "Industry"]);
                section(ui, &th, "Research in progress");
                ui.label(s.research.clone().unwrap_or_else(|| "none".into()));
                placeholder(ui, &th, "technology list and progress");
            }
            Win::Politics => {
                ui.columns(2, |c| {
                    section(&mut c[0], &th, "Government");
                    c[0].label(format!("Government: {}", s.government.clone().unwrap_or_else(|| "-".into())));
                    c[0].label(format!("Ruling party: {}", s.ruling_party.clone().unwrap_or_else(|| "-".into())));
                    for t in ["Upper house", "Election date", "Party policies", "Voters' ideology", "Peoples' ideology", "Important issues"] {
                        section(&mut c[0], &th, t);
                        placeholder(&mut c[0], &th, "data");
                    }
                    tab_strip(&mut c[1], &mut tab, &["Reforms", "Movements and rebels", "Decisions", "Release nations"]);
                    placeholder(&mut c[1], &th, ["Social and political reforms", "Movements", "Decisions", "Releasable nations"][tab.min(3)]);
                });
            }
            Win::Population => {
                tab_strip(ui, &mut tab, &["Overview", "Pop types", "Cultures", "Religions"]);
                ui.label(format!("Total population: {}", opt_num(s.population)));
                placeholder(ui, &th, "growth, breakdown");
            }
            Win::Trade => {
                ui.columns(2, |c| {
                    section(&mut c[0], &th, "Goods");
                    placeholder(&mut c[0], &th, "price and trend per good");
                    section(&mut c[1], &th, "Selected good");
                    placeholder(&mut c[1], &th, "price history, supply and demand");
                    for t in ["Government needs", "Factories", "Population", "Market activity", "Stockpile", "Common market"] {
                        c[1].label(dim(&th, t));
                    }
                });
            }
            Win::Diplomacy => self.diplomacy_body(ui, &mut tab),
            Win::Military => {
                ui.columns(3, |c| {
                    section(&mut c[0], &th, "Leaders");
                    placeholder(&mut c[0], &th, "generals and admirals");
                    section(&mut c[0], &th, "Stats");
                    for k in ["War exhaustion", "Supply consumption", "Organisation regain", "Recruit time", "Combat width", "Dig-in cap"] {
                        c[0].label(dim(&th, format!("{k}: -")));
                    }
                    section(&mut c[1], &th, "Armies");
                    placeholder(&mut c[1], &th, "name, location, strength");
                    c[1].add_enabled(false, egui::Button::new("Mobilise"));
                    section(&mut c[2], &th, "Navies");
                    placeholder(&mut c[2], &th, "name, location, strength");
                    section(&mut c[2], &th, "Being built");
                    placeholder(&mut c[2], &th, "units");
                });
            }
            Win::Ledger => self.ledger_body(ui, &mut tab),
        }
        self.tabs.insert(w, tab);
    }

    fn diplomacy_body(&mut self, ui: &mut egui::Ui, tab: &mut usize) {
        let th = self.theme;
        let em = ui.text_style_height(&TextStyle::Body);
        tab_strip(ui, tab, &["Great powers", "Wars", "Casus belli", "Crisis"]);
        match *tab {
            0 => {
                let d = self.source.data().clone();
                ui.label(dim(&th, &d.ranking_note).small());
                ui.horizontal_wrapped(|ui| {
                    for r in d.ranking.iter() {
                        let c = self.source.country(r.country);
                        egui::Frame::new().fill(th.bg_widget).stroke(egui::Stroke::new(1.0, th.edge)).inner_margin(Margin::same(6)).show(ui, |ui| {
                            ui.set_width(em * 7.0);
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    self.flags.show(ui, &c.tag, em * 1.8);
                                    ui.label(dim(&th, format!("#{}", r.rank)));
                                });
                                ui.add(egui::Label::new(RichText::new(&c.name).strong()).truncate());
                                ui.label(RichText::new(format!("{:.0}", r.score)).color(th.gold));
                            });
                        });
                    }
                });
            }
            1 => {
                let d = self.source.data().clone();
                ui.label(dim(&th, &d.wars_note).small());
                if d.wars.is_empty() {
                    ui.label("No wars.");
                }
                for w in d.wars {
                    egui::CollapsingHeader::new(&w.name).default_open(true).show(ui, |ui| {
                        ui.add(egui::ProgressBar::new(w.war_score.unwrap_or(0.5)).text("War score"));
                        ui.columns(2, |c| {
                            for (i, (side, list)) in [("Attackers", &w.attackers), ("Defenders", &w.defenders)].into_iter().enumerate() {
                                c[i].label(RichText::new(side).color(th.gold));
                                for k in list {
                                    let f = self.source.country(*k);
                                    c[i].horizontal(|ui| {
                                        self.flags.show(ui, &f.tag, em);
                                        ui.label(f.name);
                                    });
                                }
                            }
                        });
                    });
                }
            }
            2 => placeholder(ui, &th, "war justification"),
            _ => placeholder(ui, &th, "crisis"),
        }
        ui.add_space(6.0);
        // Country table with region filter tabs.
        let rows = self.source.diplomacy_rows();
        let mut regions: Vec<String> = rows.iter().map(|r| r.region.clone()).collect();
        regions.sort();
        regions.dedup();
        ui.horizontal_wrapped(|ui| {
            let mut names: Vec<String> = vec!["All".into()];
            names.extend(regions);
            names.extend(["Neighbours", "Sphere", "Enemies", "Allies"].map(String::from));
            for n in names {
                if ui.selectable_label(self.dip_filter == n, &n).clicked() {
                    self.dip_filter = n;
                }
            }
        });
        if let Some(sel) = self.dip_sel {
            let c = self.source.country(sel);
            ui.horizontal(|ui| {
                self.flags.show(ui, &c.tag, em * 1.4);
                ui.label(RichText::new(c.name).strong());
                for b in ["Form alliance", "Military access", "War subsidies", "Declare war", "Increase opinion", "Add to sphere"] {
                    ui.add_enabled(false, egui::Button::new(b).small());
                }
            });
        }
        let filter = self.dip_filter.clone();
        let rows: Vec<_> = rows
            .into_iter()
            .filter(|r| match filter.as_str() {
                "All" => true,
                "Neighbours" => r.neighbour,
                "Sphere" => r.in_sphere,
                "Enemies" => r.enemy,
                "Allies" => r.allied,
                f => r.region == f,
            })
            .collect();
        let gp: Vec<_> = self.source.data().ranking.iter().take(8).map(|r| self.source.country(r.country).tag).collect();
        egui::ScrollArea::vertical().max_height(em * 14.0).auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("dip_table").striped(true).show(ui, |ui| {
                ui.label(dim(&th, "Country"));
                ui.label(dim(&th, "Relation"));
                for t in &gp {
                    ui.label(dim(&th, t));
                }
                ui.label(dim(&th, "Opinion"));
                ui.end_row();
                for r in rows.iter().take(400) {
                    let c = self.source.country(r.country);
                    let resp = ui
                        .horizontal(|ui| {
                            self.flags.show(ui, &c.tag, em);
                            ui.selectable_label(self.dip_sel == Some(c.idx), &c.name)
                        })
                        .inner;
                    if resp.clicked() {
                        self.dip_sel = Some(c.idx);
                    }
                    ui.label(r.relation.map_or("-".into(), |v| v.to_string()));
                    for i in 0..gp.len() {
                        ui.label(opt_num(r.influence.get(i).copied().flatten()));
                    }
                    ui.label(r.opinion.clone().unwrap_or_else(|| "-".into()));
                    ui.end_row();
                }
            });
        });
    }

    fn ledger_body(&mut self, ui: &mut egui::Ui, tab: &mut usize) {
        let th = self.theme;
        let em = ui.text_style_height(&TextStyle::Body);
        tab_strip(ui, tab, &["Nation ranking", "Industry", "Population", "Trade"]);
        if *tab != 0 {
            placeholder(ui, &th, "this page");
            ui.label(dim(&th, "Left and right arrows page through the ledger.").small());
            return;
        }
        let rows = self.source.ranking_rows();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("ledger_grid").striped(true).num_columns(6).show(ui, |ui| {
                for h in ["#", "Country", "Military", "Industrial", "Prestige", "Total"] {
                    ui.label(dim(&th, h));
                }
                ui.end_row();
                for r in rows.iter().take(300) {
                    let c = self.source.country(r.country);
                    ui.label(r.rank.to_string());
                    ui.horizontal(|ui| {
                        self.flags.show(ui, &c.tag, em);
                        ui.label(c.name);
                    });
                    ui.label(opt_num(r.military));
                    ui.label(opt_num(r.industrial));
                    ui.label(RichText::new(opt_num(r.prestige)).color(th.gold));
                    ui.label(opt_num(r.total));
                    ui.end_row();
                }
            });
        });
    }
}

impl App {
    pub fn frame_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let th = self.theme;
        self.handle_keys(&ctx);
        // Live source: newest snapshot in, controls out. Never blocks on the sim.
        self.source.poll();
        if self.shot_path.is_some() && self.source.live() {
            // Capture mode: run flat out until GM_UI_RUN_DAYS days are done, then hold.
            self.paused = true;
        }
        self.source.set_control(self.paused, self.speed, self.player);
        if self.source.live() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let bar = |m: i8| egui::Frame::new().fill(th.bg_deep).inner_margin(Margin::symmetric(10, m)).stroke(egui::Stroke::new(1.0, th.edge));
        egui::Panel::top("top_bar").frame(bar(6)).show(ui, |ui| dressed(ui, "topbar_bg", true, 6.0, |ui| self.top_bar(ui)));
        egui::Panel::bottom("bottom_bar").frame(bar(4)).show(ui, |ui| dressed(ui, "msglog_bg", true, 4.0, |ui| self.bottom_bar(ui)));
        let em = ui.text_style_height(&TextStyle::Body);
        if self.outliner_open {
            egui::Panel::right("outliner")
                .resizable(false).exact_size(em * 16.0)

                .frame(egui::Frame::new().fill(th.bg_panel).inner_margin(Margin::same(8)))
                .show(ui, |ui| dressed(ui, "outliner_bg", true, 8.0, |ui| self.outliner(ui)));
        } else {
            egui::Panel::right("outliner_collapsed")
                .resizable(false)
                .exact_size(em * 2.2)
                .frame(egui::Frame::new().fill(th.bg_panel).inner_margin(Margin::same(4)))
                .show(ui, |ui| {
                    if ui.button("<").on_hover_text("Outliner").clicked() {
                        self.outliner_open = true;
                    }
                });
        }
        let avail = ui.available_rect_before_wrap();
        egui::CentralPanel::no_frame().show(ui, |ui| self.map_view(ui));

        // Province window, bottom left over the map.
        if let Some(id) = self.selected {
            if let Some(f) = self.source.province(id) {
                let corner = avail.left_bottom() + Vec2::new(8.0, -8.0);
                let max_h = avail.height() - 16.0;
                egui::Window::new("province_window")
                    .title_bar(false)
                    .resizable(false)
                    .fixed_pos(corner)
                    .pivot(Align2::LEFT_BOTTOM)
                    .frame(egui::Frame::window(&ctx.global_style()).inner_margin(Margin::same(8)))
                    .show(&ctx, |ui| {
                        dressed(ui, "province_bg", false, 8.0, |ui| {
                            ui.set_width(em * 21.0);
                            egui::ScrollArea::vertical().max_height(max_h - 24.0).auto_shrink([true, true]).show(ui, |ui| self.province_panel(ui, &f));
                        });
                    });
            }
        }

        // Country windows, centred.
        let centre = avail.center();
        for w in self.open.clone() {
            let mut open = true;
            let size = match w {
                Win::Ledger => Vec2::new(em * 40.0, em * 32.0),
                _ => Vec2::new(em * 56.0, em * 36.0),
            };
            egui::Window::new(w.label())
                .id(egui::Id::new(("win", w)))
                .open(&mut open)
                .collapsible(false)
                .pivot(Align2::CENTER_CENTER)
                .default_pos(centre)
                .default_size(size)
                .show(&ctx, |ui| {
                    dressed(ui, "window_bg", false, 6.0, |ui| {
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.window_body(ui, w));
                    });
                });
            if !open {
                self.open.retain(|x| *x != w);
            }
        }
        self.handle_screenshot(&ctx);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame_ui(ui);
    }
}
