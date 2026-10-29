//! The window's top level: main menu, settings, loading screen, start screen and the game.
//!
//! Slow work (loading the World, building the Game) runs on threads; the UI keeps drawing a loading
//! screen with progress text meanwhile. The World is preloaded as soon as the menu opens, so New game
//! is quick, and its land shape gives the menu its faint map silhouette (our own drawing of the
//! install's map, nothing copied).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Instant;

use eframe::egui::{self, Align2, Color32, ColorImage, RichText, TextureHandle, TextureOptions, Vec2};
use wiktoria_data::world::World;

use crate::settings::Settings;
use crate::start::{peek_save, Entry, Launch, StartState};
use crate::theme::{Art, Theme, SERIF};
use crate::ui::App;
use crate::watch::{StaticWorldSource, WatchSource};

pub const VERSION: &str = "prototype 0.1";

type WorldRx = Receiver<Result<World, String>>;

enum Next {
    /// Start screen; `true` = saves only (Load game).
    Start(bool),
    Game(Launch),
}

enum Screen {
    Menu,
    Settings,
    Loading { next: Next, rx: Option<WorldRx>, since: Instant },
    /// The game's source is building; shown until its first snapshot arrives.
    Building { app: Box<App>, since: Instant },
    Start(Box<App>),
    Game(Box<App>),
    Maker(Box<crate::maker::Maker>),
    Error(String),
}

pub struct Shell {
    root: PathBuf,
    install: Option<PathBuf>,
    base: PathBuf,
    settings: Settings,
    theme: Theme,
    art: Art,
    screen: Screen,
    /// Preloaded World (taken by the next screen that needs one).
    preload: Option<WorldRx>,
    world: Option<World>,
    silhouette: Option<TextureHandle>,
    // Settings form.
    form_dir: String,
    form_zoom: f32,
    form_theme: String,
    form_msg: String,
    shot: Option<String>,
    frames: u32,
    shot_requested: bool,
}

fn spawn_world(root: &Path, install: &Path) -> WorldRx {
    let (tx, rx) = channel();
    let (root, install) = (root.to_path_buf(), install.to_path_buf());
    std::thread::Builder::new()
        .name("world".into())
        .spawn(move || {
            let r = wiktoria_data::config::load_engine_rules(&root).and_then(|m| World::load(&install, &m));
            let _ = tx.send(r);
        })
        .expect("spawn world thread");
    rx
}

/// Land/sea mask of the map, downscaled, as a white-on-transparent texture.
fn silhouette(ctx: &egui::Context, w: &World) -> TextureHandle {
    let map = &w.map;
    let f = (map.width / 1024).max(1);
    let (sw, sh) = (map.width / f, map.height / f);
    let land: Vec<bool> = (0..w.provinces.len()).map(|i| w.provinces[i].as_ref().is_some_and(|p| !p.is_sea)).collect();
    let mut px = vec![Color32::TRANSPARENT; sw * sh];
    for y in 0..sh {
        for x in 0..sw {
            let id = map.ids[(y * f) * map.width + x * f] as usize;
            if land.get(id).copied().unwrap_or(false) {
                px[y * sw + x] = Color32::WHITE;
            }
        }
    }
    ctx.load_texture("menu_silhouette", ColorImage::new([sw, sh], px), TextureOptions::LINEAR)
}

/// Bookmarks (from the install), then saves in the start folder and the install's "save games".
fn entries(root: &Path, install: &Path, w: &World) -> Vec<Entry> {
    let start_save = root.join("start/1836.v2");
    let mut out = Vec::new();
    for b in &w.defs.bookmarks {
        let first = b.date == w.start_date;
        out.push(Entry {
            title: format!("Start of {}", b.date.year),
            date: format!("{}", b.date),
            save: (first && start_save.is_file()).then(|| start_save.clone()),
            tag: None,
            story: if first {
                "The great wars are over and the old order has been restored, but the ideas they spread have not gone away. \
                 Factories, railways and newspapers are beginning to change how people live and what they believe. \
                 Guide a nation through the century ahead: its economy, its politics and its place among the powers."
                    .into()
            } else {
                "This start date has no prepared save yet, so it cannot be played in this prototype.".into()
            },
            bookmark: true,
        });
    }
    let mut saves: Vec<PathBuf> = Vec::new();
    for dir in [root.join("start"), install.join("save games")] {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("v2"))).collect();
            v.sort();
            saves.extend(v);
        }
    }
    for p in saves {
        let (date, tag) = peek_save(&p).unwrap_or_default();
        let name = p.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
        out.push(Entry {
            title: name.replace('_', " "),
            date,
            story: format!("Saved game: {}", p.display()),
            save: Some(p),
            tag,
            bookmark: false,
        });
    }
    out
}

impl Shell {
    pub fn new(cc: &eframe::CreationContext<'_>, root: PathBuf, install: Option<PathBuf>, base: PathBuf, exe_dir: PathBuf, settings: Settings, direct: Option<PathBuf>) -> Self {
        let _ = exe_dir;
        let (theme, art) = Art::load(&base, settings.theme.as_deref());
        crate::slots::install_for(&cc.egui_ctx, &base, crate::theme::theme_name(settings.theme.as_deref()).as_deref(), install.as_deref());
        crate::ui::apply_style(&cc.egui_ctx, &theme);
        crate::ui::install_fonts(&cc.egui_ctx, &art);
        if std::env::var("GM_UI_ZOOM").is_err() {
            cc.egui_ctx.set_zoom_factor(settings.ui_zoom.unwrap_or(1.0).clamp(0.75, 2.5));
        }
        let shot = std::env::var("GM_UI_SHOT").ok();
        let mut s = Shell {
            root,
            install: install.clone(),
            base,
            form_dir: settings.vic2_dir.clone().or_else(|| install.as_ref().map(|p| p.display().to_string())).unwrap_or_default(),
            form_zoom: settings.ui_zoom.unwrap_or(1.0),
            form_theme: settings.theme.clone().unwrap_or_default(),
            form_msg: String::new(),
            settings,
            theme,
            art,
            screen: Screen::Menu,
            preload: None,
            world: None,
            silhouette: None,
            shot: None,
            frames: 0,
            shot_requested: false,
        };
        match (&install, direct) {
            _ if std::env::args().any(|a| a == "--ui-maker") || std::env::var("GM_UI_SCREEN").as_deref() == Ok("maker") => {
                if let Some(i) = &install {
                    s.preload = Some(spawn_world(&s.root, i));
                }
                s.screen = Screen::Maker(Box::new(crate::maker::Maker::new(&cc.egui_ctx, install.clone(), s.base.clone())));
                s.shot = shot;
            }
            (None, _) => {
                s.screen = Screen::Settings;
                s.form_msg = "Victoria 2 folder not found. Pick it here.".into();
            }
            (Some(_), Some(save)) => {
                // Straight into the game (wiktoria.bat <days>, --save).
                s.screen = Screen::Loading { next: Next::Game(Launch { save, country: u16::MAX, observe: false }), rx: None, since: Instant::now() };
            }
            (Some(i), None) => {
                s.preload = Some(spawn_world(&s.root, i));
                match std::env::var("GM_UI_SCREEN").as_deref() {
                    Ok("start") => s.screen = Screen::Loading { next: Next::Start(false), rx: None, since: Instant::now() },
                    Ok("load") => s.screen = Screen::Loading { next: Next::Start(true), rx: None, since: Instant::now() },
                    Ok("settings") => s.screen = Screen::Settings,
                    _ => {}
                }
                // The menu captures itself; the start screen and the game capture through App.
                if matches!(s.screen, Screen::Menu | Screen::Settings) {
                    s.shot = shot;
                }
            }
        }
        s
    }

    /// Take the preloaded World if ready (non-blocking).
    fn poll_preload(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.preload {
            if let Ok(r) = rx.try_recv() {
                self.preload = None;
                match r {
                    Ok(w) => {
                        if self.silhouette.is_none() {
                            self.silhouette = Some(silhouette(ctx, &w));
                        }
                        self.world = Some(w);
                    }
                    Err(e) => self.screen = Screen::Error(e),
                }
            }
        }
    }

    fn go(&mut self, next: Next) {
        self.screen = Screen::Loading { next, rx: None, since: Instant::now() };
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let rect = ctx.content_rect();
        let th = self.theme;
        self.art.menu_background(ui.painter(), rect, &th, self.silhouette.as_ref());
        let em = ui.text_style_height(&egui::TextStyle::Body);
        let mut action = None;
        egui::Area::new(egui::Id::new("menu_main")).anchor(Align2::CENTER_CENTER, Vec2::new(0.0, -em * 2.0)).show(&ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Wiktoria").family(egui::FontFamily::Name(SERIF.into())).size(em * 6.0).color(th.gold));
                ui.label(RichText::new("a nineteenth-century grand strategy").family(egui::FontFamily::Name(SERIF.into())).size(em * 1.4).color(th.ink_dim));
                ui.add_space(em * 2.5);
                let size = Vec2::new(em * 16.0, em * 2.8);
                for (label, a) in [("New game", 1), ("Load game", 2), ("Settings", 3), ("UI maker", 5), ("Exit", 4)] {
                    if self.art.menu_button(ui, &th, label, size, true).clicked() {
                        action = Some(a);
                    }
                    ui.add_space(em * 0.6);
                }
            });
        });
        egui::Area::new(egui::Id::new("menu_version")).anchor(Align2::RIGHT_BOTTOM, Vec2::new(-12.0, -8.0)).interactable(false).show(&ctx, |ui| {
            ui.label(RichText::new(VERSION).color(th.ink_dim).small());
        });
        if self.preload.is_some() {
            egui::Area::new(egui::Id::new("menu_preload")).anchor(Align2::LEFT_BOTTOM, Vec2::new(12.0, -8.0)).interactable(false).show(&ctx, |ui| {
                ui.label(RichText::new("Reading the Victoria 2 folder...").color(th.ink_dim).small());
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        match action {
            Some(1) => self.go(Next::Start(false)),
            Some(2) => self.go(Next::Start(true)),
            Some(3) => self.screen = Screen::Settings,
            Some(4) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Some(5) => self.screen = Screen::Maker(Box::new(crate::maker::Maker::new(&ctx, self.install.clone(), self.base.clone()))),
            _ => {}
        }
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let th = self.theme;
        self.art.menu_background(ui.painter(), ctx.content_rect(), &th, self.silhouette.as_ref());
        let em = ui.text_style_height(&egui::TextStyle::Body);
        let mut back = false;
        let themes: Vec<String> = std::fs::read_dir(self.base.join("themes"))
            .map(|rd| rd.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        egui::Window::new("Settings").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(&ctx, |ui| {
            ui.set_width(em * 34.0);
            egui::Grid::new("settings_grid").num_columns(2).spacing(Vec2::new(12.0, 10.0)).show(ui, |ui| {
                ui.label("Victoria 2 folder");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.form_dir).desired_width(em * 20.0));
                    if ui.button("Browse...").clicked() {
                        if let Some(p) = rfd::FileDialog::new().set_title("Victoria 2 folder (the one with v2game.exe)").pick_folder() {
                            self.form_dir = p.display().to_string();
                        }
                    }
                });
                ui.end_row();
                ui.label("UI zoom");
                ui.add(egui::Slider::new(&mut self.form_zoom, 0.75..=2.5).fixed_decimals(2));
                ui.end_row();
                ui.label("Theme");
                egui::ComboBox::from_id_salt("theme_pick").selected_text(if self.form_theme.is_empty() { "Default" } else { &self.form_theme }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.form_theme, String::new(), "Default");
                    for t in &themes {
                        ui.selectable_value(&mut self.form_theme, t.clone(), t);
                    }
                });
                ui.end_row();
            });
            if !self.form_msg.is_empty() {
                ui.label(RichText::new(&self.form_msg).color(th.gold));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    let dir_ok = Path::new(&self.form_dir).join("map").is_dir();
                    self.settings.vic2_dir = Some(self.form_dir.clone()).filter(|d| !d.is_empty());
                    self.settings.ui_zoom = Some(self.form_zoom);
                    self.settings.theme = Some(self.form_theme.clone()).filter(|t| !t.is_empty());
                    self.form_msg = match self.settings.save() {
                        Ok(()) => format!("Saved to {}", self.settings.path.display()),
                        Err(e) => e,
                    };
                    if !dir_ok {
                        self.form_msg += "  (that folder has no map/ folder; is it the Victoria 2 folder?)";
                    }
                    ctx.set_zoom_factor(self.form_zoom);
                    let (t, a) = Art::load(&self.base, self.settings.theme.as_deref());
                    self.theme = t;
                    self.art = a;
                    crate::ui::apply_style(&ctx, &t);
                    crate::ui::install_fonts(&ctx, &self.art);
                    let new_install = PathBuf::from(&self.form_dir);
                    if dir_ok && self.install.as_ref() != Some(&new_install) {
                        self.install = Some(new_install.clone());
                        self.world = None;
                        self.silhouette = None;
                        self.preload = Some(spawn_world(&self.root, &new_install));
                    }
                }
                if ui.button("Back").clicked() {
                    back = true;
                }
            });
        });
        if back && self.install.is_some() {
            self.screen = Screen::Menu;
        }
    }

    fn loading_ui(&mut self, ui: &mut egui::Ui, text: &str, since: Instant) {
        let ctx = ui.ctx().clone();
        let th = self.theme;
        self.art.menu_background(ui.painter(), ctx.content_rect(), &th, self.silhouette.as_ref());
        let em = ui.text_style_height(&egui::TextStyle::Body);
        egui::Area::new(egui::Id::new("loading")).anchor(Align2::CENTER_CENTER, Vec2::ZERO).interactable(false).show(&ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Wiktoria").family(egui::FontFamily::Name(SERIF.into())).size(em * 3.0).color(th.gold));
                ui.add_space(em);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(text).color(th.ink));
                });
                ui.label(RichText::new(format!("{:.1} s", since.elapsed().as_secs_f32())).color(th.ink_dim).small());
            });
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }

    fn step_loading(&mut self, ui: &mut egui::Ui) {
        let Screen::Loading { next, rx, since } = &mut self.screen else { return };
        let since = *since;
        let Some(install) = self.install.clone() else { return };
        // Get a World: the preloaded one, else load one on a thread.
        let world = if let Some(w) = self.world.take() {
            Some(Ok(w))
        } else if let Some(p) = self.preload.take() {
            *rx = Some(p);
            None
        } else if rx.is_none() {
            *rx = Some(spawn_world(&self.root, &install));
            None
        } else {
            rx.as_ref().unwrap().try_recv().ok()
        };
        let label = match next {
            Next::Start(_) => "Reading the map, the countries and their history...",
            Next::Game(_) => "Reading the map and the countries...",
        };
        let Some(world) = world else {
            self.loading_ui(ui, label, since);
            return;
        };
        let world = match world {
            Ok(w) => w,
            Err(e) => {
                self.screen = Screen::Error(e);
                return;
            }
        };
        if self.silhouette.is_none() {
            self.silhouette = Some(silhouette(ui.ctx(), &world));
        }
        let ctx = ui.ctx().clone();
        let theme_name = self.settings.theme.clone();
        let next = std::mem::replace(next, Next::Start(false));
        match next {
            Next::Start(saves_only) => {
                let list = entries(&self.root, &install, &world);
                let source: Box<dyn WatchSource> = Box::new(StaticWorldSource::new(world));
                let mut app = App::with_ctx(&ctx, source, install, self.base.clone(), theme_name.as_deref());
                app.start = Some(StartState::new(list, saves_only));
                app.show_labels = false;
                self.screen = Screen::Start(Box::new(app));
                // Keep a World ready for Play.
                self.preload = Some(spawn_world(&self.root, self.install.as_ref().unwrap()));
            }
            Next::Game(l) => {
                println!("Live game from {}", l.save.display());
                let source: Box<dyn WatchSource> = Box::new(crate::sim::GameSource::start(world, self.root.clone(), install.clone(), l.save.clone()));
                let mut app = App::with_ctx(&ctx, source, install, self.base.clone(), theme_name.as_deref());
                if l.country != u16::MAX {
                    app.player = l.country;
                }
                app.observer = l.observe;
                self.screen = Screen::Building { app: Box::new(app), since };
            }
        }
    }
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_preload(&ctx);
        match &mut self.screen {
            Screen::Menu => self.menu(ui),
            Screen::Settings => self.settings_ui(ui),
            Screen::Loading { .. } => self.step_loading(ui),
            Screen::Building { app, since } => {
                app.source.poll();
                let since = *since;
                let ready = app.source.revision() > 0;
                let status = app.source.sim_status().unwrap_or_default();
                if status.starts_with("sim failed") {
                    self.screen = Screen::Error(status);
                } else if ready {
                    let Screen::Building { app, .. } = std::mem::replace(&mut self.screen, Screen::Menu) else { unreachable!() };
                    self.screen = Screen::Game(app);
                } else {
                    let days = std::env::args().any(|a| a == "--start-days") || std::env::var("GM_UI_RUN_DAYS").is_ok();
                    let text = if days { "Building the game and running ahead to the start day..." } else { "Building the game (economy, armies, diplomacy)..." };
                    self.loading_ui(ui, text, since);
                }
            }
            Screen::Start(app) => {
                app.start_ui(ui);
                let st = app.start.as_mut().unwrap();
                if st.back {
                    self.screen = Screen::Menu;
                } else if let Some(l) = st.launch.take() {
                    self.go(Next::Game(l));
                }
            }
            Screen::Game(app) => app.frame_ui(ui),
            Screen::Maker(_) => self.maker_step(ui),
            Screen::Error(e) => {
                let e = e.clone();
                let th = self.theme;
                self.art.menu_background(ui.painter(), ctx.content_rect(), &th, None);
                let mut back = false;
                egui::Window::new("Error").collapsible(false).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(&ctx, |ui| {
                    ui.label(RichText::new(e).color(th.bad));
                    back = ui.button("Back to the menu").clicked();
                });
                if back {
                    self.screen = Screen::Menu;
                }
            }
        }
        self.menu_screenshot(&ctx);
    }
}

impl Shell {
    fn maker_step(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Screen::Maker(m) = &mut self.screen else { return };
        if m.needs_world() {
            if let Some(w) = self.world.take() {
                let install = m.install().unwrap().to_path_buf();
                let source: Box<dyn WatchSource> = Box::new(StaticWorldSource::new(w));
                // App::with_ctx reloads the settings theme's slots; the maker keeps its own table.
                let keep = std::mem::take(&mut crate::slots::get(&ctx).lock().unwrap().map);
                let mut app = App::with_ctx(&ctx, source, install.clone(), self.base.clone(), self.settings.theme.as_deref());
                app.shot_path = None;
                app.show_labels = false;
                let s = crate::slots::get(&ctx);
                let mut s = s.lock().unwrap();
                s.map = keep;
                s.install = Some(install);
                drop(s);
                m.app = Some(Box::new(app));
            } else if self.preload.is_none() {
                if self.install.as_deref() != m.install() {
                    self.install = m.install().map(Path::to_path_buf);
                }
                self.preload = Some(spawn_world(&self.root, m.install().unwrap()));
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        m.ui(ui);
        if m.back {
            crate::slots::install_for(&ctx, &self.base, crate::theme::theme_name(self.settings.theme.as_deref()).as_deref(), self.install.as_deref());
            self.screen = Screen::Menu;
        }
    }

    fn menu_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.shot.clone() else { return };
        // Wait for the silhouette so the backdrop is complete.
        let maker_ready = matches!(&self.screen, Screen::Maker(m) if !m.needs_world());
        if !maker_ready && (self.preload.is_some() || !matches!(self.screen, Screen::Menu | Screen::Settings)) {
            ctx.request_repaint();
            return;
        }
        self.frames += 1;
        if self.frames >= 5 && !self.shot_requested {
            self.shot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let img = ctx.input(|i| i.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
        if let Some(img) = img {
            if let Some(dir) = Path::new(&path).parent() {
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
