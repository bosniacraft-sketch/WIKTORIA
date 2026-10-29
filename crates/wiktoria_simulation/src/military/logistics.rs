//! Units and logistics rules: supply limit, attrition, organisation, supply status, reinforcement,
//! regiment/pop check, recruiting, mobilisation, transports, leaders.
//!
//! Pure functions: they take plain numbers or the unit structs and return numbers or decisions, the
//! day loop (wired by the coordinator) applies them. Dice come in as parameters. Constants are in
//! [`LogisticsRules`] (rules file `FLD_MIL_LOG_*`, hypotheses), unit statistics come from the install.

use wiktoria_data::date::Date;
use wiktoria_data::defines::Defines;
use wiktoria_data::engine_rules::EngineRules;

use super::units::Army;
use crate::num_field;

/// Engine constants of this module.
#[derive(Debug, Clone, PartialEq)]
pub struct LogisticsRules {
    /// Supply multiplier in a province the country owns and controls.
    pub supply_own: f64,
    /// Supply multiplier when it controls, or owns, or is friendly with the controller.
    pub supply_friendly: f64,
    /// Supply multiplier otherwise.
    pub supply_hostile: f64,
    /// Added to the attrition of a unit that takes part in a siege (after the cap).
    pub siege_attrition: f64,
    /// Percent of a ship's strength per point of naval attrition is the same scale as land.
    pub ship_sunk_below: f64,
    /// Supply range of a fleet, in provinces, before technology.
    pub supply_range: f64,
    /// Organisation regained a day at supply 1.
    pub org_regain: f64,
    /// Spending slider floor and weight in maximum organisation: `floor + (1 - floor) x slider`.
    pub org_slider_floor: f64,
    /// Strength regained a month before location, supply and modifiers.
    pub reinforce_speed: f64,
    /// Pop size under which it supports no regiment.
    pub pop_min_size: f64,
    /// Pop size per regiment.
    pub pop_size_per_regiment: f64,
    pub protectorate_mult: f64,
    pub colony_mult: f64,
    pub noncore_mult: f64,
    /// A regiment of a pop that is no soldier type refills at this share.
    pub mobilised_refill_share: f64,
    /// A regiment whose pop supports none is deleted below this strength.
    pub delete_below_strength: f64,
    /// Militancy above which a rebel pop's regiment defects.
    pub mil_to_autorise: f64,
    pub min_mobilize_limit: f64,
    pub mobilization_speed_base: f64,
    pub mobilization_speed_rails_mult: f64,
    pub leader_recruit_cost: f64,
    pub leader_age_death_factor: f64,
    /// Under this age (years) a leader outside battle is safe.
    pub leader_safe_years: f64,
    pub leader_max_random_prestige: f64,
    pub leader_prestige_to_morale: f64,
    pub leader_prestige_to_max_org: f64,
    /// Maximum experience of a new regiment.
    pub max_experience: f64,
    /// Months of out-of-supply a fleet adds 2 attrition points for.
    pub naval_attrition_per_month: f64,
    /// AI_ARMY_TAXBASE_FRACTION: a computer country orders no regiment while the supply goods
    /// of its regiments, standing and building, cost more than this share of its tax base
    /// (unlimited where the defines lack it).
    pub ai_army_taxbase_fraction: f64,
    /// AI_NAVY_TAXBASE_FRACTION: the same for ships.
    pub ai_navy_taxbase_fraction: f64,
}

impl LogisticsRules {
    /// Engine literals from the rules file, everything `defines.lua` of the install has from there.
    pub fn new(m: &EngineRules, defines: &Defines) -> Result<LogisticsRules, String> {
        let f = |id: &str| num_field(m, id);
        let d = |section: &str, k: &str| defines.get(section, k).ok_or(format!("defines.{section}.{k} missing"));
        Ok(LogisticsRules {
            supply_own: f("FLD_MIL_LOG_SUPPLY_OWN")?,
            supply_friendly: f("FLD_MIL_LOG_SUPPLY_FRIENDLY")?,
            supply_hostile: f("FLD_MIL_LOG_SUPPLY_HOSTILE")?,
            siege_attrition: d("military", "SIEGE_ATTRITION")?,
            ship_sunk_below: f("FLD_MIL_LOG_SHIP_SUNK_BELOW")?,
            supply_range: d("military", "SUPPLY_RANGE")?,
            org_regain: f("FLD_MIL_LOG_ORG_REGAIN")?,
            org_slider_floor: f("FLD_MIL_LOG_ORG_SLIDER_FLOOR")?,
            reinforce_speed: d("military", "REINFORCE_SPEED")?,
            pop_min_size: d("military", "POP_MIN_SIZE_FOR_REGIMENT")?,
            pop_size_per_regiment: d("military", "POP_SIZE_PER_REGIMENT")?,
            protectorate_mult: d("military", "POP_MIN_SIZE_FOR_REGIMENT_PROTECTORATE_MULTIPLIER")?,
            colony_mult: d("military", "POP_MIN_SIZE_FOR_REGIMENT_COLONY_MULTIPLIER")?,
            noncore_mult: d("military", "POP_MIN_SIZE_FOR_REGIMENT_NONCORE_MULTIPLIER")?,
            mobilised_refill_share: f("FLD_MIL_LOG_MOBILISED_REFILL_SHARE")?,
            delete_below_strength: f("FLD_MIL_LOG_DELETE_BELOW_STRENGTH")?,
            mil_to_autorise: d("pops", "MIL_TO_AUTORISE")?,
            min_mobilize_limit: d("country", "MIN_MOBILIZE_LIMIT")?,
            mobilization_speed_base: d("country", "MOBILIZATION_SPEED_BASE")?,
            mobilization_speed_rails_mult: d("country", "MOBILIZATION_SPEED_RAILS_MULT")?,
            leader_recruit_cost: d("military", "LEADER_RECRUIT_COST")?,
            leader_age_death_factor: d("military", "LEADER_AGE_DEATH_FACTOR")?,
            leader_safe_years: f("FLD_MIL_LOG_LEADER_SAFE_YEARS")?,
            leader_max_random_prestige: d("military", "LEADER_MAX_RANDOM_PRESTIGE")?,
            leader_prestige_to_morale: d("military", "LEADER_PRESTIGE_TO_MORALE_FACTOR")?,
            leader_prestige_to_max_org: d("military", "LEADER_PRESTIGE_TO_MAX_ORG_FACTOR")?,
            max_experience: f("FLD_MIL_LOG_MAX_EXPERIENCE")?,
            naval_attrition_per_month: f("FLD_MIL_LOG_NAVAL_ATTRITION_PER_MONTH")?,
            ai_army_taxbase_fraction: defines.get("military", "AI_ARMY_TAXBASE_FRACTION").unwrap_or(f64::INFINITY),
            ai_navy_taxbase_fraction: defines.get("military", "AI_NAVY_TAXBASE_FRACTION").unwrap_or(f64::INFINITY),
        })
    }
}

// ---------------------------------------------------------------- supply limit and attrition

/// How a country stands to a land province, for the supply limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupplyAccess {
    /// Owns and controls it.
    OwnControlled,
    /// Controls it without owning it.
    Controlled,
    /// Owns it, someone else controls it (the original treats it as friendly).
    OwnedOccupied,
    /// Alliance, military access, sphere or rebels, or the province is under siege.
    Friendly,
    /// None of the above.
    Hostile,
    /// A sea province.
    Sea,
}

/// Supply limit modifier of a land province: its terrain, the static block of every land province
/// and the static infrastructure block times the province's infrastructure.
pub fn province_supply_modifier(terrain: f64, land_block: f64, infrastructure_block: f64, infrastructure: f64) -> f64 {
    terrain + land_block + infrastructure_block * infrastructure
}

/// Men (in thousands, whole part) a country's armies can stand in a province without attrition.
pub fn supply_limit(r: &LogisticsRules, province_modifier: f64, access: SupplyAccess, tech_supply: f64) -> i64 {
    let m = match access {
        SupplyAccess::OwnControlled => r.supply_own,
        SupplyAccess::Controlled | SupplyAccess::OwnedOccupied | SupplyAccess::Friendly => r.supply_friendly,
        SupplyAccess::Hostile => r.supply_hostile,
        SupplyAccess::Sea => 1.0,
    };
    ((1.0 + province_modifier) * m * (1.0 + tech_supply)).floor() as i64
}

/// Whole weight of a stack in thousands of men: strength summed over regiments (1.0 = 1000 men).
pub fn stack_weight(armies: &[&Army]) -> f64 {
    armies.iter().flat_map(|a| &a.regiments).map(|r| r.strength).sum()
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LandAttrition {
    pub exiled: bool,
    /// See [`stack_weight`]: own armies only when in an own province, not in battle, not retreating.
    pub weight: f64,
    pub leader_trait: f64,
    pub tech_attrition: f64,
    /// Result of [`supply_limit`].
    pub supply_limit: i64,
    /// Highest fort level among neighbours held by a hostile controller.
    pub enemy_fort: f64,
    pub province_attrition: f64,
    pub province_max_attrition: f64,
    pub in_siege: bool,
}

/// Maximum attrition of a land province: the static block of every land province plus its terrain,
/// climate and continent (capped at the whole part of the sum).
pub fn province_max_attrition(land_block: f64, terrain: f64, climate: f64, continent: f64) -> f64 {
    land_block + terrain + climate + continent
}

/// Attrition in percent of maximum strength per application.
pub fn land_attrition_percent(r: &LogisticsRules, a: &LandAttrition) -> f64 {
    if a.exiled {
        return 0.0;
    }
    let raw = (a.weight + a.leader_trait) * (1.0 + a.tech_attrition) - a.supply_limit as f64 + a.enemy_fort + a.province_attrition;
    let mut v = raw.min(a.province_max_attrition.floor()).max(0.0);
    if a.in_siege {
        v += r.siege_attrition;
    }
    v
}

/// Strength a regiment loses at `percent`: `max strength x percent / 100`.
pub fn attrition_loss(max_strength: f64, percent: f64) -> f64 {
    max_strength * percent * 0.01
}

/// Whether a ship is hit this time: `(ship id x unit dice) mod 10 < whole percent`.
pub fn ship_hit(ship_id: u64, unit_dice: u64, percent: f64) -> bool {
    percent > 0.0 && (ship_id.wrapping_mul(unit_dice) % 10) < percent.floor() as u64
}

/// Whether a ship left at `strength` sinks.
pub fn ship_sinks(r: &LogisticsRules, strength: f64) -> bool {
    strength < r.ship_sunk_below
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NavalAttrition {
    pub in_port: bool,
    /// A neighbouring province the country controls.
    pub next_to_own_land: bool,
    /// Coastal sea within supply range of a free, own, controlled port with a naval base.
    pub in_supply_range: bool,
    pub months_out_of_supply: f64,
    pub leader_trait: f64,
    pub tech_attrition: f64,
}

/// Naval attrition in percent: 1, 3, 5 ... a month out of supply.
pub fn naval_attrition_percent(r: &LogisticsRules, n: &NavalAttrition) -> f64 {
    if n.in_port || n.next_to_own_land || n.in_supply_range {
        return 0.0;
    }
    ((1.0 + r.naval_attrition_per_month * n.months_out_of_supply + n.leader_trait).max(0.0)) * (1.0 + n.tech_attrition)
}

/// Largest distance a fleet may be from a supplying port.
pub fn fleet_supply_range(r: &LogisticsRules, tech_range: f64) -> f64 {
    r.supply_range * (1.0 + tech_range)
}

// ---------------------------------------------------------------- organisation and supply

/// Maximum organisation of a regiment or ship. Rebels skip the slider.
pub fn max_organisation(r: &LogisticsRules, type_org: f64, org_modifier: f64, leader_org: f64, slider: f64, rebel: bool) -> f64 {
    let s = if rebel { 1.0 } else { r.org_slider_floor + (1.0 - r.org_slider_floor) * slider };
    type_org * (1.0 + org_modifier) * (1.0 + leader_org) * s
}

/// Organisation after a day's regain (not in battle; not for an army on a fleet at sea).
pub fn regain_organisation(r: &LogisticsRules, org: f64, max_org: f64, supply_status: f64, leader_morale: f64, tech_morale: f64, org_regain_modifier: f64) -> f64 {
    (org + r.org_regain * supply_status * (1.0 + leader_morale + tech_morale + org_regain_modifier)).min(max_org).max(0.0)
}

/// Supply status of a unit: the mean over regiments of the share of its goods the stockpile covers
/// `covered` holds each regiment's covered share in 0..1. Rebels have 1.
pub fn supply_status(covered: &[f64], rebel: bool) -> f64 {
    if rebel {
        return 1.0;
    }
    if covered.is_empty() {
        return 1.0;
    }
    covered.iter().map(|c| c.clamp(0.0, 1.0)).sum::<f64>() / covered.len() as f64
}

/// Goods one regiment wants a day of a supply good: `amount x (1 - share) x max(floor, consumption + modifier) x slider`.
pub fn supply_need(amount: f64, free_share: f64, consumption: f64, consumption_floor: f64, consumption_modifier: f64, slider: f64) -> f64 {
    amount * (1.0 - free_share) * (consumption_floor.max(consumption + consumption_modifier)) * slider
}

// ---------------------------------------------------------------- reinforcement

/// Where an army stands, for [`reinforce_location_factor`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReinforcePlace {
    Exiled,
    /// The province belongs to the unit's country.
    Owned,
    /// Rebels: only when the rebel type reinforces; 1.0 under rebel control, else 0.5.
    Rebel { reinforcing: bool, rebel_controlled: bool },
    /// Unowned province.
    Unowned { coastal_free: bool },
    /// Controlled by the unit's country or by a neutral.
    FriendlyOrNeutral,
    /// Controlled by a hostile country.
    Hostile { friendly_land_neighbour: bool, coastal_free: bool },
}

pub fn reinforce_location_factor(p: ReinforcePlace) -> f64 {
    match p {
        ReinforcePlace::Exiled => 0.0,
        ReinforcePlace::Owned => 2.0,
        ReinforcePlace::Rebel { reinforcing, rebel_controlled } => {
            if !reinforcing {
                0.0
            } else if rebel_controlled {
                1.0
            } else {
                0.5
            }
        }
        ReinforcePlace::Unowned { coastal_free } => {
            if coastal_free {
                0.1
            } else {
                0.0
            }
        }
        ReinforcePlace::FriendlyOrNeutral => 1.0,
        ReinforcePlace::Hostile { friendly_land_neighbour, coastal_free } => {
            if friendly_land_neighbour {
                0.5
            } else if coastal_free {
                0.25
            } else {
                0.1
            }
        }
    }
}

/// Army rate a month before the supply status.
pub fn reinforce_rate(r: &LogisticsRules, tech_rate: f64, location_factor: f64, speed_modifier: f64) -> f64 {
    r.reinforce_speed * (1.0 + tech_rate) * location_factor * (1.0 + speed_modifier)
}

/// One regiment or ship waiting for men.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Refill {
    pub strength: f64,
    pub max_strength: f64,
    pub experience: f64,
    /// Share of the rate this one gets: 0 with no pop, see [`pop_refill_share`], 1 for a ship.
    pub share: f64,
}

/// A regiment's share of the rate: 0.5 for a pop of no soldier type; for a soldier pop `min(1,
/// supports / has)`.
pub fn pop_refill_share(r: &LogisticsRules, soldier_pop: bool, supports: u32, has: u32) -> f64 {
    if soldier_pop && supports > 0 && has > 0 {
        (supports as f64 / has as f64).min(1.0)
    } else {
        r.mobilised_refill_share
    }
}

/// Monthly reinforcement of a unit in list order. The running rate is multiplied by the supply
/// status for each one that needs men, so later ones get less (rate x s^k). Experience dilutes as
/// `experience / (1 + added rate / 3)`. Returns the strength added in total (in units of strength).
pub fn reinforce(rate: f64, supply: f64, units: &mut [Refill]) -> f64 {
    let mut running = rate;
    let mut added = 0.0;
    for u in units.iter_mut() {
        if u.share <= 0.0 || u.strength >= u.max_strength {
            continue;
        }
        running *= supply;
        let add = (u.max_strength * running * u.share).min(u.max_strength - u.strength).max(0.0);
        u.strength += add;
        added += add;
        u.experience /= 1.0 + running * u.share / 3.0;
    }
    added
}

// ---------------------------------------------------------------- regiments and pops

/// Where a pop's province stands for the regiment divisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopStatus {
    Protectorate,
    Colony,
    /// Not a core of the owner (and no status).
    NonCore,
    Normal,
}

/// How many regiments a pop supports. `skip_multipliers` is the flag argument.
pub fn regiments_supported(r: &LogisticsRules, size: f64, status: PopStatus, skip_multipliers: bool) -> u32 {
    if size < r.pop_min_size {
        return 0;
    }
    let mult = if skip_multipliers {
        1.0
    } else {
        match status {
            PopStatus::Protectorate => r.protectorate_mult,
            PopStatus::Colony => r.colony_mult,
            PopStatus::NonCore => r.noncore_mult,
            PopStatus::Normal => 1.0,
        }
    };
    (size / (r.pop_size_per_regiment * mult)).floor() as u32 + 1
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegimentCheck {
    pub rebel_unit: bool,
    pub has_pop: bool,
    pub pop_in_rebel_faction: bool,
    pub pop_militancy: f64,
    /// The pop's province now belongs to another country than the unit's.
    pub province_changed_owner: bool,
    pub soldier_pop: bool,
    pub at_war: bool,
    /// Result of [`regiments_supported`].
    pub supports: u32,
    pub strength: f64,
    pub experience: f64,
    pub start_experience: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RegimentVerdict {
    Keep,
    /// Look for a pop (the original calls a pop search not read).
    FindPop,
    /// Moves into a new rebel army of the province.
    Defect,
    Delete,
    /// Moves into a new army of the province's new owner.
    MoveToNewOwner,
    /// Stays, experience is raised to the start level.
    RaiseExperience(f64),
}

/// The daily regiment/pop check. Not for ships or units in battle.
pub fn regiment_check(r: &LogisticsRules, c: &RegimentCheck) -> RegimentVerdict {
    if !c.has_pop {
        return RegimentVerdict::FindPop;
    }
    if c.pop_in_rebel_faction && c.pop_militancy > r.mil_to_autorise && !c.rebel_unit {
        return RegimentVerdict::Defect;
    }
    if c.rebel_unit && !c.pop_in_rebel_faction {
        return RegimentVerdict::Delete;
    }
    if c.province_changed_owner && c.soldier_pop {
        return if c.at_war && !c.rebel_unit { RegimentVerdict::MoveToNewOwner } else if !c.rebel_unit { RegimentVerdict::Delete } else { RegimentVerdict::Keep };
    }
    if c.supports == 0 {
        return if c.strength < r.delete_below_strength { RegimentVerdict::Delete } else { RegimentVerdict::Keep };
    }
    if c.soldier_pop && c.experience < c.start_experience {
        return RegimentVerdict::RaiseExperience(c.start_experience);
    }
    RegimentVerdict::Keep
}

// ---------------------------------------------------------------- recruiting

/// Days a unit takes to build: the type's build time shortened or lengthened by the recruitment
/// time modifier ([?] the exact form), at least one day.
pub fn build_days(build_time: f64, recruit_time_modifier: f64) -> u32 {
    (build_time * (1.0 + recruit_time_modifier).max(0.0)).ceil().max(1.0) as u32
}

/// Whether the unit is finished after `elapsed` days.
pub fn build_done(days: u32, elapsed: i64) -> bool {
    elapsed >= days as i64
}

/// Experience of a new regiment: technology level plus the start experience modifier, capped.
pub fn start_experience(r: &LogisticsRules, tech_level: f64, modifier: f64) -> f64 {
    (tech_level + modifier).min(r.max_experience)
}

// ---------------------------------------------------------------- mobilisation

/// Most regiments mobilisation may raise.
pub fn mobilise_limit(r: &LogisticsRules, regiments_in_armies: usize, impact_modifier: f64) -> u32 {
    (r.min_mobilize_limit.max(regiments_in_armies as f64) * (1.0 + impact_modifier).max(0.0)).floor() as u32
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MobilisablePop {
    pub id: u64,
    pub size: f64,
    /// Regiments of this pop already standing.
    pub has: u32,
}

/// Which pops raise how many regiments. `pops` is already filtered (poor, non-soldier, primary or
/// accepted culture, owned and controlled, non-colonial) and in the country's walk order. Small
/// fractions are pooled into the next pop. The walk stops when the total passes `limit`, the pop
/// that would pass it is not scheduled. `size_share` is the country's mobilisation size.
pub fn mobilise_schedule(r: &LogisticsRules, pops: &[MobilisablePop], size_share: f64, limit: u32) -> Vec<(u64, u32)> {
    let mut out = Vec::new();
    let mut pool = 0.0;
    let mut total = 0u32;
    for p in pops {
        let want = p.size / r.pop_size_per_regiment * size_share;
        let whole = if want.floor() < 1.0 {
            if pool + want >= 1.0 {
                let w = (pool + want).floor();
                pool = 0.0;
                w
            } else {
                pool += want;
                0.0
            }
        } else {
            want.floor()
        };
        let n = (whole as i64 - p.has as i64).max(0) as u32;
        if n > 0 {
            total += n;
            pool = 0.0;
            if total > limit {
                break;
            }
            out.push((p.id, n));
        }
    }
    out
}

/// Railway factor of a state: `1 + rails mult x mean(level / max level)` over its provinces.
pub fn mobilisation_rail_factor(r: &LogisticsRules, levels: &[f64], max_level: f64) -> f64 {
    if levels.is_empty() || max_level <= 0.0 {
        return 1.0;
    }
    1.0 + r.mobilization_speed_rails_mult * (levels.iter().sum::<f64>() / levels.len() as f64 / max_level)
}

/// One day of a schedule entry: paused while a province is occupied, else progress rises, capped at 1.
pub fn mobilisation_progress(r: &LogisticsRules, progress: f64, rail_factor: f64, paused: bool) -> f64 {
    if paused {
        progress
    } else {
        (progress + r.mobilization_speed_base * rail_factor).min(1.0)
    }
}

/// Type and starting experience of a mobilised regiment: infantry if unlocked, else irregular.
pub fn mobilised_unit<'a>(infantry_unlocked: bool, infantry: &'a str, irregular: &'a str, r: &LogisticsRules, tech_level: f64, modifier: f64) -> (&'a str, f64) {
    if infantry_unlocked {
        (infantry, (tech_level + modifier).min(r.max_experience))
    } else {
        (irregular, modifier.min(r.max_experience))
    }
}

/// Production factor of the country while mobilised (`1 - impact x size share`, floor 0).
pub fn mobilisation_production_factor(mobilised: bool, impact: f64, size_share: f64) -> f64 {
    if mobilised {
        (1.0 - impact.max(0.0) * size_share).max(0.0)
    } else {
        1.0
    }
}

/// Demobilising: removes every regiment whose pop type is not a soldier type. Returns the pop ids
/// freed (for the caller's bookkeeping); an army left empty is for the caller to delete.
pub fn demobilise(army: &mut Army, is_soldier_type: &dyn Fn(&str) -> bool) -> Vec<u64> {
    let mut gone = Vec::new();
    army.regiments.retain(|reg| {
        let mobilised = reg.pop_type.as_deref().map(|t| !is_soldier_type(t)).unwrap_or(false);
        if let (true, Some(p)) = (mobilised, reg.pop) {
            gone.push(p);
        }
        !mobilised
    });
    gone
}

// ---------------------------------------------------------------- transports

/// Room of a fleet: whole part of the summed capacity of its ships (1.0 per transport ship).
pub fn fleet_room(capacities: &[f64]) -> usize {
    capacities.iter().sum::<f64>().floor() as usize
}

/// May an army step onto a sea province: its regiments plus those of the country's other armies
/// there must fit in the number of the country's transport ships there.
pub fn may_enter_sea(moving_regiments: usize, other_regiments_there: usize, transport_ships_there: usize) -> bool {
    moving_regiments + other_regiments_there <= transport_ships_there
}

/// May an army board a particular fleet.
pub fn may_board(same_country: bool, fleet_busy: bool, carried: usize, army_regiments: usize, room: usize) -> bool {
    same_country && !fleet_busy && carried + army_regiments <= room
}

/// Over capacity: deletes the first regiment of the first carried army that has any, until the
/// carried regiments fit. Emptied armies are removed. Returns the regiments lost.
pub fn trim_over_capacity(carried: &mut Vec<Army>, room: usize) -> usize {
    let mut lost = 0;
    while carried.iter().map(|a| a.regiments.len()).sum::<usize>() > room {
        let Some(a) = carried.iter_mut().find(|a| !a.regiments.is_empty()) else { break };
        a.regiments.remove(0);
        lost += 1;
    }
    carried.retain(|a| !a.regiments.is_empty());
    lost
}

// ---------------------------------------------------------------- leaders

/// Leadership points a month. Each pop type: `(leadership value, share of the
/// population, optimum share)`.
pub fn leadership_points(types: &[(f64, f64, f64)], modifier: f64, modifier_modifier: f64) -> f64 {
    let sum: f64 = types.iter().filter(|t| t.0 != 0.0).map(|t| t.0 * if t.2 > 0.0 { (t.1 / t.2).min(1.0) } else { 1.0 }).sum();
    (sum + modifier) * (1.0 + modifier_modifier)
}

/// The pool after a month, capped at 3 x the recruit cost.
pub fn add_leadership(r: &LogisticsRules, pool: f64, gain: f64) -> f64 {
    (pool + gain).min(3.0 * r.leader_recruit_cost)
}

pub fn can_create_leader(r: &LogisticsRules, pool: f64) -> bool {
    pool / r.leader_recruit_cost >= 1.0
}

/// The pool after buying a leader, not below 0.
pub fn pay_leader(r: &LogisticsRules, pool: f64) -> f64 {
    (pool - r.leader_recruit_cost).max(0.0)
}

/// A new leader's traits: personality and background drawn uniformly from entries 1.. of their
/// lists (entry 0 is "none"). `count` is the list length, `dice` a random number. Returns 0 if
/// the list has no real entry.
pub fn draw_trait(count: usize, dice: u64) -> usize {
    if count < 2 {
        0
    } else {
        1 + (dice % (count as u64 - 1)) as usize
    }
}

/// Starting prestige, `dice01` in 0..1: random up to the maximum, 0 if the maximum is 0.
pub fn new_leader_prestige(r: &LogisticsRules, dice01: f64) -> f64 {
    if r.leader_max_random_prestige > 0.0 {
        (dice01.clamp(0.0, 1.0) * r.leader_max_random_prestige).min(100.0)
    } else {
        0.0
    }
}

/// Morale and maximum organisation bonus of a leader's prestige (clamped 0..100).
pub fn prestige_bonus(r: &LogisticsRules, prestige: f64) -> (f64, f64) {
    let p = prestige.clamp(0.0, 100.0);
    (p * r.leader_prestige_to_morale, p * r.leader_prestige_to_max_org)
}

/// Whole years from `from` to `to`.
pub fn whole_years(from: Date, to: Date) -> i32 {
    let mut y = to.year - from.year;
    if (to.month, to.day) < (from.month, from.day) {
        y -= 1;
    }
    y.max(0)
}

/// Daily death test of a leader by age. `dice` is a random number.
pub fn leader_dies(r: &LogisticsRules, years: i32, in_battle: bool, dice: u64) -> bool {
    if !in_battle && (years as f64) < r.leader_safe_years {
        return false;
    }
    let mut chance = (years as f64 / r.leader_age_death_factor).floor() + 1.0;
    if in_battle {
        chance *= 2.0;
    }
    (dice % 10950) < chance as u64
}
