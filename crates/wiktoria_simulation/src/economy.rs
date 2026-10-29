//! The economy inside the live world (phase C, D15). First part: farms and mines.
//!
//! `Farms` ties the measured production rule (`production`) and the modifier totals
//! (`modifiers`) to the same `State` the pop rules and events work on, so that every day each
//! province produces from the pops it has that day.
//!
//! Second part: factories and artisans, at what they can make when their inputs are there
//! (from the original's own tooltips):
//! * a factory makes value x level x throughput x output efficiency, where throughput is its
//!   craftsmen / (workforce x level) x (1 + throughput modifiers) and output efficiency is
//!   (1 + output modifiers) x (1 + clerks / (workforce x level) x the clerks' multiplier);
//! * an artisan pop makes value x size / workforce of its production type.
//!
//! Simplifications, on purpose (D9, D15: shape at country and world scale):
//! * Factories: the craftsmen and clerks of a state are shared among its factories by level,
//!   each factory taking at most its `amount` share of them and at most the people it has
//!   hired (`FactoryState::staff`); no state bonuses (`bonus` blocks). Input shortage, money
//!   and closing are in `market_day`.
//! * Artisans never stand idle and never change what they make; an artisan pop the save gave no
//!   production type makes the average of its country's artisans.
//! * Employment: the pops of the farm's or mine's worker types work there up to its capacity
//!   and up to the number it has hired (`ProvinceState::rgo_staff`, moved every day by
//!   `rgo_staff_step` in `market_day`); nobody works in a province its owner does not control.
//! * No overseas penalty, no mobilisation.
//! * Secondary powers count as other civilized countries.
//!
//! Third part: the rules of the daily economy step as small functions without a world,
//! used by `market_day` and tested on made-up numbers.
//!
//! Fourth part: where money goes: the size of pops'
//! needs, their discount, savings and investment, the national stockpile, loans; and the books
//! all that is kept in (`MoneyState`, `State::money`).

use std::collections::{BTreeMap, BTreeSet, HashSet};

use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::world::World;

use crate::fixed::Fixed;
use crate::modifiers::{country_totals, province_totals, CountryModifierInputs, CountryModifierRules, ProvinceModifierInputs, ProvinceModifierRules, ProvinceSources, Rank, Totals};
use crate::production::{max_workers, rgo_factors, rgo_output, rgo_size, setup_base_size, ProductionBonus, RgoInputs, RgoRules};
use crate::script::{CountryState, ProvinceState, State};

pub struct Farms {
    pub(crate) rules: RgoRules,
    pub(crate) country_rules: CountryModifierRules,
    pub(crate) province_rules: ProvinceModifierRules,
    /// Base size of each province's farm or mine, fixed when the game was set up.
    pub(crate) base: BTreeMap<u16, i64>,
    /// The policies of every party that belong to the party issue groups, by party number - 1.
    pub(crate) party_policies: Vec<Vec<String>>,
    /// People one line of a pop type's needs is written for (FLD_NEEDS_PER_PEOPLE).
    pub(crate) needs_per_people: f64,
    /// Factory money: share of a day's profit paid out at full cash, the most
    /// cash a factory keeps per level, the owners' share of the pay, and the pop types that work
    /// in and own factories.
    pub(crate) factory_pay_factor: f64,
    pub(crate) factory_max_money: f64,
    /// Pops' needs: PDEF_BASE_CON, BASE_GOODS_DEMAND, INVENTION_IMPACT_ON_DEMAND;
    /// and POP_SAVINGS, the share of its cash a pop with its luxury needs met deposits a day.
    pub(crate) base_con: f64,
    pub(crate) base_goods_demand: f64,
    pub(crate) invention_impact: f64,
    pub(crate) pop_savings: f64,
    /// Loans and bankruptcy: LOAN_BASE_INTEREST, MAX_LOAN_CAP_FROM_BANKS,
    /// SHADOWY_FINANCIERS_MAX_LOAN_AMOUNT, BANKRUPTCY_FACTOR, BANKRUPTCY_EXTERNAL_LOAN_YEARS,
    /// BANKRUPCY_DURATION, and the lowest stockpile sliders (TRADE_CAP_LOW_LIMIT_*).
    pub(crate) loan_interest: f64,
    pub(crate) loan_bank_cap: f64,
    pub(crate) loan_private_max: f64,
    pub(crate) bankruptcy_prestige: f64,
    pub(crate) bankruptcy_loan_years: f64,
    pub(crate) bankruptcy_years: f64,
    pub(crate) stockpile_low: [f64; 3],
    /// Hiring and firing: the slowest step as a share of the people employed, and the speeds
    /// of farms and mines by yesterday's supply over demand.
    pub(crate) hiring: Hiring,
    /// Share of its output a civilised and an uncivilised sphere member hands to its leader.
    pub(crate) sphere_share: (f64, f64),
    pub(crate) factory_workers: BTreeSet<String>,
    pub(crate) factory_owners: BTreeSet<String>,
    /// Owners' share of farm and mine income per share of the country's people they are.
    pub(crate) owner_income_factor: f64,
    pub(crate) owner_income_max: f64,
    /// Tax efficiency without technologies, and what one point of `tax_eff` adds.
    /// The money good's mines (defines.country): pops are paid GOLD_TO_WORKER_PAY_RATE x its
    /// value, the state gets GOLD_TO_CASH_RATE a unit.
    pub(crate) gold_to_pay: f64,
    pub(crate) gold_to_cash: f64,
    pub(crate) tax_efficiency: f64,
    pub(crate) tariff_efficiency: f64,
    pub(crate) tax_eff_scale: f64,
    /// A pop's `money` is this many times the unit prices and budgets use.
    pub(crate) pop_money_scale: f64,
    /// Pop types the state pays: the budget slider that pays them and how many tiers of their
    /// needs it covers (FLD_STATE_PAID_POPS), and the multiple of that cost at a full slider.
    pub(crate) state_paid: BTreeMap<String, (String, usize)>,
    pub(crate) state_pay_factor: f64,
    /// Artisans.
    pub(crate) artisan_throttle_down: f64,
    pub(crate) artisan_throttle_up: f64,
    pub(crate) artisan_throttle_min: f64,
    pub(crate) artisan_small_pop: f64,
    pub(crate) artisan_switch_chance: f64,
    /// Base cost of every good and whether its price is fixed; the daily price rule.
    pub(crate) goods: BTreeMap<String, (f64, bool)>,
    pub(crate) price_rules: crate::market::PriceRules,
    /// The computer player's tax stand-in and loan repayment.
    pub(crate) ai_budget: AiBudget,
    /// The economy's fixed numbers (rules file).
    pub(crate) numbers: EconomyNumbers,
}

/// What one day of farm money came to, in budget units (see `Farms::farm_pops_day`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FarmMoney {
    pub income: f64,
    pub tax: f64,
    pub spent: f64,
    /// Received from the states by their soldiers, officers, clergymen and bureaucrats.
    pub salaries: f64,
    /// Charged to the treasuries for those salaries, and for the goods of army and navy.
    pub salary_cost: f64,
    pub state_goods: f64,
    /// Artisans: what they sold, what they paid for inputs, and the value of what they made
    /// against what they could make at full capacity.
    pub artisan_income: f64,
    pub artisan_inputs: f64,
    pub artisan_made: f64,
    pub artisan_capacity: f64,
    /// What pops put into their national banks, less what they took out.
    pub banked: f64,
    /// Factories: sales, input costs, pay to workers and owners, and how many made a profit.
    /// Duties pops, artisans and factories paid on what they bought from the world market
    /// (negative: paid by the states instead).
    pub tariffs: f64,
    /// For reports: the same per country.
    pub duties: BTreeMap<String, f64>,
    /// Paid by the states to the factories they subsidise.
    pub subsidies: f64,
    /// Per country: the day's taxes and duties less salaries, goods and subsidies.
    pub balance: BTreeMap<String, f64>,
    pub factory_income: f64,
    pub factory_inputs: f64,
    pub factory_pay: f64,
    pub factories_profitable: usize,
    /// The day's supply and paid demand per good, in units.
    pub supply: BTreeMap<String, f64>,
    pub demand: BTreeMap<String, f64>,
    /// For reports (nothing in the simulation reads these): per (country, good), the day's units
    /// made by its [farms and mines, factories, artisans], and its paid demand of
    /// [the state (stockpile), factories (inputs), pops (needs and artisans' inputs)].
    pub made_by: BTreeMap<(String, String), [f64; 3]>,
    pub wanted_by: BTreeMap<(String, String), [f64; 3]>,
    /// For reports: per good, the pops' part of the paid demand, split into [life, everyday and
    /// luxury needs, luxury goods bought over the needs (`luxury_overbuy`), artisans' inputs].
    pub pop_wanted: BTreeMap<String, [f64; 5]>,
    /// For reports: per (country, pop type), the day's [income before tax, tax base, tax, what
    /// artisans paid for inputs, needs paid, invested, moved into savings, and what the life,
    /// everyday and luxury needs on sale would have cost in full].
    pub taxed_by: BTreeMap<(String, String), [f64; 10]>,
    /// For reports: per (country, artisan type), the day's [sales, inputs paid, units made, people].
    pub artisans_by: BTreeMap<(String, String), [f64; 4]>,
    /// For reports: each farm's or mine's day, (good, units made, income, people at work, places).
    pub rgo: BTreeMap<u16, (String, f64, f64, f64, f64)>,
    /// For reports: per (country, budget line), what the state would pay in full today: the salary
    /// sliders and social pay (with the administration's 2 - efficiency), and "stockpile" bought.
    pub spent_on: BTreeMap<(String, String), f64>,
    /// For reports: each running factory's day.
    pub factories: Vec<FactoryFlow>,
}

/// One factory's day in the market, for reports: what it made and the inputs it asked for.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FactoryFlow {
    /// Its place in `State::factories`.
    pub index: usize,
    pub good: String,
    pub made: f64,
    pub wanted: Vec<(String, f64)>,
}

fn names(list: &[crate::script::TimedModifier]) -> Vec<String> {
    list.iter().map(|m| m.name.clone()).collect()
}

fn province_inputs(world: &World, id: u16, p: &ProvinceState) -> ProvinceModifierInputs {
    let def = world.province(id);
    ProvinceModifierInputs {
        is_sea: false,
        coastal_sea: false,
        terrain: def.and_then(|d| d.terrain.clone()),
        climate: def.and_then(|d| d.climate.clone()),
        continent: def.and_then(|d| d.continent.clone()),
        crime: p.crime,
        buildings: p.buildings.clone(),
        besieged: false,
        event_modifiers: names(&p.modifiers),
    }
}

fn sources(world: &World) -> ProvinceSources<'_> {
    ProvinceSources {
        statics: &world.defs.modifiers.static_modifiers,
        terrain: &world.defs.terrain,
        climates: &world.defs.climates,
        continents: &world.defs.continents,
        crimes: &world.defs.crimes,
        buildings: &world.defs.buildings,
        events: &world.defs.modifiers.event_modifiers,
    }
}

impl Farms {
    /// How many times a pound a pop's `money` is (FLD_POP_MONEY_EFFECT_SCALE).
    pub fn pop_money_scale(&self) -> f64 {
        self.pop_money_scale
    }

    /// Reads the rules and fixes every province's base size from the install's starting pops.
    pub fn new(m: &EngineRules, world: &World, state: &State) -> Result<Farms, String> {
        let rules = RgoRules::from_engine_rules(m)?;
        let country_rules = CountryModifierRules::from_engine_rules(m)?;
        let province_rules = ProvinceModifierRules::from_engine_rules(m)?;
        let groups: BTreeSet<&str> = world.defs.issues.iter().filter(|g| g.category == country_rules.party_issue_category).map(|g| g.name.as_str()).collect();
        let party_policies = world.countries.iter().flat_map(|c| c.country.parties.iter()).map(|p| p.policies.iter().filter(|(g, _)| groups.contains(g.as_str())).map(|(_, o)| o.clone()).collect()).collect();
        let src = sources(world);
        // The history files give a mine's people as farmers or a farm's as labourers; the game
        // makes them its own workers at the start (measured: Amiens' coal, 98,125 farmers in the
        // 1836 file, works 98,133 labourers on 1836.2.10). So every farm and mine worker type counts.
        let any_worker: HashSet<&str> = world.defs.rgo_types.iter().flat_map(|r| r.employees.iter()).map(|j| j.pop_type.as_str()).collect();
        let mut base = BTreeMap::new();
        for (id, p) in &state.provinces {
            let Some(def) = world.province(*id) else { continue };
            let Some(rgo) = def.trade_goods.as_deref().or(p.trade_goods.as_deref()).and_then(|g| world.defs.rgo_types.iter().find(|r| r.good == g)) else { continue };
            let totals = province_totals(&src, &province_rules, &province_inputs(world, *id, p));
            let workers: i64 = def.pops.iter().filter(|q| any_worker.contains(world.defs.pop_types[q.pop_type as usize].name.as_str())).map(|q| q.size as i64).sum();
            base.insert(*id, setup_base_size(&rules, rgo, &totals, workers, p.controller == p.owner));
        }
        let define = |k: &str| world.defines.get("economy", k).ok_or(format!("defines.economy.{k} missing"));
        Ok(Farms {
            rules,
            country_rules,
            province_rules,
            base,
            party_policies,
            needs_per_people: crate::num_field(m, "FLD_NEEDS_PER_PEOPLE")?,
            factory_pay_factor: world.defines.get("economy", "FACTORY_PAYCHECKS_LEFTOVER_FACTOR").ok_or("defines.economy.FACTORY_PAYCHECKS_LEFTOVER_FACTOR missing")?,
            factory_max_money: world.defines.get("economy", "MAX_FACTORY_MONEY_SAVE").ok_or("defines.economy.MAX_FACTORY_MONEY_SAVE missing")?,
            base_con: world.defines.get("pops", "PDEF_BASE_CON").ok_or("defines.pops.PDEF_BASE_CON missing")?,
            base_goods_demand: world.defines.get("pops", "BASE_GOODS_DEMAND").ok_or("defines.pops.BASE_GOODS_DEMAND missing")?,
            invention_impact: world.defines.get("pops", "INVENTION_IMPACT_ON_DEMAND").ok_or("defines.pops.INVENTION_IMPACT_ON_DEMAND missing")?,
            pop_savings: world.defines.get("pops", "POP_SAVINGS").ok_or("defines.pops.POP_SAVINGS missing")?,
            loan_interest: define("LOAN_BASE_INTEREST")?,
            loan_bank_cap: define("MAX_LOAN_CAP_FROM_BANKS")?,
            loan_private_max: define("SHADOWY_FINANCIERS_MAX_LOAN_AMOUNT")?,
            bankruptcy_prestige: define("BANKRUPTCY_FACTOR")?,
            bankruptcy_loan_years: define("BANKRUPTCY_EXTERNAL_LOAN_YEARS")?,
            bankruptcy_years: define("BANKRUPCY_DURATION")?,
            stockpile_low: [define("TRADE_CAP_LOW_LIMIT_LAND")?, define("TRADE_CAP_LOW_LIMIT_NAVAL")?, define("TRADE_CAP_LOW_LIMIT_CONSTRUCTIONS")?],
            hiring: Hiring {
                hire_lowest: define("EMPLOYMENT_HIRE_LOWEST")?,
                fire_lowest: define("EMPLOYMENT_FIRE_LOWEST")?,
                rgo_hire_hi: define("RGO_SUPPLY_DEMAND_FACTOR_HIRE_HI")?,
                rgo_hire_lo: define("RGO_SUPPLY_DEMAND_FACTOR_HIRE_LO")?,
                rgo_fire: define("RGO_SUPPLY_DEMAND_FACTOR_FIRE")?,
                speed_factory: crate::num_field(m, "FLD_STAFF_SPEED_FACTORY")?,
                fire_divisor: crate::num_field(m, "FLD_STAFF_FIRE_DIVISOR")?,
                step_floor: crate::num_field(m, "FLD_STAFF_STEP_FLOOR")?,
                rgo_fire_above: crate::num_field(m, "FLD_RGO_FIRE_ABOVE")?,
                rgo_hire_hi_below: crate::num_field(m, "FLD_RGO_HIRE_HI_BELOW")?,
                rgo_hire_lo_below: crate::num_field(m, "FLD_RGO_HIRE_LO_BELOW")?,
                rgo_fast_hire_factor: crate::num_field(m, "FLD_RGO_FAST_HIRE_FACTOR")?,
            },
            sphere_share: (define("CIV_BASE_SHARE_FACTOR")?, define("UNCIV_BASE_SHARE_FACTOR")?),
            factory_workers: world.defs.factory_types.iter().flat_map(|t| t.employees.iter()).map(|j| j.pop_type.clone()).collect(),
            factory_owners: world.defs.factory_types.iter().filter_map(|t| t.owner.as_ref()).map(|j| j.pop_type.clone()).collect(),
            owner_income_factor: crate::num_field(m, "FLD_RGO_OWNER_INCOME_FACTOR")?,
            owner_income_max: crate::num_field(m, "FLD_RGO_OWNER_INCOME_MAX")?,
            gold_to_pay: world.defines.get("country", "GOLD_TO_WORKER_PAY_RATE").ok_or("defines.country.GOLD_TO_WORKER_PAY_RATE missing")?,
            gold_to_cash: world.defines.get("country", "GOLD_TO_CASH_RATE").ok_or("defines.country.GOLD_TO_CASH_RATE missing")?,
            tax_efficiency: world.defines.get("country", "BASE_COUNTRY_TAX_EFFICIENCY").ok_or("defines.country.BASE_COUNTRY_TAX_EFFICIENCY missing")?,
            tariff_efficiency: world.defines.get("country", "BASE_TARIFF_EFFICIENCY").ok_or("defines.country.BASE_TARIFF_EFFICIENCY missing")?,
            tax_eff_scale: crate::num_field(m, "FLD_TAX_EFF_SCALE")?,
            pop_money_scale: crate::num_field(m, "FLD_POP_MONEY_EFFECT_SCALE")?,
            state_paid: m
                .require_str("FLD_STATE_PAID_POPS")?
                .split(',')
                .filter_map(|entry| {
                    let mut parts = entry.trim().split(':');
                    Some((parts.next()?.to_string(), (parts.next()?.to_string(), parts.next()?.parse().ok()?)))
                })
                .collect(),
            state_pay_factor: crate::num_field(m, "FLD_STATE_PAY_FACTOR")?,
            artisan_throttle_down: crate::num_field(m, "FLD_ARTISAN_THROTTLE_DOWN")?,
            artisan_throttle_up: crate::num_field(m, "FLD_ARTISAN_THROTTLE_UP")?,
            artisan_throttle_min: crate::num_field(m, "FLD_ARTISAN_THROTTLE_MIN")?,
            artisan_small_pop: crate::num_field(m, "FLD_ARTISAN_SMALL_POP")?,
            artisan_switch_chance: crate::num_field(m, "FLD_ARTISAN_SWITCH_CHANCE")?,
            goods: BTreeMap::new(),
            price_rules: crate::market::PriceRules::from_engine_rules(m)?,
            ai_budget: AiBudget::from_engine_rules(m)?,
            numbers: EconomyNumbers::from_engine_rules(m)?,
        })
    }

    /// Modifier totals and production bonuses of one country.
    fn country(&self, world: &World, state: &State, tag: &str, c: &CountryState, literacy: f64, at_war: bool) -> (Totals, ProductionBonus) {
        let rank = if state.great_powers.iter().any(|g| g == tag) {
            Rank::GreatPower
        } else if c.civilized {
            Rank::Civilized
        } else {
            Rank::Uncivilized
        };
        let input = CountryModifierInputs {
            rank: Some(rank),
            at_war,
            plurality: c.plurality,
            literacy,
            war_exhaustion: c.war_exhaustion,
            badboy: c.badboy,
            national_value: c.national_value.clone(),
            school: c.tech_school.clone(),
            reforms: c.reforms.clone(),
            party_policies: c.ruling_party.and_then(|n| self.party_policies.get((n as usize).wrapping_sub(1))).cloned().unwrap_or_default(),
            inventions: c.inventions.iter().filter_map(|n| world.defs.modifiers.inventions.get((*n as usize).wrapping_sub(1))).map(|(name, _)| name.clone()).collect(),
            event_modifiers: names(&c.modifiers),
        };
        let mut bonus = ProductionBonus::default();
        for t in &c.technologies {
            if let Some(e) = world.defs.tech_production.get(t) {
                bonus.add(&self.rules, e);
            }
        }
        for n in &c.inventions {
            if let Some(e) = world.defs.invention_production.get((*n as usize).wrapping_sub(1)) {
                bonus.add(&self.rules, e);
            }
        }
        (country_totals(&world.defs.modifiers, &world.defs.issues, &self.country_rules, &input), bonus)
    }

    /// What every farm and mine produces today, in units of its good, by (owner, good).
    pub fn daily_output(&self, world: &World, state: &State) -> BTreeMap<(String, String), f64> {
        let mut out: BTreeMap<(String, String), f64> = BTreeMap::new();
        for (_, owner, good, x) in self.province_output(world, state) {
            *out.entry((owner, good)).or_default() += x;
        }
        out
    }

    /// The same per province: (province, owner, good, units).
    pub fn province_output(&self, world: &World, state: &State) -> Vec<(u16, String, String, f64)> {
        self.province_work(world, state).into_iter().map(|w| (w.id, w.owner, w.good, w.units)).collect()
    }

    /// The same with the people at work and the places of each farm or mine.
    pub(crate) fn province_work(&self, world: &World, state: &State) -> Vec<ProvinceWork> {
        // Per country: people and literate people, for the literacy-scaled modifier block.
        let mut literate: BTreeMap<&str, (f64, f64)> = BTreeMap::new();
        // Per (owner, region): people, and people by pop type (for the owner pops' share).
        type People<'a> = (i64, BTreeMap<&'a str, i64>);
        let mut in_state: BTreeMap<(&str, Option<u16>), People> = BTreeMap::new();
        for p in state.provinces.values() {
            let Some(owner) = p.owner.as_deref() else { continue };
            let l = literate.entry(owner).or_default();
            let s = in_state.entry((owner, p.region)).or_default();
            for q in &p.pops {
                l.0 += q.size;
                l.1 += q.size * q.literacy;
                s.0 += q.size as i64;
                *s.1.entry(q.pop_type.as_str()).or_default() += q.size as i64;
            }
        }
        let at_war: BTreeSet<&String> = state.wars.iter().flat_map(|(a, d)| a.iter().chain(d.iter())).collect();
        let countries: BTreeMap<&str, (Totals, ProductionBonus)> = literate
            .iter()
            .filter_map(|(tag, (people, lit))| {
                let c = state.countries.get(*tag)?;
                Some((*tag, self.country(world, state, tag, c, if *people > 0.0 { lit / people } else { 0.0 }, at_war.contains(&tag.to_string()))))
            })
            .collect();
        let src = sources(world);
        let empty = (Totals::new(), ProductionBonus::default());
        // Each province on its own (pure), on threads, in province order.
        crate::pops::par_provinces(state, |id, p, out: &mut Vec<ProvinceWork>| {
            let id = &id;
            let (Some(owner), Some(good), Some(base)) = (p.owner.as_deref(), p.trade_goods.as_deref(), self.base.get(id)) else { return };
            // Nobody works in a province its owner does not control.
            if !owner_controls(p) {
                return;
            }
            let Some(rgo) = world.defs.rgo_types.iter().find(|r| r.good == good) else { return };
            let province = province_totals(&src, &self.province_rules, &province_inputs(world, *id, p));
            let (totals, bonus) = countries.get(owner).unwrap_or(&empty);
            let (people, by_type) = in_state.get(&(owner, p.region)).cloned().unwrap_or_default();
            let owners = rgo.owner.as_ref().and_then(|o| by_type.get(o.pop_type.as_str())).copied().unwrap_or(0);
            let none = BTreeMap::new();
            let sizing = RgoInputs { rgo, base: *base, bonus, real_country: true, country: totals, province: &province, employed: &none, owner_share: (owners, people), mobilisation: state.mobilisation.get(owner).map_or(Fixed::ONE, |x| Fixed::from_f64(*x)), overseas_penalty: Fixed::ZERO };
            let size = rgo_size(&self.rules, &sizing);
            // The worker types work, up to the capacity and the number hired, each type in proportion.
            let mut employed: BTreeMap<String, i64> = BTreeMap::new();
            for job in &rgo.employees {
                let n: i64 = p.pops.iter().filter(|q| q.pop_type == job.pop_type).map(|q| q.size as i64).sum();
                if n > 0 {
                    employed.insert(job.pop_type.clone(), n);
                }
            }
            let (workers, capacity) = (employed.values().sum::<i64>(), max_workers(rgo, size));
            if workers == 0 {
                return;
            }
            let allowed = capacity.min(p.rgo_staff.map_or(i64::MAX, |s| s as i64)).max(0);
            if workers > allowed {
                for n in employed.values_mut() {
                    *n = *n * allowed / workers;
                }
            }
            let inp = RgoInputs { employed: &employed, ..sizing };
            out.push(ProvinceWork { id: *id, owner: owner.to_string(), good: good.to_string(), units: rgo_output(rgo, rgo_factors(&self.rules, &inp, size), size).to_f64(), employed: workers.min(allowed) as f64, capacity: capacity as f64 });
        })
    }
}

impl Farms {
    /// Gives the market every good's base cost and whether its price is fixed.
    pub fn with_goods(mut self, goods: impl IntoIterator<Item = (String, f64, bool)>) -> Farms {
        self.goods = goods.into_iter().map(|(id, cost, fixed)| (id, (cost, fixed))).collect();
        self
    }

    /// The goods factories alone need a day, by (owner, good).
    pub fn factory_inputs(&self, world: &World, state: &State) -> BTreeMap<(String, String), f64> {
        self.workshops(world, state).2
    }

    /// Each running factory's day as the market will see it (what it makes, needs and employs),
    /// for reports; read only.
    pub fn factory_days(&self, world: &World, state: &State) -> Vec<FactoryDay> {
        self.workshops_and_factories(world, state).3
    }

    /// What factories and artisans can make today, in units of their good, by (owner, good), as
    /// (factory output, artisan output).
    pub fn workshop_output(&self, world: &World, state: &State) -> BTreeMap<(String, String), (f64, f64)> {
        self.workshops(world, state).0
    }

    /// What artisans make today as they stand, by (owner, good): each pop with a production
    /// type at its throttle and the stock it bought yesterday (`artisan_units`, the rule of
    /// `market_day`). `workshop_output` is the capacity at full throttle and full stock, and
    /// counts pops without a type as well; the two differ by the throttles and stocks.
    pub fn artisan_output_running(&self, world: &World, state: &State) -> BTreeMap<(String, String), f64> {
        let mut out: BTreeMap<(String, String), f64> = BTreeMap::new();
        for p in state.provinces.values() {
            let Some(owner) = p.owner.as_ref() else { continue };
            for q in p.pops.iter().filter(|q| q.pop_type == "artisans") {
                let Some(t) = q.production.as_deref().and_then(|k| world.defs.artisan_types.iter().find(|t| t.name == k)) else { continue };
                *out.entry((owner.clone(), t.output_good.clone())).or_default() += artisan_units(q.throttle, q.size, t.workforce, q.stock, t.value);
            }
        }
        out
    }

    /// The goods factories and artisans need a day to make that, by (owner, good).
    pub fn workshop_inputs(&self, world: &World, state: &State) -> BTreeMap<(String, String), f64> {
        self.workshops(world, state).1
    }

    /// What every pop wants to buy a day, by (owner, good), as [life, everyday, luxury] needs:
    /// the amount in its pop type's file x `pop_scale` x `tier_factor` (here
    /// without the country's modifiers); luxury needs as the market counts them, 15 times over.
    pub fn pop_needs(&self, world: &World, state: &State) -> BTreeMap<(String, String), [f64; 3]> {
        let types: BTreeMap<&str, &wiktoria_data::defs::PopType> = world.defs.pop_types.iter().map(|t| (t.name.as_str(), t)).collect();
        let mut out: BTreeMap<(String, String), [f64; 3]> = BTreeMap::new();
        for p in state.provinces.values() {
            let Some(owner) = p.owner.as_ref() else { continue };
            let inventions = state.countries.get(owner).map_or(0.0, |c| c.inventions.len() as f64);
            for q in &p.pops {
                let Some(t) = types.get(q.pop_type.as_str()) else { continue };
                let scale = pop_scale(q.size, q.consciousness, self.needs_per_people, self.base_con);
                for (i, tier) in [&t.life_needs, &t.everyday_needs, &t.luxury_needs].into_iter().enumerate() {
                    let share = scale * tier_factor(&self.numbers, i, self.base_goods_demand, 0.0, 0.0, inventions, self.invention_impact) * if i == 2 { self.numbers.luxury_needs_factor } else { 1.0 };
                    for (good, amount) in tier {
                        out.entry((owner.clone(), good.clone())).or_default()[i] += amount * share;
                    }
                }
            }
        }
        out
    }

    #[allow(clippy::type_complexity)]
    fn workshops(&self, world: &World, state: &State) -> (BTreeMap<(String, String), (f64, f64)>, BTreeMap<(String, String), f64>, BTreeMap<(String, String), f64>) {
        let (out, inputs, factory_inputs, _) = self.workshops_and_factories(world, state);
        (out, inputs, factory_inputs)
    }

    /// As `workshops`, and each factory's own day.
    #[allow(clippy::type_complexity)]
    pub(crate) fn workshops_and_factories(&self, world: &World, state: &State) -> (BTreeMap<(String, String), (f64, f64)>, BTreeMap<(String, String), f64>, BTreeMap<(String, String), f64>, Vec<FactoryDay>) {
        let mut days = Vec::new();
        let mut inputs: BTreeMap<(String, String), f64> = BTreeMap::new();
        let mut factory_inputs: BTreeMap<(String, String), f64> = BTreeMap::new();
        let mut out: BTreeMap<(String, String), (f64, f64)> = BTreeMap::new();
        // Workers of each state by pop type, and literacy for the country totals.
        let mut workers: BTreeMap<(&str, Option<u16>), BTreeMap<&str, f64>> = BTreeMap::new();
        let mut literate: BTreeMap<&str, (f64, f64)> = BTreeMap::new();
        // Artisans: per country, output by good of pops with a production type, their people,
        // and the people without one.
        let mut artisans: BTreeMap<&str, (BTreeMap<&str, f64>, f64, f64)> = BTreeMap::new();
        let mut artisan_inputs: BTreeMap<&str, BTreeMap<&str, f64>> = BTreeMap::new();
        for p in state.provinces.values() {
            let Some(owner) = p.owner.as_deref() else { continue };
            let w = workers.entry((owner, p.region)).or_default();
            let l = literate.entry(owner).or_default();
            for q in &p.pops {
                *w.entry(q.pop_type.as_str()).or_default() += q.size;
                l.0 += q.size;
                l.1 += q.size * q.literacy;
                if q.pop_type == "artisans" {
                    let a = artisans.entry(owner).or_default();
                    match q.production.as_deref().and_then(|k| world.defs.artisan_types.iter().find(|t| t.name == k)) {
                        Some(t) if t.workforce > 0.0 => {
                            *a.0.entry(t.output_good.as_str()).or_default() += t.value * q.size / t.workforce;
                            a.1 += q.size;
                            for (good, amount) in &t.input_goods {
                                *artisan_inputs.entry(owner).or_default().entry(good.as_str()).or_default() += amount * q.size / t.workforce;
                            }
                        }
                        _ => a.2 += q.size,
                    }
                }
            }
        }
        for (owner, (by_good, typed, untyped)) in artisans {
            let scale = if typed > 0.0 { 1.0 + untyped / typed } else { 1.0 };
            for (good, x) in by_good {
                out.entry((owner.to_string(), good.to_string())).or_default().1 += x * scale;
            }
            for (good, x) in artisan_inputs.remove(owner).unwrap_or_default() {
                *inputs.entry((owner.to_string(), good.to_string())).or_default() += x * scale;
            }
        }
        // Capacity each state's factories offer, to share its workers by.
        let mut capacity: BTreeMap<(&str, Option<u16>), f64> = BTreeMap::new();
        for f in state.factories.iter().filter(|f| f.level >= 1.0 && !f.closed) {
            if let Some(t) = world.defs.factory_types.iter().find(|t| t.name == f.kind) {
                *capacity.entry((f.owner.as_str(), f.region)).or_default() += t.workforce * f.level;
            }
        }
        let at_war: BTreeSet<&String> = state.wars.iter().flat_map(|(a, d)| a.iter().chain(d.iter())).collect();
        let mut totals: BTreeMap<&str, (f64, f64)> = BTreeMap::new();
        for (factory_index, f) in state.factories.iter().enumerate() {
            if f.level < 1.0 || f.closed {
                continue;
            }
            let (Some(t), Some(c)) = (world.defs.factory_types.iter().find(|t| t.name == f.kind), state.countries.get(&f.owner)) else { continue };
            let key = (f.owner.as_str(), f.region);
            let places = t.workforce * f.level;
            let (Some(pool), Some(all)) = (workers.get(&key), capacity.get(&key).filter(|c| **c > 0.0)) else { continue };
            // (throughput modifiers, output modifiers) of the country: its totals plus technologies.
            let (throughput_mod, output_mod) = *totals.entry(f.owner.as_str()).or_insert_with(|| {
                let (people, lit) = literate.get(f.owner.as_str()).copied().unwrap_or_default();
                let (t, _) = self.country(world, state, &f.owner, c, if people > 0.0 { lit / people } else { 0.0 }, at_war.contains(&f.owner));
                let tech = |k: &str| c.technologies.iter().filter_map(|n| world.defs.technologies.iter().find(|x| x.name == *n)).filter_map(|x| x.values.get(k)).sum::<f64>();
                let total = |k: &str| t.get(k).copied().unwrap_or(0) as f64 / 1000.0 + tech(k);
                (total("factory_throughput"), total("factory_output"))
            });
            let (mut throughput, mut output, mut employed) = (0.0, 1.0, 0.0);
            let offered: f64 = t.employees.iter().map(|job| (pool.get(job.pop_type.as_str()).copied().unwrap_or(0.0) / all).min(job.amount) * places).sum();
            let staffed = f.staff.map_or(1.0, |s| if offered > 0.0 { (s / offered).clamp(0.0, 1.0) } else { 0.0 });
            for job in &t.employees {
                let share = (pool.get(job.pop_type.as_str()).copied().unwrap_or(0.0) * places / all / places).min(job.amount) * staffed;
                employed += share * places;
                match job.effect {
                    wiktoria_data::production::JobEffect::Throughput => throughput += share * job.multiplier,
                    wiktoria_data::production::JobEffect::Output => output += share * job.multiplier,
                    _ => {}
                }
            }
            let running = f.level * throughput * (1.0 + throughput_mod);
            out.entry((f.owner.clone(), t.output_good.clone())).or_default().0 += t.value * running * (1.0 + output_mod) * output;
            days.push(FactoryDay { index: factory_index, good: t.output_good.clone(), units: t.value * running * (1.0 + output_mod) * output, inputs: t.input_goods.iter().chain(&t.maintenance).map(|(good, amount)| (good.clone(), amount * running)).collect(), recipe: t.input_goods.len(), workers: employed, places });
            // Recipe inputs follow how busy the factory is; so do its maintenance goods.
            for (good, amount) in t.input_goods.iter().chain(&t.maintenance) {
                *inputs.entry((f.owner.clone(), good.clone())).or_default() += amount * running;
                *factory_inputs.entry((f.owner.clone(), good.clone())).or_default() += amount * running;
            }
        }
        (out, inputs, factory_inputs, days)
    }
}

/// What one factory makes and needs on a day; `index` is its place in `State::factories`.
#[derive(Debug, Clone)]
pub struct FactoryDay {
    pub index: usize,
    pub good: String,
    pub units: f64,
    pub inputs: Vec<(String, f64)>,
    /// How many of `inputs`, from the front, are goods of the recipe (the rest is maintenance).
    pub recipe: usize,
    /// Craftsmen and clerks at work in it, and the places it has.
    pub workers: f64,
    pub places: f64,
}

/// The duty rate of a country: its tariff slider (negative = a subsidy the state pays) times its
/// tariff efficiency, which is BASE_TARIFF_EFFICIENCY + its tariff efficiency modifier + its
/// administrative efficiency, at most 1. It is charged on what pops, artisans
/// and factories buy from the world market; goods of the home and sphere pool are duty free.
pub fn tariff_rate(slider: f64, base_efficiency: f64, modifier: f64, admin_efficiency: f64) -> f64 {
    slider * (base_efficiency + modifier + admin_efficiency).min(1.0)
}

/// A farm's or mine's day: what it made, the people at work and the places it has.
#[derive(Debug, Clone)]
pub(crate) struct ProvinceWork {
    pub id: u16,
    pub owner: String,
    pub good: String,
    pub units: f64,
    pub employed: f64,
    pub capacity: f64,
}

/// Whether a province is controlled by its owner (an occupied or rebel-held one is not).
pub fn owner_controls(p: &ProvinceState) -> bool {
    p.controller.is_none() || p.controller == p.owner
}

/// The economy's fixed numbers, read from the rules file (see each FLD_ row).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EconomyNumbers {
    /// FLD_FACTORY_BUDGET_GROWTH / _CASH_SHARE: a factory spends at most this multiple of
    /// yesterday's input spending, and at least this share of its cash.
    pub factory_budget_growth: f64,
    pub factory_budget_cash_share: f64,
    /// FLD_FACTORY_PAY_RESERVE_DAYS: no profit pay until cash is above so many days of full inputs.
    pub factory_pay_reserve_days: f64,
    /// FLD_FACTORY_OVERFLOW_SHARE: the share of a factory's cash above its limit that is paid out
    /// to the owners (the workers where there are none) at once; the rest leaves the books.
    pub factory_overflow_share: f64,
    /// FLD_FACTORY_NO_INPUT_DAYS: days in a row without buying after which a factory shuts or
    /// loses a level.
    pub factory_no_input_days: u32,
    /// FLD_FACTORY_SUBSIDY_WAGE_SHARE: a subsidised factory is topped up to this share of its base
    /// wages plus a day's inputs.
    pub factory_subsidy_wage_share: f64,
    /// FLD_RGO_FAST_HIRE_DAYS: fast hiring days after the owner got control back.
    pub rgo_fast_hire_days: u32,
    /// FLD_INDUSTRIAL_SCORE_PER_LEVEL / _MIN_STAFF.
    pub industrial_score_per_level: f64,
    pub industrial_score_min_staff: f64,
    /// FLD_LUXURY_NEEDS_FACTOR: the market's count of what pops would like to have holds this many
    /// times their luxury amounts (measured on the saves).
    pub luxury_needs_factor: f64,
    /// FLD_POP_NEEDS_MEASURED_FACTOR: needs are this much larger than BASE_GOODS_DEMAND alone gives.
    pub pop_needs_measured_factor: f64,
    /// FLD_POP_DISCOUNT_BASE: pops pay this share of the price when the world's money stands still.
    pub pop_discount_base: f64,
    /// FLD_POP_NEEDS_COST_FLOOR: a pop's needs cost of the day starts at this.
    pub pop_needs_cost_floor: f64,
    /// FLD_POP_SAVINGS_COST_DAYS / FLD_POP_SAVINGS_LUXURY_MET / FLD_POP_CASH_CAP.
    pub pop_savings_cost_days: f64,
    pub pop_savings_luxury_met: f64,
    pub pop_cash_cap: f64,
    /// FLD_POP_KEEP_PAY_SHARE: what a pop keeps of the cash left after its shopping, as a share
    /// of the day's pay; the rest buys more of its luxury goods (`luxury_overbuy`).
    pub pop_keep_pay_share: f64,
    /// FLD_POP_HOLD_PAY_SHARE: the share of the day's pay a state-paid pop holds back from its
    /// shopping.
    pub pop_hold_pay_share: f64,
    /// FLD_POP_LUXURY_WITH_EVERYDAY_POOR / _MIDDLE / _RICH: luxury met as a share of everyday met
    /// for a pop short of its everyday needs, by strata (`everyday_cash`).
    pub pop_luxury_with_everyday: [f64; 3],
    /// FLD_INVEST_CASH_SHARE / FLD_INVEST_MIN_CASH.
    pub invest_cash_share: f64,
    pub invest_min_cash: f64,
    /// FLD_INVEST_SCORE_CAPITAL / _CORE / _FACTORY / _NOT_COLONY.
    pub invest_score_capital: f64,
    pub invest_score_core: f64,
    pub invest_score_factory: f64,
    pub invest_score_not_colony: f64,
    /// FLD_LOAN_MIN_INTEREST / FLD_LOAN_INTEREST_DAYS.
    pub loan_min_interest: f64,
    pub loan_interest_days: f64,
    /// FLD_BANKRUPTCY_SHORTFALL / _MILITANCY / _MILITANCY_CAP.
    pub bankruptcy_shortfall: f64,
    pub bankruptcy_militancy: f64,
    pub bankruptcy_militancy_cap: f64,
    /// FLD_AI_STOCKPILE_PEACE_LAND / _NAVAL / _PROJECTS (inferred there).
    pub ai_stockpile_peace: [f64; 3],
    /// FLD_AI_TAX_RAISE_FROM: a tax slider that is raised starts from at least this (the port's own).
    pub ai_tax_raise_from: f64,
}

impl EconomyNumbers {
    pub fn from_engine_rules(m: &EngineRules) -> Result<EconomyNumbers, String> {
        let f = |id: &str| crate::num_field(m, id);
        let days = |id: &str| crate::int_field(m, id).map(|v| v.max(0) as u32);
        Ok(EconomyNumbers {
            factory_budget_growth: f("FLD_FACTORY_BUDGET_GROWTH")?,
            factory_budget_cash_share: f("FLD_FACTORY_BUDGET_CASH_SHARE")?,
            factory_pay_reserve_days: f("FLD_FACTORY_PAY_RESERVE_DAYS")?,
            factory_overflow_share: f("FLD_FACTORY_OVERFLOW_SHARE")?,
            factory_no_input_days: days("FLD_FACTORY_NO_INPUT_DAYS")?,
            factory_subsidy_wage_share: f("FLD_FACTORY_SUBSIDY_WAGE_SHARE")?,
            rgo_fast_hire_days: days("FLD_RGO_FAST_HIRE_DAYS")?,
            industrial_score_per_level: f("FLD_INDUSTRIAL_SCORE_PER_LEVEL")?,
            industrial_score_min_staff: f("FLD_INDUSTRIAL_SCORE_MIN_STAFF")?,
            luxury_needs_factor: f("FLD_LUXURY_NEEDS_FACTOR")?,
            pop_needs_measured_factor: f("FLD_POP_NEEDS_MEASURED_FACTOR")?,
            pop_discount_base: f("FLD_POP_DISCOUNT_BASE")?,
            pop_needs_cost_floor: f("FLD_POP_NEEDS_COST_FLOOR")?,
            pop_savings_cost_days: f("FLD_POP_SAVINGS_COST_DAYS")?,
            pop_savings_luxury_met: f("FLD_POP_SAVINGS_LUXURY_MET")?,
            pop_cash_cap: f("FLD_POP_CASH_CAP")?,
            pop_keep_pay_share: f("FLD_POP_KEEP_PAY_SHARE")?,
            pop_hold_pay_share: f("FLD_POP_HOLD_PAY_SHARE")?,
            pop_luxury_with_everyday: [f("FLD_POP_LUXURY_WITH_EVERYDAY_POOR")?, f("FLD_POP_LUXURY_WITH_EVERYDAY_MIDDLE")?, f("FLD_POP_LUXURY_WITH_EVERYDAY_RICH")?],
            invest_cash_share: f("FLD_INVEST_CASH_SHARE")?,
            invest_min_cash: f("FLD_INVEST_MIN_CASH")?,
            invest_score_capital: f("FLD_INVEST_SCORE_CAPITAL")?,
            invest_score_core: f("FLD_INVEST_SCORE_CORE")?,
            invest_score_factory: f("FLD_INVEST_SCORE_FACTORY")?,
            invest_score_not_colony: f("FLD_INVEST_SCORE_NOT_COLONY")?,
            loan_min_interest: f("FLD_LOAN_MIN_INTEREST")?,
            loan_interest_days: f("FLD_LOAN_INTEREST_DAYS")?,
            bankruptcy_shortfall: f("FLD_BANKRUPTCY_SHORTFALL")?,
            bankruptcy_militancy: f("FLD_BANKRUPTCY_MILITANCY")?,
            bankruptcy_militancy_cap: f("FLD_BANKRUPTCY_MILITANCY_CAP")?,
            ai_stockpile_peace: [f("FLD_AI_STOCKPILE_PEACE_LAND")?, f("FLD_AI_STOCKPILE_PEACE_NAVAL")?, f("FLD_AI_STOCKPILE_PEACE_PROJECTS")?],
            ai_tax_raise_from: f("FLD_AI_TAX_RAISE_FROM")?,
        })
    }
}

/// Speeds of hiring and firing (defines.lua, economy) and the rules file's fixed numbers of the
/// staffing step.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hiring {
    pub hire_lowest: f64,
    pub fire_lowest: f64,
    pub rgo_hire_hi: f64,
    pub rgo_hire_lo: f64,
    pub rgo_fire: f64,
    /// FLD_STAFF_SPEED_FACTORY: share of the gap to its target a factory hires a day.
    pub speed_factory: f64,
    /// FLD_STAFF_FIRE_DIVISOR: firing moves at the hiring speed divided by this.
    pub fire_divisor: f64,
    /// FLD_STAFF_STEP_FLOOR: people hired or fired a day at least.
    pub step_floor: f64,
    /// FLD_RGO_FIRE_ABOVE / FLD_RGO_HIRE_HI_BELOW: supply / demand above which farms and mines
    /// fire, below which they hire at the high factor.
    pub rgo_fire_above: f64,
    pub rgo_hire_hi_below: f64,
    /// FLD_RGO_HIRE_LO_BELOW: supply / demand below which farms and mines hire at the low factor.
    pub rgo_hire_lo_below: f64,
    /// FLD_RGO_FAST_HIRE_FACTOR: hiring speed multiple in the fast days.
    pub rgo_fast_hire_factor: f64,
}

/// One day's move of the people employed towards a target (factories, farms and mines alike):
/// hiring takes `speed` of the gap, firing a quarter of that, each at least
/// 50 people or the lowest share of those employed, and never more than the gap.
pub fn staffing_step(employed: f64, target: f64, speed: f64, h: &Hiring) -> f64 {
    let gap = (target - employed).abs();
    if target > employed {
        employed + (speed * gap).max(h.step_floor.max(employed * h.hire_lowest)).min(gap)
    } else {
        employed - (speed * gap / h.fire_divisor).max(h.step_floor.max(employed * h.fire_lowest)).min(gap)
    }
}

/// The people a farm or mine employs tomorrow, from yesterday's supply and
/// real demand of its good: above 1.05 it fires at RGO_SUPPLY_DEMAND_FACTOR_FIRE x (ratio - 1),
/// below FLD_RGO_HIRE_LO_BELOW (1) it hires at the HIRE_HI (below 0.9) or HIRE_LO factor x (1 - ratio), in between
/// nothing changes. Without a demand figure (and for the money good: pass `None`) it hires to
/// full at once. In the fast days the hiring speed is the factor x 25.
pub fn rgo_staff_step(employed: f64, capacity: f64, supply: f64, demand: Option<f64>, fast: bool, h: &Hiring) -> f64 {
    let Some(demand) = demand.filter(|d| *d > 0.0) else { return capacity };
    let ratio = supply / demand;
    if ratio > h.rgo_fire_above {
        staffing_step(employed, 0.0, h.rgo_fire * (ratio - 1.0), h)
    } else if ratio < h.rgo_hire_lo_below {
        let factor = if ratio < h.rgo_hire_hi_below { h.rgo_hire_hi } else { h.rgo_hire_lo };
        staffing_step(employed, capacity.max(employed), if fast { factor * h.rgo_fast_hire_factor } else { factor * (1.0 - ratio) }, h)
    } else {
        employed
    }
}

/// Units an artisan pop makes in a day: throttle x size / workforce x stock x value, where
/// `stock` is the share of a day's inputs at that throttle it bought the day before.
pub fn artisan_units(throttle: f64, size: f64, workforce: f64, stock: f64, value: f64) -> f64 {
    if workforce <= 0.0 {
        return 0.0;
    }
    throttle * size / workforce * stock * value
}

/// An artisan pop's stock once its throttle moved from `before` (the throttle it bought the
/// inputs at) to `after`: the same inputs, as a share of a day at the new throttle, so that
/// tomorrow it makes exactly what it paid for (not 15% less after a throttle cut, nor 15% more
/// after a raise).
pub fn artisan_stock_after_throttle(stock: f64, before: f64, after: f64) -> f64 {
    if after > 0.0 { stock * before / after } else { stock }
}

/// What a factory may spend on inputs today: its cash, but no more than 1.05 x
/// what it spent yesterday, and at least a tenth of its cash.
pub fn factory_input_budget(n: &EconomyNumbers, cash: f64, yesterday_spending: f64) -> f64 {
    cash.min(n.factory_budget_growth * yesterday_spending).max(n.factory_budget_cash_share * cash).max(0.0)
}

/// The no-input counter after a day: reset when something was bought or
/// the factory is subsidised, else one more; at 11 a factory above level 1 loses a level and
/// starts again, one of level 1 stays at 11, which means shut. Returns (level, days).
pub fn no_input_day(n: &EconomyNumbers, level: f64, days: u32, bought: bool, subsidised: bool) -> (f64, u32) {
    if bought || subsidised {
        return (level, 0);
    }
    if days + 1 < n.factory_no_input_days {
        (level, days + 1)
    } else if level > 1.0 {
        (level - 1.0, 0)
    } else {
        (level, n.factory_no_input_days)
    }
}

/// What a factory pays out on a day and the cash it keeps.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FactoryPayout {
    pub workers: f64,
    pub owners: f64,
    pub cash: f64,
}

/// A factory's pay. `cash` is what it holds after today's sales came in and
/// the inputs were paid. Profit pay = `leftover_factor` x profit x cash / (`max_save` x level),
/// and none while cash is not above 7 days of full inputs. With owners in the state the workers
/// get the larger of the minimum wage (as far as the cash goes) and half the pay, the owners the
/// rest; without owners the workers get all of the pay and no minimum. Cash above `max_save` x
/// level is cut back to it: `factory_overflow_share` of the excess is paid out at once to the
/// owners (the port's reading: to the workers where there are none), the rest leaves the books.
#[allow(clippy::too_many_arguments)]
pub fn factory_payout(n: &EconomyNumbers, cash: f64, income: f64, input_cost: f64, full_inputs_cost: f64, minimum_wage: f64, level: f64, leftover_factor: f64, max_save: f64, has_owners: bool) -> FactoryPayout {
    let cash = cash.max(0.0);
    let cap = max_save * level.max(1.0);
    let profit = (income - input_cost).max(0.0);
    let pay = if cash <= n.factory_pay_reserve_days * full_inputs_cost { 0.0 } else { (leftover_factor * profit * cash / cap).min(cash) };
    let (mut workers, mut owners) = if has_owners {
        let w = minimum_wage.min(cash).max(pay / 2.0);
        (w, (pay - w).max(0.0))
    } else {
        (pay, 0.0)
    };
    let mut left = cash - workers - owners;
    if left > cap {
        if has_owners {
            owners += (left - cap) * n.factory_overflow_share;
        } else {
            workers += (left - cap) * n.factory_overflow_share;
        }
        left = cap;
    }
    FactoryPayout { workers, owners, cash: left }
}

/// The industrial score of a country from its states, each given as (sum of
/// the levels of its factories, sum of level x workforce of the same, people of the pop types
/// that work in factories, employed or not): 4 x levels x the share of the places those
/// people could fill, between 0.05 and 1. People and places count in whole hundreds.
/// The score for money invested abroad is added by the ranking (`rank::investment_score`).
pub fn industrial_score(n: &EconomyNumbers, states: &[(f64, f64, f64)]) -> f64 {
    states.iter().map(|(levels, places, people)| industrial_state_score(n, *levels, *places, *people)).sum()
}

/// One state's part of the industrial score (see `industrial_score`).
pub fn industrial_state_score(n: &EconomyNumbers, levels: f64, places: f64, people: f64) -> f64 {
    let (places, people) = ((places / 100.0).floor(), (people / 100.0).floor());
    if places > 0.0 { n.industrial_score_per_level * levels * (people / places).clamp(n.industrial_score_min_staff, 1.0) } else { 0.0 }
}

/// The overall ranking, best first: civilised countries before uncivilised,
/// then by total score, largest first (ties by tag). Each entry: (tag, civilised, score).
pub fn rank_order(mut scores: Vec<(String, bool, f64)>) -> Vec<String> {
    scores.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.total_cmp(&a.2)).then(a.0.cmp(&b.0)));
    scores.into_iter().map(|x| x.0).collect()
}

/// What the market of one good came to for one country: the share of its paid demand it could
/// buy, the share of what it made that sold, the share of what it bought that was imported.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cleared {
    pub fill: Option<f64>,
    pub sold: Option<f64>,
    pub imported: Option<f64>,
}

/// Clears one good for a day. `made` and `wanted` are by country,
/// `leader` gives a sphere member its leader and the share of its output it hands over,
/// `order` is the ranking: buyers are served in it, those it does not list last.
/// * A leader's pool is its own output plus the shares of all its members. A member's home
///   supply is a full copy of that pool plus the output it kept; when that is not enough it
///   draws on what is really left in the leader's pool.
/// * Everybody buys at home first. What is left in the pools of the countries without a
///   leader is the world market; the unmet demand is served from it in rank order, first come,
///   first served.
/// * A maker sells at home the share of its home supply that was bought there, and of the
///   rest the share of the world market that found a buyer.
pub fn clear_good(order: &[String], made: &BTreeMap<String, f64>, wanted: &BTreeMap<String, f64>, leader: &BTreeMap<String, (String, f64)>) -> BTreeMap<String, Cleared> {
    clear_good_whole(order, made, wanted, leader, &BTreeSet::new())
}

/// `clear_good`, where the countries in `whole` (the uncivilised ones) take their imports from
/// the world market whole or not at all: a country whose unmet demand is more than what is left
/// gets none of it, and what is left goes on to the next. The owner, 2026-10-28: a poor
/// uncivilised country goes without a good that is short rather than getting a small share; the
/// original's life needs met of such a country is made of whole goods (daily saves 1836.1.2-8).
pub fn clear_good_whole(order: &[String], made: &BTreeMap<String, f64>, wanted: &BTreeMap<String, f64>, leader: &BTreeMap<String, (String, f64)>, whole: &BTreeSet<String>) -> BTreeMap<String, Cleared> {
    let mut pool: BTreeMap<&str, f64> = BTreeMap::new();
    for (tag, x) in made {
        match leader.get(tag) {
            Some((l, share)) => *pool.entry(l.as_str()).or_default() += x * share,
            None => *pool.entry(tag.as_str()).or_default() += x,
        }
    }
    let supply_of = |tag: &str, pool: &BTreeMap<&str, f64>| match leader.get(tag) {
        Some((l, share)) => pool.get(l.as_str()).copied().unwrap_or(0.0) + made.get(tag).copied().unwrap_or(0.0) * (1.0 - share),
        None => pool.get(tag).copied().unwrap_or(0.0),
    };
    let domestic: BTreeMap<&str, f64> = made.keys().chain(wanted.keys()).map(|t| (t.as_str(), supply_of(t, &pool))).collect();
    let mut buyers: Vec<&String> = order.iter().filter(|t| wanted.contains_key(*t)).collect();
    buyers.extend(wanted.keys().filter(|t| !order.contains(t)));
    // At home; `pool` becomes what is really left in each leader's or lone country's pool.
    let mut sold_home: BTreeMap<&str, f64> = BTreeMap::new();
    let mut at_home: BTreeMap<&str, f64> = BTreeMap::new();
    for tag in &buyers {
        let want = wanted[*tag].max(0.0);
        let got = match leader.get(*tag) {
            Some((l, _)) => {
                let own = want.min(domestic[tag.as_str()]);
                let left = pool.entry(l.as_str()).or_default();
                let more = (want - own).min(*left);
                *left -= more;
                *sold_home.entry(tag.as_str()).or_default() += own;
                *sold_home.entry(l.as_str()).or_default() += more;
                own + more
            }
            None => {
                let left = pool.entry(tag.as_str()).or_default();
                let own = want.min(*left);
                *left -= own;
                *sold_home.entry(tag.as_str()).or_default() += own;
                own
            }
        };
        at_home.insert(tag.as_str(), got);
    }
    let exported: f64 = pool.values().sum();
    let mut world = exported;
    let mut out: BTreeMap<String, Cleared> = BTreeMap::new();
    for tag in &buyers {
        let (want, home) = (wanted[*tag].max(0.0), at_home[tag.as_str()]);
        if want <= 0.0 {
            continue;
        }
        let import = if whole.contains(*tag) && want - home > world { 0.0 } else { (want - home).min(world) };
        world -= import;
        let e = out.entry((*tag).clone()).or_default();
        e.fill = Some((home + import) / want);
        e.imported = Some(if home + import > 0.0 { import / (home + import) } else { 0.0 });
    }
    let world_sold = if exported > 0.0 { (exported - world) / exported } else { 1.0 };
    for (tag, x) in made {
        let supply = domestic[tag.as_str()];
        if *x > 0.0 && supply > 0.0 {
            let home = (sold_home.get(tag.as_str()).copied().unwrap_or(0.0) / supply).min(1.0);
            out.entry(tag.clone()).or_default().sold = Some(home + (1.0 - home) * world_sold);
        }
    }
    out
}

// ---- Where money goes: pops' needs and their discount,
// ---- the national stockpile, the national bank, loans and bankruptcy.

/// The computer player's budget stand-in, read from the rules file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AiBudget {
    /// FLD_AI_REPAY_SURPLUS_SHARE / _MAX / _MIN / _TREASURY: it pays back min(surplus x share,
    /// max) a day when above min and its treasury is above the last.
    pub repay_share: f64,
    pub repay_max: f64,
    pub repay_min: f64,
    pub repay_treasury: f64,
    /// FLD_AI_BUDGET_SURPLUS_SHARE / _RESERVE_DAYS: a daily surplus of this share of spending
    /// while the treasury holds less than so many days of spending (inferred).
    pub surplus_share: f64,
    pub reserve_days: f64,
    /// FLD_AI_BUDGET_SPEND_DOWN_SHARE / _RICH_DAYS: a daily deficit of this share above so many
    /// days (the sign is not settled in the description; the port's reading).
    pub spend_down_share: f64,
    pub rich_days: f64,
    /// FLD_AI_BUDGET_PERIOD_DAYS: sliders are set about once in this many days;
    /// taxes move this share of the way a day.
    pub period_days: f64,
    /// FLD_AI_BUDGET_SPENDING_FLOOR / _CAP: education and administration start from the floor
    /// and stop at the cap; FLD_AI_BUDGET_EDUCATION_FIRST_SHARE of the surplus goes to education
    /// first.
    pub spending_floor: f64,
    pub spending_cap: f64,
    pub education_first_share: f64,
    /// FLD_AI_BUDGET_TARIFF_CAP_PEACE: the highest tariff at peace; FLD_AI_BUDGET_TARIFF_BASE_MIN:
    /// below this duty at a full slider the tariff is 0.
    pub tariff_cap_peace: f64,
    pub tariff_base_min: f64,
}

impl AiBudget {
    pub fn from_engine_rules(m: &EngineRules) -> Result<AiBudget, String> {
        let f = |id: &str| crate::num_field(m, id);
        Ok(AiBudget {
            repay_share: f("FLD_AI_REPAY_SURPLUS_SHARE")?,
            repay_max: f("FLD_AI_REPAY_SURPLUS_MAX")?,
            repay_min: f("FLD_AI_REPAY_SURPLUS_MIN")?,
            repay_treasury: f("FLD_AI_REPAY_SURPLUS_TREASURY")?,
            surplus_share: f("FLD_AI_BUDGET_SURPLUS_SHARE")?,
            reserve_days: f("FLD_AI_BUDGET_SURPLUS_RESERVE_DAYS")?,
            spend_down_share: f("FLD_AI_BUDGET_SPEND_DOWN_SHARE")?,
            rich_days: f("FLD_AI_BUDGET_SPEND_DOWN_RICH_DAYS")?,
            period_days: f("FLD_AI_BUDGET_PERIOD_DAYS")?,
            spending_floor: f("FLD_AI_BUDGET_SPENDING_FLOOR")?,
            spending_cap: f("FLD_AI_BUDGET_SPENDING_CAP")?,
            education_first_share: f("FLD_AI_BUDGET_EDUCATION_FIRST_SHARE")?,
            tariff_cap_peace: f("FLD_AI_BUDGET_TARIFF_CAP_PEACE")?,
            tariff_base_min: f("FLD_AI_BUDGET_TARIFF_BASE_MIN")?,
        })
    }
}
/// The lender of a loan from the private investors, as a save writes it.
pub const PRIVATE_INVESTORS: &str = "---";

/// How many times its file's amounts a pop wants: its size over the people one
/// line is written for, x (1 + 2 x consciousness / PDEF_BASE_CON): at consciousness 5 half as
/// much again, at 10 twice. Checked on the saves: soldiers of one country with consciousness 1
/// and 3 hold cash in that proportion.
pub fn pop_scale(size: f64, consciousness: f64, per_people: f64, base_con: f64) -> f64 {
    size / per_people * (1.0 + 2.0 * consciousness / base_con)
}

/// The factor of one tier of needs for a country and strata: BASE_GOODS_DEMAND
/// and the goods demand modifier added up, x (1 + the strata's modifier for the tier) x the measured factor;
/// everyday and luxury needs x (1 + INVENTION_IMPACT_ON_DEMAND x inventions). The description
/// has luxury needs x 15 on top; the saves say that is only what pops would like to have
/// (`EconomyNumbers::luxury_needs_factor`): what they buy and what counts as met is the plain amount (officers
/// and aristocrats with their luxury needs fully met hold steady cash far below a fifteenfold
/// basket; the needs tooltip shows luxury amounts of the order of life needs).
pub fn tier_factor(n: &EconomyNumbers, tier: usize, base_demand: f64, demand_modifier: f64, tier_modifier: f64, inventions: f64, invention_impact: f64) -> f64 {
    let base = (base_demand + demand_modifier) * (1.0 + tier_modifier) * n.pop_needs_measured_factor;
    match tier {
        0 => base,
        _ => base * (1.0 + invention_impact * inventions),
    }
}

/// What a pop did about one tier of its needs: the share of the tier it bought, what it paid,
/// and how far the tier counts as met.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TierBought {
    pub share: f64,
    pub paid: f64,
    pub met: f64,
}

/// One tier of a pop's needs. `whole` is the price of the whole tier, `had_cost` the price of
/// the part the market can deliver and `available` that part's share of the units. The pop
/// orders cash / (whole x `discount`) of the tier, at most all of it: a pop short of a tier
/// spends all its cash (test save 1836.2.9: life, everyday and luxury met times the whole tiers'
/// price come to 1.21-1.30 times the pop's cash, 1 / discount). In a civilised country the order
/// counts as met and is paid in full whether the market delivers it or not; elsewhere only what
/// can be had is met and paid for, and the pop keeps the rest.
pub fn buy_tier(cash: f64, whole: f64, had_cost: f64, available: f64, discount: f64, civilised: bool) -> TierBought {
    if whole <= 0.0 || discount <= 0.0 {
        return TierBought::default();
    }
    let share = (cash.max(0.0) / (whole * discount)).min(1.0);
    if civilised {
        TierBought { share, paid: share * whole * discount, met: share }
    } else {
        TierBought { share, paid: share * had_cost.min(whole) * discount, met: share * available }
    }
}

/// What a pop short of its everyday needs spends on them; the rest of its cash buys luxury goods at
/// the same time, so that luxury met = `k` x everyday met (test save 1836.2.9: farmers and labourers
/// short of everyday needs have luxury met at 0.206 and 0.211 of it, p10-p90 0.18-0.23; clergymen
/// and bureaucrats 0.15). `everyday` and `luxury` are the whole tiers' prices x discount.
pub fn everyday_cash(cash: f64, everyday: f64, luxury: f64, k: f64) -> f64 {
    if cash >= everyday || everyday <= 0.0 || luxury <= 0.0 || k <= 0.0 {
        return cash;
    }
    cash * everyday / (everyday + k * luxury)
}

/// What a pop spends on more of its luxury goods at the end of its shopping. The cash above
/// `keep` (FLD_POP_KEEP_PAY_SHARE x the day's pay) orders a share of the luxury tier at the
/// price of the whole tier (`whole`, x `discount`); the pop pays for the part the market can
/// deliver (`had_cost` is the price of what can be had of a full tier) and keeps the rest.
/// Returns the share ordered and what it pays. It does not count as met.
pub fn luxury_overbuy(cash: f64, keep: f64, had_cost: f64, whole: f64, discount: f64) -> (f64, f64) {
    let spare = cash - keep.max(0.0);
    if spare <= 0.0 || had_cost <= 0.0 || whole <= 0.0 || discount <= 0.0 {
        return (0.0, 0.0);
    }
    let share = spare / (whole * discount);
    (share, share * had_cost.min(whole) * discount)
}

/// The pops' discount of the next day: (change of all the world's money today +
/// gold money made today) / (all money + gold money) + 0.7975; 1 when that is above 1 or not
/// positive, and on the first day.
pub fn pop_discount(n: &EconomyNumbers, total: f64, yesterday: Option<f64>, gold: f64) -> f64 {
    let Some(before) = yesterday else { return 1.0 };
    if total + gold <= 0.0 {
        return 1.0;
    }
    let k = (total - before + gold) / (total + gold) + n.pop_discount_base;
    if k > 1.0 || k <= 0.0 { 1.0 } else { k }
}

/// What a pop moves into its savings at the end of its shopping (negative = it
/// withdraws). With cash of at least two days of needs (or without savings) and luxury needs met
/// it deposits POP_SAVINGS x its cash; cash above the cap is deposited whatever else; with cash
/// under two days of needs it withdraws the gap, as far as its savings and the bank's unlent
/// money go.
pub fn savings_step(n: &EconomyNumbers, cash: f64, savings: f64, needs_cost: f64, luxury_met: f64, pop_savings: f64, bank_free: f64) -> f64 {
    let over = (cash - n.pop_cash_cap).max(0.0);
    let cash = cash - over;
    let enough = cash >= n.pop_savings_cost_days * needs_cost;
    if (enough || savings <= 0.0) && luxury_met >= n.pop_savings_luxury_met {
        over + pop_savings * cash
    } else if !enough && savings > 0.0 && over <= 0.0 {
        -(n.pop_savings_cost_days * needs_cost - cash).min(savings).min(bank_free.max(0.0))
    } else {
        over
    }
}

/// The score of a state for the capitalists' money of a day, from its
/// provinces as (is the capital, the owner has a core on it): 1000 for the capital and 200 for
/// each core province, 25 for each factory of the state, 50 when it is not a colony.
pub fn investment_state_score(n: &EconomyNumbers, provinces: &[(bool, bool)], factories: usize, colony: bool) -> f64 {
    provinces.iter().map(|(capital, core)| if *capital { n.invest_score_capital } else { 0.0 } + if *core { n.invest_score_core } else { 0.0 }).sum::<f64>() + n.invest_score_factory * factories as f64 + if colony { 0.0 } else { n.invest_score_not_colony }
}

/// What the state wants to hold of one good: one day of the army's need x the
/// land slider + the navy's x the naval slider + overseas maintenance + what its building work
/// can take x the construction slider.
pub fn stockpile_target(sliders: [f64; 3], land: f64, naval: f64, overseas: f64, construction: f64) -> f64 {
    (sliders[0] * land + sliders[1] * naval + overseas + sliders[2] * construction).max(0.0)
}

/// A pop's tax base and tax for the day: its income less what it paid for its inputs (an
/// artisan's; measured on the test saves of 1836: the original's artisan tax base is the margin, not
/// the sales), times the tax efficiency, then times the slider.
pub fn pop_tax(income: f64, inputs_paid: f64, efficiency: f64, rate: f64) -> (f64, f64) {
    let base = (income - inputs_paid).max(0.0) * efficiency;
    (base, base * rate)
}

/// One country's day, as the computer player's budget sees it: what each slider would bring or
/// cost at full (1.0), and the rest of the balance that no slider moves.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BudgetDay {
    /// Tax at a full slider of the poor, middle and rich strata (their tax base).
    pub tax_base: [f64; 3],
    /// Duty at a full tariff slider: the imported value x tariff efficiency.
    pub tariff_base: f64,
    /// Pay at a full education and administration slider.
    pub education: f64,
    pub administration: f64,
    /// Today's balance less what the taxes, the tariff and the two salaries above brought or cost.
    pub other: f64,
    /// All of today's spending, and the treasury.
    pub spending: f64,
    pub treasury: f64,
}

/// The computer player's sliders: taxes (poor, middle, rich), tariff, education, administration.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BudgetChoice {
    pub taxes: [f64; 3],
    pub tariff: f64,
    pub education: f64,
    pub administration: f64,
}

/// The computer player's budget, set again every day from today's figures. Taxes start at the
/// party's top and both salaries at their floor; the wanted balance is +10% of spending while
/// the treasury holds less than a year of spending, -5% above two years, else nothing. Short
/// of it, the tariff makes up the gap (up to the peace cap, or the party's top at war); above
/// it, the tariff is 0 and the surplus goes first a third to education, then to administration,
/// then to education again, each up to the cap, and what is left is a tax cut by the same rate
/// on all strata. Measured on the yearly saves 1837-1845: 3 in 4 countries
/// tax at 0.95 or more; a country whose education and administration rest at the floor has a
/// tariff (0.25-1.0), one that pays more has none; England and France, with both at the cap,
/// tax at 0.4-0.56 on all strata alike. THE PORT'S STAND-IN: the original's own split of the
/// surplus (the administration's share and the strata weights) and its countdown of 1 to 3
/// days are not followed.
/// `limits`: the party's [min tax, max tax, max tariff].
pub fn ai_budget_solve(b: &AiBudget, d: &BudgetDay, limits: [f64; 3], at_war: bool) -> BudgetChoice {
    let wanted = if d.treasury < b.reserve_days * d.spending {
        b.surplus_share * d.spending
    } else if d.treasury > b.rich_days * d.spending {
        -b.spend_down_share * d.spending
    } else {
        0.0
    };
    let [min_tax, max_tax, max_tariff] = limits;
    let max_tax = max_tax.max(min_tax);
    let base: f64 = d.tax_base.iter().sum();
    let floor = b.spending_floor;
    let mut c = BudgetChoice { taxes: [max_tax; 3], tariff: 0.0, education: floor, administration: floor };
    let at_floor = d.other + max_tax * base - floor * (d.education + d.administration);
    if at_floor < wanted {
        let cap = if at_war { max_tariff } else { b.tariff_cap_peace.min(max_tariff) }.max(0.0);
        if d.tariff_base >= b.tariff_base_min {
            c.tariff = ((wanted - at_floor) / d.tariff_base).clamp(0.0, cap);
        }
        return c;
    }
    let mut left = at_floor - wanted;
    let raise = |slider: &mut f64, full: f64, money: f64| -> f64 {
        if full <= 0.0 {
            return 0.0;
        }
        let step = (money / full).min(b.spending_cap - *slider).max(0.0);
        *slider += step;
        step * full
    };
    left -= raise(&mut c.education, d.education, left * b.education_first_share);
    left -= raise(&mut c.administration, d.administration, left);
    left -= raise(&mut c.education, d.education, left);
    if base > 0.0 {
        let cut = (left / base).min(max_tax - min_tax).max(0.0);
        c.taxes = [max_tax - cut; 3];
    }
    c
}

/// What a foreign bank lends at most: its unlent money, and no more than
/// MAX_LOAN_CAP_FROM_BANKS x (1 + the borrower's loan modifier) x the borrower's tax base less
/// what it owes that bank already.
pub fn foreign_bank_offer(bank_free: f64, cap_factor: f64, loan_modifier: f64, tax_base: f64, owed: f64) -> f64 {
    bank_free.min(cap_factor * (1.0 + loan_modifier) * tax_base - owed).max(0.0)
}

/// One day's interest on a debt: rate / 30, x (1 + the loan interest modifier)
/// for every lender but the country's own bank.
pub fn loan_interest(n: &EconomyNumbers, debt: f64, rate: f64, interest_modifier: f64, own_bank: bool) -> f64 {
    debt * rate / n.loan_interest_days * if own_bank { 1.0 } else { 1.0 + interest_modifier }
}

/// What a building site of the state takes out of the stockpile today: of
/// every good it still lacks as much as is open to it (`open` and `lacks` go down), and what it
/// hands back to the treasury for them: their value at today's prices, at most the money it holds.
pub fn construction_takes(lacks: &mut BTreeMap<String, f64>, open: &mut BTreeMap<String, f64>, money: f64, price: impl Fn(&str) -> f64) -> f64 {
    let mut value = 0.0;
    for (good, lack) in lacks.iter_mut() {
        let Some(stock) = open.get_mut(good) else { continue };
        let take = lack.min(*stock).max(0.0);
        (*lack, *stock) = (*lack - take, *stock - take);
        value += take * price(good);
    }
    lacks.retain(|_, x| *x > 1e-9);
    value.min(money.max(0.0))
}

/// A debt of a country (the save's `creditor`): the lender's tag (its own tag = its own bank,
/// `PRIVATE_INVESTORS` = the private investors), the rate fixed when the loan was taken, the
/// debt, and whether the last interest was paid in full.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Loan {
    pub lender: String,
    pub rate: f64,
    pub debt: f64,
    pub paid: bool,
}

/// A factory the state is building or enlarging: its place in
/// `State::factories`, the price paid at the order and not yet handed back, and the goods it
/// still lacks.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Construction {
    pub factory: usize,
    pub money: f64,
    pub lacks: BTreeMap<String, f64>,
}

/// The money side of one country.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CountryMoney {
    /// The national bank: all deposits of its pops, the part lent to states, interest received
    /// (which leaves circulation: nothing in the original was seen to use it).
    pub bank: f64,
    pub lent: f64,
    pub interest_received: f64,
    /// The national stockpile, by good.
    pub stockpile: BTreeMap<String, f64>,
    /// The three stockpile sliders: land, naval, construction.
    pub sliders: [f64; 3],
    /// What army and navy need a day at a full slider, as read from the save, and the
    /// administrative efficiency they were read at (the need carries 2 - efficiency).
    pub land_need: BTreeMap<String, f64>,
    pub naval_need: BTreeMap<String, f64>,
    pub need_efficiency: f64,
    /// What the factory sites can take tomorrow: per good the largest lack of any one site
    /// (set by the market).
    pub construction_need: BTreeMap<String, f64>,
    /// What the province building sites and the units under construction ask of the stockpile
    /// for today: per good the largest ask of any one site (set by the core's day, before the
    /// market). The market buys up to the larger of the two.
    pub site_need: BTreeMap<String, f64>,
    pub constructions: Vec<Construction>,
    pub loans: Vec<Loan>,
    pub last_bankrupt: Option<wiktoria_data::date::Date>,
    /// The budget sliders as they stood before the bankruptcy (the port's own: put back when the
    /// debts are wiped, because the port has no computer player that sets sliders).
    pub sliders_before: BTreeMap<String, f64>,
    /// Yesterday's tax at full sliders (the base of what foreign banks lend).
    pub tax_base: f64,
}

/// Sums over a run, in budget units (see `MoneyState::report`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MoneyTotals {
    pub days: f64,
    pub tax: f64,
    pub tariffs: f64,
    pub salaries_charged: f64,
    pub social: f64,
    pub stockpile: f64,
    pub repaid_by_sites: f64,
    pub interest: f64,
    pub borrowed: f64,
    pub paid_back: f64,
    pub deposited: f64,
    pub withdrawn: f64,
    pub invested: f64,
    pub needs_at_full_price: f64,
    pub needs_paid: f64,
    pub bankruptcies: Vec<String>,
}

/// Banks, stockpiles, loans and the pops' savings and discount of the whole world.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MoneyState {
    pub countries: BTreeMap<String, CountryMoney>,
    /// Savings by pop id: the bank they lie in and the amount, in budget units.
    pub savings: BTreeMap<u64, (String, f64)>,
    /// All the world's money at the end of yesterday, and the discount pops get today (0 = none
    /// worked out yet, which counts as 1).
    pub world_money: Option<f64>,
    pub discount: f64,
    pub totals: MoneyTotals,
}

impl MoneyState {
    /// Reads what a save knows: every country's bank, stockpile, stockpile sliders, supply
    /// needs, debts and last bankruptcy, and every pop's savings. Countries the save does not
    /// describe get full sliders and `CountryState::state_purchases` as their army's need.
    pub fn from_save(state: &State, save: &wiktoria_data::save::SaveGame) -> MoneyState {
        use wiktoria_data::script::Value;
        let root = &save.doc.root;
        let mut out = MoneyState::default();
        let admin = crate::pops::admin_efficiency_shares(state, 0.01);
        for tag in &save.countries {
            let Some(c) = root.get(tag).and_then(Value::as_block) else { continue };
            let num = |k: &str, default: f64| c.get(k).and_then(Value::as_f64).unwrap_or(default);
            let pool = |k: &str| -> BTreeMap<String, f64> { c.get(k).and_then(Value::as_block).map(|b| b.entries().filter_map(|(g, v, _)| Some((g.to_string(), v.as_f64()?))).collect()).unwrap_or_default() };
            let bank = c.get("bank").and_then(Value::as_block);
            let of_bank = |k: &str| bank.and_then(|b| b.get(k)).and_then(Value::as_f64).unwrap_or(0.0);
            let loans = c.get_all("creditor").filter_map(Value::as_block).filter_map(|l| Some(Loan { lender: l.get("country")?.as_str()?.trim_matches('"').to_string(), rate: l.get("interest").and_then(Value::as_f64).unwrap_or(0.0), debt: l.get("debt")?.as_f64()?, paid: l.get("was_paid").and_then(Value::as_str) != Some("no") })).collect();
            // A date before the game's first year means "never".
            let last_bankrupt = c.get("last_bankrupt").and_then(Value::as_str).and_then(|s| wiktoria_data::date::Date::parse(s.trim_matches('"'))).filter(|d| d.year > 1000);
            out.countries.insert(tag.clone(), CountryMoney { bank: of_bank("money"), lent: of_bank("money_lent"), stockpile: pool("stockpile"), sliders: [num("trade_cap_land", 1.0), num("trade_cap_naval", 1.0), num("trade_cap_projects", 1.0)], land_need: pool("land_supply_cost"), naval_need: pool("naval_supply_cost"), need_efficiency: admin.get(tag).copied().unwrap_or(0.0), loans, last_bankrupt, tax_base: num("tax_base", 0.0), ..Default::default() });
        }
        for (id, p) in &save.provinces {
            let (Some(block), Some(owner)) = (root.get(&id.to_string()).and_then(Value::as_block), p.owner.as_ref()) else { continue };
            for (_, v, _) in block.entries() {
                let Some(q) = v.as_block() else { continue };
                let (Some(pop), Some(bank)) = (q.get("id").and_then(Value::as_f64), q.get("bank").and_then(Value::as_f64)) else { continue };
                if bank > 0.0 && q.get("size").is_some() {
                    out.savings.insert(pop as u64, (owner.clone(), bank / 1000.0));
                }
            }
        }
        out
    }

    /// The money side of a country, made on first use: full sliders, and the army's need as the
    /// country block has it.
    pub fn country(&mut self, tag: &str, c: Option<&CountryState>) -> &mut CountryMoney {
        self.countries.entry(tag.to_string()).or_insert_with(|| CountryMoney { sliders: [1.0; 3], land_need: c.map(|c| c.state_purchases.clone()).unwrap_or_default(), ..Default::default() })
    }

    /// One line on where the money went over the run, per day, and who holds it now.
    pub fn report(&self, state: &State, pop_money_scale: f64) -> String {
        let t = &self.totals;
        let days = t.days.max(1.0);
        let owners: BTreeSet<&String> = state.provinces.values().filter_map(|p| p.owner.as_ref()).collect();
        let treasuries: f64 = owners.iter().filter_map(|o| state.countries.get(*o)).map(|c| c.money).sum();
        let pops: f64 = state.provinces.values().filter(|p| p.owner.is_some()).flat_map(|p| &p.pops).map(|q| q.money).sum::<f64>() / pop_money_scale;
        let factories: f64 = state.factories.iter().map(|f| f.money).sum::<f64>() / pop_money_scale;
        let banks: f64 = self.countries.values().map(|c| c.bank).sum();
        let lent: f64 = self.countries.values().map(|c| c.lent).sum();
        let debts: f64 = self.countries.values().flat_map(|c| &c.loans).map(|l| l.debt).sum();
        let funds: f64 = state.investment.values().sum::<f64>() + self.countries.values().flat_map(|c| &c.constructions).map(|x| x.money).sum::<f64>();
        let all = treasuries + pops + factories + (banks - lent).max(0.0) + funds;
        format!(
            "money sinks, a day world-wide (original 1836: taxes 10,800-11,800, tariffs 600-900, salaries 7,100-8,300, stockpile 3,300-6,200, handed back by building sites 640-1,620): taxes {:.0}, tariffs {:.0}, salaries {:.0}, pensions and unemployment pay {:.0}, stockpile {:.0}, handed back {:.0}, interest {:.1}, borrowed {:.1}, paid back {:.1}; pops: needs {:.0} for goods worth {:.0} (discount now {:.3}), deposited {:.1}, withdrawn {:.1}, invested {:.1}; held now: treasuries {:.0} ({:.0}% of {:.0}; {:.0}% of treasuries and pops' cash alone), pops {:.0}, factories {:.0}, banks {:.0} (lent {:.0}), projects {:.0}; state debts {:.0}; bankruptcies {}: {}",
            t.tax / days,
            t.tariffs / days,
            t.salaries_charged / days,
            t.social / days,
            t.stockpile / days,
            t.repaid_by_sites / days,
            t.interest / days,
            t.borrowed / days,
            t.paid_back / days,
            t.needs_paid / days,
            t.needs_at_full_price / days,
            if self.discount > 0.0 { self.discount } else { 1.0 },
            t.deposited / days,
            t.withdrawn / days,
            t.invested / days,
            treasuries,
            treasuries / all.max(1.0) * 100.0,
            all,
            treasuries / (treasuries + pops).max(1.0) * 100.0,
            pops,
            factories,
            banks,
            lent,
            funds,
            debts,
            t.bankruptcies.len(),
            t.bankruptcies.join(" ")
        )
    }
}
