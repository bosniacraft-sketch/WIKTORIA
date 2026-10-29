//! Occupation and garrison of provinces as the save holds them (the garrison starts
//! at 100 and sinks while an army besieges; at 0 the province changes controller).
//!
//! Rules (task D): the daily siege step, new controller and liberation, garrison regrowth, blockade\n//! and the occupation shares task E reads for war exhaustion. Pure functions, no state of their own.

use wiktoria_data::date::Date;
use wiktoria_data::script::{Block, Value};

use super::{date, num, text};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProvinceMilitary {
    pub owner: Option<String>,
    pub controller: Option<String>,
    /// 100 = nobody is besieging.
    pub garrison: f64,
    /// Fort and naval base level (the first of the two numbers the save writes).
    pub fort: f64,
    pub naval_base: f64,
    /// The day the controller last changed (`last_controller_change`).
    pub last_controller_change: Option<Date>,
}

impl ProvinceMilitary {
    /// Controlled by somebody other than the owner. A province nobody owns is not occupied.
    pub fn is_occupied(&self) -> bool {
        matches!((&self.owner, &self.controller), (Some(o), Some(c)) if o != c)
    }
}

/// The first number of a building written as `fort = { 1.000 1.000 }`.
fn level(b: &Block, k: &str) -> f64 {
    b.get(k).and_then(Value::as_block).and_then(|x| x.bare_scalars().first().and_then(|s| s.parse().ok())).unwrap_or(0.0)
}

impl ProvinceMilitary {
    /// The flag the economy reads: while somebody else controls the province its RGO workers leave
    /// (the siege code itself does not touch production).
    pub fn rgo_workers_leave(&self) -> bool {
        self.is_occupied()
    }
}

/// Constants of the siege and occupation rules. Rules file ids `FLD_MIL_SIEGE_*` (hypotheses).
#[derive(Debug, Clone, PartialEq)]
pub struct SiegeRules {
    pub full_garrison: f64,
    pub engineer_unit_ratio: f64,
    /// Reconnaissance factor = 1 + best unit reconnaissance among the besiegers x this
    /// (defines RECON_SIEGE_EFFECT, else FLD_MIL_SIEGE_RECON_EFFECT; fitted on 706 siege-days, MIL_SIEGE_003).
    pub recon_effect: f64,
    /// Regiment count where the numbers term switches from N / min to (N - min) x bonus.
    pub brigades_min: f64,
    pub brigades_max: f64,
    /// Numbers per regiment above the minimum (defines SIEGE_BRIGADES_BONUS, else
    /// FLD_MIL_SIEGE_NUMBER_BONUS; MIL_SIEGE_003).
    pub number_bonus: f64,
    /// Men in one regiment of the siege-size count (POP_SIZE_PER_REGIMENT).
    pub men_per_regiment: f64,
    /// Men in 1.0 of regiment strength.
    pub men_per_strength: f64,
    /// Divisor `1 + table[fort]`, fort 0..=9.
    pub fort_table: Vec<f64>,
    /// Multiplier by die 0..=9.
    pub die_table: Vec<f64>,
    pub owner_bonus: f64,
    pub core_bonus: f64,
    /// Relations a besieger earns from the owner when the owner regains the province.
    pub liberation_relations: f64,
    /// Garrison regained per day while nobody besieges (not in the sources: hypothesis).
    pub regrowth_per_day: f64,
}

fn table(m: &wiktoria_data::engine_rules::EngineRules, id: &str, n: usize) -> Result<Vec<f64>, String> {
    let v: Vec<f64> = m.require_str(id)?.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if v.len() != n {
        return Err(format!("rules field {id} needs {n} numbers"));
    }
    Ok(v)
}

impl SiegeRules {
    pub fn from_engine_rules(m: &wiktoria_data::engine_rules::EngineRules, defines: &wiktoria_data::defines::Defines) -> Result<SiegeRules, String> {
        let f = |id: &str| crate::num_field(m, id);
        // Engine constants of the install: whichever section of defines.lua holds the name.
        let get = |k: &str| defines.sections().iter().find_map(|(s, _)| defines.get(s, k));
        let d = |k: &str| get(k).ok_or(format!("defines.{k} missing"));
        // The reconnaissance factor and the numbers bonus were fitted on the daily saves
        // (MIL_SIEGE_003) and came out at the values the install's defines.lua already carries as
        // RECON_SIEGE_EFFECT and SIEGE_BRIGADES_BONUS. The install's number is read first, so a mod
        // that changes it is followed; the rules file rows are only the fallback for an install
        // whose defines lack the name (ticket #242).
        let d_or_f = |k: &str, id: &str| get(k).map(Ok).unwrap_or_else(|| f(id));
        Ok(SiegeRules {
            full_garrison: f("FLD_MIL_FULL_GARRISON")?,
            engineer_unit_ratio: d("ENGINEER_UNIT_RATIO")?,
            recon_effect: d_or_f("RECON_SIEGE_EFFECT", "FLD_MIL_SIEGE_RECON_EFFECT")?,
            brigades_min: d("SIEGE_BRIGADES_MIN")?,
            brigades_max: d("SIEGE_BRIGADES_MAX")?,
            number_bonus: d_or_f("SIEGE_BRIGADES_BONUS", "FLD_MIL_SIEGE_NUMBER_BONUS")?,
            men_per_regiment: d("POP_SIZE_PER_REGIMENT")?,
            men_per_strength: f("FLD_MIL_MEN_PER_STRENGTH")?,
            fort_table: table(m, "FLD_MIL_SIEGE_FORT_TABLE", 10)?,
            die_table: table(m, "FLD_MIL_SIEGE_DIE_TABLE", 10)?,
            owner_bonus: f("FLD_MIL_SIEGE_OWNER_BONUS")?,
            core_bonus: f("FLD_MIL_SIEGE_CORE_BONUS")?,
            liberation_relations: f("FLD_MIL_SIEGE_LIBERATION_RELATIONS")?,
            regrowth_per_day: f("FLD_MIL_SIEGE_GARRISON_REGROWTH")?,
        })
    }
}

/// Our own seeded generator (splitmix64), passed in by the caller.
#[derive(Debug, Clone)]
pub struct SiegeRng(pub u64);

impl SiegeRng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// The siege die, 0..=9.
    pub fn die(&mut self) -> usize {
        (self.next_u64() % 10) as usize
    }
}

/// What the siege step needs of one besieging regiment.
#[derive(Debug, Clone, PartialEq)]
pub struct SiegeRegiment {
    pub strength: f64,
    /// Siege value of its type (engineers), 0 for none.
    pub siege: f64,
    /// Reconnaissance value of its type, 0 for none.
    pub recon: f64,
}

/// The besiegers of one country (first country in the list leads the siege).
#[derive(Debug, Clone, PartialEq)]
pub struct Besieger {
    pub country: String,
    pub regiments: Vec<SiegeRegiment>,
}

/// Questions the step asks of the rest of the game.
pub struct SiegeContext<'a> {
    /// Is `a` at war with `b` (or one of them rebels).
    pub hostile: &'a dyn Fn(&str, &str) -> bool,
    /// Is the province a core of this country.
    pub core_of: &'a dyn Fn(&str) -> bool,
    /// Overlord of a vassal or substate.
    pub overlord: &'a dyn Fn(&str) -> Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SiegeEvent {
    /// Garrison went down but is still above 0 (new garrison value).
    Continues(f64),
    /// No besieger left or none hostile: the siege is over.
    Ended,
    /// Garrison hit 0. `liberated`: the owner got control back. `relations`: countries that earn
    /// relations with the owner (liberation by someone else).
    Captured { new_controller: String, liberated: bool, relations: Vec<String> },
}

/// Daily garrison loss of a siege (fitted on 706 siege-days of the original's daily saves):
///
/// fall = recon x numbers / (1 + fort table[fort left after engineers]) x die table[die] x home
///
/// * recon = 1 + best unit reconnaissance among the regiments x `recon_effect` (nothing else: no
///   share of reconnoitring strength);
/// * N = besieging men / men of a regiment; numbers = N / min under the minimum, else
///   (min(N, max) - min) x `number_bonus` (so exactly at the minimum it is 0, as in the fit);
/// * home = owner bonus when the owner besieges, else core bonus for a core of the first besieger.
///
/// The strengths passed in should be those of the day before (the original books today's fall
/// on yesterday's men); `MilitaryDay` keeps them.
pub fn siege_progress(rules: &SiegeRules, fort: f64, regiments: &[SiegeRegiment], die: usize, owner_is_besieger: bool, core_of_first: bool) -> f64 {
    let total: f64 = regiments.iter().map(|r| r.strength).sum();
    if total <= 0.0 {
        return 0.0;
    }
    let s = regiments.iter().filter(|r| r.siege > 0.0).map(|r| r.siege).fold(0.0, f64::max);
    let e: f64 = regiments.iter().filter(|r| r.siege > 0.0).map(|r| r.strength).sum();
    let mut fort = fort.floor().clamp(0.0, 9.0);
    if s > 0.0 {
        let share = e / total;
        let eng = if share < rules.engineer_unit_ratio { s * share / rules.engineer_unit_ratio } else { s };
        fort = (fort - eng.floor()).max(0.0);
    }
    let best_recon = regiments.iter().map(|r| r.recon).fold(0.0, f64::max);
    let mut p = 1.0 + best_recon * rules.recon_effect;
    let n = total * rules.men_per_strength / rules.men_per_regiment;
    p *= if n < rules.brigades_min { n / rules.brigades_min } else { (n.min(rules.brigades_max) - rules.brigades_min) * rules.number_bonus };
    p /= 1.0 + rules.fort_table[fort as usize];
    p *= rules.die_table[die.min(9)];
    if owner_is_besieger {
        p *= rules.owner_bonus;
    } else if core_of_first {
        p *= rules.core_bonus;
    }
    p
}

/// One day of a siege. Mutates garrison and, at 0, controller. `besiegers` must already be only
/// the units that stand in the province, do not move or retreat and are alive. `today` stamps the
/// controller change. The caller skips the call while the province holds a battle.
pub fn siege_day(rules: &SiegeRules, p: &mut ProvinceMilitary, besiegers: &[Besieger], ctx: &SiegeContext, rng: &mut SiegeRng, today: Date) -> SiegeEvent {
    let (Some(controller), Some(first)) = (p.controller.clone(), besiegers.iter().find(|b| b.regiments.iter().any(|r| r.strength > 0.001))) else { return SiegeEvent::Ended };
    let first = first.country.clone();
    if first == controller || !(ctx.hostile)(&first, &controller) {
        return SiegeEvent::Ended;
    }
    let regiments: Vec<SiegeRegiment> = besiegers.iter().flat_map(|b| b.regiments.iter()).filter(|r| r.strength > 0.001).cloned().collect();
    let owner_is_besieger = p.owner.as_ref().is_some_and(|o| besiegers.iter().any(|b| &b.country == o));
    let core = (ctx.core_of)(&first);
    let die = rng.die();
    let drop = siege_progress(rules, p.fort, &regiments, die, owner_is_besieger, core);
    p.garrison = (p.garrison - drop).min(rules.full_garrison);
    if p.garrison > 0.0 {
        return SiegeEvent::Continues(p.garrison);
    }
    // Garrison gone: choose the new controller.
    let mut taker = first;
    while let Some(o) = (ctx.overlord)(&taker) {
        if o != controller && (ctx.hostile)(&o, &controller) {
            taker = o;
        } else {
            break;
        }
    }
    let owner = p.owner.clone();
    let liberated = owner.as_ref().is_some_and(|o| *o == taker || !(ctx.hostile)(&taker, o));
    let new_controller = if liberated { owner.clone().unwrap_or(taker) } else { taker };
    p.controller = Some(new_controller.clone());
    p.garrison = rules.full_garrison;
    p.last_controller_change = Some(today);
    let relations = if liberated && Some(&new_controller) == owner.as_ref() {
        besiegers.iter().map(|b| b.country.clone()).filter(|c| *c != new_controller).collect()
    } else {
        Vec::new()
    };
    SiegeEvent::Captured { new_controller, liberated, relations }
}

/// Garrison regrowth of a province nobody besieges.
pub fn garrison_regrow_day(rules: &SiegeRules, p: &mut ProvinceMilitary, besieged: bool) {
    if !besieged {
        p.garrison = (p.garrison + rules.regrowth_per_day).min(rules.full_garrison);
    }
}

/// A fleet as the blockade test sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockadeUnit {
    pub owner: String,
    pub ships: usize,
    pub in_battle: bool,
    pub retreating: bool,
}

/// Does the fleet blockade a port whose controller is `controller`?
pub fn blockades(u: &BlockadeUnit, controller: &str, hostile: &dyn Fn(&str, &str) -> bool) -> bool {
    !u.in_battle && !u.retreating && u.ships > 0 && hostile(&u.owner, controller)
}

/// A port province is blockaded while the ships of blockading fleets in its sea zone are above 0.
pub fn port_blockaded(fleets_in_sea: &[BlockadeUnit], controller: &str, hostile: &dyn Fn(&str, &str) -> bool) -> bool {
    fleets_in_sea.iter().filter(|u| blockades(u, controller, hostile)).map(|u| u.ships).sum::<usize>() > 0
}

/// One province of a country, as the shares need it.
#[derive(Debug, Clone, PartialEq)]
pub struct HomeProvince {
    pub owner: String,
    pub controller: String,
    pub port: bool,
    /// On the capital's continent (or connected by land).
    pub home_continent: bool,
    pub blockaded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct OccupationShare {
    /// Share (0..1) of home-continent provinces with a foreign, non-rebel controller.
    pub occupied: f64,
    /// Share (0..1) of home-continent port provinces that are blockaded or not controlled by the owner.
    pub blockaded: f64,
}

/// Shares for war exhaustion (+0.05 x percent occupied, +0.5 x blockaded share a month).
pub fn occupation_share(country: &str, provinces: &[HomeProvince], rebel_tag: &str) -> OccupationShare {
    let home: Vec<&HomeProvince> = provinces.iter().filter(|p| p.owner == country && p.home_continent).collect();
    if home.is_empty() {
        return OccupationShare::default();
    }
    let occupied = home.iter().filter(|p| p.controller != country && p.controller != rebel_tag).count();
    let ports: Vec<&&HomeProvince> = home.iter().filter(|p| p.port).collect();
    let blocked = ports.iter().filter(|p| p.blockaded || p.controller != country).count();
    OccupationShare { occupied: occupied as f64 / home.len() as f64, blockaded: if ports.is_empty() { 0.0 } else { blocked as f64 / ports.len() as f64 } }
}

pub fn read_province(b: &Block) -> ProvinceMilitary {
    ProvinceMilitary { owner: text(b, "owner"), controller: text(b, "controller"), garrison: num(b, "garrison"), fort: level(b, "fort"), naval_base: level(b, "naval_base"), last_controller_change: date(b, "last_controller_change") }
}

