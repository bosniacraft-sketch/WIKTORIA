//! Locating the repo's engine_rules and the player's install.

use std::path::{Path, PathBuf};

use crate::engine_rules::EngineRules;

/// `repo_root` is the wiktoria-cli folder (apps pass `concat!(env!("CARGO_MANIFEST_DIR"), "/../..")`).
pub fn load_engine_rules(repo_root: &Path) -> Result<EngineRules, String> {
    let path = repo_root.join("data/engine_rules.json");
    let src = std::fs::read_to_string(&path).map_err(|e| {
        format!("cannot read {}: {e}", path.display())
    })?;
    EngineRules::from_json(&src)
}

/// Install folder: explicit argument, else VIC2_DIR, else first line of `<repo>/vic2_path.txt`.
pub fn install_dir(repo_root: &Path, arg: Option<String>) -> Result<PathBuf, String> {
    let dir: PathBuf = if let Some(a) = arg {
        a.into()
    } else if let Ok(v) = std::env::var("VIC2_DIR") {
        v.into()
    } else {
        let text = std::fs::read_to_string(repo_root.join("vic2_path.txt")).unwrap_or_default();
        let line = text.lines().next().unwrap_or("").trim().trim_matches('"').to_string();
        if line.is_empty() {
            return Err("No Victoria 2 folder given. Put its path (the folder containing v2game.exe) in \
                        vic2_path.txt, set VIC2_DIR, or pass it as the first argument."
                .into());
        }
        line.into()
    };
    if !dir.is_dir() {
        return Err(format!("Victoria 2 folder not found: {}", dir.display()));
    }
    Ok(dir)
}
