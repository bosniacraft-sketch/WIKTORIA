//! The military day step: the pure rule modules of this folder connected in the order of
//! (war score and ticking goals, each unit's day, movement, arrivals
//! into battles and sieges, battles, sieges, then the monthly step on the 1st).
//!
//! `MilitaryDay::day` mutates only `MilitaryState` and its own seeded generators. `State` is read
//! only. What would change in `State` (pops that lost men, provinces that changed controller) comes
//! back in the `DayReport` for the caller to apply later. There is no AI: an army only follows a path
//! it already has (a retreat or an order loaded from the save).
//!
//! Skipped on purpose (reported by the CLI): supply goods (the supply status is the rules file
//! stand-in `FLD_MIL_DAY_SUPPLY_STATUS`), leader traits, leader creation and death, exile,
//! mobilisation, construction, regiment/pop defections, the "military access" relation, and the
//! economic effects of occupation (the caller reads `ProvinceMilitary::rgo_workers_leave`).
//!
//! Crossing water (an army path with sea provinces, `movement::army_path`): the army waits on the
//! coast for a fleet of its country with room for all its regiments (transport ships times
//! `MovementRules::regiments_per_transport`) in the sea province or in port on its coast. When none
//! is there the nearest idle fleet with room is called to the sea province (a PORT CHOICE: who sends
//! the fleet is not observed); with none at all the order is cancelled. Boarding and landing are
//! steps between land and sea at `FLD_MIL_MOVE_SEA_LAND_SPEED` over `FLD_MIL_MOVE_SEA_LAND_DISTANCE`.
//! On board the army's path keeps only the land after the sea leg (`Army::target` the end), the fleet
//! sails the sea leg; once it stands next to the next land province the army lands there if it may
//! enter it, otherwise the order is cancelled and the army stays aboard.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use wiktoria_data::date::Date;
use wiktoria_data::map::ProvinceId;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::script::{self, Value};
use wiktoria_data::world::World;

use super::battle::{self, BattleRng, BattleSide, Combat, CombatKind, Entry, Fighter, JoinUnit, LandContext, NavalShip, NavalState, ShipClass, SideInfo};
use super::logistics::{self, LogisticsRules};
use super::movement::{self, Arrival, BorderContext, BorderKind, CombatView, MoveMap, MoveProvince, MovementRules, Mover, Relations, Standing};
use super::siege::{self, Besieger, ProvinceMilitary, SiegeContext, SiegeEvent, SiegeRegiment, SiegeRng, SiegeRules};
use super::units::{self, Army, Leader, Navy, TraitDef, UnitBonus, UnitDef};
use super::war::{self, CbType, WarRules};
use super::{MilitaryRules, MilitaryState};
use crate::calendar::Calendar;
use crate::script::State;

/// Terrain facts of one province the day step needs (from the install's terrain and climate files).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Terrain {
    /// Penalty to the attacker's dice.
    pub defence: f64,
    /// Battle width is multiplied by `1 + this`.
    pub width: f64,
    /// Supply limit modifier of the province.
    pub supply_limit: f64,
    /// `max_attrition` of the province (`logistics::province_max_attrition`).
    pub max_attrition: f64,
}

/// What the battle round reads from the install besides the unit files.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BattleContent {
    /// Leader traits by name (`common/traits.txt`).
    pub traits: BTreeMap<String, TraitDef>,
    /// Personality and background of the stand-in leader of an army without one.
    pub no_leader: [String; 2],
    /// Technology name -> what it adds to unit types.
    pub tech_units: BTreeMap<String, UnitBonus>,
    /// What each invention of the install adds to unit types, and its numeric effects
    /// (`military_tactics`, `combat_width`, `dig_in_cap`...), in the order a save numbers them from 1.
    pub invention_units: Vec<UnitBonus>,
    pub invention_values: Vec<BTreeMap<String, f64>>,
    /// GAS_ATTACK_MODIFIER of the install's defines.
    pub gas_attack_modifier: f64,
    /// (gas attack, gas defence) each invention of the install gives, in the order a save numbers them from 1.
    pub invention_gas: Vec<(bool, bool)>,
    /// Countries taken to have gas attack and gas defence whatever their inventions say: for saves
    /// made with another inventions folder than the install's (`check-battle --gas TAG`).
    pub gas_override: BTreeSet<String>,
}

impl BattleContent {
    /// The summed traits of a leader; of the stand-in when there is none.
    pub fn leader_traits(&self, l: Option<&Leader>) -> TraitDef {
        let names = l.map_or([self.no_leader[0].as_str(), self.no_leader[1].as_str()], |l| [l.personality.as_str(), l.background.as_str()]);
        names.iter().filter_map(|n| self.traits.get(*n)).fold(TraitDef::default(), |a, t| a.plus(*t))
    }

    /// (gas attack, gas defence) of a country with these active inventions.
    pub fn gas(&self, tag: &str, inventions: &BTreeSet<u32>) -> (bool, bool) {
        if self.gas_override.contains(tag) {
            return (true, true);
        }
        let of = |n: &u32| n.checked_sub(1).and_then(|i| self.invention_gas.get(i as usize)).copied().unwrap_or_default();
        inventions.iter().map(of).fold((false, false), |a, g| (a.0 || g.0, a.1 || g.1))
    }
}

/// Everything `MilitaryDay` is made of, so a test can build one without a whole `World`.
pub struct DayParts {
    pub logistics: LogisticsRules,
    pub movement: MovementRules,
    pub battle: battle::BattleRules,
    pub siege: SiegeRules,
    pub war: WarRules,
    pub calendar: Calendar,
    pub map: MoveMap,
    pub units: BTreeMap<String, UnitDef>,
    pub cb_types: BTreeMap<String, CbType>,
    pub rebel_tag: String,
    pub men_per_strength: f64,
    /// Rules file stand-in: supply status of every non-rebel unit (the goods stockpile is not modelled here).
    pub supply_status: f64,
    pub dig_in_days: f64,
    pub soldier_type: String,
    /// Pop type -> (leadership value, optimum share).
    pub leadership: BTreeMap<String, (f64, f64)>,
    pub terrain: BTreeMap<u16, Terrain>,
    /// Technology name -> its numeric values (`morale`, `military_tactics`, `combat_width`, `dig_in_cap`...).
    pub techs: BTreeMap<String, BTreeMap<String, f64>>,
    /// Name and per-level value of the infrastructure building (the railway).
    pub infrastructure: Option<(String, f64)>,
    pub content: BattleContent,
    /// `supply_limit` of the static block of every land province, and of the static infrastructure
    /// block (taken times the province's infrastructure).
    pub supply_land: f64,
    pub supply_infrastructure: f64,
    pub seed: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControllerChange {
    pub province: u16,
    pub from: Option<String>,
    pub to: Option<String>,
}

/// What one day did, for the caller to apply to `State` and to show.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DayReport {
    pub date: Option<Date>,
    /// (pop id, men lost) from battles and attrition today, merged per pop.
    pub casualties: Vec<(u64, f64)>,
    pub controller_changes: Vec<ControllerChange>,
    pub battles_started: usize,
    pub battles_finished: usize,
    pub sieges_started: usize,
    pub sieges_ended: usize,
    pub armies_destroyed: usize,
    pub fleets_destroyed: usize,
    /// Prestige a battle moved, per country (the caller floors the country's prestige at 0).
    pub prestige: Vec<(String, f64)>,
    /// (war name, score of the attackers' leader against the defenders' leader).
    pub war_scores: Vec<(String, f64)>,
    pub monthly: bool,
    /// Pops behind the regiments of rebel armies beaten in a land battle today, once per battle
    /// (their militancy is divided by REDUCTION_AFTER_DEFEAT).
    pub rebels_beaten: Vec<u64>,
}

pub struct MilitaryDay {
    pub logistics: LogisticsRules,
    pub movement: MovementRules,
    pub battle: battle::BattleRules,
    pub siege: SiegeRules,
    pub war: WarRules,
    pub calendar: Calendar,
    pub map: MoveMap,
    pub units: BTreeMap<String, UnitDef>,
    pub cb_types: BTreeMap<String, CbType>,
    pub rebel_tag: String,
    pub men_per_strength: f64,
    pub supply_status: f64,
    pub dig_in_days: f64,
    pub soldier_type: String,
    pub leadership: BTreeMap<String, (f64, f64)>,
    pub terrain: BTreeMap<u16, Terrain>,
    pub techs: BTreeMap<String, BTreeMap<String, f64>>,
    pub infrastructure: Option<(String, f64)>,
    pub content: BattleContent,
    pub supply_land: f64,
    pub supply_infrastructure: f64,
    pub battle_rng: BattleRng,
    pub siege_rng: SiegeRng,
    locks: BTreeMap<u64, f64>,
    retreating: BTreeSet<u64>,
    naval: BTreeMap<u16, NavalState>,
    naval_months: BTreeMap<u64, f64>,
    /// Strength of every land regiment at the end of the day before (regiment id). The original
    /// books a day's siege fall on yesterday's men (MIL_SIEGE_003). Ticket #242:
    ///
    /// * every army is kept, not only those already besieging, so an army whose siege starts
    ///   today (it arrived, or its battle in the province ended) is booked on its men of yesterday
    ///   evening as well;
    /// * on the first day after a load (or of a new game) the snapshot is seeded, before anything
    ///   moves or fights, from the strengths the save holds: a save is written at the end of a day,
    ///   so those are yesterday's men. Nothing new has to go into the save for it.
    ///
    /// Only a regiment that did not exist yesterday evening (raised today) counts with today's
    /// strength.
    siege_men: BTreeMap<u64, f64>,
    synced: bool,
    /// Fleets called to carry an army over water: fleet id -> army id.
    calls: BTreeMap<u64, u64>,
}

fn define(world: &World, k: &str) -> Result<f64, String> {
    let d = &world.defines;
    d.sections().iter().find_map(|(s, _)| d.get(s, k)).ok_or(format!("defines.{k} missing"))
}

/// The casus belli types of the install (`common/cb_types.txt`), as far as the war rules need them.
pub fn load_cb_types(src: &[u8]) -> Result<BTreeMap<String, CbType>, String> {
    let doc = script::parse_bytes(src).map_err(|e| format!("cb_types: {e}"))?;
    let mut out = BTreeMap::new();
    for (name, v, _) in doc.root.entries() {
        let Some(b) = v.as_block() else { continue };
        // `peace_order` is the execution order list, not a type.
        if name == "peace_order" {
            continue;
        }
        let yes = |k: &str| b.get(k).and_then(Value::as_bool).unwrap_or(false);
        let num = |k: &str, d: f64| b.get(k).and_then(Value::as_f64).unwrap_or(d);
        let options: Vec<String> = b.entries().filter_map(|(k, v, _)| { let o = k.strip_prefix("po_")?; (v.as_bool() == Some(true)).then(|| if o == "remove_prestige" { "prestige".to_string() } else { o.to_string() }) }).collect();
        out.insert(
            name.to_string(),
            CbType {
                name: name.to_string(),
                options,
                badboy_factor: num("badboy_factor", 1.0),
                prestige_factor: num("prestige_factor", 1.0),
                peace_cost_factor: num("peace_cost_factor", 1.0),
                penalty_factor: num("penalty_factor", 1.0),
                break_truce_infamy_factor: num("break_truce_infamy_factor", 1.0),
                break_truce_prestige_factor: num("break_truce_prestige_factor", 1.0),
                break_truce_militancy_factor: num("break_truce_militancy_factor", 1.0),
                truce_months: num("truce_months", 0.0),
                construction_speed: num("construction_speed", 1.0),
                tws_battle_factor: num("tws_battle_factor", 0.0),
                needs_no_justification: yes("always"),
                zero_cost: yes("great_war_obligatory"),
                // The AI's picker: only the constructing_cb flag gates justifying.
                // Every stock type carries is_triggered_only = yes, so it cannot be the gate.
                constructing: b.get("constructing_cb").and_then(Value::as_bool).unwrap_or(true),
            },
        );
    }
    Ok(out)
}

impl DayParts {
    /// Everything from the world, the rules file and the install.
    pub fn from_world(world: &World, engine_rules: &EngineRules, install: &Path, rules: &MilitaryRules, seed: u64) -> Result<DayParts, String> {
        let scale = crate::num_field(engine_rules, "FLD_MIL_DAY_DISTANCE_SCALE")?;
        let centroid = |p: ProvinceId| world.map.centroid.get(p as usize).copied().unwrap_or((0.0, 0.0));
        let distance = |a: ProvinceId, b: ProvinceId| {
            let (pa, pb) = (centroid(a), centroid(b));
            (f64::from(pa.0 - pb.0).hypot(f64::from(pa.1 - pb.1)) * scale).max(1.0)
        };
        let mut map = MoveMap::from_adjacency(&world.adjacency, &world.extra_adjacencies, &distance);
        // A border is a river border when at least this many river pixels lie on it; sea borders are not.
        let river_min = crate::num_field(engine_rules, "FLD_MAP_RIVER_BORDER_MIN_PIXELS")?;
        let land = |p: ProvinceId| world.province(p).is_some_and(|x| !x.is_sea);
        map.set_rivers(world.river_borders.iter().filter(|((a, b), n)| f64::from(**n) >= river_min && land(*a) && land(*b)).map(|(k, _)| *k));
        let mut terrain = BTreeMap::new();
        let statics = |block: &str, key: &str| world.defs.modifiers.static_modifiers.get(block).and_then(|v| v.get(key)).copied().unwrap_or(0.0);
        let land_block = engine_rules.require_str("FLD_PMOD_LAND_BLOCK")?;
        let group = |g: &wiktoria_data::geography::ProvinceGroups, name: &Option<String>| name.as_ref().and_then(|c| g.modifiers.get(c)).and_then(|m| m.get("max_attrition")).copied().unwrap_or(0.0);
        for p in world.provinces.iter().flatten() {
            let cat = p.terrain.as_ref().and_then(|t| world.defs.terrain.categories.get(t));
            let v = |k: &str| cat.and_then(|c| c.values.get(k)).copied().unwrap_or(0.0);
            // NOT WIRED: the crime and the timed modifiers of the province (they change during play).
            let max_attrition = if p.is_sea { 0.0 } else { logistics::province_max_attrition(statics(land_block, "max_attrition"), v("max_attrition"), group(&world.defs.climates, &p.climate), group(&world.defs.continents, &p.continent)) };
            terrain.insert(p.id, Terrain { defence: v("defence"), width: v("combat_width"), supply_limit: v("supply_limit"), max_attrition });
            // The movement cost of the terrain is read as the multiplier itself: plains 1.0 add nothing.
            let port_sea = (!p.is_sea).then(|| world.adjacency.get(p.id as usize).and_then(|n| n.iter().copied().find(|q| world.province(*q).is_some_and(|x| x.is_sea)))).flatten();
            map.provinces.insert(p.id, MoveProvince { sea: p.is_sea, infrastructure: 0.0, cost_modifier: if p.is_sea { 0.0 } else { v("movement_cost") - 1.0 }, port_sea });
        }
        let soldier_type = engine_rules.require_str("FLD_POP_TYPE_SOLDIERS")?.to_string();
        let mut leadership = BTreeMap::new();
        let dir = install.join(engine_rules.require_str("FLD_POPTYPES_DIR")?);
        for e in std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("txt") {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            let doc = script::parse_bytes(&std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?).map_err(|e| format!("{}: {e}", path.display()))?;
            let l = doc.root.get("leadership").and_then(Value::as_f64).unwrap_or(0.0);
            if l != 0.0 {
                leadership.insert(name.to_string(), (l, doc.root.get("research_optimum").and_then(Value::as_f64).unwrap_or(0.0)));
            }
        }
        let cb_file = install.join(engine_rules.require_str("FLD_MIL_DAY_CB_TYPES_FILE")?);
        let cb_types = load_cb_types(&std::fs::read(&cb_file).map_err(|e| format!("{}: {e}", cb_file.display()))?)?;
        let mut tech_units = BTreeMap::new();
        let tech_dir = install.join(engine_rules.require_str("FLD_TECHNOLOGY_DIR")?);
        for e in std::fs::read_dir(&tech_dir).map_err(|e| format!("{}: {e}", tech_dir.display()))?.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("txt") {
                continue;
            }
            let doc = script::parse_bytes(&std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?).map_err(|e| format!("{}: {e}", path.display()))?;
            tech_units.extend(doc.root.entries().filter_map(|(name, v, _)| Some((name.to_string(), units::unit_bonus(v.as_block()?, &rules.units)))).filter(|(_, b)| !b.is_empty()));
        }
        let effects: Vec<Option<&script::Block>> = world.defs.invention_blocks.iter().map(|b| b.get("effect").and_then(Value::as_block)).collect();
        let traits_file = install.join(engine_rules.require_str("FLD_MIL_BAT_TRAITS_FILE")?);
        let content = BattleContent {
            traits: units::parse_traits(&std::fs::read(&traits_file).map_err(|e| format!("{}: {e}", traits_file.display()))?).map_err(|e| format!("{}: {e}", traits_file.display()))?,
            no_leader: [engine_rules.require_str("FLD_MIL_BAT_NO_LEADER_PERSONALITY")?.to_string(), engine_rules.require_str("FLD_MIL_BAT_NO_LEADER_BACKGROUND")?.to_string()],
            tech_units,
            invention_units: effects.iter().map(|e| e.map(|e| units::unit_bonus(e, &rules.units)).unwrap_or_default()).collect(),
            invention_values: effects.iter().map(|e| e.map(|e| e.entries().filter_map(|(k, v, _)| Some((k.to_string(), v.as_f64()?))).collect()).unwrap_or_default()).collect(),
            gas_attack_modifier: define(world, "GAS_ATTACK_MODIFIER")?,
            invention_gas: world
                .defs
                .invention_blocks
                .iter()
                .map(|b| {
                    let yes = |k: &str| b.get("effect").and_then(Value::as_block).and_then(|e| e.get(k)).and_then(Value::as_bool).unwrap_or(false);
                    (yes("gas_attack"), yes("gas_defence"))
                })
                .collect(),
            gas_override: BTreeSet::new(),
        };
        Ok(DayParts {
            logistics: LogisticsRules::new(engine_rules, &world.defines)?,
            movement: MovementRules::new(&world.defines, engine_rules)?,
            battle: battle::BattleRules::from_engine_rules(engine_rules)?,
            siege: SiegeRules::from_engine_rules(engine_rules, &world.defines)?,
            war: WarRules::from_engine_rules(&world.defines, engine_rules)?,
            calendar: Calendar::from_engine_rules(engine_rules)?,
            map,
            units: rules.units.clone(),
            cb_types,
            rebel_tag: rules.rebel_tag.clone(),
            men_per_strength: rules.men_per_strength,
            supply_status: crate::num_field(engine_rules, "FLD_MIL_DAY_SUPPLY_STATUS")?,
            dig_in_days: define(world, "DIG_IN_INCREASE_EACH_DAYS")?,
            soldier_type,
            leadership,
            terrain,
            techs: world.defs.technologies.iter().map(|t| (t.name.clone(), t.values.clone())).collect(),
            infrastructure: world.defs.buildings.iter().find(|b| b.kind == "infrastructure").map(|b| (b.name.clone(), b.values.get("infrastructure").copied().unwrap_or(0.0))),
            content,
            supply_land: statics(land_block, "supply_limit"),
            supply_infrastructure: statics(engine_rules.require_str("FLD_PMOD_INFRASTRUCTURE_BLOCK")?.split(':').next().unwrap_or_default().trim(), "supply_limit"),
            seed,
        })
    }
}

fn is_sea(map: &MoveMap, p: u16) -> bool {
    map.provinces.get(&p).is_some_and(|m| m.sea)
}

fn side_countries(mil: &MilitaryState, c: &Combat, side: BattleSide) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in &c.side(side).units {
        let owner = if c.kind == CombatKind::Naval { mil.navies.iter().find(|n| n.id == *id).map(|n| n.owner.clone()) } else { mil.armies.iter().find(|a| a.id == *id).map(|a| a.owner.clone()) };
        if let Some(o) = owner && !out.contains(&o) {
            out.push(o);
        }
    }
    out
}

/// A battle taken up from a save: regiments of its armies that stand in no line and are not in the
/// save's reserve go to the end of the reserve.
fn fill_reserve(c: &mut Combat, armies: &[Army]) {
    for side in [BattleSide::Attacker, BattleSide::Defender] {
        let s = c.side(side);
        let mut seen: BTreeSet<u64> = s.front.values().chain(s.back.values()).chain(&s.reserve).chain(&s.broken).copied().collect();
        let mut reserve = Vec::new();
        for army in armies.iter().filter(|a| s.units.contains(&a.id)) {
            for r in &army.regiments {
                if seen.insert(r.id) {
                    reserve.push(r.id);
                }
            }
        }
        c.side_mut(side).reserve.extend(reserve);
    }
}

fn army_strength(a: &Army) -> f64 {
    a.regiments.iter().map(|r| r.strength).sum()
}

impl MilitaryDay {
    pub fn from_world(world: &World, engine_rules: &EngineRules, install: &Path, rules: &MilitaryRules, seed: u64) -> Result<MilitaryDay, String> {
        Ok(MilitaryDay::from_parts(DayParts::from_world(world, engine_rules, install, rules, seed)?))
    }

    pub fn from_parts(p: DayParts) -> MilitaryDay {
        MilitaryDay {
            logistics: p.logistics,
            movement: p.movement,
            battle: p.battle,
            siege: p.siege,
            war: p.war,
            calendar: p.calendar,
            map: p.map,
            units: p.units,
            cb_types: p.cb_types,
            rebel_tag: p.rebel_tag,
            men_per_strength: p.men_per_strength,
            supply_status: p.supply_status,
            dig_in_days: p.dig_in_days,
            soldier_type: p.soldier_type,
            leadership: p.leadership,
            terrain: p.terrain,
            techs: p.techs,
            infrastructure: p.infrastructure,
            content: p.content,
            supply_land: p.supply_land,
            supply_infrastructure: p.supply_infrastructure,
            battle_rng: BattleRng(p.seed),
            siege_rng: SiegeRng(p.seed ^ 0x5DEE_CE66_D1CE_4E5B),
            locks: BTreeMap::new(),
            retreating: BTreeSet::new(),
            naval: BTreeMap::new(),
            naval_months: BTreeMap::new(),
            siege_men: BTreeMap::new(),
            synced: false,
            calls: BTreeMap::new(),
        }
    }

    // ------------------------------------------------------------------ small helpers

    fn day_number(&self, d: Date) -> i64 {
        self.calendar.to_game(d).map_or(0, |g| g.0.div_euclid(24))
    }

    fn tech_sum(&self, state: &State, tag: &str, key: &str) -> f64 {
        state.countries.get(tag).map_or(0.0, |c| c.technologies.iter().filter_map(|t| self.techs.get(t)?.get(key)).sum())
    }

    /// `tech_sum` plus the same effect of the country's inventions.
    fn country_sum(&self, state: &State, tag: &str, key: &str) -> f64 {
        let inventions: f64 = state.countries.get(tag).map_or(0.0, |c| c.inventions.iter().filter_map(|n| self.content.invention_values.get((*n as usize).checked_sub(1)?)?.get(key)).sum());
        self.tech_sum(state, tag, key) + inventions
    }

    /// What a country's technologies and inventions add up to for one effect (`combat_width`,
    /// `dig_in_cap`, `military_tactics`, `morale`...), as the day's rules read it. Read-only.
    pub fn country_value(&self, state: &State, tag: &str, key: &str) -> f64 {
        self.country_sum(state, tag, key)
    }

    /// A unit type as a country fields it: the unit file's values plus what the country's
    /// technologies and inventions add to the type and to every land (or sea) type.
    pub fn unit_of(&self, state: &State, tag: &str, kind: &str) -> UnitDef {
        let mut def = self.unit(kind);
        let Some(c) = state.countries.get(tag) else { return def };
        let base = if def.land { units::ARMY_BASE } else { units::NAVY_BASE };
        let from_techs = c.technologies.iter().filter_map(|t| self.content.tech_units.get(t));
        let from_inventions = c.inventions.iter().filter_map(|n| self.content.invention_units.get((*n as usize).checked_sub(1)?));
        for bonus in from_techs.chain(from_inventions) {
            for stats in [base, kind].iter().filter_map(|k| bonus.get(*k)) {
                for (stat, v) in stats {
                    def.add(stat, *v);
                }
            }
        }
        def
    }

    /// Supply limit of a province for a country: terrain, static blocks and railway of the province,
    /// `supply_limit` of the country's technologies (`logistics::supply_limit`).
    pub fn supply_limit(&self, state: &State, tag: &str, province: u16, access: logistics::SupplyAccess) -> i64 {
        let terrain = self.terrain.get(&province).map_or(0.0, |t| t.supply_limit);
        let modifier = match self.map.provinces.get(&province) {
            Some(m) if m.sea => terrain,
            m => logistics::province_supply_modifier(terrain, self.supply_land, self.supply_infrastructure, m.map_or(0.0, |m| m.infrastructure)),
        };
        logistics::supply_limit(&self.logistics, modifier, access, self.tech_sum(state, tag, "supply_limit"))
    }

    /// How a country stands to a province for the supply limit. Friendly: the
    /// rebels, a province under siege, or a controller who is friendly for movement (`Relations`),
    /// an ally, gives the country military access or holds it in its sphere.
    pub fn supply_access(&self, mil: &MilitaryState, state: &State, rel: &Relations, tag: &str, province: u16) -> logistics::SupplyAccess {
        use logistics::SupplyAccess;
        if is_sea(&self.map, province) {
            return SupplyAccess::Sea;
        }
        let nobody = ProvinceMilitary::default();
        let p = mil.provinces.get(&province).unwrap_or(&nobody);
        let friend = |c: &str| {
            rel.friendly(tag, c)
                || state.alliances.iter().any(|(a, b)| (a == tag && b == c) || (a == c && b == tag))
                || state.countries.get(tag).is_some_and(|x| x.access.contains(c))
                || state.countries.get(c).is_some_and(|x| x.sphere.contains(tag))
        };
        match (p.owner.as_deref() == Some(tag), p.controller.as_deref() == Some(tag)) {
            (true, true) => SupplyAccess::OwnControlled,
            (false, true) => SupplyAccess::Controlled,
            (true, false) => SupplyAccess::OwnedOccupied,
            _ if tag == self.rebel_tag || mil.combats.iter().any(|c| c.kind == CombatKind::Siege && c.location == province) || p.controller.as_deref().is_some_and(friend) => SupplyAccess::Friendly,
            _ => SupplyAccess::Hostile,
        }
    }

    fn unit(&self, kind: &str) -> UnitDef {
        self.units.get(kind).cloned().unwrap_or(UnitDef { max_strength: 3.0, ..UnitDef::default() })
    }

    pub fn relations(&self, mil: &MilitaryState, state: &State) -> Relations {
        let mut rel = Relations { rebel_tag: self.rebel_tag.clone(), ..Relations::default() };
        let pair = |a: &str, b: &str| if a <= b { (a.to_string(), b.to_string()) } else { (b.to_string(), a.to_string()) };
        for w in mil.wars.iter().filter(|w| w.active) {
            for a in &w.attackers {
                for d in &w.defenders {
                    rel.wars.insert(pair(a, d));
                }
            }
            for side in [&w.attackers, &w.defenders] {
                for a in side {
                    for b in side {
                        if a != b {
                            rel.friendly.insert((a.clone(), b.clone()));
                        }
                    }
                }
            }
        }
        for (over, sub) in state.vassals.iter().chain(&state.substates) {
            rel.subjects.insert(pair(over, sub));
            rel.overlord.insert(sub.clone(), over.clone());
            rel.friendly.insert((over.clone(), sub.clone()));
            rel.friendly.insert((sub.clone(), over.clone()));
        }
        // Two subjects of one overlord are friendly to each other.
        let subs: Vec<(&String, &String)> = rel.overlord.iter().collect();
        let mut same = Vec::new();
        for (s1, o1) in &subs {
            for (s2, o2) in &subs {
                if s1 != s2 && o1 == o2 {
                    same.push(((*s1).clone(), (*s2).clone()));
                }
            }
        }
        rel.friendly.extend(same);
        rel
    }

    /// First call: infrastructure of the provinces from the state, the reserve lines of battles that
    /// were loaded from a save. Every call: owners from the state.
    fn sync(&mut self, mil: &mut MilitaryState, state: &State) {
        for (id, p) in &state.provinces {
            if let Some(m) = mil.provinces.get_mut(id) {
                m.owner = p.owner.clone();
            }
        }
        if self.synced {
            return;
        }
        self.synced = true;
        // The save's strengths are those of the evening before the first day we run.
        self.keep_siege_men(mil);
        if let Some((name, per_level)) = &self.infrastructure {
            for (id, p) in &state.provinces {
                if let Some(m) = self.map.provinces.get_mut(id) {
                    m.infrastructure = p.buildings.get(name).copied().unwrap_or(0.0) * per_level;
                }
            }
        }
        for c in mil.combats.iter_mut().filter(|c| c.kind == CombatKind::Land) {
            fill_reserve(c, &mil.armies);
        }
        for n in &mil.navies {
            self.naval_months.insert(n.id, (n.no_supply_days / 30.0).floor());
        }
    }

    fn remove_army(&mut self, mil: &mut MilitaryState, id: u64) {
        mil.armies.retain(|a| a.id != id);
        for n in &mut mil.navies {
            n.carried.retain(|x| *x != id);
        }
        for c in mil.combats.iter_mut().filter(|c| c.kind != CombatKind::Naval) {
            c.attacker.units.retain(|x| *x != id);
            c.defender.units.retain(|x| *x != id);
        }
        self.locks.remove(&id);
        self.retreating.remove(&id);
    }

    fn in_combat(mil: &MilitaryState, is_army: bool, id: u64, siege: Option<bool>) -> bool {
        mil.combats.iter().any(|c| (c.kind != CombatKind::Naval) == is_army && siege.is_none_or(|s| (c.kind == CombatKind::Siege) == s) && (c.attacker.units.contains(&id) || c.defender.units.contains(&id)))
    }

    fn leader_of(mil: &MilitaryState, id: Option<u64>) -> Option<&Leader> {
        let id = id?;
        mil.leaders.iter().find(|l| l.id == id)
    }

    // ------------------------------------------------------------------ the day

    pub fn day(&mut self, mil: &mut MilitaryState, state: &State, date: Date) -> DayReport {
        let mut rep = DayReport { date: Some(date), ..DayReport::default() };
        mil.date = date;
        self.sync(mil, state);
        let rel = self.relations(mil, state);
        let mut cas: BTreeMap<u64, f64> = BTreeMap::new();

        self.goals_day(mil, state, date);
        self.unit_day(mil, state, &rel, date, &mut cas, &mut rep);
        self.battle_day(mil, state, &rel, date, &mut cas, &mut rep);
        self.siege_day(mil, state, &rel, date, &mut rep);
        if date.day == 1 {
            rep.monthly = true;
            self.month(mil, state, &rel, &mut cas, &mut rep);
        }
        self.keep_siege_men(mil);
        rep.war_scores = self.war_scores(mil, state);
        rep.casualties = cas.into_iter().filter(|(_, m)| *m > 0.0).collect();
        rep
    }

    // ------------------------------------------------------------------ 1. war score and goals

    fn side_set(w: &war::War, attackers: bool) -> BTreeSet<String> {
        (if attackers { &w.attackers } else { &w.defenders }).iter().cloned().collect()
    }

    fn goals_day(&mut self, mil: &mut MilitaryState, state: &State, date: Date) {
        let today = self.day_number(date);
        for wi in 0..mil.wars.len() {
            if !mil.wars[wi].active {
                continue;
            }
            let (att, def) = (Self::side_set(&mil.wars[wi], true), Self::side_set(&mil.wars[wi], false));
            let started = mil.wars[wi].started();
            let mut updates = Vec::new();
            for (gi, g) in mil.wars[wi].goals.iter().enumerate() {
                let Some(cb) = self.cb_types.get(&g.casus_belli) else { continue };
                if !cb.ticks() || !((att.contains(&g.actor) && def.contains(&g.receiver)) || (def.contains(&g.actor) && att.contains(&g.receiver))) {
                    continue;
                }
                let actor_side = if att.contains(&g.actor) { &att } else { &def };
                let fulfilment = if cb.has("demand_state") || cb.has("transfer_provinces") {
                    let wanted = Self::wanted(state, g.state_province);
                    if wanted.is_empty() { 0.0 } else { wanted.iter().filter(|p| mil.provinces.get(p).and_then(|m| m.controller.as_ref()).is_some_and(|c| actor_side.contains(c))).count() as f64 / wanted.len() as f64 }
                } else if cb.has("annex") {
                    let owned: Vec<&ProvinceMilitary> = mil.provinces.values().filter(|m| m.owner.as_deref() == Some(g.receiver.as_str())).collect();
                    if owned.is_empty() { 0.0 } else { owned.iter().filter(|m| m.controller.as_ref().is_some_and(|c| actor_side.contains(c))).count() as f64 / owned.len() as f64 }
                } else {
                    let (mut count, mut won, mut lost) = (0u32, 0.0, 0.0);
                    for b in mil.wars[wi].battles() {
                        let (a, d) = (b.attacker.country.as_str(), b.defender.country.as_str());
                        if !((a == g.actor && d == g.receiver) || (a == g.receiver && d == g.actor)) {
                            continue;
                        }
                        count += 1;
                        let (w, l) = if b.attacker_won { (&b.attacker, &b.defender) } else { (&b.defender, &b.attacker) };
                        let pts = war::battle_points(&self.war, w.losses, l.losses, is_sea(&self.map, b.location));
                        if w.country == g.actor { won += pts } else { lost += pts }
                    }
                    war::battle_fulfilment(&self.war, count, won, lost, cb.tws_battle_factor)
                };
                let since = g.date.or(started).map_or(0.0, |d| (today - self.day_number(d)) as f64);
                updates.push((gi, war::tick_goal(&self.war, g.score, fulfilment, since), fulfilment > 0.0));
            }
            for (gi, score, fulfilled) in updates {
                let g = &mut mil.wars[wi].goals[gi];
                g.change = score - g.score;
                g.score = score;
                g.fulfilled = fulfilled;
            }
        }
    }

    /// Provinces of the state a goal wants: the same region and owner as its province.
    fn wanted(state: &State, province: Option<u16>) -> Vec<u16> {
        let Some(p) = province.and_then(|p| state.provinces.get(&p).map(|s| (p, s))) else { return Vec::new() };
        let (region, owner) = (p.1.region, p.1.owner.clone());
        state.provinces.iter().filter(|(id, s)| **id == p.0 || (s.region == region && s.owner == owner && region.is_some())).map(|(id, _)| *id).collect()
    }

    /// Score of every running war from the attackers' leader against the defenders' leader.
    pub fn war_scores(&self, mil: &MilitaryState, state: &State) -> Vec<(String, f64)> {
        let running: Vec<&war::War> = mil.wars.iter().filter(|w| w.active && w.attacker_leader().is_some() && w.defender_leader().is_some()).collect();
        if running.is_empty() {
            return Vec::new();
        }
        let owners: BTreeSet<String> = running.iter().flat_map(|w| w.attackers.iter().chain(&w.defenders)).cloned().collect();
        let mut value: BTreeMap<u16, f64> = BTreeMap::new();
        let mut total: BTreeMap<String, f64> = BTreeMap::new();
        for (id, p) in &mil.provinces {
            let Some(owner) = p.owner.as_ref().filter(|o| owners.contains(*o)) else { continue };
            let s = state.provinces.get(id);
            let capital = state.countries.get(owner).and_then(|c| c.capital);
            let cap_prov = capital.and_then(|c| state.provinces.get(&c));
            let overseas = capital.is_some_and(|c| state.land_mass.get(&c) != state.land_mass.get(id) && !state.land_mass.is_empty());
            let v = war::province_value(
                &self.war,
                &war::ProvinceValueInput {
                    naval_base_level: p.naval_base,
                    state_factories: 0.0,
                    fort_level: p.fort,
                    owner_has_core: s.is_some_and(|s| s.cores.contains(owner)),
                    in_owner_capital_state: s.is_some_and(|s| s.region.is_some() && s.region == cap_prov.and_then(|c| c.region)),
                    overseas,
                    colonial_or_unowned: s.is_some_and(|s| s.colonial > 0),
                },
            );
            value.insert(*id, v);
            *total.entry(owner.clone()).or_default() += v;
        }
        let mut out = Vec::new();
        for w in running {
            let (att, def) = (Self::side_set(w, true), Self::side_set(w, false));
            let (la, ld) = (w.attacker_leader().unwrap_or_default().to_string(), w.defender_leader().unwrap_or_default().to_string());
            let controlled_by = |p: &ProvinceMilitary, side: &BTreeSet<String>| p.controller.as_ref().is_some_and(|c| side.contains(c));
            let mut occupation = 0.0;
            for (id, p) in &mil.provinces {
                let Some(owner) = p.owner.as_ref() else { continue };
                let s = value.get(id).map_or(0.0, |v| war::province_score(*v, total.get(owner).copied().unwrap_or(0.0)));
                if def.contains(owner) && controlled_by(p, &att) {
                    occupation += s;
                } else if att.contains(owner) && controlled_by(p, &def) {
                    occupation -= s;
                }
            }
            let holds = |leader: &str, by: &BTreeSet<String>| {
                let own: Vec<&ProvinceMilitary> = mil.provinces.values().filter(|p| p.owner.as_deref() == Some(leader)).collect();
                let all = !own.is_empty() && own.iter().all(|p| controlled_by(p, by));
                let controls_any = mil.provinces.values().any(|p| p.controller.as_deref() == Some(leader));
                let capital = state.countries.get(leader).and_then(|c| c.capital).and_then(|c| mil.provinces.get(&c)).is_some_and(|p| controlled_by(p, by));
                let half = !own.is_empty() && own.iter().filter(|p| controlled_by(p, by)).count() * 2 > own.len();
                (all, !controls_any && (capital || half))
            };
            let (a_all, a_nothing) = holds(&ld, &att);
            let (b_all, b_nothing) = holds(&la, &def);
            let mut ticking = 0.0;
            for g in &w.goals {
                let Some(cb) = self.cb_types.get(&g.casus_belli) else { continue };
                if !cb.ticks() {
                    continue;
                }
                let cost = war::goal_cost(&self.war, cb, &war::GoalCostInput { provinces: Self::wanted(state, g.state_province).iter().filter_map(|p| value.get(p).map(|v| (*v, total.get(g.receiver.as_str()).copied().unwrap_or(0.0)))).collect(), ..Default::default() });
                let part = war::goal_score_contribution(&self.war, g.score, cost.max(1.0));
                ticking += if att.contains(&g.actor) { part } else { -part };
            }
            let battles = war::battles_part(&self.war, w, &|b| is_sea(&self.map, b.location));
            out.push((w.name.clone(), war::war_score(&war::ScoreInput { a_holds_all_of_b: a_all, b_holds_all_of_a: b_all, b_has_nothing_and_a_holds_capital_or_half: a_nothing, a_has_nothing_and_b_holds_capital_or_half: b_nothing, battles, occupation, ticking })));
        }
        out
    }

    // ------------------------------------------------------------------ 2 and 3. each unit's day

    fn border_ctx(&self, mil: &MilitaryState, rel: &Relations, owner: &str) -> BorderContext {
        BorderContext {
            hostile_fleet_seas: mil.navies.iter().filter(|n| !n.ships.is_empty() && rel.hostile(owner, &n.owner) && !self.retreating.contains(&n.id)).map(|n| n.location).collect(),
            hostile_controlled: mil.provinces.iter().filter(|(_, p)| p.controller.as_deref().is_some_and(|c| rel.hostile(owner, c))).map(|(id, _)| *id).collect(),
        }
    }

    fn unit_day(&mut self, mil: &mut MilitaryState, state: &State, rel: &Relations, date: Date, cas: &mut BTreeMap<u64, f64>, rep: &mut DayReport) {
        let hour = self.calendar.to_game(date).map_or(0, |g| g.0);
        let mut ctxs: BTreeMap<String, BorderContext> = BTreeMap::new();
        mil.armies.retain(|a| !a.regiments.is_empty());
        rep.armies_destroyed += self.fleet_losses(mil);
        // A call lapses when its army no longer waits on the coast for a fleet.
        let map = &self.map;
        self.calls.retain(|nid, aid| mil.navies.iter().any(|n| n.id == *nid) && mil.armies.iter().any(|a| a.id == *aid && a.carried_by.is_none() && a.path.first().is_some_and(|p| is_sea(map, *p))));
        let ids: Vec<u64> = mil.armies.iter().map(|a| a.id).collect();
        // Fleets first, so the armies they carry travel with them.
        let navy_ids: Vec<u64> = mil.navies.iter().map(|n| n.id).collect();
        for id in navy_ids {
            self.navy_day(mil, rel, id, hour, &mut ctxs, cas);
        }
        for id in ids {
            let Some(i) = mil.armies.iter().position(|a| a.id == id) else { continue };
            self.army_day(mil, state, rel, i, hour, date, &mut ctxs, cas, rep);
        }
        rep.armies_destroyed += self.fleet_losses(mil);
    }

    /// After ships are lost: a fleet without ships is removed, and the armies it carried go down
    /// with it at sea (as when a fleet is destroyed in battle) or walk ashore in port. A fleet
    /// that kept fewer transports than its cargo needs loses regiments until the rest fit
    /// (`logistics::trim_over_capacity`). Returns the armies destroyed.
    fn fleet_losses(&mut self, mil: &mut MilitaryState) -> usize {
        let mut gone: Vec<u64> = Vec::new();
        let dead: Vec<(u64, u16)> = mil.navies.iter().filter(|n| n.ships.is_empty()).map(|n| (n.id, n.location)).collect();
        for (nid, loc) in &dead {
            let at_sea = is_sea(&self.map, *loc);
            for a in mil.armies.iter_mut().filter(|a| a.carried_by == Some(*nid)) {
                if at_sea {
                    gone.push(a.id);
                } else {
                    a.carried_by = None;
                    a.location = *loc;
                }
            }
            self.calls.remove(nid);
        }
        mil.navies.retain(|n| !n.ships.is_empty());
        for ni in 0..mil.navies.len() {
            let nid = mil.navies[ni].id;
            let room = self.fleet_capacity(&mil.navies[ni]);
            let used: usize = mil.armies.iter().filter(|a| a.carried_by == Some(nid)).map(|a| a.regiments.len()).sum();
            if used <= room {
                continue;
            }
            let before_all: Vec<u64> = mil.armies.iter().map(|a| a.id).collect();
            let (mut carried, rest): (Vec<Army>, Vec<Army>) = std::mem::take(&mut mil.armies).into_iter().partition(|a| a.carried_by == Some(nid));
            let before: Vec<u64> = carried.iter().map(|a| a.id).collect();
            logistics::trim_over_capacity(&mut carried, room);
            gone.extend(before.into_iter().filter(|id| !carried.iter().any(|a| a.id == *id)));
            // Back in their old order.
            let order: BTreeMap<u64, usize> = before_all.iter().enumerate().map(|(i, id)| (*id, i)).collect();
            mil.armies = rest;
            mil.armies.extend(carried);
            mil.armies.sort_by_key(|a| order[&a.id]);
        }
        let n = gone.len();
        for id in gone {
            self.remove_army(mil, id);
        }
        n
    }

    #[allow(clippy::too_many_arguments)]
    fn army_day(&mut self, mil: &mut MilitaryState, state: &State, rel: &Relations, i: usize, hour: i64, date: Date, ctxs: &mut BTreeMap<String, BorderContext>, cas: &mut BTreeMap<u64, f64>, rep: &mut DayReport) {
        let id = mil.armies[i].id;
        let in_land_battle = Self::in_combat(mil, true, id, Some(false));
        let carried = mil.armies[i].carried_by.is_some();
        if let Some(n) = mil.armies[i].carried_by.and_then(|nid| mil.navies.iter().find(|n| n.id == nid)) {
            let (loc, prev) = (n.location, n.previous);
            let a = &mut mil.armies[i];
            a.location = loc;
            a.previous = prev;
        }
        // Organisation regain and dig-in.
        if !in_land_battle && !carried {
            self.regain(mil, state, i);
        }
        let owner = mil.armies[i].owner.clone();
        let at_war = mil.active_wars_of(&owner).next().is_some();
        if !carried {
            let a = &mut mil.armies[i];
            if !a.path.is_empty() || !at_war {
                a.dig_in = 0.0;
                a.dig_in_date = None;
            } else if !in_land_battle {
                let cap = self.country_sum(state, &owner, "dig_in_cap");
                let a = &mut mil.armies[i];
                let last = a.dig_in_date.unwrap_or(date);
                if a.dig_in_date.is_none() {
                    a.dig_in_date = Some(date);
                } else if (self.day_number(date) - self.day_number(last)) as f64 >= self.dig_in_days && a.dig_in < cap {
                    a.dig_in += 1.0;
                    a.dig_in_date = Some(date);
                }
            }
        }
        // One step along the path.
        if !carried && !in_land_battle && !mil.armies[i].path.is_empty() {
            self.army_step(mil, state, rel, i, hour, ctxs, cas, rep);
        } else if carried && !mil.armies[i].path.is_empty() {
            self.carried_step(mil, state, rel, i, hour, ctxs, cas, rep);
        } else if !mil.armies[i].path.is_empty() && in_land_battle {
            // Progress grows but is capped, no step.
            let a = &mil.armies[i];
            let (from, to) = (a.location, a.path[0]);
            if let Some(b) = self.map.border(from, to) {
                let d = movement::step_distance(&self.movement, true, is_sea(&self.map, from), is_sea(&self.map, to), b.distance, 0.0);
                let a = &mut mil.armies[i];
                a.movement_progress = (a.movement_progress + 1.0).min(d);
            }
        }
        if mil.armies.get(i).is_some_and(|a| a.id == id) {
            if mil.armies[i].path.is_empty() {
                self.retreating.remove(&id);
            }
            // The combat check of a standing army.
            let standing = mil.armies[i].path.is_empty() && !self.retreating.contains(&id) && !carried && !Self::in_combat(mil, true, id, None);
            if standing {
                self.check_unit(mil, rel, true, id, rep, 0);
            }
            let l = self.locks.entry(id).or_insert(0.0);
            *l = movement::lock_tick(*l);
        }
    }

    fn regain(&mut self, mil: &mut MilitaryState, state: &State, i: usize) {
        let owner = mil.armies[i].owner.clone();
        let rebel = owner == self.rebel_tag;
        let slider = state.countries.get(&owner).and_then(|c| c.spending.get("military_spending")).copied().unwrap_or(1.0).clamp(0.0, 1.0);
        let (morale_l, org_l) = Self::leader_of(mil, mil.armies[i].leader).map_or((0.0, 0.0), |l| logistics::prestige_bonus(&self.logistics, l.prestige));
        let tech_morale = self.tech_sum(state, &owner, "morale");
        let status = if rebel { 1.0 } else { self.supply_status };
        for r in &mut mil.armies[i].regiments {
            let def = self.units.get(&r.kind).cloned().unwrap_or_default();
            let max = logistics::max_organisation(&self.logistics, def.organisation, 0.0, org_l, slider, rebel);
            r.organisation = logistics::regain_organisation(&self.logistics, r.organisation, max, status, morale_l, tech_morale, 0.0);
        }
    }

    /// Speed and distance of an army's step over a land border into `to`.
    fn land_march(&self, a: &Army, to: u16, border: &movement::Border) -> (f64, f64) {
        let slowest = a.regiments.iter().map(|r| self.unit(&r.kind).speed).fold(f64::INFINITY, f64::min);
        let leader_speed = 0.0;
        let infra = self.map.provinces.get(&to).map_or(0.0, |m| m.infrastructure);
        let speed = movement::unit_speed(&self.movement, &movement::SpeedInput { is_army: true, from_sea: false, to_sea: false, unit_speed: if slowest.is_finite() { slowest } else { 0.0 }, terrain_entry: 0.0, crossing_entry: 0.0, infrastructure: infra, leader_speed });
        let left = self.map.provinces.get(&a.location).map_or(0.0, |m| m.cost_modifier);
        (speed, movement::step_distance(&self.movement, true, false, false, border.distance, left))
    }

    /// Days until a marching army takes the next step of its path; `None` for a standing one.
    pub fn days_to_next(&self, a: &Army) -> Option<f64> {
        let to = *a.path.first()?;
        let (speed, distance) = self.land_march(a, to, self.map.border(a.location, to)?);
        Some(if speed > 0.0 { ((distance - a.movement_progress) / speed).ceil().max(0.0) } else { f64::MAX })
    }

    /// The organisation ceiling of a regiment type in this army (leader, military spending), as the daily regain uses it.
    pub fn max_organisation(&self, mil: &MilitaryState, state: &State, a: &Army, kind: &str) -> f64 {
        let slider = state.countries.get(&a.owner).and_then(|c| c.spending.get("military_spending")).copied().unwrap_or(1.0).clamp(0.0, 1.0);
        let org_l = Self::leader_of(mil, a.leader).map_or(0.0, |l| logistics::prestige_bonus(&self.logistics, l.prestige).1);
        logistics::max_organisation(&self.logistics, self.units.get(kind).map_or(0.0, |u| u.organisation), 0.0, org_l, slider, a.owner == self.rebel_tag)
    }

    /// The army is retreating from a battle.
    pub fn is_retreating(&self, army: u64) -> bool {
        self.retreating.contains(&army)
    }

    #[allow(clippy::too_many_arguments)]
    fn army_step(&mut self, mil: &mut MilitaryState, state: &State, rel: &Relations, i: usize, hour: i64, ctxs: &mut BTreeMap<String, BorderContext>, cas: &mut BTreeMap<u64, f64>, rep: &mut DayReport) {
        let a = &mil.armies[i];
        let (id, owner, from, to) = (a.id, a.owner.clone(), a.location, a.path[0]);
        let (sea_from, sea_to) = (is_sea(&self.map, from), is_sea(&self.map, to));
        let Some(border) = self.map.border(from, to).copied() else {
            mil.armies[i].path.clear();
            return;
        };
        // An army at sea without a fleet has nothing to carry it on.
        if sea_from {
            let a = &mut mil.armies[i];
            a.path.clear();
            a.movement_progress = 0.0;
            return;
        }
        if sea_to {
            self.embark_step(mil, rel, i, hour, &border);
            return;
        }
        let ctx = ctxs.entry(owner.clone()).or_insert_with(|| self.border_ctx(mil, rel, &owner)).clone();
        let (pf, pt) = (mil.provinces.get(&from).cloned().unwrap_or_default(), mil.provinces.get(&to).cloned().unwrap_or_default());
        let closed = movement::border_closed(&self.map, &ctx, false, from, to) || !movement::may_enter_land(rel, &owner, false, &pf, &pt);
        let (speed, distance) = self.land_march(&mil.armies[i], to, &border);
        match movement::daily_step(mil.armies[i].movement_progress, speed, distance, false, hour, 0, closed) {
            movement::DailyStep::Cancel => {
                let a = &mut mil.armies[i];
                a.path.clear();
                a.movement_progress = 0.0;
            }
            movement::DailyStep::Wait { progress } => mil.armies[i].movement_progress = progress,
            movement::DailyStep::Step { progress } => {
                {
                    let a = &mut mil.armies[i];
                    a.movement_progress = progress;
                    a.previous = Some(from);
                    a.location = to;
                    a.path.remove(0);
                    if a.path.is_empty() {
                        a.target = None;
                    }
                }
                self.after_step(mil, state, rel, id, cas, rep);
            }
        }
    }

    /// Attrition and the arrival check of an army that just entered a province.
    fn after_step(&mut self, mil: &mut MilitaryState, state: &State, rel: &Relations, id: u64, cas: &mut BTreeMap<u64, f64>, rep: &mut DayReport) {
        self.attrition(mil, state, rel, id, cas);
        // The arrival check runs right after the step (3.6): the arriving unit is the
        // attacker. The just-moved lock only holds back the daily check of later days.
        self.locks.insert(id, 0.0);
        if mil.armies.iter().any(|a| a.id == id) && !self.retreating.contains(&id) && !Self::in_combat(mil, true, id, None) {
            self.check_unit(mil, rel, true, id, rep, 0);
        }
        self.locks.insert(id, movement::lock_after_step(&self.movement));
    }

    // ------------------------------------------------------------------ over water

    /// Regiments a fleet can carry: its transport ships (unit types with `transport = yes`) times
    /// `MovementRules::regiments_per_transport`.
    pub fn fleet_capacity(&self, n: &Navy) -> usize {
        movement::transport_capacity(&self.movement, n.ships.iter().filter(|s| self.unit(&s.kind).transport).count())
    }

    /// Regiments a fleet can still take on board.
    pub fn fleet_free_room(&self, mil: &MilitaryState, n: &Navy) -> usize {
        let used: usize = mil.armies.iter().filter(|a| a.carried_by == Some(n.id)).map(|a| a.regiments.len()).sum();
        self.fleet_capacity(n).saturating_sub(used)
    }

    /// The fleet is retreating from a naval battle.
    pub fn fleet_retreating(&self, navy: u64) -> bool {
        self.retreating.contains(&(navy | NAVY_BIT))
    }

    /// The fleet has been called to carry an army.
    pub fn fleet_called(&self, navy: u64) -> bool {
        self.calls.contains_key(&navy)
    }

    /// A fleet that may serve: not fighting, not retreating.
    fn fleet_free(&self, mil: &MilitaryState, n: &Navy) -> bool {
        !Self::in_combat(mil, false, n.id, None) && !self.retreating.contains(&(n.id | NAVY_BIT))
    }

    /// The fleet an army on the coast `from` boards to step onto the sea province `to`: one of its
    /// country's standing fleets in `to`, or in port in `from` with `to` its way out, with room for
    /// all its regiments, and not called for another army. The lowest id (`movement::embark`).
    fn boarding_fleet(&self, mil: &MilitaryState, army: &Army, from: u16, to: u16) -> Option<u64> {
        let ctx = BorderContext::default();
        let rooms: Vec<movement::FleetRoom> = mil
            .navies
            .iter()
            .filter(|n| n.owner == army.owner && n.path.is_empty() && self.fleet_free(mil, n))
            .filter(|n| n.location == to || (n.location == from && !movement::border_closed(&self.map, &ctx, true, from, to)))
            .filter(|n| self.calls.get(&n.id).is_none_or(|a| *a == army.id))
            .map(|n| movement::FleetRoom { id: n.id, owner: n.owner.clone(), free: self.fleet_free_room(mil, n) })
            .collect();
        movement::embark(&army.owner, army.regiments.len(), &rooms)
    }

    /// Fleet path from `from` to `to` (borders as `navy_day` takes them) and its length.
    fn fleet_route(&self, ctx: &BorderContext, from: u16, to: u16) -> Option<(Vec<u16>, f64)> {
        let path = movement::find_path(&self.map, from, to, &|a, b| !movement::border_closed(&self.map, ctx, true, a, b))?;
        let mut at = from;
        let mut length = 0.0;
        for p in &path {
            length += self.map.border(at, *p).map_or(0.0, |b| b.distance);
            at = *p;
        }
        Some((path, length))
    }

    /// Makes sure a fleet is on its way to carry the army from the coast `from` over the sea
    /// province `to`. False when no fleet of its country with room can get there.
    fn summon(&mut self, mil: &mut MilitaryState, rel: &Relations, ai: usize, from: u16, to: u16) -> bool {
        let (army, owner, regs) = (mil.armies[ai].id, mil.armies[ai].owner.clone(), mil.armies[ai].regiments.len());
        let ctx = self.border_ctx(mil, rel, &owner);
        if let Some(nid) = self.calls.iter().find(|(_, a)| **a == army).map(|(n, _)| *n) {
            let ok = mil.navies.iter().position(|n| n.id == nid).filter(|ni| {
                let n = &mil.navies[*ni];
                self.fleet_free(mil, n) && self.fleet_free_room(mil, n) >= regs
            });
            if let Some(ni) = ok {
                let n = &mil.navies[ni];
                if !n.path.is_empty() || n.location == to || n.location == from {
                    return true;
                }
                // Its voyage was cut short: a new way, or another fleet.
                if let Some((path, _)) = self.fleet_route(&ctx, n.location, to) {
                    let n = &mut mil.navies[ni];
                    n.path = path;
                    n.movement_progress = 0.0;
                    return true;
                }
            }
            self.calls.remove(&nid);
        }
        let mut best: Option<(f64, u64, Vec<u16>)> = None;
        for n in mil.navies.iter().filter(|n| n.owner == owner && n.path.is_empty() && !self.calls.contains_key(&n.id) && self.fleet_free(mil, n)) {
            if self.fleet_free_room(mil, n) < regs || mil.armies.iter().any(|a| a.carried_by == Some(n.id)) {
                continue;
            }
            let Some((path, length)) = self.fleet_route(&ctx, n.location, to) else { continue };
            if best.as_ref().is_none_or(|b| length < b.0) {
                best = Some((length, n.id, path));
            }
        }
        let Some((_, nid, path)) = best else { return false };
        if let Some(n) = mil.navies.iter_mut().find(|n| n.id == nid) {
            n.path = path;
            n.movement_progress = 0.0;
        }
        self.calls.insert(nid, army);
        true
    }

    /// An army on the coast whose next step is the sea province `to`: it boards a fleet there (a
    /// step between land and sea), waits for one that was called, or gives up the order.
    fn embark_step(&mut self, mil: &mut MilitaryState, rel: &Relations, i: usize, hour: i64, border: &movement::Border) {
        let (id, from, to) = (mil.armies[i].id, mil.armies[i].location, mil.armies[i].path[0]);
        let Some(nid) = self.boarding_fleet(mil, &mil.armies[i], from, to) else {
            if !self.summon(mil, rel, i, from, to) {
                let a = &mut mil.armies[i];
                a.path.clear();
                a.movement_progress = 0.0;
                a.target = None;
            }
            return;
        };
        // The fleet waits for the army while it boards: no other army may call it away.
        self.calls.insert(nid, id);
        let speed = movement::unit_speed(&self.movement, &movement::SpeedInput { is_army: true, from_sea: false, to_sea: true, unit_speed: 0.0, terrain_entry: 0.0, crossing_entry: 0.0, infrastructure: 0.0, leader_speed: 0.0 });
        let distance = movement::step_distance(&self.movement, true, false, true, border.distance, 0.0);
        match movement::daily_step(mil.armies[i].movement_progress, speed, distance, false, hour, 0, false) {
            movement::DailyStep::Cancel => {}
            movement::DailyStep::Wait { progress } => mil.armies[i].movement_progress = progress,
            movement::DailyStep::Step { .. } => {
                let ni = mil.navies.iter().position(|n| n.id == nid).expect("boarding fleet exists");
                let at = mil.navies[ni].location;
                mil.navies[ni].carried.push(id);
                let a = &mut mil.armies[i];
                a.carried_by = Some(nid);
                a.previous = Some(from);
                a.location = at;
                a.movement_progress = 0.0;
                a.dig_in = 0.0;
                a.dig_in_date = None;
                self.calls.remove(&nid);
                self.launch(mil, i, ni);
            }
        }
    }

    /// The fleet takes the sea leg at the head of a carried army's path; the army keeps the rest.
    fn launch(&mut self, mil: &mut MilitaryState, i: usize, ni: usize) {
        let sea: Vec<u16> = mil.armies[i].path.iter().copied().take_while(|p| is_sea(&self.map, *p)).collect();
        let a = &mut mil.armies[i];
        let rest = a.path.split_off(sea.len());
        a.target = rest.last().copied().or(a.target);
        a.path = rest;
        a.movement_progress = 0.0;
        let n = &mut mil.navies[ni];
        let skip = usize::from(sea.first() == Some(&n.location));
        n.path = sea[skip..].to_vec();
        n.movement_progress = 0.0;
    }

    /// An army on board with an order: it waits while the fleet sails, sends it on over the next
    /// sea leg, or lands (a step between sea and land) in the next province of its path.
    #[allow(clippy::too_many_arguments)]
    fn carried_step(&mut self, mil: &mut MilitaryState, state: &State, rel: &Relations, i: usize, hour: i64, ctxs: &mut BTreeMap<String, BorderContext>, cas: &mut BTreeMap<u64, f64>, rep: &mut DayReport) {
        let (id, owner, to) = (mil.armies[i].id, mil.armies[i].owner.clone(), mil.armies[i].path[0]);
        let Some(ni) = mil.armies[i].carried_by.and_then(|nid| mil.navies.iter().position(|n| n.id == nid)) else {
            mil.armies[i].path.clear();
            return;
        };
        if !mil.navies[ni].path.is_empty() || !self.fleet_free(mil, &mil.navies[ni]) {
            return;
        }
        let at = mil.navies[ni].location;
        let border = self.map.border(at, to).copied();
        if is_sea(&self.map, to) && (at == to || border.is_some()) {
            self.launch(mil, i, ni);
            return;
        }
        let Some(border) = border else {
            // The fleet is not next to the next step (its voyage was cut short): a new way.
            let a = &mut mil.armies[i];
            let dest = a.path.last().copied().or(a.target);
            match dest.and_then(|d| movement::army_path(&self.map, at, d)) {
                Some((path, _)) if !path.is_empty() => a.path = path,
                _ => {
                    a.path.clear();
                    a.target = None;
                    a.movement_progress = 0.0;
                }
            }
            return;
        };
        if !is_sea(&self.map, at) {
            // A fleet in port: the army walks off and marches on from there.
            mil.navies[ni].carried.retain(|x| *x != id);
            let a = &mut mil.armies[i];
            a.carried_by = None;
            a.location = at;
            return;
        }
        let ctx = ctxs.entry(owner.clone()).or_insert_with(|| self.border_ctx(mil, rel, &owner)).clone();
        let pt = mil.provinces.get(&to).cloned().unwrap_or_default();
        let closed = movement::border_closed(&self.map, &ctx, false, at, to) || !movement::may_disembark(rel, &owner, false, &ProvinceMilitary::default(), &pt);
        let speed = movement::unit_speed(&self.movement, &movement::SpeedInput { is_army: true, from_sea: true, to_sea: false, unit_speed: 0.0, terrain_entry: 0.0, crossing_entry: 0.0, infrastructure: 0.0, leader_speed: 0.0 });
        let distance = movement::step_distance(&self.movement, true, true, false, border.distance, 0.0);
        match movement::daily_step(mil.armies[i].movement_progress, speed, distance, false, hour, 0, closed) {
            movement::DailyStep::Cancel => {
                let a = &mut mil.armies[i];
                a.path.clear();
                a.target = None;
                a.movement_progress = 0.0;
            }
            movement::DailyStep::Wait { progress } => mil.armies[i].movement_progress = progress,
            movement::DailyStep::Step { progress } => {
                mil.navies[ni].carried.retain(|x| *x != id);
                let a = &mut mil.armies[i];
                a.carried_by = None;
                a.previous = Some(at);
                a.location = to;
                a.movement_progress = progress;
                a.path.remove(0);
                if a.path.is_empty() {
                    a.target = None;
                }
                self.after_step(mil, state, rel, id, cas, rep);
            }
        }
    }

    fn navy_day(&mut self, mil: &mut MilitaryState, rel: &Relations, id: u64, hour: i64, ctxs: &mut BTreeMap<String, BorderContext>, _cas: &mut BTreeMap<u64, f64>) {
        let Some(i) = mil.navies.iter().position(|n| n.id == id) else { return };
        let in_battle = Self::in_combat(mil, false, id, None);
        if mil.navies[i].path.is_empty() {
            if !in_battle {
                self.retreating.remove(&(id | NAVY_BIT));
            }
            return;
        }
        if in_battle {
            return;
        }
        let (owner, from, to) = (mil.navies[i].owner.clone(), mil.navies[i].location, mil.navies[i].path[0]);
        let Some(border) = self.map.border(from, to).copied() else {
            mil.navies[i].path.clear();
            return;
        };
        let ctx = ctxs.entry(owner.clone()).or_insert_with(|| self.border_ctx(mil, rel, &owner)).clone();
        let closed = movement::border_closed(&self.map, &ctx, true, from, to);
        let slowest = mil.navies[i].ships.iter().map(|s| self.unit(&s.kind).speed).fold(f64::INFINITY, f64::min);
        let speed = movement::unit_speed(&self.movement, &movement::SpeedInput { is_army: false, from_sea: true, to_sea: true, unit_speed: if slowest.is_finite() { slowest } else { 0.0 }, terrain_entry: 0.0, crossing_entry: 0.0, infrastructure: 0.0, leader_speed: 0.0 });
        let distance = movement::step_distance(&self.movement, false, true, true, border.distance, 0.0);
        match movement::daily_step(mil.navies[i].movement_progress, speed, distance, false, hour, 0, closed) {
            movement::DailyStep::Cancel => {
                let n = &mut mil.navies[i];
                n.path.clear();
                n.movement_progress = 0.0;
            }
            movement::DailyStep::Wait { progress } => mil.navies[i].movement_progress = progress,
            movement::DailyStep::Step { progress } => {
                let n = &mut mil.navies[i];
                n.movement_progress = progress;
                n.previous = Some(from);
                n.location = to;
                n.path.remove(0);
                self.locks.insert(id | NAVY_BIT, 1.0);
                if n.path.is_empty() {
                    self.retreating.remove(&(id | NAVY_BIT));
                }
            }
        }
    }

    // ------------------------------------------------------------------ arrivals

    fn standing_at(&self, mil: &MilitaryState, loc: u16, skip_army: Option<u64>, skip_navy: Option<u64>) -> Vec<Standing> {
        let mut out: Vec<Standing> = mil
            .armies
            .iter()
            .filter(|a| a.location == loc && Some(a.id) != skip_army)
            .map(|a| Standing { id: a.id, owner: a.owner.clone(), is_army: true, exiled: false, retreating: self.retreating.contains(&a.id), at_sea: is_sea(&self.map, loc), carried: a.carried_by.is_some(), in_port: false })
            .collect();
        out.extend(mil.navies.iter().filter(|n| n.location == loc && Some(n.id) != skip_navy && !n.ships.is_empty()).map(|n| Standing { id: n.id, owner: n.owner.clone(), is_army: false, exiled: false, retreating: self.retreating.contains(&(n.id | NAVY_BIT)), at_sea: is_sea(&self.map, loc), carried: false, in_port: !is_sea(&self.map, loc) }));
        out
    }

    fn views(&self, mil: &MilitaryState, loc: u16) -> Vec<CombatView> {
        mil.combats
            .iter()
            .enumerate()
            .filter(|(_, c)| c.location == loc)
            .map(|(index, c)| CombatView { index, siege: c.kind == CombatKind::Siege, land: c.kind != CombatKind::Naval, attacker_countries: side_countries(mil, c, BattleSide::Attacker), defender_countries: side_countries(mil, c, BattleSide::Defender) })
            .collect()
    }

    /// The combat check of one unit that stands in its province (3.6): join a battle or siege, start
    /// a battle against the first enemy, or start a siege.
    fn check_unit(&mut self, mil: &mut MilitaryState, rel: &Relations, is_army: bool, id: u64, rep: &mut DayReport, depth: u8) {
        let mover = if is_army {
            let Some(a) = mil.armies.iter().find(|a| a.id == id) else { return };
            let moving_fleet = a.carried_by.and_then(|n| mil.navies.iter().find(|x| x.id == n)).is_some_and(|n| !n.path.is_empty());
            (a.location, Mover { id, owner: a.owner.clone(), is_army: true, exiled: false, retreating: self.retreating.contains(&id), strength: army_strength(a), on_moving_fleet: moving_fleet, at_sea: is_sea(&self.map, a.location) || a.carried_by.is_some(), standing: a.path.is_empty(), lock_days: self.locks.get(&id).copied().unwrap_or(0.0) })
        } else {
            let Some(n) = mil.navies.iter().find(|n| n.id == id) else { return };
            (n.location, Mover { id, owner: n.owner.clone(), is_army: false, exiled: false, retreating: self.retreating.contains(&(id | NAVY_BIT)), strength: n.ships.iter().map(|s| s.strength).sum(), on_moving_fleet: false, at_sea: is_sea(&self.map, n.location), standing: n.path.is_empty(), lock_days: self.locks.get(&(id | NAVY_BIT)).copied().unwrap_or(0.0) })
        };
        let (loc, mover) = mover;
        let others = self.standing_at(mil, loc, is_army.then_some(id), (!is_army).then_some(id));
        let views = self.views(mil, loc);
        let controller = mil.provinces.get(&loc).and_then(|p| p.controller.clone());
        match movement::arrival(rel, &mover, controller.as_deref(), &others, &views) {
            Arrival::Nothing => {}
            Arrival::JoinBattle { combat, side } => {
                let side = if side == movement::Side::Attacker { BattleSide::Attacker } else { BattleSide::Defender };
                self.join(mil, combat, side, is_army, id);
            }
            Arrival::JoinSiege { combat, side } => {
                let side = if side == movement::Side::Attacker { BattleSide::Attacker } else { BattleSide::Defender };
                let s = mil.combats[combat].side_mut(side);
                if !s.units.contains(&id) {
                    s.units.push(id);
                }
            }
            Arrival::StartBattle { attacker, defender } => {
                let kind = if is_army { CombatKind::Land } else { CombatKind::Naval };
                mil.combats.push(Combat::new(kind, loc, Vec::new(), Vec::new()));
                let ci = mil.combats.len() - 1;
                self.join(mil, ci, BattleSide::Attacker, is_army, attacker);
                self.join(mil, ci, BattleSide::Defender, is_army, defender);
                rep.battles_started += 1;
                if depth == 0 {
                    // Every other unit in the province is offered to the new battle.
                    for o in others.iter().filter(|o| o.is_army == is_army && o.id != defender) {
                        let free = !Self::in_combat(mil, is_army, o.id, None);
                        if free {
                            self.check_unit(mil, rel, is_army, o.id, rep, 1);
                        }
                    }
                }
            }
            Arrival::StartSiege { besieger } => {
                mil.combats.push(Combat::new(CombatKind::Siege, loc, vec![besieger], Vec::new()));
                rep.sieges_started += 1;
            }
        }
    }

    fn join(&self, mil: &mut MilitaryState, combat: usize, side: BattleSide, is_army: bool, id: u64) {
        if is_army {
            let Some(a) = mil.armies.iter().find(|a| a.id == id) else { return };
            let unit = JoinUnit { army: id, regiments: a.regiments.iter().map(|r| r.id).collect() };
            battle::join_battle(&mut mil.combats[combat], side, &[unit]);
        } else {
            let s = mil.combats[combat].side_mut(side);
            if !s.units.contains(&id) {
                s.units.push(id);
            }
        }
    }

    // ------------------------------------------------------------------ 4. battles

    fn fighters(&self, mil: &MilitaryState, state: &State, armies: &[u64]) -> BTreeMap<u64, Fighter> {
        let mut out = BTreeMap::new();
        for id in armies {
            let Some(a) = mil.armies.iter().find(|a| a.id == *id) else { continue };
            let mut defs: BTreeMap<&str, UnitDef> = BTreeMap::new();
            let recon = self.content.leader_traits(Self::leader_of(mil, a.leader)).reconnaissance;
            for r in &a.regiments {
                let def = defs.entry(r.kind.as_str()).or_insert_with(|| self.unit_of(state, &a.owner, &r.kind));
                out.insert(r.id, Fighter::from_regiment(r, def, a.id, &a.owner, a.dig_in, recon));
            }
        }
        out
    }

    /// The leader whose attack or defence a side fights with (3.1): the one whose attack and defence
    /// add up highest among the leaders of its armies, the first of them when equal. The original
    /// picks by a leader value that is not identified yet; this stand-in fits the one battle of the
    /// reference saves where two leaders differ (most prestige and first army do not).
    pub fn side_leader<'a>(&self, mil: &'a MilitaryState, c: &Combat, side: BattleSide) -> Option<&'a Leader> {
        let mut best: Option<&Leader> = None;
        for l in c.side(side).units.iter().filter_map(|u| mil.armies.iter().find(|a| a.id == *u)).filter_map(|a| Self::leader_of(mil, a.leader)) {
            let skill = |l: &Leader| {
                let t = self.content.leader_traits(Some(l));
                t.attack + t.defence
            };
            if best.is_none_or(|b| skill(l) > skill(b)) {
                best = Some(l);
            }
        }
        best
    }

    #[allow(clippy::too_many_arguments)]
    fn side_info(&self, mil: &MilitaryState, state: &State, rel: &Relations, c: &Combat, side: BattleSide, controller: Option<&str>, fort: f64) -> SideInfo {
        let countries = side_countries(mil, c, side);
        let first = countries.first().cloned().unwrap_or_default();
        let leader = self.side_leader(mil, c, side);
        let traits = self.content.leader_traits(leader);
        SideInfo {
            countries: countries
                .iter()
                .map(|t| {
                    let (gas_attack, gas_defence) = state.countries.get(t).map_or((false, false), |c| self.content.gas(t, &c.inventions));
                    battle::CountryInfo { tag: t.clone(), width_modifier: self.country_sum(state, t, "combat_width"), tactics_modifier: self.country_sum(state, t, "military_tactics"), exhaustion_count: 1.0, gas_attack, gas_defence, ..Default::default() }
                })
                .collect(),
            leader: leader.map(|l| l.id),
            leader_attack: traits.attack,
            leader_defence: traits.defence,
            fort_applies: fort > 0.0 && controller.is_some_and(|ct| rel.hostile(&first, ct)),
            rebel: countries.contains(&self.rebel_tag),
        }
    }

    /// How an attacking army came into the battle province (3.1 crossing): by the border from its
    /// previous province. No previous province means a landing from ships, except for rebels; a
    /// previous province at sea is read as a landing too (not confirmed).
    pub fn entry(&self, a: &Army, to: u16) -> Entry {
        match a.previous {
            None if a.owner == self.rebel_tag => Entry::Plain,
            None => Entry::Landing,
            Some(p) if is_sea(&self.map, p) => Entry::Landing,
            Some(p) => match self.map.border(p, to).map(|b| b.kind) {
                Some(BorderKind::River) => Entry::River,
                Some(BorderKind::Strait) => Entry::Strait,
                _ => Entry::Plain,
            },
        }
    }

    fn land_context(&self, mil: &MilitaryState, state: &State, rel: &Relations, c: &Combat) -> LandContext {
        let p = mil.provinces.get(&c.location).cloned().unwrap_or_default();
        let t = self.terrain.get(&c.location).copied().unwrap_or_default();
        LandContext {
            terrain_defence: t.defence,
            terrain_width: t.width,
            fort: p.fort,
            crossing: battle::crossing_penalty(&c.attacker.units.iter().filter_map(|u| mil.armies.iter().find(|a| a.id == *u)).map(|a| self.entry(a, c.location)).collect::<Vec<_>>()),
            gas_attack_modifier: self.content.gas_attack_modifier,
            attacker: self.side_info(mil, state, rel, c, BattleSide::Attacker, p.controller.as_deref(), p.fort),
            defender: self.side_info(mil, state, rel, c, BattleSide::Defender, p.controller.as_deref(), p.fort),
        }
    }

    /// One land battle of `mil` fought for one day on its own, the dice given where the rules would
    /// roll: the round `land_round` runs, without touching `mil`. Returns the battle, the regiments
    /// and the context after it, for `check-battle` to compare with the original's next save.
    pub fn land_round_alone(&self, mil: &MilitaryState, state: &State, ci: usize, dice: [f64; 2]) -> (Combat, BTreeMap<u64, Fighter>, LandContext, battle::LandReport) {
        let mut c = mil.combats[ci].clone();
        fill_reserve(&mut c, &mil.armies);
        let units: Vec<u64> = c.attacker.units.iter().chain(&c.defender.units).copied().collect();
        let ctx = self.land_context(mil, state, &self.relations(mil, state), &c);
        let mut fighters = self.fighters(mil, state, &units);
        let rep = battle::battle_day_with_dice(&self.battle, &mut c, &mut fighters, &ctx, dice);
        (c, fighters, ctx, rep)
    }

    fn battle_day(&mut self, mil: &mut MilitaryState, state: &State, rel: &Relations, date: Date, cas: &mut BTreeMap<u64, f64>, rep: &mut DayReport) {
        let mut ci = 0;
        while ci < mil.combats.len() {
            let kind = mil.combats[ci].kind;
            if kind == CombatKind::Siege {
                ci += 1;
                continue;
            }
            let outcome = if kind == CombatKind::Land { self.land_round(mil, state, rel, ci, cas) } else { self.naval_round(mil, state, rel, ci) };
            let Some(out) = outcome else {
                ci += 1;
                continue;
            };
            let c = mil.combats.remove(ci);
            self.naval.remove(&c.location);
            rep.battles_finished += 1;
            self.finish_battle(mil, state, rel, date, &c, out, rep);
        }
    }

    fn land_round(&mut self, mil: &mut MilitaryState, state: &State, rel: &Relations, ci: usize, cas: &mut BTreeMap<u64, f64>) -> Option<battle::Outcome> {
        // Armies that no longer exist leave the battle.
        let alive: BTreeSet<u64> = mil.armies.iter().map(|a| a.id).collect();
        for s in [BattleSide::Attacker, BattleSide::Defender] {
            mil.combats[ci].side_mut(s).units.retain(|u| alive.contains(u));
        }
        let mut c = mil.combats[ci].clone();
        let units: Vec<u64> = c.attacker.units.iter().chain(&c.defender.units).copied().collect();
        let ctx = self.land_context(mil, state, rel, &c);
        let mut fighters = self.fighters(mil, state, &units);
        let rep = battle::battle_day(&self.battle, &mut c, &mut fighters, &ctx, &mut self.battle_rng);
        for a in mil.armies.iter_mut().filter(|a| units.contains(&a.id)) {
            for r in &mut a.regiments {
                if let Some(f) = fighters.get(&r.id) {
                    r.strength = f.strength;
                    r.organisation = f.organisation;
                }
            }
            if rep.dig_in_loss && c.defender.units.contains(&a.id) {
                a.dig_in = (a.dig_in - 1.0).max(0.0);
            }
        }
        for cs in &rep.casualties {
            *cas.entry(cs.pop).or_default() += cs.men;
        }
        mil.combats[ci] = c;
        rep.outcome
    }

    fn naval_round(&mut self, mil: &mut MilitaryState, state: &State, rel: &Relations, ci: usize) -> Option<battle::Outcome> {
        let alive: BTreeSet<u64> = mil.navies.iter().map(|n| n.id).collect();
        for s in [BattleSide::Attacker, BattleSide::Defender] {
            mil.combats[ci].side_mut(s).units.retain(|u| alive.contains(u));
        }
        let mut c = mil.combats[ci].clone();
        let loc = c.location;
        let ctx = self.land_context(mil, state, rel, &c);
        if !self.naval.contains_key(&loc) {
            let mut ships = Vec::new();
            for side in [BattleSide::Attacker, BattleSide::Defender] {
                for nid in &c.side(side).units {
                    let Some(n) = mil.navies.iter().find(|n| n.id == *nid) else { continue };
                    for s in &n.ships {
                        let def = self.unit(&s.kind);
                        let class = if def.transport || def.kind == "transport" { ShipClass::Transport } else if def.capital || def.kind == "big_ship" { ShipClass::Big } else { ShipClass::Light };
                        ships.push(NavalShip::new(s.id, n.id, side, class, &def, s.strength, s.organisation, s.experience));
                    }
                }
            }
            self.naval.insert(loc, NavalState { ships, fleeing: [false, false], attacker: Default::default(), defender: Default::default() });
        } else {
            // A fleet that joined later brings its ships in.
            let st = self.naval.get_mut(&loc).expect("checked above");
            for side in [BattleSide::Attacker, BattleSide::Defender] {
                for nid in &c.side(side).units {
                    if st.ships.iter().any(|x| x.fleet == *nid) {
                        continue;
                    }
                    let Some(n) = mil.navies.iter().find(|n| n.id == *nid) else { continue };
                    for s in &n.ships {
                        let def = self.units.get(&s.kind).cloned().unwrap_or_default();
                        let class = if def.transport { ShipClass::Transport } else if def.capital { ShipClass::Big } else { ShipClass::Light };
                        st.ships.push(NavalShip::new(s.id, n.id, side, class, &def, s.strength, s.organisation, s.experience));
                    }
                }
            }
        }
        let st = self.naval.get_mut(&loc).expect("inserted above");
        let rep = battle::naval_day(&self.battle, &mut c, st, &ctx, &mut self.battle_rng);
        for n in mil.navies.iter_mut() {
            for s in &mut n.ships {
                if let Some(x) = st.ships.iter().find(|x| x.id == s.id && x.fleet == n.id) {
                    s.strength = x.strength;
                    s.organisation = x.organisation;
                }
            }
        }
        let sunk: BTreeSet<u64> = rep.sunk.iter().copied().collect();
        for n in mil.navies.iter_mut() {
            n.ships.retain(|s| !sunk.contains(&s.id) || s.strength > 0.0);
        }
        mil.combats[ci] = c;
        rep.outcome
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_battle(&mut self, mil: &mut MilitaryState, state: &State, rel: &Relations, date: Date, c: &Combat, out: battle::Outcome, rep: &mut DayReport) {
        let naval = c.kind == CombatKind::Naval;
        let (ac, dc) = (side_countries(mil, c, BattleSide::Attacker), side_countries(mil, c, BattleSide::Defender));
        let res = &out.result;
        for (tag, p) in &res.prestige {
            rep.prestige.push((tag.clone(), *p));
        }
        for (tag, x) in &res.exhaustion {
            if let Some(co) = mil.countries.get_mut(tag) {
                co.war_exhaustion = (co.war_exhaustion + x).clamp(0.0, self.battle.war_exhaustion_max);
            }
        }
        for (lid, p) in &res.leader_prestige {
            if let Some(l) = mil.leaders.iter_mut().find(|l| l.id == *lid) {
                l.prestige = (l.prestige + p).clamp(0.0, 100.0);
            }
        }
        // The battle goes into the war that has the two sides against each other.
        if let (Some(a), Some(d)) = (ac.first(), dc.first())
            && let Some(w) = mil.wars.iter_mut().find(|w| w.active && ac.iter().any(|x| (w.attackers.contains(x) && w.defenders.contains(d)) || (w.defenders.contains(x) && w.attackers.contains(d))))
        {
            let rec = &res.record;
            w.history.push(war::WarEvent {
                date,
                kind: war::WarEventKind::Battle(war::BattleRecord {
                    date: Some(date),
                    name: String::new(),
                    location: c.location,
                    attacker_won: rec.attacker_won,
                    attacker: war::BattleSide { country: a.clone(), leader: String::new(), forces: BTreeMap::new(), losses: rec.attacker_losses },
                    defender: war::BattleSide { country: d.clone(), leader: String::new(), forces: BTreeMap::new(), losses: rec.defender_losses },
                }),
            });
            w.action = Some(date);
        }
        // The pops behind a beaten rebel army calm down (REDUCTION_AFTER_DEFEAT, applied by the caller).
        if !naval {
            let beaten: BTreeSet<u64> = mil.armies.iter().filter(|a| a.owner == self.rebel_tag && (out.destroyed.contains(&a.id) || out.retreating.contains(&a.id))).flat_map(|a| a.regiments.iter().filter_map(|r| r.pop)).collect();
            rep.rebels_beaten.extend(beaten);
        }
        for id in &out.destroyed {
            if naval {
                mil.navies.retain(|n| n.id != *id);
                for a in mil.armies.iter_mut().filter(|a| a.carried_by == Some(*id)) {
                    a.regiments.clear();
                }
                rep.fleets_destroyed += 1;
            } else {
                self.remove_army(mil, *id);
                rep.armies_destroyed += 1;
            }
        }
        for id in out.retreating {
            if naval {
                self.retreat_fleet(mil, id);
            } else if !self.retreat_army(mil, state, rel, id) {
                self.remove_army(mil, id);
                rep.armies_destroyed += 1;
            }
        }
        mil.armies.retain(|a| !a.regiments.is_empty());
    }

    /// Where a beaten army goes; false means nowhere, the army is deleted.
    fn retreat_army(&mut self, mil: &mut MilitaryState, _state: &State, rel: &Relations, id: u64) -> bool {
        let Some(a) = mil.armies.iter().find(|a| a.id == id) else { return true };
        let (owner, from) = (a.owner.clone(), a.location);
        let orgs: Vec<f64> = a.regiments.iter().map(|r| r.organisation).collect();
        let unit = battle::RetreatUnit { owner: owner.clone(), from, mean_organisation: if orgs.is_empty() { 0.0 } else { orgs.iter().sum::<f64>() / orgs.len() as f64 }, at_sea: a.carried_by.is_some(), rebel_can_retreat: true };
        let ctx = self.border_ctx(mil, rel, &owner);
        let hostile_armies: BTreeSet<ProvinceId> = mil.armies.iter().filter(|x| x.id != id && !x.regiments.is_empty() && rel.hostile(&owner, &x.owner) && !self.retreating.contains(&x.id)).map(|x| x.location).collect();
        let mut exposed = BTreeSet::new();
        for n in self.map.neighbours(from).collect::<Vec<_>>() {
            let ctl = mil.provinces.get(&n).and_then(|p| p.controller.clone());
            if let Some(ctl) = ctl && self.map.neighbours(n).any(|m| mil.provinces.get(&m).and_then(|p| p.controller.as_deref()).is_some_and(|mc| rel.hostile(&ctl, mc))) {
                exposed.insert(n);
            }
        }
        let room = BTreeSet::new();
        let facts = battle::RetreatFacts { map: &self.map, border_ctx: &ctx, relations: rel, provinces: &mil.provinces, hostile_armies: &hostile_armies, exposed: &exposed, sea_room: &room };
        match battle::retreat_target(&unit, &facts, &mut self.battle_rng) {
            Some(p) => {
                let a = mil.armies.iter_mut().find(|a| a.id == id).expect("found above");
                a.path = vec![p];
                a.movement_progress = 0.0;
                a.target = None;
                self.retreating.insert(id);
                true
            }
            None => false,
        }
    }

    /// A fleet that got away goes to the nearest own port with a naval base, else stays and is not
    /// flagged (so it does not fight the same day again from the same spot).
    fn retreat_fleet(&mut self, mil: &mut MilitaryState, id: u64) {
        let Some(n) = mil.navies.iter().find(|n| n.id == id) else { return };
        let (owner, from) = (n.owner.clone(), n.location);
        let mut ports: Vec<(f64, u16)> = mil
            .provinces
            .iter()
            .filter(|(_, p)| p.naval_base > 0.0 && p.owner.as_deref() == Some(owner.as_str()) && p.controller.as_deref() == Some(owner.as_str()))
            .map(|(pid, _)| (self.map.border(from, *pid).map_or(f64::MAX / 4.0, |b| b.distance), *pid))
            .collect();
        ports.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, port) in ports.into_iter().take(6) {
            let ctx = BorderContext::default();
            if let Some(path) = movement::find_path(&self.map, from, port, &|a, b| !movement::border_closed(&self.map, &ctx, true, a, b)) && !path.is_empty() {
                let n = mil.navies.iter_mut().find(|n| n.id == id).expect("found above");
                n.path = path;
                n.movement_progress = 0.0;
                self.retreating.insert(id | NAVY_BIT);
                return;
            }
        }
    }

    // ------------------------------------------------------------------ 5. sieges

    /// End of the day (and before the first day after a load): remember every land regiment's
    /// strength for tomorrow's siege step.
    fn keep_siege_men(&mut self, mil: &MilitaryState) {
        self.siege_men.clear();
        self.siege_men.extend(mil.armies.iter().flat_map(|a| a.regiments.iter().map(|r| (r.id, r.strength))));
    }

    /// The strength a regiment had at the end of the day before, as the siege step books it (None:
    /// it did not exist then, and the siege takes today's strength).
    pub fn yesterday_strength(&self, regiment: u64) -> Option<f64> {
        self.siege_men.get(&regiment).copied()
    }

    fn siege_day(&mut self, mil: &mut MilitaryState, state: &State, rel: &Relations, date: Date, rep: &mut DayReport) {
        let mut ci = 0;
        while ci < mil.combats.len() {
            if mil.combats[ci].kind != CombatKind::Siege {
                ci += 1;
                continue;
            }
            let loc = mil.combats[ci].location;
            let alive: BTreeSet<u64> = mil.armies.iter().map(|a| a.id).collect();
            mil.combats[ci].attacker.units.retain(|u| alive.contains(u));
            mil.combats[ci].defender.units.retain(|u| alive.contains(u));
            mil.combats[ci].duration += 1.0;
            if mil.combats[ci].attacker.units.is_empty() {
                mil.combats.remove(ci);
                rep.sieges_ended += 1;
                continue;
            }
            // Nothing happens while the province holds a battle.
            if mil.combats.iter().any(|c| c.kind == CombatKind::Land && c.location == loc) {
                ci += 1;
                continue;
            }
            let mut besiegers: Vec<Besieger> = Vec::new();
            for u in mil.combats[ci].attacker.units.clone() {
                let Some(a) = mil.armies.iter().find(|a| a.id == u) else { continue };
                if a.location != loc || !a.path.is_empty() || self.retreating.contains(&a.id) || army_strength(a) <= 0.001 {
                    continue;
                }
                let regs: Vec<SiegeRegiment> = a.regiments.iter().map(|r| { let d = self.unit(&r.kind); SiegeRegiment { strength: self.siege_men.get(&r.id).copied().unwrap_or(r.strength), siege: d.siege, recon: d.reconnaissance } }).collect();
                match besiegers.iter_mut().find(|b| b.country == a.owner) {
                    Some(b) => b.regiments.extend(regs),
                    None => besiegers.push(Besieger { country: a.owner.clone(), regiments: regs }),
                }
            }
            let cores = state.provinces.get(&loc).map(|p| p.cores.clone()).unwrap_or_default();
            let hostile = |a: &str, b: &str| rel.hostile(a, b);
            let core_of = |t: &str| cores.iter().any(|c| c == t);
            let overlord = |t: &str| rel.overlord.get(t).cloned();
            let sctx = SiegeContext { hostile: &hostile, core_of: &core_of, overlord: &overlord };
            let Some(p) = mil.provinces.get_mut(&loc) else {
                mil.combats.remove(ci);
                continue;
            };
            let before = p.controller.clone();
            match siege::siege_day(&self.siege, p, &besiegers, &sctx, &mut self.siege_rng, date) {
                SiegeEvent::Continues(_) => ci += 1,
                SiegeEvent::Ended => {
                    mil.combats.remove(ci);
                    rep.sieges_ended += 1;
                }
                SiegeEvent::Captured { new_controller, .. } => {
                    rep.controller_changes.push(ControllerChange { province: loc, from: before, to: Some(new_controller) });
                    ci += 1;
                }
            }
        }
        // Garrisons of provinces nobody besieges grow back.
        let besieged: BTreeSet<u16> = mil.combats.iter().filter(|c| c.kind == CombatKind::Siege).map(|c| c.location).collect();
        for (id, p) in mil.provinces.iter_mut() {
            siege::garrison_regrow_day(&self.siege, p, besieged.contains(id));
        }
    }

    // ------------------------------------------------------------------ monthly

    fn attrition(&mut self, mil: &mut MilitaryState, state: &State, rel: &Relations, army: u64, cas: &mut BTreeMap<u64, f64>) {
        let Some(a) = mil.armies.iter().find(|a| a.id == army) else { return };
        if a.carried_by.is_some() || Self::in_combat(mil, true, army, Some(false)) {
            return;
        }
        let (owner, loc) = (a.owner.clone(), a.location);
        if is_sea(&self.map, loc) {
            return;
        }
        let prov = mil.provinces.get(&loc).cloned().unwrap_or_default();
        let t = self.terrain.get(&loc).copied().unwrap_or_default();
        let in_siege = mil.combats.iter().any(|c| c.kind == CombatKind::Siege && c.attacker.units.contains(&army));
        let own_only = prov.owner.as_deref() == Some(owner.as_str()) && !self.retreating.contains(&army);
        let weight: f64 = mil.armies.iter().filter(|x| x.location == loc && x.carried_by.is_none() && (!own_only || x.owner == owner)).map(army_strength).sum();
        let limit = self.supply_limit(state, &owner, loc, self.supply_access(mil, state, rel, &owner, loc));
        let enemy_fort = self.map.neighbours(loc).filter_map(|n| mil.provinces.get(&n)).filter(|p| p.controller.as_deref().is_some_and(|c| rel.hostile(&owner, c))).map(|p| p.fort).fold(0.0, f64::max);
        let tech_attr = -self.tech_sum(state, &owner, "land_attrition");
        let pct = logistics::land_attrition_percent(&self.logistics, &logistics::LandAttrition { exiled: false, weight, leader_trait: 0.0, tech_attrition: tech_attr.max(-1.0), supply_limit: limit, enemy_fort, province_attrition: 0.0, province_max_attrition: t.max_attrition, in_siege });
        if pct <= 0.0 {
            return;
        }
        let a = mil.armies.iter_mut().find(|a| a.id == army).expect("found above");
        for r in &mut a.regiments {
            let def = self.units.get(&r.kind).cloned().unwrap_or_default();
            let max = if def.max_strength > 0.0 { def.max_strength } else { 3.0 };
            let loss = logistics::attrition_loss(max, pct).min(r.strength);
            r.strength -= loss;
            if let Some(pop) = r.pop {
                *cas.entry(pop).or_default() += loss / max * self.battle.pop_size_per_regiment * self.battle.soldier_to_pop_damage;
            }
        }
    }

    fn month(&mut self, mil: &mut MilitaryState, state: &State, rel: &Relations, cas: &mut BTreeMap<u64, f64>, rep: &mut DayReport) {
        // Attrition.
        let ids: Vec<u64> = mil.armies.iter().map(|a| a.id).collect();
        for id in &ids {
            self.attrition(mil, state, rel, *id, cas);
        }
        self.naval_attrition(mil, state, rel);
        rep.armies_destroyed += self.fleet_losses(mil);
        // Reinforcement.
        self.reinforce(mil, state, rel);
        // A regiment that is down to nothing is gone, and an army without regiments with it.
        let floor = self.logistics.delete_below_strength;
        for a in &mut mil.armies {
            a.regiments.retain(|r| r.strength >= floor);
        }
        mil.armies.retain(|a| !a.regiments.is_empty());
        // Leadership points.
        self.leadership_month(mil, state);
        // War exhaustion.
        self.exhaustion_month(mil, state, rel);
    }

    fn naval_attrition(&mut self, mil: &mut MilitaryState, state: &State, rel: &Relations) {
        let ids: Vec<u64> = mil.navies.iter().map(|n| n.id).collect();
        for id in ids {
            let Some(n) = mil.navies.iter().find(|n| n.id == id) else { continue };
            if Self::in_combat(mil, false, id, None) {
                continue;
            }
            let (owner, loc) = (n.owner.clone(), n.location);
            let in_port = !is_sea(&self.map, loc);
            let own_land = |p: u16| mil.provinces.get(&p).is_some_and(|m| m.controller.as_deref() == Some(owner.as_str()));
            let next_to_own_land = self.map.neighbours(loc).any(|p| !is_sea(&self.map, p) && own_land(p));
            let range = logistics::fleet_supply_range(&self.logistics, self.tech_sum(state, &owner, "supply_range"));
            let in_supply_range = !in_port && self.port_within(mil, loc, &owner, range);
            let months = self.naval_months.entry(id).or_insert(0.0);
            let pct = logistics::naval_attrition_percent(&self.logistics, &logistics::NavalAttrition { in_port, next_to_own_land, in_supply_range, months_out_of_supply: *months, leader_trait: 0.0, tech_attrition: 0.0 });
            if in_port {
                *months = 0.0;
            } else if !(next_to_own_land || in_supply_range) {
                *months += 1.0;
            }
            let dice = self.battle_rng.next_u64();
            let n = mil.navies.iter_mut().find(|n| n.id == id).expect("found above");
            for s in &mut n.ships {
                if logistics::ship_hit(s.id, dice, pct) {
                    let def = self.units.get(&s.kind).cloned().unwrap_or_default();
                    s.strength -= logistics::attrition_loss(if def.max_strength > 0.0 { def.max_strength } else { 1.0 }, pct).min(s.strength);
                }
            }
            n.ships.retain(|s| !logistics::ship_sinks(&self.logistics, s.strength));
        }
        let _ = rel;
    }

    /// Is an own, controlled port with a naval base within `range` of a sea province (shortest distance over the borders).
    fn port_within(&self, mil: &MilitaryState, from: u16, owner: &str, range: f64) -> bool {
        let mut best: BTreeMap<u16, f64> = BTreeMap::from([(from, 0.0)]);
        let mut todo = vec![(0.0, from)];
        while let Some((d, p)) = todo.pop() {
            if best.get(&p).is_some_and(|b| *b < d) {
                continue;
            }
            if mil.provinces.get(&p).is_some_and(|m| m.naval_base > 0.0 && m.owner.as_deref() == Some(owner) && m.controller.as_deref() == Some(owner)) {
                return true;
            }
            for n in self.map.neighbours(p) {
                let nd = d + self.map.border(p, n).map_or(f64::MAX / 4.0, |b| b.distance);
                if nd <= range && best.get(&n).is_none_or(|b| nd < *b) {
                    best.insert(n, nd);
                    todo.push((nd, n));
                }
            }
        }
        false
    }

    fn reinforce(&mut self, mil: &mut MilitaryState, state: &State, rel: &Relations) {
        let mut pops: BTreeMap<u64, (u16, String, f64)> = BTreeMap::new();
        for (id, p) in &state.provinces {
            for q in &p.pops {
                pops.insert(q.id, (*id, q.pop_type.clone(), q.size));
            }
        }
        let mut has: BTreeMap<u64, u32> = BTreeMap::new();
        for r in mil.armies.iter().flat_map(|a| &a.regiments) {
            if let Some(p) = r.pop {
                *has.entry(p).or_default() += 1;
            }
        }
        let rate_tech = |tag: &str| self.tech_sum(state, tag, "reinforce_rate");
        for i in 0..mil.armies.len() {
            let a = &mil.armies[i];
            if a.carried_by.is_some() || self.retreating.contains(&a.id) || Self::in_combat(mil, true, a.id, Some(false)) {
                continue;
            }
            let (owner, loc) = (a.owner.clone(), a.location);
            let prov = mil.provinces.get(&loc).cloned().unwrap_or_default();
            let coastal_free = self.map.provinces.get(&loc).is_some_and(|m| m.port_sea.is_some());
            let place = if prov.owner.as_deref() == Some(owner.as_str()) {
                logistics::ReinforcePlace::Owned
            } else if owner == self.rebel_tag {
                logistics::ReinforcePlace::Rebel { reinforcing: false, rebel_controlled: prov.controller.as_deref() == Some(self.rebel_tag.as_str()) }
            } else if prov.owner.is_none() {
                logistics::ReinforcePlace::Unowned { coastal_free }
            } else if prov.controller.as_deref().is_none_or(|c| c == owner || !rel.hostile(&owner, c)) {
                logistics::ReinforcePlace::FriendlyOrNeutral
            } else {
                let friendly_land = self.map.neighbours(loc).any(|n| mil.provinces.get(&n).and_then(|p| p.owner.as_deref()).is_some_and(|o| rel.friendly(&owner, o)));
                logistics::ReinforcePlace::Hostile { friendly_land_neighbour: friendly_land, coastal_free }
            };
            let rate = logistics::reinforce_rate(&self.logistics, rate_tech(&owner), logistics::reinforce_location_factor(place), 0.0);
            let mut refills: Vec<logistics::Refill> = Vec::new();
            for r in &mil.armies[i].regiments {
                let def = self.units.get(&r.kind).cloned().unwrap_or_default();
                let share = match r.pop.and_then(|p| pops.get(&p).map(|x| (p, x))) {
                    Some((p, (pid, ty, size))) => {
                        let status = match state.provinces.get(pid) {
                            Some(s) if s.colonial > 0 => logistics::PopStatus::Colony,
                            Some(s) if !s.cores.contains(&owner) => logistics::PopStatus::NonCore,
                            _ => logistics::PopStatus::Normal,
                        };
                        let supports = logistics::regiments_supported(&self.logistics, *size, status, false);
                        logistics::pop_refill_share(&self.logistics, *ty == self.soldier_type, supports, has.get(&p).copied().unwrap_or(1))
                    }
                    None => self.logistics.mobilised_refill_share,
                };
                refills.push(logistics::Refill { strength: r.strength, max_strength: if def.max_strength > 0.0 { def.max_strength } else { 3.0 }, experience: r.experience, share });
            }
            let status = if owner == self.rebel_tag { 1.0 } else { self.supply_status };
            logistics::reinforce(rate, status, &mut refills);
            for (r, f) in mil.armies[i].regiments.iter_mut().zip(&refills) {
                r.strength = f.strength;
                r.experience = f.experience;
            }
        }
    }

    fn leadership_month(&mut self, mil: &mut MilitaryState, state: &State) {
        let mut people: BTreeMap<String, (f64, BTreeMap<&str, f64>)> = BTreeMap::new();
        for p in state.provinces.values() {
            let Some(owner) = &p.owner else { continue };
            let e = people.entry(owner.clone()).or_default();
            for q in &p.pops {
                e.0 += q.size;
                if self.leadership.contains_key(&q.pop_type) {
                    *e.1.entry(q.pop_type.as_str()).or_default() += q.size;
                }
            }
        }
        for (tag, (total, by_type)) in people {
            let Some(c) = mil.countries.get_mut(&tag) else { continue };
            if total <= 0.0 {
                continue;
            }
            let types: Vec<(f64, f64, f64)> = by_type.iter().filter_map(|(t, n)| self.leadership.get(*t).map(|(l, opt)| (*l, n / total, *opt))).collect();
            let gain = logistics::leadership_points(&types, 0.0, 0.0);
            c.leadership = logistics::add_leadership(&self.logistics, c.leadership, gain);
        }
    }

    fn exhaustion_month(&mut self, mil: &mut MilitaryState, state: &State, rel: &Relations) {
        let tags: Vec<String> = mil.countries.keys().filter(|t| **t != self.rebel_tag).cloned().collect();
        for tag in tags {
            let at_war = mil.active_wars_of(&tag).next().is_some();
            let current = mil.countries[&tag].war_exhaustion;
            let (mut occupied_share, mut blockade_share) = (0.0, 0.0);
            if at_war {
                let capital_mass = state.countries.get(&tag).and_then(|c| c.capital).and_then(|c| state.land_mass.get(&c)).copied();
                let homes: Vec<siege::HomeProvince> = mil
                    .provinces
                    .iter()
                    .filter(|(_, p)| p.owner.as_deref() == Some(tag.as_str()))
                    .map(|(id, p)| {
                        let port_sea = self.map.provinces.get(id).and_then(|m| m.port_sea);
                        let fleets: Vec<siege::BlockadeUnit> = port_sea.map(|s| mil.navies.iter().filter(|n| n.location == s).map(|n| siege::BlockadeUnit { owner: n.owner.clone(), ships: n.ships.len(), in_battle: Self::in_combat(mil, false, n.id, None), retreating: self.retreating.contains(&(n.id | NAVY_BIT)) }).collect()).unwrap_or_default();
                        let controller = p.controller.clone().unwrap_or_else(|| tag.clone());
                        siege::HomeProvince { owner: tag.clone(), blockaded: !fleets.is_empty() && siege::port_blockaded(&fleets, &controller, &|a, b| rel.hostile(a, b)), controller, port: port_sea.is_some(), home_continent: capital_mass.is_none_or(|m| state.land_mass.get(id) == Some(&m)) }
                    })
                    .collect();
                let s = siege::occupation_share(&tag, &homes, &self.rebel_tag);
                (occupied_share, blockade_share) = (s.occupied, s.blockaded);
            }
            let next = war::exhaustion_monthly(&self.war, &war::ExhaustionInput { current, at_war, occupied_share, blockade_share, modifier_change: 0.0, modifier_max: None });
            if let Some(c) = mil.countries.get_mut(&tag) {
                c.war_exhaustion = next;
            }
        }
    }
}

/// Fleets and armies share id numbers in the transient maps of `MilitaryDay`; the top bit marks a fleet.
const NAVY_BIT: u64 = 1 << 62;
