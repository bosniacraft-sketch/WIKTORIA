//! Building factories, once a day.
//!
//! From the files:
//! * a factory type can be built once it is `default_enabled` or a technology or invention of
//!   the country has `activate_building` for it;
//! * it costs the `goods_cost` of its building at today's prices and takes its `time`;
//! * the ruling party's economic policy says who may do what: `pop_build_factory`,
//!   `pop_expand_factory`, `pop_open_factory` (capitalists, who pay `factory_owner_cost` of
//!   the cost), `build_factory`, `expand_factory`, `open_factory`, `can_subsidise` (the state).
//!
//! The capitalists of a state run one project at a time (`Project`): a new factory, reopening
//! a shut one or enlarging one, drawn by `project_weights`; the type of a new factory by
//! `capitalist_type_score`. A project is dropped when it can no longer be done and carried out
//! when the state's investment fund covers its price.
//!
//! A government builds as the AI does: never at war, about every `IndustryAi::build_period`
//! days, one factory type for the whole country (`government_type_score`: what the world market
//! is short of and has the inputs for), one state (`government_state_value`: idle people), then
//! reopen, else enlarge, else build, each only with a treasury above the price. And it
//! subsidises (`subsidy_change`): every running factory while its budget is not in the red, no
//! longer after 30 loss days in a row.
//!
//! The port's own, or left out:
//! * the capitalists' money: every day `market_day` puts half the cash and the
//!   savings of all capitalists of a country into the project of its best placed state
//!   (`State::investment` is the money the project of a state holds);
//! * what a government builds or enlarges is paid at the order, x (2 - administrative
//!   efficiency), and the money is kept by the site (`economy::Construction`): it takes its goods
//!   out of the national stockpile, hands their value back to the treasury as they arrive, and
//!   opens when its time is up and nothing is lacking;
//! * the goods of a capitalists' project are not bought on the market (so no duty on them either);
//! * railways, the national focus bonus, foreign investment, the AI paying into pop projects,
//!   forts and naval bases, deleting and destroying factories, the technology term of the
//!   government's type score;
//! * the price of reopening is `IndustryAi::reopen_cost_share` of the building cost and goes into the
//!   factory's cash box; a factory "can be enlarged" when it runs without loss and at least
//!   FACTORY_UPGRADE_EMPLOYEE_FACTOR of its places are taken;
//! * a new factory opens with FLD_FACTORY_START_CASH_SHARE of the most cash it may hold.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::script::{self, Block, Value};
use wiktoria_data::world::World;

use crate::economy::FarmMoney;
use crate::script::{CountryState, FactoryState, State};

/// The one building project the capitalists of a state run at a time: a new
/// factory of a type, or reopening or enlarging the factory at an index of `State::factories`.
#[derive(Debug, Clone, PartialEq)]
pub enum Project {
    New(String),
    Reopen(usize),
    Expand(usize),
}

#[derive(Debug, Clone)]
pub struct FactoryKind {
    pub name: String,
    pub cost: Vec<(String, f64)>,
    pub days: f64,
    pub default_enabled: bool,
    pub max_level: f64,
}

#[derive(Debug, Clone, Default)]
pub struct Policy {
    pub build_factory: bool,
    pub expand_factory: bool,
    pub open_factory: bool,
    pub can_subsidise: bool,
    pub pop_build_factory: bool,
    pub pop_expand_factory: bool,
    pub pop_open_factory: bool,
    /// The state may pay into its capitalists' projects (`can_invest_in_pop_projects`).
    pub invest_in_pop_projects: bool,
    pub owner_cost: f64,
}

#[derive(Debug, Clone)]
pub struct IndustryRules {
    pub kinds: Vec<FactoryKind>,
    /// Technology name -> the factory types it activates; the same for inventions.
    pub by_technology: BTreeMap<String, Vec<String>>,
    pub by_invention: BTreeMap<String, Vec<String>>,
    /// Economic policy option -> what it allows.
    pub policies: BTreeMap<String, Policy>,
    /// By party number - 1: the party's economic policy.
    pub party_policy: Vec<Option<String>>,
    pub min_employment: f64,
    pub max_per_state: usize,
    /// Share of its places that must be taken before a factory is enlarged.
    pub upgrade_staffing: f64,
    pub start_cash: f64,
    /// A factory's `money` is this many times the unit prices use.
    pub money_scale: f64,
    pub owner_types: BTreeSet<String>,
    pub worker_types: BTreeSet<String>,
    /// The AI's building, subsidy and top-up numbers and the reopening cost (rules file).
    pub ai: IndustryAi,
}

fn read(path: &Path) -> Result<script::Document, String> {
    script::parse_bytes(&std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?).map_err(|e| format!("{}: {e}", path.display()))
}

fn text_files(dir: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let mut out: Vec<_> = std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "txt")).collect();
    out.sort();
    Ok(out)
}

fn activations(block: &Block) -> Vec<String> {
    block.get_all("activate_building").filter_map(Value::as_str).map(str::to_string).collect()
}

impl IndustryRules {
    pub fn new(install: &Path, m: &EngineRules, world: &World) -> Result<IndustryRules, String> {
        let buildings = read(&install.join(m.require_str("FLD_BUILDINGS_FILE")?))?;
        let kinds = buildings
            .root
            .entries()
            .filter_map(|(name, v, _)| {
                let b = v.as_block()?;
                (b.get("type").and_then(Value::as_str) == Some("factory")).then(|| FactoryKind {
                    name: name.to_string(),
                    cost: b.get("goods_cost").and_then(Value::as_block).map(|c| c.entries().filter_map(|(g, x, _)| Some((g.to_string(), x.as_f64()?))).collect()).unwrap_or_default(),
                    days: b.get("time").and_then(Value::as_f64).unwrap_or(365.0),
                    default_enabled: b.get("default_enabled").and_then(Value::as_bool) == Some(true),
                    max_level: b.get("max_level").and_then(Value::as_f64).unwrap_or(99.0),
                })
            })
            .collect();
        let mut by_technology = BTreeMap::new();
        for file in text_files(&install.join(m.require_str("FLD_TECHNOLOGY_DIR")?))? {
            for (name, v, _) in read(&file)?.root.entries() {
                let found = v.as_block().map(activations).unwrap_or_default();
                if !found.is_empty() {
                    by_technology.insert(name.to_string(), found);
                }
            }
        }
        let mut by_invention = BTreeMap::new();
        for file in text_files(&install.join(m.require_str("FLD_INVENTIONS_DIR")?))? {
            for (name, v, _) in read(&file)?.root.entries() {
                let found = v.as_block().and_then(|b| b.get("effect")).and_then(Value::as_block).map(activations).unwrap_or_default();
                if !found.is_empty() {
                    by_invention.insert(name.to_string(), found);
                }
            }
        }
        let issues = read(&install.join(m.require_str("FLD_ISSUES_FILE")?))?;
        let mut policies = BTreeMap::new();
        if let Some(group) = issues.root.get("party_issues").and_then(Value::as_block).and_then(|b| b.get("economic_policy")).and_then(Value::as_block) {
            for (name, v, _) in group.entries() {
                let Some(b) = v.as_block() else { continue };
                let rule = |k: &str| b.get("rules").and_then(Value::as_block).and_then(|r| r.get(k)).and_then(Value::as_bool) == Some(true);
                policies.insert(
                    name.to_string(),
                    Policy {
                        build_factory: rule("build_factory"),
                        expand_factory: rule("expand_factory"),
                        open_factory: rule("open_factory"),
                        can_subsidise: rule("can_subsidise"),
                        pop_build_factory: rule("pop_build_factory"),
                        pop_expand_factory: rule("pop_expand_factory"),
                        pop_open_factory: rule("pop_open_factory"),
                        invest_in_pop_projects: rule("can_invest_in_pop_projects"),
                        owner_cost: b.get("factory_owner_cost").and_then(Value::as_f64).unwrap_or(1.0),
                    },
                );
            }
        }
        let party_policy = world.countries.iter().flat_map(|c| c.country.parties.iter()).map(|p| p.policies.iter().find(|(g, _)| g == "economic_policy").map(|(_, o)| o.clone())).collect();
        let define = |section: &str, k: &str| world.defines.get(section, k).ok_or(format!("defines.{section}.{k} missing"));
        Ok(IndustryRules {
            kinds,
            by_technology,
            by_invention,
            policies,
            party_policy,
            min_employment: define("economy", "CAPITALIST_BUILD_FACTORY_STATE_EMPLOYMENT_PERCENT")?,
            max_per_state: crate::int_field(m, "FLD_FACTORY_MAX_PER_STATE")?.max(1) as usize,
            upgrade_staffing: define("economy", "FACTORY_UPGRADE_EMPLOYEE_FACTOR")?,
            start_cash: crate::num_field(m, "FLD_FACTORY_START_CASH_SHARE")? * define("economy", "MAX_FACTORY_MONEY_SAVE")? * crate::num_field(m, "FLD_POP_MONEY_EFFECT_SCALE")?,
            money_scale: crate::num_field(m, "FLD_POP_MONEY_EFFECT_SCALE")?,
            owner_types: world.defs.factory_types.iter().filter_map(|t| t.owner.as_ref()).map(|j| j.pop_type.clone()).collect(),
            worker_types: world.defs.factory_types.iter().flat_map(|t| t.employees.iter()).map(|j| j.pop_type.clone()).collect(),
            ai: IndustryAi::from_engine_rules(m)?,
        })
    }

    /// The factory types a country can build.
    pub fn enabled(&self, world: &World, c: &CountryState) -> BTreeSet<&str> {
        let mut out: BTreeSet<&str> = self.kinds.iter().filter(|k| k.default_enabled).map(|k| k.name.as_str()).collect();
        for t in &c.technologies {
            out.extend(self.by_technology.get(t).into_iter().flatten().map(String::as_str));
        }
        for n in &c.inventions {
            if let Some((name, _)) = world.defs.modifiers.inventions.get((*n as usize).wrapping_sub(1)) {
                out.extend(self.by_invention.get(name).into_iter().flatten().map(String::as_str));
            }
        }
        out.retain(|k| self.kinds.iter().any(|x| x.name == *k));
        out
    }

    /// By party number - 1: whether its economic policy lets pops build factories.
    pub fn party_pops_build(&self) -> Vec<bool> {
        self.party_policy.iter().map(|o| o.as_ref().and_then(|o| self.policies.get(o)).is_some_and(|p| p.pop_build_factory)).collect()
    }

    /// The economic policy of the country's ruling party (all false when it has none).
    pub fn policy(&self, c: &CountryState) -> Policy {
        c.ruling_party.and_then(|n| self.party_policy.get((n as usize).wrapping_sub(1))).and_then(|o| o.as_ref()).and_then(|o| self.policies.get(o)).cloned().unwrap_or_default()
    }
}

/// What building did: factories started by capitalists and by governments, levels added to
/// existing ones (started), building sites that opened, factories reopened, and subsidies
/// switched on and off.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IndustryMonth {
    pub started_by_pops: Vec<(String, String)>,
    pub started_by_states: Vec<(String, String)>,
    pub enlarged: usize,
    pub opened: usize,
    pub reopened: usize,
    pub subsidies_on: usize,
    pub subsidies_off: usize,
    /// Money governments paid into their capitalists' projects.
    pub topped_up: f64,
}

/// The AI's factory numbers, read from the rules file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IndustryAi {
    /// FLD_FACTORY_REOPEN_COST_SHARE: cost of reopening a shut factory as a share of its
    /// building cost (the port's own: the description only says there is a cost).
    pub reopen_cost_share: f64,
    /// FLD_AI_BUILD_PERIOD_DAYS: a government looks at building once in this many days on
    /// average.
    pub build_period: f64,
    /// FLD_AI_SUBSIDY_OFF_LOSS_DAYS / FLD_AI_SUBSIDY_REVIEW_LOSS_DAYS.
    pub subsidy_off_loss_days: u32,
    pub subsidy_review_loss_days: u32,
    /// FLD_AI_INPUT_SHORT_PENALTY / FLD_AI_OUTPUT_GLUT_PENALTY.
    pub input_short: f64,
    pub output_glut: f64,
    /// FLD_AI_TOP_UP_BONUS_NEW / FLD_AI_TOP_UP_BONUS_OLD.
    pub top_up_new: f64,
    pub top_up_old: f64,
}

impl IndustryAi {
    pub fn from_engine_rules(m: &EngineRules) -> Result<IndustryAi, String> {
        let f = |id: &str| crate::num_field(m, id);
        let days = |id: &str| crate::int_field(m, id).map(|v| v.max(0) as u32);
        Ok(IndustryAi {
            reopen_cost_share: f("FLD_FACTORY_REOPEN_COST_SHARE")?,
            build_period: f("FLD_AI_BUILD_PERIOD_DAYS")?,
            subsidy_off_loss_days: days("FLD_AI_SUBSIDY_OFF_LOSS_DAYS")?,
            subsidy_review_loss_days: days("FLD_AI_SUBSIDY_REVIEW_LOSS_DAYS")?,
            input_short: f("FLD_AI_INPUT_SHORT_PENALTY")?,
            output_glut: f("FLD_AI_OUTPUT_GLUT_PENALTY")?,
            top_up_new: f("FLD_AI_TOP_UP_BONUS_NEW")?,
            top_up_old: f("FLD_AI_TOP_UP_BONUS_OLD")?,
        })
    }
}

// rules file: FLD_CAPITALIST_PROFIT_CAP - daily margin of one level above which a factory type scores no better
pub const CAPITALIST_PROFIT_CAP: f64 = 100.0;
// rules file: FLD_CAPITALIST_DEMAND_CAP - demand over supply of the output above which a type scores no better
pub const CAPITALIST_DEMAND_CAP: f64 = 2.0;

/// How the capitalists weigh the kinds of project: a new factory counts 1
/// when the state has room, its factories are busy enough (`busy`; a state without factories
/// always is) and their party lets them build; reopening counts once per shut factory;
/// enlarging once per factory that can be enlarged, and not at all while the state has room
/// but is not busy. Returns (new, reopen, expand). Railways and the national focus bonus are
/// not in the port.
pub fn project_weights(policy: &Policy, room: bool, busy: bool, shut: usize, expandable: usize) -> (f64, f64, f64) {
    let new = if policy.pop_build_factory && room && busy { 1.0 } else { 0.0 };
    let reopen = if policy.pop_open_factory { shut as f64 } else { 0.0 };
    let expand = if policy.pop_expand_factory && !(room && !busy) { expandable as f64 } else { 0.0 };
    (new, reopen, expand)
}

/// The weight the capitalists give a factory type before its score: 1, times
/// 0.95 for each factory of the type in the country that runs without loss, 0.01 for each that
/// is shut or loses money unsubsidised, 0.5 for each state already planning one, 0.01 for each
/// being built.
pub fn capitalist_type_weight(sound: usize, failing: usize, planned: usize, building: usize) -> f64 {
    0.95f64.powi(sound as i32) * 0.01f64.powi(failing as i32) * 0.5f64.powi(planned as i32) * 0.01f64.powi(building as i32)
}

/// The capitalists' score of a factory type: its daily margin at today's
/// prices (at most 100) x how available its inputs are (the average over the inputs of world
/// supply less demand over the amount needed, at most 1; a type with an input the world is
/// short of scores 0) x demand over supply of its output (at most 2) x the type's weight.
/// `inputs`: (amount needed, supply, demand) of each; `output`: (supply, demand).
pub fn capitalist_type_score(margin: f64, inputs: &[(f64, f64, f64)], output: (f64, f64), weight: f64) -> f64 {
    if inputs.iter().any(|(_, supply, demand)| demand > supply) {
        return 0.0;
    }
    let available = if inputs.is_empty() { 1.0 } else { inputs.iter().map(|(need, supply, demand)| if *need > 0.0 { ((supply - demand) / need).min(1.0) } else { 1.0 }).sum::<f64>() / inputs.len() as f64 };
    let wanted = if output.0 > 0.0 { (output.1 / output.0).min(CAPITALIST_DEMAND_CAP) } else { CAPITALIST_DEMAND_CAP };
    margin.min(CAPITALIST_PROFIT_CAP) * available * wanted * weight
}

/// The government's score of a factory type: the average over its inputs of
/// price x (world supply - demand), or -100000 for an input the world is short of; plus price x
/// (demand - supply) of its output, and -10000 more when the output is oversupplied. Profit
/// and the country's own needs do not enter. Not in the port: the technology bonus term.
/// `inputs` and `output`: (price, supply, demand).
pub fn government_type_score(ai: &IndustryAi, inputs: &[(f64, f64, f64)], output: (f64, f64, f64)) -> f64 {
    let of_inputs = if inputs.is_empty() { 0.0 } else { inputs.iter().map(|(price, supply, demand)| if supply < demand { ai.input_short } else { price * (supply - demand) }).sum::<f64>() / inputs.len() as f64 };
    let short = output.2 - output.1;
    of_inputs + output.0 * short + if short < 0.0 { ai.output_glut } else { 0.0 }
}

/// The government's value of a state for a factory: the levels its idle people
/// could staff, halved while a factory of the state is being built or enlarged, doubled when
/// the plan is to enlarge, plus 100 x the capitalists' share of its people. Not in the port:
/// a term of four state values the description leaves open.
pub fn government_state_value(idle: f64, people: f64, owners: f64, workforce: f64, building: bool, expansion: bool) -> f64 {
    if people <= 0.0 || workforce <= 0.0 {
        return 0.0;
    }
    idle / workforce * if building { 0.5 } else { 1.0 } * if expansion { 2.0 } else { 1.0 } + 100.0 * owners / people
}

/// What the AI does with the subsidy of one running factory while its budget is not negative: off after 30 loss days in a row;
/// off from 7 loss days when the other places free in the state's factories exceed its workforce; otherwise on when the balance is
/// positive. `None` = no change.
pub fn subsidy_change(ai: &IndustryAi, balance: f64, loss_days: u32, subsidised: bool, free_places: f64, workforce: f64) -> Option<bool> {
    if loss_days >= ai.subsidy_off_loss_days {
        return subsidised.then_some(false);
    }
    if loss_days >= ai.subsidy_review_loss_days && subsidised && free_places > workforce {
        return Some(false);
    }
    (balance > 0.0 && !subsidised).then_some(true)
}

/// Picks an entry by its weight with a roll in 0..1; `None` when no weight is positive.
fn pick<T: Clone>(list: &[(T, f64)], roll: f64) -> Option<T> {
    let total: f64 = list.iter().map(|x| x.1.max(0.0)).sum();
    if total <= 0.0 {
        return None;
    }
    let mut at = roll * total;
    list.iter().filter(|x| x.1 > 0.0).find(|x| {
        at -= x.1;
        at < 0.0
    }).or(list.iter().rfind(|x| x.1 > 0.0)).map(|x| x.0.clone())
}

/// Which of its capitalists' projects a government pays into, and how much: of
/// the projects that still lack money (each given as money paid, full price, whether it is a
/// new factory) the one with the highest paid share plus its bonus; the payment is what is
/// missing, at most `budget_share` (POP_PROJECT_INVESTMENT_MAX_BUDGET_FACTOR) of the treasury.
pub fn project_top_up(ai: &IndustryAi, projects: &[(f64, f64, bool)], treasury: f64, budget_share: f64) -> Option<(usize, f64)> {
    let score = |p: &(f64, f64, bool)| p.0 / p.1 + if p.2 { ai.top_up_new } else { ai.top_up_old };
    let best = projects.iter().enumerate().filter(|(_, p)| p.1 > 0.0 && p.0 < p.1).max_by(|a, b| score(a.1).total_cmp(&score(b.1)))?;
    let amount = (best.1 .1 - best.1 .0).min(treasury * budget_share);
    (amount > 0.0).then_some((best.0, amount))
}

/// One day of building. `market` is what the day's market came to (`Farms::market_day`): the
/// world's supply and paid demand per good and every country's budget balance.
pub fn industry_day(state: &mut State, world: &World, rules: &IndustryRules, market: &FarmMoney) -> IndustryMonth {
    let mut out = IndustryMonth::default();
    // Building sites whose time is up open: a new factory at level 1, an old one a level higher.
    // A site of the state that still lacks goods waits for them.
    let waiting: BTreeSet<usize> = state.money.countries.values().flat_map(|m| m.constructions.iter()).map(|x| x.factory).collect();
    let mut opened: Vec<(String, Option<u16>)> = Vec::new();
    for (i, f) in state.factories.iter_mut().enumerate() {
        if let Some(left) = f.building_days {
            if left <= 1.0 && waiting.contains(&i) {
                continue;
            }
            if left <= 1.0 {
                (f.building_days, f.level) = (None, f.level + 1.0);
                if f.level <= 1.0 {
                    f.staff = Some(0.0);
                }
                opened.push((f.owner.clone(), f.region));
                out.opened += 1;
            } else {
                f.building_days = Some(left - 1.0);
            }
        }
    }
    for p in state.provinces.values_mut() {
        if p.owner.as_ref().is_some_and(|o| opened.iter().any(|(owner, region)| owner == o && *region == p.region)) {
            p.has_factory = true;
        }
    }

    // Per state of a country: people, factory workers, owners, idle people, whether it is a colony.
    #[derive(Default)]
    struct Land {
        people: f64,
        workers: f64,
        owners: f64,
        idle: f64,
        /// Factory-worker people not yet at work in a factory of the state.
        free_workers: f64,
        colonial: bool,
    }
    let mut lands: BTreeMap<(String, Option<u16>), Land> = BTreeMap::new();
    for p in state.provinces.values() {
        let Some(owner) = p.owner.as_ref() else { continue };
        let e = lands.entry((owner.clone(), p.region)).or_default();
        e.colonial |= p.colonial != 0;
        let rgo_type = p.trade_goods.as_ref().and_then(|g| state.rgo_workers.get(g));
        let mut on_the_land = 0.0;
        for q in &p.pops {
            e.people += q.size;
            if rules.worker_types.contains(&q.pop_type) {
                e.workers += q.size;
            } else if rules.owner_types.contains(&q.pop_type) {
                e.owners += q.size;
            } else if Some(&q.pop_type) == rgo_type {
                on_the_land += q.size;
            }
        }
        e.idle += (on_the_land - p.rgo_staff.unwrap_or(on_the_land)).max(0.0);
    }
    let type_of = |name: &str| world.defs.factory_types.iter().find(|t| t.name == name);
    let places_of = |f: &FactoryState| type_of(&f.kind).map_or(0.0, |t| t.workforce * f.level);
    let staff_of = |f: &FactoryState| if f.level < 1.0 || f.closed { 0.0 } else { f.staff.unwrap_or(f64::MAX).min(places_of(f)) };
    // What stands in each state, by index; its places and the people at work in them.
    let mut standing: BTreeMap<(String, Option<u16>), Vec<usize>> = BTreeMap::new();
    let mut work: BTreeMap<(String, Option<u16>), (f64, f64)> = BTreeMap::new();
    for (i, f) in state.factories.iter().enumerate() {
        let key = (f.owner.clone(), f.region);
        let e = work.entry(key.clone()).or_default();
        (e.0, e.1) = (e.0 + if f.closed { 0.0 } else { places_of(f) }, e.1 + staff_of(f));
        standing.entry(key).or_default().push(i);
    }
    for (key, land) in &mut lands {
        let (_, staffed) = work.get(key).copied().unwrap_or_default();
        land.free_workers = (land.workers - staffed).max(0.0);
        land.idle += land.free_workers;
    }
    let prices = state.prices.clone();
    let admin = crate::pops::admin_efficiency_shares(state, world.defines.get("country", "MAX_BUREAUCRACY_PERCENTAGE").unwrap_or(0.01));
    let price = |good: &str| prices.get(good).copied().unwrap_or(0.0);
    let supply = |good: &str| market.supply.get(good).copied().unwrap_or(0.0);
    let demand = |good: &str| market.demand.get(good).copied().unwrap_or(0.0);
    let cost_of = |name: &str| rules.kinds.iter().find(|k| k.name == name).map_or(0.0, |k| k.cost.iter().map(|(g, x)| x * price(g)).sum::<f64>());
    let kind_of = |name: &str| rules.kinds.iter().find(|k| k.name == name);
    let can_grow = |f: &FactoryState| f.level >= 1.0 && f.building_days.is_none() && kind_of(&f.kind).is_some_and(|k| f.level < k.max_level);
    let at_war: BTreeSet<String> = state.wars.iter().flat_map(|(a, d)| a.iter().chain(d.iter())).cloned().collect();

    enum Action {
        Plan((String, Option<u16>), Project),
        Drop((String, Option<u16>)),
        /// Carry out a project for a price; `true` = the government pays, else the state's fund.
        Do((String, Option<u16>), Project, f64, bool),
        Subsidy(usize, bool),
        /// The government pays this much into the project of a state.
        TopUp((String, Option<u16>), f64),
    }
    let mut actions: Vec<Action> = Vec::new();
    let keys: Vec<(String, Option<u16>)> = lands.keys().cloned().collect();
    // Draws made up front so that the plans below can borrow the state.
    let rolls: Vec<[f64; 2]> = keys.iter().map(|_| [state.roll_fraction(), state.roll_fraction()]).collect();
    let tags: BTreeSet<String> = keys.iter().map(|k| k.0.clone()).collect();
    let country_rolls: BTreeMap<String, f64> = tags.iter().map(|t| (t.clone(), state.roll_fraction())).collect();
    let invest_rolls: BTreeMap<String, f64> = tags.iter().map(|t| (t.clone(), state.roll_fraction())).collect();

    // The capitalists: one project per state.
    for (key, roll) in keys.iter().zip(&rolls) {
        let (land, Some(c)) = (&lands[key], state.countries.get(&key.0)) else { continue };
        let policy = rules.policy(c);
        // A computer player that may pay into pop projects plans them where it has no capitalists
        // too (measured: Russia, interventionist and without a single capitalist, has two new
        // factories building on 1838.1.1 and 14 levels by 1845; the original's 64-year run).
        let planned_by_state = !c.human && policy.invest_in_pop_projects;
        // Capitalists plan where factory workers are free to fill the places (stand-in,
        //); a state planning for them needs idle people only (Russia, Austria and
        // Turkey have no craftsmen in 1836 and build all the same).
        let staffable = land.free_workers > 0.0 || (planned_by_state && land.idle > 0.0);
        if land.colonial || key.1.is_none() {
            continue;
        }
        let none = Vec::new();
        let here = standing.get(key).unwrap_or(&none);
        let enabled = rules.enabled(world, c);
        let buildable = |name: &str| enabled.contains(name) && here.len() < rules.max_per_state && !here.iter().any(|i| state.factories[*i].kind == name);
        if let Some(project) = state.projects.get(key) {
            // Dropped when it can no longer be done; carried out when the fund covers it.
            let (valid, price) = match project {
                Project::New(kind) => (policy.pop_build_factory && buildable(kind), cost_of(kind)),
                Project::Reopen(i) => (policy.pop_open_factory && state.factories.get(*i).is_some_and(|f| f.closed), state.factories.get(*i).map_or(0.0, |f| cost_of(&f.kind) * rules.ai.reopen_cost_share)),
                Project::Expand(i) => (policy.pop_expand_factory && state.factories.get(*i).is_some_and(|f| can_grow(f) && !f.closed), state.factories.get(*i).map_or(0.0, |f| cost_of(&f.kind))),
            };
            let price = price * policy.owner_cost;
            if !valid || price <= 0.0 {
                actions.push(Action::Drop(key.clone()));
            } else if state.investment.get(key).copied().unwrap_or(0.0) >= price {
                actions.push(Action::Do(key.clone(), project.clone(), price, false));
            }
            continue;
        }
        // Nobody plans a factory in an uncivilised country (measured: China, Egypt and the others
        // have none in the original's yearly saves to 1845; civilised Turkey has 13).
        if (land.owners <= 0.0 && !planned_by_state) || !staffable || !c.civilized {
            continue;
        }
        let (places, staffed) = work.get(key).copied().unwrap_or_default();
        let busy = places <= 0.0 || staffed / places > rules.min_employment;
        let shut: Vec<usize> = here.iter().copied().filter(|i| state.factories[*i].closed).collect();
        // The port's reading of "can be enlarged": runs without loss and is nearly fully staffed.
        let expandable: Vec<usize> = here.iter().copied().filter(|i| { let f = &state.factories[*i]; can_grow(f) && !f.closed && f.loss_days == 0 && staff_of(f) >= rules.upgrade_staffing * places_of(f) }).collect();
        // New factory: the type by its score, or any buildable one when none scores.
        let open: Vec<&FactoryKind> = rules.kinds.iter().filter(|k| buildable(&k.name)).collect();
        let (new, reopen, expand) = project_weights(&policy, !open.is_empty(), busy, shut.len(), expandable.len());
        let Some(choice) = pick(&[(0, new), (1, reopen), (2, expand)], roll[0]) else { continue };
        let project = match choice {
            0 => {
                let scored: Vec<(&str, f64)> = open
                    .iter()
                    .filter_map(|k| {
                        let t = type_of(&k.name)?;
                        let (mut sound, mut failing, mut building) = (0, 0, 0);
                        for f in state.factories.iter().filter(|f| f.owner == key.0 && f.kind == k.name) {
                            if f.level < 1.0 {
                                building += 1;
                            } else if f.closed || (f.loss_days > 0 && !f.subsidised) {
                                failing += 1;
                            } else if f.loss_days == 0 {
                                sound += 1;
                            }
                        }
                        let planned = state.projects.iter().filter(|((owner, _), p)| *owner == key.0 && **p == Project::New(k.name.clone())).count();
                        let margin = t.value * price(&t.output_good) - t.input_goods.iter().map(|(g, x)| x * price(g)).sum::<f64>();
                        let inputs: Vec<(f64, f64, f64)> = t.input_goods.iter().map(|(g, x)| (*x, supply(g), demand(g))).collect();
                        Some((k.name.as_str(), capitalist_type_score(margin, &inputs, (supply(&t.output_good), demand(&t.output_good)), capitalist_type_weight(sound, failing, planned, building))))
                    })
                    .collect();
                let Some(kind) = pick(&scored, roll[1]).or_else(|| open.get((roll[1] * open.len() as f64) as usize).map(|k| k.name.as_str())) else { continue };
                Project::New(kind.to_string())
            }
            1 => Project::Reopen(shut[((roll[1] * shut.len() as f64) as usize).min(shut.len() - 1)]),
            _ => Project::Expand(expandable[((roll[1] * expandable.len() as f64) as usize).min(expandable.len() - 1)]),
        };
        actions.push(Action::Plan(key.clone(), project));
    }

    // The governments, as the AI builds: never at war; reopen, else enlarge, else
    // build; the only money test is a treasury above the price.
    for tag in &tags {
        let Some(c) = state.countries.get(tag) else { continue };
        let policy = rules.policy(c);
        if country_rolls[tag] * rules.ai.build_period >= 1.0 || at_war.contains(tag) || !(policy.build_factory || policy.expand_factory || policy.open_factory) {
            continue;
        }
        // The type: by what the world market is short of and what it has too much of.
        let enabled = rules.enabled(world, c);
        let best = rules.kinds.iter().filter(|k| enabled.contains(k.name.as_str())).filter_map(|k| {
            let t = type_of(&k.name)?;
            let inputs: Vec<(f64, f64, f64)> = t.input_goods.keys().map(|g| (price(g), supply(g), demand(g))).collect();
            Some((t, government_type_score(&rules.ai, &inputs, (price(&t.output_good), supply(&t.output_good), demand(&t.output_good)))))
        }).max_by(|a, b| a.1.total_cmp(&b.1));
        let Some((t, _)) = best else { continue };
        // The state: by its idle people.
        type Site<'a> = (&'a (String, Option<u16>), Option<usize>, f64);
        let mut chosen: Option<Site> = None;
        for key in keys.iter().filter(|k| k.0 == *tag && k.1.is_some()) {
            let land = &lands[key];
            let none = Vec::new();
            let here = standing.get(key).unwrap_or(&none);
            let has = here.iter().copied().find(|i| state.factories[*i].kind == t.name);
            let expansion = match has {
                None if policy.build_factory && !land.colonial && here.len() < rules.max_per_state => false,
                Some(i) if policy.expand_factory && can_grow(&state.factories[i]) => true,
                _ => continue,
            };
            let value = government_state_value(land.idle, land.people, land.owners, t.workforce, here.iter().any(|i| state.factories[*i].building_days.is_some()), expansion);
            // The state needs idle people (not free factory workers: measured, see the capitalists above).
            if land.people > 0.0 && land.idle > 0.0 && chosen.is_none_or(|x| value > x.2) {
                chosen = Some((key, has, value));
            }
        }
        let Some((key, has, _)) = chosen else { continue };
        let cost = cost_of(&t.name);
        if cost <= 0.0 {
            continue;
        }
        // Whether the state already has the type is only looked at where reopening is allowed.
        let has = has.filter(|_| policy.open_factory);
        if let Some(i) = has.filter(|i| state.factories[*i].closed) {
            if c.money > cost * rules.ai.reopen_cost_share {
                actions.push(Action::Do(key.clone(), Project::Reopen(i), cost * rules.ai.reopen_cost_share, true));
            }
        } else if policy.build_factory {
            if let Some(i) = has {
                if policy.expand_factory && c.money > cost {
                    actions.push(Action::Do(key.clone(), Project::Expand(i), cost, true));
                }
            } else if c.civilized && c.money > cost && !standing.get(key).is_some_and(|here| here.iter().any(|i| state.factories[*i].kind == t.name)) {
                actions.push(Action::Do(key.clone(), Project::New(t.name.clone()), cost, true));
            }
        }
    }

    // The AI pays into its capitalists' projects, where the party allows it: in
    // CHANCE_INVEST_POP_PROJ of its looks at building. The port leaves out the "wants money"
    // flag of a project (every project short of its price counts) and has no railway projects.
    let (invest_chance, invest_share) = (world.defines.get("ai", "CHANCE_INVEST_POP_PROJ").unwrap_or(0.0), world.defines.get("ai", "POP_PROJECT_INVESTMENT_MAX_BUDGET_FACTOR").unwrap_or(0.0));
    for tag in &tags {
        let Some(c) = state.countries.get(tag).filter(|c| !c.human) else { continue };
        let policy = rules.policy(c);
        if !policy.invest_in_pop_projects || invest_rolls[tag] * rules.ai.build_period >= invest_chance {
            continue;
        }
        let open: Vec<(&(String, Option<u16>), (f64, f64, bool))> = state.projects.iter().filter(|(key, _)| key.0 == *tag).map(|(key, project)| {
            let (price, new) = match project {
                Project::New(kind) => (cost_of(kind), true),
                Project::Reopen(i) => (state.factories.get(*i).map_or(0.0, |f| cost_of(&f.kind) * rules.ai.reopen_cost_share), false),
                Project::Expand(i) => (state.factories.get(*i).map_or(0.0, |f| cost_of(&f.kind)), false),
            };
            (key, (state.investment.get(key).copied().unwrap_or(0.0), price * policy.owner_cost, new))
        }).collect();
        let list: Vec<(f64, f64, bool)> = open.iter().map(|x| x.1).collect();
        if let Some((i, amount)) = project_top_up(&rules.ai, &list, c.money.max(0.0), invest_share) {
            actions.push(Action::TopUp(open[i].0.clone(), amount));
        }
    }

    // The AI's subsidies, where the party allows them.
    for tag in &tags {
        let Some(c) = state.countries.get(tag) else { continue };
        if !rules.policy(c).can_subsidise {
            continue;
        }
        let balance = market.balance.get(tag).copied().unwrap_or(0.0);
        let running = |f: &&(usize, &FactoryState)| f.1.owner == *tag && f.1.level >= 1.0 && !f.1.closed;
        let list: Vec<(usize, &FactoryState)> = state.factories.iter().enumerate().collect();
        if balance >= 0.0 {
            for (i, f) in list.iter().filter(running) {
                let (places, staffed) = work.get(&(f.owner.clone(), f.region)).copied().unwrap_or_default();
                let free = (places - places_of(f)) - (staffed - staff_of(f));
                if let Some(on) = subsidy_change(&rules.ai, balance, f.loss_days, f.subsidised, free, places_of(f)) {
                    actions.push(Action::Subsidy(*i, on));
                }
            }
        } else {
            // A budget in the red: the subsidised factories that lose money go first, largest loss first.
            let mut losers: Vec<(usize, f64)> = list.iter().filter(running).filter(|(_, f)| f.subsidised && f.income < f.spending).map(|(i, f)| (*i, f.spending - f.income)).collect();
            losers.sort_by(|a, b| b.1.total_cmp(&a.1));
            let mut left = balance;
            for (i, loss) in losers {
                if left >= 0.0 {
                    break;
                }
                actions.push(Action::Subsidy(i, false));
                left += loss;
            }
        }
    }

    for action in actions {
        match action {
            Action::Plan(key, project) => {
                state.projects.insert(key, project);
            }
            Action::Drop(key) => {
                state.projects.remove(&key);
            }
            Action::TopUp(key, amount) => {
                if let Some(c) = state.countries.get_mut(&key.0).filter(|c| c.money >= amount) {
                    c.money -= amount;
                    *state.investment.entry(key).or_default() += amount;
                    out.topped_up += amount;
                }
            }
            Action::Subsidy(i, on) => {
                if let Some(f) = state.factories.get_mut(i) {
                    f.subsidised = on;
                    *if on { &mut out.subsidies_on } else { &mut out.subsidies_off } += 1;
                }
            }
            Action::Do(key, project, cost, by_state) => {
                // A government that has already spent its money today on another site waits.
                // What it builds or enlarges is paid at the order and kept by the site, which
                // takes its goods out of the national stockpile and hands the money back as
                // they arrive; price and goods carry 2 - administrative
                // efficiency. Reopening stays a payment into the factory's cash box.
                if by_state {
                    let overhead = if matches!(project, Project::Reopen(_)) { 1.0 } else { 2.0 - admin.get(&key.0).copied().unwrap_or(0.0) };
                    let cost = cost * overhead;
                    let Some(c) = state.countries.get_mut(&key.0).filter(|c| c.money > cost) else { continue };
                    c.money -= cost;
                    let (kind, index) = match &project {
                        Project::New(kind) => (Some(kind.clone()), state.factories.len()),
                        Project::Expand(i) => (state.factories.get(*i).map(|f| f.kind.clone()), *i),
                        Project::Reopen(i) => (None, *i),
                    };
                    if let Some(k) = kind.and_then(|kind| rules.kinds.iter().find(|k| k.name == kind)) {
                        state.money.country(&key.0, None).constructions.push(crate::economy::Construction { factory: index, money: cost, lacks: k.cost.iter().map(|(good, x)| (good.clone(), x * overhead)).collect() });
                    }
                } else {
                    *state.investment.entry(key.clone()).or_default() -= cost;
                    state.projects.remove(&key);
                }
                match project {
                    Project::New(kind) => {
                        let days = rules.kinds.iter().find(|k| k.name == kind).map_or(365.0, |k| k.days);
                        if by_state { &mut out.started_by_states } else { &mut out.started_by_pops }.push((key.0.clone(), kind.clone()));
                        state.factories.push(FactoryState { owner: key.0, region: key.1, kind, level: 0.0, money: rules.start_cash, building_days: Some(days), state_built: by_state, ..Default::default() });
                    }
                    Project::Expand(i) => {
                        if let Some(f) = state.factories.get_mut(i) {
                            let days = rules.kinds.iter().find(|k| k.name == f.kind).map_or(365.0, |k| k.days);
                            (f.building_days, f.state_built) = (Some(days), by_state);
                            out.enlarged += 1;
                        }
                    }
                    Project::Reopen(i) => {
                        // The port's reading: what reopening costs goes into the factory's cash box.
                        if let Some(f) = state.factories.get_mut(i) {
                            (f.closed, f.no_input_days, f.loss_days, f.staff) = (false, 0, 0, Some(0.0));
                            (f.income, f.spending) = (0.0, 0.0);
                            f.money += cost * rules.money_scale;
                            out.reopened += 1;
                        }
                    }
                }
            }
        }
    }
    out
}
