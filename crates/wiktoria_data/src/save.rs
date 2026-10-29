//! Reader for original Victoria 2 save games (`.v2`): plain Windows-1252 text in the same
//! script format as the game files.
//!
//! Only the parts the port uses so far are extracted; the full parsed document is kept so
//! later phases (and the save writer) can reach everything else.

use std::collections::{BTreeMap, HashSet};

use crate::date::Date;
use crate::map::ProvinceId;
use crate::script::{self, Block, Document, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct SavePop {
    pub pop_type: String,
    pub id: u64,
    pub size: f64,
    pub culture: String,
    pub religion: String,
    pub money: f64,
    pub literacy: f64,
    pub militancy: f64,
    pub consciousness: f64,
    /// Ideology support by ideology index, as written in the save (`ideology = { 2=11.03 ... }`).
    pub ideology: BTreeMap<u32, f64>,
    /// Issue support by issue index (`issues = { ... }`).
    pub issues: BTreeMap<u32, f64>,
    /// How far each tier of needs was met (0..1). The game leaves a tier out when it is fully
    /// met, so a missing entry reads as 1.
    pub life_needs: f64,
    pub everyday_needs: f64,
    pub luxury_needs: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SaveProvince {
    pub name: String,
    pub owner: Option<String>,
    pub controller: Option<String>,
    pub cores: Vec<String>,
    pub pops: Vec<SavePop>,
}

impl SaveProvince {
    pub fn population(&self) -> f64 {
        self.pops.iter().map(|p| p.size).sum()
    }
}

#[derive(Debug)]
pub struct SaveGame {
    pub date: Date,
    pub start_date: Option<Date>,
    pub player: Option<String>,
    pub provinces: BTreeMap<ProvinceId, SaveProvince>,
    /// Country tags present as top-level blocks.
    pub countries: Vec<String>,
    pub doc: Document,
    pub warnings: Vec<String>,
}

fn text(b: &Block, k: &str) -> Option<String> {
    b.get(k).and_then(Value::as_str).map(str::to_string)
}

/// A block of `<index>=<number>` entries, such as a pop's `ideology` or `issues`.
fn indexed(b: &Block, k: &str) -> BTreeMap<u32, f64> {
    let Some(inner) = b.get(k).and_then(Value::as_block) else { return BTreeMap::new() };
    inner.entries().filter_map(|(k, v, _)| Some((k.parse().ok()?, v.as_f64()?))).collect()
}

fn is_tag(k: &str) -> bool {
    k.len() == 3 && k.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// Parses a save. `pop_types` and `cultures` come from the install: a pop is a province entry
/// whose key is a pop type, and its culture/religion is written as `<culture>=<religion>`.
pub fn parse_save(bytes: &[u8], pop_types: &HashSet<String>, cultures: &HashSet<String>) -> Result<SaveGame, String> {
    let doc = script::parse_bytes(bytes).map_err(|e| format!("save: {e}"))?;
    let mut warnings = doc.warnings.clone();
    let r = &doc.root;
    let date = text(r, "date").and_then(|d| Date::parse(&d)).ok_or("save: no valid `date`")?;
    let start_date = text(r, "start_date").and_then(|d| Date::parse(&d));
    let player = text(r, "player");

    let mut provinces = BTreeMap::new();
    let mut countries = Vec::new();
    for (key, value, line) in r.entries() {
        let Some(b) = value.as_block() else { continue };
        if let Ok(id) = key.parse::<ProvinceId>() {
            let mut p = SaveProvince {
                name: text(b, "name").unwrap_or_default(),
                owner: text(b, "owner"),
                controller: text(b, "controller"),
                cores: b.get_all("core").filter_map(Value::as_str).map(str::to_string).collect(),
                pops: Vec::new(),
            };
            for (pk, pv, pline) in b.entries() {
                if !pop_types.contains(pk) {
                    continue;
                }
                let Some(pb) = pv.as_block() else { continue };
                let num = |k: &str| pb.get(k).and_then(Value::as_f64);
                let cr = pb.entries().find(|(k, v, _)| cultures.contains(*k) && v.as_str().is_some());
                match (num("size"), cr) {
                    (Some(size), Some((culture, religion, _))) => p.pops.push(SavePop {
                        pop_type: pk.to_string(),
                        id: num("id").unwrap_or(0.0) as u64,
                        size,
                        culture: culture.to_string(),
                        religion: religion.as_str().unwrap_or_default().to_string(),
                        money: num("money").unwrap_or(0.0),
                        literacy: num("literacy").unwrap_or(0.0),
                        militancy: num("mil").unwrap_or(0.0),
                        consciousness: num("con").unwrap_or(0.0),
                        ideology: indexed(pb, "ideology"),
                        issues: indexed(pb, "issues"),
                        // The game leaves a tier out when it is fully met, and also when it is
                        // empty because the tier below it is not met.
                        life_needs: num("life_needs").unwrap_or(1.0),
                        everyday_needs: num("everyday_needs").unwrap_or(if num("life_needs").unwrap_or(1.0) < 1.0 { 0.0 } else { 1.0 }),
                        luxury_needs: num("luxury_needs").unwrap_or(if num("life_needs").unwrap_or(1.0) < 1.0 || num("everyday_needs").unwrap_or(1.0) < 1.0 { 0.0 } else { 1.0 }),
                    }),
                    _ => warnings.push(format!("save line {pline}: province {id} pop '{pk}' without size or culture=religion")),
                }
            }
            if provinces.insert(id, p).is_some() {
                warnings.push(format!("save line {line}: province {id} appears twice"));
            }
        } else if is_tag(key) {
            countries.push(key.to_string());
        }
    }
    Ok(SaveGame { date, start_date, player, provinces, countries, doc, warnings })
}
