//! Reforms passed by the upper house.
//!
//! The ideologies file gives every ideology a weight for adding a political and a social reform:
//! a `base` plus the `factor` of every modifier whose conditions hold for the country (liberals
//! always want political reform; conservatives 0.1 for every step of militancy). The support for
//! a kind of reform is the sum over the upper house of share x weight. When it is above
//! FLD_REFORM_SUPPORT_NEEDED and the last reform is at least MIN_DELAY_BETWEEN_REFORMS months
//! back, a country passes one reform on the 1st of the month.
//!
//! Which one (the original's computer player; a person picks freely): social
//! reforms are looked at first, then political ones. The support of an option is the share of
//! the country's people asking for it (the pops' issue shares) plus those asking for options
//! further on in the same group, minus those asking for options behind the present one. Of the
//! options the country may enact now (one step when the group says `next_step_only`, any further
//! option otherwise; an option whose `allow` conditions fail is not taken) the best supported is
//! passed if its support is above AI_SUPPORT_REFORM (5%), or above UNREST_SUPPORT (0.1%) when
//! the country's average militancy is above UNREST_MILITANCY. The original's player looks every
//! 10 to 26 days; the port on the 1st of the month. The original doubles (support + movement
//! size) for an option a movement demands; the port has no movements.
//!
//! What passing does to every pop of the country:
//! * consciousness changes by CON_REFORM_IMPACT x the pop's share asking for the new option;
//! * militancy changes by MIL_REFORM_IMPACT x the sum over ideologies of the pop's share for
//!   the ideology x minus that ideology's weight for this kind of reform (the same weight the
//!   upper house test uses, but not cut off at 0): a liberal pop calms down after a political
//!   reform, a conservative one does not, a pop of an ideology against the reform gets angrier;
//! * then militancy falls by WRONG_REFORM_MILITANCY_IMPACT (in the original it RISES by that
//!   much for a pop in a movement for another option; the port has no movements, so it falls
//!   for everybody). Both are kept within 0..10.
//!
//! In the upper house the conservatives gain CONSERVATIVE_INCREASE_AFTER_REFORM and the house is
//! rescaled to add up to 1.
//!
//! Not included (D15): taking reforms back, reform movements (and their radicalism),
//! uncivilized countries' economic and military reforms.

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_data::script::{Block, Value};
use wiktoria_data::world::World;

use crate::script::{conditions_hold, Scope, State};

// rules file: FLD_AI_REFORM_UNREST_MILITANCY - average militancy above which the computer player passes a reform on almost no support (that the average is militancy is inferred).
const UNREST_MILITANCY: f64 = 4.0;
// rules file: FLD_AI_REFORM_UNREST_SUPPORT - the support needed then, in place of AI_SUPPORT_REFORM.
const UNREST_SUPPORT: f64 = 0.001;
// rules file: FLD_POP_MOOD_MAX - militancy and consciousness stay within 0..10.
const MOOD_MAX: f64 = 10.0;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReformGroup {
    pub name: String,
    /// `political_reforms` or `social_reforms`.
    pub political: bool,
    pub next_step_only: bool,
    /// Options in file order, each with its `allow` conditions.
    pub options: Vec<(String, Option<Block>)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReformRules {
    /// Months between two reforms of a country.
    pub min_delay: i32,
    /// Share of the people that must ask for a reform.
    pub ai_support: f64,
    /// Upper house support a reform needs (FLD_REFORM_SUPPORT_NEEDED).
    pub support_needed: f64,
    /// CON_REFORM_IMPACT and MIL_REFORM_IMPACT of the defines.
    pub con_impact: f64,
    pub mil_impact: f64,
    /// WRONG_REFORM_MILITANCY_IMPACT of the defines.
    pub wrong_impact: f64,
    /// CONSERVATIVE_INCREASE_AFTER_REFORM of the defines.
    pub conservative_increase: f64,
    pub groups: Vec<ReformGroup>,
    /// Ideology -> its `add_political_reform` and `add_social_reform` blocks.
    pub wants: BTreeMap<String, (Option<Block>, Option<Block>)>,
}

impl ReformRules {
    /// `issues` and `ideologies` are the roots of the two files.
    pub fn new(world: &World, m: &wiktoria_data::engine_rules::EngineRules, issues: &Block, ideologies: &Block) -> Result<ReformRules, String> {
        let support_needed = crate::num_field(m, "FLD_REFORM_SUPPORT_NEEDED")?;
        let d = |k: &str| world.defines.get("country", k).ok_or(format!("defines.country.{k} missing"));
        let mut rules = ReformRules { min_delay: d("MIN_DELAY_BETWEEN_REFORMS")? as i32, ai_support: d("AI_SUPPORT_REFORM")?, support_needed, con_impact: world.defines.get("pops", "CON_REFORM_IMPACT").ok_or("defines.pops.CON_REFORM_IMPACT missing")?, mil_impact: world.defines.get("pops", "MIL_REFORM_IMPACT").ok_or("defines.pops.MIL_REFORM_IMPACT missing")?, wrong_impact: d("WRONG_REFORM_MILITANCY_IMPACT")?, conservative_increase: d("CONSERVATIVE_INCREASE_AFTER_REFORM")?, ..Default::default() };
        for (category, groups, _) in issues.entries() {
            let political = category == "political_reforms";
            if !political && category != "social_reforms" {
                continue;
            }
            let Some(groups) = groups.as_block() else { continue };
            for (name, g, _) in groups.entries() {
                let Some(g) = g.as_block() else { continue };
                let options = g.entries().filter_map(|(k, v, _)| Some((k.to_string(), v.as_block()?.get("allow").and_then(Value::as_block).cloned()))).collect();
                rules.groups.push(ReformGroup { name: name.to_string(), political, next_step_only: g.get("next_step_only").and_then(Value::as_bool).unwrap_or(false), options });
            }
        }
        for (_, group, _) in ideologies.entries() {
            let Some(group) = group.as_block() else { continue };
            for (name, b, _) in group.entries() {
                let Some(b) = b.as_block() else { continue };
                let block = |k: &str| b.get(k).and_then(Value::as_block).cloned();
                rules.wants.insert(name.to_string(), (block("add_political_reform"), block("add_social_reform")));
            }
        }
        Ok(rules)
    }
}

/// `base` plus the factor of every modifier (loose or in a group) whose conditions hold.
fn weight(state: &State, block: &Block, scope: &Scope) -> f64 {
    let mut total = block.get("base").and_then(Value::as_f64).unwrap_or(0.0);
    for item in &block.items {
        match (item.key.as_deref(), item.value.as_block()) {
            (Some("modifier"), Some(m)) => {
                if conditions_hold(state, m, scope) == Ok(true) {
                    total += m.get("factor").and_then(Value::as_f64).unwrap_or(0.0);
                }
            }
            (Some("group"), Some(g)) => total += weight(state, g, scope) - g.get("base").and_then(Value::as_f64).unwrap_or(0.0),
            _ => {}
        }
    }
    total
}

/// The upper house's support for adding a political (true) or social reform, 0..1 and beyond.
pub fn house_support(state: &State, rules: &ReformRules, tag: &str, political: bool) -> f64 {
    let Some(c) = state.countries.get(tag) else { return 0.0 };
    let scope = Scope::Country(tag.to_string());
    c.upper_house
        .iter()
        .map(|(ideology, share)| {
            let block = rules.wants.get(ideology).and_then(|w| if political { w.0.as_ref() } else { w.1.as_ref() });
            share * block.map_or(0.0, |b| weight(state, b, &scope)).max(0.0)
        })
        .sum()
}

/// Share of the country's people asking for each issue option.
pub fn wanted(state: &State, tag: &str) -> BTreeMap<String, f64> {
    let (mut by_option, mut people): (BTreeMap<String, f64>, f64) = (BTreeMap::new(), 0.0);
    for p in state.provinces.values().filter(|p| p.owner.as_deref() == Some(tag)) {
        for q in &p.pops {
            people += q.size;
            for (n, share) in &q.issues {
                if let Some((option, _)) = n.checked_sub(1).and_then(|i| state.issues.get(i as usize)) {
                    *by_option.entry(option.clone()).or_default() += q.size * share / 100.0;
                }
            }
        }
    }
    if people > 0.0 {
        by_option.values_mut().for_each(|v| *v /= people);
    }
    by_option
}

/// Average militancy of the country's people.
fn average_militancy(state: &State, tag: &str) -> f64 {
    let (mut people, mut militancy) = (0.0, 0.0);
    for q in state.provinces.values().filter(|p| p.owner.as_deref() == Some(tag)).flat_map(|p| &p.pops) {
        people += q.size;
        militancy += q.size * q.militancy;
    }
    if people > 0.0 { militancy / people } else { 0.0 }
}

/// The reform of that kind the country's computer player would pass now: (group, option).
/// The upper house and the delay are tested by `reform_day`.
pub fn pick_reform(state: &State, rules: &ReformRules, tag: &str, political: bool) -> Option<(String, String)> {
    let c = state.countries.get(tag)?;
    let asked = wanted(state, tag);
    let ask = |option: &String| asked.get(option).copied().unwrap_or(0.0);
    let scope = Scope::Country(tag.to_string());
    let needed = if average_militancy(state, tag) > UNREST_MILITANCY { UNREST_SUPPORT } else { rules.ai_support };
    let mut best: Option<(f64, String, String)> = None;
    for g in rules.groups.iter().filter(|g| g.political == political) {
        let Some(now) = c.reforms.get(&g.name).and_then(|o| g.options.iter().position(|(n, _)| n == o)) else { continue };
        let behind: f64 = g.options[..now].iter().map(|(n, _)| ask(n)).sum();
        let last = if g.next_step_only { (now + 1).min(g.options.len() - 1) } else { g.options.len() - 1 };
        for i in now + 1..=last {
            let (option, allow) = &g.options[i];
            let support = g.options[i..].iter().map(|(n, _)| ask(n)).sum::<f64>() - behind;
            if support <= needed || allow.as_ref().is_some_and(|a| conditions_hold(state, a, &scope) != Ok(true)) {
                continue;
            }
            if best.as_ref().is_none_or(|b| support > b.0) {
                best = Some((support, g.name.clone(), option.clone()));
            }
        }
    }
    best.map(|(_, g, o)| (g, o))
}

/// The options of a reform group the country could pass now (read-only, for the interface): the
/// next one when the group says `next_step_only`, any further one otherwise, and only those whose
/// `allow` conditions hold. Support and the delay between reforms are not tested here.
pub fn open_options(state: &State, group: &ReformGroup, tag: &str) -> Vec<String> {
    let Some(c) = state.countries.get(tag) else { return Vec::new() };
    let Some(now) = c.reforms.get(&group.name).and_then(|o| group.options.iter().position(|(n, _)| n == o)) else { return Vec::new() };
    let last = if group.next_step_only { (now + 1).min(group.options.len() - 1) } else { group.options.len() - 1 };
    let scope = Scope::Country(tag.to_string());
    group.options[now + 1..=last.max(now)].iter().filter(|(_, allow)| allow.as_ref().is_none_or(|a| conditions_hold(state, a, &scope) == Ok(true))).map(|(n, _)| n.clone()).collect()
}

/// Reforms passed today, as (country, group, option). Only on the 1st of a month.
pub fn reform_day(state: &mut State, rules: &ReformRules) -> Vec<(String, String, String)> {
    let mut passed = Vec::new();
    if state.date.day != 1 {
        return passed;
    }
    let today = state.date;
    let months = |d: wiktoria_data::date::Date| d.year * 12 + i32::from(d.month);
    let owners: BTreeSet<String> = state.provinces.values().filter_map(|p| p.owner.clone()).collect();
    for tag in owners {
        let Some(c) = state.countries.get(&tag) else { continue };
        if !c.civilized || c.last_reform.is_some_and(|d| months(today) < months(d) + rules.min_delay) {
            continue;
        }
        // Social reforms first, then political ones.
        let choice = [false, true].into_iter().filter(|political| house_support(state, rules, &tag, *political) > rules.support_needed).find_map(|political| Some((political, pick_reform(state, rules, &tag, political)?)));
        let Some((political, (group, option))) = choice else { continue };
        // Each ideology's weight for this kind of reform, by its number in a pop's shares.
        let scope = Scope::Country(tag.clone());
        let factors: BTreeMap<u32, f64> = state.ideologies.iter().enumerate().filter_map(|(i, name)| {
            let block = rules.wants.get(name).and_then(|w| if political { w.0.as_ref() } else { w.1.as_ref() })?;
            Some((i as u32 + 1, weight(state, block, &scope)))
        }).collect();
        let conservative = state.ideologies.iter().any(|i| i == "conservative");
        let Some(c) = state.countries.get_mut(&tag) else { continue };
        c.reforms.insert(group.clone(), option.clone());
        c.last_reform = Some(today);
        if conservative || c.upper_house.contains_key("conservative") {
            *c.upper_house.entry("conservative".to_string()).or_default() += rules.conservative_increase;
            let all: f64 = c.upper_house.values().sum();
            if all > 0.0 {
                c.upper_house.values_mut().for_each(|v| *v /= all);
            }
        }
        let n = state.issues.iter().position(|(o, _)| *o == option).map(|i| i as u32 + 1);
        for q in state.provinces.values_mut().filter(|p| p.owner.as_deref() == Some(tag.as_str())).flat_map(|p| &mut p.pops) {
            let asked = n.and_then(|n| q.issues.get(&n)).copied().unwrap_or(0.0) / 100.0;
            q.consciousness = (q.consciousness + rules.con_impact * asked).clamp(0.0, MOOD_MAX);
            let against: f64 = q.ideology.iter().map(|(i, share)| share / 100.0 * -factors.get(i).copied().unwrap_or(0.0)).sum();
            q.militancy = (q.militancy + rules.mil_impact * against).clamp(0.0, MOOD_MAX);
            q.militancy = (q.militancy - rules.wrong_impact).clamp(0.0, MOOD_MAX);
        }
        passed.push((tag, group, option));
    }
    passed
}
