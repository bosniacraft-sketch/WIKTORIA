//! One combined game day: every system the port has, run in one order on one state.
//!
//! `Game` owns the world, the script `State` (economy, pops, politics, events) and a [`Core`] that
//! holds the military, diplomacy, unrest, colony and building states with their rules. `Game::day`
//! runs one day; monthly steps run on the 1st (`date.day == 1`), yearly ones on 1 January.
//!
//! Order of the day:
//! (0) the orders of the countries a person plays (player.rs, D20), before anything else.
//! (1) research, politics (elections), reforms: the country part of the tick.
//! (2) [`Core::day`], the military and the cross-system effects, ahead of the market so that what the
//! army buys and what occupation and mobilisation cut are known to the economy:
//! (a) army AI orders for countries at war (CAIMilitaryPlan; its place in the tick is not traced:
//! chosen here, before units move);
//! (b) the military day (wars and failed goals, ticking war score, units,
//! battles, sieges; the monthly part on the 1st);
//! (c) its results applied to the state: casualties off soldier pops, new controllers, prestige,
//! war exhaustion, the list of running wars;
//! (d) mobilisation (decision on the 1st, progress daily, regiments from pops when complete);
//! (e) army supply need (proportional to regiments) put on the stockpile the market buys for;
//! (f) building sites (construction, completions feed forts, railways);
//! (g) on the 1st: diplomacy drift and points, unrest factions and risings, colonial points,
//! AI railway orders; daily: influence and colonial races (the last step of the tick);
//! (h) on the foreign-minister cadence: war goals wanted and peace offers.
//! (3) market day (economy), rank, events which move the date on, pops' due
//! steps (growth, literacy, mood, ideology, issues), national focus (1st), industry, country
//!    month and inventions (1st), promotion, assimilation, migration.
//!
//! Every place where a cross effect is not connected carries a `NOT WIRED:` comment.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use wiktoria_data::date::Date;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::save::SaveGame;
use wiktoria_data::script;
use wiktoria_data::world::World;

use crate::buildings::{self, BuildingRules, ProvinceConstruction, RailState};
use crate::colonies::{self, ColonyRules, ColonyState, Contender, Race};
use crate::diplomacy::relations::{monthly_change, monthly_points, DriftInput};
use crate::diplomacy::crisis::{self, CrisisRules, CrisisState};
use crate::diplomacy::{influence, DipRules, DiplomacyState};
use crate::events::{parse_decisions, parse_events, EventLog, EventRules, EventRunner};
use crate::military::ai::{self as mil_ai, AiArmy, AiProvince, AiRegiment, AiRules, AiWorld, AreaIndex, Sides};
use crate::military::battle::CombatKind;
use crate::military::day::MilitaryDay;
use crate::military::logistics::{self, MobilisablePop, SupplyAccess};
use crate::military::movement::{self, find_path, BorderKind};
use crate::military::units::{Army, Regiment};
use crate::military::war::{self, PeaceAnswerInput, PeaceRequest, PeaceScope, War, WarEvent, WarEventKind, WarGoal};
use crate::military::war_ai::{self, WarAiRules};
use crate::military::{MilitaryCountry, MilitaryRules, MilitaryState};
use crate::pops::{self, CountryRules, GrowthRules, InventionRules, LiteracyRules, MobilityDefs, MobilityRules, MoodRules, TargetCache};
use crate::script::{EffectRules, State};
use crate::unrest::{self, UnrestRules, UnrestState};
use crate::{Calendar, PopSchedule};

/// Engine stand-ins of the combined day (rules file `FLD_GAME_*`, hypotheses).
#[derive(Debug, Clone, PartialEq)]
pub struct GameNumbers {
    /// Days between two rounds of the AI foreign minister (war goals, peace offers).
    pub foreign_minister_days: u64,
    /// Unit type of a mobilised regiment once unlocked, and the fallback / rebel unit type.
    pub mobilised_unit: String,
    pub irregular_unit: String,
}

impl GameNumbers {
    pub fn from_engine_rules(m: &EngineRules) -> Result<GameNumbers, String> {
        Ok(GameNumbers {
            foreign_minister_days: crate::int_field(m, "FLD_GAME_FOREIGN_MINISTER_DAYS")?.max(1) as u64,
            mobilised_unit: m.require_str("FLD_GAME_MOBILISED_UNIT")?.to_string(),
            irregular_unit: m.require_str("FLD_GAME_IRREGULAR_UNIT")?.to_string(),
        })
    }
}

/// What one day of the core did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CoreLog {
    /// Pops whose militancy a beaten rebel army divided today.
    pub rebels_calmed: usize,
    pub battles_started: usize,
    pub battles_finished: usize,
    pub sieges_started: usize,
    pub controller_changes: usize,
    pub men_lost: f64,
    pub orders: usize,
    pub mobilised: Vec<String>,
    pub regiments_raised: usize,
    pub risings: usize,
    /// Pops that joined or left a rebel faction, and factions founded, this day.
    pub faction_joins: usize,
    pub faction_leaves: usize,
    pub factions_founded: usize,
    /// Factions that enforced their demands today (`type in country`).
    pub rebel_wins: Vec<String>,
    /// Pops that joined or left a movement, and movements that turned rebel, this day.
    pub movement_joins: usize,
    pub movement_leaves: usize,
    pub movements_turned: usize,
    pub rebel_regiments: usize,
    pub peaces: Vec<String>,
    /// One line per peace made: who signed what, at which score, and what moved (`run --why-peace`).
    pub peace_trace: Vec<String>,
    pub goals_wanted: usize,
    pub goals_added: usize,
    pub wars_declared: Vec<String>,
    pub cbs_justified: usize,
    pub provinces_transferred: usize,
    pub separate_peaces: usize,
    pub buildings_done: usize,
    pub buildings_ordered: usize,
    pub units_ordered: usize,
    pub units_built: usize,
    /// What happened in crises today, one line each (start, answers, settlement).
    pub crisis: Vec<String>,
}

/// The military and every system that is not the economy, with their rules.
pub struct Core {
    pub mil: MilitaryState,
    pub day: MilitaryDay,
    pub ai: AiRules,
    pub numbers: GameNumbers,
    pub dip: Option<(DipRules, DiplomacyState)>,
    /// Defines `BASE_MONTHLY_DIPLOPOINTS`.
    pub base_diplopoints: f64,
    pub unrest: Option<(UnrestRules, UnrestState)>,
    /// The rebel types pops join factions by.
    pub factions: Option<factions::FactionRules>,
    /// Factions that won since `Game::day` last ran their effects.
    pub rebel_wins: Vec<factions::RebelWin>,
    /// The reform options pops may ask for in movements.
    pub movement_rules: Option<movements::MovementRules>,
    pub colonies: Option<(ColonyRules, ColonyState)>,
    pub races: BTreeMap<u32, Race>,
    pub colonial_points: BTreeMap<String, f64>,
    pub buildings: Option<BuildingRules>,
    pub sites: Vec<(String, ProvinceConstruction)>,
    /// Which technology or invention unlocks which unit; `None` switches unit orders off.
    pub unlocks: Option<crate::military::recruit::UnitUnlocks>,
    /// The province buildings each technology (by name) and invention (by number - 1) activates.
    pub tech_buildings: BTreeMap<String, Vec<String>>,
    pub invention_buildings: Vec<Vec<String>>,
    /// Per country (mobilisation size, mobilisation impact) from its modifiers, set by the caller.
    pub mobilisation_mods: BTreeMap<String, (f64, f64)>,
    /// Mobilisation under way: progress 0..1.
    pub mobilising: BTreeMap<String, f64>,
    pub ai_targets: BTreeMap<u64, u16>,
    /// Armies that recover: they rest and take no other order (military/ai.rs `plan_recovery`).
    pub ai_resting: BTreeSet<u64>,
    /// Armies walking to their main army (military/ai.rs `plan_gather`), by the province they go to.
    pub ai_gathering: BTreeMap<u64, u16>,
    /// Armies marching on an order of the computer (any plan, with or without a target in
    /// `ai_targets`); a player's order takes the army out.
    pub ai_moved: BTreeSet<u64>,
    /// The war AI's numbers (military/war_ai.rs); `None` switches AI declarations and goal adding off.
    pub war_ai: Option<WarAiRules>,
    /// Each AI country's war target (one at a time).
    pub war_targets: BTreeMap<String, String>,
    /// Casus belli being justified: (actor, target) -> (type, progress, discovered).
    pub justifying: BTreeMap<(String, String), (String, f64, bool)>,
    /// Casus belli held: (actor, target) -> held ones.
    pub held_cbs: BTreeMap<(String, String), Vec<war::HeldCb>>,
    /// Each running war's direction, by war name.
    pub war_direction: BTreeMap<String, war::Direction>,
    /// Questions the computer put to played countries: peace offers, calls to arms (player.rs).
    pub asks: crate::player::Asks,
    /// Regiments and army supply need per country as the save had them (the need scales with regiments).
    supply_base: BTreeMap<String, (usize, BTreeMap<String, f64>)>,
    /// The one random source of the core (splitmix64 state, seeded from the game seed).
    pub rng: u64,
    pub days: u64,
    next_id: u64,
    /// Wall time per step of the core's day, summed over days, when `Some` (the run CLI's --profile).
    pub profile: Option<BTreeMap<&'static str, std::time::Duration>>,
    /// Country pairs that share a land border (both directions), refreshed monthly for influence.
    pub dip_neighbours: BTreeSet<(String, String)>,
    /// When `Some`, every influence gain of the day: (power, target, gain, its inputs) (`run --influence`).
    pub influence_log: Option<Vec<(String, String, f64, influence::GainInput)>>,
    /// Influence actions the AI took, (date, power, action, target, rival); the caller may drain it.
    pub influence_actions: Vec<(Date, String, influence::Action, String, Option<String>)>,
    /// Flashpoints and the crisis (diplomacy/crisis.rs); `None` switches crises off.
    pub crisis: Option<(CrisisRules, CrisisState)>,
}

/// Country pairs that share a land border, both directions.
pub fn country_neighbours(state: &State) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    for (id, p) in &state.provinces {
        let Some(a) = p.owner.as_deref() else { continue };
        for n in state.neighbours.get(id).into_iter().flatten() {
            if let Some(b) = state.provinces.get(n).and_then(|q| q.owner.as_deref()).filter(|b| *b != a) {
                out.insert((a.to_string(), b.to_string()));
            }
        }
    }
    out
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Pop id -> (province, index in the province's list).
fn pop_index(state: &State) -> BTreeMap<u64, (u16, usize)> {
    state.provinces.iter().flat_map(|(id, p)| p.pops.iter().enumerate().map(move |(i, q)| (q.id, (*id, i)))).collect()
}

impl Core {
    /// The military parts only; diplomacy, unrest, colonies and buildings are added by the caller.
    pub fn new(mil: MilitaryState, day: MilitaryDay, ai: AiRules, numbers: GameNumbers, seed: u64) -> Core {
        let next_id = mil.armies.iter().map(|a| a.id).chain(mil.armies.iter().flat_map(|a| a.regiments.iter().map(|r| r.id))).chain(mil.navies.iter().map(|n| n.id)).max().unwrap_or(0) + 1_000_000;
        Core {
            mil,
            day,
            ai,
            numbers,
            dip: None,
            base_diplopoints: 0.0,
            unrest: None,
            factions: None,
            rebel_wins: Vec::new(),
            movement_rules: None,
            colonies: None,
            races: BTreeMap::new(),
            colonial_points: BTreeMap::new(),
            buildings: None,
            sites: Vec::new(),
            tech_buildings: BTreeMap::new(),
            invention_buildings: Vec::new(),
            unlocks: None,
            mobilisation_mods: BTreeMap::new(),
            mobilising: BTreeMap::new(),
            ai_targets: BTreeMap::new(),
            ai_resting: BTreeSet::new(),
            ai_gathering: BTreeMap::new(),
            ai_moved: BTreeSet::new(),
            war_ai: None,
            war_targets: BTreeMap::new(),
            justifying: BTreeMap::new(),
            held_cbs: BTreeMap::new(),
            war_direction: BTreeMap::new(),
            asks: crate::player::Asks::default(),
            supply_base: BTreeMap::new(),
            rng: seed ^ 0xC0DE_6A3E,
            days: 0,
            next_id,
            profile: None,
            dip_neighbours: BTreeSet::new(),
            influence_log: None,
            influence_actions: Vec::new(),
            crisis: None,
        }
    }

    fn roll(&mut self) -> u64 {
        splitmix(&mut self.rng)
    }

    fn roll01(&mut self) -> f64 {
        (self.roll() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub(crate) fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Colonial races from the save's `colonize` blocks (one race per region; stage from the number of contenders).
    pub fn attach_colonies(&mut self, rules: ColonyRules, c: ColonyState) {
        let mut races: BTreeMap<u32, Race> = BTreeMap::new();
        for (tag, cc) in &c.countries {
            for (region, level) in &cc.races {
                // The points it has standing there, from the save's race record of the region.
                let points = c.races.iter().filter(|r| r.region == *region).flat_map(|r| &r.contenders).filter(|x| x.0 == *tag).map(|x| x.2).sum();
                races.entry(*region).or_default().contenders.push(Contender { tag: tag.clone(), level: *level, points, last_step: 0 });
            }
        }
        for r in races.values_mut() {
            r.stage = if r.contenders.len() >= 2 { colonies::Stage::Interest } else { colonies::Stage::Open };
        }
        // NOT WIRED: the save's race stage, date and temperature are read (`ColonyState::races`) but the races here still start from the levels alone.
        self.races = races;
        self.colonies = Some((rules, c));
    }

    pub fn day_number(&self, d: Date) -> i64 {
        self.day.calendar.to_game(d).map_or(0, |g| g.0.div_euclid(24))
    }

    /// One day of the military and every cross-system effect, on `state` at `state.date`.
    pub fn day(&mut self, state: &mut State) -> CoreLog {
        let date = state.date;
        let monthly = date.day == 1;
        let mut log = CoreLog::default();
        let mut prof = self.profile.take();
        let mut t = std::time::Instant::now();
        // Adds the time since the last mark to `name` when profiling.
        let mut mark = |name: &'static str| {
            if let Some(p) = prof.as_mut() {
                let now = std::time::Instant::now();
                *p.entry(name).or_default() += now - t;
                t = now;
            }
        };
        if self.supply_base.is_empty() {
            for (tag, m) in &state.money.countries {
                self.supply_base.insert(tag.clone(), (self.mil.regiment_count(tag), m.land_need.clone()));
            }
        }
        for c in self.mil.countries.values_mut() {
            c.war_exhaustion = c.war_exhaustion.max(0.0);
        }
        if !self.mil.countries.contains_key(&self.day.rebel_tag) {
            self.mil.countries.insert(self.day.rebel_tag.clone(), MilitaryCountry::default());
        }
        // a. Army AI, then the navy's blockades.
        if self.mil.wars.iter().any(|w| w.active) {
            log.orders = self.army_ai(state);
            log.orders += self.navy_ai(state);
        }
        if !self.ai.off.posts {
            log.orders += self.peace_posts(state);
        }
        mark("core: army ai");
        // b. The military day.
        let rep = self.day.day(&mut self.mil, state, date);
        mark("core: military day");
        log.battles_started = rep.battles_started;
        log.battles_finished = rep.battles_finished;
        log.sieges_started = rep.sieges_started;
        log.controller_changes = rep.controller_changes.len();
        // c. Results into the state.
        if !rep.casualties.is_empty() {
            let index = pop_index(state);
            for (pop, men) in &rep.casualties {
                log.men_lost += men;
                if let Some((p, i)) = index.get(pop) && let Some(q) = state.provinces.get_mut(p).and_then(|p| p.pops.get_mut(*i)) {
                    q.size = (q.size - men).max(0.0);
                }
            }
        }
        // Rebels beaten in battle: the militancy of the pops behind them is divided by
        // REDUCTION_AFTER_DEFEAT (the defines' own description; seen in the original's century2
        // run: a Nicaraguan farmer of a rising faction went from 9.00 to 2.95 in 1838).
        if !rep.rebels_beaten.is_empty() && let Some((rules, _)) = &self.unrest && rules.reduction_after_defeat > 0.0 {
            let index = pop_index(state);
            for pop in &rep.rebels_beaten {
                if let Some((p, i)) = index.get(pop) && let Some(q) = state.provinces.get_mut(p).and_then(|p| p.pops.get_mut(*i)) {
                    q.militancy /= rules.reduction_after_defeat;
                    log.rebels_calmed += 1;
                }
            }
        }
        for c in &rep.controller_changes {
            // Occupation: `economy::owner_controls` reads the controller, so the RGO workers of an
            // occupied province leave (ProvinceMilitary::rgo_workers_leave) on the next market day.
            if let Some(p) = state.provinces.get_mut(&c.province) {
                p.controller = c.to.clone();
                p.controller_since = Some(state.date);
            }
        }
        // NOT WIRED: the controller of an occupied province taking its taxes (the rule description does not say it does).
        for (tag, x) in &rep.prestige {
            if let Some(c) = state.countries.get_mut(tag) {
                c.prestige = (c.prestige + x).max(0.0);
            }
        }
        for (tag, m) in &self.mil.countries {
            if let Some(c) = state.countries.get_mut(tag) {
                // War exhaustion moves pops' militancy through pops::mood_due_pops (MIL_WAR_EXHAUSTION).
                c.war_exhaustion = m.war_exhaustion;
                c.mobilized = m.mobilized;
            }
        }
        // NOT WIRED: militancy from failed war goals and from refused good peace offers (war::failed_goal_penalty
        // is applied only when the AI makes peace below, war::decline_penalty nowhere).
        state.wars = self.mil.wars.iter().filter(|w| w.active).map(|w| (w.attackers.iter().cloned().collect(), w.defenders.iter().cloned().collect())).collect();
        mark("core: results into state");
        // d. Mobilisation.
        self.mobilisation(state, monthly, &mut log);
        mark("core: mobilisation");
        // e. Army supply need: what the save's armies needed, scaled by today's regiments; the market buys
        // it into the stockpile and pays from the treasury.
        if monthly {
            for (tag, (base, need)) in &self.supply_base {
                if *base == 0 {
                    continue;
                }
                let k = self.mil.regiment_count(tag) as f64 / *base as f64;
                if let Some(m) = state.money.countries.get_mut(tag) {
                    m.land_need = need.iter().map(|(g, x)| (g.clone(), x * k)).collect();
                }
            }
            // NOT WIRED: supply status of units from the stockpile (MilitaryDay keeps FLD_MIL_DAY_SUPPLY_STATUS).
        }
        mark("core: supply need");
        // f. Building sites.
        self.building_day(state, monthly, &mut log);
        mark("core: building sites");
        self.unit_building_day(state, &mut log);
        mark("core: units");
        // g. Diplomacy, unrest, colonies.
        self.diplomacy_day(state, monthly);
        mark("core: diplomacy");
        self.crisis_day(state, &mut log);
        mark("core: crisis");
        self.rebel_win_day(state, &mut log);
        if monthly {
            self.unrest_month(state, &mut log);
            mark("core: unrest (monthly)");
            self.colony_month(state);
            mark("core: colonies (monthly)");
        }
        let today = self.day_number(date);
        if let Some((rules, _)) = &self.colonies {
            for race in self.races.values_mut() {
                race.daily(rules, today, 30, &colonies::Climate::default());
            }
            // NOT WIRED: the climate of a race (crisis, war shares) and the AI's colonial steps (colonies::ai_pick_region).
        }
        mark("core: colonial races");
        // h. Foreign minister: casus belli under construction every day; goals, peace and war on its cadence.
        self.cb_day(state);
        // Questions to players whose time is up get the computer's answer (game/play.rs).
        self.answer_due_asks(state, &mut log);
        mark("core: casus belli");
        if self.days.is_multiple_of(self.numbers.foreign_minister_days) {
            self.foreign_minister(state, &mut log);
            mark("core: foreign minister");
            self.war_declarations(state, &mut log);
            mark("core: war declarations");
        }
        self.days += 1;
        self.profile = prof;
        log
    }

    fn army_ai(&mut self, state: &State) -> usize {
        let sides = Sides::from_wars(&self.mil.wars, &self.day.rebel_tag);
        let rebel = self.day.rebel_tag.clone();
        // A country a person plays gets no army orders from the computer (player.rs).
        // A country fights when it is at war or rebels stand in its land or hold a province of it.
        let mut rebels_in: BTreeSet<&str> = self.mil.armies_of(&rebel).filter_map(|a| self.mil.provinces.get(&a.location)?.owner.as_deref()).collect();
        rebels_in.extend(self.mil.provinces.values().filter(|p| p.controller.as_deref() == Some(rebel.as_str())).filter_map(|p| p.owner.as_deref()));
        let tags: Vec<String> = self.mil.countries.keys().filter(|t| **t != rebel && (sides.at_war(t) || rebels_in.contains(t.as_str())) && !crate::player::is_player(state, t)).cloned().collect();
        if tags.is_empty() {
            return 0;
        }
        let goal_regions: BTreeSet<u16> = self.mil.wars.iter().filter(|w| w.active).flat_map(|w| &w.goals).filter_map(|g| g.state_province).filter_map(|p| state.provinces.get(&p).and_then(|q| q.region)).collect();
        let capitals: BTreeSet<u16> = state.countries.values().filter_map(|c| c.capital).collect();
        let provinces: BTreeMap<u16, AiProvince> = self
            .mil
            .provinces
            .iter()
            .map(|(id, p)| (*id, AiProvince { owner: p.owner.clone(), controller: p.controller.clone(), war_goal: state.provinces.get(id).and_then(|q| q.region).is_some_and(|r| goal_regions.contains(&r)), capital: capitals.contains(id) }))
            .collect();
        // Targets that were reached are dropped so the army can be given a new one.
        let at: BTreeMap<u64, u16> = self.mil.armies.iter().map(|a| (a.id, a.location)).collect();
        self.ai_targets.retain(|id, t| at.get(id).is_some_and(|l| l != t));
        self.ai_resting.retain(|id| at.contains_key(id));
        // A gathering army is done once it stands where it went or stopped on the way.
        let walking: BTreeSet<u64> = self.mil.armies.iter().filter(|a| !a.path.is_empty()).map(|a| a.id).collect();
        self.ai_gathering.retain(|id, to| at.get(id).is_some_and(|l| l != to) && walking.contains(id));
        let marching: BTreeSet<u64> = self.mil.armies.iter().filter(|a| !a.path.is_empty()).map(|a| a.id).collect();
        self.ai_moved.retain(|id| marching.contains(id));
        // Against rebels the excess of a besieging stack leaves the siege as an army of its own.
        if !self.ai.off.siege_split {
            self.split_siege_excess(&tags);
        }
        let rel = self.day.relations(&self.mil, state);
        // Armies standing together merge, before anything else is decided.
        let mut armies = self.ai_armies(state);
        let merges: Vec<(u64, u64)> = if self.ai.off.merge {
            Vec::new()
        } else {
            let (mil, day) = (&self.mil, &self.day);
            let supply = |tag: &str, p: u16| -> i64 { day.supply_limit(state, tag, p, day.supply_access(mil, state, &rel, tag, p)) };
            let w = AiWorld { map: &day.map, provinces: &provinces, armies: &armies, sides: &sides, supply: &supply };
            tags.iter().flat_map(|tag| mil_ai::plan_merges(&self.ai, &w, tag)).collect()
        };
        for (keep, gone) in &merges {
            self.merge_armies(*keep, *gone);
        }
        if !merges.is_empty() {
            armies = self.ai_armies(state);
        }
        // Marches whose next step is no longer safe are cancelled; worn stacks go to rest.
        let areas = AreaIndex::new(&self.day.map, &provinces);
        let (halts, recovery) = {
            let (mil, day) = (&self.mil, &self.day);
            let supply = |tag: &str, p: u16| -> i64 { day.supply_limit(state, tag, p, day.supply_access(mil, state, &rel, tag, p)) };
            let w = AiWorld { map: &day.map, provinces: &provinces, armies: &armies, sides: &sides, supply: &supply };
            let mut halts: BTreeSet<u64> = BTreeSet::new();
            let mut recovery = mil_ai::Recovery::default();
            for tag in &tags {
                if !self.ai.off.safe_step {
                    halts.extend(mil_ai::unsafe_marches(&self.ai, &w, tag));
                }
                if !self.ai.off.recover {
                    let r = mil_ai::plan_recovery(&self.ai, &w, &areas, tag);
                    recovery.go.extend(r.go.into_iter().filter(|(id, _)| !halts.contains(id)));
                    recovery.done.extend(r.done);
                }
            }
            (halts, recovery)
        };
        let mut walks: Vec<(u64, u16, bool)> = Vec::new();
        for a in &mut armies {
            if halts.contains(&a.id) {
                // The stack stops and chooses again as an idle one, a day later.
                if let Some(army) = self.mil.armies.iter_mut().find(|x| x.id == a.id) {
                    army.path.clear();
                    army.movement_progress = 0.0;
                }
                self.ai_targets.remove(&a.id);
                (a.busy, a.next, a.destination, a.target) = (true, None, None, None);
            }
            if recovery.done.contains(&a.id) {
                self.ai_resting.remove(&a.id);
                a.resting = false;
            }
            if let Some((_, to)) = recovery.go.iter().find(|(id, _)| *id == a.id) {
                self.ai_resting.insert(a.id);
                self.ai_targets.remove(&a.id);
                (a.resting, a.target) = (true, None);
                if *to != a.location && a.destination != Some(*to) {
                    walks.push((a.id, *to, false));
                }
            }
        }
        // Field armies gather on the main army first; those walking there take no other order.
        let gather: Vec<(u64, u16)> = if self.ai.off.gather {
            Vec::new()
        } else {
            let (mil, day) = (&self.mil, &self.day);
            let supply = |tag: &str, p: u16| -> i64 { day.supply_limit(state, tag, p, day.supply_access(mil, state, &rel, tag, p)) };
            let w = AiWorld { map: &day.map, provinces: &provinces, armies: &armies, sides: &sides, supply: &supply };
            tags.iter().flat_map(|tag| mil_ai::plan_gather(&self.ai, &w, tag)).filter(|(id, _)| !self.ai_gathering.contains_key(id) && !walks.iter().any(|x| x.0 == *id)).collect()
        };
        for (id, to) in &gather {
            self.ai_gathering.insert(*id, *to);
            self.ai_targets.remove(id);
        }
        for a in &mut armies {
            if self.ai_gathering.contains_key(&a.id) {
                a.busy = true;
            }
        }
        let orders: Vec<(u64, u16, bool)> = {
            let (mil, day) = (&self.mil, &self.day);
            let supply = |tag: &str, p: u16| -> i64 { day.supply_limit(state, tag, p, day.supply_access(mil, state, &rel, tag, p)) };
            let w = AiWorld { map: &day.map, provinces: &provinces, armies: &armies, sides: &sides, supply: &supply };
            let mut out = walks;
            out.extend(gather.iter().map(|(a, p)| (*a, *p, false)));
            for tag in &tags {
                out.extend(mil_ai::plan_armies(&self.ai, &w, tag).into_iter().map(|(a, p)| (a, p, true)));
                out.extend(mil_ai::plan_stacks(&w, tag).into_iter().map(|(a, p)| (a, p, false)));
            }
            out
        };
        let landings: Vec<(u64, Vec<u16>)> = {
            let (mil, day) = (&self.mil, &self.day);
            let supply = |tag: &str, p: u16| -> i64 { day.supply_limit(state, tag, p, day.supply_access(mil, state, &rel, tag, p)) };
            let w = AiWorld { map: &day.map, provinces: &provinces, armies: &armies, sides: &sides, supply: &supply };
            let ordered: BTreeSet<u64> = orders.iter().map(|o| o.0).collect();
            let mut out = Vec::new();
            for tag in &tags {
                let rooms: Vec<usize> = mil.navies_of(tag).filter(|n| n.path.is_empty() && !day.fleet_called(n.id) && !mil.armies.iter().any(|a| a.carried_by == Some(n.id))).map(|n| day.fleet_capacity(n)).filter(|x| *x > 0).collect();
                if !rooms.is_empty() {
                    out.extend(mil_ai::plan_landings(&self.ai, &w, tag, rooms, &ordered));
                }
            }
            out
        };
        let map = &self.day.map;
        let passable = |a: u16, b: u16| map.border(a, b).is_some_and(|x| x.kind != BorderKind::Impassable) && !map.provinces.get(&b).is_some_and(|m| m.sea);
        let mut n = 0;
        for (id, to, keep) in orders {
            let Some(army) = self.mil.armies.iter_mut().find(|a| a.id == id) else { continue };
            if let Some(path) = find_path(map, army.location, to, &passable) {
                army.path = path;
                self.ai_moved.insert(id);
                n += 1;
                if keep {
                    self.ai_targets.insert(id, to);
                }
            }
        }
        // Landings go over land and sea legs; the day step boards, sails and lands them.
        for (id, path) in landings {
            let (Some(army), Some(&to)) = (self.mil.armies.iter_mut().find(|a| a.id == id), path.last()) else { continue };
            army.path = path;
            army.movement_progress = 0.0;
            self.ai_moved.insert(id);
            self.ai_targets.insert(id, to);
            n += 1;
        }
        // NOT WIRED: rebel armies get no orders; fronts (areas handed to groups of armies).
        n
    }

    /// Computer countries at peace keep their armies on guard posts (`mil_ai::plan_posts`, our own
    /// design); each looks again every `post_days` days, staggered by its place in the list, and
    /// armies standing together merge every day. Returns the orders given.
    fn peace_posts(&mut self, state: &State) -> usize {
        let sides = Sides::from_wars(&self.mil.wars, &self.day.rebel_tag);
        let rebel = self.day.rebel_tag.clone();
        let mut rebels_in: BTreeSet<&str> = self.mil.armies_of(&rebel).filter_map(|a| self.mil.provinces.get(&a.location)?.owner.as_deref()).collect();
        rebels_in.extend(self.mil.provinces.values().filter(|p| p.controller.as_deref() == Some(rebel.as_str())).filter_map(|p| p.owner.as_deref()));
        let day = self.day_number(state.date);
        let all: Vec<String> = self.mil.countries.keys().filter(|t| **t != rebel && !sides.at_war(t) && !rebels_in.contains(t.as_str()) && !crate::player::is_player(state, t) && state.countries.get(*t).is_some_and(|c| !c.human)).cloned().collect();
        if all.is_empty() {
            return 0;
        }
        let looking: Vec<&String> = all.iter().enumerate().filter(|(i, _)| (day + *i as i64).rem_euclid(self.ai.post_days) == 0).map(|(_, t)| t).collect();
        let provinces: BTreeMap<u16, AiProvince> = self.mil.provinces.iter().map(|(id, p)| (*id, AiProvince { owner: p.owner.clone(), controller: p.controller.clone(), ..AiProvince::default() })).collect();
        let rel = self.day.relations(&self.mil, state);
        let mut armies = self.ai_armies(state);
        let merges: Vec<(u64, u64)> = {
            let (mil, d) = (&self.mil, &self.day);
            let supply = |tag: &str, p: u16| -> i64 { d.supply_limit(state, tag, p, d.supply_access(mil, state, &rel, tag, p)) };
            let w = AiWorld { map: &d.map, provinces: &provinces, armies: &armies, sides: &sides, supply: &supply };
            all.iter().flat_map(|tag| mil_ai::plan_merges(&self.ai, &w, tag)).collect()
        };
        for (keep, gone) in &merges {
            self.merge_armies(*keep, *gone);
        }
        if looking.is_empty() {
            return 0;
        }
        if !merges.is_empty() {
            armies = self.ai_armies(state);
        }
        let mut regiments: BTreeMap<&str, usize> = BTreeMap::new();
        for a in &self.mil.armies {
            *regiments.entry(a.owner.as_str()).or_default() += a.regiments.len();
        }
        let orders: Vec<(u64, u16)> = {
            let (mil, d) = (&self.mil, &self.day);
            let supply = |tag: &str, p: u16| -> i64 { d.supply_limit(state, tag, p, d.supply_access(mil, state, &rel, tag, p)) };
            let w = AiWorld { map: &d.map, provinces: &provinces, armies: &armies, sides: &sides, supply: &supply };
            let mut out = Vec::new();
            for tag in looking {
                let Some(c) = state.countries.get(tag) else { continue };
                let allied = |o: &str| rel.friendly.contains(&(tag.clone(), o.to_string())) || state.alliances.iter().any(|(a, b)| (a == tag && b == o) || (b == tag && a == o));
                let threat = |o: &str| {
                    if allied(o) {
                        return 0.0;
                    }
                    let n = regiments.get(o).copied().unwrap_or(0) as f64;
                    let hostile = c.relations.get(o).is_some_and(|r| *r < self.ai.post_hostile_relation);
                    n * if hostile { self.ai.post_hostile_factor } else { 1.0 }
                };
                out.extend(mil_ai::plan_posts(&self.ai, &w, tag, c.capital, &threat));
            }
            out
        };
        let map = &self.day.map;
        let passable = |a: u16, b: u16| map.border(a, b).is_some_and(|x| x.kind != BorderKind::Impassable) && !map.provinces.get(&b).is_some_and(|m| m.sea);
        let mut n = 0;
        for (id, to) in orders {
            let Some(army) = self.mil.armies.iter_mut().find(|a| a.id == id) else { continue };
            if let Some(path) = find_path(map, army.location, to, &passable) {
                army.path = path;
                army.movement_progress = 0.0;
                n += 1;
            }
        }
        n
    }

    /// The computer's fleets blockade enemy ports (`mil_ai::plan_blockades`). Returns the orders given.
    fn navy_ai(&mut self, state: &State) -> usize {
        let sides = Sides::from_wars(&self.mil.wars, &self.day.rebel_tag);
        let rebel = self.day.rebel_tag.clone();
        let tags: Vec<String> = self.mil.countries.keys().filter(|t| **t != rebel && sides.at_war(t) && !crate::player::is_player(state, t)).cloned().collect();
        if tags.is_empty() {
            return 0;
        }
        let (map, day) = (&self.day.map, &self.day);
        let fleet_way = |a: u16, b: u16| !movement::border_closed(map, &movement::BorderContext::default(), true, a, b);
        let in_battle: BTreeSet<u64> = self.mil.combats.iter().filter(|c| c.kind == CombatKind::Naval).flat_map(|c| c.attacker.units.iter().chain(&c.defender.units).copied()).collect();
        let fleets: Vec<mil_ai::AiFleet> = self
            .mil
            .navies
            .iter()
            .map(|n| {
                let warships = n.ships.iter().filter(|s| day.units.get(&s.kind).is_none_or(|u| !u.transport)).count();
                let idle = n.path.is_empty() && !in_battle.contains(&n.id) && !day.fleet_retreating(n.id) && !day.fleet_called(n.id) && !self.mil.armies.iter().any(|a| a.carried_by == Some(n.id));
                mil_ai::AiFleet { id: n.id, owner: n.owner.clone(), location: n.location, warships, idle }
            })
            .collect();
        let ports: Vec<mil_ai::AiPort> = self.mil.provinces.iter().filter_map(|(id, p)| Some(mil_ai::AiPort { province: *id, sea: map.provinces.get(id)?.port_sea?, controller: p.controller.clone()? })).collect();
        let orders = mil_ai::plan_blockades(&self.ai, &sides, &tags, &fleets, &ports, &|from| movement::distances(map, from, &fleet_way));
        let mut n = 0;
        for (id, sea) in orders {
            let Some(navy) = self.mil.navies.iter_mut().find(|x| x.id == id) else { continue };
            if let Some(path) = find_path(map, navy.location, sea, &fleet_way) {
                navy.path = path;
                navy.movement_progress = 0.0;
                n += 1;
            }
        }
        n
    }

    /// The armies as the army AI sees them.
    fn ai_armies(&self, state: &State) -> Vec<AiArmy> {
        let in_siege: BTreeSet<u64> = self.mil.combats.iter().filter(|c| c.kind == CombatKind::Siege).flat_map(|c| c.attacker.units.iter().copied()).collect();
        let in_battle: BTreeSet<u64> = self.mil.combats.iter().filter(|c| c.kind == CombatKind::Land).flat_map(|c| c.attacker.units.iter().chain(&c.defender.units).copied()).collect();
        self.mil
            .armies
            .iter()
            .map(|a| {
                let regs: Vec<AiRegiment> = a
                    .regiments
                    .iter()
                    .map(|r| {
                        let u = self.day.units.get(&r.kind);
                        AiRegiment { strength: r.strength, max_strength: u.map(|u| u.max_strength).filter(|x| *x > 0.0).unwrap_or(3.0), organisation: r.organisation, max_organisation: self.day.max_organisation(&self.mil, state, a, &r.kind).max(1.0), experience: r.experience }
                    })
                    .collect();
                let org = if regs.is_empty() { 0.0 } else { regs.iter().map(|g| g.organisation / g.max_organisation).sum::<f64>() / regs.len() as f64 };
                let (fighting, retreating) = (in_battle.contains(&a.id), self.day.is_retreating(a.id));
                AiArmy {
                    id: a.id,
                    owner: a.owner.clone(),
                    location: a.location,
                    target: self.ai_targets.get(&a.id).copied(),
                    strength: logistics::stack_weight(&[a]),
                    regiments: a.regiments.len(),
                    org: org.min(1.0),
                    sieging: in_siege.contains(&a.id),
                    busy: fighting || retreating || a.carried_by.is_some(),
                    power: mil_ai::stack_power(&self.ai, &regs),
                    next: a.path.first().copied(),
                    arrive: self.day.days_to_next(a).unwrap_or(0.0),
                    destination: a.path.last().copied(),
                    fighting,
                    retreating,
                    resting: self.ai_resting.contains(&a.id),
                }
            })
            .collect()
    }

    /// A stack of `tags` besieging a rebel-held province sheds its excess over the siege maximum as a new army.
    fn split_siege_excess(&mut self, tags: &[String]) {
        let besieging: Vec<u64> = self.mil.combats.iter().filter(|c| c.kind == CombatKind::Siege).flat_map(|c| c.attacker.units.iter().copied()).collect();
        for id in besieging {
            let Some(i) = self.mil.armies.iter().position(|a| a.id == id && tags.contains(&a.owner)) else { continue };
            let a = &self.mil.armies[i];
            let n = mil_ai::siege_excess(&self.ai, self.mil.provinces.get(&a.location).and_then(|p| p.controller.as_deref()), a.regiments.len());
            if n == 0 {
                continue;
            }
            let new = self.new_id();
            let a = &mut self.mil.armies[i];
            let regiments = a.regiments.split_off(a.regiments.len() - n);
            let part = Army { id: new, name: a.name.clone(), owner: a.owner.clone(), location: a.location, previous: a.previous, base: a.base, supplies: a.supplies, regiments, ..Army::default() };
            self.mil.armies.push(part);
        }
    }

    /// The regiments of `gone` join `keep`, which stands in the same province.
    fn merge_armies(&mut self, keep: u64, gone: u64) {
        let (Some(i), true) = (self.mil.armies.iter().position(|a| a.id == gone), self.mil.armies.iter().any(|a| a.id == keep)) else { return };
        let from = self.mil.armies.remove(i);
        if let Some(k) = self.mil.armies.iter_mut().find(|a| a.id == keep) {
            k.regiments.extend(from.regiments);
        }
        self.ai_targets.remove(&gone);
        self.ai_resting.remove(&gone);
        self.ai_moved.remove(&gone);
    }

    fn mobilisation(&mut self, state: &mut State, monthly: bool, log: &mut CoreLog) {
        if monthly {
            let sides = Sides::from_wars(&self.mil.wars, &self.day.rebel_tag);
            let strength: BTreeMap<String, f64> = self.mil.countries.keys().filter(|t| **t != self.day.rebel_tag).map(|t| (t.clone(), self.mil.armies_of(t).map(|a| logistics::stack_weight(&[a])).sum())).collect();
            for (tag, want) in mil_ai::mobilise_decisions(&self.ai, &sides, &strength) {
                let human = state.countries.get(&tag).is_some_and(|c| c.human);
                let now = self.mil.countries.get(&tag).is_some_and(|c| c.mobilized);
                if human || want == now {
                    continue;
                }
                if want {
                    self.mobilising.insert(tag.clone(), 0.0);
                    if let Some(c) = self.mil.countries.get_mut(&tag) {
                        c.mobilized = true;
                    }
                    log.mobilised.push(tag);
                } else {
                    // Demobilise: regiments of non-soldier pops go home.
                    let soldier = self.day.soldier_type.clone();
                    for a in self.mil.armies.iter_mut().filter(|a| a.owner == tag) {
                        logistics::demobilise(a, &|t| t == soldier);
                    }
                    self.mil.armies.retain(|a| !a.regiments.is_empty());
                    self.mobilising.remove(&tag);
                    if let Some(c) = self.mil.countries.get_mut(&tag) {
                        c.mobilized = false;
                    }
                }
            }
        }
        // Production while mobilised (read by economy::province_work).
        state.mobilisation = self.mil.countries.iter().filter(|(_, c)| c.mobilized).map(|(t, _)| {
            let (size, impact) = self.mobilisation_mods.get(t).copied().unwrap_or((0.0, 0.0));
            (t.clone(), logistics::mobilisation_production_factor(true, impact, size))
        }).collect();
        // Progress: the railway factor of the capital's state.
        let rail = self.buildings.as_ref().and_then(|b| b.of_kind("infrastructure")).map(|d| (d.name.clone(), d.max_level as f64));
        let tags: Vec<String> = self.mobilising.keys().cloned().collect();
        for tag in tags {
            let capital_region = state.countries.get(&tag).and_then(|c| c.capital).and_then(|p| state.provinces.get(&p)).and_then(|p| p.region);
            let levels: Vec<f64> = match (&rail, capital_region) {
                (Some((name, _)), Some(r)) => state.provinces.values().filter(|p| p.region == Some(r) && p.owner.as_deref() == Some(tag.as_str())).map(|p| p.buildings.get(name).copied().unwrap_or(0.0)).collect(),
                _ => Vec::new(),
            };
            let factor = logistics::mobilisation_rail_factor(&self.day.logistics, &levels, rail.as_ref().map_or(0.0, |r| r.1));
            let paused = state.provinces.values().any(|p| p.owner.as_deref() == Some(tag.as_str()) && p.controller != p.owner);
            let old = self.mobilising[&tag];
            let new = logistics::mobilisation_progress(&self.day.logistics, old, factor, paused);
            if new >= 1.0 {
                self.mobilising.remove(&tag);
                log.regiments_raised += self.raise_regiments(state, &tag);
            } else {
                self.mobilising.insert(tag, new);
            }
        }
    }

    /// Which pops mobilisation would raise how many regiments from now (the country's poor,
    /// non-soldier pops of accepted culture), and where each pop lives with its type.
    #[allow(clippy::type_complexity)]
    fn mobilisation_schedule(&self, state: &State, tag: &str) -> Option<(Vec<(u64, u32)>, BTreeMap<u64, (u16, String)>)> {
        let c = state.countries.get(tag)?;
        let (size, impact) = self.mobilisation_mods.get(tag).copied().unwrap_or((0.0, 0.0));
        let accepted = |cul: &str| c.primary_culture.as_deref() == Some(cul) || c.accepted_cultures.iter().any(|a| a == cul);
        let mut pops = Vec::new();
        let mut where_: BTreeMap<u64, (u16, String)> = BTreeMap::new();
        for (id, p) in state.provinces.iter().filter(|(_, p)| p.owner.as_deref() == Some(tag) && p.controller == p.owner && p.colonial == 0) {
            for q in p.pops.iter().filter(|q| q.pop_type != self.day.soldier_type && state.strata.get(&q.pop_type).is_none_or(|s| s == "poor") && accepted(&q.culture)) {
                pops.push(MobilisablePop { id: q.id, size: q.size, has: 0 });
                where_.insert(q.id, (*id, q.pop_type.clone()));
            }
        }
        let limit = logistics::mobilise_limit(&self.day.logistics, self.mil.regiment_count(tag), impact);
        Some((logistics::mobilise_schedule(&self.day.logistics, &pops, size, limit), where_))
    }

    /// Regiments mobilisation would raise for the country if it mobilised today. Read-only.
    pub fn mobilisable_regiments(&self, state: &State, tag: &str) -> Option<u32> {
        self.mobilisation_schedule(state, tag).map(|(s, _)| s.iter().map(|(_, n)| n).sum())
    }

    /// Mobilised regiments from the country's poor, non-soldier pops of accepted culture.
    fn raise_regiments(&mut self, state: &State, tag: &str) -> usize {
        let Some(c) = state.countries.get(tag) else { return 0 };
        let Some((schedule, where_)) = self.mobilisation_schedule(state, tag) else { return 0 };
        let infantry = self.day.units.get(&self.numbers.mobilised_unit).is_some_and(|u| u.active);
        let (kind, exp) = logistics::mobilised_unit(infantry, &self.numbers.mobilised_unit, &self.numbers.irregular_unit, &self.day.logistics, 0.0, 0.0);
        let (kind, unit) = (kind.to_string(), self.day.units.get(kind).cloned().unwrap_or_default());
        let at = c.capital.unwrap_or(0);
        let mut regs = Vec::new();
        for (pop, n) in schedule {
            for _ in 0..n {
                let id = self.new_id();
                regs.push(Regiment { id, name: String::new(), kind: kind.clone(), pop: Some(pop), pop_type: where_.get(&pop).map(|w| w.1.clone()), organisation: unit.organisation, strength: unit.max_strength, experience: exp, name_count: 0 });
            }
        }
        let n = regs.len();
        if n > 0 && at != 0 {
            let id = self.new_id();
            self.mil.armies.push(Army { id, name: format!("Mobilised {tag}"), owner: tag.to_string(), location: at, supplies: 1.0, regiments: regs, ..Army::default() });
        }
        n
    }

    /// Whether a country may build a province building: enabled by default, activated by none of
    /// the technologies and inventions (naval bases: built from 1836 on), or by one it has.
    fn building_open(&self, state: &State, tag: &str, def: &buildings::BuildingDef) -> bool {
        let named = |list: &Vec<String>| list.contains(&def.name);
        let anywhere = self.tech_buildings.values().any(named) || self.invention_buildings.iter().any(named);
        let Some(c) = state.countries.get(tag) else { return false };
        let has = || c.technologies.iter().any(|t| self.tech_buildings.get(t).is_some_and(named)) || c.inventions.iter().any(|n| self.invention_buildings.get((*n as usize).wrapping_sub(1)).is_some_and(named));
        buildings::unlocked(def, &|_| !anywhere || has())
    }

    fn building_day(&mut self, state: &mut State, monthly: bool, log: &mut CoreLog) {
        let Some(rules) = self.buildings.take() else { return };
        let prices = state.prices.clone();
        let price = |g: &str| prices.get(g).copied().unwrap_or(0.0);
        let mut need: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
        let mut keep = Vec::new();
        for (owner, mut site) in std::mem::take(&mut self.sites) {
            let money = state.money.country(&owner, None);
            // NOT WIRED: the stockpile reserve: the site takes from the whole stockpile.
            let r = buildings::construction_day(&mut site, &mut money.stockpile, price);
            if let Some(c) = state.countries.get_mut(&owner) {
                c.money += r.to_treasury + r.refund;
            }
            if let Some(level) = r.completed {
                log.buildings_done += 1;
                let id = site.province as u16;
                if let Some(def) = rules.def(&site.building) {
                    if let Some(p) = state.provinces.get_mut(&id) {
                        p.buildings.insert(def.name.clone(), f64::from(level));
                    }
                    match def.kind.as_str() {
                        "fort" => {
                            if let Some(m) = self.mil.provinces.get_mut(&id) {
                                m.fort = buildings::fort_level(def, level);
                            }
                        }
                        "naval_base" => {
                            if let Some(m) = self.mil.provinces.get_mut(&id) {
                                m.naval_base = f64::from(level);
                            }
                        }
                        "infrastructure" => {
                            if let Some(m) = self.day.map.provinces.get_mut(&id) {
                                m.infrastructure = buildings::infrastructure(def, level);
                            }
                        }
                        _ => {}
                    }
                }
            } else {
                let e = need.entry(owner.clone()).or_default();
                for (g, x) in buildings::daily_ask(&site, rules.n.daily_ask_share) {
                    let v = e.entry(g).or_default();
                    *v = v.max(x);
                }
                keep.push((owner, site));
            }
        }
        self.sites = keep;
        for (tag, m) in state.money.countries.iter_mut() {
            m.site_need = need.remove(tag).unwrap_or_default();
        }
        if let Some(def) = rules.of_kind("infrastructure").cloned() {
            // The AI's railway step. NOT WIRED: forts (`buildings::ai_fort` needs threat inputs).
            let busy: BTreeSet<usize> = self.sites.iter().map(|(_, s)| s.province).collect();
            let tags: Vec<String> = state.countries.iter().filter(|(t, c)| !c.human && c.civilized && state.provinces.values().any(|p| p.owner.as_deref() == Some(t.as_str()))).map(|(t, _)| t.clone()).collect();
            for tag in tags {
                // A run of the production minister today, about once in FLD_BLD_AI_RUN_DAYS.
                let allowed = self.building_open(state, &tag, &def);
                if self.roll01() * rules.n.run_days >= 1.0 || !allowed {
                    continue;
                }
                let price_one = buildings::order_price(&rules.n, &def, 1.0, price, false);
                let mut states: BTreeMap<u16, RailState> = BTreeMap::new();
                for (id, p) in state.provinces.iter().filter(|(_, p)| p.owner.as_deref() == Some(tag.as_str())) {
                    let Some(r) = p.region else { continue };
                    let e = states.entry(r).or_insert_with(|| RailState { state: r as usize, factories: state.factories.iter().filter(|f| f.owner == tag && f.region == Some(r)).count(), colonial: p.colonial > 0, province: None, price: price_one });
                    let level = p.buildings.get(&def.name).copied().unwrap_or(0.0) as u32;
                    if e.province.is_none() && p.controller == p.owner && level < def.max_level && !busy.contains(&(*id as usize)) {
                        e.province = Some(*id as usize);
                    }
                }
                let list: Vec<RailState> = states.into_values().collect();
                let treasury = state.countries.get(&tag).map_or(0.0, |c| c.money);
                let goods_price: f64 = def.goods_cost.iter().map(|(g, x)| x * price(g)).sum();
                let r = self.roll01();
                if let Some(plan) = buildings::ai_railway(&rules, r, allowed, &list, treasury, false, goods_price) {
                    let level = state.provinces.get(&(plan.province as u16)).and_then(|p| p.buildings.get(&def.name)).copied().unwrap_or(0.0) as u32;
                    if let Some(c) = state.countries.get_mut(&tag) {
                        c.money -= plan.price;
                    }
                    self.sites.push((tag, buildings::order(&def, plan.province, level, 1.0, plan.price)));
                    log.buildings_ordered += 1;
                }
            }
        }
        if monthly && let Some(def) = rules.of_kind("naval_base").cloned() {
            // The AI's naval base step (`buildings::ai_naval_base`), at peace. The chooser of the
            // original is not read; THE PORT'S STAND-IN: the first coastal province (by number)
            // of a state with no naval base and no naval base site, else the first base below
            // its top level. Measured on the century1 save of 1838.1.1: 77 of the 83 naval base
            // sites are in provinces without one, most of them not the capital.
            let at_war: BTreeSet<&str> = state.wars.iter().flat_map(|(a, d)| a.iter().chain(d.iter())).map(String::as_str).collect();
            let tags: Vec<String> = state.countries.iter().filter(|(t, c)| !c.human && c.civilized && !at_war.contains(t.as_str())).map(|(t, _)| t.clone()).collect();
            let price = buildings::order_price(&rules.n, &def, 1.0, price, false);
            for tag in tags {
                let own: Vec<(&u16, &crate::script::ProvinceState)> = state.provinces.iter().filter(|(_, p)| p.owner.as_deref() == Some(tag.as_str())).collect();
                let level = |p: &crate::script::ProvinceState| p.buildings.get(&def.name).copied().unwrap_or(0.0) as u32;
                if own.is_empty() || !self.building_open(state, &tag, &def) {
                    continue;
                }
                let busy: BTreeSet<usize> = self.sites.iter().filter(|(_, s)| s.building == def.name).map(|(_, s)| s.province).collect();
                let taken: BTreeSet<u16> = own.iter().filter(|(id, p)| level(p) > 0 || busy.contains(&(**id as usize))).filter_map(|(_, p)| p.region).collect();
                let free = |id: &u16, p: &crate::script::ProvinceState| p.coastal && p.controller == p.owner && !busy.contains(&(*id as usize));
                let chosen = own.iter().find(|(id, p)| free(id, p) && p.region.is_some_and(|r| !taken.contains(&r))).or_else(|| own.iter().find(|(id, p)| free(id, p) && level(p) > 0 && level(p) < def.max_level)).map(|(id, _)| **id as usize);
                let weak = !state.great_powers.contains(&tag);
                let treasury = state.countries.get(&tag).map_or(0.0, |c| c.money);
                let r = self.roll01();
                if let Some(plan) = buildings::ai_naval_base(&rules, r, weak, chosen, treasury, price) {
                    let now = state.provinces.get(&(plan.province as u16)).map_or(0, level);
                    if let Some(c) = state.countries.get_mut(&tag) {
                        c.money -= plan.price;
                    }
                    self.sites.push((tag, buildings::order(&def, plan.province, now, 1.0, plan.price)));
                    log.buildings_ordered += 1;
                }
            }
        }
        // NOT WIRED: building sites under way in the save are not loaded.
        self.buildings = Some(rules);
    }

    fn diplomacy_day(&mut self, state: &mut State, monthly: bool) {
        let Some((rules, mut dip)) = self.dip.take() else { return };
        let today = state.date;
        let sides = Sides::from_wars(&self.mil.wars, &self.day.rebel_tag);
        if monthly {
            // Neighbouring countries from the province map.
            self.dip_neighbours = country_neighbours(state);
            let neighbours = &self.dip_neighbours;
            let pairs: Vec<(String, String)> = dip.relations.keys().filter(|(a, b)| a < b).cloned().collect();
            for (a, b) in pairs {
                let human = |t: &str| dip.countries.get(t).is_some_and(|c| c.human);
                let link = dip.subject_link(&a, &b);
                let at_war = sides.hostile(&a, &b) && a != self.day.rebel_tag && b != self.day.rebel_tag;
                let i = DriftInput {
                    relation: dip.relation_value(&a, &b),
                    subject_link: link,
                    at_war,
                    peaceful_neighbours: neighbours.contains(&(a.clone(), b.clone())) && !at_war,
                    guarantee: dip.guaranteed_by(&a, &b) || dip.guaranteed_by(&b, &a),
                    access: dip.has_access(&a, &b) || dip.has_access(&b, &a),
                    // NOT WIRED: casus belli held between the two (no CB state yet).
                    casus_belli: false,
                    ai_overlord: link && !human(&a) && !human(&b),
                };
                let d = monthly_change(&rules.drift, &i);
                if d != 0.0 {
                    dip.change_relation(&rules, &a, &b, d);
                }
            }
            for c in dip.countries.values_mut() {
                // NOT WIRED: diplomatic point technologies and modifiers.
                c.points = monthly_points(&rules.drift, self.base_diplopoints, 0.0, 0.0, c.points);
            }
            for ((a, b), r) in &dip.relations {
                if let Some(c) = state.countries.get_mut(a) {
                    c.relations.insert(b.clone(), r.value);
                }
            }
        }
        // Influence, every day for every great power.
        if monthly || self.dip_neighbours.is_empty() {
            self.dip_neighbours = country_neighbours(state);
        }
        let mut population: BTreeMap<String, f64> = BTreeMap::new();
        for p in state.provinces.values() {
            if let Some(o) = p.owner.as_deref() {
                *population.entry(o.to_string()).or_default() += p.pops.iter().map(|q| q.size).sum::<f64>();
            }
        }
        let continents: BTreeMap<String, String> = state.countries.iter().filter_map(|(t, c)| Some((t.clone(), state.provinces.get(&c.capital?)?.continent.clone()?))).collect();
        let continent = |t: &str| continents.get(t).cloned();
        // All foreign investment in each country (a host's `foreign_investment` lists it by investor).
        let invested: BTreeMap<String, f64> = state.countries.iter().map(|(host, c)| (host.clone(), c.foreign_investment.iter().filter(|(i, _)| *i != host).map(|(_, x)| x).sum())).collect();
        let powers: Vec<String> = dip.great_powers().map(str::to_string).collect();
        let leaders: BTreeMap<String, String> = dip.relations.iter().filter(|(_, r)| r.level == crate::diplomacy::relations::level::IN_SPHERE).map(|((a, b), _)| (b.clone(), a.clone())).collect();
        for power in &powers {
            let power_score = state.countries.get(power).map_or(0.0, |c| c.total_score);
            let home = continent(power);
            let targets: BTreeMap<String, influence::TargetInfo> = population
                .iter()
                .filter(|(t, _)| *t != power && !sides.hostile(power, t))
                .map(|(t, people)| {
                    let t = t.as_str();
                    let neighbour = self.dip_neighbours.contains(&(power.clone(), t.to_string()));
                    let sphere_neighbour = !neighbour && self.dip_neighbours.range((t.to_string(), String::new())..).take_while(|(a, _)| a == t).any(|(_, n)| leaders.get(n.as_str()) == Some(power));
                    let gain = influence::GainInput {
                        relation: dip.relation_value(power, t),
                        population: *people,
                        target_score: state.countries.get(t).map_or(0.0, |c| c.total_score),
                        power_score,
                        neighbour,
                        sphere_neighbour,
                        other_continent: home.is_some() && continent(t).is_some() && continent(t) != home,
                        puppet: dip.overlord_of(t) == Some(power.as_str()) && !dip.is_substate(t),
                        investment: state.countries.get(t).and_then(|c| c.foreign_investment.get(power)).copied().unwrap_or(0.0),
                        foreign_investment: invested.get(t).copied().unwrap_or(0.0),
                        // The discredit is read from the relation record by run_day.
                        discredited: false,
                    };
                    (t.to_string(), influence::TargetInfo { owns_land: true, gain })
                })
                .collect();
            // NOT WIRED: the influence modifier and technology bonus (commerce techs give +0.1/+0.2 from 1850s).
            let gains = influence::run_day(&mut dip, &rules, power, today, 0.0, 0.0, &targets);
            if let Some(log) = self.influence_log.as_mut() {
                for (t, g) in &gains {
                    log.push((power.clone(), t.clone(), *g, targets[t].gain));
                }
            }
            // The computer's round: priorities, then spending (players choose their own).
            let human = dip.countries.get(power).is_some_and(|c| c.human);
            if !human && influence::ai_round(&rules.ai_influence, power, self.days) {
                let picked = influence::ai_priorities(&dip, &rules, power, today, rules.ai_influence.targets, &targets);
                dip.priorities.retain(|(p, _), _| p != power);
                for (t, n) in picked {
                    dip.priorities.insert((power.clone(), t), n);
                }
                for (a, t, g, fx) in influence::ai_spend(&mut dip, &rules, &rules.cal, today, power) {
                    if let Some(c) = state.countries.get_mut(power) {
                        c.badboy += fx.infamy;
                        c.prestige += fx.prestige;
                    }
                    self.influence_actions.push((today, power.clone(), a, t, g));
                }
            }
            if let Some(c) = state.countries.get_mut(power) {
                c.influence.clear();
                for t in targets.keys() {
                    let x = dip.influence(power, t);
                    if x > 0.0 {
                        c.influence.insert(t.clone(), x);
                    }
                }
            }
        }
        // The state's sphere lists follow the relation records.
        for c in state.countries.values_mut() {
            c.sphere.clear();
        }
        for ((a, b), r) in &dip.relations {
            if r.level == crate::diplomacy::relations::level::IN_SPHERE
                && let Some(c) = state.countries.get_mut(a)
            {
                c.sphere.insert(b.clone());
            }
        }
        // NOT WIRED: AI alliances and access (treaties.rs AI scores).
        self.dip = Some((rules, dip));
    }

    /// Flashpoint tension, the start of a crisis, the powers' answers and the settlement.
    fn crisis_day(&mut self, state: &mut State, log: &mut CoreLog) {
        let Some((r, mut cs)) = self.crisis.take() else { return };
        let Some((dr, mut dip)) = self.dip.take() else {
            self.crisis = Some((r, cs));
            return;
        };
        let today = state.date;
        // Owners follow the map; a flashpoint its tag now owns is gone.
        for f in &mut cs.flashpoints {
            if let Some(o) = state.provinces.get(&f.province).and_then(|p| p.owner.clone()) {
                f.owner = o;
            }
        }
        cs.flashpoints.retain(|f| f.owner != f.tag);
        let at_war: BTreeSet<&str> = state.wars.iter().flat_map(|(a, d)| a.iter().chain(d.iter())).map(String::as_str).collect();
        let powers: Vec<String> = dip.great_powers().map(str::to_string).collect();
        let at_war_share = if powers.is_empty() { 0.0 } else { powers.iter().filter(|g| at_war.contains(g.as_str())).count() as f64 / powers.len() as f64 };
        let movements: BTreeMap<(String, String), f64> = self.unrest.as_ref().map(|(_, u)| u.movements.iter().filter(|m| m.kind == unrest::MovementKind::Independence).map(|m| ((m.country.clone(), m.target.clone()), m.support)).collect()).unwrap_or_default();
        let running = cs.crisis.is_some();
        for f in &mut cs.flashpoints {
            let key = (f.tag.clone(), f.owner.clone());
            let i = crisis::TensionInput {
                owner_rank: state.rank_order.iter().position(|t| *t == f.owner).map_or(0, |n| n + 1),
                casus_belli: self.held_cbs.get(&key).map_or(0, Vec::len) + usize::from(self.justifying.contains_key(&key)),
                movement: movements.get(&(f.owner.clone(), f.tag.clone())).copied().unwrap_or(0.0),
                at_war_share,
                crisis_running: running,
            };
            // NOT WIRED: the jumps on a discovered casus belli and on a revolt (TENSION_ON_*).
            crisis::add_tension(&r, f, crisis::tension_change(&r, &i));
        }
        let owns_land: BTreeSet<&str> = state.provinces.values().filter_map(|p| p.owner.as_deref()).collect();
        if let Some(mut c) = cs.crisis.take() {
            c.temperature += crisis::temperature_rise(&r, &c);
            // NOT WIRED: CRISIS_TEMPERATURE_ON_MOBILIZE (mobilisation during a crisis).
            for g in c.interested.clone() {
                if self.roll01() * r.answer_days >= 1.0 {
                    continue;
                }
                let tied = |x: &str| dip.sphere_leader(x) == Some(g.as_str()) || dip.allied(&g, x) || dip.overlord_of(x) == Some(g.as_str());
                let side = crisis::answer(&crisis::AnswerInput { relation_attacker: dip.relation_value(&g, &c.attacker), relation_defender: dip.relation_value(&g, &c.defender), tied_attacker: tied(&c.attacker), tied_defender: tied(&c.defender) });
                log.crisis.push(format!("{today}: {g} {} in the crisis {} v {}", match side { Some(crisis::Side::Attacker) => "backs the attacker", Some(crisis::Side::Defender) => "backs the defender", None => "stays out" }, c.attacker, c.defender));
                crisis::record_answer(&mut c, &g, side);
            }
            if crisis::settles(&r, &c) {
                let s = crisis::settle(&r, &c, today, &|t| state.countries.get(t).map_or(0.0, |x| x.military_score), &|t| state.countries.get(t).map_or(0.0, |x| x.prestige));
                for (t, p) in &s.prestige {
                    if let Some(x) = state.countries.get_mut(t) {
                        x.prestige = (x.prestige + p).max(0.0);
                    }
                }
                for (a, b, d) in &s.relations {
                    dip.change_relation(&dr, a, b, *d);
                }
                let region = state.provinces.get(&c.province).and_then(|p| p.region);
                let mut moved = 0;
                if s.state_ceded {
                    let ids: Vec<u16> = state.provinces.iter().filter(|(_, p)| region.is_some() && p.region == region && p.owner.as_deref() == Some(c.defender.as_str())).map(|(id, _)| *id).collect();
                    for p in &ids {
                        if let Some(x) = state.provinces.get_mut(p) {
                            x.owner = Some(c.attacker.clone());
                            x.controller = Some(c.attacker.clone());
                        }
                        if let Some(m) = self.mil.provinces.get_mut(p) {
                            m.owner = Some(c.attacker.clone());
                            m.controller = Some(c.attacker.clone());
                        }
                    }
                    moved = ids.len();
                    log.provinces_transferred += moved;
                }
                log.crisis.push(format!("{today}: crisis {} v {} settled at temperature {:.1}, {} wins{}", c.attacker, c.defender, c.temperature, if s.winner == crisis::Side::Attacker { &c.attacker } else { &c.defender }, if moved > 0 { format!(", {moved} provinces ceded") } else { String::new() }));
                for f in cs.flashpoints.iter_mut().filter(|f| state.provinces.get(&f.province).and_then(|p| p.region) == region) {
                    f.tension = 0.0;
                }
                cs.last_end = Some(today);
            } else {
                cs.crisis = Some(c);
            }
        } else if crisis::cooldown_over(&r, &cs, today)
            && self.roll01() < crisis::start_chance(&r)
            && let Some(f) = crisis::crisis_candidate(&r, &cs, &|t| owns_land.contains(t)).cloned()
        {
            {
                let candidates: Vec<(String, f64, bool)> = powers.iter().map(|g| (g.clone(), state.countries.get(g).map_or(0.0, |x| x.war_exhaustion), at_war.contains(g.as_str()))).collect();
                let interested = crisis::may_be_interested(&r, &candidates, &f.tag, &f.owner);
                // STAND-IN: the attacker's backer is the interested power that likes it best (relation above 0).
                let backer = interested.iter().map(|g| (g.clone(), dip.relation_value(g, &f.tag))).filter(|(_, v)| *v > 0.0).max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0))).map(|(g, _)| g);
                if let Some(b) = backer {
                    let rest: Vec<String> = interested.into_iter().filter(|g| *g != b).collect();
                    log.crisis.push(format!("{today}: crisis {} v {} over the state of province {} (tension {:.0}), {b} backs the attacker, {} interested", f.tag, f.owner, f.province, f.tension, rest.len()));
                    cs.crisis = Some(crisis::start_crisis(&r, &f, &b, rest, today));
                }
            }
        }
        self.dip = Some((dr, dip));
        self.crisis = Some((r, cs));
    }

    fn unrest_month(&mut self, state: &mut State, log: &mut CoreLog) {
        let Some((rules, mut u)) = self.unrest.take() else { return };
        self.movement_month(state, &rules, &mut u, log);
        self.faction_month(state, &rules, &mut u, log);
        let index = pop_index(state);
        let pop = |id: u64| index.get(&id).and_then(|(p, i)| state.provinces.get(p).and_then(|q| q.pops.get(*i)));
        let month_days = 30.0;
        let mut spawns: Vec<unrest::SpawnOrder> = Vec::new();
        let mut militancy: BTreeMap<u64, f64> = BTreeMap::new();
        for f in &mut u.factions {
            f.pops.retain(|id| index.contains_key(id));
            let members: Vec<unrest::Member> = f.pops.iter().filter_map(|id| pop(*id)).map(|q| unrest::Member { money_today: q.money * rules.org_money_fit, literacy: q.literacy, militancy: q.militancy, brigades: unrest::brigades(&rules, q.size) }).collect();
            // The daily gain taken a month at a time. NOT WIRED: admin spending and technology bonus, and each
            // faction's own day of the month for its rising roll (all roll on the 1st).
            f.organization = (f.organization + month_days * unrest::organisation_gain(&rules, &members, 0.0, 0.0)).clamp(0.0, 1.0);
            let angry: u32 = f.pops.iter().filter_map(|id| pop(*id)).filter(|q| q.militancy >= rules.mil_to_join_rising).map(|q| unrest::brigades(&rules, q.size)).sum();
            let army = self.mil.regiment_count(&f.country) as u32;
            let Some(chance) = unrest::rising_chance(&rules, f.organization, angry, army) else { continue };
            if !unrest::rises(&rules, chance, splitmix(&mut self.rng)) {
                continue;
            }
            let rising: Vec<unrest::RisingPop> = f
                .pops
                .iter()
                .filter_map(|id| Some((*id, pop(*id)?, index.get(id)?.0)))
                .map(|(id, q, p)| {
                    let limit = self.day.supply_limit(state, &self.day.rebel_tag, p, SupplyAccess::Friendly);
                    unrest::RisingPop { pop_id: id, size: q.size, militancy: q.militancy, brigades_in_service: 0, province: u32::from(p), rebel_supply_limit: limit as f64 }
                })
                .collect();
            let res = unrest::rise(&rules, f, &rising);
            f.organization = res.organisation;
            militancy.extend(res.militancy);
            spawns.extend(res.orders);
            log.risings += 1;
        }
        for (id, m) in militancy {
            if let Some((p, i)) = index.get(&id) && let Some(q) = state.provinces.get_mut(p).and_then(|p| p.pops.get_mut(*i)) {
                q.militancy = m;
            }
        }
        // Spawned regiments become rebel armies, one per province.
        let unit = self.day.units.get(&self.numbers.irregular_unit).cloned().unwrap_or_default();
        let mut by_province: BTreeMap<u16, Vec<Regiment>> = BTreeMap::new();
        for s in spawns {
            let pop_type = index.get(&s.pop_id).and_then(|(p, i)| state.provinces.get(p)?.pops.get(*i)).map(|q| q.pop_type.clone());
            for _ in 0..s.regiments {
                let id = self.new_id();
                by_province.entry(s.province as u16).or_default().push(Regiment { id, name: String::new(), kind: self.numbers.irregular_unit.clone(), pop: Some(s.pop_id), pop_type: pop_type.clone(), organisation: unit.organisation, strength: unit.max_strength, experience: 0.0, name_count: 0 });
            }
        }
        for (at, regs) in by_province {
            log.rebel_regiments += regs.len();
            let id = self.new_id();
            self.mil.armies.push(Army { id, name: "Rebels".into(), owner: self.day.rebel_tag.clone(), location: at, supplies: 1.0, regiments: regs, ..Army::default() });
        }
        // NOT WIRED: suppression, rebel victories
        // (demands_enforced), defection of rebel-held provinces.
        self.unrest = Some((rules, u));
    }

    fn colony_month(&mut self, state: &State) {
        let Some((rules, c)) = &self.colonies else { return };
        let mut points = BTreeMap::new();
        let _ = (rules, c);
        for (rank, tag) in state.rank_order.iter().enumerate() {
            if let Some(p) = self.colonial_points_of(state, tag, (rank + 1) as f64) {
                points.insert(tag.clone(), p.free());
            }
        }
        self.colonial_points = points;
    }

    /// A country's colonial points with their parts: its technologies' points, its naval bases'
    /// points (the base building's list by level; overseas without a core a flat amount for every
    /// coastal state instead) and its ships'
    /// points, the last cut by how far its ships overuse its naval supply; and what is in use:
    /// points standing in races and the upkeep of protectorates and colonies. `rank` is its
    /// overall rank, 1 = first. None before the colonies and the buildings are attached.
    pub fn colonial_points_of(&self, state: &State, tag: &str, rank: f64) -> Option<colonies::Points> {
        let ((rules, c), buildings) = (self.colonies.as_ref()?, self.buildings.as_ref()?);
        let country = state.countries.get(tag)?;
        let capital_continent = country.capital.and_then(|p| state.provinces.get(&p)).and_then(|p| p.continent.clone());
        let per_level: &[f64] = buildings.defs.iter().find(|d| d.kind == "naval_base").map_or(&[], |d| &d.colonial_points);
        let mut out = colonies::Points { techs: rules.tech_points.iter().filter(|(t, _)| country.technologies.contains(*t)).map(|(t, x)| (t.clone(), *x)).collect(), ..Default::default() };
        // Naval supply counts a state once: its best base, or the empty score when it has a coast.
        let mut naval: BTreeMap<Option<u16>, crate::buildings::StateNaval> = BTreeMap::new();
        for (id, p) in state.provinces.iter().filter(|(_, p)| p.owner.as_deref() == Some(tag)) {
            let level = c.provinces.get(id).map_or(0.0, |x| x.naval_base) as u32;
            let (is_core, home) = (p.cores.iter().any(|x| x == tag), p.continent == capital_continent);
            let s = naval.entry(p.region).or_default();
            s.coastal |= p.coastal;
            if level > s.best_level || (s.best_level == 0 && p.coastal) {
                (s.best_level, s.non_core_overseas) = (level, !is_core && !home);
            }
            if level > 0 && (is_core || home) {
                out.bases.push((*id, level, colonies::naval_base_points(per_level, level)));
            }
        }
        out.supply_states = naval.into_iter().map(|(region, s)| (region, s.best_level, s.non_core_overseas, crate::buildings::naval_supply_limit(&buildings.n, std::slice::from_ref(&s)))).filter(|x| x.3 != 0.0).collect();
        out.supply_limit = out.supply_states.iter().map(|x| x.3).sum();
        out.overseas = out.supply_states.iter().filter(|x| x.2).map(|x| (x.0, rules.non_core_base_points)).collect();
        let mut ships: BTreeMap<&str, f64> = BTreeMap::new();
        for ship in self.mil.navies_of(tag).flat_map(|n| &n.ships) {
            *ships.entry(ship.kind.as_str()).or_default() += 1.0;
        }
        for (kind, n) in ships {
            let Some(u) = self.day.units.get(kind) else { continue };
            out.supply_used += n * u.supply_consumption_score;
            if u.colonial_points != 0.0 {
                out.ships.push((kind.to_string(), n, u.colonial_points));
            }
        }
        out.overuse = colonies::overuse(out.supply_used, out.supply_limit);
        // NOT WIRED: the stored bonus (events).
        let input = colonies::PointsInput { rank, tech_points: out.techs.iter().map(|t| t.1).sum(), base_points: out.bases.iter().map(|b| b.2).sum::<f64>() + out.overseas.iter().map(|b| b.1).sum::<f64>(), ship_points: out.ships.iter().map(|s| s.1 * s.2).sum(), supply_overuse: out.overuse, bonus: 0.0 };
        out.total = colonies::total_points(rules, &input);
        out.races = self.races.values().flat_map(|r| &r.contenders).filter(|x| x.tag == tag).map(|x| x.points).sum();
        let holdings = c.holdings_of(tag);
        out.upkeep = holdings.iter().map(|h| colonies::upkeep_of(rules, h)).sum();
        out.used = colonies::points_used(rules, rank, out.races, &holdings);
        Some(out)
    }

    fn foreign_minister(&mut self, state: &mut State, log: &mut CoreLog) {
        let scores: BTreeMap<String, f64> = self.day.war_scores(&self.mil, state).into_iter().collect();
        let today = state.date;
        // The wars' direction. STAND-IN: looked at on the minister's day, not every day.
        for (name, score) in &scores {
            war::update_direction(&self.day.war, self.war_direction.entry(name.clone()).or_default(), *score, today);
        }
        self.war_direction.retain(|name, _| scores.contains_key(name));
        let wr = &self.day.war;
        let strength = |tags: &[String], mil: &MilitaryState| -> f64 { tags.iter().map(|t| mil.armies_of(t).map(|a| logistics::stack_weight(&[a])).sum::<f64>()).sum() };
        let owners: BTreeSet<&str> = state.provinces.values().filter_map(|p| p.owner.as_deref()).collect();
        let mut offers: Vec<(usize, String, String, Vec<usize>, String)> = Vec::new();
        let mut wanted_goals: Vec<(usize, String, String, f64)> = Vec::new();
        let human_of = |t: &str| state.countries.get(t).is_some_and(|c| c.human);
        for (i, w) in self.mil.wars.iter().enumerate().filter(|(_, w)| w.active) {
            let (Some(a), Some(d)) = (w.attacker_leader(), w.defender_leader()) else { continue };
            let score = scores.get(&w.name).copied().unwrap_or(0.0);
            let (sa, sd) = (strength(&w.attackers, &self.mil), strength(&w.defenders, &self.mil));
            // The winning leader is M, asked to accept a white peace from the loser.
            let (m, m_score, own, foe) = if score >= 0.0 { (a, score, sa, sd) } else { (d, -score, sd, sa) };
            let human = human_of(m);
            let exh = self.mil.countries.get(m).map_or(0.0, |c| c.war_exhaustion);
            let goal_in = war::AiGoalInput { exhaustion: exh, max_exhaustion: wr.exh_war_max, land_ratio: own / foe.max(1e-9), score: m_score, own_goals: w.goals.iter().filter(|g| g.actor == m).count() as u32, great_war: false, score_above_demanded: false };
            if !human && war::ai_wants_goal(wr, &goal_in) {
                log.goals_wanted += 1;
                let enemy = if m == a { d } else { a };
                wanted_goals.push((i, m.to_string(), enemy.to_string(), m_score));
            }
            // Calendar months of war, days not counted.
            let months = w.started().map_or(0.0, |s| f64::from((today.year - s.year) * 12 + i32::from(today.month) - i32::from(s.month)));
            let ctx = PeaceCtx { war: w, months, score, direction: self.war_direction.get(&w.name).map_or(0.0, |d| d.value) };
            // A war in which no goal is left ends by itself in a white peace between the leaders;
            // a leader a person plays is asked (by the other leader; nothing when both are played).
            if w.goals.is_empty() {
                let (sender, target) = if human_of(a) { (d, a) } else { (a, d) };
                if !human_of(sender) && (!human_of(target) || self.asks.may_ask_peace(&w.name, sender, target, today)) {
                    offers.push((i, sender.to_string(), target.to_string(), Vec::new(), "no goal left in the war".to_string()));
                }
                continue;
            }
            // 2.4 step 3: every AI country offers peace to each enemy it may negotiate with (one of
            // the two leads its side). One peace per war and round.
            // A country a person plays sends no offer from the computer; one sent to it is asked
            // (player.rs), and its answer is the computer's guess while the offer is shaped.
            // NOT WIRED: the diplomatic point an offer costs and the pause between two offers to one country.
            let party = |t: &str| t != self.day.rebel_tag && owners.contains(t);
            'war: for (c, attacks) in w.attackers.iter().map(|c| (c, true)).chain(w.defenders.iter().map(|c| (c, false))) {
                for e in if attacks { &w.defenders } else { &w.attackers } {
                    if !(war::is_leader(w, c) || war::is_leader(w, e)) || !party(c) || !party(e) || human_of(c) || (human_of(e) && !self.asks.may_ask_peace(&w.name, c, e, today)) {
                        continue;
                    }
                    // STAND-IN: a member's score is its leader's (the member's own occupation score is not counted).
                    let s = if attacks { score } else { -score };
                    // A winning sender demands its side's goals and the enemy has to sign; any other
                    // offers the enemy's goals as far as it would sign them itself.
                    let winning = s >= wr.ai_peace_winning_score;
                    let (demands, concedes) = if winning { (c, e) } else { (e, c) };
                    let Some((table, forced)) = Self::peace_table(w, &self.day.cb_types, demands, concedes) else { continue };
                    let obligatory = |g: usize| forced.contains(&g) || w.goals.get(g).and_then(|g| self.day.cb_types.get(&g.casus_belli)).is_some_and(|c| c.zero_cost);
                    let answer = |m: &str, o: &str, kept: &[usize]| self.peace_answer(state, &ctx, m, o, kept);
                    let signs = |kept: &[usize]| answer(concedes, demands, kept).0;
                    let both = |kept: &[usize]| answer(e, c, kept).0 && answer(c, e, kept).0;
                    if let Some(kept) = war_ai::peace_offer(&table, &obligatory, &signs, &both) {
                        let cost: f64 = kept.iter().map(|&g| self.goal_cost(state, &w.goals[g])).sum();
                        let (wc, we) = (answer(c, e, &kept).1, answer(e, c, &kept).1);
                        let why = format!("{c} {} at score {s:+.0} in month {months:.0}, cost {cost:.0}, willingness {c} {wc:+.0} {e} {we:+.0}", if winning { "demands" } else { "concedes" });
                        offers.push((i, c.clone(), e.clone(), kept, why));
                        break 'war;
                    }
                }
            }
        }
        for (i, sender, target, kept, why) in offers {
            if crate::player::is_player(state, &target) {
                let w = &self.mil.wars[i];
                let goals = kept.iter().filter_map(|&g| w.goals.get(g).cloned()).collect();
                self.asks.peace(&target, &sender, &w.name.clone(), goals, today, &self.day.calendar);
            } else {
                self.execute_peace(state, i, &sender, &target, &kept, &why, log);
            }
        }
        // 2.4 step 2: add a goal (one per war and round) where the AI wants one.
        for (i, m, enemy, m_score) in wanted_goals {
            if self.mil.wars[i].active && self.add_ai_goal(state, i, &m, &enemy, m_score) {
                log.goals_added += 1;
            }
        }
        state.wars = self.mil.wars.iter().filter(|w| w.active).map(|w| (w.attackers.iter().cloned().collect(), w.defenders.iter().cloned().collect())).collect();
    }

    /// The goals on the table when `demands` asks `concedes` for peace: the goals of the demanding
    /// country, or of its whole side when it leads; against a member only those it is to give.
    /// The second list holds goals that may not be dropped: a separate peace that would take every
    /// goal out of the war must annex, and is no offer (`None`) when it cannot.
    fn peace_table(w: &War, cbs: &BTreeMap<String, CbTypeRef>, demands: &str, concedes: &str) -> Option<(Vec<usize>, Vec<usize>)> {
        let (dl, cl) = (war::is_leader(w, demands), war::is_leader(w, concedes));
        let side = if w.attackers.iter().any(|t| t == demands) { &w.attackers } else { &w.defenders };
        let on_table = |g: &WarGoal| (if dl { side.contains(&g.actor) } else { g.actor == demands }) && (cl || g.receiver == concedes);
        let table: Vec<usize> = (0..w.goals.len()).filter(|&g| on_table(&w.goals[g])).collect();
        // NOT WIRED: the vassals of a conceding member; the obligatory goals a great war adds.
        let mut forced = Vec::new();
        if !(dl && cl) && table.len() == w.goals.len() {
            forced = table.iter().copied().filter(|&g| cbs.get(&w.goals[g].casus_belli).is_some_and(|c| c.has("annex"))).collect();
            if forced.is_empty() {
                return None;
            }
        }
        Some((table, forced))
    }

    /// Land and naval strength of some countries. STAND-IN: men under arms in thousands and the
    /// number of ships (the original values the unit types and adds the land tactics).
    fn strength<'a>(&'a self, tags: impl Iterator<Item = &'a str>) -> (f64, f64) {
        tags.fold((0.0, 0.0), |(land, sea), t| (land + self.mil.armies_of(t).map(|a| logistics::stack_weight(&[a])).sum::<f64>(), sea + self.mil.ship_count(t) as f64))
    }

    /// The AI's answer of `m` to a treaty with `o` that enforces `kept`, and its willingness.
    fn peace_answer(&self, state: &State, ctx: &PeaceCtx, m: &str, o: &str, kept: &[usize]) -> (bool, f64) {
        let (w, wr) = (ctx.war, &self.day.war);
        let m_attacks = w.attackers.iter().any(|t| t == m);
        let (m_side, o_side) = if m_attacks { (&w.attackers, &w.defenders) } else { (&w.defenders, &w.attackers) };
        let (m_leader, o_leader) = (war::is_leader(w, m), war::is_leader(w, o));
        let cb = |g: &WarGoal| self.day.cb_types.get(&g.casus_belli);
        let obligatory = |g: &WarGoal| cb(g).is_some_and(|c| c.zero_cost);
        // The goals of M's side that M may count: its own, or its side's when it leads.
        let mine: Vec<(f64, bool)> = (0..w.goals.len())
            .filter(|&g| m_side.contains(&w.goals[g].actor) && (m_leader || w.goals[g].actor == m) && (o_leader || w.goals[g].receiver == o))
            .map(|g| (self.goal_cost(state, &w.goals[g]), kept.contains(&g)))
            .collect();
        let (own_goals, own_granted) = war::count_own_goals(wr, &mine);
        let chosen: Vec<&WarGoal> = kept.iter().filter_map(|&g| w.goals.get(g)).collect();
        // Everyone a country is at war with, in all its wars.
        let enemies_of = |t: &str| -> BTreeSet<&str> {
            let mut out = BTreeSet::new();
            for x in self.mil.wars.iter().filter(|x| x.active) {
                if x.attackers.iter().any(|y| y == t) {
                    out.extend(x.defenders.iter().map(String::as_str));
                } else if x.defenders.iter().any(|y| y == t) {
                    out.extend(x.attackers.iter().map(String::as_str));
                }
            }
            out
        };
        let (m_enemies, mut o_others) = (enemies_of(m), enemies_of(o));
        o_others.retain(|t| !m_side.iter().any(|x| x == t));
        let (own, foe) = (self.strength(m_side.iter().map(String::as_str)), self.strength(o_side.iter().map(String::as_str)));
        let (own_enemies, foe_others) = (self.strength(m_enemies.into_iter()), self.strength(o_others.into_iter()));
        let m_score = if m_attacks { ctx.score } else { -ctx.score };
        let input = PeaceAnswerInput {
            m_is_leader: m_leader,
            o_is_leader: o_leader,
            m_score,
            o_score: -m_score,
            direction: if m_attacks { ctx.direction } else { -ctx.direction },
            months_of_war: ctx.months,
            m_exhaustion: self.mil.countries.get(m).map_or(0.0, |c| c.war_exhaustion),
            land: war::force_ratio(wr, own.0, foe.0, own_enemies.0, foe_others.0),
            naval: war::force_ratio(wr, own.1, foe.1, own_enemies.1, foe_others.1),
            offer_cost: chosen.iter().map(|g| self.goal_cost(state, g)).sum(),
            own_goals,
            own_granted,
            annexes_m: chosen.iter().any(|g| g.receiver == m && cb(g).is_some_and(|c| c.has("annex"))),
            has_enemy_obligatory: chosen.iter().any(|g| o_side.contains(&g.actor) && obligatory(g)),
            has_own_obligatory: chosen.iter().any(|g| m_side.contains(&g.actor) && obligatory(g)),
            m_infamy: state.countries.get(m).map_or(0.0, |c| c.badboy),
            // NOT WIRED: whether M may still add goals (jingoism, status quo): taken as yes.
            m_cannot_add_goals: false,
            m_holds_cb: o_side.iter().any(|x| self.held_cbs.get(&(m.to_string(), x.clone())).is_some_and(|v| !v.is_empty())),
        };
        (war::ai_accepts_peace(wr, &input), war::ai_willingness(wr, &input))
    }

    /// Provinces a goal with a state province covers: that province's state as its owner holds it.
    fn goal_provinces(state: &State, g: &WarGoal) -> Vec<u16> {
        let Some(p) = g.state_province.and_then(|p| state.provinces.get(&p).map(|s| (p, s))) else { return Vec::new() };
        let (region, owner) = (p.1.region, p.1.owner.clone());
        state.provinces.iter().filter(|(id, s)| **id == p.0 || (region.is_some() && s.region == region && s.owner == owner)).map(|(id, _)| *id).collect()
    }

    fn province_value(&self, state: &State, id: u16) -> f64 {
        let (Some(s), Some(m)) = (state.provinces.get(&id), self.mil.provinces.get(&id)) else { return 1.0 };
        let Some(owner) = s.owner.as_deref() else { return 1.0 };
        let cap_region = state.countries.get(owner).and_then(|c| c.capital).and_then(|c| state.provinces.get(&c)).and_then(|c| c.region);
        war::province_value(
            &self.day.war,
            &war::ProvinceValueInput {
                naval_base_level: m.naval_base,
                state_factories: 0.0,
                fort_level: m.fort,
                owner_has_core: s.cores.iter().any(|c| c == owner),
                in_owner_capital_state: s.region.is_some() && s.region == cap_region,
                overseas: false,
                colonial_or_unowned: s.colonial > 0,
            },
        )
    }

    /// Cost of a goal in war score points (5.1).
    fn goal_cost(&self, state: &State, g: &WarGoal) -> f64 {
        let Some(cb) = self.day.cb_types.get(&g.casus_belli) else { return 0.0 };
        let total: f64 = state.provinces.iter().filter(|(_, p)| p.owner.as_deref() == Some(g.receiver.as_str())).map(|(id, _)| self.province_value(state, *id)).sum();
        let provinces = Self::goal_provinces(state, g).iter().map(|p| (self.province_value(state, *p), total)).collect();
        war::goal_cost(&self.day.war, cb, &war::GoalCostInput { provinces, ..Default::default() })
    }

    /// A province of `t` for a state goal: the lowest-numbered one outside its capital's state, else any.
    pub(crate) fn goal_state_province(state: &State, t: &str) -> Option<u16> {
        let cap = state.countries.get(t).and_then(|c| c.capital).and_then(|c| state.provinces.get(&c)).and_then(|p| p.region);
        let own: Vec<(u16, Option<u16>)> = state.provinces.iter().filter(|(_, p)| p.owner.as_deref() == Some(t)).map(|(id, p)| (*id, p.region)).collect();
        own.iter().find(|(_, r)| r.is_some() && *r != cap).or(own.first()).map(|(id, _)| *id)
    }

    fn states_of(state: &State, t: &str) -> f64 {
        state.provinces.values().filter(|p| p.owner.as_deref() == Some(t)).filter_map(|p| p.region).collect::<BTreeSet<u16>>().len() as f64
    }

    /// The goal picker's facts for `c` against `t`.
    #[allow(clippy::too_many_arguments)]
    fn goal_context(&self, state: &State, states: Option<&BTreeMap<String, f64>>, c: &str, t: &str, ceiling: f64, held_only: bool, at_war: bool, cost_so_far: f64) -> war_ai::GoalContext {
        let cs = state.countries.get(c);
        let ts = state.countries.get(t);
        let great = |x: &str| state.great_powers.iter().any(|g| g == x);
        let civ = |x: Option<&crate::script::CountryState>| x.is_some_and(|x| x.civilized);
        let infamy = |x: Option<&crate::script::CountryState>| x.map_or(0.0, |x| x.badboy);
        war_ai::GoalContext {
            own_infamy: infamy(cs),
            ceiling,
            held_only,
            at_war,
            t_states: states.map_or_else(|| Self::states_of(state, t), |m| m.get(t).copied().unwrap_or(0.0)),
            own_states: states.map_or_else(|| Self::states_of(state, c), |m| m.get(c).copied().unwrap_or(0.0)),
            t_military: ts.map_or(0.0, |x| x.military_score),
            own_military: cs.map_or(0.0, |x| x.military_score),
            // STAND-IN: industrial strength read as total score minus military score (no cached industrial score).
            t_industry: ts.map_or(0.0, |x| (x.total_score - x.military_score).max(0.0)),
            own_industry: cs.map_or(0.0, |x| (x.total_score - x.military_score).max(0.0)),
            t_prestige: ts.map_or(0.0, |x| x.prestige),
            own_prestige: cs.map_or(0.0, |x| x.prestige),
            t_over_limit: infamy(ts) > self.day.war.badboy_limit,
            alike: great(c) == great(t) && civ(cs) == civ(ts),
            t_civilised: civ(ts),
            great_war: false,
            cost_so_far,
        }
    }

    /// Mid-war goal (2.4 step 2): the picker with ceiling BADBOY_LIMIT - margin, then war::can_add_goal.
    fn add_ai_goal(&mut self, state: &mut State, i: usize, m: &str, enemy: &str, m_score: f64) -> bool {
        let Some(r) = self.war_ai.clone() else { return false };
        let w = self.mil.wars[i].clone();
        let wr = &self.day.war;
        let infamy = state.countries.get(m).map_or(0.0, |c| c.badboy);
        let status_quo = w.goals.iter().filter(|g| w.attackers.contains(&g.actor) == w.attackers.iter().any(|x| x == m)).any(|g| self.day.cb_types.get(&g.casus_belli).is_some_and(|c| c.has("status_quo")));
        // Share of m's people backing jingoism (the issue's share of every pop, weighted by size).
        let jingo = state.issues.iter().position(|(o, _)| o == "jingoism").map(|n| n as u32 + 1);
        let (mut j, mut people) = (0.0, 0.0);
        for p in state.provinces.values().filter(|p| p.owner.as_deref() == Some(m)) {
            for q in &p.pops {
                people += q.size;
                j += q.size * jingo.and_then(|n| q.issues.get(&n)).copied().unwrap_or(0.0) / 100.0;
            }
        }
        let jingoism_share = if people > 0.0 { j / people } else { 0.0 };
        let may_add_unjust = war::can_add_goal(wr, &war::AddGoalInput { status_quo_goal: status_quo, score_vs_leader: m_score, jingoism_share, great_war: false }).is_ok();
        let held_only = infamy > r.held_only_infamy_war || !may_add_unjust;
        let held: Vec<String> = self.held_cbs.get(&(m.to_string(), enemy.to_string())).map(|v| v.iter().map(|h| h.cb_type.clone()).collect()).unwrap_or_default();
        let cost_so_far: f64 = w.goals.iter().filter(|g| g.actor == m).map(|g| self.goal_cost(state, g)).sum();
        let types: Vec<CbTypeRef> = self.day.cb_types.values().filter(|c| !c.needs_no_justification && (c.constructing || held.contains(&c.name))).cloned().collect();
        let sp = Self::goal_state_province(state, enemy);
        let goal_of = |c: &CbTypeRef| WarGoal { casus_belli: c.name.clone(), actor: m.to_string(), receiver: enemy.to_string(), state_province: if c.has("demand_state") { sp } else { None }, date: Some(state.date), ..WarGoal::default() };
        let options: Vec<war_ai::GoalOption> = types
            .iter()
            .map(|c| war_ai::GoalOption { cb: c, held: held.contains(&c.name), cost: self.goal_cost(state, &goal_of(c)), present: w.goals.iter().any(|g| g.casus_belli == c.name && g.actor == m && g.receiver == enemy) })
            .collect();
        let ctx = self.goal_context(state, None, m, enemy, war::ai_goal_infamy_ceiling(wr), held_only, true, cost_so_far);
        let Some((k, v)) = war_ai::pick_goal(&r, wr, &options, &ctx, None) else { return false };
        if !war::ai_goal_value_ok(wr, v) {
            return false;
        }
        let cb = &types[k];
        let is_held = held.contains(&cb.name);
        let cost = war::add_goal_cost(wr, cb, is_held, false, false);
        if is_held && let Some(v) = self.held_cbs.get_mut(&(m.to_string(), enemy.to_string())) {
            v.retain(|h| h.cb_type != cb.name);
        }
        if let Some(c) = state.countries.get_mut(m) {
            c.badboy += cost.infamy;
        }
        let g = goal_of(cb);
        let war = &mut self.mil.wars[i];
        war.history.push(WarEvent { date: state.date, kind: WarEventKind::WarGoal(g.clone()) });
        war.goals.push(g);
        true
    }

    /// Executes an accepted peace between `sender` and `target` in war `i`.
    #[allow(clippy::too_many_arguments)]
    fn execute_peace(&mut self, state: &mut State, i: usize, sender: &str, target: &str, kept: &[usize], why: &str, log: &mut CoreLog) {
        let w = self.mil.wars[i].clone();
        let today = state.date;
        let req = PeaceRequest { war: &w, crisis: false, sender, target, selected: kept };
        let prestige: BTreeMap<String, f64> = state.countries.iter().map(|(t, c)| (t.clone(), c.prestige)).collect();
        let possible = |g: &WarGoal| state.provinces.values().any(|p| p.owner.as_deref() == Some(g.receiver.as_str()));
        let Ok(out) = war::plan_peace(&self.day.war, &req, &self.day.cb_types, &possible, &|t| prestige.get(t).copied().unwrap_or(0.0)) else { return };
        let mut trace = format!("{today}: {}: {sender} -> {target} ({why}), {} goals in the war, enforced:", w.name, w.goals.len());
        for (_, actor, pen) in &out.failed {
            if let Some(c) = state.countries.get_mut(actor) {
                c.prestige = (c.prestige + pen.prestige).max(0.0);
            }
            for p in state.provinces.values_mut().filter(|p| p.owner.as_deref() == Some(actor.as_str())) {
                for q in &mut p.pops {
                    q.militancy = (q.militancy + pen.militancy).clamp(0.0, 10.0);
                }
            }
        }
        for &g in &out.enforced {
            let goal = &w.goals[g];
            let Some(cb) = self.day.cb_types.get(&goal.casus_belli).cloned() else { continue };
            let moved: Vec<u16> = if cb.has("annex") {
                state.provinces.iter().filter(|(_, p)| p.owner.as_deref() == Some(goal.receiver.as_str())).map(|(id, _)| *id).collect()
            } else if cb.has("demand_state") || cb.has("transfer_provinces") {
                // NOT WIRED: transfer_provinces to a third country; it goes to the actor here.
                Self::goal_provinces(state, goal).into_iter().filter(|p| state.provinces.get(p).and_then(|s| s.owner.as_deref()) == Some(goal.receiver.as_str())).collect()
            } else {
                // NOT WIRED: disarmament, reparations, humiliation's prestige cut, sphere, puppet, government goals.
                Vec::new()
            };
            for p in &moved {
                if let Some(s) = state.provinces.get_mut(p) {
                    s.owner = Some(goal.actor.clone());
                    s.controller = Some(goal.actor.clone());
                }
                if let Some(m) = self.mil.provinces.get_mut(p) {
                    m.owner = Some(goal.actor.clone());
                    m.controller = Some(goal.actor.clone());
                }
            }
            log.provinces_transferred += moved.len();
            trace.push_str(&format!(" [{} {}->{} cost {:.0}, {} provinces to {}]", goal.casus_belli, goal.actor, goal.receiver, self.goal_cost(state, goal), moved.len(), goal.actor));
            let gain = war::goal_prestige(&self.day.war, &cb, prestige.get(&goal.actor).copied().unwrap_or(0.0));
            if let Some(c) = state.countries.get_mut(&goal.actor) {
                c.prestige = (c.prestige + gain).max(0.0);
            }
            if let Some(c) = state.countries.get_mut(&goal.receiver) {
                c.prestige = (c.prestige - gain).max(0.0);
            }
        }
        if out.enforced.is_empty() {
            trace.push_str(" nothing (white peace)");
        }
        log.peace_trace.push(trace);
        let leaving: BTreeSet<&String> = out.leaving.iter().collect();
        let attacking = |t: &str| w.attackers.iter().any(|x| x == t);
        let enemies_of = |t: &str| -> Vec<&String> { if attacking(t) { w.defenders.iter().collect() } else { w.attackers.iter().collect() } };
        let pairs: Vec<(String, String)> = out.leaving.iter().flat_map(|x| enemies_of(x).into_iter().map(move |y| (x.clone(), y.clone()))).collect();
        let pair_set: BTreeSet<(&str, &str)> = pairs.iter().flat_map(|(x, y)| [(x.as_str(), y.as_str()), (y.as_str(), x.as_str())]).collect();
        // Occupied provinces between the parties go back to their owners.
        for (id, p) in self.mil.provinces.iter_mut() {
            if let (Some(o), Some(c)) = (&p.owner, &p.controller) && o != c && pair_set.contains(&(o.as_str(), c.as_str())) {
                p.controller = p.owner.clone();
                if let Some(s) = state.provinces.get_mut(id) {
                    s.controller = s.owner.clone();
                }
            }
        }
        let armies: BTreeMap<u64, String> = self.mil.armies.iter().map(|x| (x.id, x.owner.clone())).collect();
        self.mil.combats.retain(|c| c.kind == CombatKind::Naval || !c.attacker.units.iter().chain(&c.defender.units).any(|u| armies.get(u).is_some_and(|o| leaving.contains(o))));
        if let Some((_, dip)) = &mut self.dip {
            let until = war::add_months(today, out.truce_months.max(0.0) as u32);
            for (x, y) in &pairs {
                dip.relation_mut(x, y).truce_until = Some(until);
                dip.relation_mut(y, x).truce_until = Some(until);
            }
        }
        let war = &mut self.mil.wars[i];
        match out.scope {
            PeaceScope::WholeWar => {
                war.active = false;
                war.attackers.clear();
                war.defenders.clear();
                log.peaces.push(w.name.clone());
            }
            PeaceScope::Separate(x) => {
                let kind = if attacking(&x) { WarEventKind::RemAttacker(x.clone()) } else { WarEventKind::RemDefender(x.clone()) };
                war.attackers.retain(|t| *t != x);
                war.defenders.retain(|t| *t != x);
                war.goals.retain(|g| g.actor != x && g.receiver != x);
                war.history.push(WarEvent { date: today, kind });
                log.separate_peaces += 1;
                log.peaces.push(format!("{} ({x} leaves)", w.name));
            }
        }
    }

    /// `c` declares war on `t` with a goal of type `kind` (held or not: `declare_war` prices it),
    /// and both leaders call their allies. An ally a person plays is asked (player.rs) instead of
    /// answering by the computer's rule. Returns the war's name.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_war(&mut self, state: &mut State, rules: &DipRules, dip: &mut DiplomacyState, r: &WarAiRules, c: &str, t: &str, kind: &str, truce: bool, at_war: &mut BTreeSet<String>, log: &mut CoreLog) -> Result<String, String> {
        let today = state.date;
        let rebel = self.day.rebel_tag.clone();
        let Some(cb) = self.day.cb_types.get(kind).cloned() else { return Err(format!("no casus belli {kind}")) };
        let key = (c.to_string(), t.to_string());
        let holds_cb = self.held_cbs.get(&key).is_some_and(|v| v.iter().any(|h| h.cb_type == kind));
        let d = war::declare_war(&self.day.war, &cb, &war::DeclareInput { holds_cb, just_flag: false, truce_active: truce, great_war_with_target: false });
        if d.consumes_cb && let Some(v) = self.held_cbs.get_mut(&key) {
            if let Some(pos) = v.iter().position(|h| h.cb_type == kind) {
                v.remove(pos);
            }
            if v.is_empty() {
                self.held_cbs.remove(&key);
            }
        }
        if let Some(s) = state.countries.get_mut(c) {
            s.badboy += d.infamy;
            s.prestige = (s.prestige + d.prestige).max(0.0);
        }
        let goal = WarGoal { casus_belli: kind.to_string(), actor: c.to_string(), receiver: t.to_string(), state_province: if cb.has("demand_state") { Self::goal_state_province(state, t) } else { None }, date: Some(today), ..WarGoal::default() };
        // A name no other war has (it names the war in the players' orders).
        let base = format!("{c}-{t} war of {}", today.year);
        let mut name = base.clone();
        for n in 2.. {
            if !self.mil.wars.iter().any(|w| w.name == name) {
                break;
            }
            name = format!("{base} ({n})");
        }
        let mut w = War {
            name,
            active: true,
            attackers: vec![c.to_string()],
            defenders: vec![t.to_string()],
            original_attacker: Some(c.to_string()),
            original_defender: Some(t.to_string()),
            original_goal: Some(goal.clone()),
            goals: vec![goal.clone()],
            action: Some(today),
            history: vec![
                WarEvent { date: today, kind: WarEventKind::AddAttacker(c.to_string()) },
                WarEvent { date: today, kind: WarEventKind::AddDefender(t.to_string()) },
                WarEvent { date: today, kind: WarEventKind::WarGoal(goal) },
            ],
        };
        at_war.insert(c.to_string());
        at_war.insert(t.to_string());
        // Both leaders call their allies (STAND-IN answer: war_ai::ally_joins).
        for (leader, enemy, attackers) in [(c, t, true), (t, c, false)] {
            let allies: Vec<String> = dip.allies_of(leader).map(str::to_string).collect();
            for ally in allies {
                if w.takes_part(&ally) || ally == rebel {
                    continue;
                }
                if crate::player::is_player(state, &ally) {
                    self.asks.call(&ally, leader, &w.name, attackers, today, &self.day.calendar);
                    continue;
                }
                let joins = war_ai::ally_joins(r, dip.relation_value(&ally, leader), dip.allied(&ally, enemy), at_war.contains(&ally));
                if joins {
                    if attackers {
                        w.attackers.push(ally.clone());
                        w.history.push(WarEvent { date: today, kind: WarEventKind::AddAttacker(ally.clone()) });
                    } else {
                        w.defenders.push(ally.clone());
                        w.history.push(WarEvent { date: today, kind: WarEventKind::AddDefender(ally.clone()) });
                    }
                    at_war.insert(ally);
                } else {
                    // A refused call ends the alliance and costs prestige.
                    crate::diplomacy::treaties::end_alliance(dip, &ally, leader);
                    if let Some(s) = state.countries.get_mut(&ally) {
                        s.prestige = (s.prestige + rules.treaties.call_ally_decline_prestige).max(0.0);
                    }
                }
            }
        }
        dip.relation_mut(c, t).last_war = Some(today);
        dip.relation_mut(t, c).last_war = Some(today);
        let name = w.name.clone();
        log.wars_declared.push(name.clone());
        self.mil.wars.push(w);
        state.wars = self.mil.wars.iter().filter(|w| w.active).map(|w| (w.attackers.iter().cloned().collect(), w.defenders.iter().cloned().collect())).collect();
        Ok(name)
    }

    /// Casus belli under construction, every day: progress, detection, gain; expiry of held ones.
    fn cb_day(&mut self, state: &mut State) {
        if self.war_ai.is_none() {
            return;
        }
        let today = state.date;
        for v in self.held_cbs.values_mut() {
            v.retain(|h| !war::cb_expired(h, today));
        }
        self.held_cbs.retain(|_, v| !v.is_empty());
        let keys: Vec<(String, String)> = self.justifying.keys().cloned().collect();
        for key in keys {
            let Some((kind, progress, discovered)) = self.justifying.get(&key).cloned() else { continue };
            let Some(cb) = self.day.cb_types.get(&kind).cloned() else {
                self.justifying.remove(&key);
                continue;
            };
            // Dropped when the two are at war on opposing sides.
            if Sides::from_wars(&self.mil.wars, &self.day.rebel_tag).hostile(&key.0, &key.1) {
                self.justifying.remove(&key);
                continue;
            }
            // NOT WIRED: cb_generation_speed_modifier, colonial competition and great-war speed-ups.
            let gain = war::cb_daily_progress(&self.day.war, &cb, &war::CbSpeedInput::default());
            let (p, done) = war::cb_build_day(&self.day.war, progress, gain);
            let mut found = discovered;
            let dice = (self.roll() % 1000) as u32;
            if !discovered && war::cb_detected(&self.day.war, dice) {
                found = true;
                let inf = war::cb_detection_infamy(&self.day.war, &cb, p, false);
                if let Some(c) = state.countries.get_mut(&key.0) {
                    c.badboy += inf;
                }
                if let Some((rules, dip)) = &mut self.dip {
                    dip.change_relation(rules, &key.0, &key.1, self.day.war.cb_detected_relation);
                }
            }
            if done {
                self.justifying.remove(&key);
                let held = war::cb_gain(&self.day.war, &kind, today);
                self.held_cbs.entry(key).or_default().push(held);
            } else {
                self.justifying.insert(key, (kind, p, found));
            }
        }
    }

    /// The AI's war declarations: target choice, justifying, relations, declaration.
    fn war_declarations(&mut self, state: &mut State, log: &mut CoreLog) {
        let Some(r) = self.war_ai.clone() else { return };
        let Some((rules, mut dip)) = self.dip.take() else { return };
        let today = state.date;
        let wr = self.day.war.clone();
        let rebel = self.day.rebel_tag.clone();
        let cbs: Vec<CbTypeRef> = self.day.cb_types.values().filter(|c| !c.needs_no_justification).cloned().collect();
        // Facts of every country with land.
        let mut provinces: BTreeMap<String, f64> = BTreeMap::new();
        let mut neighbours: BTreeSet<(String, String)> = BTreeSet::new();
        for (id, p) in &state.provinces {
            let Some(a) = p.owner.as_deref() else { continue };
            *provinces.entry(a.to_string()).or_default() += 1.0;
            for n in state.neighbours.get(id).into_iter().flatten() {
                if let Some(b) = state.provinces.get(n).and_then(|q| q.owner.as_deref()).filter(|b| *b != a) {
                    neighbours.insert((a.to_string(), b.to_string()));
                }
            }
        }
        let mut strength: BTreeMap<String, f64> = BTreeMap::new();
        let mut regiments: BTreeMap<String, f64> = BTreeMap::new();
        for a in &self.mil.armies {
            *strength.entry(a.owner.clone()).or_default() += logistics::stack_weight(&[a]);
            *regiments.entry(a.owner.clone()).or_default() += a.regiments.len() as f64;
        }
        let get = |m: &BTreeMap<String, f64>, t: &str| m.get(t).copied().unwrap_or(0.0);
        let mut owned_by: BTreeMap<String, Vec<u16>> = BTreeMap::new();
        for (id, p) in &state.provinces {
            if let Some(o) = &p.owner {
                owned_by.entry(o.clone()).or_default().push(*id);
            }
        }
        let ranks: BTreeMap<String, f64> = state.rank_order.iter().enumerate().map(|(k, t)| (t.clone(), k as f64 + 1.0)).collect();
        let rank_of = |t: &str| ranks.get(t).copied().unwrap_or(0.0);
        // States per country (goal_context), counted once: nothing below moves a province.
        let state_counts: BTreeMap<String, f64> = owned_by.iter().map(|(t, ids)| (t.clone(), ids.iter().filter_map(|id| state.provinces[id].region).collect::<BTreeSet<u16>>().len() as f64)).collect();
        let mut at_war: BTreeSet<String> = self.mil.wars.iter().filter(|w| w.active).flat_map(|w| w.attackers.iter().chain(&w.defenders)).cloned().collect();
        let tags: Vec<String> = provinces.keys().filter(|t| **t != rebel && state.countries.get(*t).is_some_and(|c| !c.human)).cloned().collect();
        for c in &tags {
            if at_war.contains(c) {
                continue;
            }
            let cs = &state.countries[c];
            let own_infamy = cs.badboy;
            let civ = cs.civilized;
            // Drop a target that owns nothing any more.
            if self.war_targets.get(c).is_some_and(|t| get(&provinces, t) <= 0.0) {
                self.war_targets.remove(c);
            }
            // A. Choose a target.
            if !self.war_targets.contains_key(c) {
                // Sphere leaders and overlords, read once (part A changes no diplomacy; part B of an
                // earlier country may have, so they are read again for each country).
                let mut leaders: BTreeMap<&str, &str> = BTreeMap::new();
                for ((a, b), rel) in &dip.relations {
                    if rel.level == crate::diplomacy::relations::level::IN_SPHERE {
                        leaders.entry(b.as_str()).or_insert(a.as_str());
                    }
                }
                let mut subjects: BTreeSet<&str> = BTreeSet::new();
                for s in &dip.subjects {
                    subjects.insert(s.subject.as_str());
                }
                let values: Vec<(String, f64)> = provinces
                    .keys()
                    .filter(|x| *x != c && **x != rebel)
                    .map(|x| {
                        let xs = state.countries.get(x);
                        let i = war_ai::AntagonizeInput {
                            x_wanted_value: {
                                // C's conquer_prov summed over X's provinces; the stand-in per province when the save has no list.
                                let wanted = &cs.ai_lists.conquer_prov;
                                if wanted.is_empty() { r.province_value * get(&provinces, x) } else { owned_by.get(x).map_or(0.0, |ps| ps.iter().filter_map(|p| wanted.get(p)).sum()) }
                            },
                            x_provinces: get(&provinces, x),
                            x_regiments: get(&regiments, x),
                            own_provinces: get(&provinces, c),
                            own_regiments: get(&regiments, c),
                            neighbour: neighbours.contains(&(c.clone(), x.clone())),
                            x_infamy: xs.map_or(0.0, |s| s.badboy),
                            holds_cb: self.held_cbs.contains_key(&(c.clone(), x.clone())),
                            civ_vs_unciv: civ && !xs.is_some_and(|s| s.civilized),
                            in_own_sphere: leaders.get(x.as_str()).copied() == Some(c.as_str()),
                            x_is_subject: subjects.contains(x.as_str()),
                        };
                        (x.clone(), war_ai::antagonize_value(&r, &i))
                    })
                    .collect();
                let list = war_ai::antagonize_list(&r, values);
                let mut scored: Vec<(String, f64)> = Vec::new();
                for (t, antagonize) in &list {
                    let ts = state.countries.get(t);
                    let allies_of = |x: &str, other: &str| -> Vec<(f64, bool)> { dip.allies_of(x).filter(|y| *y != other).map(|y| (get(&strength, y), neighbours.contains(&(y.to_string(), other.to_string())))).collect() };
                    let mut target_side = war_ai::side_strength(&r, get(&strength, t), &allies_of(t, c));
                    // The target's sphere leader stands with it (2.1 A 10), unless it is the target's ally (counted).
                    if let Some(l) = leaders.get(t.as_str()).copied().filter(|l| *l != c.as_str() && !dip.allied(l, t)) {
                        target_side += get(&strength, l);
                    }
                    // NOT WIRED: half the strength of the target's other enemies, friendly great powers.
                    let holds_cb = self.held_cbs.contains_key(&(c.clone(), t.to_string()));
                    let held: Vec<String> = self.held_cbs.get(&(c.clone(), t.to_string())).map(|v| v.iter().map(|h| h.cb_type.clone()).collect()).unwrap_or_default();
                    // NOT WIRED: the types' can_use triggers (no casus belli condition is evaluated).
                    let options: Vec<war_ai::GoalOption> = cbs.iter().filter(|cb| cb.constructing || held.contains(&cb.name)).map(|cb| war_ai::GoalOption { cb, held: held.contains(&cb.name), cost: 0.0, present: false }).collect();
                    let ctx = self.goal_context(state, Some(&state_counts), c, t, wr.badboy_limit - r.target_goal_margin, own_infamy > r.held_only_infamy_target || holds_cb, false, 0.0);
                    let best = war_ai::pick_goal(&r, &wr, &options, &ctx, None);
                    let unjust = best.filter(|(k, _)| !options[*k].held).map(|(k, _)| (war::goal_infamy(&wr, options[k].cb, false), own_infamy));
                    let i = war_ai::TargetInput {
                        own_war_exhaustion: cs.war_exhaustion,
                        truce: dip.truce_active(c, t, today),
                        t_subject_or_own_sphere: subjects.contains(t.as_str()) || leaders.get(t.as_str()).copied() == Some(c.as_str()),
                        neighbour: neighbours.contains(&(c.clone(), t.to_string())),
                        own_regiments: get(&regiments, c),
                        own_side: war_ai::side_strength(&r, get(&strength, c), &allies_of(c, t)),
                        target_side,
                        t_over_limit: ts.is_some_and(|s| s.badboy > wr.badboy_limit),
                        antagonize: *antagonize,
                        rival: cs.ai_lists.rival.get(t).copied().unwrap_or(0.0),
                        threat: cs.ai_lists.threat.get(t).copied().unwrap_or(0.0),
                        befriend: cs.ai_lists.befriend.get(t).copied().unwrap_or(0.0),
                        own_rank: rank_of(c),
                        t_rank: rank_of(t),
                        relation: dip.relation_value(c, t),
                        t_infamy: ts.map_or(0.0, |s| s.badboy),
                        holds_cb,
                        civ_vs_unciv: civ && !ts.is_some_and(|s| s.civilized),
                        goal_value: best.map_or(0.0, |b| b.1),
                        unjust_goal_infamy: unjust,
                    };
                    scored.push((t.to_string(), war_ai::target_score(&r, &wr, &i)));
                }
                let roll = self.roll01();
                if let Some(t) = war_ai::draw_target(&r, &scored, roll) {
                    self.war_targets.insert(c.clone(), t);
                }
            }
            // B. Work on the target.
            let Some(t) = self.war_targets.get(c).cloned() else { continue };
            if at_war.contains(&t) && Sides::from_wars(&self.mil.wars, &rebel).hostile(c, &t) {
                continue;
            }
            let key = (c.clone(), t.clone());
            let held: Vec<String> = self.held_cbs.get(&key).map(|v| v.iter().map(|h| h.cb_type.clone()).collect()).unwrap_or_default();
            let civ_vs_unciv = civ && !state.countries.get(&t).is_some_and(|s| s.civilized);
            let ceiling = wr.badboy_limit - r.justify_goal_margin;
            let spread: Vec<f64> = (0..cbs.len()).map(|_| self.roll01()).collect();
            let justify_options: Vec<war_ai::GoalOption> = cbs.iter().filter(|cb| cb.constructing).map(|cb| war_ai::GoalOption { cb, held: held.contains(&cb.name), cost: 0.0, present: false }).collect();
            let ctx = self.goal_context(state, Some(&state_counts), c, &t, ceiling, own_infamy > r.held_only_infamy_justify, false, 0.0);
            let justify_goal = war_ai::pick_goal(&r, &wr, &justify_options, &ctx, Some(&spread)).map(|(k, v)| (justify_options[k].cb.name.clone(), v));
            let held_options: Vec<war_ai::GoalOption> = cbs.iter().filter(|cb| held.contains(&cb.name)).map(|cb| war_ai::GoalOption { cb, held: true, cost: 0.0, present: false }).collect();
            let ctx = self.goal_context(state, Some(&state_counts), c, &t, ceiling, true, false, 0.0);
            let declare_goal = war_ai::pick_goal(&r, &wr, &held_options, &ctx, None).map(|(k, _)| held_options[k].cb.name.clone());
            let truce = dip.truce_active(c, &t, today);
            let mv = war_ai::MoveInput {
                relation: dip.relation_value(c, &t),
                truce,
                has_cb: !held.is_empty(),
                justifying: self.justifying.contains_key(&key),
                roll: (self.roll() % 1000) as f64,
                justify_chance: war_ai::justify_chance(&r, &wr, own_infamy, civ_vs_unciv),
                justify_goal,
                declare_goal,
            };
            match war_ai::target_move(&r, &mv) {
                war_ai::TargetMove::Justify(kind) => {
                    if !truce && !self.justifying.keys().any(|(a, _)| a == c) {
                        self.justifying.insert(key, (kind, 0.0, false));
                        log.cbs_justified += 1;
                    }
                }
                war_ai::TargetMove::DecreaseRelation => dip.change_relation(&rules, c, &t, r.relation_decrease),
                war_ai::TargetMove::Declare(kind) => {
                    if self.start_war(state, &rules, &mut dip, &r, c, &t, &kind, truce, &mut at_war, log).is_ok() {
                        self.war_targets.remove(c);
                    }
                }
                war_ai::TargetMove::Nothing => {}
            }
        }
        self.dip = Some((rules, dip));
        state.wars = self.mil.wars.iter().filter(|w| w.active).map(|w| (w.attackers.iter().cloned().collect(), w.defenders.iter().cloned().collect())).collect();
    }
}

type CbTypeRef = war::CbType;

mod play;
mod factions;
mod movements;
mod recruit;

/// Facts of one war for the peace answers.
struct PeaceCtx<'a> {
    war: &'a War,
    months: f64,
    /// The attackers' war score and the war's direction as they see it.
    score: f64,
    direction: f64,
}

/// The economy and pop systems with their rules (what `run-events` used to build itself).
pub struct Economy {
    pub cal: Calendar,
    pub effect_rules: EffectRules,
    pub event_rules: EventRules,
    pub runner: EventRunner,
    pub log: EventLog,
    pub schedule: PopSchedule,
    pub growth: GrowthRules,
    pub mobility: MobilityDefs,
    pub mobility_rules: MobilityRules,
    pub literacy_rules: LiteracyRules,
    pub ideology_rules: pops::IdeologyRules,
    pub mood_rules: MoodRules,
    pub country_rules: CountryRules,
    pub invention_rules: InventionRules,
    pub research_rules: crate::research::ResearchRules,
    pub politics_rules: crate::politics::PoliticsRules,
    pub reform_rules: crate::reforms::ReformRules,
    pub rank_rules: crate::rank::RankRules,
    pub industry_rules: crate::industry::IndustryRules,
    pub farms: crate::economy::Farms,
    pub focus_ai: crate::ai::FocusAi,
    pub targets: TargetCache,
    /// Leave pops' ideologies and issues as they are (GAMEMIX_NO_DRIFT).
    pub no_drift: bool,
    pub event_titles: BTreeMap<u64, String>,
    pub random_events: usize,
}

/// What one combined day did, for the callers to add up.
#[derive(Debug, Clone, Default)]
pub struct DayLog {
    /// What came of the players' orders carried out at the start of the day.
    pub orders: Vec<crate::player::Outcome>,
    pub researched: Vec<(String, String)>,
    pub elections: Vec<(String, Option<u32>, u32)>,
    pub reforms: Vec<(String, String, String)>,
    pub money: crate::economy::FarmMoney,
    pub built: crate::industry::IndustryMonth,
    pub discovered: Vec<(String, u32)>,
    pub flows: BTreeMap<(String, String), f64>,
    pub assimilated: f64,
    pub moved_within: f64,
    pub emigrated: BTreeMap<(String, String), f64>,
    pub core: CoreLog,
}

pub struct Game {
    pub world: World,
    pub state: State,
    pub econ: Economy,
    pub core: Core,
    /// The core (military and cross effects) runs; `run-events` switches it off to keep its old meaning.
    pub core_on: bool,
    /// The players' orders waiting for the start of the next day (player.rs).
    pub players: crate::player::Players,
    /// Wall time per system, summed over days, when `Some` (the run CLI's --profile).
    pub profile: Option<BTreeMap<&'static str, std::time::Duration>>,
}

fn read_doc(path: &Path) -> Result<script::Document, String> {
    script::parse_bytes(&std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?).map_err(|e| format!("{}: {e}", path.display()))
}

impl Game {
    /// Everything from the world, the rules file, the install and a save.
    pub fn new(world: World, engine_rules: &EngineRules, install: &Path, save: &SaveGame) -> Result<Game, String> {
        let w = &world;
        let (mut events, mut decisions) = (Vec::new(), Vec::new());
        for folder in ["events", "decisions"] {
            let dir = install.join(folder);
            let mut files: Vec<_> = std::fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            files.sort();
            for path in files {
                let Ok(doc) = read_doc(&path) else { continue };
                if folder == "events" {
                    events.extend(parse_events(&doc.root));
                } else {
                    decisions.extend(parse_decisions(&doc.root));
                }
            }
        }
        let event_titles: BTreeMap<u64, String> = events.iter().map(|e| (e.id, e.title.clone())).collect();
        let random_events = events.iter().filter(|e| !e.triggered_only && e.mean_time.is_some()).count();
        let (cal, effect_rules, event_rules) = (Calendar::from_engine_rules(engine_rules)?, EffectRules::from_engine_rules(engine_rules)?, EventRules::from_engine_rules(engine_rules)?);
        let mut state = State::from_save(save);
        state.attach_world(w, save);
        pops::attach_admin_rules(&mut state, w);
        state.seed = effect_rules.seed;
        let rank_rules = crate::rank::RankRules::new(install, engine_rules, w)?;
        let read = |field: &str| -> Result<script::Document, String> { read_doc(&install.join(engine_rules.require_str(field)?)) };
        let (rules_doc, focus_doc, ideology_doc, issues_doc) = (read("FLD_POP_RULES_FILE")?, read("FLD_NATIONAL_FOCUS_FILE")?, read("FLD_IDEOLOGIES_FILE")?, read("FLD_ISSUES_FILE")?);
        let mut mobility = MobilityDefs::default().with_ideology_file(&ideology_doc.root).with_chances(&rules_doc.root).with_foci(&focus_doc.root, &w.defs.pop_types.iter().map(|t| t.name.clone()).collect());
        let mut country_rules = CountryRules::new(engine_rules, &w.defs.modifiers)?.with_technologies(&w.defs.technologies);
        for t in &w.defs.pop_types {
            let path = install.join(engine_rules.require_str("FLD_POPTYPES_DIR")?).join(format!("{}.txt", t.name));
            if let Ok(doc) = read_doc(&path) {
                mobility.add_pop_type(&t.name, &doc.root);
                country_rules = country_rules.with_pop_type(&t.name, &doc.root);
            }
        }
        let industry_rules = crate::industry::IndustryRules::new(install, engine_rules, w)?;
        state.party_pops_build = industry_rules.party_pops_build();
        let politics_rules = crate::politics::PoliticsRules::new(w, &read("FLD_GOVERNMENTS_FILE")?.root, &issues_doc.root)?;
        state.election_months = politics_rules.governments.iter().filter(|(_, g)| g.election).map(|(name, g)| (name.clone(), g.duration)).collect();
        let reform_rules = crate::reforms::ReformRules::new(w, engine_rules, &issues_doc.root, &ideology_doc.root)?;
        let goods_doc = read("FLD_GOODS_FILE")?;
        let fixed_flag = engine_rules.require_str("FLD_PRICE_FIXED_GOOD_FLAG")?.to_string();
        let farms = crate::economy::Farms::new(engine_rules, w, &state)?.with_goods(wiktoria_data::goods::load_goods(&goods_doc)?.goods.into_iter().map(|g| {
            let fixed = if fixed_flag == "money" { g.money } else { !g.tradeable };
            (g.id, g.cost, fixed)
        }));
        state.money = crate::economy::MoneyState::from_save(&state, save);
        state.economy_numbers = crate::economy::EconomyNumbers::from_engine_rules(engine_rules)?;
        let focus_ai = crate::ai::FocusAi::new(engine_rules, w, &focus_doc.root, &state)?;
        let econ = Economy {
            runner: EventRunner::new(events, decisions),
            log: EventLog::default(),
            schedule: PopSchedule::from_engine_rules(engine_rules)?,
            growth: GrowthRules::new(&w.defines, engine_rules)?,
            mobility,
            mobility_rules: MobilityRules::new(&w.defines, engine_rules)?,
            literacy_rules: LiteracyRules::new(&w.defines, engine_rules)?,
            ideology_rules: pops::IdeologyRules::new(engine_rules)?,
            mood_rules: MoodRules::new(&w.defines, engine_rules)?.with_reforms(&w.defs.issues),
            country_rules,
            invention_rules: InventionRules::new(engine_rules, &w.defs.invention_blocks)?,
            research_rules: crate::research::ResearchRules::new(engine_rules, w)?,
            politics_rules,
            reform_rules,
            rank_rules,
            industry_rules,
            farms,
            focus_ai,
            targets: TargetCache::default(),
            no_drift: false,
            event_titles,
            random_events,
            cal,
            effect_rules,
            event_rules,
        };
        // The core: military, AI, diplomacy, unrest, colonies, buildings. One seed: the game's.
        let mil_rules = MilitaryRules::new(install, engine_rules)?;
        let mil = MilitaryState::from_save(save, &mil_rules);
        let seed = state.seed;
        let day = MilitaryDay::from_world(w, engine_rules, install, &mil_rules, splitmix(&mut seed.clone()))?;
        let mut core = Core::new(mil, day, AiRules::new(engine_rules, &w.defines)?, GameNumbers::from_engine_rules(engine_rules)?, seed);
        core.dip = Some((DipRules::new(engine_rules, &w.defines)?, DiplomacyState::from_save(save)));
        core.crisis = Some((CrisisRules::new(engine_rules, &w.defines)?, CrisisState::from_save(save)));
        core.war_ai = Some(WarAiRules::from_engine_rules(engine_rules)?);
        core.base_diplopoints = w.defines.get("diplomacy", "BASE_MONTHLY_DIPLOPOINTS").or_else(|| w.defines.get("country", "BASE_MONTHLY_DIPLOPOINTS")).unwrap_or(0.0);
        let rebel_file = install.join(engine_rules.require_str("FLD_GAME_REBEL_TYPES_FILE")?);
        let types = unrest::RebelType::parse_file(&std::fs::read(&rebel_file).map_err(|e| format!("cannot read {}: {e}", rebel_file.display()))?)?;
        core.factions = Some(factions::FactionRules::new(types, &w.defs.cultures));
        core.movement_rules = Some(movements::MovementRules::new(&state, &w.defs.issues, &read_doc(&install.join(engine_rules.require_str("FLD_ISSUES_FILE")?))?.root, &w.defs.cultures));
        core.unrest = Some((UnrestRules::new(engine_rules, &w.defines)?, UnrestState::from_save(save, &w.defs.cultures, &w.defs.issues)));
        let colony_rules = ColonyRules::from_defines(&w.defines, engine_rules)?.with_technologies(w.defs.technologies.iter().filter_map(|t| Some((t.name.as_str(), *t.values.get("colonial_points")?))));
        core.attach_colonies(colony_rules, ColonyState::from_save(save));
        let buildings_file = install.join(engine_rules.require_str("FLD_BUILDINGS_FILE")?);
        core.buildings = Some(BuildingRules::new(&String::from_utf8_lossy(&std::fs::read(&buildings_file).map_err(|e| format!("cannot read {}: {e}", buildings_file.display()))?), &w.defines, engine_rules)?);
        core.unlocks = Some(recruit::unlocks(install, engine_rules)?);
        core.tech_buildings = econ.industry_rules.by_technology.clone();
        core.invention_buildings = w.defs.modifiers.inventions.iter().map(|(name, _)| econ.industry_rules.by_invention.get(name).cloned().unwrap_or_default()).collect();
        for (id, p) in &core.mil.provinces {
            if let Some(q) = state.provinces.get_mut(id) {
                q.controller_since = p.last_controller_change;
            }
        }
        let mut game = Game { world, state, econ, core, core_on: true, players: crate::player::Players::default(), profile: None };
        game.refresh_mobilisation_mods();
        Ok(game)
    }

    /// Mobilisation size and impact of every country from its modifiers.
    fn refresh_mobilisation_mods(&mut self) {
        let lib = &self.world.defs.modifiers;
        self.core.mobilisation_mods = self.state.countries.iter().map(|(t, c)| (t.clone(), (pops::country_modifier(c, lib, "mobilisation_size"), pops::country_modifier(c, lib, "mobilisation_impact")))).collect();
        // NOT WIRED: mobilisation size and impact from technologies (country_modifier reads inventions,
        // reforms, national value and event modifiers).
    }

    /// Carries out the players' waiting orders now (player.rs). `day` does this first; a caller
    /// may do it between two days, which is the same.
    pub fn apply_orders(&mut self) -> Vec<crate::player::Outcome> {
        let e = &mut self.econ;
        let events = crate::player::EventSide { runner: &mut e.runner, cal: &e.cal, rules: &e.effect_rules, log: &mut e.log };
        crate::player::apply_all(&mut self.players, &mut self.state, &mut self.core, &e.research_rules, &e.politics_rules, Some(events))
    }

    /// Gives a country to a person or back to the computer (player.rs, D20).
    pub fn set_player(&mut self, tag: &str, on: bool) -> Result<(), String> {
        crate::player::set_player(&mut self.state, &mut self.core, tag, on)
    }

    /// One day. See the module notes for the order.
    pub fn day(&mut self) -> DayLog {
        let orders = self.apply_orders();
        let (w, e, state) = (&self.world, &mut self.econ, &mut self.state);
        let lib = &w.defs.modifiers;
        let mut prof = self.profile.take();
        let mut t = std::time::Instant::now();
        // Adds the time since the last mark to `name` when profiling.
        let mut mark = |name: &'static str| {
            if let Some(p) = prof.as_mut() {
                let now = std::time::Instant::now();
                *p.entry(name).or_default() += now - t;
                t = now;
            }
        };
        let mut log = DayLog { orders, researched: crate::research::research_day(state, &e.research_rules, lib), ..DayLog::default() };
        mark("research");
        log.elections = crate::politics::politics_day(state, &e.politics_rules).elections;
        mark("politics");
        log.reforms = crate::reforms::reform_day(state, &e.reform_rules);
        mark("reforms");
        if self.core_on {
            log.core = self.core.day(state);
            // The demands of factions that won: their type's effect on the country.
            for win in std::mem::take(&mut self.core.rebel_wins) {
                if let Some(b) = &win.effect {
                    let mut rep = crate::script::Report::default();
                    crate::script::run_effects(state, &e.cal, &e.effect_rules, b, &crate::script::Scope::Country(win.country.clone()), &mut rep);
                }
            }
        }
        mark("core (military etc)");
        log.money = e.farms.market_day(w, state);
        mark("market");
        crate::rank::rank_day(state, &e.rank_rules, lib);
        mark("rank");
        e.runner.day(state, &e.cal, &e.effect_rules, &e.event_rules, &mut e.log);
        mark("events");
        pops::grow_due_pops(state, &e.growth, &e.schedule, &e.cal, lib);
        mark("pops grow");
        pops::teach_due_pops(state, &e.literacy_rules, &e.schedule, &e.cal, lib);
        mark("pops teach");
        pops::mood_due_pops(state, &e.mood_rules, &e.schedule, &e.cal, lib);
        mark("pops mood");
        if !e.no_drift {
            pops::drift_due_pops(state, &e.mobility, &e.ideology_rules, &e.schedule, &e.cal);
            mark("pops drift");
            pops::issues_due_pops(state, &e.mobility, &e.schedule, &e.cal, lib);
            mark("pops issues");
        }
        if state.date.day == 1 {
            e.focus_ai.month(state);
        }
        mark("focus ai");
        log.built = crate::industry::industry_day(state, w, &e.industry_rules, &log.money);
        mark("industry");
        if state.date.day == 1 {
            let ended = if state.date.month == 1 { 12 } else { state.date.month - 1 };
            pops::country_month(state, &e.country_rules, lib, e.cal.month_len(ended) as f64);
            log.discovered = pops::discover_inventions(state, &e.invention_rules);
        }
        mark("country month");
        log.flows = pops::move_due_pops(state, &e.mobility, &e.mobility_rules, &e.schedule, &e.cal);
        mark("pops move");
        log.assimilated = pops::assimilate_due_pops(state, &e.mobility, &e.mobility_rules, &e.schedule, &e.cal);
        mark("pops assimilate");
        let (within, abroad) = pops::migrate_due_pops(state, &e.mobility, &e.mobility_rules, &e.schedule, &e.cal, &mut e.targets, lib);
        (log.moved_within, log.emigrated) = (within, abroad);
        mark("pops migrate");
        pops::drop_empty_pops(state);
        if self.core_on && self.state.date.day == 1 {
            self.refresh_mobilisation_mods();
        }
        mark("cleanup");
        self.profile = prof;
        log
    }
}
