//! Alliances, military access, guarantees, vassals and substates, and the AI's answers to offers.

use wiktoria_data::defines::Defines;
use wiktoria_data::engine_rules::EngineRules;

use super::influence::{enter_sphere, leave_sphere};
use super::relations::pay_points;
use super::{define, DipRules, DiplomacyState, Subject, SubjectKind, Treaty};

#[derive(Debug, Clone, PartialEq)]
pub struct TreatyRules {
    pub alliance_accept: f64,
    pub alliance_decline: f64,
    pub alliance_cost: f64,
    pub cancel_alliance_relation: f64,
    pub cancel_alliance_cost: f64,
    pub ask_access_accept: f64,
    pub ask_access_decline: f64,
    pub ask_access_cost: f64,
    pub give_access_accept: f64,
    pub give_access_decline: f64,
    pub give_access_cost: f64,
    pub cancel_ask_access_relation: f64,
    pub cancel_give_access_relation: f64,
    pub cancel_access_cost: f64,
    pub call_ally_decline_prestige: f64,
    pub release_nation_infamy: f64,
    pub release_nation_prestige: f64,
    /// The AI will not ally below this relation.
    pub no_alliance_offer_below: f64,
    // rules file (hypotheses)
    pub release_relation: f64,
    pub ai: AiRules,
}

/// The AI's scores: fixed numbers, in the rules file as `FLD_DIP_AI_*`.
#[derive(Debug, Clone, PartialEq)]
pub struct AiRules {
    pub level_step: f64,
    pub level_offset: f64,
    pub different_government: f64,
    pub civ_vs_uncivilised: f64,
    pub uncivilised_vs_civ: f64,
    pub same_sphere: f64,
    pub other_sphere: f64,
    pub overlord: f64,
    pub infamy_weight: f64,
    pub badboy_limit: f64,
    pub hard: f64,
    pub relation_share: f64,
    pub hostile_clamp: f64,
    pub great_power_wants_great_ally: f64,
    pub conflict_unit: f64,
    pub ally_of_ally: f64,
    pub casus_belli: f64,
    /// Strength ladder: ratio thresholds low to high and the points each step gives.
    pub ladder_ratios: [f64; 4],
    pub ladder_points: [f64; 4],
    pub accept_min: f64,
    pub access_give_offset: f64,
    pub access_ally_bonus: f64,
}

impl TreatyRules {
    pub fn new(m: &EngineRules, d: &Defines) -> Result<TreatyRules, String> {
        let dip = |k: &str| define(d, "diplomacy", k);
        let w = |k: &str| crate::num_field(m, k);
        Ok(TreatyRules {
            alliance_accept: dip("ALLIANCE_RELATION_ON_ACCEPT")?,
            alliance_decline: dip("ALLIANCE_RELATION_ON_DECLINE")?,
            alliance_cost: dip("ALLIANCE_DIPLOMATIC_COST")?,
            cancel_alliance_relation: dip("CANCELALLIANCE_RELATION_ON_ACCEPT")?,
            cancel_alliance_cost: dip("CANCELALLIANCE_DIPLOMATIC_COST")?,
            ask_access_accept: dip("ASKMILACCESS_RELATION_ON_ACCEPT")?,
            ask_access_decline: dip("ASKMILACCESS_RELATION_ON_DECLINE")?,
            ask_access_cost: dip("ASKMILACCESS_DIPLOMATIC_COST")?,
            give_access_accept: dip("GIVEMILACCESS_RELATION_ON_ACCEPT")?,
            give_access_decline: dip("GIVEMILACCESS_RELATION_ON_DECLINE")?,
            give_access_cost: dip("GIVEMILACCESS_DIPLOMATIC_COST")?,
            cancel_ask_access_relation: dip("CANCELASKMILACCESS_RELATION_ON_ACCEPT")?,
            cancel_give_access_relation: dip("CANCELGIVEMILACCESS_RELATION_ON_ACCEPT")?,
            cancel_access_cost: dip("CANCELASKMILACCESS_DIPLOMATIC_COST")?,
            call_ally_decline_prestige: dip("DISHONORED_CALLALLY_PRESTIGE_PENALTY")?,
            release_nation_infamy: dip("RELEASE_NATION_INFAMY")?,
            release_nation_prestige: dip("RELEASE_NATION_PRESTIGE")?,
            no_alliance_offer_below: define(d, "pops", "RELATION_LIMIT_NO_ALLIANCE_OFFER")?,
            release_relation: w("FLD_DIP_RELEASE_RELATION")?,
            ai: AiRules {
                level_step: w("FLD_DIP_AI_LEVEL_STEP")?,
                level_offset: w("FLD_DIP_AI_LEVEL_OFFSET")?,
                different_government: w("FLD_DIP_AI_DIFFERENT_GOVERNMENT")?,
                civ_vs_uncivilised: w("FLD_DIP_AI_CIV_VS_UNCIV")?,
                uncivilised_vs_civ: w("FLD_DIP_AI_UNCIV_VS_CIV")?,
                same_sphere: w("FLD_DIP_AI_SAME_SPHERE")?,
                other_sphere: w("FLD_DIP_AI_OTHER_SPHERE")?,
                overlord: w("FLD_DIP_AI_OVERLORD")?,
                infamy_weight: w("FLD_DIP_AI_INFAMY_WEIGHT")?,
                badboy_limit: define(d, "country", "BADBOY_LIMIT")?,
                hard: w("FLD_DIP_AI_HARD")?,
                relation_share: w("FLD_DIP_AI_RELATION_SHARE")?,
                hostile_clamp: w("FLD_DIP_AI_HOSTILE_CLAMP")?,
                great_power_wants_great_ally: w("FLD_DIP_AI_GP_WANTS_GP")?,
                conflict_unit: w("FLD_DIP_AI_CONFLICT_UNIT")?,
                ally_of_ally: w("FLD_DIP_AI_ALLY_OF_ALLY")?,
                casus_belli: w("FLD_DIP_AI_CASUS_BELLI")?,
                ladder_ratios: [w("FLD_DIP_AI_LADDER_R1")?, w("FLD_DIP_AI_LADDER_R2")?, w("FLD_DIP_AI_LADDER_R3")?, w("FLD_DIP_AI_LADDER_R4")?],
                ladder_points: [w("FLD_DIP_AI_LADDER_P1")?, w("FLD_DIP_AI_LADDER_P2")?, w("FLD_DIP_AI_LADDER_P3")?, w("FLD_DIP_AI_LADDER_P4")?],
                accept_min: w("FLD_DIP_AI_ACCEPT_MIN")?,
                access_give_offset: w("FLD_DIP_AI_ACCESS_GIVE_OFFSET")?,
                access_ally_bonus: w("FLD_DIP_AI_ACCESS_ALLY_BONUS")?,
            },
        })
    }
}

// ---------------------------------------------------------------- alliances

/// The common gate and the class condition of an alliance offer from `a` to `t`
/// `at_war` = the two fight each other, `crisis` = a crisis runs.
pub fn can_offer_alliance(state: &DiplomacyState, rules: &DipRules, a: &str, t: &str, at_war: bool, crisis: bool) -> Result<(), &'static str> {
    if a == t {
        return Err("not to itself");
    }
    if at_war {
        return Err("at war with each other");
    }
    if state.relation(a, t).and_then(|r| r.banned_until).is_some() && state.date.zip(state.relation(a, t).and_then(|r| r.banned_until)).is_some_and(|(today, until)| today <= until) {
        return Err("embassy banned");
    }
    if state.is_subject(a) && state.overlord_of(a) != Some(t) {
        return Err("a subject may only ally its overlord");
    }
    if state.is_subject(t) && state.overlord_of(t) != Some(a) {
        return Err("a subject can only be addressed by its overlord");
    }
    if crisis && state.is_great_power(a) && state.is_great_power(t) {
        return Err("two great powers in a crisis");
    }
    if state.allied(a, t) {
        return Err("already allied");
    }
    if state.points(a) < rules.treaties.alliance_cost {
        return Err("not enough diplomatic points");
    }
    Ok(())
}

/// Accepting: one alliance object, +ALLIANCE_RELATION_ON_ACCEPT, one point paid.
pub fn make_alliance(state: &mut DiplomacyState, rules: &DipRules, a: &str, t: &str) {
    if !state.allied(a, t) {
        state.alliances.push(Treaty { start: state.date, ..Treaty::new(a, t) });
    }
    state.change_relation(rules, a, t, rules.treaties.alliance_accept);
    pay(state, rules, a, rules.treaties.alliance_cost);
}

/// A refused offer: relation change, point paid.
pub fn decline_alliance(state: &mut DiplomacyState, rules: &DipRules, a: &str, t: &str) {
    state.change_relation(rules, a, t, rules.treaties.alliance_decline);
    pay(state, rules, a, rules.treaties.alliance_cost);
}

/// Cancelling: needs an alliance, a free actor and no war fought on the same side.
pub fn can_cancel_alliance(state: &DiplomacyState, a: &str, t: &str, fought_together: bool) -> Result<(), &'static str> {
    if !state.allied(a, t) {
        return Err("no alliance");
    }
    if state.is_subject(a) {
        return Err("a subject cannot cancel");
    }
    if fought_together {
        return Err("they fought on the same side");
    }
    Ok(())
}

/// Deletes the alliance without cost (the script effect `leave_alliance`, a refused call, war).
pub fn end_alliance(state: &mut DiplomacyState, a: &str, t: &str) -> bool {
    let before = state.alliances.len();
    state.alliances.retain(|x| !x.joins(a, t));
    state.alliances.len() != before
}

/// The player's cancel: end it, the relation change and one point.
pub fn cancel_alliance(state: &mut DiplomacyState, rules: &DipRules, a: &str, t: &str) {
    if end_alliance(state, a, t) {
        state.change_relation(rules, a, t, rules.treaties.cancel_alliance_relation);
        pay(state, rules, a, rules.treaties.cancel_alliance_cost);
    }
}

/// `neutrality = yes`: every alliance of the country ends.
pub fn end_all_alliances(state: &mut DiplomacyState, tag: &str) {
    state.alliances.retain(|x| x.first != tag && x.second != tag);
}

fn pay(state: &mut DiplomacyState, rules: &DipRules, tag: &str, cost: f64) {
    if let Some(c) = state.countries.get_mut(tag) {
        c.points = pay_points(&rules.drift, c.points, cost);
    }
}

// ---------------------------------------------------------------- military access

/// `a` gets access through `t` (an accepted ask), or `t` gives it (the flag still sits at the
/// receiver's record towards the giver).
pub fn grant_access(state: &mut DiplomacyState, rules: &DipRules, receiver: &str, giver: &str, asked: bool) {
    state.relation_mut(receiver, giver).access = true;
    let (rel, cost, payer) = if asked { (rules.treaties.ask_access_accept, rules.treaties.ask_access_cost, receiver) } else { (rules.treaties.give_access_accept, rules.treaties.give_access_cost, giver) };
    state.change_relation(rules, receiver, giver, rel);
    pay(state, rules, payer, cost);
}

/// `end_military_access` effect and the two cancel actions: clears the flag.
pub fn end_access(state: &mut DiplomacyState, rules: &DipRules, receiver: &str, giver: &str, by_receiver: bool) -> bool {
    let had = state.has_access(receiver, giver);
    if had {
        state.relation_mut(receiver, giver).access = false;
        let (rel, payer) = if by_receiver { (rules.treaties.cancel_ask_access_relation, receiver) } else { (rules.treaties.cancel_give_access_relation, giver) };
        state.change_relation(rules, receiver, giver, rel);
        pay(state, rules, payer, rules.treaties.cancel_access_cost);
    }
    had
}

// ---------------------------------------------------------------- vassals

/// `create_vassal = T` from `overlord`: T's alliances and subject links go, a vassal link is made.
/// Returns false if T is already the overlord's subject.
pub fn create_vassal(state: &mut DiplomacyState, overlord: &str, t: &str) -> bool {
    if state.overlord_of(t) == Some(overlord) || overlord == t {
        return false;
    }
    end_all_alliances(state, t);
    state.subjects.retain(|s| s.subject != t && s.overlord != t);
    state.subjects.push(Subject { overlord: overlord.into(), subject: t.into(), kind: SubjectKind::Vassal, start: state.date, end: None });
    true
}

/// "Release nation" / `release_vassal`: a vassal link, relations raised, and a great power's
/// release goes into its sphere. Returns the infamy and prestige change for the releaser.
pub fn release_vassal(state: &mut DiplomacyState, rules: &DipRules, overlord: &str, t: &str, independent: bool) -> (f64, f64) {
    if !independent {
        state.subjects.retain(|s| s.subject != t);
        state.subjects.push(Subject { overlord: overlord.into(), subject: t.into(), kind: SubjectKind::Vassal, start: state.date, end: None });
        if state.is_great_power(overlord) {
            enter_sphere(state, overlord, t);
        }
    }
    state.change_relation(rules, overlord, t, rules.treaties.release_relation);
    (rules.treaties.release_nation_infamy, rules.treaties.release_nation_prestige)
}

/// Frees a subject from its overlord (`release_vassal` on a subject, a peace term, becoming a
/// great power): the link goes and nothing else.
pub fn end_vassal(state: &mut DiplomacyState, t: &str) -> bool {
    let before = state.subjects.len();
    state.subjects.retain(|s| s.subject != t);
    state.subjects.len() != before
}

/// Takes a country out of every treaty, link and sphere (annexation, loss of all land).
pub fn erase_country(state: &mut DiplomacyState, rules: &DipRules, t: &str) {
    state.alliances.retain(|x| x.first != t && x.second != t);
    state.guarantees.retain(|x| x.first != t && x.second != t);
    state.subjects.retain(|s| s.subject != t && s.overlord != t);
    leave_sphere(state, rules, t);
    state.relations.retain(|(a, b), _| a != t && b != t);
}

// ---------------------------------------------------------------- the AI's scores

/// Inputs of the base opinion S holds about O.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct OpinionInput {
    /// B: 1 or 0.5 by the caller.
    pub scale: f64,
    pub protect: f64,
    pub antagonize: f64,
    pub rival: f64,
    pub threat: f64,
    pub befriend: f64,
    pub relation: f64,
    /// Exactly one of the two is a great power: that power's opinion level.
    pub level_if_one_great: Option<u8>,
    pub different_government: bool,
    pub s_civilised: bool,
    pub o_civilised: bool,
    pub same_sphere: bool,
    pub other_sphere: bool,
    pub o_is_overlord: bool,
    pub o_infamy: f64,
    /// O is over the limit and played by a human.
    pub o_over_limit_human: bool,
}

pub fn base_opinion(r: &AiRules, i: &OpinionInput) -> f64 {
    let mut v = i.scale * ((i.protect - i.antagonize - i.rival + i.threat + i.befriend) / 4.0).clamp(-r.hostile_clamp, r.hostile_clamp);
    v += i.relation / r.relation_share;
    if let Some(l) = i.level_if_one_great {
        v += r.level_step * f64::from(l) - r.level_offset;
    }
    if i.different_government {
        v += r.different_government;
    }
    if i.s_civilised && !i.o_civilised {
        v += r.civ_vs_uncivilised;
    } else if !i.s_civilised && i.o_civilised {
        v += r.uncivilised_vs_civ;
    }
    if i.same_sphere {
        v += r.same_sphere;
    }
    if i.other_sphere {
        v += r.other_sphere;
    }
    if i.o_is_overlord {
        v += r.overlord;
    }
    v -= if i.o_over_limit_human { r.hard } else { r.infamy_weight * i.o_infamy / r.badboy_limit };
    v
}

/// Inputs of the alliance score beyond the base opinion.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AllianceInput {
    pub opinion: f64,
    pub overlord_of_vassal: bool,
    pub not_overlord_of_vassal: bool,
    pub other_leads_another_sphere: bool,
    pub at_war_with_sphere_leader: bool,
    pub other_holds_cores: bool,
    pub relation: f64,
    pub too_many_great_allies: bool,
    pub justifying_cb_on_other: bool,
    pub s_great: bool,
    pub other_great: bool,
    pub conflict_counter: f64,
    pub ally_of_ally: bool,
    /// The other's strength over ours (military score ratio).
    pub strength_ratio: f64,
    /// Sum of the war terms when negative (CURRENT_WARS).
    pub wars: f64,
    pub cb_either_way: u8,
}

/// The alliance score: hard results first, then the sum. An offer or acceptance needs
/// `accept_min`.
pub fn alliance_score(rules: &TreatyRules, i: &AllianceInput) -> f64 {
    let r = &rules.ai;
    if i.overlord_of_vassal {
        return r.hard;
    }
    if i.not_overlord_of_vassal || i.other_leads_another_sphere || i.at_war_with_sphere_leader || i.other_holds_cores || i.relation < rules.no_alliance_offer_below || i.too_many_great_allies || i.justifying_cb_on_other {
        return -r.hard;
    }
    let mut v = i.opinion;
    if i.s_great && i.other_great {
        v += r.great_power_wants_great_ally;
    }
    v -= r.conflict_unit * i.conflict_counter;
    if i.ally_of_ally {
        v += r.ally_of_ally;
    }
    // The ladder: the highest step whose ratio the strength ratio reaches.
    for (ratio, pts) in r.ladder_ratios.iter().zip(r.ladder_points.iter()).rev() {
        if i.strength_ratio >= *ratio {
            v += pts;
            break;
        }
    }
    v += i.wars.min(0.0);
    v -= r.casus_belli * f64::from(i.cb_either_way);
    v
}

pub fn ai_accepts_alliance(rules: &TreatyRules, i: &AllianceInput) -> bool {
    alliance_score(rules, i) >= rules.ai.accept_min
}

/// Ask access: the base opinion plus the side terms; +ally bonus; -hard for attacking an ally.
pub fn ask_access_score(rules: &TreatyRules, opinion: f64, side_terms: f64, allied: bool, attacking_ally: bool) -> f64 {
    if attacking_ally {
        return -rules.ai.hard;
    }
    opinion + side_terms + if allied { rules.ai.access_ally_bonus } else { 0.0 }
}

/// Give access: the ask score minus the offset; -hard for strategic interests.
pub fn give_access_score(rules: &TreatyRules, ask_score: f64, strategic_interest: bool) -> f64 {
    if strategic_interest {
        -rules.ai.hard
    } else {
        ask_score - rules.ai.access_give_offset
    }
}

pub fn ai_accepts_access(rules: &TreatyRules, score: f64) -> bool {
    score >= rules.ai.accept_min
}
