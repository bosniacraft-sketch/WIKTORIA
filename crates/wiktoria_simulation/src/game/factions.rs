//! Pops joining and leaving rebel factions, a month at a time (the rules are in unrest.rs).
//!
//! Measured on the yearly saves of the original (1836-1843, docs/FIDELITY-UNREST.md, "factions"):
//! 4 factions with 38 pops on 1836.1.7, 6 with 70 on 1837.1.1, 9 on 1840.1.1, 13 on 1843.1.1.
//! Nationalist factions split by the country they would create (UCA -> HON, NIC, COS, ...): the
//! pop wants a country of its own culture with a core on its province.
//! THE PORT'S STAND-INS: the step runs on the 1st of the month (the original's day per pop is not
//! known); `other_land_area` is never set; a pop creates the first such country in tag order.

use std::collections::BTreeMap;

use super::{Core, CoreLog};
use crate::cond::{self, Weight};
use crate::script::{Scope, State};
use crate::unrest::{self, Aim, Faction, PopFacts, RebelType, UnrestRules, UnrestState};

/// The rebel types with their spawn chances compiled, and the culture facts the rules ask.
#[derive(Debug, Clone, Default)]
pub struct FactionRules {
    pub types: Vec<RebelType>,
    weights: Vec<Option<Weight>>,
    /// Culture -> its group's name, and the group's union tag if it has one.
    group: BTreeMap<String, String>,
    union: BTreeMap<String, String>,
}

impl FactionRules {
    pub fn new(types: Vec<RebelType>, cultures: &wiktoria_data::defs::Cultures) -> FactionRules {
        let weights = types.iter().map(|t| t.spawn_chance.as_ref().map(Weight::new)).collect();
        let mut group = BTreeMap::new();
        let mut union = BTreeMap::new();
        for c in &cultures.cultures {
            if let Some(g) = cultures.groups.get(c.group as usize) {
                group.insert(c.name.clone(), g.name.clone());
                if let Some(u) = &g.union {
                    union.insert(c.name.clone(), u.clone());
                }
            }
        }
        FactionRules { types, weights, group, union }
    }

    fn kind(&self, name: &str) -> Option<&RebelType> {
        self.types.iter().find(|t| t.name == name)
    }
}

/// The ideology with the largest share of a pop (ties to the first).
fn top_ideology(state: &State, shares: &BTreeMap<u32, f64>) -> Option<String> {
    let mut best: Option<(u32, f64)> = None;
    for (n, x) in shares {
        if best.is_none_or(|(_, b)| *x > b) {
            best = Some((*n, *x));
        }
    }
    best.and_then(|(n, _)| state.ideologies.get((n as usize).wrapping_sub(1)).cloned())
}

/// The country a pop of `culture` in province `at` would create: a tag other than the owner whose
/// primary culture is the pop's and that holds a core there.
fn country_to_create(state: &State, culture: &str, at: u16, owner: &str) -> Option<String> {
    let p = state.provinces.get(&at)?;
    p.cores.iter().filter(|t| t.as_str() != owner).find(|t| state.countries.get(t.as_str()).and_then(|c| c.primary_culture.as_deref()) == Some(culture)).cloned()
}

impl Core {
    /// Members whose militancy fell below the line, or whose province changed hands, leave;
    /// pops above it that are in no faction or movement pick a type and join a faction that
    /// takes them or found one; factions that are finished go.
    pub(super) fn faction_month(&self, state: &mut State, rules: &UnrestRules, u: &mut UnrestState, log: &mut CoreLog) {
        let Some(fr) = self.factions.as_ref() else { return };
        // The spawn chances name the pop's country THIS.
        let candidates: Vec<(u16, usize)> = state.provinces.iter().filter(|(_, p)| p.owner.is_some()).flat_map(|(at, p)| p.pops.iter().enumerate().filter(|(_, q)| q.militancy > rules.mil_to_join_rebel).map(move |(i, _)| (*at, i))).collect();
        let this = state.this.take();
        let mut chances: BTreeMap<(u16, usize), Vec<f64>> = BTreeMap::new();
        for (at, i) in candidates {
            let scope = Scope::Pop(at, i);
            state.this = Some(scope.clone());
            chances.insert((at, i), fr.weights.iter().map(|w| w.as_ref().map_or(0.0, |w| cond::product_lenient(state, None, w, &scope))).collect());
        }
        state.this = this;
        let state = &*state;
        let index = super::pop_index(state);
        let owner_of = |at: u16| state.provinces.get(&at).and_then(|p| p.owner.clone());
        let pop = |id: u64| index.get(&id).and_then(|(p, i)| state.provinces.get(p).and_then(|q| q.pops.get(*i)));
        // Leaving.
        for f in &mut u.factions {
            f.pops.retain(|id| {
                let stays = index.get(id).is_some_and(|(at, _)| owner_of(*at).as_deref() == Some(f.country.as_str())) && pop(*id).is_some_and(|q| q.militancy >= rules.mil_to_join_rebel);
                if !stays {
                    u.pop_faction.remove(id);
                    log.faction_leaves += 1;
                }
                stays
            });
        }
        // Joining, in province order.
        let mut next_id = u.factions.iter().map(|f| f.id).max().unwrap_or(0) + 1;
        for (at, p) in &state.provinces {
            let Some(owner) = p.owner.as_deref() else { continue };
            for (i, q) in p.pops.iter().enumerate() {
                if q.militancy <= rules.mil_to_join_rebel {
                    continue;
                }
                let facts = PopFacts {
                    id: q.id,
                    size: q.size,
                    literacy: q.literacy,
                    consciousness: q.consciousness,
                    militancy: q.militancy,
                    culture: q.culture.clone(),
                    culture_group: fr.group.get(&q.culture).cloned().unwrap_or_default(),
                    religion: q.religion.clone(),
                    top_ideology: top_ideology(state, &q.ideology),
                    in_movement: u.pop_movement.contains_key(&q.id),
                    in_faction: u.pop_faction.contains_key(&q.id),
                    other_land_area: false,
                };
                if !unrest::may_join_faction(rules, &facts) {
                    continue;
                }
                let Some(chance) = chances.get(&(*at, i)) else { continue };
                let creates_for = |t: &RebelType| -> Option<String> {
                    match t.independence {
                        Aim::Culture => country_to_create(state, &q.culture, *at, owner),
                        Aim::PanNationalist => fr.union.get(&q.culture).filter(|x| x.as_str() != owner).cloned(),
                        _ => None,
                    }
                };
                let value = |t: &RebelType| fr.types.iter().position(|x| x.name == t.name).map_or(0.0, |k| chance[k]);
                let Some(t) = unrest::pick_rebel_type(&fr.types, &facts, &value, &|t| creates_for(t).is_some()) else { continue };
                let creates = creates_for(t);
                let brigades = unrest::brigades(rules, q.size);
                let take = u.factions.iter_mut().find(|f| f.country == owner && unrest::faction_accepts(t, f, &facts, creates.as_deref()));
                let fid = match take {
                    Some(f) => {
                        let old: u32 = f.pops.iter().filter_map(|id| pop(*id)).map(|q| unrest::brigades(rules, q.size)).sum();
                        f.organization = unrest::organisation_after_join(f.organization, old, brigades);
                        f.pops.push(q.id);
                        f.id
                    }
                    None => {
                        let government = state.countries.get(owner).and_then(|c| c.government.as_deref()).and_then(|g| t.government.get(g)).cloned().unwrap_or_default();
                        let id = next_id;
                        next_id += 1;
                        u.factions.push(Faction { id, kind: t.name.clone(), country: owner.to_string(), creates, culture: q.culture.clone(), culture_group: facts.culture_group.clone(), religion: q.religion.clone(), government, province: u32::from(*at), pops: vec![q.id], ..Faction::default() });
                        log.factions_founded += 1;
                        id
                    }
                };
                u.pop_faction.insert(q.id, fid);
                log.faction_joins += 1;
            }
        }
        // Finished factions.
        let owns = |tag: &str| state.provinces.values().any(|p| p.owner.as_deref() == Some(tag));
        u.factions.retain(|f| {
            let Some(t) = fr.kind(&f.kind) else { return !f.pops.is_empty() };
            let wanted_exists = f.creates.as_deref().is_some_and(owns);
            let union_own = fr.union.get(&f.culture).is_some_and(|x| *x == f.country);
            let finished = unrest::faction_finished(t, f, owns(&f.country), wanted_exists, f.creates.is_some(), union_own);
            if finished {
                for id in &f.pops {
                    u.pop_faction.remove(id);
                }
            }
            !finished
        });
    }
}

/// A faction that won: what the game still has to run on the country (its effect block is
/// script, run by `Game::day` with the effect runner).
#[derive(Debug, Clone, PartialEq)]
pub struct RebelWin {
    pub country: String,
    pub kind: String,
    pub effect: Option<wiktoria_data::script::Block>,
}

impl Core {
    /// Every day: a faction whose type's `demands_enforced_trigger` holds for its country while
    /// its own regiments stand in the field wins (the faction with the most regiments when
    /// several could). Its armies go, the provinces rebels hold in the country return to the
    /// owner, the government becomes the type's choice, prestige takes the break hit, and its
    /// effect is handed back to run. NOT WIRED: `unit_transfer` and the country a nationalist
    /// win creates (no country is released), `break_alliance_on_win`.
    pub(super) fn rebel_win_day(&mut self, state: &mut State, log: &mut CoreLog) {
        let (Some(fr), Some((rules, u))) = (self.factions.as_ref(), self.unrest.as_mut()) else { return };
        let rebel = self.day.rebel_tag.clone();
        // Rebel regiments per faction, from the pop each was raised from.
        let mut field: BTreeMap<u64, usize> = BTreeMap::new();
        for r in self.mil.armies_of(&rebel).flat_map(|a| &a.regiments) {
            if let Some(f) = r.pop.and_then(|p| u.pop_faction.get(&p)) {
                *field.entry(*f).or_default() += 1;
            }
        }
        let mut winners: BTreeMap<String, (usize, u64)> = BTreeMap::new();
        for f in &u.factions {
            let Some(n) = field.get(&f.id).copied() else { continue };
            let Some(trigger) = fr.kind(&f.kind).and_then(|t| t.demands_enforced_trigger.as_ref()) else { continue };
            if crate::script::conditions_hold(state, trigger, &Scope::Country(f.country.clone())) != Ok(true) {
                continue;
            }
            let e = winners.entry(f.country.clone()).or_insert((0, f.id));
            if n > e.0 {
                *e = (n, f.id);
            }
        }
        if winners.is_empty() {
            return;
        }
        let rules = rules.clone();
        for (country, (_, fid)) in winners {
            let Some(at) = u.factions.iter().position(|f| f.id == fid) else { continue };
            let f = u.factions.remove(at);
            let Some(t) = fr.kind(&f.kind) else { continue };
            let current = state.countries.get(&country).and_then(|c| c.government.clone()).unwrap_or_default();
            let d = unrest::demands_enforced(&rules, t, &f, &current, &wiktoria_data::defs::Cultures::default());
            if let Some(c) = state.countries.get_mut(&country) {
                if let Some(g) = d.government {
                    c.government = Some(g);
                }
                c.prestige = unrest::prestige_after_break(&rules, c.prestige, 0.0);
            }
            for id in &f.pops {
                u.pop_faction.remove(id);
            }
            let members: std::collections::BTreeSet<u64> = f.pops.iter().copied().collect();
            self.mil.armies.retain(|a| a.owner != rebel || !a.regiments.iter().any(|r| r.pop.is_some_and(|p| members.contains(&p))));
            for (id, p) in self.mil.provinces.iter_mut() {
                if p.owner.as_deref() == Some(country.as_str()) && p.controller.as_deref() == Some(rebel.as_str()) {
                    p.controller = p.owner.clone();
                    p.last_controller_change = Some(state.date);
                    if let Some(q) = state.provinces.get_mut(id) {
                        q.controller = q.owner.clone();
                        q.controller_since = Some(state.date);
                    }
                }
            }
            log.rebel_wins.push(format!("{} in {country}", f.kind));
            self.rebel_wins.push(RebelWin { country, kind: f.kind, effect: d.effect.cloned() });
        }
    }
}
