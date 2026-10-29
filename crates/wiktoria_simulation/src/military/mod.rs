//! The military half of the game: armies, fleets, regiments, ships, leaders, wars, occupation,
//! sieges and battles in progress.
//!
//! This step holds the state and the loader from an original save, no rules yet. Unit statistics
//! come from the install's unit files at runtime (`units::UnitDef`), engine constants from the
//! rules file (`MilitaryRules`). Rules (movement, battle, siege, war) are added one module at a time
//! by the tasks of the plan, each owning its file.

use std::collections::BTreeMap;
use std::path::Path;

use wiktoria_data::date::Date;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::save::SaveGame;
use wiktoria_data::script::{Block, Value};

pub mod ai;
pub mod battle;
pub mod day;
pub mod logistics;
pub mod movement;
pub mod recruit;
pub mod siege;
pub mod units;
pub mod war;
pub mod war_ai;

use battle::{Combat, CombatKind};
use siege::ProvinceMilitary;
use units::{Army, Construction, Leader, Navy, UnitDef};
use war::War;

pub(crate) fn num(b: &Block, k: &str) -> f64 {
    b.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

pub(crate) fn text(b: &Block, k: &str) -> Option<String> {
    b.get(k).and_then(Value::as_str).map(str::to_string)
}

pub(crate) fn date(b: &Block, k: &str) -> Option<Date> {
    b.get(k).and_then(Value::as_str).and_then(Date::parse)
}

/// The number inside an id record: `leader = { id = 6 type = 37 }` gives 6.
pub(crate) fn id_of(b: &Block, k: &str) -> Option<u64> {
    b.get(k)?.as_block()?.get("id")?.as_f64().map(|x| x as u64)
}

/// What the military code takes from the rules file (`FLD_MIL_*`, new rows are hypotheses) and the
/// install's unit files.
#[derive(Debug, Clone, PartialEq)]
pub struct MilitaryRules {
    /// Tag the save uses for rebels, who own armies and leaders but no land.
    pub rebel_tag: String,
    /// Garrison of a province nobody besieges.
    pub full_garrison: f64,
    /// Men in one point of a regiment's strength (3.0 strength = 3000 men).
    pub men_per_strength: f64,
    /// Every unit type of the install, by name.
    pub units: BTreeMap<String, UnitDef>,
}

impl MilitaryRules {
    /// The rules file part only; `units` is empty.
    pub fn from_engine_rules(m: &EngineRules) -> Result<MilitaryRules, String> {
        Ok(MilitaryRules {
            rebel_tag: m.require_str("FLD_MIL_REBEL_TAG")?.to_string(),
            full_garrison: crate::num_field(m, "FLD_MIL_FULL_GARRISON")?,
            men_per_strength: crate::num_field(m, "FLD_MIL_MEN_PER_STRENGTH")?,
            units: BTreeMap::new(),
        })
    }

    /// The rules file part plus the unit types of the install.
    pub fn new(install: &Path, m: &EngineRules) -> Result<MilitaryRules, String> {
        let mut rules = MilitaryRules::from_engine_rules(m)?;
        rules.units = units::read_unit_defs(&install.join(m.require_str("FLD_UNITS_DIR")?))?.into_iter().map(|u| (u.name.clone(), u)).collect();
        Ok(rules)
    }
}

/// What a save holds about one country's forces besides the units themselves.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MilitaryCountry {
    pub mobilized: bool,
    pub war_exhaustion: f64,
    /// Leadership points saved up.
    pub leadership: f64,
    pub auto_assign_leaders: bool,
    pub auto_create_leaders: bool,
    pub great_power: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MilitaryState {
    pub date: Date,
    /// By tag, rebels included.
    pub countries: BTreeMap<String, MilitaryCountry>,
    pub leaders: Vec<Leader>,
    /// Every army, also those carried by a fleet (`Army::carried_by`).
    pub armies: Vec<Army>,
    pub navies: Vec<Navy>,
    pub constructions: Vec<Construction>,
    /// Running wars first, then the finished ones, each in the save's order.
    pub wars: Vec<War>,
    /// Land battles, naval battles and sieges in progress.
    pub combats: Vec<Combat>,
    /// Owner, controller, garrison, fort and naval base of every province.
    pub provinces: BTreeMap<u16, ProvinceMilitary>,
    pub warnings: Vec<String>,
}

impl MilitaryState {
    /// Reads everything military from an original save.
    pub fn from_save(save: &SaveGame, rules: &MilitaryRules) -> MilitaryState {
        let root = &save.doc.root;
        let mut out = MilitaryState {
            date: save.date,
            countries: BTreeMap::new(),
            leaders: Vec::new(),
            armies: Vec::new(),
            navies: Vec::new(),
            constructions: Vec::new(),
            wars: Vec::new(),
            combats: Vec::new(),
            provinces: BTreeMap::new(),
            warnings: Vec::new(),
        };
        // The pop behind a regiment is named by id only; its type is in the province.
        let pop_types: BTreeMap<u64, String> = save.provinces.values().flat_map(|p| &p.pops).map(|q| (q.id, q.pop_type.clone())).collect();
        // great_nations numbers the country blocks from 1.
        let great: Vec<&String> = root.get("great_nations").and_then(Value::as_block).map(|b| b.bare_scalars()).unwrap_or_default().iter().filter_map(|n| save.countries.get(n.parse::<usize>().ok()?.checked_sub(1)?)).collect();
        for tag in &save.countries {
            let Some(b) = root.get(tag).and_then(Value::as_block) else { continue };
            let flag = |k: &str| b.get(k).and_then(Value::as_bool).unwrap_or(false);
            out.countries.insert(
                tag.clone(),
                MilitaryCountry { mobilized: flag("mobilize"), war_exhaustion: num(b, "war_exhaustion"), leadership: num(b, "leadership"), auto_assign_leaders: flag("auto_assign_leaders"), auto_create_leaders: flag("auto_create_leaders"), great_power: great.contains(&tag) },
            );
            out.leaders.extend(b.get_all("leader").filter_map(Value::as_block).filter_map(|l| units::read_leader(l, tag)));
            for a in b.get_all("army").filter_map(Value::as_block) {
                match units::read_army(a, tag, &pop_types, None) {
                    Some(army) => out.armies.push(army),
                    None => out.warnings.push(format!("{tag}: an army without an id")),
                }
            }
            for n in b.get_all("navy").filter_map(Value::as_block) {
                let Some(mut navy) = units::read_navy(n, tag) else {
                    out.warnings.push(format!("{tag}: a fleet without an id"));
                    continue;
                };
                // An army on board lies inside the navy that carries it.
                for a in n.get_all("army").filter_map(Value::as_block) {
                    if let Some(army) = units::read_army(a, tag, &pop_types, Some(navy.id)) {
                        navy.carried.push(army.id);
                        out.armies.push(army);
                    }
                }
                out.navies.push(navy);
            }
        }
        for army in out.armies.iter().flat_map(|a| &a.regiments).filter(|r| r.pop.is_some() && r.pop_type.is_none()) {
            out.warnings.push(format!("regiment {}: its pop is in no province", army.id));
        }
        for (id, b, _) in province_blocks(root) {
            out.constructions.extend(b.get_all("military_construction").filter_map(Value::as_block).filter_map(units::read_construction));
            out.provinces.insert(id, siege::read_province(b));
        }
        out.wars = war::read_wars(root);
        out.combats = battle::read_combats(root);
        let _ = rules;
        out
    }

    pub fn armies_of<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a Army> + 'a {
        self.armies.iter().filter(move |a| a.owner == tag)
    }

    pub fn navies_of<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a Navy> + 'a {
        self.navies.iter().filter(move |n| n.owner == tag)
    }

    pub fn leaders_of<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a Leader> + 'a {
        self.leaders.iter().filter(move |l| l.owner == tag)
    }

    /// Regiments the country has, in armies on land and on ships.
    pub fn regiment_count(&self, tag: &str) -> usize {
        self.armies_of(tag).map(|a| a.regiments.len()).sum()
    }

    pub fn ship_count(&self, tag: &str) -> usize {
        self.navies_of(tag).map(|n| n.ships.len()).sum()
    }

    /// Running wars the country fights in, on either side.
    pub fn active_wars_of<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a War> + 'a {
        self.wars.iter().filter(move |w| w.active && w.takes_part(tag))
    }

    pub fn sieges(&self) -> impl Iterator<Item = &Combat> {
        self.combats.iter().filter(|c| c.kind == CombatKind::Siege)
    }

    /// Provinces whose controller is not their owner.
    pub fn occupied(&self) -> impl Iterator<Item = (u16, &ProvinceMilitary)> {
        self.provinces.iter().filter(|(_, p)| p.is_occupied()).map(|(id, p)| (*id, p))
    }
}

/// The province blocks of a save: key is a number.
pub(crate) fn province_blocks(root: &Block) -> impl Iterator<Item = (u16, &Block, usize)> {
    root.entries().filter_map(|(k, v, line)| Some((k.parse::<u16>().ok()?, v.as_block()?, line)))
}
