//! Pops joining and leaving political, social and independence movements, a month at a time (the
//! rules are in unrest.rs).
//!
//! Measured on the yearly saves of the original: none on
//! 1836.1.7, 5 movements on 1838.1.1, 27 on 1840.1.1, 49 on 1843.1.1; the members are almost all
//! in issue movements (censored press, landed voting), a handful in independence ones (NOR in
//! Sweden, PLC in Prussia, TRI in Turkey).
//! THE PORT'S STAND-INS: the step runs on the 1st of the month; an option may be asked for when it
//! is a political or social reform other than the one in force (the next one only, for groups that
//! move a step at a time) and its `allow` holds for the country; an independence movement
//! wants the first core of the pop's province whose country has the pop's culture as primary
//! culture and owns no land; nationalism is the save's; the suppression part of radicalism is 0.

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_data::script::{Block, Value};

use super::{Core, CoreLog, splitmix};
use crate::script::State;
use crate::unrest::{self, IssueOption, Movement, MovementDay, MovementKind, PopFacts, RadicalismInput, UnrestRules, UnrestState};

/// The reform options pops may ask for, by their number in `State::issues` (position + 1).
#[derive(Debug, Clone, Default)]
pub struct MovementRules {
    /// Option number -> (group, option, position in the group, next step only).
    options: BTreeMap<u32, (String, String, usize, bool)>,
    /// Option -> its `allow` conditions on the country (issues.txt).
    allow: BTreeMap<String, Block>,
    /// Culture -> its `radicalism` (cultures.txt).
    culture_radicalism: BTreeMap<String, f64>,
}

impl MovementRules {
    pub fn new(state: &State, groups: &[wiktoria_data::defs::IssueGroup], issues: &Block, cultures: &wiktoria_data::defs::Cultures) -> MovementRules {
        let mut options = BTreeMap::new();
        for (n, (option, group)) in state.issues.iter().enumerate() {
            let Some(g) = groups.iter().find(|g| g.name == *group && (g.category == "political_reforms" || g.category == "social_reforms")) else { continue };
            let Some(at) = g.options.iter().position(|o| o == option) else { continue };
            options.insert(n as u32 + 1, (group.clone(), option.clone(), at, g.next_step_only));
        }
        let culture_radicalism = cultures.cultures.iter().map(|c| (c.name.clone(), c.radicalism)).collect();
        let mut allow = BTreeMap::new();
        for (_, c, _) in issues.entries() {
            for (_, g, _) in c.as_block().into_iter().flat_map(|c| c.entries()) {
                for (option, o, _) in g.as_block().into_iter().flat_map(|g| g.entries()) {
                    if let Some(a) = o.as_block().and_then(|o| o.get("allow")).and_then(Value::as_block) {
                        allow.insert(option.to_string(), a.clone());
                    }
                }
            }
        }
        MovementRules { options, allow, culture_radicalism }
    }

    /// The options a pop of a country whose reforms are `current` may ask for, with its support.
    /// Measured: on 1840.1.1 144 of the 148 members of issue movements had the movement's option
    /// as their largest issue of all (party issues included), so only that one is offered.
    fn choices(&self, current: &BTreeMap<String, String>, groups: &BTreeMap<String, usize>, barred: &BTreeSet<String>, shares: &BTreeMap<u32, f64>) -> Vec<IssueOption> {
        let top = shares.iter().fold(None::<(&u32, &f64)>, |b, (n, x)| if b.is_none_or(|(_, y)| x > y) { Some((n, x)) } else { b });
        top.into_iter()
            .filter_map(|(n, x)| {
                let (group, option, at, step) = self.options.get(n)?;
                let now = groups.get(group).copied();
                let available = current.get(group) != Some(option) && (!step || now.is_none_or(|k| k.abs_diff(*at) == 1)) && !barred.contains(option);
                Some(IssueOption { name: option.clone(), support: *x, available })
            })
            .collect()
    }

    fn number_of(&self, option: &str) -> Option<u32> {
        self.options.iter().find(|(_, o)| o.1 == option).map(|(n, _)| *n)
    }
}

impl Core {
    /// Members whose support or militancy fell, or whose province changed hands, leave; free pops
    /// may join an issue movement, else an independence one; movements are recounted, their
    /// radicalism worked out, and the obsolete ones and those at the turning point go (the
    /// members of a movement that turns rebel get MIL_ON_REB_MOVE and find a faction after).
    pub(super) fn movement_month(&mut self, state: &mut State, rules: &UnrestRules, u: &mut UnrestState, log: &mut CoreLog) {
        let Some(mr) = self.movement_rules.take() else { return };
        let index = super::pop_index(state);
        let owner_of = |at: u16| state.provinces.get(&at).and_then(|p| p.owner.clone());
        let existing: std::collections::BTreeSet<String> = state.provinces.values().filter_map(|p| p.owner.clone()).collect();
        // Reform positions in force, per country.
        let positions: BTreeMap<String, BTreeMap<String, usize>> = state
            .countries
            .iter()
            .map(|(t, c)| (t.clone(), c.reforms.iter().filter_map(|(g, o)| Some((g.clone(), mr.options.values().find(|x| x.0 == *g && x.1 == *o)?.2))).collect()))
            .collect();
        let mut home: BTreeMap<u64, String> = BTreeMap::new();
        for (id, (at, _)) in &index {
            if let Some(o) = owner_of(*at) {
                home.insert(*id, o);
            }
        }
        // Options whose `allow` fails for the country.
        let barred: BTreeMap<String, BTreeSet<String>> = state
            .countries
            .keys()
            .filter(|t| existing.contains(t.as_str()))
            .map(|t| (t.clone(), mr.allow.iter().filter(|(_, a)| crate::script::conditions_hold(state, a, &crate::script::Scope::Country(t.clone())) != Ok(true)).map(|(o, _)| o.clone()).collect()))
            .collect();
        // Leaving.
        u.pop_movement.retain(|id, (kind, target)| {
            let Some(q) = index.get(id).and_then(|(p, i)| state.provinces.get(p)?.pops.get(*i)) else { return false };
            // A member whose province changed hands is counted in its new country's movement.
            let stays = match kind {
                MovementKind::Issue => mr.number_of(target).is_some_and(|n| unrest::stays_in_issue_movement(rules, q.issues.get(&n).copied().unwrap_or(0.0))),
                MovementKind::Independence => unrest::stays_in_independence_movement(rules, q.militancy),
            };
            if !stays {
                log.movement_leaves += 1;
            }
            stays
        });
        // Joining, in province order.
        let mut joins: Vec<(u64, MovementKind, String)> = Vec::new();
        for p in state.provinces.values() {
            let Some(owner) = p.owner.as_deref() else { continue };
            let Some(c) = state.countries.get(owner) else { continue };
            let primary = c.primary_culture.as_deref().unwrap_or("");
            for q in &p.pops {
                // Slaves join no movement (the owner, 2026-10-28; and on 1840.1.1 none of Brazil's 70
                // slave pops whose largest issue is no_slavery is in one).
                if u.pop_movement.contains_key(&q.id) || u.pop_faction.contains_key(&q.id) || state.slave_types.contains(&q.pop_type) {
                    continue;
                }
                let facts = PopFacts { id: q.id, size: q.size, literacy: q.literacy, consciousness: q.consciousness, militancy: q.militancy, culture: q.culture.clone(), ..PopFacts::default() };
                if unrest::blocked_from_movements(rules, &facts) {
                    continue;
                }
                let options = mr.choices(&c.reforms, positions.get(owner).unwrap_or(&BTreeMap::new()), barred.get(owner).unwrap_or(&BTreeSet::new()), &q.issues);
                if let Some((o, n)) = unrest::issue_choice(rules, &facts, &options)
                    && unrest::joins_issue(rules, n, splitmix(&mut self.rng) as u32)
                {
                    joins.push((q.id, MovementKind::Issue, o.name.clone()));
                    continue;
                }
                if unrest::joins_independence(rules, &facts, primary, splitmix(&mut self.rng) as u32)
                    && let Some(tag) = p.cores.iter().find(|t| t.as_str() != owner && !existing.contains(t.as_str()) && state.countries.get(t.as_str()).and_then(|c| c.primary_culture.as_deref()) == Some(q.culture.as_str()))
                {
                    joins.push((q.id, MovementKind::Independence, tag.clone()));
                }
            }
        }
        log.movement_joins += joins.len();
        for (id, kind, target) in joins {
            u.pop_movement.insert(id, (kind, target));
        }
        // Movements recounted from their members.
        let mut population: BTreeMap<&str, f64> = BTreeMap::new();
        for p in state.provinces.values() {
            if let Some(o) = p.owner.as_deref() {
                *population.entry(o).or_default() += p.pops.iter().map(|q| q.size).sum::<f64>();
            }
        }
        let mut support: BTreeMap<(String, MovementKind, String), (f64, Vec<u64>)> = BTreeMap::new();
        for (id, (kind, target)) in &u.pop_movement {
            let (Some(o), Some(q)) = (home.get(id), index.get(id).and_then(|(p, i)| state.provinces.get(p)?.pops.get(*i))) else { continue };
            let e = support.entry((o.clone(), *kind, target.clone())).or_default();
            e.0 += q.size;
            e.1.push(*id);
        }
        let mut movements = Vec::new();
        let mut rebels: Vec<u64> = Vec::new();
        let mut dropped: Vec<u64> = Vec::new();
        for ((country, kind, target), (size, members)) in support {
            let c = state.countries.get(&country);
            let independence = (kind == MovementKind::Independence).then(|| {
                let culture = state.countries.get(&target).and_then(|c| c.primary_culture.clone()).unwrap_or_default();
                let nationalism = u.provinces.values().filter(|(_, _, cores)| cores.contains(&target)).map(|(_, n, _)| *n).fold(0.0, f64::max);
                (mr.culture_radicalism.get(&culture).copied().unwrap_or(0.0), nationalism)
            });
            let radicalism = unrest::radicalism(rules, &RadicalismInput { suppression_part: 0.0, reform_steps_passed: u.reform_steps.get(&country).copied().unwrap_or(0.0), size, country_population: population.get(country.as_str()).copied().unwrap_or(0.0), independence });
            let obsolete = match kind {
                // In force, or no longer allowed (the_slavery_debate bars no_slavery in the USA).
                MovementKind::Issue => c.is_some_and(|c| c.reforms.values().any(|o| *o == target)) || barred.get(&country).is_some_and(|b| b.contains(&target)),
                MovementKind::Independence => existing.contains(&target),
            };
            match unrest::movement_day(rules, radicalism, obsolete) {
                MovementDay::Keep => movements.push(Movement { country, kind, target, support: size, radicalism }),
                MovementDay::Dissolve => dropped.extend(members),
                MovementDay::TurnRebel => {
                    log.movements_turned += 1;
                    rebels.extend(members);
                }
            }
        }
        for id in dropped.iter().chain(&rebels) {
            u.pop_movement.remove(id);
        }
        for id in rebels {
            if let Some((p, i)) = index.get(&id)
                && let Some(q) = state.provinces.get_mut(p).and_then(|p| p.pops.get_mut(*i))
            {
                q.militancy = unrest::turn_rebel_militancy(rules, q.militancy);
            }
        }
        u.movements = movements;
        self.movement_rules = Some(mr);
    }
}
