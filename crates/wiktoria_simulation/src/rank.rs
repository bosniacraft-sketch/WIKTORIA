//! Scores, the daily ranking and great-power status.
//!
//! Every day each country that owns land gets three scores, added without weights:
//! * industrial: `State::industrial_score` (factory levels by how well the state could staff
//!   them) plus 0.01 x INVESTMENT_SCORE_FACTOR x the money it has invested in every other
//!   land-owning country;
//! * military: R x A x S, cut to a whole number, / 7, plus capital ships, plus leaders. R =
//!   the regiments its soldier pops could support, at most 4 x the regiments it has (mobilised
//!   regiments do not count while it is mobilised); A = the average over its land unit types
//!   of (attack + defence + its land attack and defence modifiers) x discipline; S = 1 + its
//!   supply-consumption modifier, at least 0.001. A capital ship adds (gun power + naval attack
//!   modifier) x (hull + naval defence modifier) / 250. Leaders add min(leaders, regiments);
//! * prestige: prestige plus the permanent prestige of its inventions.
//!
//! The ranking puts civilised countries first, then the larger total. The first
//! GREAT_NATIONS_COUNT civilised countries with two or more states are great powers; a great
//! power pushed out of that band keeps its seat for GREATNESS_DAYS, one that is no longer
//! eligible loses it at once, in both cases only if there is a candidate to take the seat.
//! A country that is a great power has no overlord, no sphere leader, and nobody has influence
//! in it; one that stops being one loses its sphere and its influence inside it.
//!
//! Not in the port (it has no wars of its own making, no crisis, no disarmament): a great power
//! also keeps its seat while it fights a war with two great powers on each side, a great war, or
//! takes part in the crisis; a disarmed country's R is multiplied by DISARMAMENT_ARMY_HIT; the
//! events and messages on a change of seat; opinion levels (the port's `State` has none).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use wiktoria_data::defines::Defines;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::modifiers::ModifierLibrary;
use wiktoria_data::save::SaveGame;
use wiktoria_data::script::{self, Block, Value};
use wiktoria_data::world::World;

use crate::calendar::Calendar;
use crate::script::State;

// rules file: FLD_MILITARY_SCORE_LAND_DIVISOR - the land part of the military score is divided by this.
const MILITARY_LAND_DIVISOR: f64 = 7.0;
// rules file: FLD_MILITARY_SCORE_REGIMENT_CAP - supportable regiments count up to this many times the regiments in being.
const MILITARY_REGIMENT_CAP: f64 = 4.0;
// rules file: FLD_MILITARY_SCORE_MIN_SUPPLY - lowest value of 1 + supply consumption in the military score.
const MILITARY_MIN_SUPPLY: f64 = 0.001;
// rules file: FLD_MILITARY_SCORE_SHIP_DIVISOR - a capital ship scores gun power x hull / this.
const CAPITAL_SHIP_DIVISOR: f64 = 250.0;
// rules file: FLD_INVESTMENT_SCORE_SHARE - share of money invested abroad that INVESTMENT_SCORE_FACTOR is applied to.
const INVESTMENT_SCORE_SHARE: f64 = 0.01;
// rules file: FLD_GREAT_POWER_MIN_STATES - states a country needs to be a great power.
const GREAT_POWER_MIN_STATES: usize = 2;

/// Statistics of a unit type that the scores use: attack, defence, discipline, hull, gun power.
pub type UnitStats = [f64; 5];
const STAT_KEYS: [&str; 5] = ["attack", "defence", "discipline", "hull", "gun_power"];
const ATTACK: usize = 0;
const DEFENCE: usize = 1;
const DISCIPLINE: usize = 2;
const HULL: usize = 3;
const GUN_POWER: usize = 4;
/// The two names a technology uses to change every land or every naval unit type.
const ALL_LAND: &str = "army_base";
const ALL_NAVAL: &str = "navy_base";

#[derive(Debug, Clone, PartialEq, Default)]
pub struct UnitType {
    pub name: String,
    /// A land unit (`type = land`); otherwise a ship.
    pub land: bool,
    /// `capital = yes`: a capital ship.
    pub capital: bool,
    pub stats: UnitStats,
}

/// What a technology or invention does to unit types.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UnitEffects {
    /// Unit type name (or `army_base` / `navy_base`) -> what it adds to the statistics.
    pub changes: Vec<(String, UnitStats)>,
}

fn stats(b: &Block) -> UnitStats {
    STAT_KEYS.map(|k| b.get(k).and_then(Value::as_f64).unwrap_or(0.0))
}

fn unit_effects(b: &Block, names: &BTreeSet<&str>) -> UnitEffects {
    let mut out = UnitEffects::default();
    for (k, v, _) in b.entries() {
        match v {
            Value::Block(x) if names.contains(k) || k == ALL_LAND || k == ALL_NAVAL => out.changes.push((k.to_string(), stats(x))),
            _ => {}
        }
    }
    out
}

fn read(path: &Path) -> Result<script::Document, String> {
    script::parse_bytes(&std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?).map_err(|e| format!("{}: {e}", path.display()))
}

fn text_files(dir: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let mut out: Vec<_> = std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "txt")).collect();
    out.sort();
    Ok(out)
}

/// What the scores and the ranking need from the install.
#[derive(Debug, Clone, PartialEq)]
pub struct RankRules {
    pub units: Vec<UnitType>,
    /// Technology name -> what it does to unit types.
    pub tech_units: BTreeMap<String, UnitEffects>,
    /// The same for every invention, in the order a save numbers them.
    pub invention_units: Vec<UnitEffects>,
    /// Pop types regiments are raised from in peace (`can_be_recruited`).
    pub soldier_types: BTreeSet<String>,
    /// Names of the static modifier blocks of a country at peace and at war.
    pub peace_block: String,
    pub war_block: String,
    pub calendar: Calendar,
    pub great_nations: f64,
    pub greatness_days: f64,
    pub colonial_rank: f64,
    pub investment_score_factor: f64,
    /// POP_MIN_SIZE_FOR_REGIMENT, POP_SIZE_PER_REGIMENT and the multipliers of the latter in a
    /// protectorate, a colony and a province that is no core of its owner.
    pub regiment_min_pop: f64,
    pub regiment_pop: f64,
    pub regiment_protectorate: f64,
    pub regiment_colony: f64,
    pub regiment_noncore: f64,
}

impl RankRules {
    /// The rules with the given calendar and defines, and nothing from the install's files.
    pub fn from_defines(calendar: Calendar, defines: &Defines) -> Result<RankRules, String> {
        let get = |section: &str, key: &str| defines.get(section, key).ok_or(format!("defines: {key} missing"));
        Ok(RankRules {
            units: Vec::new(),
            tech_units: BTreeMap::new(),
            invention_units: Vec::new(),
            soldier_types: BTreeSet::new(),
            peace_block: String::new(),
            war_block: String::new(),
            calendar,
            great_nations: get("country", "GREAT_NATIONS_COUNT")?,
            greatness_days: get("country", "GREATNESS_DAYS")?,
            colonial_rank: get("country", "COLONIAL_RANK")?,
            investment_score_factor: get("country", "INVESTMENT_SCORE_FACTOR")?,
            regiment_min_pop: get("military", "POP_MIN_SIZE_FOR_REGIMENT")?,
            regiment_pop: get("military", "POP_SIZE_PER_REGIMENT")?,
            regiment_protectorate: get("military", "POP_MIN_SIZE_FOR_REGIMENT_PROTECTORATE_MULTIPLIER")?,
            regiment_colony: get("military", "POP_MIN_SIZE_FOR_REGIMENT_COLONY_MULTIPLIER")?,
            regiment_noncore: get("military", "POP_MIN_SIZE_FOR_REGIMENT_NONCORE_MULTIPLIER")?,
        })
    }

    /// Reads the unit types, what technologies and inventions do to them, and which pop types
    /// can be recruited.
    pub fn new(install: &Path, m: &EngineRules, world: &World) -> Result<RankRules, String> {
        let mut rules = RankRules::from_defines(Calendar::from_engine_rules(m)?, &world.defines)?;
        for file in text_files(&install.join(m.require_str("FLD_UNITS_DIR")?))? {
            for (name, v, _) in read(&file)?.root.entries() {
                let Some(b) = v.as_block() else { continue };
                let text = |k: &str| b.get(k).and_then(Value::as_str);
                rules.units.push(UnitType { name: name.to_string(), land: text("type") == Some("land"), capital: text("capital") == Some("yes"), stats: stats(b) });
            }
        }
        let names: BTreeSet<&str> = rules.units.iter().map(|u| u.name.as_str()).collect();
        for file in text_files(&install.join(m.require_str("FLD_TECHNOLOGY_DIR")?))? {
            for (name, v, _) in read(&file)?.root.entries() {
                if let Some(b) = v.as_block() {
                    rules.tech_units.insert(name.to_string(), unit_effects(b, &names));
                }
            }
        }
        rules.invention_units = world.defs.invention_blocks.iter().map(|b| b.get("effect").and_then(Value::as_block).map(|e| unit_effects(e, &names)).unwrap_or_default()).collect();
        for t in &world.defs.pop_types {
            let path = install.join(m.require_str("FLD_POPTYPES_DIR")?).join(format!("{}.txt", t.name));
            if read(&path).is_ok_and(|d| d.root.get("can_be_recruited").and_then(Value::as_bool) == Some(true)) {
                rules.soldier_types.insert(t.name.clone());
            }
        }
        let blocks = crate::modifiers::CountryModifierRules::from_engine_rules(m)?;
        (rules.peace_block, rules.war_block) = (blocks.peace_block, blocks.war_block);
        Ok(rules)
    }

    /// Whole days from the calendar's first day to `d`.
    fn day_number(&self, d: wiktoria_data::date::Date) -> Option<i64> {
        self.calendar.to_game(d).map(|g| g.0.div_euclid(24))
    }
}

/// Loads from a save what the military score and the seats need and `State::from_save` leaves
/// out: per country the regiments (by the type of the pop behind them), the ships (by unit
/// type), the number of leaders, whether it is mobilised, the last day it ranked as a great
/// power, and the money other countries have invested in it.
pub fn attach_forces(state: &mut State, save: &SaveGame) {
    let pop_types: BTreeMap<u64, &str> = save.provinces.values().flat_map(|p| &p.pops).map(|q| (q.id, q.pop_type.as_str())).collect();
    for tag in &save.countries {
        let (Some(b), Some(c)) = (save.doc.root.get(tag).and_then(Value::as_block), state.countries.get_mut(tag)) else { continue };
        (c.regiments, c.ships) = (BTreeMap::new(), BTreeMap::new());
        let navies: Vec<&Block> = b.get_all("navy").filter_map(Value::as_block).collect();
        // An army on board lies inside the navy that carries it.
        for army in b.get_all("army").chain(navies.iter().flat_map(|n| n.get_all("army"))).filter_map(Value::as_block) {
            for r in army.get_all("regiment").filter_map(Value::as_block) {
                let pop = r.get("pop").and_then(Value::as_block).and_then(|p| p.get("id")).and_then(Value::as_f64).and_then(|id| pop_types.get(&(id as u64)));
                *c.regiments.entry(pop.copied().unwrap_or("").to_string()).or_default() += 1.0;
            }
        }
        for ship in navies.iter().flat_map(|n| n.get_all("ship")).filter_map(Value::as_block) {
            if let Some(kind) = ship.get("type").and_then(Value::as_str) {
                *c.ships.entry(kind.to_string()).or_default() += 1.0;
            }
        }
        c.leaders = b.get_all("leader").filter(|l| l.as_block().is_some()).count() as f64;
        c.mobilized = b.get("mobilize").and_then(Value::as_bool) == Some(true);
        c.last_greatness = b.get("last_greatness_date").and_then(Value::as_str).and_then(wiktoria_data::date::Date::parse);
        // One amount per country number (slot 0 unused); the country's own slot is not foreign.
        // The save writes plain money: a save with 37,834 invested shows 1.88 score in the original.
        let amounts = b.get("foreign_investment").and_then(Value::as_block).map(|x| x.bare_scalars()).unwrap_or_default();
        c.foreign_investment = amounts.iter().enumerate().filter_map(|(i, v)| Some((save.countries.get(i.checked_sub(1)?)?.clone(), v.parse::<f64>().ok().filter(|x| *x > 0.0)?))).filter(|(t, _)| t != tag).collect();
    }
}

/// Regiments one soldier pop can support: none below POP_MIN_SIZE_FOR_REGIMENT, otherwise
/// size / (POP_SIZE_PER_REGIMENT x multiplier) in whole numbers, plus one.
pub fn possible_regiments(rules: &RankRules, size: f64, multiplier: f64) -> f64 {
    if size < rules.regiment_min_pop { 0.0 } else { (size / (rules.regiment_pop * multiplier)).floor() + 1.0 }
}

/// What money invested abroad adds to the industrial score: 0.01 x INVESTMENT_SCORE_FACTOR x the money.
pub fn investment_score(rules: &RankRules, money: f64) -> f64 {
    money * INVESTMENT_SCORE_SHARE * rules.investment_score_factor
}

/// The money a country has invested in each other country that owns land, as (host, money).
pub fn invested_abroad<'a>(state: &'a State, tag: &str) -> Vec<(&'a str, f64)> {
    let owners: BTreeSet<&str> = state.provinces.values().filter_map(|p| p.owner.as_deref()).collect();
    state.countries.iter().filter(|(host, _)| host.as_str() != tag && owners.contains(host.as_str())).filter_map(|(host, h)| Some((host.as_str(), *h.foreign_investment.get(tag)?))).collect()
}

/// The three scores of every country that owns land: (industrial, military, prestige).
pub fn scores(state: &State, rules: &RankRules, lib: &ModifierLibrary) -> BTreeMap<String, [f64; 3]> {
    // Regiments the soldier pops could support, by owner.
    let mut possible: BTreeMap<&str, f64> = BTreeMap::new();
    for p in state.provinces.values() {
        let Some(owner) = p.owner.as_deref() else { continue };
        let multiplier = match p.colonial {
            1 => rules.regiment_protectorate,
            2 => rules.regiment_colony,
            _ if p.cores.iter().any(|c| c == owner) => 1.0,
            _ => rules.regiment_noncore,
        };
        *possible.entry(owner).or_default() += p.pops.iter().filter(|q| rules.soldier_types.contains(&q.pop_type)).map(|q| possible_regiments(rules, q.size, multiplier)).sum::<f64>();
    }
    let at_war: BTreeSet<&str> = state.wars.iter().flat_map(|(a, d)| a.iter().chain(d)).map(String::as_str).collect();
    let owners: BTreeSet<&str> = possible.keys().copied().collect();
    // Each country on its own (they only read the state), on threads, merged in tag order.
    let list: Vec<(&str, f64)> = possible.into_iter().collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 16);
    let chunk = list.len().div_ceil(threads).max(8);
    let one = |tag: &str, possible: f64| -> Option<(String, [f64; 3])> {
        let c = state.countries.get(tag)?;
        let modifier = |key: &str| crate::pops::country_modifier(c, lib, key);
        // What its technologies and inventions add to each unit type.
        let effects: Vec<&UnitEffects> = c.technologies.iter().filter_map(|t| rules.tech_units.get(t)).chain(c.inventions.iter().filter_map(|n| rules.invention_units.get((*n as usize).wrapping_sub(1)))).collect();
        let stats_of = |u: &UnitType| -> UnitStats {
            let mut s = u.stats;
            for (_, add) in effects.iter().flat_map(|e| &e.changes).filter(|(name, _)| *name == u.name || name == if u.land { ALL_LAND } else { ALL_NAVAL }) {
                s.iter_mut().zip(add).for_each(|(a, b)| *a += b);
            }
            s
        };
        // Every land type enters the average, unlocked or not: observation leaves open what
        // marks the types counted, and on the original's yearly saves this reading puts the
        // great powers in the original's order more often (46 of 65) than "unlocked only" (38).
        let land: Vec<UnitStats> = rules.units.iter().filter(|u| u.land).map(stats_of).collect();
        let (land_attack, land_defence) = (modifier("land_attack_modifier"), modifier("land_defense_modifier"));
        let average = if land.is_empty() { 0.0 } else { land.iter().map(|s| (s[ATTACK] + s[DEFENCE] + land_attack + land_defence) * s[DISCIPLINE]).sum::<f64>() / land.len() as f64 };
        // Regiments in being; while mobilised only those with a soldier pop behind them.
        let regiments: f64 = c.regiments.iter().filter(|(pop, _)| !c.mobilized || rules.soldier_types.contains(*pop)).map(|(_, n)| n).sum();
        let block = lib.static_modifiers.get(if at_war.contains(tag) { &rules.war_block } else { &rules.peace_block });
        let supply = (1.0 + modifier("supply_consumption") + block.and_then(|v| v.get("supply_consumption")).copied().unwrap_or(0.0)).max(MILITARY_MIN_SUPPLY);
        let land_part = (possible.min(MILITARY_REGIMENT_CAP * regiments) * average * supply).floor() / MILITARY_LAND_DIVISOR;
        let (naval_attack, naval_defence) = (modifier("naval_attack_modifier"), modifier("naval_defense_modifier"));
        let ships: f64 = rules.units.iter().filter(|u| !u.land && u.capital).map(|u| (c.ships.get(&u.name).copied().unwrap_or(0.0), stats_of(u))).map(|(n, s)| n * (s[GUN_POWER] + naval_attack) * (s[HULL] + naval_defence) / CAPITAL_SHIP_DIVISOR).sum();
        let military = land_part + ships + c.leaders.min(regiments);
        // Money invested in every other country that owns land.
        let abroad: f64 = state.countries.iter().filter(|(host, _)| host.as_str() != tag && owners.contains(host.as_str())).filter_map(|(_, h)| h.foreign_investment.get(tag)).sum();
        let industrial = state.industrial_score(tag) + investment_score(rules, abroad);
        // The save's prestige is taken to leave out the permanent prestige of inventions (the
        // observation suggests adding the two); the saves do not settle it: with or without, the great
        // powers come out in the same order in all but one of 65 yearly saves.
        Some((tag.to_string(), [industrial, military, c.prestige + modifier("permanent_prestige")]))
    };
    let one = &one;
    std::thread::scope(|s| {
        let handles: Vec<_> = list.chunks(chunk).map(|part| s.spawn(move || part.iter().filter_map(|(tag, x)| one(tag, *x)).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().expect("rank worker panicked")).collect()
    })
}

/// What a day's ranking changed among the great powers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RankDay {
    pub promoted: Vec<String>,
    pub demoted: Vec<String>,
}

/// A great power has no overlord and no sphere leader, and nobody has influence in it.
fn impose_great_power(state: &mut State, tag: &str) {
    state.vassals.retain(|(_, v)| v != tag);
    for c in state.countries.values_mut() {
        c.sphere.remove(tag);
        c.influence.remove(tag);
    }
}

/// A fallen great power: its sphere dissolves and its influence inside it is wiped; its
/// influence elsewhere stays.
fn dissolve_sphere(state: &mut State, tag: &str) {
    if let Some(c) = state.countries.get_mut(tag) {
        for member in std::mem::take(&mut c.sphere) {
            c.influence.remove(&member);
        }
    }
}

/// The daily scores, ranking and great-power pass. Fills `State::rank_order`,
/// `State::great_powers` (best first) and every ranked country's `military_score` and
/// `total_score`.
pub fn rank_day(state: &mut State, rules: &RankRules, lib: &ModifierLibrary) -> RankDay {
    let all = scores(state, rules, lib);
    let mut states: BTreeMap<&str, BTreeSet<Option<u16>>> = BTreeMap::new();
    for p in state.provinces.values() {
        if let Some(owner) = p.owner.as_deref() {
            states.entry(owner).or_default().insert(p.region);
        }
    }
    let civilised = |tag: &str| state.countries.get(tag).is_some_and(|c| c.civilized);
    let eligible: BTreeSet<String> = states.iter().filter(|(tag, s)| civilised(tag) && s.len() >= GREAT_POWER_MIN_STATES).map(|(tag, _)| tag.to_string()).collect();
    // Civilised first, then the larger total (the original leaves equal totals as its earlier
    // sorts left them; here they fall by tag).
    let mut order: Vec<(String, bool, f64)> = all.iter().map(|(tag, s)| (tag.clone(), civilised(tag), s.iter().sum())).collect();
    order.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.total_cmp(&a.2)).then(a.0.cmp(&b.0)));
    let order: Vec<String> = order.into_iter().map(|x| x.0).collect();
    for (tag, s) in &all {
        if let Some(c) = state.countries.get_mut(tag) {
            (c.military_score, c.total_score) = (s[1], s.iter().sum());
        }
    }
    let seats = (rules.great_nations.max(0.0) as usize).min(order.len());
    let mut day = RankDay::default();
    let mut great: Vec<String> = state.great_powers.iter().filter(|g| all.contains_key(*g)).cloned().collect();
    let candidate = |great: &[String]| order.iter().find(|t| eligible.contains(*t) && !great.contains(t)).cloned();
    let first_fill = great.is_empty();
    while great.len() < seats {
        let Some(new) = candidate(&great) else { break };
        impose_great_power(state, &new);
        if !first_fill {
            day.promoted.push(new.clone());
        }
        great.push(new);
    }
    let today = rules.day_number(state.date);
    let mut free = seats as i64;
    for tag in &order {
        let (sits, fits) = (great.contains(tag), eligible.contains(tag));
        let keeps = if !fits {
            if let Some(c) = state.countries.get_mut(tag) {
                c.last_greatness = None;
            }
            false
        } else {
            free -= 1;
            if sits && free >= 0 {
                impose_great_power(state, tag);
                if let Some(c) = state.countries.get_mut(tag) {
                    c.last_greatness = Some(state.date);
                }
            }
            // Out of the band: the seat lasts GREATNESS_DAYS from the last day inside it. (The
            // original also keeps it through a war of great powers, a great war and the
            // crisis; the port has none of them.)
            free >= 0 || state.countries.get(tag).and_then(|c| c.last_greatness).and_then(|d| rules.day_number(d)).zip(today).is_some_and(|(last, now)| now as f64 <= last as f64 + rules.greatness_days)
        };
        if sits && !keeps {
            // Without somebody to take the seat the loser stays.
            let Some(new) = candidate(&great) else { continue };
            impose_great_power(state, &new);
            dissolve_sphere(state, tag);
            great.retain(|g| g != tag);
            great.push(new.clone());
            day.promoted.push(new);
            day.demoted.push(tag.clone());
        }
    }
    great.sort_by_key(|g| order.iter().position(|t| t == g));
    state.great_powers = great;
    state.rank_order = order;
    day
}

/// 0 great power, 1 secondary power (civilised, not a great power, rank no worse than
/// COLONIAL_RANK - also inside the great powers' band), 2 other civilised, 3 uncivilised.
pub fn status(state: &State, rules: &RankRules, tag: &str) -> u8 {
    let rank = state.rank_order.iter().position(|t| t == tag).map(|i| i + 1);
    match state.countries.get(tag) {
        Some(c) if !c.civilized => 3,
        None => 3,
        _ if state.great_powers.iter().any(|g| g == tag) => 0,
        _ if rank.is_some_and(|r| r as f64 <= rules.colonial_rank) => 1,
        _ => 2,
    }
}

/// The yearly report: the great powers in order and the eight largest military scores.
pub fn report(state: &State) -> String {
    let mut military: Vec<(&String, f64)> = state.countries.iter().filter(|(t, _)| state.rank_order.contains(t)).map(|(t, c)| (t, c.military_score)).collect();
    military.sort_by(|a, b| b.1.total_cmp(&a.1));
    format!("great powers: {}; military score: {}", state.great_powers.join(" "), military.iter().take(8).map(|(t, x)| format!("{t} {x:.1}")).collect::<Vec<_>>().join(" "))
}

/// For checking against an original save: ranks the state as loaded and says how the result
/// compares with the save's own list of great powers.
pub fn check(state: &mut State, rules: &RankRules, lib: &ModifierLibrary) -> String {
    let theirs = state.great_powers.clone();
    let all = scores(state, rules, lib);
    let mut fresh = state.clone();
    fresh.great_powers.clear();
    rank_day(&mut fresh, rules, lib);
    let day = rank_day(state, rules, lib);
    let set = |v: &[String]| v.iter().cloned().collect::<BTreeSet<_>>();
    let show = |t: &String| all.get(t).map(|s| format!("{t} {:.0}={:.0}+{:.0}+{:.0}", s.iter().sum::<f64>(), s[0], s[1], s[2])).unwrap_or_else(|| format!("{t} -"));
    format!(
        "rankcheck {} theirs [{}] daily [{}] same_set {} same_order {} top8 [{}] top8_set {} changes +{:?} -{:?}\n  scores: {}",
        state.date,
        theirs.join(" "),
        state.great_powers.join(" "),
        set(&theirs) == set(&state.great_powers),
        theirs == state.great_powers,
        fresh.great_powers.join(" "),
        set(&theirs) == set(&fresh.great_powers),
        day.promoted,
        day.demoted,
        state.rank_order.iter().take(12).map(show).collect::<Vec<_>>().join(", ")
    )
}
