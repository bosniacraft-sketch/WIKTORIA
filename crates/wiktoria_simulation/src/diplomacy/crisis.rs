//! Flashpoints and crises.
//!
//! A flashpoint is a state the original marks `flashpoint = yes` with the tag that wants it
//! (`crisis = "GRE"`); the port reads them from the save. Tension builds on a flashpoint by the
//! defines' terms; once the cooldown since the last crisis is over, a flashpoint at full tension
//! can start a crisis: the tag that wants the state (attacker) against its owner (defender), one
//! great power backing the attacker from the start, the other great powers interested. Interested
//! powers take a side or stay out; the temperature rises the faster the more have taken sides;
//! the crisis is settled without war: the weaker side gives in, a defender that gives in hands
//! the state over.
//!
//! Measured (the original's 64-year run, crises of 1845-1883): every crisis started 2 to 58 days
//! after the 60-month cooldown ran out; the attacker had a backer the day before the crisis
//! began; 2 to 5 powers joined a side 2 to 137 days in (mean 52); crises lasted 90 to 271 days;
//! none became a war; Greece won the Aegean Islands (1846) and East Macedonia (1857) from the
//! Ottomans, Romania did not get Western Transylvania from Austria (1883). A crisis waiting for
//! its first backer stood at temperature 2.5. Everything else is the defines' wording read as
//! numbers, marked STAND-IN where the port had to choose.

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_data::date::Date;
use wiktoria_data::defines::Defines;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::save::SaveGame;
use wiktoria_data::script::{Block, Value};

use super::define;

#[derive(Debug, Clone, PartialEq)]
pub struct CrisisRules {
    pub tension_decay: f64,
    pub tension_from_cb: f64,
    /// Per 1000 supporters of the tag's independence movement, with its total cap.
    pub tension_from_movement: f64,
    pub tension_from_movement_max: f64,
    /// For each potentially interested great power at war, as a share of all of them.
    pub at_war_tension_decay: f64,
    pub tension_on_cb_discovered: f64,
    pub tension_while_crisis: f64,
    /// Extra decay on a flashpoint owned by the country of rank 1..8 (index 0..7).
    pub rank_decay: Vec<f64>,
    pub cooldown_months: u32,
    pub base_chance: f64,
    pub temperature_increase: f64,
    pub participant_factor: f64,
    pub temperature_on_mobilize: f64,
    pub interest_war_exhaustion_limit: f64,
    pub not_sided_prestige_base: f64,
    pub not_sided_prestige_year: f64,
    pub winner_prestige_base: f64,
    pub winner_prestige_year: f64,
    pub winner_relations: f64,
    // ---- rules file
    pub tension_max: f64,
    pub start_temperature: f64,
    /// Daily chance of a crisis = base_chance / this (fitted to the start delays).
    pub chance_divisor: f64,
    /// Mean days an interested power takes to answer (daily chance 1 / this).
    pub answer_days: f64,
    pub settle_temperature: f64,
    pub start_year: i32,
}

impl CrisisRules {
    pub fn new(m: &EngineRules, d: &Defines) -> Result<CrisisRules, String> {
        let g = |k: &str| define(d, "diplomacy", k);
        Ok(CrisisRules {
            tension_decay: g("TENSION_DECAY")?,
            tension_from_cb: g("TENSION_FROM_CB")?,
            tension_from_movement: g("TENSION_FROM_MOVEMENT")?,
            tension_from_movement_max: g("TENSION_FROM_MOVEMENT_MAX")?,
            at_war_tension_decay: g("AT_WAR_TENSION_DECAY")?,
            tension_on_cb_discovered: g("TENSION_ON_CB_DISCOVERED")?,
            tension_while_crisis: g("TENSION_WHILE_CRISIS")?,
            rank_decay: (1..=8).map(|n| g(&format!("RANK_{n}_TENSION_DECAY"))).collect::<Result<_, _>>()?,
            cooldown_months: g("CRISIS_COOLDOWN_MONTHS")?.max(0.0) as u32,
            base_chance: g("CRISIS_BASE_CHANCE")?,
            temperature_increase: g("CRISIS_TEMPERATURE_INCREASE")?,
            participant_factor: g("CRISIS_TEMPERATURE_PARTICIPANT_FACTOR")?,
            temperature_on_mobilize: g("CRISIS_TEMPERATURE_ON_MOBILIZE")?,
            interest_war_exhaustion_limit: g("CRISIS_INTEREST_WAR_EXHAUSTION_LIMIT")?,
            not_sided_prestige_base: g("CRISIS_DID_NOT_TAKE_SIDE_PRESTIGE_FACTOR_BASE")?,
            not_sided_prestige_year: g("CRISIS_DID_NOT_TAKE_SIDE_PRESTIGE_FACTOR_YEAR")?,
            winner_prestige_base: g("CRISIS_WINNER_PRESTIGE_FACTOR_BASE")?,
            winner_prestige_year: g("CRISIS_WINNER_PRESTIGE_FACTOR_YEAR")?,
            winner_relations: g("CRISIS_WINNER_RELATIONS_IMPACT")?,
            tension_max: crate::num_field(m, "FLD_DIP_TENSION_MAX")?,
            start_temperature: crate::num_field(m, "FLD_DIP_CRISIS_START_TEMPERATURE")?,
            chance_divisor: crate::num_field(m, "FLD_DIP_CRISIS_CHANCE_DIVISOR")?,
            answer_days: crate::num_field(m, "FLD_DIP_CRISIS_ANSWER_DAYS")?,
            settle_temperature: crate::num_field(m, "FLD_DIP_CRISIS_SETTLE_TEMPERATURE")?,
            start_year: crate::int_field(m, "FLD_DIP_CRISIS_START_YEAR")? as i32,
        })
    }

    /// Years since the start, for the defines' per-year factors.
    fn years(&self, today: Date) -> f64 {
        f64::from(today.year - self.start_year).max(0.0)
    }
}

/// A flashpoint state: named by one of its provinces (the save's first), its owner and the tag
/// that wants it.
#[derive(Debug, Clone, PartialEq)]
pub struct Flashpoint {
    pub province: u16,
    pub owner: String,
    pub tag: String,
    pub tension: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Attacker,
    Defender,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Crisis {
    /// A province of the state in question.
    pub province: u16,
    pub attacker: String,
    pub defender: String,
    pub attacker_backers: Vec<String>,
    pub defender_backers: Vec<String>,
    /// Great powers that have not answered yet.
    pub interested: Vec<String>,
    /// Great powers that answered by staying out.
    pub declined: Vec<String>,
    pub temperature: f64,
    pub started: Option<Date>,
}

impl Crisis {
    pub fn participants(&self) -> usize {
        self.attacker_backers.len() + self.defender_backers.len()
    }

    pub fn involves(&self, tag: &str) -> bool {
        self.attacker == tag || self.defender == tag || self.attacker_backers.iter().chain(&self.defender_backers).any(|x| x == tag)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CrisisState {
    pub flashpoints: Vec<Flashpoint>,
    pub crisis: Option<Crisis>,
    /// When the last crisis ended (the save's `crisis_manager = { date = ... }`).
    pub last_end: Option<Date>,
}

fn strs(b: &Block, k: &str) -> Vec<String> {
    b.get_all(k).filter_map(Value::as_str).map(|s| s.trim_matches('"').to_string()).collect()
}

impl CrisisState {
    /// The flashpoints of every country's states and the crisis manager of an original save.
    /// The save holds no tension: every flashpoint starts at 0.
    pub fn from_save(save: &SaveGame) -> CrisisState {
        let root = &save.doc.root;
        let mut out = CrisisState::default();
        for tag in &save.countries {
            let Some(b) = root.get(tag).and_then(Value::as_block) else { continue };
            for s in b.get_all("state").filter_map(Value::as_block) {
                if s.get("flashpoint").and_then(Value::as_bool) != Some(true) {
                    continue;
                }
                let Some(want) = s.get("crisis").and_then(Value::as_str) else { continue };
                let Some(p) = s.get("provinces").and_then(Value::as_block).and_then(|x| x.bare_scalars().first().and_then(|n| n.parse().ok())) else { continue };
                out.flashpoints.push(Flashpoint { province: p, owner: tag.clone(), tag: want.trim_matches('"').to_string(), tension: 0.0 });
            }
        }
        if let Some(m) = root.get("crisis_manager").and_then(Value::as_block) {
            out.last_end = m.get("date").and_then(Value::as_str).and_then(Date::parse);
            if let Some(c) = m.get("crisis").and_then(Value::as_block) {
                let num = |k: &str| c.get(k).and_then(Value::as_f64);
                let one = |k: &str| strs(c, k).into_iter().next();
                if let (Some(p), Some(a), Some(d)) = (num("state"), one("attacker"), one("defender")) {
                    out.crisis = Some(Crisis {
                        province: p as u16,
                        attacker: a,
                        defender: d,
                        attacker_backers: strs(c, "attacker_backer"),
                        defender_backers: strs(c, "defender_backer"),
                        interested: strs(c, "interested"),
                        declined: Vec::new(),
                        temperature: num("temperature").unwrap_or(0.0),
                        started: None,
                    });
                }
            }
        }
        out
    }
}

/// What the day's tension step knows about one flashpoint.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TensionInput {
    /// Rank of the owner (1 = first), 0 if unranked.
    pub owner_rank: usize,
    /// Casus belli held or being made on the owner by the tag that wants the state.
    pub casus_belli: usize,
    /// Supporters of the tag's independence movement in the owner.
    pub movement: f64,
    /// Share of the potentially interested great powers that are at war (0..1).
    pub at_war_share: f64,
    pub crisis_running: bool,
}

/// One day's tension change of a flashpoint (before clamping to 0..max).
pub fn tension_change(r: &CrisisRules, i: &TensionInput) -> f64 {
    let mut d = r.tension_decay;
    if (1..=r.rank_decay.len()).contains(&i.owner_rank) {
        d += r.rank_decay[i.owner_rank - 1];
    }
    d += r.tension_from_cb * i.casus_belli as f64;
    d += (r.tension_from_movement * i.movement / 1000.0).min(r.tension_from_movement_max);
    d += r.at_war_tension_decay * i.at_war_share;
    if i.crisis_running {
        d += r.tension_while_crisis;
    }
    d
}

pub fn add_tension(r: &CrisisRules, f: &mut Flashpoint, delta: f64) {
    f.tension = (f.tension + delta).clamp(0.0, r.tension_max);
}

/// The cooldown since the last crisis is over.
pub fn cooldown_over(r: &CrisisRules, s: &CrisisState, today: Date) -> bool {
    s.last_end.is_none_or(|d| today >= crate::military::war::add_months(d, r.cooldown_months))
}

/// Today's chance that a crisis starts at a flashpoint at full tension (no crisis running, the
/// cooldown over). Fitted: CRISIS_BASE_CHANCE / the divisor per day gives the mean 17.6-day delay
/// measured after the cooldown.
pub fn start_chance(r: &CrisisRules) -> f64 {
    (r.base_chance / r.chance_divisor).clamp(0.0, 1.0)
}

/// The flashpoint a crisis would start at: the one with the most tension among those at the
/// maximum, whose tag exists and is not the owner. STAND-IN: a flashpoint whose tag does not
/// exist (a country to be liberated) starts none.
pub fn crisis_candidate<'a>(r: &CrisisRules, s: &'a CrisisState, exists: &dyn Fn(&str) -> bool) -> Option<&'a Flashpoint> {
    s.flashpoints.iter().filter(|f| f.tension >= r.tension_max && f.tag != f.owner && exists(&f.tag)).max_by(|a, b| a.tension.total_cmp(&b.tension).then(b.province.cmp(&a.province)))
}

/// Starts a crisis: `backer` is the great power that backs the attacker (measured: one does from
/// the first day), `interested` every other great power that may take part.
pub fn start_crisis(r: &CrisisRules, f: &Flashpoint, backer: &str, interested: Vec<String>, today: Date) -> Crisis {
    Crisis {
        province: f.province,
        attacker: f.tag.clone(),
        defender: f.owner.clone(),
        attacker_backers: vec![backer.to_string()],
        defender_backers: Vec::new(),
        interested,
        declined: Vec::new(),
        temperature: r.start_temperature,
        started: Some(today),
    }
}

/// One day's temperature rise: the increase, times 1 + the participant factor x the share of the
/// powers that took a side among all that were asked. STAND-IN reading of the defines' comment
/// ("how much faster a crisis heats up if all interested parties have taken sides").
pub fn temperature_rise(r: &CrisisRules, c: &Crisis) -> f64 {
    let asked = c.participants() + c.interested.len() + c.declined.len();
    let share = if asked == 0 { 0.0 } else { c.participants() as f64 / asked as f64 };
    r.temperature_increase * (1.0 + r.participant_factor * share)
}

/// What an interested power weighs when it answers.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AnswerInput {
    pub relation_attacker: f64,
    pub relation_defender: f64,
    /// The attacker / defender is in its sphere, its ally or its subject.
    pub tied_attacker: bool,
    pub tied_defender: bool,
}

/// An interested power's answer: the side it is tied to, otherwise the side it likes better if
/// it likes it at all, otherwise none. STAND-IN for the original's AI.
pub fn answer(i: &AnswerInput) -> Option<Side> {
    match (i.tied_attacker, i.tied_defender) {
        (true, false) => return Some(Side::Attacker),
        (false, true) => return Some(Side::Defender),
        _ => {}
    }
    if i.relation_attacker > i.relation_defender && i.relation_attacker > 0.0 {
        Some(Side::Attacker)
    } else if i.relation_defender > i.relation_attacker && i.relation_defender > 0.0 {
        Some(Side::Defender)
    } else {
        None
    }
}

pub fn record_answer(c: &mut Crisis, power: &str, side: Option<Side>) {
    c.interested.retain(|x| x != power);
    match side {
        Some(Side::Attacker) => c.attacker_backers.push(power.to_string()),
        Some(Side::Defender) => c.defender_backers.push(power.to_string()),
        None => c.declined.push(power.to_string()),
    }
}

/// How a settled crisis ends.
#[derive(Debug, Clone, PartialEq)]
pub struct Settlement {
    pub winner: Side,
    /// The state changes hands (the defender gave in).
    pub state_ceded: bool,
    /// Prestige to add, by country.
    pub prestige: BTreeMap<String, f64>,
    /// Relation changes between a backer and the side it backed: (backer, main, change).
    pub relations: Vec<(String, String, f64)>,
}

/// Whether the crisis is settled today (measured: every crisis of 1845-1883 ended without war).
/// STAND-IN: settled once the temperature reaches the rules file's settling point.
pub fn settles(r: &CrisisRules, c: &Crisis) -> bool {
    c.temperature >= r.settle_temperature
}

/// The settlement: the side with less strength (the sum of `strength` over its main country and
/// backers) gives in (STAND-IN); a giving-in defender cedes the state. Winners gain the defines'
/// winner prestige (base + per year, read as points), powers that stayed out lose that share of
/// their prestige, backers' relations with the side they backed move by the winner impact
/// (up for winners, down for losers).
pub fn settle(r: &CrisisRules, c: &Crisis, today: Date, strength: &dyn Fn(&str) -> f64, prestige: &dyn Fn(&str) -> f64) -> Settlement {
    let side = |main: &str, backers: &[String]| strength(main) + backers.iter().map(|b| strength(b)).sum::<f64>();
    let (a, d) = (side(&c.attacker, &c.attacker_backers), side(&c.defender, &c.defender_backers));
    let winner = if a > d { Side::Attacker } else { Side::Defender };
    let years = r.years(today);
    let mut out = Settlement { winner, state_ceded: winner == Side::Attacker, prestige: BTreeMap::new(), relations: Vec::new() };
    let (win_main, win_backers, lose_main, lose_backers) = match winner {
        Side::Attacker => (&c.attacker, &c.attacker_backers, &c.defender, &c.defender_backers),
        Side::Defender => (&c.defender, &c.defender_backers, &c.attacker, &c.attacker_backers),
    };
    let gain = r.winner_prestige_base + r.winner_prestige_year * years;
    for w in std::iter::once(win_main).chain(win_backers.iter()) {
        *out.prestige.entry(w.clone()).or_default() += gain;
    }
    let factor = r.not_sided_prestige_base + r.not_sided_prestige_year * years;
    for x in c.declined.iter().chain(&c.interested) {
        *out.prestige.entry(x.clone()).or_default() += prestige(x) * factor;
    }
    for b in win_backers {
        out.relations.push((b.clone(), win_main.clone(), r.winner_relations));
    }
    for b in lose_backers {
        out.relations.push((b.clone(), lose_main.clone(), -r.winner_relations));
    }
    out
}

/// The great powers that may be interested: not at war, war exhaustion within the limit, not
/// the attacker or defender.
pub fn may_be_interested(r: &CrisisRules, powers: &[(String, f64, bool)], attacker: &str, defender: &str) -> Vec<String> {
    powers.iter().filter(|(t, we, war)| t != attacker && t != defender && !war && *we <= r.interest_war_exhaustion_limit).map(|(t, _, _)| t.clone()).collect()
}

/// The countries that want a flashpoint state today: tags with a set of flashpoints by owner,
/// for the reports.
pub fn by_owner(s: &CrisisState) -> BTreeMap<&str, BTreeSet<&str>> {
    let mut out: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for f in &s.flashpoints {
        out.entry(f.owner.as_str()).or_default().insert(f.tag.as_str());
    }
    out
}
