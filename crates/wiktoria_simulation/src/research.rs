//! Technology research.
//!
//! Every day a civilized country that owns land has its research points of the day
//! (`CountryState::research_rate`, set monthly by `pops::country_month`) plus the points it has
//! stored. At most MAX_DAILY_RESEARCH of them, and no more than the technology still needs, go
//! into the technology it is researching; the rest stays stored. A technology costs its `cost`
//! from the file / (1 + the bonus of the country's technology school for the technology's
//! folder) x (1 - (this year - its `year`) / TECH_YEAR_SPAN) once its year has passed, never
//! less than 1. When the points reach the cost the country has the technology.
//!
//! With no technology chosen the points of the day are stored, up to 365 days' worth for a
//! civilized country and up to MAX_RESEARCH_POINTS for an uncivilized one (which never
//! researches).
//!
//! What a country may pick: a technology it does not have, whose `year` has come, and whose
//! predecessor in the same area (the one before it in the file) it has. Among those the computer
//! player picks at random, weighted by `ai::research_weight`: the technology's `ai_chance`
//! (`factor` times the factor of every modifier whose conditions hold) x (1 + school bonus)
//! x (1 + 0.5% for every year it is overdue) x (2 - its cost / the cost of the dearest
//! technology). The original rounds the weight; the port does not.
//!
//! Not included: the cheaper cost (TECH_FACTOR_VASSAL) for a vassal whose overlord has the
//! technology, what a technology unlocks besides its modifiers (buildings, units, goods), the
//! plurality a finished technology may add.
//!
//! A country played by a person (`CountryState::human`, D20) is never given a technology by the
//! computer: it researches what its player chose (`player::choose_research`) and stores its points
//! while nothing is chosen.

use std::collections::BTreeSet;

use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::modifiers::ModifierLibrary;
use wiktoria_data::script::Block;
use wiktoria_data::world::World;

use crate::ai::research_weight;
use crate::pops::product_weight;
use crate::script::{Scope, State};

#[derive(Debug, Clone, PartialEq)]
pub struct TechInfo {
    pub name: String,
    pub folder: String,
    pub year: i32,
    pub cost: f64,
    pub ai_chance: Option<Block>,
    /// The technology before this one in its area.
    pub previous: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResearchRules {
    pub year_span: f64,
    pub max_daily: f64,
    pub max_saved: f64,
    pub techs: Vec<TechInfo>,
    /// FLD_TECH_MIN_COST: a technology never costs less than this.
    pub min_cost: f64,
    /// FLD_RESEARCH_STORE_DAYS: a civilized country without a project stores at most so many
    /// days of research points.
    pub store_days: f64,
    /// The computer player's research weights (`ai::research_weight`).
    pub weights: crate::ai::ResearchWeights,
}

impl ResearchRules {
    pub fn new(m: &EngineRules, world: &World) -> Result<ResearchRules, String> {
        let d = |k: &str| world.defines.get("country", k).ok_or(format!("defines.country.{k} missing"));
        let mut techs: Vec<TechInfo> = Vec::new();
        for t in &world.defs.technologies {
            let previous = techs.iter().rev().find(|p| p.folder == t.folder && world.defs.technologies.iter().any(|x| x.name == p.name && x.area == t.area)).map(|p| p.name.clone());
            techs.push(TechInfo { name: t.name.clone(), folder: t.folder.clone(), year: t.year, cost: t.cost, ai_chance: t.ai_chance.clone(), previous });
        }
        Ok(ResearchRules {
            year_span: d("TECH_YEAR_SPAN")?,
            max_daily: world.defines.get("economy", "MAX_DAILY_RESEARCH").ok_or("defines.economy.MAX_DAILY_RESEARCH missing")?,
            max_saved: d("MAX_RESEARCH_POINTS")?,
            techs,
            min_cost: crate::num_field(m, "FLD_TECH_MIN_COST")?,
            store_days: crate::num_field(m, "FLD_RESEARCH_STORE_DAYS")?,
            weights: crate::ai::ResearchWeights::from_engine_rules(m)?,
        })
    }

    /// What the technology costs a country with this school bonus in this year.
    pub fn cost(&self, tech: &TechInfo, year: i32, school_bonus: f64) -> f64 {
        let age = ((year - tech.year) as f64 / self.year_span).clamp(0.0, 1.0);
        (tech.cost * (1.0 - age) / (1.0 + school_bonus).max(0.01)).max(self.min_cost)
    }
}

/// Whether a country may start this technology in this year: it does not have it, the
/// technology's year has come and it has the one before it in the same area.
pub fn may_research(c: &crate::script::CountryState, t: &TechInfo, year: i32) -> bool {
    t.year <= year && !c.technologies.contains(&t.name) && t.previous.as_ref().is_none_or(|p| c.technologies.contains(p))
}

/// One day of research for every country. Returns the technologies finished, as (country, name).
pub fn research_day(state: &mut State, rules: &ResearchRules, lib: &ModifierLibrary) -> Vec<(String, String)> {
    let year = state.date.year;
    let owners: BTreeSet<String> = state.provinces.values().filter_map(|p| p.owner.clone()).collect();
    let mut finished = Vec::new();
    for tag in owners {
        let Some(c) = state.countries.get(&tag) else { continue };
        let rate = c.research_rate.max(0.0);
        if !c.civilized {
            let saved = (c.research_points + rate).min(rules.max_saved);
            if let Some(c) = state.countries.get_mut(&tag) {
                c.research_points = saved;
            }
            continue;
        }
        let school_bonus = |folder: &str| c.tech_school.as_ref().and_then(|s| lib.schools.get(s)).and_then(|v| v.get(&format!("{folder}_research_bonus"))).copied().unwrap_or(0.0);
        // Pick a technology when none is being researched.
        let mut current = c.research.clone().filter(|(name, _)| !c.technologies.contains(name) && rules.techs.iter().any(|t| t.name == *name));
        if current.is_none() && !c.human {
            let scope = Scope::Country(tag.clone());
            // The dearest technology at this country's prices.
            let top_cost = rules.techs.iter().map(|t| rules.cost(t, year, school_bonus(&t.folder))).fold(0.0, f64::max);
            let open: Vec<(String, f64)> = rules
                .techs
                .iter()
                .filter(|t| may_research(c, t, year))
                .map(|t| {
                    let bonus = school_bonus(&t.folder);
                    (t.name.clone(), research_weight(&rules.weights, t.ai_chance.as_ref().map_or(1.0, |b| product_weight(state, b, &scope)), bonus, year - t.year, rules.cost(t, year, bonus), top_cost))
                })
                .filter(|(_, w)| *w > 0.0)
                .collect();
            let total: f64 = open.iter().map(|(_, w)| w).sum();
            if total > 0.0 {
                let mut pick = state.roll_fraction() * total;
                let chosen = open.iter().find(|(_, w)| { pick -= w; pick < 0.0 }).or(open.last()).map(|(n, _)| n.clone());
                current = chosen.map(|n| (n, 0.0));
            }
        }
        let Some(c) = state.countries.get(&tag) else { continue };
        let Some((name, progress)) = current else {
            // Nothing to research: the points are stored, up to a year's worth.
            let saved = (c.research_points + rate).min(rules.store_days * rate);
            if let Some(c) = state.countries.get_mut(&tag) {
                c.research = None;
                c.research_points = saved;
            }
            continue;
        };
        let school_bonus = |folder: &str| c.tech_school.as_ref().and_then(|s| lib.schools.get(s)).and_then(|v| v.get(&format!("{folder}_research_bonus"))).copied().unwrap_or(0.0);
        let Some(tech) = rules.techs.iter().find(|t| t.name == name) else { continue };
        let cost = rules.cost(tech, year, school_bonus(&tech.folder));
        // Today's points and the stored ones: at most the daily maximum and no more than is
        // still missing goes into the technology, the rest stays stored.
        let pool = rate + c.research_points;
        let spent = pool.min(rules.max_daily).min((cost - progress).max(0.0));
        let progress = progress + spent;
        let Some(c) = state.countries.get_mut(&tag) else { continue };
        c.research_points = pool - spent;
        if progress >= cost {
            c.technologies.insert(name.clone());
            c.research = None;
            finished.push((tag.clone(), name));
        } else {
            c.research = Some((name, progress));
        }
    }
    finished
}
