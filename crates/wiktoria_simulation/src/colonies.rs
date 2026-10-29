//! Colonisation as pure rules.
//!
//! Every value comes from the install's defines at runtime (`ColonyRules::from_defines`). The
//! functions take what they need and return what changed, nothing here owns the world. Hypotheses
//! (marked "hypothesis") stand in where the rule description has no answer: what a first step sets the level
//! to, and the AI's choice of region.

use std::collections::BTreeMap;

use wiktoria_data::defines::Defines;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::save::SaveGame;
use wiktoria_data::script::Value;

use crate::military::{num, province_blocks, text};

/// What the install's defines say (`country` section unless noted).
#[derive(Debug, Clone, PartialEq)]
pub struct ColonyRules {
    /// Rules file (`FLD_COL_*`, hypotheses): level a newcomer's first step sets, and the contender slots of a region.
    pub first_step_level: f64,
    pub max_contenders: usize,
    /// Rules file (`FLD_COL_STATE_DISTANCE_PIXELS`): map pixels that make one unit of COLONIZATION_COLONY_STATE_DISTANCE.
    pub state_distance_pixels: f64,
    /// Technology name -> its `colonial_points` (the install's technology files; the caller fills it, `with_technologies`).
    pub tech_points: BTreeMap<String, f64>,
    pub colonial_rank: f64,
    pub life_rating: f64,
    pub supply_base: f64,
    pub supply_empty: f64,
    pub supply_non_core: f64,
    /// COLONIAL_POINTS_FOR_NON_CORE_BASE: colonial points of a coastal state overseas that is no core.
    pub non_core_base_points: f64,
    pub points_from_supply: f64,
    pub interest_lead: f64,
    pub influence_lead: f64,
    pub months_to_colonize: f64,
    pub days_between_investment: i64,
    pub days_for_initial_investment: i64,
    pub protectorate_upkeep: f64,
    pub colony_upkeep: f64,
    pub colony_industry_upkeep: f64,
    pub colony_railway_upkeep: f64,
    pub interest_cost_initial: f64,
    pub interest_cost_neighbor: f64,
    pub interest_cost: f64,
    pub influence_cost: f64,
    pub extra_guard_cost: f64,
    pub state_cost: f64,
    pub protectorate_cost: f64,
    pub colony_cost: f64,
    pub state_distance: f64,
    pub temp_per_day: f64,
    pub temp_per_level: f64,
    pub colony_formed_prestige: f64,
    pub colony_to_state_prestige: f64,
    /// `diplomacy` section.
    pub tension_while_crisis: f64,
    pub at_war_tension_decay: f64,
    pub peace_cost_colony: f64,
    pub infamy_colony: f64,
}

impl ColonyRules {
    pub fn from_defines(d: &Defines, m: &EngineRules) -> Result<ColonyRules, String> {
        let c = |k: &str| d.get("country", k).ok_or(format!("defines.country.{k} missing"));
        let p = |k: &str| d.get("diplomacy", k).ok_or(format!("defines.diplomacy.{k} missing"));
        Ok(ColonyRules {
            first_step_level: crate::num_field(m, "FLD_COL_FIRST_STEP_LEVEL")?,
            max_contenders: crate::int_field(m, "FLD_COL_MAX_CONTENDERS")? as usize,
            state_distance_pixels: crate::num_field(m, "FLD_COL_STATE_DISTANCE_PIXELS")?,
            tech_points: BTreeMap::new(),
            colonial_rank: c("COLONIAL_RANK")?,
            life_rating: c("COLONIAL_LIFERATING")?,
            supply_base: c("NAVAL_BASE_SUPPLY_SCORE_BASE")?,
            supply_empty: c("NAVAL_BASE_SUPPLY_SCORE_EMPTY")?,
            supply_non_core: c("NAVAL_BASE_NON_CORE_SUPPLY_SCORE")?,
            non_core_base_points: c("COLONIAL_POINTS_FOR_NON_CORE_BASE")?,
            points_from_supply: c("COLONIAL_POINTS_FROM_SUPPLY_FACTOR")?,
            interest_lead: c("COLONIZATION_INTEREST_LEAD")?,
            influence_lead: c("COLONIZATION_INFLUENCE_LEAD")?,
            months_to_colonize: c("COLONIZATION_MONTHS_TO_COLONIZE")?,
            days_between_investment: c("COLONIZATION_DAYS_BETWEEN_INVESTMENT")? as i64,
            days_for_initial_investment: c("COLONIZATION_DAYS_FOR_INITIAL_INVESTMENT")? as i64,
            protectorate_upkeep: c("COLONIZATION_PROTECTORATE_PROVINCE_MAINTAINANCE")?,
            colony_upkeep: c("COLONIZATION_COLONY_PROVINCE_MAINTAINANCE")?,
            colony_industry_upkeep: c("COLONIZATION_COLONY_INDUSTRY_MAINTAINANCE")?,
            colony_railway_upkeep: c("COLONIZATION_COLONY_RAILWAY_MAINTAINANCE")?,
            interest_cost_initial: c("COLONIZATION_INTEREST_COST_INITIAL")?,
            interest_cost_neighbor: c("COLONIZATION_INTEREST_COST_NEIGHBOR_MODIFIER")?,
            interest_cost: c("COLONIZATION_INTEREST_COST")?,
            influence_cost: c("COLONIZATION_INFLUENCE_COST")?,
            extra_guard_cost: c("COLONIZATION_EXTRA_GUARD_COST")?,
            state_cost: c("COLONIZATION_CREATE_STATE_COST")?,
            protectorate_cost: c("COLONIZATION_CREATE_PROTECTORATE_COST")?,
            colony_cost: c("COLONIZATION_CREATE_COLONY_COST")?,
            state_distance: c("COLONIZATION_COLONY_STATE_DISTANCE")?,
            temp_per_day: c("COLONIZATION_INFLUENCE_TEMPERATURE_PER_DAY")?,
            temp_per_level: c("COLONIZATION_INFLUENCE_TEMPERATURE_PER_LEVEL")?,
            colony_formed_prestige: c("COLONY_FORMED_PRESTIGE")?,
            colony_to_state_prestige: c("COLONY_TO_STATE_PRESTIGE_GAIN")?,
            tension_while_crisis: p("TENSION_WHILE_CRISIS")?,
            at_war_tension_decay: p("AT_WAR_TENSION_DECAY")?,
            peace_cost_colony: p("PEACE_COST_COLONY")?,
            infamy_colony: p("INFAMY_COLONY")?,
        })
    }

    /// Adds the colonial points of the technologies that give any, as (name, points).
    pub fn with_technologies<'a>(mut self, techs: impl Iterator<Item = (&'a str, f64)>) -> ColonyRules {
        self.tech_points = techs.filter(|(_, x)| *x != 0.0).map(|(n, x)| (n.to_string(), x)).collect();
        self
    }
}

/// The colonial status of a province or state, as the save writes it (`colonial=` / `is_colonial=`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    State,
    Protectorate,
    Colony,
}

impl Status {
    pub fn from_save(v: f64) -> Status {
        match v as i64 {
            1 => Status::Protectorate,
            2 => Status::Colony,
            _ => Status::State,
        }
    }

    pub fn to_save(self) -> u8 {
        match self {
            Status::State => 0,
            Status::Protectorate => 1,
            Status::Colony => 2,
        }
    }

    /// Only full states allow factories (owner's answer; the rule description left it open).
    pub fn allows_factories(self) -> bool {
        self == Status::State
    }
}

// ---- colonial points -------------------------------------------------------------------

/// Naval supply score of one naval base: base x 2^(level-1), level 0 gives the empty score; a base
/// outside the country's cores on another continent than the capital is scaled, at least 1.
/// This is naval supply, not colonial points (`naval_base_points`).
pub fn naval_base_score(r: &ColonyRules, level: u32, core: bool, same_continent_as_capital: bool) -> f64 {
    let s = if level == 0 { r.supply_empty } else { r.supply_base * 2f64.powi(level as i32 - 1) };
    if !core && !same_continent_as_capital {
        (s * r.supply_non_core).max(1.0)
    } else {
        s
    }
}

/// Colonial points of one naval base in a core province or on the capital's continent: the
/// building's `colonial_points` entry for its level (the list starts at level 1; a level past the
/// list takes the last entry). Read off the original on 1841.7.5: seven core bases give 290.
///
/// Overseas without a core it is not the base that counts: every coastal state there gives
/// COLONIAL_POINTS_FOR_NON_CORE_BASE, base or not (hypothesis from three readings: nine such states,
/// one with a level 1 base, gave 9; a tenth, without a base, made it 10. A base at 30 x 0.3 = 9 and
/// nothing for the others fits the first reading only).
pub fn naval_base_points(per_level: &[f64], level: u32) -> f64 {
    (level as usize).checked_sub(1).and_then(|i| per_level.get(i).or(per_level.last())).copied().unwrap_or(0.0)
}

/// How far naval supply used exceeds the limit, as a fraction of the limit, 0..1.
pub fn overuse(used: f64, limit: f64) -> f64 {
    if limit <= 0.0 || used <= limit {
        0.0
    } else {
        ((used - limit) / limit).min(1.0)
    }
}

pub struct PointsInput {
    /// Overall rank; above `colonial_rank` the country has no points at all.
    pub rank: f64,
    /// Colonial points from technologies.
    pub tech_points: f64,
    /// Colonial points of its naval bases (`naval_base_points`, summed) and of its coastal states
    /// overseas that are no cores (COLONIAL_POINTS_FOR_NON_CORE_BASE each).
    pub base_points: f64,
    /// Colonial points of its ships (each unit type's `colonial_points`), before the supply cut.
    pub ship_points: f64,
    /// `overuse` of its naval supply: the ships' points shrink by this share.
    pub supply_overuse: f64,
    /// Stored bonus (events).
    pub bonus: f64,
}

/// Technologies + COLONIAL_POINTS_FROM_SUPPLY_FACTOR x (naval bases + ships x (1 - overuse)) + bonus.
/// Read off the original: 100 + 299 + 120 = 519 with the navy inside its supply; a year later, with
/// one more colony, 520, and 400 with the navy at more than twice its supply (the ships' 120 gone).
pub fn total_points(r: &ColonyRules, i: &PointsInput) -> f64 {
    if i.rank > r.colonial_rank {
        return 0.0;
    }
    i.tech_points + r.points_from_supply * (i.base_points + i.ship_points * (1.0 - i.supply_overuse)) + i.bonus
}

/// A country's colonial points with the parts they are made of (`Core::colonial_points_of`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Points {
    /// (technology, points).
    pub techs: Vec<(String, f64)>,
    /// (province, level, points) of every naval base in a core province or on the capital's continent.
    pub bases: Vec<(u16, u32, f64)>,
    /// (region, points) of every coastal state overseas that is no core.
    pub overseas: Vec<(Option<u16>, f64)>,
    /// (unit type, ships, points of one).
    pub ships: Vec<(String, f64, f64)>,
    pub supply_used: f64,
    pub supply_limit: f64,
    /// Naval supply by state: (region, best base level, non-core overseas, supply it gives).
    pub supply_states: Vec<(Option<u16>, u32, bool, f64)>,
    pub overuse: f64,
    pub total: f64,
    /// Points standing in open races.
    pub races: f64,
    /// Upkeep of protectorates and colonies, not rounded.
    pub upkeep: f64,
    pub used: f64,
}

impl Points {
    pub fn free(&self) -> f64 {
        free_points(self.total, self.used)
    }
}

/// One state with colonial status, for the upkeep.
#[derive(Debug, Clone, PartialEq)]
pub struct Holding {
    pub status: Status,
    pub provinces: usize,
    pub factories: usize,
    pub railway_levels: f64,
}

/// Points in use: points held in the country's open races plus the upkeep of every holding, rounded.
pub fn points_used(r: &ColonyRules, rank: f64, race_points: f64, holdings: &[Holding]) -> f64 {
    if rank > r.colonial_rank {
        return 0.0;
    }
    let upkeep: f64 = holdings.iter().map(|h| upkeep_of(r, h)).sum();
    (race_points + upkeep).round()
}

/// Colonial upkeep of one holding. A full state costs nothing.
pub fn upkeep_of(r: &ColonyRules, h: &Holding) -> f64 {
    match h.status {
        Status::State => 0.0,
        Status::Protectorate => h.provinces as f64 * r.protectorate_upkeep,
        Status::Colony => h.provinces as f64 * r.colony_upkeep + h.factories as f64 * r.colony_industry_upkeep + h.railway_levels * r.colony_railway_upkeep,
    }
}

pub fn free_points(total: f64, used: f64) -> f64 {
    total - used
}

// ---- who may step in -------------------------------------------------------------------

/// Why a country may not step into a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    NotGreatOrSecondary,
    NotColonisable,
    LifeRating,
    CrisisRegion,
    AtWar,
    AlreadyIn,
    NotInRace,
    TooSoon,
    OutOfReach,
    RaceClosed,
    NotEnoughPoints,
    WrongStatus,
    ConditionsNotMet,
}

pub struct Candidate {
    pub rank: f64,
    pub great_power: bool,
    /// Highest life rating among the region's still unowned provinces (one good province opens the whole region).
    pub best_life_rating: f64,
    /// The country's life-rating limit after technologies and inventions (base `life_rating`, lowered by them).
    pub life_rating_limit: f64,
    pub at_great_war: bool,
    pub region_is_crisis: bool,
    /// Reach: land contact, a subject's land or a naval base in range.
    pub in_reach: bool,
    pub borders_region: bool,
}

/// The colonisation test plus the race rules. `today` in game days.
pub fn may_step(r: &ColonyRules, c: &Candidate, race: &Race, tag: &str, today: i64) -> Result<(), Refusal> {
    if c.rank > r.colonial_rank && !c.great_power {
        return Err(Refusal::NotGreatOrSecondary);
    }
    if c.best_life_rating < 0.0 {
        return Err(Refusal::NotColonisable);
    }
    // Life rating: refused when the best province is below the limit.
    if c.best_life_rating < c.life_rating_limit {
        return Err(Refusal::LifeRating);
    }
    if c.region_is_crisis {
        return Err(Refusal::CrisisRegion);
    }
    if c.at_great_war {
        return Err(Refusal::AtWar);
    }
    let inside = race.contender(tag);
    match race.stage {
        Stage::Open => {
            if inside.is_some() {
                return Err(Refusal::AlreadyIn);
            }
        }
        Stage::Interest => {}
        Stage::Influence => {
            if inside.is_none() {
                return Err(Refusal::NotInRace);
            }
        }
        Stage::Won => return Err(Refusal::RaceClosed),
    }
    if inside.is_some_and(|i| today - i.last_step < r.days_between_investment) {
        return Err(Refusal::TooSoon);
    }
    if !c.in_reach {
        return Err(Refusal::OutOfReach);
    }
    Ok(())
}

/// Colonial-point cost of one step.
pub fn step_cost(r: &ColonyRules, stage: Stage, already_in: bool, borders_region: bool, level: f64) -> f64 {
    match stage {
        Stage::Open | Stage::Interest => {
            if already_in {
                r.interest_cost
            } else {
                r.interest_cost_initial + if borders_region { r.interest_cost_neighbor } else { 0.0 }
            }
        }
        Stage::Influence => r.influence_cost + r.extra_guard_cost * (level - 4.0).max(0.0),
        Stage::Won => 0.0,
    }
}

// ---- the race --------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stage {
    /// Stage 0: nobody or one contender.
    #[default]
    Open,
    /// Stage 1: two or more contenders.
    Interest,
    /// Stage 2: the top two, temperature runs.
    Influence,
    /// Stage 3: one winner who may create the protectorate.
    Won,
}

impl Stage {
    pub fn from_save(v: i64) -> Stage {
        match v {
            1 => Stage::Interest,
            2 => Stage::Influence,
            3 => Stage::Won,
            _ => Stage::Open,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Contender {
    pub tag: String,
    pub level: f64,
    pub points: f64,
    pub last_step: i64,
}

/// The record of one colonial region (at most four contenders).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Race {
    pub stage: Stage,
    /// Day the race started (stage 0) or was decided (stage 3).
    pub date: i64,
    pub temperature: f64,
    pub contenders: Vec<Contender>,
}

/// What drives the temperature besides the race itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct Climate {
    pub crisis_running: bool,
    pub is_crisis_region: bool,
    /// Share (0..1) of the great powers of the continents concerned that are at war / disarmed.
    pub war_share: f64,
    pub disarmed_share: f64,
}

impl Race {
    pub fn contender(&self, tag: &str) -> Option<&Contender> {
        self.contenders.iter().find(|c| c.tag == tag)
    }

    fn top_two(&self) -> (f64, f64) {
        let mut l: Vec<f64> = self.contenders.iter().map(|c| c.level).collect();
        l.sort_by(|a, b| b.total_cmp(a));
        (l.first().copied().unwrap_or(0.0), l.get(1).copied().unwrap_or(f64::NEG_INFINITY))
    }

    /// A paid step. The first step of a newcomer sets `first_step_level` (hypothesis: the design gives the cost of
    /// entering but not the level it sets); later steps add 1. Returns false when the region is full
    /// (four slots) or the race is closed.
    pub fn add_step(&mut self, r: &ColonyRules, tag: &str, cost: f64, today: i64) -> bool {
        if self.stage == Stage::Won {
            return false;
        }
        if let Some(c) = self.contenders.iter_mut().find(|c| c.tag == tag) {
            c.level += 1.0;
            c.points += cost;
            c.last_step = today;
            return true;
        }
        if self.stage == Stage::Influence || self.contenders.len() >= r.max_contenders {
            return false;
        }
        if self.contenders.is_empty() {
            self.date = today;
        }
        self.contenders.push(Contender { tag: tag.to_string(), level: r.first_step_level, points: cost, last_step: today });
        true
    }

    /// The daily stage machine. `days_per_month` converts COLONIZATION_MONTHS_TO_COLONIZE.
    pub fn daily(&mut self, r: &ColonyRules, today: i64, days_per_month: i64, climate: &Climate) {
        if self.contenders.is_empty() {
            *self = Race::default();
            return;
        }
        match self.stage {
            Stage::Open => {
                if self.contenders.len() >= 2 {
                    self.stage = Stage::Interest;
                } else if today - self.date > r.days_for_initial_investment {
                    self.contenders[0].level = r.interest_lead;
                    self.win(today);
                }
            }
            Stage::Interest => self.interest(r, today),
            Stage::Influence => {
                let (top, second) = self.top_two();
                if self.contenders.len() == 1 || top - second >= r.influence_lead {
                    self.keep_top(1);
                    self.win(today);
                }
            }
            Stage::Won => {
                if (today - self.date) as f64 > r.months_to_colonize * days_per_month as f64 {
                    *self = Race::default();
                }
            }
        }
        self.temperature = if self.stage == Stage::Influence { self.next_temperature(r, climate) } else { 0.0 };
    }

    /// The race is decided: the points invested in it are released at once (read off the original:
    /// a race won on 1842.7.1 stands with `invest=0` before any protectorate is made).
    fn win(&mut self, today: i64) {
        self.stage = Stage::Won;
        self.date = today;
        self.contenders.iter_mut().for_each(|c| c.points = 0.0);
    }

    fn keep_top(&mut self, n: usize) {
        self.contenders.sort_by(|a, b| b.level.total_cmp(&a.level));
        self.contenders.truncate(n);
    }

    fn interest(&mut self, r: &ColonyRules, today: i64) {
        let (top, _) = self.top_two();
        if self.contenders.len() == 1 {
            if top >= r.interest_lead {
                self.win(today);
            }
            return;
        }
        self.contenders.sort_by(|a, b| b.level.total_cmp(&a.level));
        if self.contenders.len() >= 3 {
            let lead = r.interest_lead;
            let mut i = 0;
            self.contenders.retain(|c| {
                i += 1;
                i <= 2 || top - c.level < lead
            });
        }
        let (top, second) = self.top_two();
        let n = self.contenders.len();
        if top - second >= r.interest_lead || (top >= r.interest_lead && n == 2) {
            self.keep_top(2);
            self.contenders[1].level = top - 1.0;
            self.contenders[0].points = r.influence_cost;
            self.stage = Stage::Influence;
        }
    }

    fn next_temperature(&self, r: &ColonyRules, c: &Climate) -> f64 {
        if c.is_crisis_region {
            return 100.0;
        }
        let (top, _) = self.top_two();
        let mut t = self.temperature + r.temp_per_day + r.temp_per_level * top;
        if c.crisis_running {
            t += r.tension_while_crisis;
        }
        t += r.at_war_tension_decay * (c.war_share + c.disarmed_share);
        t.clamp(0.0, 100.0)
    }

    /// In stage 2 the trailing contender steps when its last step is old enough and the
    /// leader may step on the same terms; this is the same day test as `may_step`.
    pub fn can_step_by_day(&self, r: &ColonyRules, tag: &str, today: i64) -> bool {
        self.contender(tag).is_none_or(|c| today - c.last_step >= r.days_between_investment)
    }

    /// A failed colony war goal costs the actor his claim: he is thrown out of the race.
    /// Returns true when he was in it. The race goes back to its start when it has no contender left.
    pub fn lose_claim(&mut self, tag: &str) -> bool {
        let before = self.contenders.len();
        self.contenders.retain(|c| c.tag != tag);
        let lost = self.contenders.len() != before;
        if lost && self.stage != Stage::Won && self.contenders.len() < 2 {
            self.stage = Stage::Open;
            self.temperature = 0.0;
        }
        if self.contenders.is_empty() {
            *self = Race::default();
        }
        lost
    }
}

/// The peace option po_colony: the colonial region goes to the actor, who becomes owner of its
/// unowned provinces as a protectorate, and the race is reset. Costs and consequences are
/// returned for the caller to apply (peace cost, infamy from the defines).
#[derive(Debug, Clone, PartialEq)]
pub struct PoColony {
    pub provinces: usize,
    pub prestige: f64,
    pub infamy: f64,
    pub war_score_cost: f64,
}

pub fn po_colony(r: &ColonyRules, race: &mut Race, unowned_provinces: usize) -> PoColony {
    *race = Race::default();
    PoColony { provinces: unowned_provinces, prestige: r.colony_formed_prestige * unowned_provinces as f64, infamy: r.infamy_colony, war_score_cost: r.peace_cost_colony }
}

// ---- the three commands ----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Created {
    pub cost: f64,
    pub prestige: f64,
}

/// Winner creates the protectorate: needs free points >= the cost; every unowned province becomes
/// his, prestige per province. The race is reset.
pub fn create_protectorate(r: &ColonyRules, race: &mut Race, tag: &str, unowned_provinces: usize, free: f64) -> Result<Created, Refusal> {
    if race.stage != Stage::Won || race.contenders.first().map(|c| c.tag.as_str()) != Some(tag) {
        return Err(Refusal::RaceClosed);
    }
    if free < r.protectorate_cost {
        return Err(Refusal::NotEnoughPoints);
    }
    *race = Race::default();
    Ok(Created { cost: r.protectorate_cost, prestige: r.colony_formed_prestige * unowned_provinces as f64 })
}

/// Protectorate to colony: status 1 and free points >= provinces x colony cost.
pub fn create_colony(r: &ColonyRules, status: Status, provinces: usize, free: f64) -> Result<Created, Refusal> {
    if status != Status::Protectorate {
        return Err(Refusal::WrongStatus);
    }
    let cost = r.colony_cost * provinces as f64;
    if free < cost {
        return Err(Refusal::NotEnoughPoints);
    }
    Ok(Created { cost, prestige: 0.0 })
}

/// Cost of turning a colony into a state: provinces x state cost x min(1, distance / limit), nothing on the
/// capital's own continent. The distance is `state_distance` of the two capitals.
pub fn state_cost(r: &ColonyRules, provinces: usize, distance_from_capital: f64, same_continent: bool) -> f64 {
    if same_continent {
        return 0.0;
    }
    provinces as f64 * r.state_cost * (distance_from_capital / r.state_distance).min(1.0)
}

/// The distance the state cost uses: the straight line between the centre of the state's capital
/// province and the centre of the owner's capital, in map pixels / `state_distance_pixels`.
/// Read off the original: a two-province state costs 388 where the middles of the two provinces'
/// bounding boxes are 1036 pixels apart (their pixel centroids, 1033 apart, would give 387).
pub fn state_distance(r: &ColonyRules, a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1) / r.state_distance_pixels
}

/// Colony to state. `conditions_met` stands for the bureaucrat/population test, whose
/// operands the rule description lost: the caller decides (open point).
pub fn create_state(r: &ColonyRules, status: Status, at_war: bool, conditions_met: bool, cost: f64, free: f64) -> Result<Created, Refusal> {
    if at_war {
        return Err(Refusal::AtWar);
    }
    if status != Status::Colony {
        return Err(Refusal::WrongStatus);
    }
    if !conditions_met {
        return Err(Refusal::ConditionsNotMet);
    }
    if free < cost {
        return Err(Refusal::NotEnoughPoints);
    }
    Ok(Created { cost, prestige: r.colony_to_state_prestige })
}

// ---- the AI's choice (hypothesis) ----------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct RegionOption {
    pub region: u32,
    pub cost: f64,
    pub provinces: usize,
}

/// The rule description says only that the AI runs its colonisation step with chance 1/3 per run and that it
/// reads the colonial costs. Stand-in (hypothesis): among the regions it may step into
/// and can pay for, take the most provinces for the cost, ties to the lower id.
pub fn ai_pick_region(options: &[RegionOption], free: f64) -> Option<u32> {
    options
        .iter()
        .filter(|o| o.cost <= free && o.provinces > 0)
        .max_by(|a, b| (a.provinces as f64 / a.cost.max(1.0)).total_cmp(&(b.provinces as f64 / b.cost.max(1.0))).then(b.region.cmp(&a.region)))
        .map(|o| o.region)
}

// ---- state from a save -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ColonyProvince {
    pub owner: Option<String>,
    pub status: Option<Status>,
    pub naval_base: f64,
    pub railroad: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColonyHolding {
    pub owner: String,
    pub provinces: Vec<u16>,
    pub status: Status,
    pub factories: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ColonyCountry {
    pub great_power: bool,
    /// The country's `colonize` block: region id and the level it holds there.
    pub races: BTreeMap<u32, f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColonyState {
    pub date: wiktoria_data::date::Date,
    pub countries: BTreeMap<String, ColonyCountry>,
    pub provinces: BTreeMap<u16, ColonyProvince>,
    /// Every state a country owns, colonial or not.
    pub states: Vec<ColonyHolding>,
    /// The save's race records, one per colonial region with a history.
    pub races: Vec<RaceRecord>,
}

/// One top-level `region` block of a save: the race for a colonial region.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RaceRecord {
    pub region: u32,
    pub stage: Stage,
    pub date: Option<wiktoria_data::date::Date>,
    pub temperature: f64,
    /// (country, level, points invested, day of its last step).
    pub contenders: Vec<(String, f64, f64, Option<wiktoria_data::date::Date>)>,
}

fn first_number(b: &wiktoria_data::script::Block, k: &str) -> f64 {
    match b.get(k) {
        Some(Value::Block(x)) => x.bare_scalars().first().and_then(|s| s.parse().ok()).unwrap_or(0.0),
        Some(v) => v.as_f64().unwrap_or(0.0),
        None => 0.0,
    }
}

impl ColonyState {
    pub fn from_save(save: &SaveGame) -> ColonyState {
        let root = &save.doc.root;
        let great: Vec<&String> = root.get("great_nations").and_then(Value::as_block).map(|b| b.bare_scalars()).unwrap_or_default().iter().filter_map(|n| save.countries.get(n.parse::<usize>().ok()?.checked_sub(1)?)).collect();
        let mut out = ColonyState { date: save.date, countries: BTreeMap::new(), provinces: BTreeMap::new(), states: Vec::new(), races: Vec::new() };
        let date = |b: &wiktoria_data::script::Block| b.get("date").and_then(Value::as_str).and_then(wiktoria_data::date::Date::parse);
        for r in root.get_all("region").filter_map(Value::as_block) {
            let contenders = r.get_all("colony").filter_map(Value::as_block).filter_map(|c| Some((text(c, "tag")?, num(c, "points"), num(c, "invest"), date(c)))).collect();
            out.races.push(RaceRecord { region: num(r, "index") as u32, stage: Stage::from_save(num(r, "phase") as i64), date: date(r), temperature: num(r, "temperature"), contenders });
        }
        for (id, b, _) in province_blocks(root) {
            out.provinces.insert(id, ColonyProvince { owner: text(b, "owner"), status: b.get("colonial").and_then(Value::as_f64).map(Status::from_save), naval_base: first_number(b, "naval_base"), railroad: first_number(b, "railroad") });
        }
        for tag in &save.countries {
            let Some(b) = root.get(tag).and_then(Value::as_block) else { continue };
            let races = b.get("colonize").and_then(Value::as_block).map(|c| c.entries().filter_map(|(k, v, _)| Some((k.parse().ok()?, v.as_f64()?))).collect()).unwrap_or_default();
            out.countries.insert(tag.clone(), ColonyCountry { great_power: great.contains(&tag), races });
            for s in b.get_all("state").filter_map(Value::as_block) {
                let provinces = s.get("provinces").and_then(Value::as_block).map(|p| p.bare_scalars().iter().filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                let factories = s.get_all("state_buildings").count();
                out.states.push(ColonyHolding { owner: tag.clone(), provinces, status: Status::from_save(num(s, "is_colonial")), factories });
            }
        }
        out
    }

    /// Points a country has standing in the save's races.
    pub fn invested(&self, tag: &str) -> f64 {
        self.races.iter().flat_map(|r| &r.contenders).filter(|c| c.0 == tag).map(|c| c.2).sum()
    }

    /// The upkeep inputs of one country.
    pub fn holdings_of(&self, tag: &str) -> Vec<Holding> {
        self.states
            .iter()
            .filter(|s| s.owner == tag && s.status != Status::State)
            .map(|s| Holding { status: s.status, provinces: s.provinces.len(), factories: s.factories, railway_levels: s.provinces.iter().filter_map(|p| self.provinces.get(p)).map(|p| p.railroad).sum() })
            .collect()
    }

    /// (colonies, protectorates) counted by state.
    pub fn counts_by_state(&self, tag: &str) -> (usize, usize) {
        let n = |st| self.states.iter().filter(|s| s.owner == tag && s.status == st).count();
        (n(Status::Colony), n(Status::Protectorate))
    }

    /// (colony, protectorate) provinces the country owns, counted from the province blocks.
    pub fn counts_by_province(&self, tag: &str) -> (usize, usize) {
        let n = |st| self.provinces.values().filter(|p| p.owner.as_deref() == Some(tag) && p.status == Some(st)).count();
        (n(Status::Colony), n(Status::Protectorate))
    }
}
