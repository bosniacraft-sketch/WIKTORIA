//! Relation values, their monthly drift and the diplomatic points.
//!
//! Relations are stored symmetric: a change is applied to both directions and clamped to
//! `+-FLD_RELATION_LIMIT`. Every constant here is an input, never a literal: the engine's own
//! numbers come from the rules file (`FLD_DIP_*`, hypotheses) and the install's `defines.lua`.

/// The opinion levels a country's relation record holds (the level of A towards B).
pub mod level {
    pub const HOSTILE: u8 = 0;
    pub const OPPOSED: u8 = 1;
    pub const NEUTRAL: u8 = 2;
    pub const CORDIAL: u8 = 3;
    pub const FRIENDLY: u8 = 4;
    pub const IN_SPHERE: u8 = 5;
}

/// A's view of B.
#[derive(Debug, Clone, PartialEq)]
pub struct Relation {
    pub value: f64,
    /// Opinion level (`level::*`); a record without one is neutral.
    pub level: u8,
    /// Influence points A holds in B.
    pub influence: f64,
    /// A has military access through B.
    pub access: bool,
    pub truce_until: Option<wiktoria_data::date::Date>,
    pub discredited_until: Option<wiktoria_data::date::Date>,
    pub discredited_by: Option<String>,
    pub banned_until: Option<wiktoria_data::date::Date>,
    pub last_war: Option<wiktoria_data::date::Date>,
}

impl Default for Relation {
    fn default() -> Self {
        Relation { value: 0.0, level: level::NEUTRAL, influence: 0.0, access: false, truce_until: None, discredited_until: None, discredited_by: None, banned_until: None, last_war: None }
    }
}

/// What the monthly drift of one pair depends on.
#[derive(Debug, Clone, Copy, Default)]
pub struct DriftInput {
    pub relation: f64,
    /// A subject link between the two, either way.
    pub subject_link: bool,
    pub at_war: bool,
    /// The neighbour flag of the pair and nobody at war.
    pub peaceful_neighbours: bool,
    pub guarantee: bool,
    /// Military access, either way.
    pub access: bool,
    /// One holds a usable casus belli on the other and the relation is above 0.
    pub casus_belli: bool,
    /// Neither is played by a human and one is the other's overlord.
    pub ai_overlord: bool,
}

/// The rules file's drift terms (`FLD_DIP_DRIFT_*`, all hypotheses until measured).
#[derive(Debug, Clone, PartialEq)]
pub struct DriftRules {
    pub subject: f64,
    pub war: f64,
    pub peaceful_neighbour: f64,
    pub guarantee: f64,
    pub access: f64,
    pub casus_belli: f64,
    /// The share of the summed drift that is applied.
    pub share: f64,
    /// Above this relation a positive drift counts as 0.
    pub cap: f64,
    /// AI overlord and subject: added to the drift, and the least it may be afterwards.
    pub ai_overlord: f64,
    pub relation_limit: f64,
    pub points_max: f64,
}

impl DriftRules {
    /// Rules file literals (`FLD_DIP_DRIFT_*`, hypotheses) and the relation limit.
    pub fn from_engine_rules(m: &wiktoria_data::engine_rules::EngineRules) -> Result<DriftRules, String> {
        let w = |k: &str| crate::num_field(m, k);
        Ok(DriftRules {
            subject: w("FLD_DIP_DRIFT_SUBJECT")?,
            war: w("FLD_DIP_DRIFT_WAR")?,
            peaceful_neighbour: w("FLD_DIP_DRIFT_NEIGHBOUR")?,
            guarantee: w("FLD_DIP_DRIFT_GUARANTEE")?,
            access: w("FLD_DIP_DRIFT_ACCESS")?,
            casus_belli: w("FLD_DIP_DRIFT_CASUS_BELLI")?,
            share: w("FLD_DIP_DRIFT_SHARE")?,
            cap: w("FLD_DIP_DRIFT_CAP")?,
            ai_overlord: w("FLD_DIP_DRIFT_AI_OVERLORD")?,
            relation_limit: w("FLD_RELATION_LIMIT")?,
            points_max: w("FLD_DIP_POINTS_MAX")?,
        })
    }
}

/// Summed drift d of one pair for one month (before halving).
pub fn drift_sum(r: &DriftRules, i: &DriftInput) -> f64 {
    let mut d = 0.0;
    if i.subject_link {
        d += r.subject;
    }
    if i.at_war {
        d += r.war;
    }
    if i.peaceful_neighbours && !i.at_war {
        d += r.peaceful_neighbour;
    }
    if i.guarantee {
        d += r.guarantee;
    }
    if i.access {
        d += r.access;
    }
    if i.casus_belli && i.relation > 0.0 {
        d += r.casus_belli;
    }
    if i.relation > r.cap && d > 0.0 {
        d = 0.0;
    }
    if i.ai_overlord {
        d = (d + r.ai_overlord).max(r.ai_overlord);
    }
    d
}

/// The change of the relation value in one month: half the summed drift.
pub fn monthly_change(r: &DriftRules, i: &DriftInput) -> f64 {
    drift_sum(r, i) * r.share
}

/// The value after adding `delta`, kept within the limits.
pub fn clamp_relation(r: &DriftRules, value: f64) -> f64 {
    value.clamp(-r.relation_limit, r.relation_limit)
}

/// Diplomatic points after one month: `BASE_MONTHLY_DIPLOPOINTS x (1 + tech + modifier)` added,
/// kept between 0 and the maximum.
pub fn monthly_points(r: &DriftRules, base_monthly: f64, tech_sum: f64, modifier: f64, current: f64) -> f64 {
    (current + base_monthly * (1.0 + tech_sum + modifier)).clamp(0.0, r.points_max)
}

/// Points after paying `cost`; kept between 0 and the maximum.
pub fn pay_points(r: &DriftRules, current: f64, cost: f64) -> f64 {
    (current - cost).clamp(0.0, r.points_max)
}
