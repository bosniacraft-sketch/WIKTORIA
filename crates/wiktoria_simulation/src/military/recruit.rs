//! Units under construction and the computer's unit orders.
//!
//! Measured on the test saves of 1836 :
//! - A unit is ordered with its build cost times 2 - administrative efficiency (the inner
//!   `input_goods` of a `military_construction`). The state's stockpile buys for the sites, per
//!   good, the project slider times a tenth of ONE site's whole list (the same share as a
//!   province building site, FLD_BLD_DAILY_ASK_SHARE), and the sites take it in their order.
//! - While a site still lacks goods its start and end move with the day; the build time only
//!   runs once everything is in.
//! - A computer country orders at most one unit a day, regiments while its soldier pops support
//!   more than it has and builds, a ship only while none of its ships waits for goods.
//!   THE PORT'S STAND-INS: the mix of types (`land_class`, `sea_class`), ships only once the
//!   land is full and while the naval supply has room, the pop a regiment comes from (the first
//!   with room in province order).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::script;

use super::units::UnitDef;

/// Which technology or invention unlocks which unit (`activate_unit`; one per block).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UnitUnlocks {
    pub by_tech: BTreeMap<String, String>,
    pub by_invention: BTreeMap<String, String>,
}

fn unlocks_in(dir: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    let mut files: Vec<_> = std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "txt")).collect();
    files.sort();
    for file in files {
        let doc = script::parse_bytes(&std::fs::read(&file).map_err(|e| format!("cannot read {}: {e}", file.display()))?).map_err(|e| format!("{}: {e}", file.display()))?;
        for (name, v, _) in doc.root.entries() {
            if let Some(unit) = v.as_block().and_then(|b| crate::script::activations(b).1) {
                out.insert(name.to_string(), unit);
            }
        }
    }
    Ok(out)
}

impl UnitUnlocks {
    pub fn new(install: &Path, m: &EngineRules) -> Result<UnitUnlocks, String> {
        Ok(UnitUnlocks { by_tech: unlocks_in(&install.join(m.require_str("FLD_TECHNOLOGY_DIR")?))?, by_invention: unlocks_in(&install.join(m.require_str("FLD_INVENTIONS_DIR")?))? })
    }

    /// The units a country may build: active from the start, or unlocked by one of its
    /// technologies or inventions.
    pub fn unlocked(&self, units: &BTreeMap<String, UnitDef>, technologies: &BTreeSet<String>, inventions: &BTreeSet<String>) -> BTreeSet<String> {
        let mut out: BTreeSet<String> = units.values().filter(|u| u.active).map(|u| u.name.clone()).collect();
        out.extend(technologies.iter().filter_map(|t| self.by_tech.get(t)).cloned());
        out.extend(inventions.iter().filter_map(|i| self.by_invention.get(i)).cloned());
        out
    }
}

/// The best unit of a `unit_type` the country may build: the highest `priority`. Units raised
/// only from the primary culture (the guard) are left to the player (STAND-IN: the computer of
/// the test saves orders none).
pub fn best_unit<'a>(units: &'a BTreeMap<String, UnitDef>, unlocked: &BTreeSet<String>, unit_type: &str) -> Option<&'a UnitDef> {
    units.values().filter(|u| u.kind == unit_type && !u.primary_culture_only && unlocked.contains(&u.name)).max_by(|a, b| a.priority.total_cmp(&b.priority))
}

/// The `unit_type` of the next regiment, from what the country has and builds by type. STAND-IN
/// for the original's choice: artillery up to a quarter of the regiments, cavalry up to a
/// seventh (a tenth without artillery), the rest infantry; on 1.1.1840 the original's Britain
/// has 70 infantry, 17 artillery and 12 cavalry built or building, China 203 irregulars and 30
/// cavalry.
pub fn land_class(infantry: usize, support: usize, cavalry: usize, has_support: bool) -> &'static str {
    let total = (infantry + support + cavalry + 1) as f64;
    let (want_support, want_cavalry) = if has_support { (0.25, 1.0 / 7.0) } else { (0.0, 0.1) };
    let lack_support = want_support * total - support as f64;
    let lack_cavalry = want_cavalry * total - cavalry as f64;
    if lack_support >= 1.0 && lack_support >= lack_cavalry {
        "support"
    } else if lack_cavalry >= 1.0 {
        "cavalry"
    } else {
        "infantry"
    }
}

/// The `unit_type` of the next ship. STAND-IN: big ships, light ships and transports at
/// 35 / 45 / 20 of the fleet.
pub fn sea_class(big: usize, light: usize, transport: usize) -> &'static str {
    let total = (big + light + transport + 1) as f64;
    let lacks = [("big_ship", 0.35 * total - big as f64), ("light_ship", 0.45 * total - light as f64), ("transport", 0.2 * total - transport as f64)];
    lacks.iter().max_by(|a, b| a.1.total_cmp(&b.1)).map_or("light_ship", |x| x.0)
}

/// The goods a unit is ordered with: its build cost times 2 - administrative efficiency.
pub fn order_goods(unit: &UnitDef, admin_efficiency: f64) -> BTreeMap<String, f64> {
    let k = 2.0 - admin_efficiency.clamp(0.0, 1.0);
    unit.build_cost.iter().map(|(g, x)| (g.clone(), x * k)).collect()
}

/// A site takes what it lacks out of the stockpile. True when it lacks nothing any more.
pub fn site_takes(lacks: &mut BTreeMap<String, f64>, stockpile: &mut BTreeMap<String, f64>) -> bool {
    for (good, lack) in lacks.iter_mut() {
        let Some(stock) = stockpile.get_mut(good) else { continue };
        let take = lack.min(*stock).max(0.0);
        (*lack, *stock) = (*lack - take, *stock - take);
    }
    lacks.retain(|_, x| *x > 1e-6);
    lacks.is_empty()
}

/// What the stockpile buys for the waiting sites tomorrow, per good: the largest ask of any one
/// site, an ask being `share` of its whole list, at most what it still lacks (as
/// `buildings::daily_ask`; the market multiplies by the project slider).
pub fn sites_ask<'a>(sites: impl Iterator<Item = (&'a BTreeMap<String, f64>, &'a BTreeMap<String, f64>)>, share: f64) -> BTreeMap<String, f64> {
    let mut out: BTreeMap<String, f64> = BTreeMap::new();
    for (lacks, full) in sites {
        for (good, lack) in lacks {
            let x = lack.min(full.get(good).copied().unwrap_or(0.0) * share);
            let e = out.entry(good.clone()).or_default();
            *e = e.max(x);
        }
    }
    out
}
