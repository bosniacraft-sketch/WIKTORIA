//! The periodic pop update, one process at a time: growth (measured) and a
//! simple form of promotion and demotion (tuned at country scale, not measured
//! pop by pop).
//!
//! Measured on two saves of the original one day apart (the pops due that day):
//! * A pop changes size on its own update day (FLD_POP_UPDATE_PERIOD).
//! * The rate is base growth + life rating bonus x (life rating - minimum), the life rating
//!   counted up to FLD_GROWTH_LIFE_RATING_CAP, plus the growth modifiers of the province and its
//!   owner. Each of these is held in 1/32768ths, cut off, not rounded: 0.0001 counts as 3 units
//!   and 0.0002 as 6. A pop of 342,350 in a life rating 35 province grew by 188 = 18 units.
//! * People added = size x units / 32768, cut off.
//! * A country-wide growth modifier (`global_population_growth`, e.g. health care) counts for
//!   FLD_GLOBAL_GROWTH_SCALE of its written value: 0.002 from the UK's health care added 6
//!   units, not 65.
//! * The full rate applies down to the starvation limit; below it the province's rate is cut by
//!   FLD_POP_STARVATION_SLOPE x the shortfall, so a pop stops growing at a quarter of its life
//!   needs and shrinks at the full rate at none (from observation, not yet
//!   measured pop by pop).
//! * Slaves grow at 1 / SLAVE_GROWTH_DIVISOR (from the define's name; not seen in the data,
//!   where it rounds to nobody).
//! * Pops in a province nobody owns do not grow.
//!
//! Owner's decision D12: the port does not model what removes people in the original (war
//! deaths, hunger over time), so people gained are multiplied by FLD_POP_GROWTH_SCALE, set so
//! the world grows to about 1.25 times its size in 100 years. With the scale at 1 the rule is
//! the measured one.

use std::collections::BTreeMap;

use wiktoria_data::defines::Defines;
use wiktoria_data::script::{Block, Value};
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::modifiers::ModifierLibrary;

use crate::calendar::Calendar;
use crate::cond::{Asked, DeepProduct, DeepSum, Facts, Memo};
use crate::schedule::PopSchedule;
use crate::script::{conditions_hold, CountryState, PopState, ProvinceState, Scope, State};

/// Which pops are due today: `PopSchedule::is_due_in` for one day of one month, worked out with
/// one remainder per pop (every monthly rule asks it of every pop, every day).
#[derive(Debug, Clone, Copy)]
pub struct DueToday {
    month_days: u64,
    /// What the pop id must leave when divided by the days of the month.
    rest: u64,
    /// 2^64 / days of the month, rounded up: `id x this` (cut to 64 bits) x days / 2^64 is the
    /// remainder of an id below 2^32.
    magic: u64,
}

impl DueToday {
    pub fn new(schedule: &PopSchedule, day_of_month: u8, month_days: u64) -> DueToday {
        if month_days == 0 {
            return DueToday { month_days, rest: 0, magic: 0 };
        }
        // (day + days - id % days) % days == offset % days  <=>  id % days == (day + days - offset % days) % days
        DueToday { month_days, rest: (u64::from(day_of_month) + month_days - schedule.offset % month_days) % month_days, magic: (u64::MAX / month_days).wrapping_add(1) }
    }

    pub fn is_due(&self, pop_id: u64) -> bool {
        if self.month_days == 0 {
            return false;
        }
        let small = pop_id <= u64::from(u32::MAX) && self.month_days <= u64::from(u32::MAX);
        let rest = if small { ((u128::from(self.magic.wrapping_mul(pop_id)) * u128::from(self.month_days)) >> 64) as u64 } else { pop_id % self.month_days };
        rest == self.rest
    }
}

/// 1/32768: the grain the original keeps rates in.
const UNITS: f64 = 32768.0;

fn units(x: f64) -> i64 {
    (x * UNITS).trunc() as i64
}

/// Whole people of an amount, cut off. The original counts in fixed point; the small allowance
/// keeps binary fractions (0.002 x 0.7 x 100,000 = 139.99999999999997) from losing a person.
fn whole(x: f64) -> f64 {
    (x + 1e-9).trunc()
}

#[derive(Debug, Clone, PartialEq)]
pub struct GrowthRules {
    pub base_units: i64,
    pub bonus_units: i64,
    pub min_life_rating: f64,
    pub life_rating_cap: f64,
    pub starvation_limit: f64,
    /// How fast growth falls off under the starvation limit (FLD_POP_STARVATION_SLOPE).
    pub starvation_slope: f64,
    pub slave_divisor: f64,
    pub global_scale: f64,
    /// Share of the people gained that is kept (D12).
    pub growth_scale: f64,
}

impl GrowthRules {
    pub fn new(defines: &Defines, m: &EngineRules) -> Result<GrowthRules, String> {
        let d = |k: &str| defines.get("pops", k).ok_or(format!("defines.pops.{k} missing"));
        Ok(GrowthRules {
            base_units: units(d("BASE_POPGROWTH")?),
            bonus_units: units(d("LIFE_RATING_GROWTH_BONUS")?),
            min_life_rating: d("MIN_LIFE_RATING_FOR_GROWTH")?,
            life_rating_cap: crate::num_field(m, "FLD_GROWTH_LIFE_RATING_CAP")?,
            starvation_limit: d("LIFE_NEED_STARVATION_LIMIT")?,
            starvation_slope: crate::num_field(m, "FLD_POP_STARVATION_SLOPE")?,
            slave_divisor: d("SLAVE_GROWTH_DIVISOR")?,
            global_scale: crate::num_field(m, "FLD_GLOBAL_GROWTH_SCALE")?,
            growth_scale: crate::num_field(m, "FLD_POP_GROWTH_SCALE")?,
        })
    }
}

/// Growth modifier of a country, in units: active inventions, chosen reforms and its timed
/// modifiers.
pub fn country_growth_units(c: &CountryState, lib: &ModifierLibrary, global_scale: f64) -> i64 {
    let mut total = 0;
    for n in &c.inventions {
        if let Some((_, values)) = lib.inventions.get((*n as usize).wrapping_sub(1)) {
            total += units(values.get("pop_growth").copied().unwrap_or(0.0));
        }
    }
    for option in c.reforms.values() {
        total += units(lib.issue_options.get(option).and_then(|v| v.get("global_population_growth")).copied().unwrap_or(0.0) * global_scale);
    }
    for m in &c.modifiers {
        total += units(lib.event_modifiers.get(&m.name).and_then(|v| v.get("global_population_growth")).copied().unwrap_or(0.0) * global_scale);
    }
    total
}

/// Growth modifier of a province, in units: its timed modifiers.
pub fn province_growth_units(p: &ProvinceState, lib: &ModifierLibrary) -> i64 {
    p.modifiers.iter().map(|m| units(lib.event_modifiers.get(&m.name).and_then(|v| v.get("population_growth")).copied().unwrap_or(0.0))).sum()
}

/// People a pop gains (or loses) at its update.
pub fn growth(rules: &GrowthRules, size: f64, life_needs: f64, life_rating: f64, is_slave: bool, modifier_units: i64) -> f64 {
    let levels = (life_rating.min(rules.life_rating_cap) - rules.min_life_rating).max(0.0) as i64;
    let province = (rules.base_units + rules.bonus_units * levels) as f64;
    // Under the starvation limit the province's rate is cut by slope x the shortfall: with the
    // limit at a half and the slope 4, nothing is left at a quarter of the life needs and the pop
    // shrinks at the full rate at none. The modifiers count in full either way.
    let hunger = if life_needs < rules.starvation_limit { (life_needs - rules.starvation_limit) * rules.starvation_slope * province } else { 0.0 };
    let people = size * (province + hunger + modifier_units as f64) / UNITS;
    let people = if people > 0.0 { people * rules.growth_scale } else { people };
    (if is_slave { people / rules.slave_divisor } else { people }).trunc()
}

/// Applies growth to every pop due today. Returns (pops updated, people added).
pub fn grow_due_pops(state: &mut State, rules: &GrowthRules, schedule: &PopSchedule, cal: &Calendar, lib: &ModifierLibrary) -> (usize, f64) {
    let today = DueToday::new(schedule, state.date.day, cal.month_len(state.date.month));
    // Worked out for a country when the first of its pops is due.
    let mut country_units: BTreeMap<&str, i64> = BTreeMap::new();
    let countries = &state.countries;
    let slave_types = state.slave_types.clone();
    let (mut updated, mut added) = (0, 0.0);
    for p in state.provinces.values_mut() {
        let Some(owner) = p.owner.as_deref() else { continue };
        if !p.pops.iter().any(|q| today.is_due(q.id)) {
            continue;
        }
        let of_country = countries.get_key_value(owner).map_or(0, |(tag, c)| *country_units.entry(tag.as_str()).or_insert_with(|| country_growth_units(c, lib, rules.global_scale)));
        let modifier_units = province_growth_units(p, lib) + of_country;
        let life_rating = p.life_rating;
        for pop in p.pops.iter_mut().filter(|q| today.is_due(q.id)) {
            let change = growth(rules, pop.size, pop.life_needs, life_rating, slave_types.contains(&pop.pop_type), modifier_units);
            pop.size = (pop.size + change).max(0.0);
            updated += 1;
            added += change;
        }
    }
    (updated, added)
}

// ---------------------------------------------------------------- promotion and demotion

/// The script side of pops changing type: the two chances shared by all pop types
/// (FLD_POP_RULES_FILE) and each type's `promote_to` targets (the pop type files).
#[derive(Debug, Clone, Default)]
pub struct MobilityDefs {
    pub promotion_chance: Block,
    pub demotion_chance: Block,
    /// Pop type -> (target type, weight block).
    pub promote_to: BTreeMap<String, Vec<(String, Block)>>,
    pub assimilation_chance: Block,
    pub migration_chance: Block,
    pub emigration_chance: Block,
    /// Pop type -> how attractive a province / a country is to it.
    pub migration_target: BTreeMap<String, Block>,
    pub country_migration_target: BTreeMap<String, Block>,
    /// National focus name -> the pop type it encourages and by how much (`promote_bureaucrats`
    /// -> bureaucrats, 0.2).
    pub focus: BTreeMap<String, (String, f64)>,
    /// Pop type -> (ideology, weight block) and (issue option, weight block) of its file.
    pub ideologies: BTreeMap<String, Vec<(String, Block)>>,
    pub issues: BTreeMap<String, Vec<(String, Block)>>,
    /// Ideologies marked `uncivilized = no` in the ideologies file.
    pub civilized_only: std::collections::BTreeSet<String>,
    /// National focus name -> its `immigrant_attract` (the immigration focus, 0.2).
    pub focus_attract: BTreeMap<String, f64>,
    /// `ideologies` and `issues` compiled (made on first use, after loading).
    compiled: std::sync::OnceLock<CompiledWeights>,
}

/// The weight blocks of `MobilityDefs` compiled: pop type -> (name, weight) of `ideologies` and
/// `issues`, the five chances, pop type -> the weights of its `promote_to` targets in the same
/// order, and pop type -> its two migration target weights.
#[derive(Debug, Clone, Default)]
struct CompiledWeights {
    ideologies: BTreeMap<String, Vec<(String, DeepProduct)>>,
    issues: BTreeMap<String, Vec<(String, DeepProduct)>>,
    promotion: DeepSum,
    demotion: DeepSum,
    assimilation: DeepSum,
    migration: DeepSum,
    emigration: DeepSum,
    promote_to: BTreeMap<String, Vec<DeepSum>>,
    migration_target: BTreeMap<String, DeepProduct>,
    country_migration_target: BTreeMap<String, DeepProduct>,
}

impl MobilityDefs {
    fn compiled(&self) -> &CompiledWeights {
        self.compiled.get_or_init(|| {
            let compile = |m: &BTreeMap<String, Vec<(String, Block)>>| m.iter().map(|(k, list)| (k.clone(), list.iter().map(|(n, b)| (n.clone(), DeepProduct::new(b))).collect())).collect();
            let targets = |m: &BTreeMap<String, Block>| m.iter().map(|(k, b)| (k.clone(), DeepProduct::new(b))).collect();
            CompiledWeights {
                ideologies: compile(&self.ideologies),
                issues: compile(&self.issues),
                promotion: DeepSum::new(&self.promotion_chance),
                demotion: DeepSum::new(&self.demotion_chance),
                assimilation: DeepSum::new(&self.assimilation_chance),
                migration: DeepSum::new(&self.migration_chance),
                emigration: DeepSum::new(&self.emigration_chance),
                promote_to: self.promote_to.iter().map(|(k, list)| (k.clone(), list.iter().map(|(_, b)| DeepSum::new(b)).collect())).collect(),
                migration_target: targets(&self.migration_target),
                country_migration_target: targets(&self.country_migration_target),
            }
        })
    }

    /// Reads the two chances from the root of the pop rules file.
    pub fn with_chances(mut self, root: &Block) -> MobilityDefs {
        let block = |k: &str| root.get(k).and_then(Value::as_block).cloned().unwrap_or_default();
        (self.promotion_chance, self.demotion_chance) = (block("promotion_chance"), block("demotion_chance"));
        (self.assimilation_chance, self.migration_chance, self.emigration_chance) = (block("assimilation_chance"), block("migration_chance"), block("emigration_chance"));
        self.compiled = std::sync::OnceLock::new();
        self
    }

    /// Reads the foci that encourage a pop type from the root of the national focus file.
    /// Reads which ideologies are for civilized countries only from the ideologies file.
    pub fn with_ideology_file(mut self, root: &Block) -> MobilityDefs {
        for group in root.entries().filter_map(|(_, v, _)| v.as_block()) {
            for (name, v, _) in group.entries() {
                if v.as_block().is_some_and(|b| b.get("uncivilized").and_then(Value::as_bool) == Some(false)) {
                    self.civilized_only.insert(name.to_string());
                }
            }
        }
        self
    }

    pub fn with_foci(mut self, root: &Block, pop_types: &std::collections::BTreeSet<String>) -> MobilityDefs {
        for (_, group, _) in root.entries() {
            for (name, focus, _) in group.as_block().into_iter().flat_map(|b| b.entries()) {
                let Some(focus) = focus.as_block() else { continue };
                if let Some(x) = focus.get("immigrant_attract").and_then(Value::as_f64) {
                    self.focus_attract.insert(name.to_string(), x);
                }
                if let Some((kind, value)) = focus.entries().find_map(|(k, v, _)| Some((k, v.as_f64()?)).filter(|(k, _)| pop_types.contains(*k))) {
                    self.focus.insert(name.to_string(), (kind.to_string(), value));
                }
            }
        }
        self
    }

    /// Adds the `promote_to` targets of one pop type file.
    pub fn add_pop_type(&mut self, name: &str, root: &Block) {
        let targets = root.get("promote_to").and_then(Value::as_block).map(|b| b.entries().filter_map(|(k, v, _)| Some((k.to_string(), v.as_block()?.clone()))).collect()).unwrap_or_default();
        self.promote_to.insert(name.to_string(), targets);
        for (key, map) in [("ideologies", &mut self.ideologies), ("issues", &mut self.issues)] {
            let list = root.get(key).and_then(Value::as_block).map(|b| b.entries().filter_map(|(k, v, _)| Some((k.to_string(), v.as_block()?.clone()))).collect()).unwrap_or_default();
            map.insert(name.to_string(), list);
        }
        for (key, map) in [("migration_target", &mut self.migration_target), ("country_migration_target", &mut self.country_migration_target)] {
            if let Some(b) = root.get(key).and_then(Value::as_block) {
                map.insert(name.to_string(), b.clone());
            }
        }
        self.compiled = std::sync::OnceLock::new();
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MobilityRules {
    /// PROMOTION_SCALE of the defines.
    pub scale: f64,
    /// The port's own tuning of how many people move (FLD_PROMOTION_TUNE / FLD_DEMOTION_TUNE).
    pub promotion_tune: f64,
    pub demotion_tune: f64,
    /// ASSIMILATION_SCALE and IMMIGRATION_SCALE of the defines.
    pub assimilation_scale: f64,
    pub immigration_scale: f64,
    /// BASE_COUNTRY_ADMIN_EFFICIENCY and MAX_BUREAUCRACY_PERCENTAGE of the defines.
    pub admin_base: f64,
    pub max_bureaucracy: f64,
    /// A pop smaller than this moves as a whole (FLD_MIGRATION_WHOLE_POP).
    pub whole_pop: f64,
    /// How many of the best targets share the migrants (FLD_MIGRATION_TARGETS).
    pub targets: usize,
    /// The port's own factors, tuned on a year of the original at country scale: on people
    /// assimilated in a state / in a colonial province, moving within a country, and emigrating.
    pub assimilation_state: f64,
    pub assimilation_colony: f64,
    pub migration_tune: f64,
    pub emigration_tune: f64,
    /// Militancy and consciousness of a pop that promotion, demotion, assimilation or migration
    /// founds (FLD_POP_NEW_POP_MOOD; measured: 0 for all 198 new pops of 1836.1.6-1.7,
    /// 2.8-2.9 and 2.9-2.10, also where their source pops stood at militancy 8).
    pub new_pop_mood: f64,
}

impl MobilityRules {
    pub fn new(defines: &Defines, m: &EngineRules) -> Result<MobilityRules, String> {
        Ok(MobilityRules {
            scale: defines.get("pops", "PROMOTION_SCALE").ok_or("defines.pops.PROMOTION_SCALE missing")?,
            promotion_tune: crate::num_field(m, "FLD_PROMOTION_TUNE")?,
            demotion_tune: crate::num_field(m, "FLD_DEMOTION_TUNE")?,
            assimilation_scale: defines.get("pops", "ASSIMILATION_SCALE").ok_or("defines.pops.ASSIMILATION_SCALE missing")?,
            immigration_scale: defines.get("pops", "IMMIGRATION_SCALE").ok_or("defines.pops.IMMIGRATION_SCALE missing")?,
            admin_base: defines.get("country", "BASE_COUNTRY_ADMIN_EFFICIENCY").ok_or("defines.country.BASE_COUNTRY_ADMIN_EFFICIENCY missing")?,
            max_bureaucracy: defines.get("country", "MAX_BUREAUCRACY_PERCENTAGE").ok_or("defines.country.MAX_BUREAUCRACY_PERCENTAGE missing")?,
            whole_pop: crate::num_field(m, "FLD_MIGRATION_WHOLE_POP")?,
            targets: crate::int_field(m, "FLD_MIGRATION_TARGETS")?.max(1) as usize,
            assimilation_state: crate::num_field(m, "FLD_ASSIMILATION_STATE_FACTOR")?,
            assimilation_colony: crate::num_field(m, "FLD_ASSIMILATION_COLONY_FACTOR")?,
            migration_tune: crate::num_field(m, "FLD_MIGRATION_TUNE")?,
            emigration_tune: crate::num_field(m, "FLD_EMIGRATION_TUNE")?,
            new_pop_mood: crate::num_field(m, "FLD_POP_NEW_POP_MOOD")?,
        })
    }
}

/// A weight written as `factor = base` plus `modifier = { factor = x <conditions> }` entries,
/// loose or inside `group = { }`: the base plus the factor of every modifier whose conditions
/// hold. Inside a group the modifiers are added in order and the group stops at the first one
/// whose conditions fail (read from the original's routine; the ladders `literacy = 0.05`,
/// `literacy = 0.1` ... are written that way). A modifier with a condition the engine does not
/// know is left out and does not stop a group.
pub fn additive_weight(state: &State, block: &Block, scope: &Scope) -> f64 {
    let mut total = block.get("factor").and_then(Value::as_f64).unwrap_or(0.0);
    for item in &block.items {
        match (item.key.as_deref(), item.value.as_block()) {
            (Some("modifier"), Some(m)) => {
                if conditions_hold(state, m, scope) == Ok(true) {
                    total += m.get("factor").and_then(Value::as_f64).unwrap_or(0.0);
                }
            }
            (Some("group"), Some(g)) => {
                total += g.get("factor").and_then(Value::as_f64).unwrap_or(0.0);
                for m in g.get_all("modifier").filter_map(Value::as_block) {
                    match conditions_hold(state, m, scope) {
                        Ok(true) => total += m.get("factor").and_then(Value::as_f64).unwrap_or(0.0),
                        Ok(false) => break,
                        Err(_) => {}
                    }
                }
            }
            _ => {}
        }
    }
    total
}

/// A chance block as the original evaluates it: the block's own `factor` times the sum of the
/// modifiers that hold (promotion 0.25 x sum, demotion 1 x sum).
pub fn chance_value(state: &State, block: &Block, scope: &Scope) -> f64 {
    let factor = block.get("factor").and_then(Value::as_f64).unwrap_or(0.0);
    factor * (additive_weight(state, block, scope) - factor)
}

fn rank(strata: Option<&String>) -> u8 {
    match strata.map(String::as_str) {
        Some("rich") => 2,
        Some("middle") => 1,
        _ => 0,
    }
}

// rules file: FLD_POP_MOVE_THRESHOLD - people that must be waiting before a pop's promotion, demotion or migration is carried out (the pop type's default, read in the original's routine)
const MOVE_THRESHOLD: f64 = 1.0;

/// The pop types that work in factories: the only ones whose unemployment the port can tell
/// (none of them has work in a state without a factory).
fn factory_worker(kind: &str) -> bool {
    matches!(kind, "craftsmen" | "clerks")
}

/// One target type for the movers of a pop: `forced` (the state's focus) or, failing that, the
/// first type at which the running sum of the positive weights passes `r` x their total.
fn pick_target(weights: &[(String, f64)], forced: Option<&String>, r: f64) -> Option<String> {
    if let Some(f) = forced {
        return Some(f.clone());
    }
    let total: f64 = weights.iter().filter(|w| w.1 > 0.0).map(|w| w.1).sum();
    let (threshold, mut sum) = (r * total, 0.0);
    weights.iter().filter(|w| w.1 > 0.0).find(|w| { sum += w.1; sum > threshold }).map(|w| w.0.clone())
}

/// Per lot (promotion, demotion) that is due: the weights of its targets and the target the
/// state's focus forces.
type Picks = [Option<(Vec<(String, f64)>, Option<String>)>; 2];

/// Promotion and demotion of every pop due today (read from the original's routines; the
/// people moved were measured on daily saves).
///
/// Each month a pop adds to the people waiting to promote: PROMOTION_SCALE x chance x size x
/// the country's administrative efficiency, cut off, at least 1 once the chance is positive.
/// The chance is the block's `factor` x the sum of the modifiers that hold. Demotion the same
/// without the efficiency. The people move only once more than FLD_POP_MOVE_THRESHOLD are
/// waiting (or the whole pop), all to ONE target type; what would be left below the threshold
/// goes too. The targets are the `promote_to` entries of the pop type: of the same or a higher
/// strata for promotion, the same or a lower for demotion (the port's split of the one list),
/// each weighed by its own additive block; one number drawn per pop serves both picks.
///
/// National focus: when the pop's own chance is positive, the focus value (0.2)
/// is added to it, provided the focus is on another type that is among the pop's targets and
/// not of a lower strata. A pop of the focused type does not promote at all; the focus pulls
/// demotion in the same way. When the focused type is a possible target (its weight plus the
/// focus value is positive), ALL the movers go to it.
///
/// Demotion never goes to a type that is worse employed in the province than the pop's own. The
/// port can tell that only for factory workers: craftsmen and clerks are not a target in a
/// state without a factory (unless the pop is one of them).
///
/// A promotion that finds no target is dropped and the pop does not demote that month either
/// (the original also skips its assimilation and growth; the port runs those apart and does
/// not). Not modelled: the move to the state capital for aristocrats, bureaucrats and
/// capitalists, and the swap between farmers and labourers by what the province produces.
/// Returns the people moved, by (from, to).
pub fn move_due_pops(state: &mut State, defs: &MobilityDefs, rules: &MobilityRules, schedule: &PopSchedule, cal: &Calendar) -> BTreeMap<(String, String), f64> {
    use std::cmp::Ordering;
    let today = DueToday::new(schedule, state.date.day, cal.month_len(state.date.month));
    let efficiency = admin_efficiency(state, rules);
    let compiled = defs.compiled();
    let (facts, memo) = (Facts::new(), Memo::new());
    // Province, pop, people now waiting to promote and to demote, and the picks of the lots due.
    let due: Vec<(u16, usize, [f64; 2], Picks)> = par_due_provinces(state, today, |id, p, due| {
        let Some(admin) = p.owner.as_ref().and_then(|o| efficiency.get(o)) else { return };
        for (i, pop) in p.pops.iter().enumerate().filter(|(_, q)| today.is_due(q.id)) {
            let Some(targets) = defs.promote_to.get(&pop.pop_type).filter(|_| !state.slave_types.contains(&pop.pop_type)) else { continue };
            let weights_of = compiled.promote_to.get(&pop.pop_type).map_or(&[][..], Vec::as_slice);
            let scope = Scope::Pop(id, i);
            let at = Asked::new(state, &scope);
            let mine = rank(state.strata.get(&pop.pop_type));
            let focus = p.region.zip(p.owner.as_ref()).and_then(|(r, o)| state.countries.get(o)?.national_focus.get(&r)).and_then(|f| defs.focus.get(f));
            let own_focus = focus.is_some_and(|f| f.0 == pop.pop_type);
            let pull = focus.filter(|f| !own_focus && targets.iter().any(|t| t.0 == f.0) && mine <= rank(state.strata.get(&f.0))).map_or(0.0, |f| f.1);
            let mut waiting = [pop.promotion_due, pop.demotion_due];
            let mut picks: Picks = [None, None];
            for (n, up, block, tune) in [(0, true, &compiled.promotion, rules.promotion_tune * admin), (1, false, &compiled.demotion, rules.demotion_tune)] {
                // `chance_value`: the block's factor times the sum of the modifiers that hold.
                let chance = block.base * (block.weight(state, Some(&facts), &memo, &at) - block.base);
                if chance > 0.0 && !(up && own_focus) {
                    waiting[n] += whole(rules.scale * (chance + pull) * pop.size * tune).max(1.0);
                }
                if waiting[n] <= 0.0 || (waiting[n] <= MOVE_THRESHOLD && waiting[n] < pop.size) {
                    continue;
                }
                let barred = if up { Ordering::Less } else { Ordering::Greater };
                let mut forced = None;
                let mut weights = Vec::new();
                for ((t, _), b) in targets.iter().zip(weights_of).filter(|((t, _), _)| rank(state.strata.get(t)).cmp(&mine) != barred) {
                    if !up && factory_worker(t) && !p.has_factory && !factory_worker(&pop.pop_type) {
                        continue;
                    }
                    let pulled = focus.filter(|f| f.0 == *t).map_or(0.0, |f| f.1);
                    let w = b.weight(state, Some(&facts), &memo, &at) + pulled;
                    if pulled != 0.0 && w > 0.0 {
                        forced = Some(t.clone());
                    }
                    weights.push((t.clone(), w));
                }
                picks[n] = Some((weights, forced));
            }
            if waiting != [pop.promotion_due, pop.demotion_due] || picks.iter().any(Option::is_some) {
                due.push((id, i, waiting, picks));
            }
        }
    });
    let mut moves: Vec<Move> = Vec::new();
    for (id, i, waiting, picks) in due {
        let r = if picks.iter().any(Option::is_some) { state.roll_fraction() } else { 0.0 };
        let Some(pop) = state.provinces.get_mut(&id).and_then(|p| p.pops.get_mut(i)) else { continue };
        (pop.promotion_due, pop.demotion_due) = (waiting[0], waiting[1]);
        let mut left = pop.size;
        for (n, pick) in picks.into_iter().enumerate() {
            let Some((weights, forced)) = pick else { continue };
            if n == 0 { pop.promotion_due = 0.0 } else { pop.demotion_due = 0.0 }
            let Some(target) = pick_target(&weights, forced.as_ref(), r) else {
                // No target: the original leaves the pop's routine here, so a promotion without
                // a target also keeps the pop from demoting this month.
                break;
            };
            let people = if waiting[n] > left || left - waiting[n] < MOVE_THRESHOLD { left } else { waiting[n] };
            if people > 0.0 {
                left -= people;
                moves.push(Move { from: (id, i), to: id, pop_type: Some(target), culture: None, people, any_culture: true });
            }
        }
    }
    let mut flows = BTreeMap::new();
    for (from, m) in apply_moves(state, moves, rules.new_pop_mood) {
        *flows.entry((from, m.pop_type.unwrap_or_default())).or_insert(0.0) += m.people;
    }
    flows
}

/// People leaving one pop for another: in province `to`, of `pop_type` and `culture` when given
/// (else the source pop's own). `any_culture`: they may join a pop of the type that has another
/// culture or religion when none has theirs (promotion and demotion).
#[derive(Debug, Clone)]
struct Move {
    from: (u16, usize),
    to: u16,
    pop_type: Option<String>,
    culture: Option<String>,
    people: f64,
    any_culture: bool,
}

/// Migrants arriving in a province with their cash: they join the pop of their type, culture and
/// religion there or found a new one, whose militancy and consciousness start at `new_mood`.
fn settle(state: &mut State, to: u16, who: &PopState, people: f64, money: f64, new_mood: f64, next_id: &mut u64) {
    let Some(target) = state.provinces.get_mut(&to) else { return };
    match settle_with(&target.pops, &who.pop_type, &who.culture, &who.religion, false) {
        Some(i) => {
            target.pops[i].size += people;
            target.pops[i].money += money;
        }
        None => {
            target.pops.push(PopState { id: *next_id, size: people, money, promotion_due: 0.0, demotion_due: 0.0, militancy: new_mood, consciousness: new_mood, ..who.clone() });
            *next_id += 1;
        }
    }
}

/// The pop that arrivals of this type, culture and religion join (see `settle`), if there is one.
fn settle_with(pops: &[PopState], kind: &str, culture: &str, religion: &str, any_culture: bool) -> Option<usize> {
    let find = |test: &dyn Fn(&PopState) -> bool| pops.iter().position(|q| q.pop_type == kind && q.size > 0.0 && test(q));
    find(&|q| q.culture == culture && q.religion == religion).or_else(|| if any_culture { find(&|q| q.culture == culture).or_else(|| find(&|_| true)) } else { None })
}

fn next_pop_id(state: &State) -> u64 {
    state.provinces.values().flat_map(|p| &p.pops).map(|q| q.id).max().unwrap_or(0) + 1
}

/// Carries the moves out: the people leave with their share of the pop's cash and settle in the
/// target province as `settle` has it: they join the pop of their type, culture and religion
/// there or found a new one, which is the pop they left in everything else but its militancy and
/// consciousness, which start at `new_mood`. Returns each move with the type of the pop it left.
fn apply_moves(state: &mut State, moves: Vec<Move>, new_mood: f64) -> Vec<(String, Move)> {
    let mut next_id = next_pop_id(state);
    let mut done = Vec::with_capacity(moves.len());
    for mut m in moves {
        if !state.provinces.contains_key(&m.to) {
            continue;
        }
        let Some(source) = state.provinces.get_mut(&m.from.0).and_then(|p| p.pops.get_mut(m.from.1)) else { continue };
        m.people = m.people.min(source.size);
        if m.people <= 0.0 {
            continue;
        }
        let money = source.money * m.people / source.size;
        source.size -= m.people;
        source.money -= money;
        let from = source.pop_type.clone();
        let (kind, culture, religion) = (m.pop_type.clone().unwrap_or_else(|| from.clone()), m.culture.clone().unwrap_or_else(|| source.culture.clone()), source.religion.clone());
        let joined = state.provinces.get(&m.to).and_then(|target| settle_with(&target.pops, &kind, &culture, &religion, m.any_culture));
        // A new pop: the one they left as it is now, with their type, culture, number and cash,
        // and the mood of a new pop.
        let founded = if joined.is_none() { state.provinces.get(&m.from.0).and_then(|p| p.pops.get(m.from.1)).map(|left| PopState { id: next_id, size: m.people, money, promotion_due: 0.0, demotion_due: 0.0, pop_type: kind, culture, militancy: new_mood, consciousness: new_mood, ..left.clone() }) } else { None };
        if let Some(target) = state.provinces.get_mut(&m.to) {
            match (joined, founded) {
                (Some(i), _) => {
                    target.pops[i].size += m.people;
                    target.pops[i].money += money;
                }
                (None, Some(new)) => {
                    target.pops.push(new);
                    next_id += 1;
                }
                (None, None) => {}
            }
        }
        done.push((from, m));
    }
    done
}

/// What administrative efficiency needs from the install besides MAX_BUREAUCRACY_PERCENTAGE
/// (kept on `State::admin_rules`; empty until `attach_admin_rules`, which leaves the plain rule
/// "bureaucrat share / MAX_BUREAUCRACY_PERCENTAGE").
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AdminRules {
    /// BUREAUCRACY_PERCENTAGE_INCREMENT: added to the required share for each administrative reform level.
    pub increment: f64,
    /// Reform option -> its `administrative_multiplier` (the level it counts for).
    pub option_levels: BTreeMap<String, f64>,
    /// Reform option, national value or timed modifier -> its `administrative_efficiency_modifier`.
    pub modifier_bonus: BTreeMap<String, f64>,
    /// Technology -> its `administrative_efficiency`.
    pub tech_bonus: BTreeMap<String, f64>,
}

/// Fills `State::admin_rules` from the install. Call once after the state is built.
pub fn attach_admin_rules(state: &mut State, world: &wiktoria_data::world::World) {
    let lib = &world.defs.modifiers;
    state.admin_rules = AdminRules {
        increment: world.defines.get("country", "BUREAUCRACY_PERCENTAGE_INCREMENT").unwrap_or(0.0),
        option_levels: lib.issue_options.iter().filter_map(|(o, v)| Some((o.clone(), *v.get("administrative_multiplier")?))).collect(),
        modifier_bonus: lib.issue_options.iter().chain(&lib.national_values).chain(&lib.event_modifiers).filter_map(|(o, v)| Some((o.clone(), *v.get("administrative_efficiency_modifier")?))).collect(),
        tech_bonus: world.defs.technologies.iter().filter_map(|t| Some((t.name.clone(), *t.values.get("administrative_efficiency")?))).collect(),
    };
}

/// Administrative efficiency of every country (it slows promotion, sets the minimum wage and
/// the overhead of state pay, and adds to tariff efficiency). See `admin_efficiency_shares`.
pub fn admin_efficiency(state: &State, rules: &MobilityRules) -> BTreeMap<String, f64> {
    admin_efficiency_shares(state, rules.max_bureaucracy)
}

/// Administrative efficiency of every country:
/// min(1, bureaucrat share x (1 + administrative efficiency modifiers + technologies)
/// / (MAX_BUREAUCRACY_PERCENTAGE + BUREAUCRACY_PERCENTAGE_INCREMENT x administrative reform levels)),
/// where the levels are the `administrative_multiplier` of the country's reform options. The base
/// of the defines (BASE_COUNTRY_ADMIN_EFFICIENCY) is not part of it. With no reform and no
/// modifier this is the share against 1%, as measured on the original's own tooltip:
/// Russia 1836 with 0.168% bureaucrats shows "16.82%".
/// The description infers that only non-colonial states are counted; the tooltip figure was
/// matched with all the country's people, so the port counts all of them.
pub fn admin_efficiency_shares(state: &State, max_bureaucracy: f64) -> BTreeMap<String, f64> {
    let mut people: BTreeMap<&str, (f64, f64)> = BTreeMap::new();
    for p in state.provinces.values() {
        let Some(owner) = p.owner.as_deref() else { continue };
        let e = people.entry(owner).or_default();
        for q in &p.pops {
            e.0 += q.size;
            if q.pop_type == "bureaucrats" {
                e.1 += q.size;
            }
        }
    }
    let rules = &state.admin_rules;
    people
        .into_iter()
        .map(|(t, (all, crats))| {
            let c = state.countries.get(t);
            let levels: f64 = c.map_or(0.0, |c| c.reforms.values().filter_map(|o| rules.option_levels.get(o)).sum());
            let bonus: f64 = c.map_or(0.0, |c| c.technologies.iter().filter_map(|n| rules.tech_bonus.get(n)).sum::<f64>() + c.reforms.values().chain(&c.national_value).chain(c.modifiers.iter().map(|m| &m.name)).filter_map(|n| rules.modifier_bonus.get(n)).sum::<f64>());
            let required = max_bureaucracy + rules.increment * levels;
            (t.to_string(), (if all > 0.0 && required > 0.0 { crats / all * (1.0 + bonus) / required } else { 0.0 }).clamp(0.0, 1.0))
        })
        .collect()
}

/// A weight written as `factor = base` times the factor of every `modifier` whose conditions
/// hold (how attractive a migration target is). Unknown conditions are left out.
pub fn product_weight(state: &State, block: &Block, scope: &Scope) -> f64 {
    // Also used for a technology's `ai_chance`.
    let mut w = block.get("factor").and_then(Value::as_f64).unwrap_or(1.0);
    for m in block.get_all("modifier").filter_map(Value::as_block) {
        if conditions_hold(state, m, scope) == Ok(true) {
            w *= m.get("factor").and_then(Value::as_f64).unwrap_or(1.0);
        }
    }
    w.max(0.0)
}

/// Runs `f` on every province, split into contiguous runs over a few threads, and returns what
/// it pushed in province order: the same list a plain loop gives, whatever the thread timing.
/// `f` only reads the state.
pub(crate) fn par_provinces<T: Send>(state: &State, f: impl Fn(u16, &ProvinceState, &mut Vec<T>) + Sync) -> Vec<T> {
    let all: Vec<(&u16, &ProvinceState)> = state.provinces.iter().collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 16);
    let chunk = all.len().div_ceil(threads).max(64);
    let f = &f;
    std::thread::scope(|s| {
        let handles: Vec<_> = all
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    let mut out = Vec::new();
                    for (id, p) in part {
                        f(**id, p, &mut out);
                    }
                    out
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("province worker panicked")).collect()
    })
}

/// `par_provinces` over the provinces that have a pop due today only, on no more threads than the
/// pops due are worth (few are due on one day, and starting a thread costs as much as some dozens
/// of pop updates). The same list a plain loop over all provinces gives, provided `f` does
/// nothing in a province without a pop due.
fn par_due_provinces<T: Send>(state: &State, today: DueToday, f: impl Fn(u16, &ProvinceState, &mut Vec<T>) + Sync) -> Vec<T> {
    let mut pops = 0;
    let mut has_due = |p: &ProvinceState| {
        let n = p.pops.iter().filter(|q| today.is_due(q.id)).count();
        pops += n;
        n > 0
    };
    let all: Vec<(u16, &ProvinceState)> = state.provinces.iter().map(|(id, p)| (*id, p)).filter(|(_, p)| has_due(p)).collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 16).min(pops / DUE_POPS_PER_THREAD);
    if threads <= 1 {
        let mut out = Vec::new();
        for (id, p) in all {
            f(id, p, &mut out);
        }
        return out;
    }
    // Each thread takes the next province not yet taken (the work per province is uneven: a
    // share of the list each would leave some threads idle) and keeps what `f` pushed for it
    // under the province's place in the list.
    let next = std::sync::atomic::AtomicUsize::new(0);
    let (f, all, next) = (&f, &all, &next);
    let mut parts: Vec<(usize, Vec<T>)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(move || {
                    let mut done = Vec::new();
                    while let Some((id, p)) = all.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed)) {
                        let mut out = Vec::new();
                        f(*id, p, &mut out);
                        if !out.is_empty() {
                            done.push((*id as usize, out));
                        }
                    }
                    done
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("province worker panicked")).collect()
    });
    parts.sort_by_key(|part| part.0);
    parts.into_iter().flat_map(|part| part.1).collect()
}

// Pops due on one day that make one more thread worth starting (speed only: no result depends on it).
const DUE_POPS_PER_THREAD: usize = 100;

/// The steps of the monthly ideology drift.
#[derive(Debug, Clone)]
pub struct IdeologyRules {
    /// Points (of 100) added to an ideology whose weight share is above the pop's share of it.
    pub step: f64,
    /// Points added instead when the weight share is above `big_ratio` times the pop's share.
    pub big_step: f64,
    pub big_ratio: f64,
}

impl IdeologyRules {
    pub fn new(m: &EngineRules) -> Result<IdeologyRules, String> {
        Ok(IdeologyRules { step: crate::num_field(m, "FLD_IDEOLOGY_STEP")?, big_step: crate::num_field(m, "FLD_IDEOLOGY_BIG_STEP")?, big_ratio: crate::num_field(m, "FLD_IDEOLOGY_BIG_RATIO")? })
    }
}

/// Scales shares so that they add up to 100 (left alone when they add up to nothing).
fn rescale(shares: &mut BTreeMap<u32, f64>) {
    let sum: f64 = shares.values().sum();
    if sum > 0.0 {
        shares.values_mut().for_each(|x| *x *= 100.0 / sum);
    }
}

/// One pop's ideology shares (points of 100) after a month. `weights` holds (ideology number,
/// weight) of the ideologies open to the pop; their weights, taken as shares of their sum, are
/// the targets. An open ideology whose target is above `big_ratio` times the pop's share gains
/// `big_step`, one whose target is above the share `step`; one whose target is below the share
/// divided by `big_ratio` LOSES `big_step` (not below 0); the others stay. Ideologies that are
/// not open are not stepped. Then all shares are scaled to add up to 100. All comparisons are
/// strict (read from the original's routine; the gains measured).
pub fn ideology_drift(rules: &IdeologyRules, shares: &BTreeMap<u32, f64>, weights: &[(u32, f64)]) -> BTreeMap<u32, f64> {
    let (total, weight_total): (f64, f64) = (shares.values().sum(), weights.iter().map(|w| w.1).sum());
    if total <= 0.0 {
        return shares.clone();
    }
    let mut out = shares.clone();
    for (n, w) in weights {
        let (mine, target) = (shares.get(n).copied().unwrap_or(0.0), if weight_total > 0.0 { w * 100.0 / weight_total } else { 0.0 });
        let change = if rules.big_ratio * mine < target { rules.big_step } else if mine < target { rules.step } else if target < mine / rules.big_ratio { -rules.big_step } else { 0.0 };
        if change != 0.0 {
            let x = out.entry(*n).or_default();
            *x = (*x + change).max(0.0);
        }
    }
    rescale(&mut out);
    out
}

/// Ideology drift of every pop due today. Open to a pop are the ideologies whose date has come
/// (or that an effect enabled), without those marked `uncivilized = no` in an uncivilized
/// country. Returns the pops updated.
pub fn drift_due_pops(state: &mut State, defs: &MobilityDefs, rules: &IdeologyRules, schedule: &PopSchedule, cal: &Calendar) -> usize {
    let today = DueToday::new(schedule, state.date.day, cal.month_len(state.date.month));
    let open: Vec<bool> = state.ideologies.iter().map(|i| state.enabled_ideologies.contains(i) || state.ideology_dates.get(i).is_none_or(|d| state.date >= *d)).collect();
    let st: &State = state;
    let compiled = defs.compiled();
    let (facts, memo) = (Facts::new(), Memo::new());
    let changes = par_due_provinces(st, today, |id, p, changes| {
        let Some(c) = p.owner.as_ref().and_then(|o| st.countries.get(o)) else { return };
        for (i, pop) in p.pops.iter().enumerate().filter(|(_, q)| today.is_due(q.id)) {
            let Some(lists) = compiled.ideologies.get(&pop.pop_type) else { continue };
            let scope = Scope::Pop(id, i);
            let at = Asked::new(st, &scope);
            let weights: Vec<(u32, f64)> = lists.iter().filter_map(|(name, w)| {
                let n = st.ideologies.iter().position(|x| x == name)?;
                (open[n] && (c.civilized || !defs.civilized_only.contains(name))).then(|| (n as u32 + 1, w.weight(st, Some(&facts), &memo, &at)))
            }).collect();
            changes.push((id, i, ideology_drift(rules, &pop.ideology, &weights)));
        }
    });
    let n = changes.len();
    for (id, i, shares) in changes {
        if let Some(pop) = state.provinces.get_mut(&id).and_then(|p| p.pops.get_mut(i)) {
            pop.ideology = shares;
        }
    }
    n
}

// ---------------------------------------------------------------- issues

// rules file: FLD_ISSUE_STEP - points of 100 an issue option moves a month before the country's issue_change_speed (a literal of the original's routine)
const ISSUE_STEP: f64 = 0.05;
// rules file: FLD_ISSUE_BIG_STEP - points gained instead when the target is above FLD_ISSUE_BIG_RATIO times the support (literal)
const ISSUE_BIG_STEP: f64 = 0.25;
// rules file: FLD_ISSUE_BIG_RATIO - see FLD_ISSUE_BIG_STEP (literal)
const ISSUE_BIG_RATIO: f64 = 2.0;
// rules file: FLD_ISSUE_FULL_STEP - points gained when the target is above FLD_ISSUE_FULL_RATIO times the support; not scaled by issue_change_speed (literal)
const ISSUE_FULL_STEP: f64 = 1.0;
// rules file: FLD_ISSUE_FULL_RATIO - see FLD_ISSUE_FULL_STEP (literal)
const ISSUE_FULL_RATIO: f64 = 5.0;

/// What kind of option a pop may hold a wish for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueKind {
    Party,
    Political,
    Social,
    /// A reform of an uncivilised country: listed, but its support is always wiped.
    Other,
}

/// The country side of the monthly issue rule.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IssueLists {
    /// Options a pop of this country may hold, by issue number: every party option but the
    /// first, and the next step of each of its reform groups.
    pub candidates: Vec<(u32, IssueKind)>,
    /// Reform options whose support is wiped every month (all but the first of them).
    pub excluded: Vec<u32>,
    /// 1 + the country's `issue_change_speed`, `political_reform_desire`, `social_reform_desire`.
    pub speed: f64,
    pub political: f64,
    pub social: f64,
    pub civilized: bool,
}

/// The two lists of a country (read from the original's routine). Candidates are every party
/// option and, for each political and social reform group of a civilised country (each economic
/// and military reform group of an uncivilised one), the option that follows the one in force;
/// every other option of those groups is excluded. The original fills both lists from the first
/// place but reads them from the second, so the first party option in load order is never
/// stepped and the first excluded option is never wiped: both are left out here. (The order of
/// the excluded list is taken to be the order of the issues file.) A group the country has no
/// option of is left alone. Not modelled: the political option that stays a candidate while in
/// force (a flag of the option the description could not name).
pub fn issue_lists(state: &State, c: &CountryState) -> IssueLists {
    let mut lists = IssueLists { speed: 1.0, political: 1.0, social: 1.0, civilized: c.civilized, ..Default::default() };
    let (mut first_party, mut first_excluded) = (true, true);
    let mut group_start = 0;
    // The place within its group of the option in force, for the group that starts at the number.
    let mut in_force: Option<(usize, Option<usize>)> = None;
    for (i, (_, group)) in state.issues.iter().enumerate() {
        if i > 0 && state.issues[i - 1].1 != *group {
            group_start = i;
        }
        let kind = match state.issue_categories.get(i).map(String::as_str) {
            Some("party_issues") => IssueKind::Party,
            Some("political_reforms") if c.civilized => IssueKind::Political,
            Some("social_reforms") if c.civilized => IssueKind::Social,
            Some("economic_reforms" | "military_reforms") if !c.civilized => IssueKind::Other,
            _ => continue,
        };
        if kind == IssueKind::Party {
            if !std::mem::take(&mut first_party) {
                lists.candidates.push((i as u32 + 1, kind));
            }
            continue;
        }
        if in_force.is_none_or(|(start, _)| start != group_start) {
            in_force = Some((group_start, c.reforms.get(group).and_then(|now| state.issues[group_start..].iter().take_while(|x| x.1 == *group).position(|x| x.0 == *now))));
        }
        let Some(current) = in_force.and_then(|(_, place)| place) else { continue };
        if i == group_start + current + 1 {
            lists.candidates.push((i as u32 + 1, kind));
        } else if !std::mem::take(&mut first_excluded) {
            lists.excluded.push(i as u32 + 1);
        }
    }
    lists
}

/// One pop's issue shares (points of 100) after a month (read from the original's routine and
/// checked on its saves for party options). `weights` holds the pop type's weight for each
/// candidate of `lists`, in the same order; taken as shares of their sum they are the targets.
/// * Every excluded option loses all its support.
/// * A candidate moves up when its target is at or above its support, else down. Up by
///   FLD_ISSUE_FULL_STEP when the target is above 5 times the support, by FLD_ISSUE_BIG_STEP x
///   speed when above twice, else by FLD_ISSUE_STEP x speed; down always by the small step. A
///   political reform option moves `political` times as far, a social one `social` times.
/// * In an uncivilised country only party options keep support.
/// * Then all shares are scaled to add up to 100.
///
/// An option with no weight and no support therefore gains the small step one month and loses
/// it the next: the flicker of reform wishes seen on the saves.
pub fn issue_drift(lists: &IssueLists, shares: &BTreeMap<u32, f64>, weights: &[f64]) -> BTreeMap<u32, f64> {
    let weight_total: f64 = weights.iter().sum();
    let mut out = shares.clone();
    for n in &lists.excluded {
        out.remove(n);
    }
    for ((n, kind), w) in lists.candidates.iter().zip(weights) {
        if *kind == IssueKind::Other || (!lists.civilized && *kind != IssueKind::Party) {
            out.remove(n);
            continue;
        }
        let (mine, target) = (out.get(n).copied().unwrap_or(0.0), if weight_total > 0.0 { w * 100.0 / weight_total } else { 0.0 });
        let desire = match kind {
            IssueKind::Political => lists.political,
            IssueKind::Social => lists.social,
            _ => 1.0,
        };
        let size = if ISSUE_FULL_RATIO * mine < target { ISSUE_FULL_STEP } else if ISSUE_BIG_RATIO * mine < target { ISSUE_BIG_STEP * lists.speed } else { ISSUE_STEP * lists.speed };
        let change = if target < mine { -ISSUE_STEP * lists.speed * desire } else { size * desire };
        let next = (mine + change).max(0.0);
        if next > 0.0 {
            out.insert(*n, next);
        } else {
            out.remove(n);
        }
    }
    rescale(&mut out);
    out
}

/// The monthly issue rule for every pop due today (`issue_lists`, `issue_drift`). The pop type's
/// weights are those of its file (`MobilityDefs::issues`), evaluated like ideology weights, with
/// `is_next_reform` answered by the script engine for the pop's country. The
/// speed and the two reform desires are the country's own modifier sums: in 1836 most countries
/// allow public meetings (`issue_change_speed = 1`), which gives the step of 0.1 seen on saves.
/// Returns the pops updated.
pub fn issues_due_pops(state: &mut State, defs: &MobilityDefs, schedule: &PopSchedule, cal: &Calendar, lib: &ModifierLibrary) -> usize {
    let today = DueToday::new(schedule, state.date.day, cal.month_len(state.date.month));
    let numbers: BTreeMap<&str, u32> = state.issues.iter().enumerate().map(|(i, (option, _))| (option.as_str(), i as u32 + 1)).collect();
    let blocks: BTreeMap<&str, BTreeMap<u32, &DeepProduct>> = defs.compiled().issues.iter().map(|(kind, list)| (kind.as_str(), list.iter().filter_map(|(name, b)| Some((*numbers.get(name.as_str())?, b))).collect())).collect();
    let mut lists: BTreeMap<&str, IssueLists> = BTreeMap::new();
    let sums = CountryModifiers::new(lib, ["issue_change_speed", "political_reform_desire", "social_reform_desire"]);
    // The country lists first (cheap), then the pops' weights in parallel.
    for p in state.provinces.values() {
        let Some((tag, c)) = p.owner.as_ref().and_then(|o| state.countries.get_key_value(o)) else { continue };
        if lists.contains_key(tag.as_str()) || !p.pops.iter().any(|q| today.is_due(q.id)) {
            continue;
        }
        let mut l = issue_lists(state, c);
        let [speed, political, social] = sums.of(c);
        (l.speed, l.political, l.social) = (1.0 + speed, 1.0 + political, 1.0 + social);
        lists.insert(tag.as_str(), l);
    }
    let st: &State = state;
    let (facts, memo) = (Facts::new(), Memo::new());
    let changes = par_due_provinces(st, today, |id, p, changes| {
        let Some(list) = p.owner.as_ref().and_then(|o| lists.get(o.as_str())) else { return };
        for (i, pop) in p.pops.iter().enumerate().filter(|(_, q)| today.is_due(q.id)) {
            let mine = blocks.get(pop.pop_type.as_str());
            let scope = Scope::Pop(id, i);
            let at = Asked::new(st, &scope);
            let weights: Vec<f64> = list.candidates.iter().map(|(n, _)| mine.and_then(|m| m.get(n)).map_or(0.0, |b| b.weight(st, Some(&facts), &memo, &at))).collect();
            changes.push((id, i, issue_drift(list, &pop.issues, &weights)));
        }
    });
    let n = changes.len();
    for (id, i, shares) in changes {
        if let Some(pop) = state.provinces.get_mut(&id).and_then(|p| p.pops.get_mut(i)) {
            pop.issues = shares;
        }
    }
    n
}

/// Assimilation of every pop due today: ASSIMILATION_SCALE x size x assimilation chance people
/// take the owner's primary culture. Slaves and pops of the primary or an accepted culture stay
/// as they are. Returns the people assimilated.
pub fn assimilate_due_pops(state: &mut State, defs: &MobilityDefs, rules: &MobilityRules, schedule: &PopSchedule, cal: &Calendar) -> f64 {
    let today = DueToday::new(schedule, state.date.day, cal.month_len(state.date.month));
    let mut moves = Vec::new();
    let chance_of = &defs.compiled().assimilation;
    let (facts, memo) = (Facts::new(), Memo::new());
    for (id, p) in &state.provinces {
        let Some(c) = p.owner.as_ref().and_then(|o| state.countries.get(o)) else { continue };
        let Some(primary) = c.primary_culture.as_ref() else { continue };
        for (i, pop) in p.pops.iter().enumerate().filter(|(_, q)| today.is_due(q.id)) {
            if state.slave_types.contains(&pop.pop_type) || pop.culture == *primary || c.accepted_cultures.contains(&pop.culture) {
                continue;
            }
            let scope = Scope::Pop(*id, i);
            let chance = chance_of.weight(state, Some(&facts), &memo, &Asked::new(state, &scope));
            let place = if p.colonial != 0 { rules.assimilation_colony } else { rules.assimilation_state };
            let people = (pop.size * rules.assimilation_scale * chance * place).trunc();
            if people >= 1.0 {
                moves.push(Move { from: (*id, i), to: *id, pop_type: None, culture: Some(primary.clone()), people, any_culture: false });
            }
        }
    }
    apply_moves(state, moves, rules.new_pop_mood).iter().map(|(_, m)| m.people).sum()
}

/// Migration targets, worked out once a month.
#[derive(Debug, Clone, Default)]
pub struct TargetCache {
    month: (i32, u8),
    /// Owner -> pop type -> the provinces that attract migrants with their weights, best first.
    provinces: BTreeMap<String, BTreeMap<String, Vec<(u16, f64)>>>,
    /// Pop type -> every country that owns land, with its score.
    countries: BTreeMap<String, Vec<(String, f64)>>,
    /// Country -> the countries it shares a land border with (as the neighbour lists have them).
    borders: Option<BTreeMap<String, std::collections::BTreeSet<String>>>,
}

// rules file: FLD_MIGRATION_CHUNK - a province gets its share of a country's migrants only when the share is above this many people; the rest go to one province (read in the original's routine)
const MIGRATION_CHUNK: f64 = 10.0;
// rules file: FLD_MIGRATION_SCORE_FLOOR - the least score of a country that emigrants may choose (read in the original's routine)
const MIGRATION_SCORE_FLOOR: f64 = 0.01;

impl TargetCache {
    /// The provinces of `owner` outside colonial states, each weighed by the pop type's
    /// `migration_target` x (1 + the immigration focus of its state) x (1 + its
    /// `immigrant_attract` modifiers); those with a positive weight, the best first. Worked out
    /// from the state as it is at the first call of the month.
    fn provinces_for(&mut self, state: &State, defs: &MobilityDefs, lib: &ModifierLibrary, owner: &str, kind: &str) -> &[(u16, f64)] {
        if !self.provinces.get(owner).is_some_and(|m| m.contains_key(kind)) {
            let country = state.countries.get(owner);
            let focus = |p: &ProvinceState| p.region.and_then(|r| country?.national_focus.get(&r)).and_then(|f| defs.focus_attract.get(f)).copied().filter(|x| *x > 0.0).unwrap_or(0.0);
            let attract = |p: &ProvinceState| p.modifiers.iter().map(|m| lib.event_modifiers.get(&m.name).and_then(|v| v.get("immigrant_attract")).copied().unwrap_or(0.0)).sum::<f64>();
            let (facts, memo) = (Facts::new(), Memo::new());
            let weight = |id: u16, w: &DeepProduct| {
                let scope = Scope::Province(id);
                w.weight(state, Some(&facts), &memo, &Asked::new(state, &scope))
            };
            let mut list: Vec<(u16, f64)> = match defs.compiled().migration_target.get(kind) {
                Some(w) => state.provinces.iter().filter(|(_, p)| p.owner.as_deref() == Some(owner) && p.colonial == 0).map(|(id, p)| (*id, weight(*id, w) * (1.0 + focus(p)) * (1.0 + attract(p)))).filter(|x| x.1 > 0.0).collect(),
                None => Vec::new(),
            };
            list.sort_by(|a, b| b.1.total_cmp(&a.1));
            self.provinces.entry(owner.to_string()).or_default().insert(kind.to_string(), list);
        }
        self.provinces.get(owner).and_then(|m| m.get(kind)).map_or(&[], Vec::as_slice)
    }
}

/// Every country that owns land with its score for emigrants of a pop type: the type's
/// `country_migration_target`, at least FLD_MIGRATION_SCORE_FLOOR, x (1 + the country's
/// `global_immigrant_attract`). Kept in `countries` (`TargetCache::countries`) for the month.
fn country_scores<'c>(countries: &'c mut BTreeMap<String, Vec<(String, f64)>>, state: &State, defs: &MobilityDefs, lib: &ModifierLibrary, kind: &str, facts: &Facts, memo: &Memo) -> &'c [(String, f64)] {
    if !countries.contains_key(kind) {
        let list: Vec<(String, f64)> = match defs.compiled().country_migration_target.get(kind) {
            Some(w) => state
                .existing()
                .into_iter()
                .map(|t| {
                    let attract = state.countries.get(&t).map_or(0.0, |c| country_modifier(c, lib, "global_immigrant_attract"));
                    let scope = Scope::Country(t);
                    let score = w.weight(state, Some(facts), memo, &Asked::new(state, &scope)).max(MIGRATION_SCORE_FLOOR) * (1.0 + attract);
                    let Scope::Country(t) = scope else { unreachable!() };
                    (t, score)
                })
                .collect(),
            None => Vec::new(),
        };
        countries.insert(kind.to_string(), list);
    }
    countries.get(kind).map_or(&[], Vec::as_slice)
}

/// Whether two countries share a land border. The list of borders is made at the first call
/// (`TargetCache::borders`).
fn share_border(borders: &mut Option<BTreeMap<String, std::collections::BTreeSet<String>>>, state: &State, a: &str, b: &str) -> bool {
    let borders = borders.get_or_insert_with(|| {
        let owner = |id: &u16| state.provinces.get(id).and_then(|p| p.owner.as_deref());
        let mut map: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
        for (x, y) in state.neighbours.iter().flat_map(|(id, near)| near.iter().filter_map(move |n| Some((owner(id)?, owner(n)?)))).filter(|(x, y)| x != y) {
            if !map.get(x).is_some_and(|set| set.contains(y)) {
                map.entry(x.to_string()).or_default().insert(y.to_string());
            }
        }
        map
    });
    let listed = |x: &str, y: &str| borders.get(x).is_some_and(|set| set.contains(y));
    listed(a, b) || listed(b, a)
}

/// Migrants of one kind on their way into a country: its migrant pool.
struct Pool {
    who: PopState,
    people: f64,
    money: f64,
}

/// Migration of every pop due today (read from the original's routines; not measured pop by
/// pop). Not for slaves, not out of colonial provinces.
/// * A pop of fewer than FLD_MIGRATION_WHOLE_POP people leaves WHOLE, whatever its chance,
///   unless it is the only pop of its province or the state's focus encourages its type.
/// * The others: IMMIGRATION_SCALE x size x chance people move within the country (chance x
///   (1 + the province's `immigrant_push`)), and from a civilised country as many by the
///   emigration chance go abroad, with the push counted twice when it is positive: (1 + push)
///   squared. The chances keep the port's own reading (the block's factor plus the modifiers
///   that hold, times FLD_MIGRATION_TUNE / FLD_EMIGRATION_TUNE).
/// * A lot moves only when it is more than FLD_POP_MOVE_THRESHOLD people or the whole pop.
/// * Emigrants of one pop all go to ONE country, drawn by score among the FLD_MIGRATION_TARGETS
///   best. Out of the draw: the pop's own country, countries of the other civilised status, and
///   countries whose capital is on the continent of the own capital unless they share a border.
/// * Nobody is sent to a province directly. The people join the receiving country's pool, and
///   each pool is spread the same day: going down the provinces by weight, a province gets
///   people left x weight / sum of weights when that is more than FLD_MIGRATION_CHUNK; what is
///   left goes to one province picked at random among those with a weight (when a country has
///   none for the pop type, nobody leaves for it).
///
/// Not modelled: migration to the colonies, the test on the culture group for emigration, the
/// slaves that follow. Returns (people who left for a pool of their own country, people who
/// changed country by (from, to)).
pub fn migrate_due_pops(state: &mut State, defs: &MobilityDefs, rules: &MobilityRules, schedule: &PopSchedule, cal: &Calendar, cache: &mut TargetCache, lib: &ModifierLibrary) -> (f64, BTreeMap<(String, String), f64>) {
    let today = DueToday::new(schedule, state.date.day, cal.month_len(state.date.month));
    if cache.month != (state.date.year, state.date.month) {
        *cache = TargetCache { month: (state.date.year, state.date.month), ..Default::default() };
    }
    let due = |amount: f64, size: f64| amount > MOVE_THRESHOLD || (amount > 0.0 && amount >= size);
    // Province, pop, owner, people moving within the country, people going abroad and the
    // countries they may choose.
    type Leaving = (u16, usize, String, f64, f64, Vec<(String, f64)>);
    // First the script chances of every pop due, on threads (they only read the state).
    let st: &State = state;
    let compiled = defs.compiled();
    let (facts, memo) = (Facts::new(), Memo::new());
    let chances: Vec<(u16, usize, f64, f64)> = par_due_provinces(st, today, |id, p, out| {
        let Some(c) = p.owner.as_ref().and_then(|o| st.countries.get(o)) else { return };
        if p.colonial != 0 {
            return;
        }
        let alone = p.pops.iter().filter(|q| q.size > 0.0).count() <= 1;
        let focus = p.region.and_then(|r| c.national_focus.get(&r)).and_then(|f| defs.focus.get(f));
        let push: f64 = p.modifiers.iter().map(|m| lib.event_modifiers.get(&m.name).and_then(|v| v.get("immigrant_push")).copied().unwrap_or(0.0)).sum();
        for (i, pop) in p.pops.iter().enumerate().filter(|(_, q)| today.is_due(q.id)) {
            if st.slave_types.contains(&pop.pop_type) || pop.size <= 0.0 {
                continue;
            }
            let small = pop.size < rules.whole_pop;
            if small && alone {
                continue;
            }
            let scope = Scope::Pop(id, i);
            let at = Asked::new(st, &scope);
            let amount = |chance: f64| if chance <= 0.0 { 0.0 } else { whole(pop.size * rules.immigration_scale * chance) };
            let (within, abroad) = if small && focus.is_none_or(|f| f.0 != pop.pop_type) {
                (pop.size, 0.0)
            } else {
                let within = amount(compiled.migration.weight(st, Some(&facts), &memo, &at) * rules.migration_tune * (1.0 + push));
                let twice = if push > 0.0 { (1.0 + push) * (1.0 + push) } else { 1.0 + push };
                (within, if c.civilized { amount(compiled.emigration.weight(st, Some(&facts), &memo, &at) * rules.emigration_tune * twice) } else { 0.0 })
            };
            out.push((id, i, within, abroad));
        }
    });
    // The state has not changed since the chances were read: the country scores made in this
    // loop may share their answers.
    let mut leaving: Vec<Leaving> = Vec::new();
    for (id, i, within, mut abroad) in chances {
        let p = &st.provinces[&id];
        let Some((owner, c)) = p.owner.as_ref().and_then(|o| st.countries.get_key_value(o)) else { continue };
        let continent = |c: &CountryState| c.capital.and_then(|id| st.provinces.get(&id)).and_then(|p| p.continent.as_deref());
        let pop = &p.pops[i];
        let mut targets = Vec::new();
        {
            if due(abroad, pop.size) {
                let home = continent(c);
                let scores = country_scores(&mut cache.countries, st, defs, lib, &pop.pop_type, &facts, &memo);
                // The countries in the draw, by their place in the list of scores.
                let mut open: Vec<(usize, f64)> = Vec::new();
                for (n, (tag, score)) in scores.iter().enumerate() {
                    let Some(other) = st.countries.get(tag).filter(|o| tag != owner && o.civilized == c.civilized) else { continue };
                    if continent(other) != home || home.is_none() || share_border(&mut cache.borders, st, owner, tag) {
                        open.push((n, *score));
                    }
                }
                open.sort_by(|a, b| b.1.total_cmp(&a.1));
                open.truncate(rules.targets);
                targets = open.into_iter().map(|(n, score)| (scores[n].0.clone(), score)).collect();
            }
            if targets.is_empty() {
                abroad = 0.0;
            }
            if due(within, pop.size) || due(abroad, pop.size) {
                leaving.push((id, i, owner.clone(), if due(within, pop.size) { within } else { 0.0 }, if due(abroad, pop.size) { abroad } else { 0.0 }, targets));
            }
        }
    }
    drop((facts, memo));
    let (mut within, mut abroad) = (0.0, BTreeMap::new());
    let mut pools: BTreeMap<(String, String, String, String), Pool> = BTreeMap::new();
    for (id, i, owner, inside, outside, targets) in leaving {
        // The country the emigrants go to: one draw by score. A country without a province that
        // attracts the pop type takes nobody (in the original they would return home).
        let total: f64 = targets.iter().map(|t| t.1).sum();
        let mut pick = if targets.is_empty() { 0.0 } else { state.roll_fraction() * total };
        let country = targets.iter().find(|(_, w)| { pick -= w; pick < 0.0 }).or(targets.last()).map(|t| t.0.clone());
        let kind = state.provinces[&id].pops[i].pop_type.clone();
        let home = Some(owner.clone()).filter(|t| !cache.provinces_for(state, defs, lib, t, &kind).is_empty());
        let country = country.filter(|t| !cache.provinces_for(state, defs, lib, t, &kind).is_empty());
        for (people, to) in [(inside, home), (outside, country)] {
            let Some(to) = to else { continue };
            let Some(pop) = state.provinces.get_mut(&id).and_then(|p| p.pops.get_mut(i)) else { continue };
            let people = people.min(pop.size);
            if people <= 0.0 {
                continue;
            }
            let money = pop.money * people / pop.size;
            (pop.size, pop.money) = (pop.size - people, pop.money - money);
            if to == owner { within += people } else { *abroad.entry((owner.clone(), to.clone())).or_insert(0.0) += people }
            let pool = pools.entry((to, pop.pop_type.clone(), pop.culture.clone(), pop.religion.clone())).or_insert_with(|| Pool { who: pop.clone(), people: 0.0, money: 0.0 });
            (pool.people, pool.money) = (pool.people + people, pool.money + money);
        }
    }
    let mut next_id = next_pop_id(state);
    for ((country, kind, _, _), pool) in pools {
        let list = cache.provinces_for(state, defs, lib, &country, &kind);
        let total: f64 = list.iter().map(|x| x.1).sum();
        let mut left = pool.people;
        let mut placed: Vec<(u16, f64)> = Vec::new();
        for (to, w) in list {
            if left <= MIGRATION_CHUNK {
                break;
            }
            let n = (left * w / total).trunc();
            if n > MIGRATION_CHUNK {
                placed.push((*to, n));
                left -= n;
            }
        }
        if left > 0.0 && !list.is_empty() {
            placed.push((list[((state.roll_fraction() * list.len() as f64) as usize).min(list.len() - 1)].0, left));
        }
        for (to, n) in placed {
            settle(state, to, &pool.who, n, pool.money * n / pool.people, rules.new_pop_mood, &mut next_id);
        }
    }
    (within, abroad)
}

/// Removes pops that nobody is left in. Call at the end of a day, never between processes that
/// refer to pops by position.
pub fn drop_empty_pops(state: &mut State) {
    for p in state.provinces.values_mut() {
        p.pops.retain(|q| q.size >= 1.0);
    }
}

// ---------------------------------------------------------------- literacy

/// Literacy (measured on daily saves): at its update a pop's literacy changes by
/// FLD_LITERACY_SCALE x LITERACY_CHANGE_SPEED x the owner's education spending x the clergy
/// factor x (1 + education efficiency modifiers), held in 1/32768ths. The clergy factor is
/// (share of clergymen in the province - BASE_CLERGY_FOR_LITERACY) / (MAX_CLERGY_FOR_LITERACY -
/// BASE_CLERGY_FOR_LITERACY), the share above the base counted up to MAX_CLERGY_FOR_LITERACY
/// itself (so the factor reaches 0.04 / 0.035 = 1.14 at a share of 4.5%); below the base it is
/// negative and literacy falls, at a rate that spending and efficiency do not change (read from
/// the original's routine).
#[derive(Debug, Clone, PartialEq)]
pub struct LiteracyRules {
    pub scale: f64,
    pub speed: f64,
    pub base_clergy: f64,
    pub max_clergy: f64,
    pub min: f64,
    pub max: f64,
}

impl LiteracyRules {
    pub fn new(defines: &Defines, m: &EngineRules) -> Result<LiteracyRules, String> {
        let d = |k: &str| defines.get("pops", k).ok_or(format!("defines.pops.{k} missing"));
        Ok(LiteracyRules {
            scale: crate::num_field(m, "FLD_LITERACY_SCALE")?,
            speed: d("LITERACY_CHANGE_SPEED")?,
            base_clergy: d("BASE_CLERGY_FOR_LITERACY")?,
            max_clergy: d("MAX_CLERGY_FOR_LITERACY")?,
            min: crate::num_field(m, "FLD_POP_LITERACY_MIN")?,
            max: crate::num_field(m, "FLD_POP_LITERACY_MAX")?,
        })
    }
}

/// The literacy change of one update.
pub fn literacy_change(rules: &LiteracyRules, clergy_share: f64, education_spending: f64, efficiency: f64) -> f64 {
    let clergy = (clergy_share - rules.base_clergy).min(rules.max_clergy) / (rules.max_clergy - rules.base_clergy);
    let factor = if clergy > 0.0 { clergy * education_spending * (1.0 + efficiency) } else { clergy };
    units(rules.scale * rules.speed * factor) as f64 / UNITS
}

/// Education efficiency of a country: national value, chosen reforms and timed modifiers.
fn education_efficiency(c: &CountryState, lib: &ModifierLibrary) -> f64 {
    let key = "education_efficiency_modifier";
    let of = |values: Option<&wiktoria_data::modifiers::ModifierValues>| values.and_then(|v| v.get(key)).copied().unwrap_or(0.0);
    c.national_value.as_ref().map_or(0.0, |n| of(lib.national_values.get(n)))
        + c.reforms.values().map(|o| of(lib.issue_options.get(o))).sum::<f64>()
        + c.modifiers.iter().map(|m| of(lib.event_modifiers.get(&m.name))).sum::<f64>()
}

/// Applies the literacy change to every pop due today. Returns the pops updated.
pub fn teach_due_pops(state: &mut State, rules: &LiteracyRules, schedule: &PopSchedule, cal: &Calendar, lib: &ModifierLibrary) -> usize {
    let today = DueToday::new(schedule, state.date.day, cal.month_len(state.date.month));
    // Education spending and efficiency, worked out for a country when the first of its pops is due.
    let mut per_country: BTreeMap<&str, (f64, f64)> = BTreeMap::new();
    let countries = &state.countries;
    let mut updated = 0;
    for p in state.provinces.values_mut() {
        if !p.pops.iter().any(|q| today.is_due(q.id)) {
            continue;
        }
        let Some((tag, c)) = p.owner.as_ref().and_then(|o| countries.get_key_value(o)) else { continue };
        let (spending, efficiency) = per_country.entry(tag.as_str()).or_insert_with(|| (c.spending.get("education_spending").copied().unwrap_or(0.0), education_efficiency(c, lib)));
        let people: f64 = p.pops.iter().map(|q| q.size).sum();
        if people <= 0.0 {
            continue;
        }
        let clergy: f64 = p.pops.iter().filter(|q| q.pop_type == "clergymen").map(|q| q.size).sum();
        let change = literacy_change(rules, clergy / people, *spending, *efficiency);
        for pop in p.pops.iter_mut().filter(|q| today.is_due(q.id)) {
            pop.literacy = (pop.literacy + change).clamp(rules.min, rules.max);
            updated += 1;
        }
    }
    updated
}

// ---------------------------------------------------------------- consciousness and militancy

/// Pop mood at the monthly update (read from the original's routines; the militancy lines are
/// those of its own tooltip). A term counts only when it is beyond
/// FLD_MOOD_TERM_THRESHOLD (above +0.01 for the terms that raise, below -0.01 for those that
/// lower); the modifiers always count. Both values stay within 0..10.
///
/// Consciousness, with cf = CON_COLONIAL_FACTOR in a colonial province, else 1:
/// * CON_LITERACY x literacy x (1 + the country's `literacy_con_impact`) x cf; when that passes
///   the threshold it is multiplied by plurality / 100;
/// * CON_LUXURY_GOODS x luxury needs met x cf;
/// * CON_POOR_CLERGY (poor pops) or CON_MIDRICH_CLERGY (others) x the clergy share of the
///   province x cf;
/// * the province's `pop_consciousness_modifier`, the country's `global_...`, its `core_...`
///   outside colonial provinces and its `non_accepted_...` for a culture not accepted: these are
///   not multiplied by cf.
///
/// Militancy:
/// * the province's `pop_militancy_modifier`, the country's `global_...`, its `core_...` outside
///   colonial provinces;
/// * for a culture not accepted MIL_NON_ACCEPTED and the country's `non_accepted_...` modifier;
/// * MIL_NO_LIFE_NEED x how far life needs are below one half;
/// * MIL_HAS_EVERYDAY_NEED x how far everyday needs are above one half, or
///   MIL_LACK_EVERYDAY_NEED x how far below; MIL_HAS_LUXURY_NEED x luxury needs above one half;
/// * MIL_IDEOLOGY x the pop's conservative share (which ideologies count is the port's reading);
/// * MIL_RULING_PARTY x the pop's share of the ruling party's ideology;
/// * outside colonial provinces MIL_REQUIRE_REFORM x the share of the pop's issue support that
///   sits on political and social reform options (as it stood after last month's issue step);
/// * MIL_WAR_EXHAUSTION x the country's war exhaustion x the pop's support (0..1) for party
///   options weighed by their `war_exhaustion_effect`.
///
/// Not included: modifiers from technologies (among them the one on MIL_NON_ACCEPTED), the
/// ruling party's own issues, and the term for the leader of the pop's regiments.
#[derive(Debug, Clone, PartialEq)]
pub struct MoodRules {
    pub con_luxury: f64,
    pub con_literacy: f64,
    pub con_poor_clergy: f64,
    pub con_midrich_clergy: f64,
    pub con_colonial: f64,
    pub mil_no_life: f64,
    pub mil_lack_everyday: f64,
    pub mil_has_everyday: f64,
    pub mil_has_luxury: f64,
    pub mil_non_accepted: f64,
    pub mil_war_exhaustion: f64,
    pub mil_ideology: f64,
    pub mil_ruling_party: f64,
    pub mil_require_reform: f64,
    pub max: f64,
    /// By issue number - 1: the option belongs to a political or social reform.
    pub reform_options: Vec<bool>,
}

/// What the militancy of one pop depends on besides the pop itself.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MilitancyInputs {
    pub accepted: bool,
    pub colonial: bool,
    pub war_exhaustion: f64,
    /// The pop's issue support (0..1) weighed by each option's `war_exhaustion_effect`.
    pub war_support: f64,
    /// Share (0..1) of the pop that is conservative, and that shares the ruling party's ideology.
    pub conservative: f64,
    pub ruling: f64,
    /// Share (0..1) of its issue support that sits on political and social reform options.
    pub reform_want: f64,
    /// Sum of the modifiers that apply to this pop.
    pub modifiers: f64,
}

impl MoodRules {
    pub fn new(defines: &Defines, m: &EngineRules) -> Result<MoodRules, String> {
        let d = |k: &str| defines.get("pops", k).ok_or(format!("defines.pops.{k} missing"));
        Ok(MoodRules {
            con_luxury: d("CON_LUXURY_GOODS")?,
            con_literacy: d("CON_LITERACY")?,
            con_poor_clergy: d("CON_POOR_CLERGY")?,
            con_midrich_clergy: d("CON_MIDRICH_CLERGY")?,
            con_colonial: d("CON_COLONIAL_FACTOR")?,
            mil_no_life: d("MIL_NO_LIFE_NEED")?,
            mil_lack_everyday: d("MIL_LACK_EVERYDAY_NEED")?,
            mil_has_everyday: d("MIL_HAS_EVERYDAY_NEED")?,
            mil_has_luxury: d("MIL_HAS_LUXURY_NEED")?,
            mil_non_accepted: d("MIL_NON_ACCEPTED")?,
            mil_war_exhaustion: d("MIL_WAR_EXHAUSTION")?,
            mil_ideology: d("MIL_IDEOLOGY")?,
            mil_ruling_party: d("MIL_RULING_PARTY")?,
            mil_require_reform: d("MIL_REQUIRE_REFORM")?,
            reform_options: Vec::new(),
            max: crate::num_field(m, "FLD_POP_MIL_MAX")?.min(crate::num_field(m, "FLD_POP_CON_MAX")?),
        })
    }
}

impl MoodRules {
    /// Marks which issue options are political or social reforms (issue numbers follow file order).
    pub fn with_reforms(mut self, issues: &[wiktoria_data::defs::IssueGroup]) -> MoodRules {
        self.reform_options = issues.iter().flat_map(|g| g.options.iter().map(move |_| g.category == "political_reforms" || g.category == "social_reforms")).collect();
        self
    }
}

// rules file: FLD_MOOD_TERM_THRESHOLD - a term of the monthly militancy or consciousness change counts only beyond this (a literal of the original's routines)
const MOOD_THRESHOLD: f64 = 0.01;
// rules file: FLD_MOOD_NEEDS_MIDPOINT - needs met above this calm a pop, below it stir it (a literal of the original's routine)
const NEEDS_MIDPOINT: f64 = 0.5;

fn raises(x: f64) -> f64 {
    if x > MOOD_THRESHOLD { x } else { 0.0 }
}

fn lowers(x: f64) -> f64 {
    if x < -MOOD_THRESHOLD { x } else { 0.0 }
}

/// What the consciousness of one pop depends on besides the pop itself.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConsciousnessInputs {
    pub poor: bool,
    /// Clergymen as a share (0..1) of the province's people.
    pub clergy_share: f64,
    /// The country's plurality as a percentage.
    pub plurality: f64,
    pub colonial: bool,
    /// The country's `literacy_con_impact`.
    pub literacy_impact: f64,
    /// Sum of the modifiers that apply to this pop.
    pub modifiers: f64,
}

/// What one update adds to a pop's consciousness.
pub fn consciousness_change(rules: &MoodRules, pop: &PopState, x: &ConsciousnessInputs) -> f64 {
    let cf = if x.colonial { rules.con_colonial } else { 1.0 };
    let literacy = rules.con_literacy * pop.literacy * (1.0 + x.literacy_impact) * cf;
    let clergy = if x.poor { rules.con_poor_clergy } else { rules.con_midrich_clergy } * x.clergy_share * cf;
    (if literacy > MOOD_THRESHOLD { literacy * x.plurality / 100.0 } else { 0.0 }) + raises(rules.con_luxury * pop.luxury_needs * cf) + lowers(clergy) + x.modifiers
}

/// What one update adds to a pop's militancy.
pub fn militancy_change(rules: &MoodRules, pop: &PopState, x: &MilitancyInputs) -> f64 {
    let everyday = pop.everyday_needs - NEEDS_MIDPOINT;
    x.modifiers
        + if x.accepted { 0.0 } else { rules.mil_non_accepted }
        + raises((NEEDS_MIDPOINT - pop.life_needs) * rules.mil_no_life)
        + if everyday >= 0.0 { lowers(everyday * rules.mil_has_everyday) } else { raises(-everyday * rules.mil_lack_everyday) }
        + if pop.luxury_needs > NEEDS_MIDPOINT { lowers((pop.luxury_needs - NEEDS_MIDPOINT) * rules.mil_has_luxury) } else { 0.0 }
        + lowers(rules.mil_ideology * x.conservative)
        + lowers(rules.mil_ruling_party * x.ruling)
        + if x.colonial { 0.0 } else { raises(rules.mil_require_reform * x.reform_want) }
        + if x.war_exhaustion > 0.0 { raises(rules.mil_war_exhaustion * x.war_exhaustion * x.war_support) } else { 0.0 }
}

/// Sum of one modifier value over everything a country carries: national value, technology
/// school, inventions, chosen reforms and timed modifiers.
pub fn country_modifier(c: &CountryState, lib: &ModifierLibrary, key: &str) -> f64 {
    let of = |v: Option<&wiktoria_data::modifiers::ModifierValues>| v.and_then(|v| v.get(key)).copied().unwrap_or(0.0);
    of(c.national_value.as_ref().and_then(|n| lib.national_values.get(n)))
        + of(c.tech_school.as_ref().and_then(|n| lib.schools.get(n)))
        + c.inventions.iter().map(|n| of(lib.inventions.get((*n as usize).wrapping_sub(1)).map(|(_, v)| v))).sum::<f64>()
        + c.reforms.values().map(|o| of(lib.issue_options.get(o))).sum::<f64>()
        + c.modifiers.iter().map(|m| of(lib.event_modifiers.get(&m.name))).sum::<f64>()
}

/// `country_modifier` for several keys at once: what every invention and every reform option
/// gives for each key is read once here, not once per country.
pub struct CountryModifiers<'a, const N: usize> {
    lib: &'a ModifierLibrary,
    keys: [&'a str; N],
    inventions: Vec<[f64; N]>,
    options: BTreeMap<&'a str, [f64; N]>,
}

impl<'a, const N: usize> CountryModifiers<'a, N> {
    pub fn new(lib: &'a ModifierLibrary, keys: [&'a str; N]) -> CountryModifiers<'a, N> {
        let values = |v: &wiktoria_data::modifiers::ModifierValues| keys.map(|k| v.get(k).copied().unwrap_or(0.0));
        CountryModifiers { lib, keys, inventions: lib.inventions.iter().map(|(_, v)| values(v)).collect(), options: lib.issue_options.iter().map(|(name, v)| (name.as_str(), values(v))).collect() }
    }

    /// `country_modifier(c, lib, key)` for each key, in the order of the keys.
    pub fn of(&self, c: &CountryState) -> [f64; N] {
        let (value, school) = (c.national_value.as_ref().and_then(|n| self.lib.national_values.get(n)), c.tech_school.as_ref().and_then(|n| self.lib.schools.get(n)));
        let options: Vec<Option<&[f64; N]>> = c.reforms.values().map(|o| self.options.get(o.as_str())).collect();
        let timed: Vec<Option<&wiktoria_data::modifiers::ModifierValues>> = c.modifiers.iter().map(|m| self.lib.event_modifiers.get(&m.name)).collect();
        std::array::from_fn(|k| {
            let of = |v: Option<&wiktoria_data::modifiers::ModifierValues>| v.and_then(|v| v.get(self.keys[k])).copied().unwrap_or(0.0);
            of(value)
                + of(school)
                + c.inventions.iter().map(|n| self.inventions.get((*n as usize).wrapping_sub(1)).map_or(0.0, |v| v[k])).sum::<f64>()
                + options.iter().map(|v| v.map_or(0.0, |v| v[k])).sum::<f64>()
                + timed.iter().map(|v| of(*v)).sum::<f64>()
        })
    }
}

/// Applies both changes to every pop due today. Returns the pops updated.
pub fn mood_due_pops(state: &mut State, rules: &MoodRules, schedule: &PopSchedule, cal: &Calendar, lib: &ModifierLibrary) -> usize {
    let today = DueToday::new(schedule, state.date.day, cal.month_len(state.date.month));
    let strata = state.strata.clone();
    let number = |name: &str| state.ideologies.iter().position(|i| i == name).map(|i| i as u32 + 1);
    let conservative = number("conservative");
    // By issue number - 1: the option's `war_exhaustion_effect`.
    let war_effect: Vec<f64> = state.issues.iter().map(|(option, _)| lib.issue_options.get(option).and_then(|v| v.get("war_exhaustion_effect")).copied().unwrap_or(0.0)).collect();
    // Per country: its three militancy modifiers, the number of the ruling party's ideology, its
    // three consciousness modifiers and its `literacy_con_impact`.
    // Worked out for a country when the first of its pops is due.
    type Politics = ([f64; 3], Option<u32>, [f64; 3], f64);
    let party_ideologies = &state.party_ideologies;
    let sums = CountryModifiers::new(lib, ["global_pop_militancy_modifier", "core_pop_militancy_modifier", "non_accepted_pop_militancy_modifier", "global_pop_consciousness_modifier", "core_pop_consciousness_modifier", "non_accepted_pop_consciousness_modifier", "literacy_con_impact"]);
    let politics_of = |c: &CountryState| -> Politics {
        let [m0, m1, m2, a0, a1, a2, literacy_impact] = sums.of(c);
        let ruling = c.ruling_party.and_then(|n| party_ideologies.get((n as usize).wrapping_sub(1))).and_then(|name| number(name));
        ([m0, m1, m2], ruling, [a0, a1, a2], literacy_impact)
    };
    let mut politics: BTreeMap<&str, Politics> = BTreeMap::new();
    let countries = &state.countries;
    let mut updated = 0;
    for p in state.provinces.values_mut() {
        // Nothing below changes a province none of whose pops is due.
        if !p.pops.iter().any(|q| today.is_due(q.id)) {
            continue;
        }
        let Some((tag, c)) = p.owner.as_ref().and_then(|o| countries.get_key_value(o)) else { continue };
        let (modifiers, ruling, awareness, literacy_impact) = politics.entry(tag.as_str()).or_insert_with(|| politics_of(c));
        let people: f64 = p.pops.iter().map(|q| q.size).sum();
        if people <= 0.0 {
            continue;
        }
        // In a province the rebels own nothing changes consciousness and no modifier stirs.
        let rebel = p.owner.as_deref() == Some("REB");
        let clergy_share = p.pops.iter().filter(|q| q.pop_type == "clergymen").map(|q| q.size).sum::<f64>() / people;
        let colonial = p.colonial != 0;
        // `core_pop_..._modifier` applies in provinces that are not colonial: the original's own
        // name for it is "Militancy in Non-Colonial" (its localisation), not "in core provinces".
        let province_modifier: f64 = p.modifiers.iter().filter_map(|m| lib.event_modifiers.get(&m.name)).filter_map(|v| v.get("pop_militancy_modifier")).sum();
        let province_awareness: f64 = p.modifiers.iter().filter_map(|m| lib.event_modifiers.get(&m.name)).filter_map(|v| v.get("pop_consciousness_modifier")).sum();
        for pop in p.pops.iter_mut().filter(|q| today.is_due(q.id)) {
            let poor = strata.get(&pop.pop_type).is_none_or(|s| s == "poor");
            let accepted = c.primary_culture.as_deref() == Some(pop.culture.as_str()) || c.accepted_cultures.contains(&pop.culture);
            if !rebel {
                let inputs = ConsciousnessInputs { poor, clergy_share, plurality: c.plurality, colonial, literacy_impact: *literacy_impact, modifiers: province_awareness + awareness[0] + if colonial { 0.0 } else { awareness[1] } + if accepted { 0.0 } else { awareness[2] } };
                pop.consciousness = (pop.consciousness + consciousness_change(rules, pop, &inputs)).clamp(0.0, rules.max);
            }
            let share = |n: Option<u32>| n.and_then(|n| pop.ideology.get(&n)).copied().unwrap_or(0.0) / 100.0;
            let support: f64 = pop.issues.values().sum();
            let on = |test: &dyn Fn(usize) -> f64| if support > 0.0 { pop.issues.iter().map(|(n, x)| x * test((*n as usize).wrapping_sub(1))).sum::<f64>() / support } else { 0.0 };
            let inputs = MilitancyInputs {
                accepted,
                colonial,
                war_exhaustion: c.war_exhaustion,
                war_support: pop.issues.iter().map(|(n, x)| x * war_effect.get((*n as usize).wrapping_sub(1)).copied().unwrap_or(0.0)).sum::<f64>() / 100.0,
                conservative: share(conservative),
                ruling: share(*ruling),
                reform_want: on(&|i| if rules.reform_options.get(i).copied().unwrap_or(false) { 1.0 } else { 0.0 }),
                modifiers: if rebel { 0.0 } else { province_modifier + modifiers[0] + if colonial { 0.0 } else { modifiers[1] } } + if accepted { 0.0 } else { modifiers[2] },
            };
            pop.militancy = (pop.militancy + militancy_change(rules, pop, &inputs)).clamp(0.0, rules.max);
            updated += 1;
        }
    }
    updated
}

// ---------------------------------------------------------------- country values

/// Monthly drift of country values.
/// * Plurality rises by the average consciousness of the country's people times
///   FLD_PLURALITY_PER_CONSCIOUSNESS (measured: 0.052-0.059 a month at consciousness 2.4-2.7).
/// * Infamy moves by the `badboy` of the static modifier for peace or war (-0.1 / -0.033),
///   not below FLD_BADBOY_MIN. From the file; not measured (every country sampled stood at 0).
///
/// * Prestige rises by the `prestige` of the static modifier for the country's rank (great power
///   0.05, other civilized country 0.01) times one plus the `prestige` of its technologies.
///   Secondary powers (0.025 in the original) count as other civilized
///   countries: the port does not rank countries yet.
///
/// * Research points (the original's own tooltip): each day
///   (pop types + rank + literacy) x (1 + plurality + technologies + modifiers), where
///   - a pop type with `research_points` gives them in full at its `research_optimum` share of
///     the population, and in proportion below it (clergymen 3 at 2%, clerks 2 at 4%);
///   - rank is the `research_points` of the static modifier (great power 1.5, other civilized
///     country 1; secondary powers, 1.25 in the original, count as other civilized countries);
///   - literacy gives the `research_points` of `average_literacy` (4) times the literate share;
///   - plurality counts FLD_RESEARCH_PER_PLURALITY (2) times its value as a percentage;
///   - technologies add their `increase_research`, modifiers their `research_points_modifier`.
///
///   The rate is set on the 1st of the month; `research::research_day` spends it every day.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CountryRules {
    /// Pop types that give research points: name, points at the optimum, optimum share.
    pub research_pops: Vec<(String, f64, f64)>,
    pub research_great_power: f64,
    pub research_civilized: f64,
    pub research_literacy: f64,
    pub research_per_plurality: f64,
    /// `increase_research` of each technology that has one, by name.
    pub tech_research: BTreeMap<String, f64>,
    pub prestige_great_power: f64,
    pub prestige_civilized: f64,
    /// `prestige` of each technology that has one, by name.
    pub tech_prestige: BTreeMap<String, f64>,
    pub plurality_per_consciousness: f64,
    pub plurality_max: f64,
    pub badboy_peace: f64,
    pub badboy_war: f64,
    pub badboy_min: f64,
}

impl CountryRules {
    pub fn new(m: &EngineRules, lib: &ModifierLibrary) -> Result<CountryRules, String> {
        let badboy = |name: &str| lib.static_modifiers.get(name).and_then(|v| v.get("badboy")).copied().unwrap_or(0.0);
        Ok(CountryRules {
            plurality_per_consciousness: crate::num_field(m, "FLD_PLURALITY_PER_CONSCIOUSNESS")?,
            plurality_max: crate::num_field(m, "FLD_PLURALITY_MAX")?,
            badboy_peace: badboy("peace"),
            badboy_war: badboy("war"),
            badboy_min: crate::num_field(m, "FLD_BADBOY_MIN")?,
            prestige_great_power: lib.static_modifiers.get("great_power").and_then(|v| v.get("prestige")).copied().unwrap_or(0.0),
            prestige_civilized: lib.static_modifiers.get("civ_nation").and_then(|v| v.get("prestige")).copied().unwrap_or(0.0),
            tech_prestige: BTreeMap::new(),
            research_pops: Vec::new(),
            research_great_power: lib.static_modifiers.get("great_power").and_then(|v| v.get("research_points")).copied().unwrap_or(0.0),
            research_civilized: lib.static_modifiers.get("civ_nation").and_then(|v| v.get("research_points")).copied().unwrap_or(0.0),
            research_literacy: lib.static_modifiers.get("average_literacy").and_then(|v| v.get("research_points")).copied().unwrap_or(0.0),
            research_per_plurality: crate::num_field(m, "FLD_RESEARCH_PER_PLURALITY")?,
            tech_research: BTreeMap::new(),
        })
    }

    /// Reads `research_points` and `research_optimum` from one pop type file.
    pub fn with_pop_type(mut self, name: &str, root: &Block) -> CountryRules {
        if let (Some(points), Some(optimum)) = (root.get("research_points").and_then(Value::as_f64), root.get("research_optimum").and_then(Value::as_f64)) {
            self.research_pops.push((name.to_string(), points, optimum));
        }
        self
    }

    /// Research points a day. `shares` holds the share of the population (0..1) of each pop type,
    /// `rank` the points for the country's rank, `bonus` the sum of technologies and modifiers.
    pub fn research_per_day(&self, shares: &BTreeMap<String, f64>, rank: f64, literacy: f64, plurality: f64, bonus: f64) -> f64 {
        let pops: f64 = self.research_pops.iter().map(|(t, points, optimum)| points * (shares.get(t).copied().unwrap_or(0.0) / optimum).min(1.0)).sum();
        (pops + rank + self.research_literacy * literacy) * (1.0 + self.research_per_plurality * plurality / 100.0 + bonus)
    }

    pub fn with_technologies(mut self, technologies: &[wiktoria_data::defs::Technology]) -> CountryRules {
        self.tech_prestige = technologies.iter().filter_map(|t| Some((t.name.clone(), *t.values.get("prestige")?))).collect();
        self.tech_research = technologies.iter().filter_map(|t| Some((t.name.clone(), *t.values.get("increase_research")?))).collect();
        self
    }
}

/// The monthly step for every country that owns land. Call on the 1st of a month; `days` is
/// the length of the month that ended.
pub fn country_month(state: &mut State, rules: &CountryRules, lib: &ModifierLibrary, days: f64) {
    let mut mood: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    // Per country: literate people, and people of each pop type that gives research points.
    let mut learned: BTreeMap<String, (f64, BTreeMap<String, f64>)> = BTreeMap::new();
    for p in state.provinces.values() {
        let Some(owner) = p.owner.as_ref() else { continue };
        let e = mood.entry(owner.clone()).or_default();
        let l = learned.entry(owner.clone()).or_default();
        for q in &p.pops {
            e.0 += q.consciousness * q.size;
            e.1 += q.size;
            l.0 += q.literacy * q.size;
            if rules.research_pops.iter().any(|(t, _, _)| *t == q.pop_type) {
                *l.1.entry(q.pop_type.clone()).or_default() += q.size;
            }
        }
    }
    let great: std::collections::BTreeSet<String> = state.great_powers.iter().cloned().collect();
    let at_war: std::collections::BTreeSet<String> = state.wars.iter().flat_map(|(a, d)| a.iter().chain(d.iter())).cloned().collect();
    for (tag, (con, people)) in mood {
        let Some(c) = state.countries.get_mut(&tag) else { continue };
        if people > 0.0 {
            c.plurality = (c.plurality + con / people * rules.plurality_per_consciousness).clamp(0.0, rules.plurality_max);
        }
        let drift = if at_war.contains(&tag) { rules.badboy_war } else { rules.badboy_peace };
        c.badboy = (c.badboy + drift).max(rules.badboy_min);
        let rank = if great.contains(&tag) { rules.prestige_great_power } else if c.civilized { rules.prestige_civilized } else { 0.0 };
        c.prestige += rank * (1.0 + c.technologies.iter().filter_map(|t| rules.tech_prestige.get(t)).sum::<f64>());
        if people > 0.0 {
            let (literate, by_type) = learned.remove(&tag).unwrap_or_default();
            let shares = by_type.into_iter().map(|(t, n)| (t, n / people)).collect();
            let rank = if great.contains(&tag) { rules.research_great_power } else if c.civilized { rules.research_civilized } else { 0.0 };
            let bonus = c.technologies.iter().filter_map(|t| rules.tech_research.get(t)).sum::<f64>() + country_modifier(c, lib, "research_points_modifier");
            let _ = days;
            c.research_rate = rules.research_per_day(&shares, rank, literate / people, c.plurality, bonus);
        }
    }
}

// ---------------------------------------------------------------- inventions

/// Discovery of inventions. On the 1st of each month every country that owns
/// land rolls once for each invention it does not have and whose `limit` holds. The chance in
/// percent is the `base` of the invention's `chance` block plus the `factor` of every `modifier`
/// in it whose conditions hold (the original's tooltip lists them this way). A discovered
/// invention becomes active, so its modifiers count from then on, and a `plurality` in its
/// effect is paid out once, as 100 times the value in plurality points.
///
/// Not included: `shared_prestige`, and effects other than modifiers and plurality.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InventionRules {
    /// Per invention, in the game's numbering: its `limit`, its `chance` and the plurality it pays.
    pub inventions: Vec<(Option<Block>, Option<Block>, f64)>,
    pub plurality_scale: f64,
    pub plurality_max: f64,
}

impl InventionRules {
    pub fn new(m: &EngineRules, blocks: &[Block]) -> Result<InventionRules, String> {
        let part = |b: &Block, key: &str| b.get(key).and_then(Value::as_block).cloned();
        Ok(InventionRules {
            inventions: blocks.iter().map(|b| (part(b, "limit"), part(b, "chance"), part(b, "effect").and_then(|e| e.get("plurality").and_then(Value::as_f64)).unwrap_or(0.0))).collect(),
            plurality_scale: crate::num_field(m, "FLD_INVENTION_PLURALITY_SCALE")?,
            plurality_max: crate::num_field(m, "FLD_PLURALITY_MAX")?,
        })
    }
}

/// Chance in percent that a country discovers an invention this month.
pub fn invention_chance(state: &State, chance: &Block, scope: &Scope) -> f64 {
    let mut total = chance.get("base").and_then(Value::as_f64).unwrap_or(0.0);
    for item in &chance.items {
        if let (Some("modifier"), Some(m)) = (item.key.as_deref(), item.value.as_block())
            && conditions_hold(state, m, scope) == Ok(true)
        {
            total += m.get("factor").and_then(Value::as_f64).unwrap_or(0.0);
        }
    }
    total
}

/// The monthly roll for every country. Returns the discoveries as (country, invention number).
pub fn discover_inventions(state: &mut State, rules: &InventionRules) -> Vec<(String, u32)> {
    let owners: std::collections::BTreeSet<String> = state.provinces.values().filter_map(|p| p.owner.clone()).collect();
    let mut found = Vec::new();
    for tag in owners {
        let scope = Scope::Country(tag.clone());
        for (i, (limit, chance, _)) in rules.inventions.iter().enumerate() {
            let n = i as u32 + 1;
            if state.countries.get(&tag).is_none_or(|c| c.inventions.contains(&n)) {
                continue;
            }
            // An invention without a `limit` is not tied to a technology: left alone.
            let Some(limit) = limit else { continue };
            if conditions_hold(state, limit, &scope) != Ok(true) {
                continue;
            }
            let percent = chance.as_ref().map_or(0.0, |c| invention_chance(state, c, &scope));
            if state.roll_fraction() * 100.0 < percent {
                found.push((tag.clone(), n));
            }
        }
    }
    for (tag, n) in &found {
        let Some(c) = state.countries.get_mut(tag) else { continue };
        c.inventions.insert(*n);
        let plurality = rules.inventions[*n as usize - 1].2 * rules.plurality_scale;
        c.plurality = (c.plurality + plurality).clamp(0.0, rules.plurality_max);
    }
    found
}
