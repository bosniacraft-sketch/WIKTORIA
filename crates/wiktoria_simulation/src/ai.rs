//! The computer player at home: where it puts its national focuses, and how it weighs the
//! technologies it may research (rules from the original game's observed behaviour, `ai_home`).
//!
//! National focus, once a month, for every country that is not played by a person:
//! * every province the country owns gets a score for every focus kind ([`focus_score`]);
//! * a region offers ONE candidate: the focus whose positive province scores, added up and divided
//!   by the number of ALL provinces of the region (also other owners'), is highest and above 0;
//! * the candidates are sorted, best first; the best N (N = the country's focus points) that are
//!   accepted (the score of the region's first own province alone is above 0) must carry exactly
//!   their focus;
//! * a region among them that carries another focus, or none, gets the new one; when no point is
//!   free the country's weakest other focus (scored on the first province of its region) is
//!   cancelled first;
//! * nothing protects a standing focus (no minimum time, no bonus for the one in place), but one
//!   is only cancelled to free a point for a better region.
//!
//! Stand-ins of the port (said in the report too):
//! * the number of focus points of a country is the number of focuses it has in the save the run
//!   starts from (the port does not work out the real number);
//! * who works in which factory: the state's factory workers are shared over its running
//!   factories by their size, as the port's economy does it;
//! * a focus's own `limit` conditions and the colonising, flashpoint and party loyalty focuses
//!   are not looked at: the original's computer player only ever sets promotion and railroad
//!   focuses on its own land.

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::script::Block;
use wiktoria_data::world::World;

use crate::script::State;

/// The rules file's numbers of the focus scores.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FocusNumbers {
    /// FLD_AI_FOCUS_NEVER: the score that means "never".
    pub never: f64,
    /// FLD_AI_FOCUS_CAPITAL_FACTOR / FLD_AI_FOCUS_CAPITAL_SHARE: the bureaucrat score is
    /// multiplied by the factor in the capital while its (boosted) bureaucrat share is under the share.
    pub capital_factor: f64,
    pub capital_share: f64,
    /// FLD_AI_CAPITALIST_FOCUS_MIN_FIGURE: a province figure under this rules the capitalist focus
    /// out (what the figure measures is unknown).
    pub capitalist_min_figure: f64,
    /// FLD_AI_RAIL_FOCUS_BASE / FLD_AI_RAIL_FOCUS_FACTORIES_PER_POINT: railroad score = base +
    /// factories of the state / per point, rounded down.
    pub rail_base: f64,
    pub rail_factories_per_point: f64,
    /// FLD_AI_SOLDIER_TARGET_ARMY_SCHOOL / FLD_AI_SOLDIER_TARGET_CULTURE_SCHOOL: added to
    /// SOLDIER_FRACTION for the schools numbered 2 and 5 (read as army and culture, inferred).
    pub soldier_target_army_school: f64,
    pub soldier_target_culture_school: f64,
}

impl FocusNumbers {
    pub fn from_engine_rules(m: &EngineRules) -> Result<FocusNumbers, String> {
        let f = |id: &str| crate::num_field(m, id);
        Ok(FocusNumbers {
            never: f("FLD_AI_FOCUS_NEVER")?,
            capital_factor: f("FLD_AI_FOCUS_CAPITAL_FACTOR")?,
            capital_share: f("FLD_AI_FOCUS_CAPITAL_SHARE")?,
            capitalist_min_figure: f("FLD_AI_CAPITALIST_FOCUS_MIN_FIGURE")?,
            rail_base: f("FLD_AI_RAIL_FOCUS_BASE")?,
            rail_factories_per_point: f("FLD_AI_RAIL_FOCUS_FACTORIES_PER_POINT")?,
            soldier_target_army_school: f("FLD_AI_SOLDIER_TARGET_ARMY_SCHOOL")?,
            soldier_target_culture_school: f("FLD_AI_SOLDIER_TARGET_CULTURE_SCHOOL")?,
        })
    }
}

/// The rules file's numbers of the computer player's research weight.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResearchWeights {
    /// FLD_AI_RESEARCH_OVERDUE_PER_YEAR: weight added for every year a technology is overdue.
    pub overdue_per_year: f64,
    /// FLD_AI_RESEARCH_MIN_TOP_COST: the dearest technology counts as costing at least this.
    pub min_top_cost: f64,
}

impl ResearchWeights {
    pub fn from_engine_rules(m: &EngineRules) -> Result<ResearchWeights, String> {
        Ok(ResearchWeights { overdue_per_year: crate::num_field(m, "FLD_AI_RESEARCH_OVERDUE_PER_YEAR")?, min_top_cost: crate::num_field(m, "FLD_AI_RESEARCH_MIN_TOP_COST")? })
    }
}

/// What a focus is to the computer player.
#[derive(Debug, Clone, PartialEq)]
pub enum FocusKind {
    Bureaucrats,
    Soldiers,
    Clergy,
    /// Promotes a pop type that works in factories (craftsmen, clerks): the type's name.
    Workers(String),
    Capitalists,
    Railroad,
    /// Any other focus: scores 0, never chosen.
    Other,
}

/// The constants of the original's defines that the focus scores use.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FocusWeights {
    /// ai.ADMINISTRATOR_WEIGHT, ai.INDUSTRYWORKER_WEIGHT, ai.EDUCATOR_WEIGHT, ai.SOLDIER_WEIGHT.
    pub administrator: f64,
    pub worker: f64,
    pub educator: f64,
    pub soldier: f64,
    /// ai.SOLDIER_FRACTION, ai.CAPITALIST_FRACTION: the share of its people a country wants.
    pub soldier_fraction: f64,
    pub capitalist_fraction: f64,
    /// pops.MAX_CLERGY_FOR_LITERACY.
    pub max_clergy: f64,
    /// country.MAX_BUREAUCRACY_PERCENTAGE and country.BUREAUCRACY_PERCENTAGE_INCREMENT.
    pub max_bureaucracy: f64,
    pub bureaucracy_increment: f64,
    /// The rules file's numbers of the scores.
    pub numbers: FocusNumbers,
}

/// Everything one score needs to know about one province and its owner.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FocusInput {
    /// People in the province.
    pub people: f64,
    /// Share of them whose culture is neither the primary nor an accepted culture of the owner.
    pub foreign_share: f64,
    /// The province's state is a colony or protectorate.
    pub colonial: bool,
    /// The province is the owner's capital.
    pub capital: bool,
    /// Bureaucrats and clergy of the province as shares of its people.
    pub bureaucrat_share: f64,
    pub clergy_share: f64,
    /// The owner's administrative efficiency bonuses (technologies and modifiers), e.g. 0.05.
    pub admin_bonus: f64,
    /// Sum of the levels of the owner's social reforms that are marked administrative.
    pub admin_reform_levels: f64,
    /// Soldiers and capitalists of the whole COUNTRY as shares of its people.
    pub country_soldier_share: f64,
    pub country_capitalist_share: f64,
    /// Added to SOLDIER_FRACTION for the owner's technology school.
    pub soldier_target_shift: f64,
    pub civilized: bool,
    /// The province figure (0..1) the capitalist focus needs to be at least 0.3. What the original
    /// measures here is not known. The port's monthly round gives 0 (so: never): with literacy in
    /// its place the port set 9 capitalist focuses in 1837 (small German states), the original's
    /// saves of 1836-1865 have none at all.
    pub capitalist_figure: f64,
    /// Number of factories in the province's state.
    pub state_factories: usize,
    /// The state of the owner's capital has a factory.
    pub capital_state_has_factory: bool,
    /// Per factory worker type: over the running factories of the state, the sum of (the type's
    /// share of the places - the share of the places it fills), e.g. 0.8 for an empty factory.
    pub unfilled: BTreeMap<String, f64>,
    /// Some province of the state can take one more railroad level with the owner's technology.
    pub rail_possible: bool,
}

/// The score of one focus in one province, as the original's computer player works it out.
///
/// * No people: never (-100).
/// * F = 1 - the share of people of a culture the owner does not accept.
/// * Bureaucrats: a = the province's bureaucrat share x (1 + administrative efficiency bonuses);
///   R = MAX_BUREAUCRACY_PERCENTAGE + BUREAUCRACY_PERCENTAGE_INCREMENT x administrative reform
///   levels; score = (R - a) / R x ADMINISTRATOR_WEIGHT, x10 when a is under 1% AND the province
///   is the capital. No culture factor and no colonial rule here.
/// * Soldiers: (target - the COUNTRY's soldier share) / target x SOLDIER_WEIGHT x F, target =
///   SOLDIER_FRACTION (+ the school's shift); never in a colonial state.
/// * Clergy: (MAX_CLERGY_FOR_LITERACY - the province's clergy share) / MAX_CLERGY_FOR_LITERACY x
///   EDUCATOR_WEIGHT x F; never in a colonial state.
/// * Factory workers: unfilled places of the state's running factories (as shares, summed) x
///   INDUSTRYWORKER_WEIGHT x F; 0 in a state without factories; never in a colonial state.
/// * Capitalists: (CAPITALIST_FRACTION - the COUNTRY's capitalist share) / CAPITALIST_FRACTION x
///   INDUSTRYWORKER_WEIGHT x F; never when the owner is uncivilised, a province figure is
///   under 0.3 (what it measures is unknown, see `FocusInput::capitalist_figure`), the state is colonial, or the
///   capital's state has no factory and this is not the capital.
/// * Railroad: never in a colonial state, without a factory or when no province can take another
///   level; otherwise 2 + factories of the state / 2, rounded down.
/// * Anything else: 0.
pub fn focus_score(w: &FocusWeights, kind: &FocusKind, i: &FocusInput) -> f64 {
    let n = &w.numbers;
    if i.people <= 0.0 {
        return n.never;
    }
    let f = 1.0 - i.foreign_share.clamp(0.0, 1.0);
    let gap = |target: f64, share: f64| if target > 0.0 { (target - share) / target } else { 0.0 };
    match kind {
        FocusKind::Bureaucrats => {
            let a = i.bureaucrat_share * (1.0 + i.admin_bonus);
            let score = gap(w.max_bureaucracy + w.bureaucracy_increment * i.admin_reform_levels, a) * w.administrator;
            if a < n.capital_share && i.capital { score * n.capital_factor } else { score }
        }
        FocusKind::Other => 0.0,
        _ if i.colonial => n.never,
        FocusKind::Soldiers => gap(w.soldier_fraction + i.soldier_target_shift, i.country_soldier_share) * w.soldier * f,
        FocusKind::Clergy => gap(w.max_clergy, i.clergy_share) * w.educator * f,
        FocusKind::Workers(kind) if i.state_factories > 0 => i.unfilled.get(kind).copied().unwrap_or(0.0) * w.worker * f,
        FocusKind::Workers(_) => 0.0,
        FocusKind::Capitalists => {
            if !i.civilized || i.capitalist_figure < n.capitalist_min_figure || (!i.capital_state_has_factory && !i.capital) {
                n.never
            } else {
                gap(w.capitalist_fraction, i.country_capitalist_share) * w.worker * f
            }
        }
        FocusKind::Railroad => {
            if i.state_factories == 0 || !i.rail_possible {
                n.never
            } else {
                n.rail_base + (i.state_factories as f64 / n.rail_factories_per_point).floor()
            }
        }
    }
}

/// The candidate of one region: (average, focus index) of the focus with the highest sum of
/// positive province scores divided by the number of all provinces of the region, if above 0.
/// `scores[p][k]` = score of focus `k` in the country's province `p` of the region. Of equal
/// averages the focus that comes first wins.
pub fn region_candidate(scores: &[Vec<f64>], provinces_in_region: usize) -> Option<(f64, usize)> {
    let kinds = scores.first()?.len();
    let mut best: Option<(f64, usize)> = None;
    for k in 0..kinds {
        let average = scores.iter().map(|s| s[k].max(0.0)).sum::<f64>() / provinces_in_region.max(1) as f64;
        if average > 0.0 && best.is_none_or(|b| average > b.0) {
            best = Some((average, k));
        }
    }
    best
}

/// One candidate of the monthly ranking.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub region: u16,
    /// Average over the region.
    pub average: f64,
    /// Score of the focus in the region's first own province alone.
    pub first_score: f64,
    pub focus: String,
}

/// What changes for one country: `candidates` in any order, `current` = its focuses now (region
/// -> focus), `current_score` = the score of each standing focus on the first province of its
/// region, `points` = how many focuses it may have. Returns (focuses to cancel, focuses to set),
/// in the order they are carried out.
///
/// The best candidates are walked until `points` of them are accepted (first province score above
/// 0). A region that already has exactly the focus is kept. Any other gets it: at once when the
/// region has a focus already (it is replaced: the port's reading) or a point is free, otherwise
/// after cancelling the standing focus with the lowest score that is neither kept nor set in this
/// same round (the port's reading: the original does not say whether a focus just set can be
/// cancelled again); if there is none to cancel the candidate is skipped. `never` is the score
/// of a standing focus that has none (FLD_AI_FOCUS_NEVER).
pub fn plan_focuses(mut candidates: Vec<Candidate>, current: &BTreeMap<u16, String>, current_score: &BTreeMap<u16, f64>, points: usize, never: f64) -> (Vec<u16>, Vec<(u16, String)>) {
    candidates.sort_by(|a, b| b.average.total_cmp(&a.average));
    let accepted: Vec<&Candidate> = candidates.iter().filter(|c| c.first_score > 0.0).take(points).collect();
    let mut protected: BTreeSet<u16> = accepted.iter().filter(|c| current.get(&c.region) == Some(&c.focus)).map(|c| c.region).collect();
    let mut standing: BTreeSet<u16> = current.keys().copied().collect();
    let (mut cancel, mut set) = (Vec::new(), Vec::new());
    for c in accepted.iter().filter(|c| current.get(&c.region) != Some(&c.focus)) {
        if !standing.contains(&c.region) && standing.len() >= points {
            let weakest = standing.iter().filter(|r| !protected.contains(r)).min_by(|a, b| current_score.get(a).copied().unwrap_or(never).total_cmp(&current_score.get(b).copied().unwrap_or(never))).copied();
            let Some(weakest) = weakest else { continue };
            standing.remove(&weakest);
            cancel.push(weakest);
        }
        standing.insert(c.region);
        protected.insert(c.region);
        set.push((c.region, c.focus.clone()));
    }
    (cancel, set)
}

/// The computer player's focus rules with what they need from the game files, and its counters.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FocusAi {
    pub weights: FocusWeights,
    /// The focuses the computer player can score, in file order: (name, kind).
    pub kinds: Vec<(String, FocusKind)>,
    /// Pop type that owns factories (the capitalists).
    pub owner_type: String,
    /// Pop types the original marks by a flag in their files; the port names them in the
    /// rules file (FLD_POP_TYPE_SOLDIERS / _BUREAUCRATS / _CLERGY).
    pub soldiers_type: String,
    pub bureaucrats_type: String,
    pub clergy_type: String,
    /// The kind of the railroad among the province buildings (FLD_BUILDING_KIND_RAILROAD).
    pub rail_kind: String,
    /// Factory type -> (people one level employs, its jobs as (pop type, share of the places)).
    pub factory_jobs: BTreeMap<String, (f64, Vec<(String, f64)>)>,
    /// Technology -> its administrative efficiency, and the railroad levels it allows.
    pub tech_admin: BTreeMap<String, f64>,
    pub tech_rail: BTreeMap<String, f64>,
    /// Reform option -> its administrative level (`administrative_multiplier`).
    pub option_levels: BTreeMap<String, f64>,
    /// Reform option, national value or timed modifier -> its administrative efficiency modifier.
    pub modifier_admin: BTreeMap<String, f64>,
    /// Technology school -> what it adds to SOLDIER_FRACTION.
    pub soldier_shift: BTreeMap<String, f64>,
    /// Focus points of every country (the number of focuses it had in the starting save).
    pub points: BTreeMap<String, usize>,
    /// Focuses set and cancelled so far.
    pub set: usize,
    pub cancelled: usize,
}

impl FocusAi {
    /// `focus_file` = the root of the national focus file; `state` = the state at the start of
    /// the run (for the focus points).
    pub fn new(m: &EngineRules, world: &World, focus_file: &Block, state: &State) -> Result<FocusAi, String> {
        let soldiers_type = m.require_str("FLD_POP_TYPE_SOLDIERS")?.to_string();
        let bureaucrats_type = m.require_str("FLD_POP_TYPE_BUREAUCRATS")?.to_string();
        let clergy_type = m.require_str("FLD_POP_TYPE_CLERGY")?.to_string();
        let rail_kind = m.require_str("FLD_BUILDING_KIND_RAILROAD")?.to_string();
        let d = |section: &str, k: &str| world.defines.get(section, k).ok_or(format!("defines.{section}.{k} missing"));
        let numbers = FocusNumbers::from_engine_rules(m)?;
        let (army_shift, culture_shift) = (numbers.soldier_target_army_school, numbers.soldier_target_culture_school);
        let weights = FocusWeights {
            administrator: d("ai", "ADMINISTRATOR_WEIGHT")?,
            worker: d("ai", "INDUSTRYWORKER_WEIGHT")?,
            educator: d("ai", "EDUCATOR_WEIGHT")?,
            soldier: d("ai", "SOLDIER_WEIGHT")?,
            soldier_fraction: d("ai", "SOLDIER_FRACTION")?,
            capitalist_fraction: d("ai", "CAPITALIST_FRACTION")?,
            max_clergy: d("pops", "MAX_CLERGY_FOR_LITERACY")?,
            max_bureaucracy: d("country", "MAX_BUREAUCRACY_PERCENTAGE")?,
            bureaucracy_increment: d("country", "BUREAUCRACY_PERCENTAGE_INCREMENT")?,
            numbers,
        };
        let workers: BTreeSet<&str> = world.defs.factory_types.iter().flat_map(|t| t.employees.iter()).map(|j| j.pop_type.as_str()).collect();
        let owner_type = world.defs.factory_types.iter().find_map(|t| t.owner.as_ref()).map(|j| j.pop_type.clone()).unwrap_or_default();
        let pop_types: BTreeSet<&str> = world.defs.pop_types.iter().map(|t| t.name.as_str()).collect();
        let mut kinds = Vec::new();
        for (_, group, _) in focus_file.entries() {
            for (name, focus, _) in group.as_block().into_iter().flat_map(|b| b.entries()) {
                let Some(focus) = focus.as_block() else { continue };
                let positive = |k: &str| focus.get(k).and_then(|v| v.as_f64()).is_some_and(|v| v > 0.0);
                let kind = if positive("railroads") {
                    FocusKind::Railroad
                } else {
                    match focus.entries().map(|(k, _, _)| k).find(|k| pop_types.contains(k) && positive(k)) {
                        Some(t) if t == soldiers_type => FocusKind::Soldiers,
                        Some(t) if t == bureaucrats_type => FocusKind::Bureaucrats,
                        Some(t) if t == clergy_type => FocusKind::Clergy,
                        Some(t) if t == owner_type => FocusKind::Capitalists,
                        Some(t) if workers.contains(t) => FocusKind::Workers(t.to_string()),
                        _ => FocusKind::Other,
                    }
                };
                if kind != FocusKind::Other {
                    kinds.push((name.to_string(), kind));
                }
            }
        }
        let lib = &world.defs.modifiers;
        let tech = |k: &str| -> BTreeMap<String, f64> { world.defs.technologies.iter().filter_map(|t| Some((t.name.clone(), *t.values.get(k)?))).collect() };
        // The schools numbered 2 and 5 in the original: read as the army and the culture school
        // (inferred), found here as the schools whose best research bonus is for that folder.
        let favourite = |v: &BTreeMap<String, f64>| v.iter().filter(|(k, x)| k.ends_with("_research_bonus") && **x > 0.0).max_by(|a, b| a.1.total_cmp(b.1)).map(|(k, _)| k.clone());
        let soldier_shift = lib
            .schools
            .iter()
            .filter_map(|(name, v)| match favourite(v)?.as_str() {
                "army_tech_research_bonus" => Some((name.clone(), army_shift)),
                "culture_tech_research_bonus" => Some((name.clone(), culture_shift)),
                _ => None,
            })
            .collect();
        Ok(FocusAi {
            weights,
            kinds,
            owner_type,
            soldiers_type,
            bureaucrats_type,
            clergy_type,
            rail_kind,
            factory_jobs: world.defs.factory_types.iter().map(|t| (t.name.clone(), (t.workforce, t.employees.iter().map(|j| (j.pop_type.clone(), j.amount)).collect()))).collect(),
            tech_admin: tech("administrative_efficiency"),
            tech_rail: tech("max_railroad"),
            option_levels: lib.issue_options.iter().filter_map(|(o, v)| Some((o.clone(), *v.get("administrative_multiplier")?))).collect(),
            modifier_admin: lib.issue_options.iter().chain(&lib.national_values).chain(&lib.event_modifiers).filter_map(|(o, v)| Some((o.clone(), *v.get("administrative_efficiency_modifier")?))).collect(),
            soldier_shift,
            points: state.countries.iter().map(|(t, c)| (t.clone(), c.national_focus.len())).collect(),
            set: 0,
            cancelled: 0,
        })
    }

    /// The monthly focus round of every country that is not played by a person (see the module
    /// text). Returns what was done: (country, region, the focus set or None for a cancelled one).
    pub fn month(&mut self, state: &mut State) -> Vec<(String, u16, Option<String>)> {
        // Per country: people, soldiers, capitalists. Per (country, region): its provinces in
        // the region's order, and its factory workers by type.
        let mut totals: BTreeMap<&str, [f64; 3]> = BTreeMap::new();
        let mut lands: BTreeMap<&str, BTreeMap<u16, Vec<u16>>> = BTreeMap::new();
        let mut pool: BTreeMap<(&str, u16), BTreeMap<&str, f64>> = BTreeMap::new();
        let worker_types: BTreeSet<&str> = self.factory_jobs.values().flat_map(|(_, jobs)| jobs.iter().map(|(t, _)| t.as_str())).collect();
        for (id, p) in &state.provinces {
            let (Some(owner), Some(region)) = (p.owner.as_deref(), p.region) else { continue };
            lands.entry(owner).or_default().entry(region).or_default().push(*id);
            let t = totals.entry(owner).or_default();
            for q in &p.pops {
                t[0] += q.size;
                if q.pop_type == self.soldiers_type {
                    t[1] += q.size;
                } else if q.pop_type == self.owner_type {
                    t[2] += q.size;
                } else if let Some(kind) = worker_types.get(q.pop_type.as_str()) {
                    *pool.entry((owner, region)).or_default().entry(*kind).or_default() += q.size;
                }
            }
        }
        for regions in lands.values_mut() {
            for (region, ids) in regions.iter_mut() {
                if let Some(order) = state.region_members.get(region) {
                    ids.sort_by_key(|id| order.iter().position(|x| x == id).unwrap_or(usize::MAX));
                }
            }
        }
        // Per (country, region): the number of factories, the places of the running ones and the
        // unfilled share of the places per worker type.
        let mut factories: BTreeMap<(&str, u16), usize> = BTreeMap::new();
        let mut capacity: BTreeMap<(&str, u16), f64> = BTreeMap::new();
        for f in &state.factories {
            let Some(region) = f.region else { continue };
            *factories.entry((f.owner.as_str(), region)).or_default() += 1;
            if let Some((workforce, _)) = self.factory_jobs.get(&f.kind).filter(|_| f.level >= 1.0 && !f.closed) {
                *capacity.entry((f.owner.as_str(), region)).or_default() += workforce * f.level;
            }
        }
        let mut unfilled: BTreeMap<(&str, u16), BTreeMap<String, f64>> = BTreeMap::new();
        for f in state.factories.iter().filter(|f| f.level >= 1.0 && !f.closed) {
            let (Some(region), Some((workforce, jobs))) = (f.region, self.factory_jobs.get(&f.kind)) else { continue };
            let key = (f.owner.as_str(), region);
            let Some(all) = capacity.get(&key).copied().filter(|c| *c > 0.0) else { continue };
            let offered = |kind: &str, cap: f64| (pool.get(&key).and_then(|p| p.get(kind)).copied().unwrap_or(0.0) / all).min(cap);
            // A factory that lets people go (`staff`) fills only that part of what is offered.
            let places = workforce * f.level;
            let total: f64 = jobs.iter().map(|(kind, cap)| offered(kind, *cap) * places).sum();
            let staffed = f.staff.map_or(1.0, |s| if total > 0.0 { (s / total).clamp(0.0, 1.0) } else { 0.0 });
            for (kind, cap) in jobs {
                *unfilled.entry(key).or_default().entry(kind.clone()).or_default() += cap - offered(kind, *cap) * staffed;
            }
        }
        let rail = state.building_kinds.get(&self.rail_kind).map(|(name, _)| name.clone());
        let mut done = Vec::new();
        for (tag, regions) in &lands {
            let Some(c) = state.countries.get(*tag).filter(|c| !c.human) else { continue };
            let points = self.points.get(*tag).copied().unwrap_or(0);
            if points == 0 && c.national_focus.is_empty() {
                continue;
            }
            let [people, soldiers, capitalists] = totals.get(tag).copied().unwrap_or_default();
            let capital_region = c.capital.and_then(|id| state.provinces.get(&id)).filter(|p| p.owner.as_deref() == Some(*tag)).and_then(|p| p.region);
            let rail_levels: f64 = c.technologies.iter().filter_map(|t| self.tech_rail.get(t)).sum();
            let country = FocusInput {
                admin_bonus: c.technologies.iter().filter_map(|t| self.tech_admin.get(t)).sum::<f64>() + c.reforms.values().chain(&c.national_value).chain(c.modifiers.iter().map(|m| &m.name)).filter_map(|n| self.modifier_admin.get(n)).sum::<f64>(),
                admin_reform_levels: c.reforms.values().filter_map(|o| self.option_levels.get(o)).sum(),
                country_soldier_share: soldiers / people.max(1.0),
                country_capitalist_share: capitalists / people.max(1.0),
                soldier_target_shift: c.tech_school.as_ref().and_then(|s| self.soldier_shift.get(s)).copied().unwrap_or(0.0),
                civilized: c.civilized,
                capital_state_has_factory: capital_region.is_some_and(|r| factories.contains_key(&(*tag, r))),
                ..Default::default()
            };
            let accepted = |culture: &String| c.primary_culture.as_ref() == Some(culture) || c.accepted_cultures.contains(culture);
            let input = |region: u16, id: u16| -> FocusInput {
                let p = &state.provinces[&id];
                let (mut all, mut foreign, mut crats, mut clergy) = (0.0, 0.0, 0.0, 0.0);
                for q in &p.pops {
                    all += q.size;
                    if !accepted(&q.culture) {
                        foreign += q.size;
                    }
                    if q.pop_type == self.bureaucrats_type {
                        crats += q.size;
                    } else if q.pop_type == self.clergy_type {
                        clergy += q.size;
                    }
                }
                let share = |x: f64| if all > 0.0 { x / all } else { 0.0 };
                FocusInput {
                    people: all,
                    foreign_share: share(foreign),
                    colonial: p.colonial > 0,
                    capital: c.capital == Some(id),
                    bureaucrat_share: share(crats),
                    clergy_share: share(clergy),
                    state_factories: factories.get(&(*tag, region)).copied().unwrap_or(0),
                    unfilled: unfilled.get(&(*tag, region)).cloned().unwrap_or_default(),
                    rail_possible: rail.as_ref().is_some_and(|name| regions[&region].iter().any(|x| state.provinces[x].buildings.get(name).copied().unwrap_or(0.0) < rail_levels)),
                    ..country.clone()
                }
            };
            // A railroad focus needs the railroad unlocked.
            let kinds: Vec<&(String, FocusKind)> = self.kinds.iter().filter(|(_, k)| *k != FocusKind::Railroad || rail_levels >= 1.0).collect();
            let mut candidates = Vec::new();
            for (region, ids) in regions {
                let scores: Vec<Vec<f64>> = ids.iter().map(|id| { let i = input(*region, *id); kinds.iter().map(|(_, k)| focus_score(&self.weights, k, &i)).collect() }).collect();
                let size = state.region_members.get(region).map_or(ids.len(), Vec::len);
                if let Some((average, k)) = region_candidate(&scores, size) {
                    candidates.push(Candidate { region: *region, average, first_score: scores[0][k], focus: kinds[k].0.clone() });
                }
            }
            // Standing focuses on the first province of their region; never where the country
            // holds no land any more, 0 for a focus the computer player cannot score.
            let current_score: BTreeMap<u16, f64> = c
                .national_focus
                .iter()
                .map(|(region, focus)| {
                    let score = match (regions.get(region), self.kinds.iter().find(|(n, _)| n == focus)) {
                        (None, _) => self.weights.numbers.never,
                        (Some(ids), Some((_, k))) => focus_score(&self.weights, k, &input(*region, ids[0])),
                        (Some(_), None) => 0.0,
                    };
                    (*region, score)
                })
                .collect();
            let (cancel, set) = plan_focuses(candidates, &c.national_focus, &current_score, points, self.weights.numbers.never);
            done.extend(cancel.into_iter().map(|r| (tag.to_string(), r, None)).chain(set.into_iter().map(|(r, f)| (tag.to_string(), r, Some(f)))));
        }
        for (tag, region, focus) in &done {
            let Some(c) = state.countries.get_mut(tag) else { continue };
            match focus {
                Some(f) => {
                    c.national_focus.insert(*region, f.clone());
                    self.set += 1;
                }
                None => {
                    c.national_focus.remove(region);
                    self.cancelled += 1;
                }
            }
        }
        done
    }

    /// One line for the end of a run: the focuses of the world by kind, and how often one was
    /// moved. With `lines`, every country's focuses follow on lines of their own
    /// (`focus TAG region=name ...`, regions by their number in a save).
    pub fn summary(&self, state: &State, lines: bool) -> String {
        let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
        for f in state.countries.values().flat_map(|c| c.national_focus.values()) {
            *by_kind.entry(f.as_str()).or_default() += 1;
        }
        let mut kinds: Vec<(&str, usize)> = by_kind.into_iter().collect();
        kinds.sort_by_key(|k| std::cmp::Reverse(k.1));
        let mut out = format!("national focuses: {} in the world ({}); set {} times and cancelled {} times by the computer players over the run (focus points: as many as each country had in the starting save)", kinds.iter().map(|k| k.1).sum::<usize>(), kinds.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", "), self.set, self.cancelled);
        if lines {
            for (tag, c) in state.countries.iter().filter(|(_, c)| !c.national_focus.is_empty()) {
                out.push_str(&format!("\nfocus {tag} {}", c.national_focus.iter().map(|(r, f)| format!("{r}={f}")).collect::<Vec<_>>().join(" ")));
            }
        }
        out
    }
}

/// The weight of one technology in the computer player's random pick of what to research next:
/// `ai_chance` (the technology's own factor) x (1 + the bonus of the country's school for the
/// technology's folder) x (1 + 0.005 x years the technology is overdue) x (2 - its cost / the
/// cost of the dearest technology, which counts as at least 3600). Cheap, overdue and
/// school-favoured technologies are likelier; the pick itself is random by these weights.
pub fn research_weight(w: &ResearchWeights, ai_chance: f64, school_bonus: f64, years_overdue: i32, cost: f64, top_cost: f64) -> f64 {
    ai_chance * (1.0 + school_bonus) * (1.0 + w.overdue_per_year * f64::from(years_overdue.max(0))) * (2.0 - cost / top_cost.max(w.min_top_cost))
}
