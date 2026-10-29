//! Province buildings: forts, naval bases, railroads and every other non-factory building of
//! the install's `common/buildings.txt`. Pure rules: the caller keeps the world.
//!
//! From the files, read at runtime: which types exist, their `type`, `max_level`, `goods_cost`,
//! `time`, `default_enabled`, the placement flags (`province`, `port`, `capital`) and every
//! numeric entry as a per-level effect. A type is open to a country when it is
//! `default_enabled` or a technology or invention of the country has `activate_building` for it
//! (the caller passes the set of activated names). The AI chances and the naval supply numbers
//! come from the defines.
//!
//! Money: the price is paid at the order and kept by the site. The goods list is the type's cost
//! x (2 - administrative efficiency). The site then takes goods from the
//! national stockpile through `economy::construction_takes`, asking for at most 10% of the full
//! amount a day, and opens when its time is up and nothing is lacking.
//!
//! The port's own, or left out:
//! * the build timer starts at the order and counts down every day (the rule description says it runs
//!   elsewhere), the level goes up by one when it is out and the goods are in;
//! * the order price is the goods at the prices of the day of the order (the original's own price
//!   function is not read), x3 for a railroad built abroad, as in the AI's money test;
//! * the technology factor of some types' cost, the "next level costs more" question, the
//!   railway's small threshold for counting, the fort-list and the naval
//!   base province choosers are left to the caller (`FortCandidate::listed`,
//!   the naval candidate list);
//! * colonies: any building may be ordered there by the general rule; the AI skips colonial
//!   states for forts and railroads (AI home steps 1 and 4).

use std::collections::BTreeMap;

use wiktoria_data::defines::Defines;
use wiktoria_data::script::{self, Value};

use wiktoria_data::engine_rules::EngineRules;
use crate::num_field;
use crate::economy::{construction_takes, Construction};
use crate::military::siege::ProvinceMilitary;

/// One building type of buildings.txt.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BuildingDef {
    pub name: String,
    /// `fort`, `naval_base`, `infrastructure`, `factory`, ...
    pub kind: String,
    pub max_level: u32,
    pub goods_cost: Vec<(String, f64)>,
    pub days: f64,
    pub default_enabled: bool,
    /// Built in a province (not a state building).
    pub province: bool,
    /// Needs a port (a coastal province).
    pub port: bool,
    /// Only in the capital province.
    pub capital: bool,
    /// Every numeric entry: per-level effects (`fort_level`, `infrastructure`, ...), `time`...
    pub values: BTreeMap<String, f64>,
    /// `colonial_points = { ... }`: colonial points the building gives at level 1, 2, ... (naval bases).
    pub colonial_points: Vec<f64>,
}

impl BuildingDef {
    /// What the building adds per level for a key of its definition, 0 when absent.
    pub fn per_level(&self, key: &str) -> f64 {
        self.values.get(key).copied().unwrap_or(0.0)
    }

    /// The goods list of an order: the cost x (2 - administrative efficiency).
    pub fn order_goods(&self, efficiency: f64) -> BTreeMap<String, f64> {
        let k = 2.0 - efficiency;
        let mut out = BTreeMap::new();
        for (g, x) in &self.goods_cost {
            *out.entry(g.clone()).or_insert(0.0) += x * k;
        }
        out
    }
}

/// The AI's dice and the naval supply numbers (defines).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BuildingNumbers {
    pub chance_fort: f64,
    pub chance_naval_base: f64,
    pub chance_railroad: f64,
    pub naval_supply_base: f64,
    pub naval_supply_empty: f64,
    pub naval_supply_non_core: f64,
    pub mobilization_rails_mult: f64,
    /// Share of the full goods list a site asks for per day.
    pub daily_ask_share: f64,
    pub fort_treasury_factor: f64,
    pub abroad_price_factor: f64,
    pub rail_reserve_goods: f64,
    pub weak_naval_divisor: f64,
    /// FLD_BLD_AI_RUN_DAYS: the production minister looks at railways once in this many days on
    /// average; FLD_BLD_AI_RAIL_CHANCE: the chance it orders one when it may (measured, in place
    /// of CHANCE_BUILD_RAILROAD).
    pub run_days: f64,
    pub rail_chance: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BuildingRules {
    pub defs: Vec<BuildingDef>,
    pub n: BuildingNumbers,
}

fn define(defines: &Defines, key: &str) -> Result<f64, String> {
    defines.sections().iter().find_map(|(s, _)| defines.get(s, key)).ok_or(format!("defines.{key} missing"))
}

/// Province building types of a buildings.txt text (everything but factories).
pub fn parse_defs(src: &str) -> Result<Vec<BuildingDef>, String> {
    let doc = script::parse(src).map_err(|e| format!("buildings.txt: {e}"))?;
    let mut out = Vec::new();
    for (name, v, _) in doc.root.entries() {
        let Some(b) = v.as_block() else { continue };
        let flag = |k: &str| b.get(k).and_then(Value::as_bool) == Some(true);
        let mut def = BuildingDef {
            name: name.to_string(),
            kind: b.get("type").and_then(Value::as_str).unwrap_or("").to_string(),
            max_level: b.get("max_level").and_then(Value::as_f64).unwrap_or(1.0) as u32,
            goods_cost: b.get("goods_cost").and_then(Value::as_block).map(|c| c.entries().filter_map(|(g, x, _)| Some((g.to_string(), x.as_f64()?))).collect()).unwrap_or_default(),
            days: b.get("time").and_then(Value::as_f64).unwrap_or(0.0),
            default_enabled: flag("default_enabled"),
            province: flag("province"),
            port: flag("port"),
            capital: flag("capital"),
            values: wiktoria_data::modifiers::modifier_values(b),
            colonial_points: b.get("colonial_points").and_then(Value::as_block).map(|l| l.bare_scalars().iter().filter_map(|x| x.parse().ok()).collect()).unwrap_or_default(),
        };
        def.values.remove("max_level");
        out.push(def);
    }
    Ok(out)
}

impl BuildingRules {
    /// Every non-factory building type of `buildings_src`, with the numbers from `defines`.
    pub fn new(buildings_src: &str, defines: &Defines, m: &EngineRules) -> Result<BuildingRules, String> {
        let defs: Vec<BuildingDef> = parse_defs(buildings_src)?.into_iter().filter(|d| d.kind != "factory").collect();
        let d = |k: &str| define(defines, k);
        Ok(BuildingRules {
            defs,
            n: BuildingNumbers {
                chance_fort: d("CHANCE_BUILD_FORT")?,
                chance_naval_base: d("CHANCE_BUILD_NAVAL_BASE")?,
                chance_railroad: d("CHANCE_BUILD_RAILROAD")?,
                naval_supply_base: d("NAVAL_BASE_SUPPLY_SCORE_BASE")?,
                naval_supply_empty: d("NAVAL_BASE_SUPPLY_SCORE_EMPTY")?,
                naval_supply_non_core: d("NAVAL_BASE_NON_CORE_SUPPLY_SCORE")?,
                mobilization_rails_mult: d("MOBILIZATION_SPEED_RAILS_MULT")?,
                daily_ask_share: num_field(m, "FLD_BLD_DAILY_ASK_SHARE")?,
                fort_treasury_factor: num_field(m, "FLD_BLD_AI_FORT_TREASURY_FACTOR")?,
                abroad_price_factor: num_field(m, "FLD_BLD_ABROAD_PRICE_FACTOR")?,
                rail_reserve_goods: num_field(m, "FLD_BLD_AI_RAIL_RESERVE_GOODS")?,
                weak_naval_divisor: num_field(m, "FLD_BLD_WEAK_NAVAL_DIVISOR")?,
                run_days: num_field(m, "FLD_BLD_AI_RUN_DAYS")?,
                rail_chance: num_field(m, "FLD_BLD_AI_RAIL_CHANCE")?,
            },
        })
    }

    pub fn def(&self, name: &str) -> Option<&BuildingDef> {
        self.defs.iter().find(|d| d.name == name)
    }

    /// The first type of a kind (`fort`, `naval_base`, `infrastructure`).
    pub fn of_kind(&self, kind: &str) -> Option<&BuildingDef> {
        self.defs.iter().find(|d| d.kind == kind)
    }
}

/// Is the type open to a country: `default_enabled`, or named by an `activate_building` of one
/// of its technologies or inventions (`activated`).
pub fn unlocked(def: &BuildingDef, activated: &dyn Fn(&str) -> bool) -> bool {
    def.default_enabled || activated(&def.name)
}

/// The province as the building rules see it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Site {
    /// Level of the building now.
    pub level: u32,
    /// A project of this building is already running here.
    pub under_construction: bool,
    pub coastal: bool,
    pub is_capital: bool,
    pub owner: Option<String>,
    pub controller: Option<String>,
    pub colonial: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    NotUnlocked,
    NotAProvinceBuilding,
    AtMaxLevel,
    AlreadyBuilding,
    NeedsPort,
    CapitalOnly,
    /// An occupied province (the controller is not the owner) cannot be built in.
    Occupied,
    /// Foreign provinces: only buildings of the types the owner's country allows (railroads),
    /// the caller decides through `foreign_ok`.
    ForeignNotAllowed,
}

/// May `builder` order the next level of `def` in `site`? `foreign_ok` says whether the builder may
/// build in other countries' provinces (foreign investment of railroads).
pub fn can_build(def: &BuildingDef, unlocked: bool, site: &Site, builder: &str, foreign_ok: bool) -> Result<(), Refusal> {
    if !def.province {
        return Err(Refusal::NotAProvinceBuilding);
    }
    if !unlocked {
        return Err(Refusal::NotUnlocked);
    }
    if site.level >= def.max_level {
        return Err(Refusal::AtMaxLevel);
    }
    if site.under_construction {
        return Err(Refusal::AlreadyBuilding);
    }
    if def.port && !site.coastal {
        return Err(Refusal::NeedsPort);
    }
    if def.capital && !site.is_capital {
        return Err(Refusal::CapitalOnly);
    }
    if site.owner.is_some() && site.owner != site.controller {
        return Err(Refusal::Occupied);
    }
    if site.owner.as_deref().is_some_and(|o| o != builder) && !foreign_ok {
        return Err(Refusal::ForeignNotAllowed);
    }
    Ok(())
}

/// The price of an order: the goods at today's prices (x3 when built abroad).
pub fn order_price(n: &BuildingNumbers, def: &BuildingDef, efficiency: f64, price: impl Fn(&str) -> f64, abroad: bool) -> f64 {
    let v: f64 = def.order_goods(efficiency).iter().map(|(g, x)| x * price(g)).sum();
    v * if abroad { n.abroad_price_factor } else { 1.0 }
}

/// A province building site: the economy's construction object plus what it builds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProvinceConstruction {
    pub province: usize,
    pub building: String,
    pub to_level: u32,
    pub days_left: f64,
    /// Goods list at the order (for the daily ask), money and lacks as in a factory site.
    pub full: BTreeMap<String, f64>,
    pub site: Construction,
}

/// Order the next level. The price is paid by the caller (`order_price`) and kept here.
pub fn order(def: &BuildingDef, province: usize, level_now: u32, efficiency: f64, paid: f64) -> ProvinceConstruction {
    let full = def.order_goods(efficiency);
    ProvinceConstruction { province, building: def.name.clone(), to_level: level_now + 1, days_left: def.days, site: Construction { factory: usize::MAX, money: paid, lacks: full.clone() }, full }
}

/// What the site asks of the stockpile for tomorrow: per good min(lack, 10% of the full amount).
pub fn daily_ask(c: &ProvinceConstruction, share: f64) -> BTreeMap<String, f64> {
    c.site.lacks.iter().map(|(g, l)| (g.clone(), l.min(share * c.full.get(g).copied().unwrap_or(0.0)))).collect()
}

/// What one day of a site did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DayResult {
    /// Handed back to the treasury for the goods taken (booked as income slot 9).
    pub to_treasury: f64,
    /// The level the province now has, once the building is done.
    pub completed: Option<u32>,
    /// Money still in the site that is returned to the treasury at completion.
    pub refund: f64,
}

/// One day of a site: take goods out of `open` (the stock above the reserve), pay back, count
/// the timer down, and finish when the time is out and nothing is lacking.
pub fn construction_day(c: &mut ProvinceConstruction, open: &mut BTreeMap<String, f64>, price: impl Fn(&str) -> f64) -> DayResult {
    let paid = construction_takes(&mut c.site.lacks, open, c.site.money, price);
    c.site.money -= paid;
    c.days_left = (c.days_left - 1.0).max(0.0);
    let mut r = DayResult { to_treasury: paid, ..Default::default() };
    if c.days_left <= 0.0 && c.site.lacks.is_empty() {
        r.refund = c.site.money.max(0.0);
        c.site.money = 0.0;
        r.completed = Some(c.to_level);
    }
    r
}

/// Bankruptcy cancels the site and refunds its money.
pub fn cancel(c: &mut ProvinceConstruction) -> f64 {
    std::mem::take(&mut c.site.money)
}

// ------------------------------------------------------------------ effects other code reads

/// Fort level of a province for sieges and battles: the level times the type's `fort_level`
/// per-level value (1 when the file has none).
pub fn fort_level(def: &BuildingDef, level: u32) -> f64 {
    let per = if def.values.contains_key("fort_level") { def.per_level("fort_level") } else { 1.0 };
    level as f64 * per
}

/// Infrastructure of a province (movement, supply): the level times `infrastructure`.
pub fn infrastructure(def: &BuildingDef, level: u32) -> f64 {
    level as f64 * def.per_level("infrastructure")
}

/// Supply limit bonus of a province from the building (`supply_limit` per level, if the file has it).
pub fn supply_bonus(def: &BuildingDef, level: u32) -> f64 {
    level as f64 * def.per_level("supply_limit")
}

/// The railway factor of mobilisation in a state: 1 + MULT x (sum of level / max level) / provinces.
pub fn mobilisation_rail_factor(n: &BuildingNumbers, rail: &BuildingDef, levels: &[u32]) -> f64 {
    if levels.is_empty() || rail.max_level == 0 {
        return 1.0;
    }
    let share: f64 = levels.iter().map(|l| (*l).min(rail.max_level) as f64 / rail.max_level as f64).sum::<f64>() / levels.len() as f64;
    1.0 + n.mobilization_rails_mult * share
}

/// A state's naval base facts for the supply limit.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StateNaval {
    pub coastal: bool,
    /// Highest naval base level among its ports (only one base per state counts).
    pub best_level: u32,
    /// The base is not in a core of the country AND on another continent than the capital.
    pub non_core_overseas: bool,
}

/// Naval supply points of a country: per state BASE x 2^(level - 1) for a
/// base, EMPTY for a coastal state without one, x NON_CORE (at least 1) when non-core overseas.
pub fn naval_supply_limit(n: &BuildingNumbers, states: &[StateNaval]) -> f64 {
    states
        .iter()
        .filter(|s| s.coastal || s.best_level > 0)
        .map(|s| {
            let score = if s.best_level > 0 { n.naval_supply_base * 2f64.powi(s.best_level as i32 - 1) } else { n.naval_supply_empty };
            if s.non_core_overseas { (score * n.naval_supply_non_core).max(1.0) } else { score }
        })
        .sum()
}

/// A naval base is needed for a ship to be built or to rest: level above 0.
pub fn has_naval_base(level: u32) -> bool {
    level > 0
}

// ------------------------------------------------------------------ peace option

/// Which building a peace term destroys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destroy {
    Forts,
    NavalBases,
}

/// `destroy_forts` / `destroy_naval_bases`: the level goes to 0 in every province the loser owns.
/// Returns how many provinces changed.
pub fn destroy_buildings(provinces: &mut [ProvinceMilitary], loser: &str, what: Destroy) -> usize {
    let mut n = 0;
    for p in provinces.iter_mut().filter(|p| p.owner.as_deref() == Some(loser)) {
        let slot = match what {
            Destroy::Forts => &mut p.fort,
            Destroy::NavalBases => &mut p.naval_base,
        };
        if *slot > 0.0 {
            *slot = 0.0;
            n += 1;
        }
    }
    n
}

// ------------------------------------------------------------------ the AI

/// A neighbouring land province of another owner, for the fort score.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Neighbour {
    /// Border kind as the map has it (1 = land border, 2 = river/other).
    pub border_kind: u8,
    /// The AI's stored threat figure for the neighbour's owner: table value + other / 4.
    pub threat: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct FortCandidate {
    pub province: usize,
    pub capital: bool,
    /// In the list of the fort province chooser (passed by the caller).
    pub listed: bool,
    pub neighbours: Vec<Neighbour>,
    /// The province passes `can_build` for the fort type.
    pub buildable: bool,
    pub colonial_state: bool,
}

/// The fort score of a province.
pub fn fort_score(c: &FortCandidate) -> f64 {
    let mut s = 0.0;
    if c.capital {
        s += 500.0;
    }
    if c.listed {
        s += 500.0;
    }
    for nb in &c.neighbours {
        if nb.border_kind == 1 {
            s += 200.0;
        }
        if nb.threat >= 150.0 {
            s += nb.threat
                * match nb.border_kind {
                    1 => 2.0,
                    2 => 1.5,
                    _ => 1.0,
                };
        }
    }
    s
}

/// What the AI decided to build this run: the province and the building kind.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildPlan {
    pub province: usize,
    pub kind: String,
    /// The treasury the order needs to exceed was met; the caller pays `price`.
    pub price: f64,
}

/// The AI's fort step: only if r <= CHANCE_BUILD_FORT; highest score above 0 among buildable
/// provinces not in colonial states; built only if the treasury exceeds TWICE the price.
pub fn ai_fort(rules: &BuildingRules, r: f64, candidates: &[FortCandidate], treasury: f64, fort_price: f64) -> Option<BuildPlan> {
    if r > rules.n.chance_fort {
        return None;
    }
    let def = rules.of_kind("fort")?;
    let (best, score) = candidates.iter().filter(|c| c.buildable && !c.colonial_state).map(|c| (c, fort_score(c))).fold(None, |acc: Option<(&FortCandidate, f64)>, x| match acc {
        Some((_, s)) if s >= x.1 => acc,
        _ => Some(x),
    })?;
    (score > 0.0 && treasury > rules.n.fort_treasury_factor * fort_price).then(|| BuildPlan { province: best.province, kind: def.kind.clone(), price: fort_price })
}

/// The AI's naval base step: r <= CHANCE_BUILD_NAVAL_BASE, the chance divided by 3 when the
/// country value is below 1 and the great-power-like flag is clear (`weak`). The province
/// comes from the caller's chooser (not read): the first candidate returned. Built if
/// the treasury exceeds the price.
pub fn ai_naval_base(rules: &BuildingRules, r: f64, weak: bool, chosen: Option<usize>, treasury: f64, price: f64) -> Option<BuildPlan> {
    let def = rules.of_kind("naval_base")?;
    let chance = rules.n.chance_naval_base / if weak { rules.n.weak_naval_divisor } else { 1.0 };
    let province = chosen?;
    (r <= chance && treasury > price).then(|| BuildPlan { province, kind: def.kind.clone(), price })
}

/// A state, for the railway choice.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RailState {
    pub state: usize,
    pub factories: usize,
    pub colonial: bool,
    /// The first province controlled by its owner where the next level may be built,
    /// with the price of that level (goods at today's prices).
    pub province: Option<usize>,
    pub price: f64,
}

/// The AI's railway step: r <= FLD_BLD_AI_RAIL_CHANCE (on century1 England
/// ordered a railroad on each run of its production minister in a row, 15-21 days apart, so
/// CHANCE_BUILD_RAILROAD = 0.2 is not the chance per run); among states whose first province can take
/// one more level, the one with the most factories (not colonial; first in the list on a tie);
/// built if the treasury exceeds the price (x3 abroad). `price_goods` is 25 x the price of the
/// building's goods, which the next level must be payable with on top.
pub fn ai_railway(rules: &BuildingRules, r: f64, allowed: bool, states: &[RailState], treasury: f64, abroad: bool, price_goods: f64) -> Option<BuildPlan> {
    if !allowed || r > rules.n.rail_chance {
        return None;
    }
    let def = rules.of_kind("infrastructure")?;
    let mut best: Option<&RailState> = None;
    for s in states.iter().filter(|s| !s.colonial && s.province.is_some() && treasury >= s.price + rules.n.rail_reserve_goods * price_goods) {
        if best.is_none_or(|b| s.factories > b.factories) {
            best = Some(s);
        }
    }
    let s = best?;
    let price = s.price * if abroad { rules.n.abroad_price_factor } else { 1.0 };
    (treasury > price).then(|| BuildPlan { province: s.province.unwrap(), kind: def.kind.clone(), price })
}
