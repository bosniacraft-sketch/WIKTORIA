//! Starting population: `history/pops/<date>/*.txt`, blocks keyed by province id:
//! `300 = { farmers = { culture = british religion = protestant size = 12000 } ... }`.

use std::collections::HashMap;

use crate::date::Date;
use crate::map::ProvinceId;
use crate::script::{Document, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct PopEntry {
    pub pop_type: String,
    pub culture: String,
    pub religion: String,
    pub size: f64,
    pub militancy: f64,
    pub rebel_type: Option<String>,
}

/// Pops per province from one file. Repeated province blocks (across files too) add up.
pub fn parse_pop_file(doc: &Document, label: &str, out: &mut HashMap<ProvinceId, Vec<PopEntry>>, warnings: &mut Vec<String>) {
    for (key, value, line) in doc.root.entries() {
        let Ok(id) = key.parse::<ProvinceId>() else {
            warnings.push(format!("{label} line {line}: '{key}' is not a province id"));
            continue;
        };
        let Some(block) = value.as_block() else {
            warnings.push(format!("{label} line {line}: province {id} is not a block"));
            continue;
        };
        let list = out.entry(id).or_default();
        for (pop_type, pv, pline) in block.entries() {
            let Some(p) = pv.as_block() else {
                warnings.push(format!("{label} line {pline}: pop '{pop_type}' is not a block"));
                continue;
            };
            let text = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
            let (Some(culture), Some(religion), Some(size)) = (text("culture"), text("religion"), p.get("size").and_then(Value::as_f64))
            else {
                warnings.push(format!("{label} line {pline}: pop '{pop_type}' needs culture, religion and size"));
                continue;
            };
            for (k, _, l) in p.entries() {
                if !matches!(k, "culture" | "religion" | "size" | "militancy" | "rebel_type") {
                    warnings.push(format!("{label} line {l}: unhandled pop key '{k}'"));
                }
            }
            list.push(PopEntry {
                pop_type: pop_type.to_string(),
                culture,
                religion,
                size,
                militancy: p.get("militancy").and_then(Value::as_f64).unwrap_or(0.0),
                rebel_type: text("rebel_type"),
            });
        }
    }
}

/// Picks the pop folder for a start date: the latest dated folder on or before it.
/// `names` are folder names such as "1836.1.1".
pub fn pick_pop_folder(names: &[String], start: Date) -> Option<&str> {
    names
        .iter()
        .filter_map(|n| Date::parse(n).map(|d| (d, n.as_str())))
        .filter(|(d, _)| *d <= start)
        .max_by_key(|(d, _)| *d)
        .map(|(_, n)| n)
}
