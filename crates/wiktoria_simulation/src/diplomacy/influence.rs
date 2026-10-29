//! Great-power influence, spheres and the seven influence actions. Opinion level 5 in a great power's relation record is the sphere.

use std::collections::BTreeMap;

use wiktoria_data::date::Date;
use wiktoria_data::defines::Defines;
use wiktoria_data::engine_rules::EngineRules;

use super::relations::level;
use super::{define, DipRules, DiplomacyState};
use crate::calendar::Calendar;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Action {
    IncreaseOpinion,
    DecreaseOpinion,
    AddToSphere,
    RemoveFromSphere,
    Discredit,
    ExpelAdvisors,
    BanEmbassy,
}

impl Action {
    pub const ALL: [Action; 7] = [Action::IncreaseOpinion, Action::DecreaseOpinion, Action::AddToSphere, Action::RemoveFromSphere, Action::Discredit, Action::ExpelAdvisors, Action::BanEmbassy];

    /// The constant prefix in defines.lua.
    fn prefix(self) -> &'static str {
        match self {
            Action::IncreaseOpinion => "INCREASEOPINION",
            Action::DecreaseOpinion => "DECREASEOPINION",
            Action::AddToSphere => "ADDTOSPHERE",
            Action::RemoveFromSphere => "REMOVEFROMSPHERE",
            Action::Discredit => "DISCREDIT",
            Action::ExpelAdvisors => "EXPELADVISORS",
            Action::BanEmbassy => "BANEMBASSY",
        }
    }

    /// The three of them that aim at a rival great power.
    pub fn aims_at_rival(self) -> bool {
        matches!(self, Action::DecreaseOpinion | Action::Discredit | Action::ExpelAdvisors | Action::BanEmbassy)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActionCost {
    pub influence: f64,
    pub relation_on_accept: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InfluenceRules {
    pub base_daily: f64,
    pub max_influence: f64,
    pub neighbour: f64,
    pub sphere_neighbour: f64,
    pub other_continent: f64,
    pub puppet: f64,
    pub relation_div: f64,
    pub investment_defense: f64,
    pub large_pop_penalty: f64,
    pub large_pop_limit: f64,
    pub large_pop_chunk: f64,
    pub discredit_gain: f64,
    pub discredit_days: f64,
    pub ban_days: f64,
    pub removefromsphere_prestige: f64,
    pub removefromsphere_infamy: f64,
    /// Opinion level a country holds in the leader's record after it is removed from the sphere.
    pub left_sphere_level: u8,
    pub actions: BTreeMap<Action, ActionCost>,
}

impl InfluenceRules {
    /// Install values from defines.lua; `m` is read for the level numbers (hypothesis rows).
    pub fn new(m: &EngineRules, d: &Defines) -> Result<InfluenceRules, String> {
        let mut actions = BTreeMap::new();
        for a in Action::ALL {
            let p = a.prefix();
            actions.insert(a, ActionCost { influence: define(d, "diplomacy", &format!("{p}_INFLUENCE_COST"))?, relation_on_accept: define(d, "diplomacy", &format!("{p}_RELATION_ON_ACCEPT"))? });
        }
        Ok(InfluenceRules {
            base_daily: define(d, "country", "BASE_GREATPOWER_DAILY_INFLUENCE")?,
            max_influence: define(d, "diplomacy", "MAX_INFLUENCE")?,
            neighbour: define(d, "diplomacy", "NEIGHBOUR_BONUS_INFLUENCE_PERCENT")?,
            sphere_neighbour: define(d, "diplomacy", "SPHERE_NEIGHBOUR_BONUS_INFLUENCE_PERCENT")?,
            other_continent: define(d, "diplomacy", "OTHER_CONTINENT_BONUS_INFLUENCE_PERCENT")?,
            puppet: define(d, "diplomacy", "PUPPET_BONUS_INFLUENCE_PERCENT")?,
            relation_div: define(d, "diplomacy", "RELATION_INFLUENCE_MODIFIER")?,
            investment_defense: define(d, "diplomacy", "INVESTMENT_INFLUENCE_DEFENSE")?,
            large_pop_penalty: define(d, "diplomacy", "LARGE_POPULATION_INFLUENCE_PENALTY")?,
            large_pop_limit: define(d, "pops", "LARGE_POPULATION_LIMIT")?,
            large_pop_chunk: define(d, "pops", "LARGE_POPULATION_INFLUENCE_PENALTY_CHUNK")?,
            discredit_gain: define(d, "diplomacy", "DISCREDIT_INFLUENCE_GAIN_FACTOR")?,
            discredit_days: define(d, "diplomacy", "DISCREDIT_DAYS")?,
            ban_days: define(d, "diplomacy", "BANEMBASSY_DAYS")?,
            removefromsphere_prestige: define(d, "diplomacy", "REMOVEFROMSPHERE_PRESTIGE_COST")?,
            removefromsphere_infamy: define(d, "diplomacy", "REMOVEFROMSPHERE_INFAMY_COST")?,
            left_sphere_level: crate::int_field(m, "FLD_DIP_LEVEL_FRIENDLY")? as u8,
            actions,
        })
    }

    pub fn cost(&self, a: Action) -> ActionCost {
        self.actions[&a]
    }
}

/// What the gain factor of one (power, target) pair depends on.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GainInput {
    pub discredited: bool,
    pub neighbour: bool,
    /// One of the target's neighbours has the power as sphere leader (only if not a neighbour).
    pub sphere_neighbour: bool,
    pub other_continent: bool,
    /// The power is the target's overlord and the target is no substate.
    pub puppet: bool,
    pub relation: f64,
    /// The power's investment in the target and all foreign investment there.
    pub investment: f64,
    pub foreign_investment: f64,
    pub population: f64,
    pub target_score: f64,
    pub power_score: f64,
}

/// The factor the daily share is multiplied by (never negative).
pub fn gain_factor(r: &InfluenceRules, i: &GainInput) -> f64 {
    let mut f = 1.0;
    if i.discredited {
        f += r.discredit_gain;
    }
    if i.neighbour {
        f += r.neighbour;
    } else if i.sphere_neighbour {
        f += r.sphere_neighbour;
    }
    if i.other_continent {
        f += r.other_continent;
    }
    if i.puppet {
        f += r.puppet;
    }
    f += i.relation / r.relation_div;
    if i.foreign_investment > 0.0 {
        f -= r.investment_defense * (1.0 - i.investment / i.foreign_investment);
    }
    let chunks = ((i.population - r.large_pop_limit) / r.large_pop_chunk).trunc();
    if chunks > 0.0 {
        f += r.large_pop_penalty * chunks;
    }
    let divisor = if i.power_score == 0.0 { 1.0 } else { i.power_score };
    f += 1.0 - (i.target_score / divisor).clamp(0.0, 2.0);
    f.max(0.0)
}

/// The great power's daily amount to split: base x (1 + influence modifier) x (1 + tech sum).
pub fn daily_amount(r: &InfluenceRules, modifier: f64, tech: f64) -> f64 {
    r.base_daily * (1.0 + modifier) * (1.0 + tech)
}

/// Share of every taking-part target: amount x priority / sum of priorities; with a sum of 0 each
/// gets amount / count.
pub fn shares(amount: f64, targets: &[(String, u8)]) -> BTreeMap<String, f64> {
    let sum: f64 = targets.iter().map(|(_, p)| f64::from(*p)).sum();
    targets.iter().map(|(t, p)| (t.clone(), if sum > 0.0 { amount * f64::from(*p) / sum } else { amount / targets.len() as f64 })).collect()
}

/// Influence after a gain: never below 0, above the cap the excess is returned apart.
pub fn add_influence(r: &InfluenceRules, current: f64, gain: f64) -> (f64, f64) {
    let v = (current + gain).max(0.0);
    if v > r.max_influence {
        (r.max_influence, v - r.max_influence)
    } else {
        (v, 0.0)
    }
}

/// What the caller knows about a target that the state does not.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TargetInfo {
    pub owns_land: bool,
    pub gain: GainInput,
}

/// A target takes part in the day's split.
pub fn takes_part(state: &DiplomacyState, r: &InfluenceRules, power: &str, target: &str, today: Date) -> bool {
    let rel = state.relation(power, target);
    if rel.and_then(|x| x.banned_until).is_some_and(|d| today <= d) {
        return false;
    }
    // At war: the rule description reads it from the relation's war field; the caller removes such pairs
    // from `targets` (the war state is not held here).
    let in_sphere = state.level(power, target) > level::FRIENDLY;
    if state.truce_active(power, target, today) && !in_sphere && state.overlord_of(power) != Some(target) {
        return false;
    }
    let mine = state.influence(power, target);
    mine < r.max_influence || state.great_powers().any(|g| g != power && state.influence(g, target) > 0.0)
}

/// One day of one great power's influence. `targets` = every country with land that is not a great
/// power and not at war with the power, with what the state does not hold. Returns the gains.
pub fn run_day(state: &mut DiplomacyState, rules: &DipRules, power: &str, today: Date, modifier: f64, tech: f64, targets: &BTreeMap<String, TargetInfo>) -> BTreeMap<String, f64> {
    let r = &rules.influence;
    let mut taking: Vec<(String, u8)> = Vec::new();
    for (t, info) in targets {
        if t == power || !info.owns_land || state.is_great_power(t) {
            continue;
        }
        if takes_part(state, r, power, t, today) {
            taking.push((t.clone(), state.priorities.get(&(power.to_string(), t.clone())).copied().unwrap_or(0)));
        }
    }
    let mut gains = BTreeMap::new();
    if taking.is_empty() {
        return gains;
    }
    let split = shares(daily_amount(r, modifier, tech), &taking);
    let mut overflow: Vec<(String, f64)> = Vec::new();
    for (t, share) in split {
        let banned = state.relation(power, &t).and_then(|x| x.banned_until).is_some_and(|d| today <= d);
        if banned {
            state.relation_mut(power, &t).influence = 0.0;
            continue;
        }
        let mut gi = targets[&t].gain;
        gi.discredited = state.relation(power, &t).and_then(|x| x.discredited_until).is_some_and(|d| today <= d);
        gi.relation = state.relation_value(power, &t);
        let gain = share * gain_factor(r, &gi);
        let (v, over) = add_influence(r, state.influence(power, &t), gain);
        state.relation_mut(power, &t).influence = v;
        gains.insert(t.clone(), gain);
        if over > 0.0 {
            overflow.push((t, over));
        }
    }
    // What goes above the cap is taken from the rival with most influence there (the amount
    // passed is read as minus the overflow).
    for (t, over) in overflow {
        let rival = state.great_powers().filter(|g| *g != power).map(|g| (g.to_string(), state.influence(g, &t))).filter(|(_, v)| *v > 0.0).max_by(|a, b| a.1.total_cmp(&b.1)).map(|(g, _)| g);
        if let Some(g) = rival {
            let x = state.influence(&g, &t);
            state.relation_mut(&g, &t).influence = (x - over).max(0.0);
        }
    }
    gains
}

// ---------------------------------------------------------------- spheres

/// Adds a country to the leader's sphere: level 5 in the leader's record.
pub fn enter_sphere(state: &mut DiplomacyState, leader: &str, target: &str) {
    state.relation_mut(leader, target).level = level::IN_SPHERE;
}

/// Takes a country out of its leader's sphere: the leader's level becomes friendly (4).
pub fn leave_sphere(state: &mut DiplomacyState, rules: &DipRules, target: &str) -> Option<String> {
    let leader = state.sphere_leader(target)?.to_string();
    state.relation_mut(&leader, target).level = rules.influence.left_sphere_level;
    Some(leader)
}

/// A country becomes a great power: out of its sphere, free of its overlord, and everybody's
/// influence in it is 0 and everybody's level towards it neutral.
pub fn becomes_great_power(state: &mut DiplomacyState, rules: &DipRules, tag: &str) {
    leave_sphere(state, rules, tag);
    state.subjects.retain(|s| s.subject != tag);
    for ((_, b), r) in state.relations.iter_mut() {
        if b == tag {
            r.influence = 0.0;
            r.level = level::NEUTRAL;
        }
    }
    if let Some(c) = state.countries.get_mut(tag) {
        c.great_power = true;
    }
}

/// A great power falls: every member leaves the sphere, its level towards them is neutral and its
/// influence there 0. Influence outside the sphere is kept.
pub fn dissolve_sphere(state: &mut DiplomacyState, tag: &str) {
    let members: Vec<String> = state.sphere_members(tag).map(str::to_string).collect();
    for m in members {
        let r = state.relation_mut(tag, &m);
        r.level = level::NEUTRAL;
        r.influence = 0.0;
    }
    if let Some(c) = state.countries.get_mut(tag) {
        c.great_power = false;
    }
}

// ---------------------------------------------------------------- the seven actions

fn plus_days(cal: &Calendar, d: Date, days: f64) -> Option<Date> {
    cal.to_game(d).map(|g| cal.to_date(crate::calendar::GameDate(g.0 + days as i64 * 24)))
}

/// Why an action cannot be done, or `Ok`. `rival` is the great power an action aims at.
pub fn check_action(state: &DiplomacyState, rules: &DipRules, a: Action, actor: &str, target: &str, rival: Option<&str>) -> Result<(), &'static str> {
    let r = &rules.influence;
    if state.influence(actor, target) < r.cost(a).influence {
        return Err("not enough influence");
    }
    let mine = state.level(actor, target);
    let rival_ok = |max_level_check: bool| -> Result<&str, &'static str> {
        let g = rival.ok_or("no rival named")?;
        if g == actor || !state.is_great_power(g) {
            return Err("the rival must be another great power");
        }
        if max_level_check && state.level(g, target) > mine {
            return Err("the rival's level is above the actor's");
        }
        Ok(g)
    };
    match a {
        Action::IncreaseOpinion => (mine < level::FRIENDLY).then_some(()).ok_or("level is already friendly"),
        Action::AddToSphere => {
            if state.is_great_power(target) {
                Err("the target is a great power")
            } else if mine < level::FRIENDLY {
                Err("level below friendly")
            } else if state.sphere_leader(target).is_some() {
                Err("the target already has a sphere leader")
            } else {
                Ok(())
            }
        }
        Action::RemoveFromSphere => {
            if state.sphere_leader(target) == Some(actor) {
                return Ok(());
            }
            match state.sphere_leader(target) {
                Some(l) if mine >= level::FRIENDLY && state.level(l, target) == level::IN_SPHERE => Ok(()),
                _ => Err("not in the actor's sphere, and no friendly standing against the leader"),
            }
        }
        Action::DecreaseOpinion => {
            let g = rival_ok(true)?;
            (state.level(g, target) > level::HOSTILE).then_some(()).ok_or("the rival's level is already 0")
        }
        Action::Discredit => {
            rival_ok(true)?;
            (mine > level::HOSTILE).then_some(()).ok_or("actor level 0")
        }
        Action::ExpelAdvisors => {
            rival_ok(true)?;
            (mine > level::NEUTRAL).then_some(()).ok_or("actor level not above neutral")
        }
        Action::BanEmbassy => {
            rival_ok(true)?;
            (mine > level::CORDIAL).then_some(()).ok_or("actor level not above cordial")
        }
    }
}

/// What an action took besides influence, for the caller to apply (infamy and prestige belong to
/// other modules).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ActionEffects {
    pub infamy: f64,
    pub prestige: f64,
}

/// Runs a checked action: pays the influence, applies the effect and the relation change.
#[allow(clippy::too_many_arguments)]
pub fn do_action(state: &mut DiplomacyState, rules: &DipRules, cal: &Calendar, today: Date, a: Action, actor: &str, target: &str, rival: Option<&str>) -> Result<ActionEffects, &'static str> {
    check_action(state, rules, a, actor, target, rival)?;
    let r = &rules.influence;
    let cost = r.cost(a);
    let mut out = ActionEffects::default();
    {
        let x = state.relation_mut(actor, target);
        x.influence = (x.influence - cost.influence).max(0.0);
    }
    match a {
        Action::IncreaseOpinion => state.relation_mut(actor, target).level += 1,
        Action::DecreaseOpinion => {
            let g = rival.unwrap_or_default();
            state.relation_mut(g, target).level = state.level(g, target).saturating_sub(1);
        }
        Action::AddToSphere => enter_sphere(state, actor, target),
        Action::RemoveFromSphere => {
            if state.sphere_leader(target) == Some(actor) {
                out = ActionEffects { infamy: r.removefromsphere_infamy, prestige: -r.removefromsphere_prestige };
            }
            leave_sphere(state, rules, target);
        }
        Action::Discredit => {
            let g = rival.unwrap_or_default().to_string();
            let until = plus_days(cal, today, r.discredit_days);
            let x = state.relation_mut(&g, target);
            x.discredited_until = until;
            x.discredited_by = Some(actor.to_string());
        }
        Action::ExpelAdvisors => {
            let g = rival.unwrap_or_default().to_string();
            state.relation_mut(&g, target).influence = 0.0;
            // The actor is lifted from a discredit the same rival put on it.
            let x = state.relation_mut(actor, target);
            if x.discredited_by.as_deref() == Some(g.as_str()) {
                x.discredited_until = None;
                x.discredited_by = None;
            }
        }
        Action::BanEmbassy => {
            let g = rival.unwrap_or_default().to_string();
            let until = plus_days(cal, today, r.ban_days);
            state.relation_mut(&g, target).banned_until = until;
        }
    }
    state.change_relation(rules, actor, target, cost.relation_on_accept);
    Ok(out)
}

// ---------------------------------------------------------------- the computer's choices

/// The AI's influence numbers (rules file `FLD_DIP_AI_INFLUENCE_*`).
#[derive(Debug, Clone, PartialEq)]
pub struct AiInfluenceRules {
    /// Days between two rounds of a great power's influence decisions (measured: a power's
    /// actions fall on dates 10 or 20 days apart).
    pub days: u64,
    /// Countries the AI puts a priority on (measured: 5 to 9 per power on 1 January).
    pub targets: usize,
    /// Its choice: gain factor + this if a neighbour + relation (0..the divisor) / the divisor
    /// (fitted to the original's picks of Jan and Feb 1836).
    pub neighbour: f64,
    pub relation_div: f64,
}

impl AiInfluenceRules {
    pub fn from_engine_rules(m: &EngineRules) -> Result<AiInfluenceRules, String> {
        Ok(AiInfluenceRules { days: crate::int_field(m, "FLD_DIP_AI_INFLUENCE_DAYS")?.max(1) as u64, targets: crate::int_field(m, "FLD_DIP_AI_INFLUENCE_TARGETS")?.max(0) as usize,
            neighbour: crate::num_field(m, "FLD_DIP_AI_INFLUENCE_NEIGHBOUR")?,
            relation_div: crate::num_field(m, "FLD_DIP_AI_INFLUENCE_RELATION")?,
        })
    }
}

/// Whether `power` takes its influence decisions on day number `day`: each power on its own
/// day of the cycle. STAND-IN: the original's day of a power in the cycle is not known; the
/// port spreads the powers by their tag.
pub fn ai_round(rules: &AiInfluenceRules, power: &str, day: u64) -> bool {
    let phase = power.bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b)));
    (day.wrapping_add(phase)).is_multiple_of(rules.days)
}

/// The AI's influence priorities: the `n` countries with the best score = gain factor +
/// neighbour bonus + relation share, skipping its own sphere members that no rival threatens (no
/// rival at friendly or above). All at priority 1 (measured: nearly all AI priorities are 1).
/// Fitted: the score's top 7 hold 49 of the 81 countries the original's AI picked on 1836.1.7
/// and 1836.2.9 (the gain factor alone: 39). STAND-IN for the original's own choice.
pub fn ai_priorities(state: &DiplomacyState, rules: &DipRules, power: &str, today: Date, n: usize, targets: &BTreeMap<String, TargetInfo>) -> Vec<(String, u8)> {
    let r = &rules.influence;
    let ai = &rules.ai_influence;
    let mut scored: Vec<(f64, &String)> = Vec::new();
    for (t, info) in targets {
        if t == power || !info.owns_land || state.is_great_power(t) || !takes_part(state, r, power, t, today) {
            continue;
        }
        if state.sphere_leader(t) == Some(power) && !state.great_powers().any(|g| g != power && state.level(g, t) >= level::FRIENDLY) {
            continue;
        }
        let mut gi = info.gain;
        gi.discredited = state.relation(power, t).and_then(|x| x.discredited_until).is_some_and(|d| today <= d);
        let f = gain_factor(r, &gi);
        if f > 0.0 {
            let score = f + if gi.neighbour { ai.neighbour } else { 0.0 } + gi.relation.clamp(0.0, ai.relation_div) / ai.relation_div;
            scored.push((score, t));
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(b.1)));
    scored.into_iter().take(n).map(|(_, t)| (t.clone(), 1)).collect()
}

/// One round of a great power's influence spending: at most one action per country, the first
/// that is allowed and paid for in this order (STAND-IN order; the costs are the install's): add
/// to the sphere, raise the opinion, take it out of a rival's sphere, discredit the rival with
/// the most influence among those that have a priority there. Measured: of the 34 discredits of
/// Jan-Feb 1836 every one hit a rival that had the country on its priority list (even rivals
/// holding under 1 point), and countries where no rival had a priority saw none. On its own
/// priority countries outside its sphere a power discredits only a rival that is ahead of it
/// in influence or level (33 of the 34 fit).
/// Returns what was done: (action, target, rival, effects for the caller).
pub fn ai_spend(state: &mut DiplomacyState, rules: &DipRules, cal: &Calendar, today: Date, power: &str) -> Vec<(Action, String, Option<String>, ActionEffects)> {
    let r = &rules.influence;
    let cheapest = Action::ALL.iter().map(|a| r.cost(*a).influence).fold(f64::INFINITY, f64::min);
    let targets: Vec<String> = state.relations.range((power.to_string(), String::new())..).take_while(|((a, _), _)| a == power).filter(|(_, x)| x.influence >= cheapest).map(|((_, b), _)| b.clone()).collect();
    let mut done = Vec::new();
    for t in targets {
        if state.is_great_power(&t) {
            continue;
        }
        let foreign_leader = state.sphere_leader(&t).is_some_and(|l| l != power);
        let rival = state
            .great_powers()
            .filter(|g| *g != power && state.influence(g, &t) > 0.0 && state.priorities.get(&(g.to_string(), t.clone())).is_some_and(|p| *p > 0))
            .filter(|g| state.relation(g, &t).and_then(|x| x.discredited_until).is_none_or(|d| today > d))
            .map(|g| (g.to_string(), state.influence(g, &t)))
            .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
            .map(|(g, _)| g);
        let mut tries: Vec<(Action, Option<String>)> = vec![(Action::AddToSphere, None), (Action::IncreaseOpinion, None)];
        if foreign_leader {
            tries.push((Action::RemoveFromSphere, None));
        }
        // On a country of its own priority list outside its sphere the power saves for the next
        // step unless the rival is ahead of it there (measured: France kept its 25 points in
        // Sardinia, where Spain held 0.02, and added it to its sphere two years later).
        let mine = (state.influence(power, &t), state.level(power, &t));
        let saving = state.priorities.get(&(power.to_string(), t.clone())).is_some_and(|p| *p > 0) && state.sphere_leader(&t) != Some(power);
        if let Some(g) = rival.filter(|g| !saving || state.influence(g, &t) >= mine.0 || state.level(g, &t) >= mine.1) {
            tries.push((Action::Discredit, Some(g)));
        }
        for (a, g) in tries {
            if let Ok(fx) = do_action(state, rules, cal, today, a, power, &t, g.as_deref()) {
                done.push((a, t.clone(), g, fx));
                break;
            }
        }
    }
    done
}
