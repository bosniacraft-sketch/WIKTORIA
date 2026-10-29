//! Wiktoria viewer: main menu, start screen and the watch/play view, built on eframe/egui.
//!
//! `viewer [install] [--save <file.v2>] [--start-days N]`: with a save it goes straight into the game
//! (wiktoria.bat <days> uses this). Without one it opens the main menu.
//!
//! Map: drag = pan, wheel = zoom, click = select a province. Space toggles pause.
//! Env vars: `GM_UI_SHOT=<png path>` captures the window after a few frames and exits,
//! `GM_UI_SCREEN=menu|start` picks the screen to capture without a save, `GM_UI_START_TAG=<TAG>`
//! preselects a country on the start screen, `GM_UI_SELECT=<province id>` preselects a province,
//! `GM_UI_SIZE=<w>x<h>` sets the window size, `GM_UI_ZOOM=<factor>` sets the manual UI zoom,
//! `GM_UI_OPEN=<window[,window]>` opens country windows (budget, diplomacy, ledger, ...),
//! `GM_UI_THEME=<name>` picks `themes/<name>/`, `GM_UI_BASE=<dir>` the art base.
//! `viewer --ui-maker` (or `GM_UI_SCREEN=maker`) opens the UI maker.
//!
//! The UI reads only through `watch::WatchSource`; nothing here ticks the simulation.

mod dds;
mod keys;
mod labels;
mod maker;
mod mapview;
mod menu;
mod settings;
mod sim;
mod slots;
mod start;
mod theme;
mod ui;
mod watch;

use std::path::Path;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn env_size() -> [f32; 2] {
    std::env::var("GM_UI_SIZE")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some([w.parse().ok()?, h.parse().ok()?])
        })
        .unwrap_or([1600.0, 900.0])
}

/// Folder holding `theme.toml`, `ui_art/` and `themes/`: next to the exe (override with GM_UI_BASE).
fn art_base() -> std::path::PathBuf {
    std::env::var_os("GM_UI_BASE").map(Into::into).unwrap_or_else(exe_dir)
}

fn exe_dir() -> std::path::PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf())).unwrap_or_default()
}

/// `viewer [install] [--save <file.v2>]`; GM_UI_SAVE also gives the save.
fn save_arg() -> Option<std::path::PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == "--save").and_then(|i| args.get(i + 1)).cloned().or_else(|| std::env::var("GM_UI_SAVE").ok()).map(Into::into)
}

fn positional_install() -> Option<String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--save" || a == "--start-days" {
            it.next();
        } else if !a.starts_with("--") {
            return Some(a.clone());
        }
    }
    None
}

fn run() -> Result<(), String> {
    let root = Path::new(ROOT).to_path_buf();
    let settings = settings::Settings::load(&exe_dir());
    // Install: command line, then the settings file, then VIC2_DIR / vic2_path.txt.
    let install = match positional_install().or_else(|| settings.vic2_dir.clone().filter(|d| Path::new(d).is_dir())) {
        Some(d) => wiktoria_data::config::install_dir(&root, Some(d)),
        None => wiktoria_data::config::install_dir(&root, None),
    };
    let direct = save_arg();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Wiktoria")
            .with_inner_size(env_size())
            .with_min_inner_size([960.0, 540.0]),
        ..Default::default()
    };
    eframe::run_native(
        "wiktoria",
        options,
        Box::new(move |cc| Ok(Box::new(menu::Shell::new(cc, root, install.ok(), art_base(), exe_dir(), settings, direct)))),
    )
    .map_err(|e| format!("window error: {e}"))
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
