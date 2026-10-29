//! Diplomacy, as pure rules (docs/DECISIONS.md D9/D10/D15/D16): relations, alliances, military
//! access, guarantees, vassals and substates, truces, spheres and influence, diplomatic points.
//!
//! This step holds the state, the loader from an original save and the rules as functions on
//! it. Constants: the engine's own come from the install's `defines.lua` at runtime
//! (`DipRules::new`), the fixed numbers from the rules file (`FLD_DIP_*`, hypotheses). Nothing is
//! hard-coded here.

use std::collections::BTreeMap;

use wiktoria_data::date::Date;
use wiktoria_data::defines::Defines;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::save::SaveGame;
use wiktoria_data::script::{Block, Value};

pub mod crisis;
pub mod influence;
pub mod relations;
pub mod treaties;

use relations::{DriftRules, Relation};

/// A treaty between two countries: alliance, guarantee (first = guarantor).
#[derive(Debug, Clone, PartialEq)]
pub struct Treaty {
    pub first: String,
    pub second: String,
    pub start: Option<Date>,
    pub end: Option<Date>,
}

impl Treaty {
    pub fn new(first: &str, second: &str) -> Treaty {
        Treaty { first: first.into(), second: second.into(), start: None, end: None }
    }

    pub fn joins(&self, a: &str, b: &str) -> bool {
        (self.first == a && self.second == b) || (self.first == b && self.second == a)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjectKind {
    Vassal,
    /// Only the start data makes one.
    Substate,
}

/// `first` is the overlord.
#[derive(Debug, Clone, PartialEq)]
pub struct Subject {
    pub overlord: String,
    pub subject: String,
    pub kind: SubjectKind,
    pub start: Option<Date>,
    pub end: Option<Date>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DipCountry {
    /// Diplomatic points (kept between 0 and the maximum).
    pub points: f64,
    pub great_power: bool,
    pub human: bool,
}

/// All of a world's standing relations between countries.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DiplomacyState {
    pub date: Option<Date>,
    pub countries: BTreeMap<String, DipCountry>,
    /// A's view of B, by (A, B). Values are kept symmetric by the rules; the loader keeps what
    /// the save has.
    pub relations: BTreeMap<(String, String), Relation>,
    pub alliances: Vec<Treaty>,
    pub guarantees: Vec<Treaty>,
    pub subjects: Vec<Subject>,
    /// Influence priority 0..3 a great power set for a country: (power, target).
    pub priorities: BTreeMap<(String, String), u8>,
    pub warnings: Vec<String>,
}

fn is_tag(k: &str) -> bool {
    k.len() == 3 && k.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

fn date_of(b: &Block, k: &str) -> Option<Date> {
    b.get(k).and_then(Value::as_str).and_then(Date::parse)
}

impl DiplomacyState {
    pub fn new(date: Date) -> DiplomacyState {
        DiplomacyState { date: Some(date), ..Default::default() }
    }

    /// Reads the diplomacy block, the relation records of every country, diplomatic points and
    /// the great-power list from an original save.
    pub fn from_save(save: &SaveGame) -> DiplomacyState {
        let root = &save.doc.root;
        let mut out = DiplomacyState::new(save.date);
        // great_nations numbers the country blocks from 1.
        let great: Vec<&String> = root.get("great_nations").and_then(Value::as_block).map(|b| b.bare_scalars()).unwrap_or_default().iter().filter_map(|n| save.countries.get(n.parse::<usize>().ok()?.checked_sub(1)?)).collect();
        for tag in &save.countries {
            let Some(b) = root.get(tag).and_then(Value::as_block) else { continue };
            // The influence priorities a great power set: `influence = { "BEL"="3" ... }`.
            if let Some(p) = b.get("influence").and_then(Value::as_block) {
                for (k, v, _) in p.entries() {
                    if let Some(n) = v.as_str().and_then(|s| s.trim_matches('"').parse::<f64>().ok()) {
                        out.priorities.insert((tag.clone(), k.trim_matches('"').to_string()), n.clamp(0.0, 3.0) as u8);
                    }
                }
            }
            out.countries.insert(tag.clone(), DipCountry { points: b.get("diplomatic_points").and_then(Value::as_f64).unwrap_or(0.0), great_power: great.contains(&tag), human: b.get("human").and_then(Value::as_str) == Some("yes") });
            for (k, v, _) in b.entries() {
                let Some(r) = v.as_block().filter(|_| is_tag(k)) else { continue };
                let num = |key: &str| r.get(key).and_then(Value::as_f64);
                out.relations.insert(
                    (tag.clone(), k.to_string()),
                    Relation {
                        value: num("value").unwrap_or(0.0),
                        level: num("level").map_or(relations::level::NEUTRAL, |x| x as u8),
                        influence: num("influence_value").unwrap_or(0.0),
                        access: r.get("military_access").and_then(Value::as_str) == Some("yes"),
                        truce_until: date_of(r, "truce_until"),
                        discredited_until: date_of(r, "discredit"),
                        discredited_by: r.get("discreditor").and_then(Value::as_str).map(str::to_string),
                        banned_until: None,
                        last_war: date_of(r, "last_war"),
                    },
                );
            }
        }
        if let Some(d) = root.get("diplomacy").and_then(Value::as_block) {
            let treaty = |b: &Block| Some(Treaty { first: b.get("first")?.as_str()?.to_string(), second: b.get("second")?.as_str()?.to_string(), start: date_of(b, "start_date"), end: date_of(b, "end_date") });
            for (key, v, line) in d.entries() {
                let Some(b) = v.as_block() else { continue };
                let Some(t) = treaty(b) else {
                    if matches!(key, "alliance" | "vassal" | "substate" | "guarantee") {
                        out.warnings.push(format!("save line {line}: a {key} without first and second"));
                    }
                    continue;
                };
                match key {
                    "alliance" => out.alliances.push(t),
                    "guarantee" => out.guarantees.push(t),
                    "vassal" | "substate" => out.subjects.push(Subject { overlord: t.first, subject: t.second, kind: if key == "vassal" { SubjectKind::Vassal } else { SubjectKind::Substate }, start: t.start, end: t.end }),
                    _ => {}
                }
            }
        }
        out
    }

    // ---- queries

    pub fn relation(&self, from: &str, to: &str) -> Option<&Relation> {
        self.relations.get(&(from.to_string(), to.to_string()))
    }

    pub fn relation_mut(&mut self, from: &str, to: &str) -> &mut Relation {
        self.relations.entry((from.to_string(), to.to_string())).or_default()
    }

    pub fn relation_value(&self, a: &str, b: &str) -> f64 {
        self.relation(a, b).map_or(0.0, |r| r.value)
    }

    pub fn level(&self, from: &str, to: &str) -> u8 {
        self.relation(from, to).map_or(relations::level::NEUTRAL, |r| r.level)
    }

    pub fn influence(&self, from: &str, to: &str) -> f64 {
        self.relation(from, to).map_or(0.0, |r| r.influence)
    }

    pub fn is_great_power(&self, tag: &str) -> bool {
        self.countries.get(tag).is_some_and(|c| c.great_power)
    }

    pub fn great_powers(&self) -> impl Iterator<Item = &str> {
        self.countries.iter().filter(|(_, c)| c.great_power).map(|(t, _)| t.as_str())
    }

    pub fn allied(&self, a: &str, b: &str) -> bool {
        self.alliances.iter().any(|t| t.joins(a, b))
    }

    pub fn allies_of<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.alliances.iter().filter_map(move |t| if t.first == tag { Some(t.second.as_str()) } else if t.second == tag { Some(t.first.as_str()) } else { None })
    }

    pub fn guaranteed_by(&self, guarantor: &str, tag: &str) -> bool {
        self.guarantees.iter().any(|t| t.first == guarantor && t.second == tag)
    }

    pub fn overlord_of(&self, tag: &str) -> Option<&str> {
        self.subjects.iter().find(|s| s.subject == tag).map(|s| s.overlord.as_str())
    }

    pub fn is_subject(&self, tag: &str) -> bool {
        self.overlord_of(tag).is_some()
    }

    pub fn is_substate(&self, tag: &str) -> bool {
        self.subjects.iter().any(|s| s.subject == tag && s.kind == SubjectKind::Substate)
    }

    pub fn subjects_of<'a>(&'a self, overlord: &'a str) -> impl Iterator<Item = &'a Subject> + 'a {
        self.subjects.iter().filter(move |s| s.overlord == overlord)
    }

    /// Either one is the other's overlord.
    pub fn subject_link(&self, a: &str, b: &str) -> bool {
        self.overlord_of(a) == Some(b) || self.overlord_of(b) == Some(a)
    }

    /// The countries `leader` holds at opinion level 5 (the sphere).
    pub fn sphere_members<'a>(&'a self, leader: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.relations.iter().filter(move |((a, _), r)| a == leader && r.level == relations::level::IN_SPHERE).map(|((_, b), _)| b.as_str())
    }

    /// Only a great power holds a sphere (one that loses the rank lets its sphere go), so the
    /// great powers' records are the only ones looked at.
    pub fn sphere_leader(&self, tag: &str) -> Option<&str> {
        self.great_powers().find(|g| self.level(g, tag) == relations::level::IN_SPHERE)
    }

    pub fn truce_active(&self, a: &str, b: &str, today: Date) -> bool {
        [(a, b), (b, a)].iter().any(|(x, y)| self.relation(x, y).and_then(|r| r.truce_until).is_some_and(|d| today <= d))
    }

    pub fn has_access(&self, a: &str, through: &str) -> bool {
        self.relation(a, through).is_some_and(|r| r.access)
    }

    pub fn points(&self, tag: &str) -> f64 {
        self.countries.get(tag).map_or(0.0, |c| c.points)
    }

    // ---- changes shared by the rule modules

    /// Adds to the relation value of both directions (stored symmetric), kept within the limit.
    pub fn change_relation(&mut self, rules: &DipRules, a: &str, b: &str, delta: f64) {
        for (x, y) in [(a, b), (b, a)] {
            let r = self.relation_mut(x, y);
            r.value = (r.value + delta).clamp(-rules.drift.relation_limit, rules.drift.relation_limit);
        }
    }

    /// Sets the relation value of both directions.
    pub fn set_relation(&mut self, rules: &DipRules, a: &str, b: &str, value: f64) {
        for (x, y) in [(a, b), (b, a)] {
            self.relation_mut(x, y).value = value.clamp(-rules.drift.relation_limit, rules.drift.relation_limit);
        }
    }
}

/// Everything the diplomacy rules read: the install's defines and the rules file's literals.
#[derive(Debug, Clone, PartialEq)]
pub struct DipRules {
    pub drift: DriftRules,
    pub influence: influence::InfluenceRules,
    pub treaties: treaties::TreatyRules,
    pub ai_influence: influence::AiInfluenceRules,
    /// The calendar, for the end dates of discredits and bans.
    pub cal: crate::calendar::Calendar,
}

impl DipRules {
    pub fn new(m: &EngineRules, defines: &Defines) -> Result<DipRules, String> {
        Ok(DipRules { drift: DriftRules::from_engine_rules(m)?, influence: influence::InfluenceRules::new(m, defines)?, treaties: treaties::TreatyRules::new(m, defines)?, ai_influence: influence::AiInfluenceRules::from_engine_rules(m)?, cal: crate::calendar::Calendar::from_engine_rules(m)? })
    }
}

/// A define of the install, or an error naming it.
pub(crate) fn define(d: &Defines, section: &str, key: &str) -> Result<f64, String> {
    // The named section first; installs file some keys elsewhere (RELATION_LIMIT_NO_ALLIANCE_OFFER is under `ai`).
    d.get(section, key).or_else(|| d.sections().iter().find_map(|(s, _)| d.get(s, key))).ok_or(format!("defines.{section}.{key} missing"))
}
