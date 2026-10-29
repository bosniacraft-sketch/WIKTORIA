//! Path resolution over the base install plus zero or more mod folders, the way the game layers them.
//!
//! A file at relative path P comes from the last mod that has it, else from the base. A directory a mod
//! lists as `replace_path` hides the base's (and earlier mods') files under it. Directory listings are
//! the union of all layers, a same-named file from a later layer replacing the earlier one. A path
//! written inside a file that points into `mod/<name>/...` (relative to the game folder) is sent to
//! that mod's real folder, wherever it sits.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ModLayer {
    pub root: PathBuf,
    /// Normalised (lowercase, `/`) replace_path entries.
    pub replace: Vec<String>,
    /// Normalised game-relative folder the mod declares, such as `mod/gfm`.
    pub alias: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct InstallFs {
    pub base: PathBuf,
    pub mods: Vec<ModLayer>,
}

/// Lexically normalises a relative path: `/` separators, no `.`/empty parts, `a/../` folded.
fn normalise(rel: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for p in rel.split(['/', '\\']) {
        match p {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|l| *l != "..") {
                    parts.pop();
                } else {
                    parts.push("..");
                }
            }
            _ => parts.push(p),
        }
    }
    parts.join("/")
}

fn under(path: &str, prefix: &str) -> bool {
    path == prefix || (path.len() > prefix.len() && path.starts_with(prefix) && path.as_bytes()[prefix.len()] == b'/')
}

impl ModLayer {
    /// Builds a layer from a `.mod` descriptor. The mod folder is the declared `path` (relative to the
    /// game folder): taken under `game` when it exists there, else next to the descriptor, with the
    /// leading `mod/` dropped (or kept) so a mod stored anywhere is found.
    pub fn from_descriptor(mod_file: &Path, game: &Path) -> Result<ModLayer, String> {
        let bytes = std::fs::read(mod_file).map_err(|e| format!("cannot read {}: {e}", mod_file.display()))?;
        let text = String::from_utf8_lossy(&bytes);
        let mut path = None;
        let mut name = None;
        let mut replace = Vec::new();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim().trim_matches('"').to_string();
            match k.trim() {
                "path" => path = Some(v),
                "name" => name = Some(v),
                "replace_path" => replace.push(normalise(&v).to_lowercase()),
                _ => {}
            }
        }
        let path = path.ok_or(format!("{}: no path = \"...\"", mod_file.display()))?;
        let alias = normalise(&path);
        let dir = mod_file.parent().unwrap_or(Path::new("."));
        let stripped = alias.strip_prefix("mod/").unwrap_or(&alias).to_string();
        let candidates = [game.join(&alias), dir.join(&stripped), dir.join(&alias)];
        let root = candidates.into_iter().find(|c| c.is_dir()).ok_or(format!("mod folder for '{path}' not found (looked in the game folder and next to {})", mod_file.display()))?;
        let name = name.unwrap_or_else(|| mod_file.file_stem().and_then(|s| s.to_str()).unwrap_or("mod").to_string());
        Ok(ModLayer { root, replace, alias: alias.to_lowercase(), name })
    }

    fn replaces(&self, rel: &str) -> bool {
        let l = rel.to_lowercase();
        self.replace.iter().any(|r| under(&l, r))
    }
}

impl InstallFs {
    pub fn base_only(base: &Path) -> InstallFs {
        InstallFs { base: base.to_path_buf(), mods: Vec::new() }
    }

    /// Base install plus the mods named by `.mod` files, in load order (later wins).
    pub fn with_mods(base: &Path, mod_files: &[PathBuf]) -> Result<InstallFs, String> {
        let mods = mod_files.iter().map(|m| ModLayer::from_descriptor(m, base)).collect::<Result<Vec<_>, _>>()?;
        Ok(InstallFs { base: base.to_path_buf(), mods })
    }

    /// If `rel` points into a mod's own folder (`mod/<name>/...` or `../mod/<name>/...` seen from a
    /// game subfolder), the mod layer index and the path inside the mod.
    fn mod_target(&self, rel: &str) -> Option<(usize, String)> {
        if self.mods.is_empty() {
            return None;
        }
        let n = normalise(rel);
        let l = n.to_lowercase();
        self.mods.iter().enumerate().rev().find_map(|(i, m)| {
            if under(&l, &m.alias) {
                Some((i, n[m.alias.len()..].trim_start_matches('/').to_string()))
            } else {
                None
            }
        })
    }

    /// Resolves a game-relative file. Falls back to the base path when nobody has it.
    pub fn file(&self, rel: &str) -> PathBuf {
        if let Some((i, inner)) = self.mod_target(rel) {
            return self.mods[i].root.join(inner);
        }
        for m in self.mods.iter().rev() {
            let p = m.root.join(rel);
            if p.is_file() {
                return p;
            }
            if m.replaces(&normalise(rel)) {
                return p; // hidden from the base: report the mod path (missing)
            }
        }
        self.base.join(rel)
    }

    fn collect(root: &Path, dir: &str, ext: &str, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
        let mut stack = vec![(root.join(dir), dir.to_string())];
        while let Some((d, r)) = stack.pop() {
            for e in std::fs::read_dir(&d).map_err(|e| format!("cannot list {}: {e}", d.display()))? {
                let e = e.map_err(|e| e.to_string())?;
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                let child = if r.is_empty() { name } else { format!("{r}/{name}") };
                if p.is_dir() {
                    stack.push((p, child));
                } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case(ext)) {
                    out.push((child, p));
                }
            }
        }
        Ok(())
    }

    /// Files under `rel` (recursive) with extension `ext`, unioned over the layers and sorted by
    /// relative path. Same-named files from later layers replace earlier ones.
    pub fn dir_files(&self, rel: &str, ext: &str) -> Result<Vec<PathBuf>, String> {
        let rel = normalise(rel);
        let mut acc: Vec<(String, PathBuf)> = Vec::new();
        let mut base_err = None;
        if let Err(e) = Self::collect(&self.base, &rel, ext, &mut acc) {
            base_err = Some(e);
        }
        let mut any = base_err.is_none();
        for m in &self.mods {
            // Drop what this mod hides.
            acc.retain(|(r, _)| !m.replaces(r));
            if m.replaces(&rel) {
                acc.clear();
            }
            if m.root.join(&rel).is_dir() {
                any = true;
                let mut mine = Vec::new();
                Self::collect(&m.root, &rel, ext, &mut mine)?;
                for (r, p) in mine {
                    let key = r.to_lowercase();
                    acc.retain(|(o, _)| o.to_lowercase() != key);
                    acc.push((r, p));
                }
            }
        }
        if !any {
            return Err(base_err.unwrap_or_default());
        }
        let mut keyed: Vec<(PathBuf, PathBuf)> = acc.into_iter().map(|(r, p)| (PathBuf::from(r), p)).collect();
        keyed.sort();
        Ok(keyed.into_iter().map(|(_, p)| p).collect())
    }

    /// Names of the immediate subfolders of `rel`, unioned over the layers, sorted.
    pub fn subdirs(&self, rel: &str) -> Result<Vec<String>, String> {
        let rel = normalise(rel);
        let list = |root: &Path| -> Option<Vec<String>> {
            let rd = std::fs::read_dir(root.join(&rel)).ok()?;
            Some(rd.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).filter_map(|e| e.file_name().to_str().map(str::to_string)).collect())
        };
        let base = list(&self.base);
        let mut any = base.is_some();
        let mut out = base.unwrap_or_default();
        for m in &self.mods {
            if m.replaces(&rel) {
                out.clear();
            }
            if let Some(v) = list(&m.root) {
                any = true;
                out.extend(v);
            }
        }
        if !any {
            return Err(format!("cannot list {}", self.base.join(&rel).display()));
        }
        out.sort();
        out.dedup();
        Ok(out)
    }
}
