//! Unit types (read from the install at runtime), regiments, ships, armies, fleets, leaders and
//! units under construction, as an original save holds them.
//!
//! Rules for them (recruiting, supply, reinforcement, attrition, leaders) belong to task A of the plan.

use std::collections::BTreeMap;
use std::path::Path;

use wiktoria_data::date::Date;
use wiktoria_data::script::{self, Block, Value};

use super::{date, id_of, num, text};

/// One unit type file of the install (`units/<name>.txt`). Nothing here is copied into the repo.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UnitDef {
    pub name: String,
    /// `type = land`, otherwise a ship.
    pub land: bool,
    /// `unit_type`: infantry, cavalry, special, support, big_ship, light_ship, transport.
    pub kind: String,
    pub capital: bool,
    pub transport: bool,
    pub sail: bool,
    /// `active = no`: available only after a technology or invention unlocks it. Absent = yes.
    pub active: bool,
    pub priority: f64,
    pub max_strength: f64,
    pub organisation: f64,
    pub speed: f64,
    pub weighted_value: f64,
    pub build_time: f64,
    pub build_cost: BTreeMap<String, f64>,
    pub supply_consumption: f64,
    pub supply_cost: BTreeMap<String, f64>,
    pub reconnaissance: f64,
    pub attack: f64,
    pub defence: f64,
    pub discipline: f64,
    pub support: f64,
    pub maneuver: f64,
    pub siege: f64,
    pub hull: f64,
    pub gun_power: f64,
    pub fire_range: f64,
    pub evasion: f64,
    pub colonial_points: f64,
    pub supply_consumption_score: f64,
    pub min_port_level: f64,
    /// `limit_per_port`, negative or absent = no limit.
    pub limit_per_port: Option<f64>,
    pub can_build_overseas: Option<bool>,
    pub primary_culture_only: bool,
}

fn goods(b: &Block, k: &str) -> BTreeMap<String, f64> {
    b.get(k).and_then(Value::as_block).map(|g| g.entries().filter_map(|(n, v, _)| Some((n.to_string(), v.as_f64()?))).collect()).unwrap_or_default()
}

impl UnitDef {
    pub fn from_block(name: &str, b: &Block) -> UnitDef {
        let yes = |k: &str| b.get(k).and_then(Value::as_bool);
        UnitDef {
            name: name.to_string(),
            land: text(b, "type").as_deref() == Some("land"),
            kind: text(b, "unit_type").unwrap_or_default(),
            capital: yes("capital").unwrap_or(false),
            transport: yes("transport").unwrap_or(false),
            sail: yes("sail").unwrap_or(false),
            active: yes("active").unwrap_or(true),
            priority: num(b, "priority"),
            max_strength: num(b, "max_strength"),
            organisation: num(b, "default_organisation"),
            speed: num(b, "maximum_speed"),
            weighted_value: num(b, "weighted_value"),
            build_time: num(b, "build_time"),
            build_cost: goods(b, "build_cost"),
            supply_consumption: num(b, "supply_consumption"),
            supply_cost: goods(b, "supply_cost"),
            reconnaissance: num(b, "reconnaissance"),
            attack: num(b, "attack"),
            defence: num(b, "defence"),
            discipline: num(b, "discipline"),
            support: num(b, "support"),
            maneuver: num(b, "maneuver"),
            siege: num(b, "siege"),
            hull: num(b, "hull"),
            gun_power: num(b, "gun_power"),
            fire_range: num(b, "fire_range"),
            evasion: num(b, "evasion"),
            colonial_points: num(b, "colonial_points"),
            supply_consumption_score: num(b, "supply_consumption_score"),
            min_port_level: num(b, "min_port_level"),
            limit_per_port: b.get("limit_per_port").and_then(Value::as_f64).filter(|x| *x >= 0.0),
            can_build_overseas: yes("can_build_overseas"),
            primary_culture_only: yes("primary_culture").unwrap_or(false),
        }
    }
}

impl UnitDef {
    /// Adds what a technology or invention gives to one statistic, under the name the unit files use.
    pub fn add(&mut self, stat: &str, v: f64) {
        let field = match stat {
            "max_strength" => &mut self.max_strength,
            "default_organisation" => &mut self.organisation,
            "maximum_speed" => &mut self.speed,
            "supply_consumption" => &mut self.supply_consumption,
            "reconnaissance" => &mut self.reconnaissance,
            "attack" => &mut self.attack,
            "defence" => &mut self.defence,
            "discipline" => &mut self.discipline,
            "support" => &mut self.support,
            "maneuver" => &mut self.maneuver,
            "siege" => &mut self.siege,
            "hull" => &mut self.hull,
            "gun_power" => &mut self.gun_power,
            "fire_range" => &mut self.fire_range,
            "evasion" => &mut self.evasion,
            _ => return,
        };
        *field += v;
    }
}

/// The blocks of a technology or an invention effect that add to every land or every sea unit type.
pub const ARMY_BASE: &str = "army_base";
pub const NAVY_BASE: &str = "navy_base";

/// Unit type (or `army_base` / `navy_base`) -> statistic -> what one technology or invention adds.
pub type UnitBonus = BTreeMap<String, BTreeMap<String, f64>>;

/// The unit blocks of a technology, or of an invention's effect.
pub fn unit_bonus(b: &Block, units: &BTreeMap<String, UnitDef>) -> UnitBonus {
    b.entries()
        .filter(|(k, _, _)| *k == ARMY_BASE || *k == NAVY_BASE || units.contains_key(*k))
        .filter_map(|(k, v, _)| Some((k.to_string(), v.as_block()?.entries().filter_map(|(s, x, _)| Some((s.to_string(), x.as_f64()?))).collect())))
        .collect()
}

/// Every `*.txt` in the units folder of the install, one unit type per top-level block, in file
/// name order.
pub fn read_unit_defs(dir: &Path) -> Result<Vec<UnitDef>, String> {
    let mut files: Vec<_> = std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "txt")).collect();
    files.sort();
    let mut out = Vec::new();
    for file in files {
        let bytes = std::fs::read(&file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
        let doc = script::parse_bytes(&bytes).map_err(|e| format!("{}: {e}", file.display()))?;
        out.extend(doc.root.entries().filter_map(|(name, v, _)| Some(UnitDef::from_block(name, v.as_block()?))));
    }
    Ok(out)
}

/// One leader trait of the install (`common/traits.txt`), or the sum of a leader's two.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TraitDef {
    /// Added to the dice of the attacking side (whole part).
    pub attack: f64,
    /// Added to the dice of the defending side (whole part).
    pub defence: f64,
    pub morale: f64,
    pub organisation: f64,
    pub reconnaissance: f64,
    pub speed: f64,
    pub attrition: f64,
    pub experience: f64,
    pub reliability: f64,
}

impl TraitDef {
    pub fn from_block(b: &Block) -> TraitDef {
        TraitDef {
            attack: num(b, "attack"),
            defence: num(b, "defence"),
            morale: num(b, "morale"),
            organisation: num(b, "organisation"),
            reconnaissance: num(b, "reconnaissance"),
            speed: num(b, "speed"),
            attrition: num(b, "attrition"),
            experience: num(b, "experience"),
            reliability: num(b, "reliability"),
        }
    }

    pub fn plus(self, o: TraitDef) -> TraitDef {
        TraitDef {
            attack: self.attack + o.attack,
            defence: self.defence + o.defence,
            morale: self.morale + o.morale,
            organisation: self.organisation + o.organisation,
            reconnaissance: self.reconnaissance + o.reconnaissance,
            speed: self.speed + o.speed,
            attrition: self.attrition + o.attrition,
            experience: self.experience + o.experience,
            reliability: self.reliability + o.reliability,
        }
    }
}

/// Every trait of a traits file by name: the entries of its top-level blocks (personality, background).
pub fn parse_traits(src: &[u8]) -> Result<BTreeMap<String, TraitDef>, String> {
    let doc = script::parse_bytes(src).map_err(|e| e.to_string())?;
    Ok(doc.root.entries().filter_map(|(_, v, _)| v.as_block()).flat_map(|group| group.entries().filter_map(|(name, v, _)| Some((name.to_string(), TraitDef::from_block(v.as_block()?))))).collect())
}

/// A land regiment, from a soldier pop (or a pop of any other type while mobilised).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Regiment {
    pub id: u64,
    pub name: String,
    /// Unit type name.
    pub kind: String,
    /// Id of the pop behind it and, once looked up in the save's provinces, that pop's type.
    pub pop: Option<u64>,
    pub pop_type: Option<String>,
    pub organisation: f64,
    /// 1.0 = 1000 men at full size (`MilitaryRules::men_per_strength`).
    pub strength: f64,
    pub experience: f64,
    /// `count` in the save: the number in the unit's name, as far as the names show (inferred).
    pub name_count: u32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Ship {
    pub id: u64,
    pub name: String,
    pub kind: String,
    pub organisation: f64,
    pub strength: f64,
    pub experience: f64,
    pub name_count: u32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Army {
    pub id: u64,
    pub name: String,
    pub owner: String,
    pub leader: Option<u64>,
    pub location: u16,
    pub previous: Option<u16>,
    /// Distance travelled towards the next province on the path.
    pub movement_progress: f64,
    /// Provinces still to enter, in order.
    pub path: Vec<u16>,
    /// Where the army is headed when it travels by sea (`target`, seen on carried armies).
    pub target: Option<u16>,
    /// Home base province (`base`).
    pub base: Option<u16>,
    pub dig_in: f64,
    pub dig_in_date: Option<Date>,
    /// 0..1.
    pub supplies: f64,
    pub regiments: Vec<Regiment>,
    /// The fleet carrying it.
    pub carried_by: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Navy {
    pub id: u64,
    pub name: String,
    pub owner: String,
    pub leader: Option<u64>,
    pub location: u16,
    pub previous: Option<u16>,
    pub movement_progress: f64,
    pub path: Vec<u16>,
    pub supplies: f64,
    pub ships: Vec<Ship>,
    /// Ids of the armies on board (they are in `MilitaryState::armies`).
    pub carried: Vec<u64>,
    /// `at_sea` as written (a number, 0 in port; its exact meaning is not settled).
    pub at_sea: f64,
    /// Days spent out of supply range.
    pub no_supply_days: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LeaderKind {
    #[default]
    Land,
    Sea,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Leader {
    pub id: u64,
    pub name: String,
    pub owner: String,
    pub kind: LeaderKind,
    pub personality: String,
    pub background: String,
    pub picture: String,
    pub prestige: f64,
    /// The day the leader was created (`date`): the age of death rolls counts from here.
    pub created: Option<Date>,
}

/// A regiment or ship being built (`military_construction` in a province).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Construction {
    pub id: u64,
    pub owner: String,
    pub location: u16,
    pub start: Option<Date>,
    pub end: Option<Date>,
    pub name: String,
    pub kind: String,
    /// Id of the unit it will become and, for a regiment, of the pop it is raised from.
    pub unit: Option<u64>,
    pub pop: Option<u64>,
    /// Index of the rally point it marches to, -1 = none.
    pub rally_point: i64,
    /// Goods it still lacks (`goods_demand`) and the whole list it was ordered with (the inner
    /// `input_goods`: the build cost times 2 - administrative efficiency). Both empty once the
    /// goods are in: the save then writes no `input_goods` block.
    pub lacks: BTreeMap<String, f64>,
    pub full: BTreeMap<String, f64>,
}

fn provinces(b: &Block, k: &str) -> Vec<u16> {
    b.get(k).and_then(Value::as_block).map(|p| p.bare_scalars().iter().filter_map(|s| s.parse().ok()).collect()).unwrap_or_default()
}

fn province(b: &Block, k: &str) -> Option<u16> {
    b.get(k).and_then(Value::as_f64).map(|x| x as u16)
}

pub fn read_regiment(b: &Block, pops: &BTreeMap<u64, String>) -> Regiment {
    let pop = id_of(b, "pop");
    Regiment {
        id: id_of(b, "id").unwrap_or(0),
        name: text(b, "name").unwrap_or_default(),
        kind: text(b, "type").unwrap_or_default(),
        pop,
        pop_type: pop.and_then(|p| pops.get(&p).cloned()),
        organisation: num(b, "organisation"),
        strength: num(b, "strength"),
        experience: num(b, "experience"),
        name_count: num(b, "count") as u32,
    }
}

pub fn read_ship(b: &Block) -> Ship {
    Ship { id: id_of(b, "id").unwrap_or(0), name: text(b, "name").unwrap_or_default(), kind: text(b, "type").unwrap_or_default(), organisation: num(b, "organisation"), strength: num(b, "strength"), experience: num(b, "experience"), name_count: num(b, "count") as u32 }
}

/// `None` when the block has no id record.
pub fn read_army(b: &Block, owner: &str, pops: &BTreeMap<u64, String>, carried_by: Option<u64>) -> Option<Army> {
    Some(Army {
        id: id_of(b, "id")?,
        name: text(b, "name").unwrap_or_default(),
        owner: owner.to_string(),
        leader: id_of(b, "leader"),
        location: province(b, "location").unwrap_or(0),
        previous: province(b, "previous"),
        movement_progress: num(b, "movement_progress"),
        path: provinces(b, "path"),
        target: province(b, "target"),
        base: province(b, "base"),
        dig_in: num(b, "dig_in"),
        dig_in_date: date(b, "dig_in_last_date"),
        supplies: num(b, "supplies"),
        regiments: b.get_all("regiment").filter_map(Value::as_block).map(|r| read_regiment(r, pops)).collect(),
        carried_by,
    })
}

/// The fleet itself; the armies inside it are read by the caller.
pub fn read_navy(b: &Block, owner: &str) -> Option<Navy> {
    Some(Navy {
        id: id_of(b, "id")?,
        name: text(b, "name").unwrap_or_default(),
        owner: owner.to_string(),
        leader: id_of(b, "leader"),
        location: province(b, "location").unwrap_or(0),
        previous: province(b, "previous"),
        movement_progress: num(b, "movement_progress"),
        path: provinces(b, "path"),
        supplies: num(b, "supplies"),
        ships: b.get_all("ship").filter_map(Value::as_block).map(read_ship).collect(),
        carried: Vec::new(),
        at_sea: num(b, "at_sea"),
        no_supply_days: num(b, "no_supply_days"),
    })
}

pub fn read_leader(b: &Block, owner: &str) -> Option<Leader> {
    Some(Leader {
        id: id_of(b, "id")?,
        name: text(b, "name").unwrap_or_default(),
        owner: owner.to_string(),
        kind: if text(b, "type").as_deref() == Some("sea") { LeaderKind::Sea } else { LeaderKind::Land },
        personality: text(b, "personality").unwrap_or_default(),
        background: text(b, "background").unwrap_or_default(),
        picture: text(b, "picture").unwrap_or_default(),
        prestige: num(b, "prestige"),
        created: date(b, "date"),
    })
}

pub fn read_construction(b: &Block) -> Option<Construction> {
    Some(Construction {
        id: id_of(b, "id")?,
        owner: text(b, "country").unwrap_or_default(),
        location: province(b, "location").unwrap_or(0),
        start: date(b, "start_date"),
        end: date(b, "date"),
        name: text(b, "name").unwrap_or_default(),
        kind: text(b, "type").unwrap_or_default(),
        unit: id_of(b, "unit"),
        pop: id_of(b, "pop"),
        rally_point: b.get("rally_point").and_then(Value::as_f64).map(|x| x as i64).unwrap_or(-1),
        lacks: b.get("input_goods").and_then(Value::as_block).map(|g| goods(g, "goods_demand")).unwrap_or_default(),
        full: b.get("input_goods").and_then(Value::as_block).map(|g| goods(g, "input_goods")).unwrap_or_default(),
    })
}

impl Army {
    /// Men in the army: the sum of strength over its regiments times `men_per_strength`.
    pub fn men(&self, men_per_strength: f64) -> f64 {
        self.regiments.iter().map(|r| r.strength).sum::<f64>() * men_per_strength
    }
}
