//! Small settings file next to the exe (`wiktoria_settings.txt`, `key = value` lines).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub vic2_dir: Option<String>,
    pub ui_zoom: Option<f32>,
    pub theme: Option<String>,
    pub path: PathBuf,
}

impl Settings {
    pub fn load(dir: &Path) -> Settings {
        let path = dir.join("wiktoria_settings.txt");
        let mut kv = HashMap::new();
        if let Ok(s) = std::fs::read_to_string(&path) {
            crate::theme::parse_kv(&s, &mut kv);
        }
        let get = |k: &str| kv.get(k).cloned().filter(|v: &String| !v.is_empty());
        Settings { vic2_dir: get("vic2_dir"), ui_zoom: get("ui_zoom").and_then(|v| v.parse().ok()), theme: get("theme"), path }
    }

    pub fn save(&self) -> Result<(), String> {
        let mut s = String::from("# Wiktoria settings (written by the Settings screen)\n");
        s += &format!("vic2_dir = \"{}\"\n", self.vic2_dir.clone().unwrap_or_default());
        s += &format!("ui_zoom = {}\n", self.ui_zoom.unwrap_or(1.0));
        s += &format!("theme = \"{}\"\n", self.theme.clone().unwrap_or_default());
        std::fs::write(&self.path, s).map_err(|e| format!("cannot write {}: {e}", self.path.display()))
    }
}
