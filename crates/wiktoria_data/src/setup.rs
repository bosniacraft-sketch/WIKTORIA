//! Game setup: the structural changes the original game makes to the history pops before day 1.
//!
//! Each step is switched on by a boolean field in the rules file. Everything here is deterministic. The game additionally runs one random pop update
//! (growth, assimilation, conversion, migration, promotion) during setup;
//! that belongs to the daily simulation and is not part of this pass.
//!
//! Steps, in order:
//! 1. `FLD_SETUP_RGO_WORKER_TYPE`: a pop whose type has an `equivalent` becomes that type when the
//!    province's RGO employs the equivalent and not the pop's own type.
//! 2. `FLD_SETUP_SLAVE_CONVERSION`: in an owned province, slave pops become the RGO's first
//!    non-slave worker type unless the owner is civilized and its reforms allow slavery.
//! 3. `FLD_SETUP_STATE_CAPITAL_ONLY`: pops of `state_capital_only` types move to their state's
//!    capital: the first province of the region, in region-file order, held by the same owner.
//! 4. `FLD_SETUP_MERGE_IDENTICAL`: pops of the same type, culture and religion in one province merge,
//!    in file order, as long as the result does not exceed the type's `merge_max_size`.

use std::collections::HashMap;

use crate::map::ProvinceId;
use crate::engine_rules::EngineRules;
use crate::world::{CountryIdx, Pop, World};

/// Which steps to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupSteps {
    pub rgo_worker_type: bool,
    pub slave_conversion: bool,
    pub state_capital_only: bool,
    pub merge_identical: bool,
}

impl SetupSteps {
    pub const ALL: SetupSteps = SetupSteps { rgo_worker_type: true, slave_conversion: true, state_capital_only: true, merge_identical: true };
    pub const NONE: SetupSteps = SetupSteps { rgo_worker_type: false, slave_conversion: false, state_capital_only: false, merge_identical: false };

    /// Reads the step switches from the rules file. A missing switch is an error, so a rules file
    /// without them cannot silently skip setup.
    pub fn from_engine_rules(m: &EngineRules) -> Result<SetupSteps, String> {
        let flag = |id: &str| m.field(id).and_then(|v| v.as_bool()).ok_or(format!("rules field {id} missing or not a boolean"));
        Ok(SetupSteps {
            rgo_worker_type: flag("FLD_SETUP_RGO_WORKER_TYPE")?,
            slave_conversion: flag("FLD_SETUP_SLAVE_CONVERSION")?,
            state_capital_only: flag("FLD_SETUP_STATE_CAPITAL_ONLY")?,
            merge_identical: flag("FLD_SETUP_MERGE_IDENTICAL")?,
        })
    }
}

/// Starting literacy of pops, measured on 19,966 of 19,967 pops of a day-1 save:
/// a pop whose culture is its owner's primary culture or one of its accepted cultures starts
/// with the country's history `literacy`; every other pop with the country's
/// `non_state_culture_literacy`, or FLD_SETUP_NON_STATE_LITERACY when history gives none. Pop
/// type plays no part. Values are kept within 0..FLD_POP_LITERACY_MAX. Pops in unowned
/// provinces are left alone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiteracyRules {
    pub enabled: bool,
    pub non_state_default: f64,
    pub max: f64,
}

impl LiteracyRules {
    pub fn from_engine_rules(m: &EngineRules) -> Result<LiteracyRules, String> {
        let num = |id: &str| m.field(id).and_then(|v| v.as_f64()).ok_or(format!("rules field {id} missing or not a number"));
        Ok(LiteracyRules {
            enabled: m.field("FLD_SETUP_LITERACY").and_then(|v| v.as_bool()).ok_or("rules field FLD_SETUP_LITERACY missing or not a boolean")?,
            non_state_default: num("FLD_SETUP_NON_STATE_LITERACY")?,
            max: num("FLD_POP_LITERACY_MAX")?,
        })
    }
}

/// Gives every pop in an owned province its starting literacy. Returns how many pops were set.
pub fn starting_literacy(world: &mut World, rules: &LiteracyRules) -> usize {
    if !rules.enabled {
        return 0;
    }
    // Per country: (state-culture literacy, other literacy, indexes of its state cultures).
    let per_country: Vec<(f64, f64, Vec<u16>)> = world
        .countries
        .iter()
        .map(|c| {
            let h = &c.history;
            let cultures = h.primary_culture.iter().chain(h.cultures.iter()).filter_map(|n| world.defs.cultures.index.get(n)).collect();
            (h.literacy.unwrap_or(0.0), h.non_state_culture_literacy.unwrap_or(rules.non_state_default), cultures)
        })
        .collect();
    let mut set = 0;
    for p in world.provinces.iter_mut().flatten() {
        let Some((state, other, cultures)) = p.owner.and_then(|o| per_country.get(o as usize)) else { continue };
        for pop in &mut p.pops {
            pop.literacy = if cultures.contains(&pop.culture) { *state } else { *other }.clamp(0.0, rules.max);
            set += 1;
        }
    }
    set
}

/// What the pass changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SetupReport {
    pub type_changes: usize,
    pub slaves_converted: usize,
    pub slaves_kept: usize,
    pub moved_to_capital: usize,
    pub merged: usize,
    pub pops_before: usize,
    pub pops_after: usize,
    pub warnings: Vec<String>,
}

/// Value of a reform rule (such as `slavery_allowed`) for a country: the rule of the chosen option
/// of every reform group, falling back to the group's first option when history sets none.
pub fn country_rule(world: &World, country: CountryIdx, rule: &str) -> Option<bool> {
    let reforms = &world.countries[country as usize].history.reforms;
    let mut value = None;
    for g in &world.defs.issues {
        let i = match reforms.get(&g.name) {
            Some(opt) => g.options.iter().position(|o| o == opt),
            None if !g.options.is_empty() => Some(0),
            None => None,
        };
        if let Some(v) = i.and_then(|i| g.rules.get(i)).and_then(|r| r.get(rule)) {
            value = Some(*v);
        }
    }
    value
}

pub fn apply(world: &mut World, steps: SetupSteps) -> SetupReport {
    let mut r = SetupReport { pops_before: count_pops(world), ..Default::default() };
    if steps.rgo_worker_type {
        rgo_worker_type(world, &mut r);
    }
    if steps.slave_conversion {
        slave_conversion(world, &mut r);
    }
    if steps.state_capital_only {
        state_capital_only(world, &mut r);
    }
    if steps.merge_identical {
        merge_identical(world, &mut r);
    }
    r.pops_after = count_pops(world);
    r
}

fn count_pops(world: &World) -> usize {
    world.provinces.iter().flatten().map(|p| p.pops.len()).sum()
}

/// Pop types the province's RGO employs, if its trade good is an RGO good.
fn rgo_workers<'a>(world: &'a World, goods: Option<&String>) -> Option<&'a [u16]> {
    world.defs.rgo_employees.get(goods?).map(Vec::as_slice)
}

fn rgo_worker_type(world: &mut World, r: &mut SetupReport) {
    let equivalent: Vec<Option<u16>> = world
        .defs
        .pop_types
        .iter()
        .map(|t| t.equivalent.as_deref().and_then(|e| world.defs.pop_type_index.get(e)))
        .collect();
    let employees = world.defs.rgo_employees.clone();
    for p in world.provinces.iter_mut().flatten() {
        let Some(workers) = p.trade_goods.as_ref().and_then(|g| employees.get(g)) else { continue };
        for pop in &mut p.pops {
            if let Some(e) = equivalent[pop.pop_type as usize]
                && !workers.contains(&pop.pop_type)
                && workers.contains(&e)
            {
                pop.pop_type = e;
                r.type_changes += 1;
            }
        }
    }
}

fn slave_conversion(world: &mut World, r: &mut SetupReport) {
    let is_slave: Vec<bool> = world.defs.pop_types.iter().map(|t| t.is_slave).collect();
    let allowed: Vec<bool> = (0..world.countries.len())
        .map(|c| {
            let civilized = world.countries[c].history.civilized.unwrap_or(false);
            civilized && country_rule(world, c as CountryIdx, "slavery_allowed").unwrap_or(false)
        })
        .collect();
    for i in 0..world.provinces.len() {
        let Some(p) = &world.provinces[i] else { continue };
        let Some(owner) = p.owner else {
            r.slaves_kept += p.pops.iter().filter(|x| is_slave[x.pop_type as usize]).count();
            continue;
        };
        let target = rgo_workers(world, p.trade_goods.as_ref()).and_then(|w| w.iter().copied().find(|t| !is_slave[*t as usize]));
        let (id, goods) = (p.id, p.trade_goods.clone());
        let p = world.provinces[i].as_mut().expect("checked above");
        for pop in &mut p.pops {
            if !is_slave[pop.pop_type as usize] {
                continue;
            }
            if allowed[owner as usize] {
                r.slaves_kept += 1;
            } else if let Some(t) = target {
                pop.pop_type = t;
                r.slaves_converted += 1;
            } else {
                r.warnings.push(format!("province {id}: slaves not converted, no RGO worker type for {goods:?}"));
            }
        }
    }
}

/// State capital of every (region, owner) pair: the first province of the region held by the owner.
pub fn state_capitals(world: &World) -> HashMap<(u16, CountryIdx), ProvinceId> {
    let mut caps = HashMap::new();
    for (ri, region) in world.regions.iter().enumerate() {
        for &id in &region.provinces {
            let Some(Some(p)) = world.provinces.get(id as usize) else { continue };
            // A province listed in several regions belongs to the first (see World::load).
            if p.region != Some(ri as u16) {
                continue;
            }
            if let Some(owner) = p.owner {
                caps.entry((ri as u16, owner)).or_insert(id);
            }
        }
    }
    caps
}

fn state_capital_only(world: &mut World, r: &mut SetupReport) {
    let only: Vec<bool> = world.defs.pop_types.iter().map(|t| t.state_capital_only).collect();
    let caps = state_capitals(world);
    let mut moving: Vec<(ProvinceId, Pop)> = Vec::new();
    for p in world.provinces.iter_mut().flatten() {
        let (Some(region), Some(owner)) = (p.region, p.owner) else { continue };
        let Some(&cap) = caps.get(&(region, owner)) else { continue };
        if cap == p.id {
            continue;
        }
        let (go, stay): (Vec<Pop>, Vec<Pop>) = std::mem::take(&mut p.pops).into_iter().partition(|x| only[x.pop_type as usize]);
        p.pops = stay;
        moving.extend(go.into_iter().map(|x| (cap, x)));
    }
    r.moved_to_capital = moving.len();
    for (cap, pop) in moving {
        if let Some(Some(p)) = world.provinces.get_mut(cap as usize) {
            p.pops.push(pop);
        }
    }
}

fn merge_identical(world: &mut World, r: &mut SetupReport) {
    let cap: Vec<Option<f64>> = world.defs.pop_types.iter().map(|t| t.merge_max_size).collect();
    for p in world.provinces.iter_mut().flatten() {
        let mut out: Vec<Pop> = Vec::with_capacity(p.pops.len());
        for pop in std::mem::take(&mut p.pops) {
            let limit = cap[pop.pop_type as usize];
            let into = out.iter_mut().find(|o| {
                (o.pop_type, o.culture, o.religion) == (pop.pop_type, pop.culture, pop.religion)
                    && limit.is_none_or(|l| o.size + pop.size <= l)
            });
            match into {
                Some(o) => {
                    o.size += pop.size;
                    r.merged += 1;
                }
                None => out.push(pop),
            }
        }
        p.pops = out;
    }
}
