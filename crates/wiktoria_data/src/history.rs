//! Province history (`history/provinces/<region>/<id> - <name>.txt`) evaluated at a date.
//!
//! Undated keys are the initial state. Dated blocks (`1861.1.1 = { ... }`) apply in date
//! order when their date is on or before the requested date.

use crate::date::Date;
use crate::script::{Block, Document, Value};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProvinceHistory {
    /// `terrain = x` overrides the terrain map.
    pub terrain: Option<String>,
    pub owner: Option<String>,
    pub controller: Option<String>,
    pub cores: Vec<String>,
    pub trade_goods: Option<String>,
    pub life_rating: Option<f64>,
    pub colonial: Option<f64>,
    pub fort: u32,
    pub naval_base: u32,
    pub railroad: u32,
    /// Factories placed in the province's state (`state_building = { level building upgrade }`).
    pub state_buildings: Vec<StateBuilding>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StateBuilding {
    pub building: String,
    pub level: u32,
    pub upgrade: bool,
}

/// Leading digits of a history file name: `"300 - London.txt"` → 300.
pub fn province_id_from_filename(name: &str) -> Option<u16> {
    let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

impl ProvinceHistory {
    /// Applies one history document at `date` on top of the current state.
    pub fn apply(&mut self, doc: &Document, date: Date, warnings: &mut Vec<String>, label: &str) {
        let mut dated: Vec<(Date, &Block)> = Vec::new();
        for (key, value, line) in doc.root.entries() {
            if let Some(d) = Date::parse(key) {
                match value.as_block() {
                    Some(b) => dated.push((d, b)),
                    None => warnings.push(format!("{label} line {line}: dated entry {key} is not a block")),
                }
            } else {
                self.apply_key(key, value, warnings, label, line);
            }
        }
        dated.sort_by_key(|(d, _)| *d); // stable: same-date blocks keep file order
        for (d, block) in dated {
            if d <= date {
                for (key, value, line) in block.entries() {
                    self.apply_key(key, value, warnings, label, line);
                }
            }
        }
    }

    fn apply_key(&mut self, key: &str, value: &Value, warnings: &mut Vec<String>, label: &str, line: usize) {
        let text = || value.as_str().map(str::to_string);
        match key {
            "owner" => self.owner = text(),
            "controller" => self.controller = text(),
            "add_core" => {
                if let Some(t) = text()
                    && !self.cores.contains(&t)
                {
                    self.cores.push(t);
                }
            }
            "remove_core" => {
                if let Some(t) = value.as_str() {
                    self.cores.retain(|c| c != t);
                }
            }
            "trade_goods" => self.trade_goods = text(),
            "life_rating" => self.life_rating = value.as_f64(),
            "colonial" => self.colonial = value.as_f64(),
            "fort" => self.fort = value.as_f64().unwrap_or(0.0) as u32,
            "naval_base" => self.naval_base = value.as_f64().unwrap_or(0.0) as u32,
            "railroad" => self.railroad = value.as_f64().unwrap_or(0.0) as u32,
            "state_building" => match value.as_block() {
                Some(b) => match b.get("building").and_then(Value::as_str) {
                    Some(name) => self.state_buildings.push(StateBuilding {
                        building: name.to_string(),
                        level: b.get("level").and_then(Value::as_f64).unwrap_or(1.0) as u32,
                        upgrade: b.get("upgrade").and_then(Value::as_bool).unwrap_or(false),
                    }),
                    None => warnings.push(format!("{label} line {line}: state_building without building")),
                },
                None => warnings.push(format!("{label} line {line}: state_building is not a block")),
            },
            // Handled by later phases.
            "terrain" => self.terrain = value.as_str().map(str::to_string),
            "party_loyalty" | "is_slave" | "colony" => {}
            other => warnings.push(format!("{label} line {line}: unhandled history key '{other}'")),
        }
    }
}
