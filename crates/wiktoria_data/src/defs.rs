//! Static definitions from `common/` and friends: cultures, religions, pop types,
//! reforms (issues), technologies, building types and bookmarks.
//! Each parser takes file text and returns plain data; interning lives in [`Index`].

use std::collections::{BTreeMap, HashMap};

use crate::date::Date;
use crate::script::{self, Block, Value};

fn color_255(b: &Block) -> Option<[u8; 3]> {
    let c: Vec<u8> = b.bare_scalars().iter().filter_map(|s| s.parse().ok()).collect();
    (c.len() == 3).then(|| [c[0], c[1], c[2]])
}

/// Colours written as 0..1 fractions (religion.txt).
fn color_unit(b: &Block) -> Option<[u8; 3]> {
    let c: Vec<f64> = b.bare_scalars().iter().filter_map(|s| s.parse().ok()).collect();
    let to = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    (c.len() == 3).then(|| [to(c[0]), to(c[1]), to(c[2])])
}

fn doc(src: &str, label: &str) -> Result<script::Document, String> {
    script::parse(src).map_err(|e| format!("{label}: {e}"))
}

/// Name → index map with insertion order, used for every definition table.
#[derive(Debug, Default, Clone)]
pub struct Index {
    pub names: Vec<String>,
    map: HashMap<String, u16>,
}

impl Index {
    pub fn insert(&mut self, name: &str) -> Result<u16, String> {
        if self.map.contains_key(name) {
            return Err(format!("duplicate definition '{name}'"));
        }
        let i = self.names.len() as u16;
        self.names.push(name.to_string());
        self.map.insert(name.to_string(), i);
        Ok(i)
    }
    pub fn get(&self, name: &str) -> Option<u16> {
        self.map.get(name).copied()
    }
    pub fn len(&self) -> usize {
        self.names.len()
    }
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

// ---------------------------------------------------------------- cultures

#[derive(Debug, Clone, PartialEq)]
pub struct Culture {
    pub name: String,
    pub group: u16,
    pub color: [u8; 3],
    pub radicalism: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CultureGroup {
    pub name: String,
    pub is_overseas: bool,
    /// Union tag, e.g. the tag that unites the group.
    pub union: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct Cultures {
    pub groups: Vec<CultureGroup>,
    pub cultures: Vec<Culture>,
    pub index: Index,
}

/// Group-level keys that are settings, not cultures.
const CULTURE_GROUP_KEYS: [&str; 4] = ["leader", "unit", "is_overseas", "union"];

pub fn parse_cultures(src: &str) -> Result<Cultures, String> {
    let d = doc(src, "cultures.txt")?;
    let mut out = Cultures::default();
    for (gname, gval, line) in d.root.entries() {
        let g = gval.as_block().ok_or(format!("cultures.txt line {line}: group '{gname}' is not a block"))?;
        let gi = out.groups.len() as u16;
        out.groups.push(CultureGroup {
            name: gname.to_string(),
            is_overseas: g.get("is_overseas").and_then(Value::as_bool).unwrap_or(false),
            union: g.get("union").and_then(Value::as_str).map(str::to_string),
        });
        for (cname, cval, cline) in g.entries() {
            if CULTURE_GROUP_KEYS.contains(&cname) {
                continue;
            }
            let c = cval.as_block().ok_or(format!("cultures.txt line {cline}: culture '{cname}' is not a block"))?;
            out.index.insert(cname).map_err(|e| format!("cultures.txt line {cline}: {e}"))?;
            out.cultures.push(Culture {
                name: cname.to_string(),
                group: gi,
                color: c.get("color").and_then(Value::as_block).and_then(color_255).unwrap_or([128, 128, 128]),
                radicalism: c.get("radicalism").and_then(Value::as_f64).unwrap_or(0.0),
            });
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- religions

#[derive(Debug, Clone, PartialEq)]
pub struct Religion {
    pub name: String,
    pub group: String,
    pub color: [u8; 3],
    pub icon: u32,
}

#[derive(Debug, Default, Clone)]
pub struct Religions {
    pub religions: Vec<Religion>,
    pub index: Index,
}

pub fn parse_religions(src: &str) -> Result<Religions, String> {
    let d = doc(src, "religion.txt")?;
    let mut out = Religions::default();
    for (gname, gval, line) in d.root.entries() {
        let g = gval.as_block().ok_or(format!("religion.txt line {line}: group '{gname}' is not a block"))?;
        for (rname, rval, rline) in g.entries() {
            let Some(r) = rval.as_block() else { continue };
            out.index.insert(rname).map_err(|e| format!("religion.txt line {rline}: {e}"))?;
            out.religions.push(Religion {
                name: rname.to_string(),
                group: gname.to_string(),
                color: r.get("color").and_then(Value::as_block).and_then(color_unit).unwrap_or([128, 128, 128]),
                icon: r.get("icon").and_then(Value::as_f64).unwrap_or(0.0) as u32,
            });
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- pop types

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PopType {
    pub name: String,
    pub color: [u8; 3],
    pub strata: String,
    pub sprite: u32,
    /// `state_capital_only = yes`: these pops live only in the state capital.
    pub state_capital_only: bool,
    /// `equivalent = <type>`: the type this one turns into when the province's RGO employs that
    /// type instead (farmers <-> labourers).
    pub equivalent: Option<String>,
    /// `merge_max_size`: identical pops merge only while the result stays at or below this.
    pub merge_max_size: Option<f64>,
    /// `max_size`: largest size of one pop of this type.
    pub max_size: Option<f64>,
    /// `is_slave = yes` (top level of the file).
    pub is_slave: bool,
    pub life_needs: BTreeMap<String, f64>,
    pub everyday_needs: BTreeMap<String, f64>,
    pub luxury_needs: BTreeMap<String, f64>,
}

fn parse_needs(b: &Block, k: &str) -> BTreeMap<String, f64> {
    b.get(k)
        .and_then(Value::as_block)
        .map(|inner| inner.entries().filter_map(|(name, val, _)| Some((name.to_string(), val.as_f64()?))).collect())
        .unwrap_or_default()
}

/// One `poptypes/<name>.txt` file; the type's name is the file stem.
pub fn parse_pop_type(name: &str, src: &str) -> Result<PopType, String> {
    let d = doc(src, &format!("poptypes/{name}.txt"))?;
    let r = &d.root;
    Ok(PopType {
        name: name.to_string(),
        color: r.get("color").and_then(Value::as_block).and_then(color_255).unwrap_or([128, 128, 128]),
        strata: r.get("strata").and_then(Value::as_str).unwrap_or("poor").to_string(),
        sprite: r.get("sprite").and_then(Value::as_f64).unwrap_or(0.0) as u32,
        state_capital_only: r.get("state_capital_only").and_then(Value::as_bool).unwrap_or(false),
        equivalent: r.get("equivalent").and_then(Value::as_str).map(str::to_string),
        merge_max_size: r.get("merge_max_size").and_then(Value::as_f64),
        max_size: r.get("max_size").and_then(Value::as_f64),
        is_slave: r.get("is_slave").and_then(Value::as_bool).unwrap_or(false),
        life_needs: parse_needs(r, "life_needs"),
        everyday_needs: parse_needs(r, "everyday_needs"),
        luxury_needs: parse_needs(r, "luxury_needs"),
    })
}

// ---------------------------------------------------------------- ideologies

/// `common/ideologies.txt`: ideology names in file order (groups hold the ideologies).
/// A save numbers ideologies by this order, starting at 1.
pub fn parse_ideologies(src: &str) -> Result<Vec<String>, String> {
    Ok(parse_ideology_dates(src)?.into_iter().map(|(name, _)| name).collect())
}

/// Ideology names in file order, each with its `date` (the day it becomes available), if any.
pub fn parse_ideology_dates(src: &str) -> Result<Vec<(String, Option<crate::date::Date>)>, String> {
    let d = doc(src, "ideologies.txt")?;
    let mut out = Vec::new();
    for (_, gval, _) in d.root.entries() {
        let Some(g) = gval.as_block() else { continue };
        for (name, v, _) in g.entries() {
            if let Some(b) = v.as_block() {
                out.push((name.to_string(), b.get("date").and_then(Value::as_str).and_then(crate::date::Date::parse)));
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- issues (reforms)

#[derive(Debug, Clone, PartialEq)]
pub struct IssueGroup {
    /// `party_issues`, `political_reforms`, `social_reforms`, `economic_reforms` or `military_reforms`.
    pub category: String,
    pub name: String,
    /// Options in file order (reform steps are ordered).
    pub options: Vec<String>,
    /// Per option (same order): its `rules = { name = yes/no }`, e.g. `slavery_allowed`.
    pub rules: Vec<BTreeMap<String, bool>>,
    pub next_step_only: bool,
}

pub fn parse_issues(src: &str) -> Result<Vec<IssueGroup>, String> {
    let d = doc(src, "issues.txt")?;
    let mut out = Vec::new();
    for (cat, cval, _) in d.root.entries() {
        let Some(c) = cval.as_block() else { continue };
        for (gname, gval, gline) in c.entries() {
            let g = gval.as_block().ok_or(format!("issues.txt line {gline}: '{gname}' is not a block"))?;
            let opts: Vec<(&str, &Block)> = g.entries().filter_map(|(k, v, _)| Some((k, v.as_block()?))).collect();
            let options = opts.iter().map(|(k, _)| k.to_string()).collect();
            let rules = opts
                .iter()
                .map(|(_, b)| {
                    b.get("rules")
                        .and_then(Value::as_block)
                        .map(|r| r.entries().filter_map(|(k, v, _)| Some((k.to_string(), v.as_bool()?))).collect())
                        .unwrap_or_default()
                })
                .collect();
            out.push(IssueGroup {
                category: cat.to_string(),
                name: gname.to_string(),
                options,
                rules,
                next_step_only: g.get("next_step_only").and_then(Value::as_bool).unwrap_or(false),
            });
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- production types (RGOs)

/// For every good produced by an RGO (`type = rgo`), the pop types it employs, in file order.
/// Production types may take their fields from a `template = <other production type>`.
pub fn parse_rgo_employees(src: &str) -> Result<BTreeMap<String, Vec<String>>, String> {
    let d = doc(src, "production_types.txt")?;
    let blocks: HashMap<&str, &Block> = d.root.entries().filter_map(|(k, v, _)| Some((k, v.as_block()?))).collect();
    // A field from the production type itself, else from its template (one level, as in the files).
    let field = |b: &Block, k: &str| -> Option<Value> {
        b.get(k).cloned().or_else(|| {
            let t = b.get("template")?.as_str()?;
            blocks.get(t)?.get(k).cloned()
        })
    };
    let mut out = BTreeMap::new();
    for (_, b) in d.root.entries().filter_map(|(k, v, _)| Some((k, v.as_block()?))) {
        let Some(good) = b.get("output_goods").and_then(Value::as_str) else { continue };
        if field(b, "type").as_ref().and_then(Value::as_str) != Some("rgo") {
            continue;
        }
        let employees: Vec<String> = field(b, "employees")
            .as_ref()
            .and_then(Value::as_block)
            .map(|e| {
                e.items
                    .iter()
                    .filter_map(|i| i.value.as_block()?.get("poptype")?.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        out.insert(good.to_string(), employees);
    }
    Ok(out)
}

// ---------------------------------------------------------------- technologies

#[derive(Debug, Clone, PartialEq)]
pub struct Technology {
    pub name: String,
    pub area: String,
    pub year: i32,
    pub cost: f64,
    /// The numeric entries of the technology's block (`prestige`, `increase_research`, ...).
    pub values: BTreeMap<String, f64>,
    /// The file it is defined in, without the ending (`army_tech`, ...): the folder a
    /// technology school's research bonus is written for.
    pub folder: String,
    /// The technology's `ai_chance` block: how likely a country is to pick it.
    pub ai_chance: Option<crate::script::Block>,
}

/// One `technologies/<folder>.txt` file.
pub fn parse_technologies(src: &str, label: &str) -> Result<Vec<Technology>, String> {
    let d = doc(src, label)?;
    Ok(d.root
        .entries()
        .filter_map(|(name, v, _)| {
            let b = v.as_block()?;
            Some(Technology {
                name: name.to_string(),
                area: b.get("area").and_then(Value::as_str).unwrap_or("").to_string(),
                year: b.get("year").and_then(Value::as_f64).unwrap_or(0.0) as i32,
                cost: b.get("cost").and_then(Value::as_f64).unwrap_or(0.0),
                values: crate::modifiers::modifier_values(b),
                folder: label.trim_end_matches(".txt").to_string(),
                ai_chance: b.get("ai_chance").and_then(Value::as_block).cloned(),
            })
        })
        .collect())
}

/// Tech folder names listed in `common/technology.txt` under `folders = { name = { areas } }`.
pub fn parse_tech_folders(src: &str) -> Result<Vec<String>, String> {
    let d = doc(src, "technology.txt")?;
    let folders = d.root.get("folders").and_then(Value::as_block).ok_or("technology.txt: no `folders` block")?;
    Ok(folders.entries().map(|(k, _, _)| k.to_string()).collect())
}

/// Invention names from one `inventions/*.txt` file (top-level blocks).
pub fn parse_inventions(src: &str, label: &str) -> Result<Vec<String>, String> {
    let d = doc(src, label)?;
    Ok(d.root.entries().filter(|(_, v, _)| v.as_block().is_some()).map(|(k, _, _)| k.to_string()).collect())
}

// ---------------------------------------------------------------- buildings

#[derive(Debug, Clone, PartialEq)]
pub struct BuildingType {
    pub name: String,
    /// `factory`, `fort`, `naval_base`, `infrastructure`, ...
    pub kind: String,
    pub max_level: u32,
    /// Every numeric entry (cost, time, per-level effects such as fort_level or infrastructure).
    pub values: crate::modifiers::ModifierValues,
}

pub fn parse_buildings(src: &str) -> Result<Vec<BuildingType>, String> {
    let d = doc(src, "buildings.txt")?;
    Ok(d.root
        .entries()
        .filter_map(|(name, v, _)| {
            let b = v.as_block()?;
            Some(BuildingType {
                name: name.to_string(),
                kind: b.get("type").and_then(Value::as_str).unwrap_or("").to_string(),
                max_level: b.get("max_level").and_then(Value::as_f64).unwrap_or(0.0) as u32,
                values: crate::modifiers::modifier_values(b),
            })
        })
        .collect())
}

// ---------------------------------------------------------------- bookmarks

#[derive(Debug, Clone, PartialEq)]
pub struct Bookmark {
    pub name: String,
    pub date: Date,
    /// Map position, in the game's coordinates (y counted from the bottom/south).
    pub camera: (f64, f64),
}

pub fn parse_bookmarks(src: &str) -> Result<Vec<Bookmark>, String> {
    let d = doc(src, "bookmarks.txt")?;
    let mut out = Vec::new();
    for b in d.root.get_all("bookmark").filter_map(Value::as_block) {
        let date = b.get("date").and_then(Value::as_str).and_then(Date::parse).ok_or("bookmarks.txt: bookmark without date")?;
        let num = |k: &str| b.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        out.push(Bookmark {
            name: b.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            date,
            camera: (num("cameraX"), num("cameraY")),
        });
    }
    Ok(out)
}
