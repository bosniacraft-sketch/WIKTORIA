//! Diplomacy rules on synthetic numbers: defines are made up here, rules file literals come from
//! the repo's engine_rules.

use wiktoria_data::date::Date;
use wiktoria_data::defines::Defines;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_simulation::calendar::Calendar;
use wiktoria_simulation::diplomacy::influence::*;
use wiktoria_simulation::diplomacy::relations::*;
use wiktoria_simulation::diplomacy::treaties::*;
use wiktoria_simulation::diplomacy::{DipCountry, DipRules, DiplomacyState, SubjectKind};

const DEFINES: &str = r#"
defines = {
	country = {
		BASE_GREATPOWER_DAILY_INFLUENCE = 0.25,
		BADBOY_LIMIT = 25,
	},
	pops = {
		LARGE_POPULATION_LIMIT = 9000000,
		LARGE_POPULATION_INFLUENCE_PENALTY_CHUNK = 250000,
		RELATION_LIMIT_NO_ALLIANCE_OFFER = 0,
	},
	diplomacy = {
		MAX_INFLUENCE = 100,
		NEIGHBOUR_BONUS_INFLUENCE_PERCENT = 0.50,
		SPHERE_NEIGHBOUR_BONUS_INFLUENCE_PERCENT = 0.2,
		OTHER_CONTINENT_BONUS_INFLUENCE_PERCENT = -0.50,
		PUPPET_BONUS_INFLUENCE_PERCENT = 1.0,
		RELATION_INFLUENCE_MODIFIER = 1000,
		INVESTMENT_INFLUENCE_DEFENSE = 0.5,
		LARGE_POPULATION_INFLUENCE_PENALTY = -0.4,
		DISCREDIT_INFLUENCE_GAIN_FACTOR = -0.75,
		DISCREDIT_DAYS = 180,
		BANEMBASSY_DAYS = 365,
		REMOVEFROMSPHERE_PRESTIGE_COST = 10,
		REMOVEFROMSPHERE_INFAMY_COST = 1,
		INCREASEOPINION_RELATION_ON_ACCEPT = 20, INCREASEOPINION_INFLUENCE_COST = 50,
		DECREASEOPINION_RELATION_ON_ACCEPT = -10, DECREASEOPINION_INFLUENCE_COST = 50,
		ADDTOSPHERE_RELATION_ON_ACCEPT = 0, ADDTOSPHERE_INFLUENCE_COST = 100,
		REMOVEFROMSPHERE_RELATION_ON_ACCEPT = -10, REMOVEFROMSPHERE_INFLUENCE_COST = 100,
		DISCREDIT_RELATION_ON_ACCEPT = -5, DISCREDIT_INFLUENCE_COST = 25,
		EXPELADVISORS_RELATION_ON_ACCEPT = -5, EXPELADVISORS_INFLUENCE_COST = 50,
		BANEMBASSY_RELATION_ON_ACCEPT = -10, BANEMBASSY_INFLUENCE_COST = 65,
		ALLIANCE_RELATION_ON_ACCEPT = 100, ALLIANCE_RELATION_ON_DECLINE = -50, ALLIANCE_DIPLOMATIC_COST = 1,
		CANCELALLIANCE_RELATION_ON_ACCEPT = -20, CANCELALLIANCE_DIPLOMATIC_COST = 1,
		ASKMILACCESS_RELATION_ON_ACCEPT = 30, ASKMILACCESS_RELATION_ON_DECLINE = -10, ASKMILACCESS_DIPLOMATIC_COST = 1,
		GIVEMILACCESS_RELATION_ON_ACCEPT = 10, GIVEMILACCESS_RELATION_ON_DECLINE = 0, GIVEMILACCESS_DIPLOMATIC_COST = 1,
		CANCELASKMILACCESS_RELATION_ON_ACCEPT = 0, CANCELASKMILACCESS_DIPLOMATIC_COST = 1,
		CANCELGIVEMILACCESS_RELATION_ON_ACCEPT = -10,
		DISHONORED_CALLALLY_PRESTIGE_PENALTY = -3,
		RELEASE_NATION_INFAMY = -5, RELEASE_NATION_PRESTIGE = 0,
	},
}
"#;

fn rules() -> DipRules {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/engine_rules.json");
    let m = EngineRules::from_json(&std::fs::read_to_string(p).unwrap()).unwrap();
    DipRules::new(&m, &Defines::parse(DEFINES).unwrap()).unwrap()
}

fn cal() -> Calendar {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/engine_rules.json");
    Calendar::from_engine_rules(&EngineRules::from_json(&std::fs::read_to_string(p).unwrap()).unwrap()).unwrap()
}

fn d(y: i32, m: u8, dd: u8) -> Date {
    Date::new(y, m, dd)
}

/// GPA and GPB are great powers; SMA, SMB, SMC small countries.
fn world() -> DiplomacyState {
    let mut s = DiplomacyState::new(d(1836, 1, 2));
    for (t, gp) in [("GPA", true), ("GPB", true), ("SMA", false), ("SMB", false), ("SMC", false)] {
        s.countries.insert(t.into(), DipCountry { points: 5.0, great_power: gp, human: false });
    }
    s
}

#[test]
fn relations_are_symmetric_and_clamped_to_the_limit() {
    let (r, mut s) = (rules(), world());
    s.change_relation(&r, "SMA", "SMB", 150.0);
    s.change_relation(&r, "SMA", "SMB", 150.0);
    assert_eq!(s.relation_value("SMA", "SMB"), 200.0);
    assert_eq!(s.relation_value("SMB", "SMA"), 200.0);
    s.change_relation(&r, "SMA", "SMB", -1000.0);
    assert_eq!(s.relation_value("SMB", "SMA"), -200.0);
}

#[test]
fn monthly_drift_terms_and_the_halving() {
    let r = rules();
    let base = DriftInput::default();
    assert_eq!(drift_sum(&r.drift, &base), 0.0);
    let subject = DriftInput { subject_link: true, ..base };
    assert_eq!(drift_sum(&r.drift, &subject), 0.25);
    assert_eq!(monthly_change(&r.drift, &subject), 0.125);
    // war and a casus belli together
    let hostile = DriftInput { at_war: true, casus_belli: true, relation: 10.0, ..base };
    assert!((drift_sum(&r.drift, &hostile) - (-0.01 - 0.3)).abs() < 1e-12);
    // no peaceful-neighbour term while at war
    let war_neighbours = DriftInput { at_war: true, peaceful_neighbours: true, ..base };
    assert!((drift_sum(&r.drift, &war_neighbours) - (-0.01)).abs() < 1e-12);
    // above 100 a positive drift is 0, a negative one stays
    assert_eq!(drift_sum(&r.drift, &DriftInput { subject_link: true, relation: 150.0, ..base }), 0.0);
    // AI overlord and subject: plus 2, at least 2
    assert_eq!(drift_sum(&r.drift, &DriftInput { subject_link: true, ai_overlord: true, ..base }), 2.25);
    assert_eq!(drift_sum(&r.drift, &DriftInput { at_war: true, ai_overlord: true, ..base }), 2.0);
}

#[test]
fn diplomatic_points_gain_and_cap() {
    let r = rules();
    // 0.3 x (1 + 0.5 + 0.2) = 0.51
    assert!((monthly_points(&r.drift, 0.3, 0.5, 0.2, 1.0) - 1.51).abs() < 1e-12);
    assert_eq!(monthly_points(&r.drift, 0.3, 0.0, 0.0, 8.9), 9.0);
    assert_eq!(pay_points(&r.drift, 0.4, 1.0), 0.0);
}

fn gi() -> GainInput {
    GainInput { target_score: 100.0, power_score: 100.0, ..GainInput::default() }
}

#[test]
fn influence_gain_factor_terms() {
    let r = rules();
    // equal scores: 1 + (1 - 1) = 1
    assert!((gain_factor(&r.influence, &gi()) - 1.0).abs() < 1e-12);
    assert!((gain_factor(&r.influence, &GainInput { neighbour: true, ..gi() }) - 1.5).abs() < 1e-12);
    // a sphere neighbour counts only when it is no neighbour
    assert!((gain_factor(&r.influence, &GainInput { sphere_neighbour: true, ..gi() }) - 1.2).abs() < 1e-12);
    assert!((gain_factor(&r.influence, &GainInput { neighbour: true, sphere_neighbour: true, ..gi() }) - 1.5).abs() < 1e-12);
    assert!((gain_factor(&r.influence, &GainInput { other_continent: true, puppet: true, ..gi() }) - 1.5).abs() < 1e-12);
    // discredited: 1 - 0.75
    assert!((gain_factor(&r.influence, &GainInput { discredited: true, ..gi() }) - 0.25).abs() < 1e-12);
    // relation 200 / 1000
    assert!((gain_factor(&r.influence, &GainInput { relation: 200.0, ..gi() }) - 1.2).abs() < 1e-12);
    // investment: nobody invested = no penalty; a third of it = minus 0.5 x (1 - 1/3)
    assert!((gain_factor(&r.influence, &GainInput { investment: 1.0, foreign_investment: 3.0, ..gi() }) - (1.0 - 0.5 * (2.0 / 3.0))).abs() < 1e-12);
    // two whole chunks over the limit: 2 x -0.4
    assert!((gain_factor(&r.influence, &GainInput { population: 9_000_000.0 + 2.0 * 250_000.0 + 1.0, ..gi() }) - 0.2).abs() < 1e-12);
    // a much bigger target: ratio clamps to 2, 1 + (1 - 2) = 0
    assert_eq!(gain_factor(&r.influence, &GainInput { target_score: 900.0, ..gi() }), 0.0);
    // a divisor of 0 counts as 1; a tiny target
    assert!((gain_factor(&r.influence, &GainInput { target_score: 0.0, power_score: 0.0, ..gi() }) - 2.0).abs() < 1e-12);
}

#[test]
fn daily_amount_and_shares() {
    let r = rules();
    assert!((daily_amount(&r.influence, 0.2, 0.5) - 0.25 * 1.2 * 1.5).abs() < 1e-12);
    let sh = shares(1.0, &[("SMA".into(), 3), ("SMB".into(), 1), ("SMC".into(), 0)]);
    assert!((sh["SMA"] - 0.75).abs() < 1e-12 && (sh["SMB"] - 0.25).abs() < 1e-12 && sh["SMC"] == 0.0);
    // no priorities at all: equal split
    let eq = shares(0.9, &[("SMA".into(), 0), ("SMB".into(), 0), ("SMC".into(), 0)]);
    assert!(eq.values().all(|x| (x - 0.3).abs() < 1e-12));
}

#[test]
fn influence_stops_at_the_cap_and_pushes_the_rival_down() {
    let (r, mut s) = (rules(), world());
    assert_eq!(add_influence(&r.influence, 99.5, 2.0), (100.0, 1.5));
    assert_eq!(add_influence(&r.influence, 1.0, -5.0), (0.0, 0.0));
    s.relation_mut("GPA", "SMA").influence = 99.9;
    s.relation_mut("GPB", "SMA").influence = 40.0;
    s.priorities.insert(("GPA".into(), "SMA".into()), 1);
    let targets = [("SMA".to_string(), TargetInfo { owns_land: true, gain: gi() })].into_iter().collect();
    // amount 10 so the pair overflows by about 9.9
    let mut big = r.clone();
    big.influence.base_daily = 10.0;
    let gains = run_day(&mut s, &big, "GPA", d(1836, 1, 3), 0.0, 0.0, &targets);
    assert!((gains["SMA"] - 10.0).abs() < 1e-9);
    assert_eq!(s.influence("GPA", "SMA"), 100.0);
    assert!((s.influence("GPB", "SMA") - (40.0 - 9.9)).abs() < 1e-9);
}

#[test]
fn a_target_held_at_the_cap_without_rival_takes_no_share_and_bans_stop_gain() {
    let (r, mut s) = (rules(), world());
    s.relation_mut("GPA", "SMA").influence = 100.0;
    let today = d(1836, 1, 3);
    assert!(!takes_part(&s, &r.influence, "GPA", "SMA", today));
    s.relation_mut("GPB", "SMA").influence = 1.0;
    assert!(takes_part(&s, &r.influence, "GPA", "SMA", today));
    // a ban: no part, and a run zeroes the influence
    s.relation_mut("GPA", "SMB").influence = 20.0;
    s.relation_mut("GPA", "SMB").banned_until = Some(d(1836, 6, 1));
    assert!(!takes_part(&s, &r.influence, "GPA", "SMB", today));
    // truce blocks unless in the sphere (level above 4)
    s.relation_mut("GPA", "SMC").truce_until = Some(d(1837, 1, 1));
    assert!(!takes_part(&s, &r.influence, "GPA", "SMC", today));
    s.relation_mut("GPA", "SMC").level = level::IN_SPHERE;
    assert!(takes_part(&s, &r.influence, "GPA", "SMC", today));
}

#[test]
fn the_seven_actions_cost_and_effects() {
    let (r, mut s, c) = (rules(), world(), cal());
    let today = d(1836, 1, 3);
    // costs come from the defines
    let costs: Vec<f64> = Action::ALL.iter().map(|a| r.influence.cost(*a).influence).collect();
    assert_eq!(costs, vec![50.0, 50.0, 100.0, 100.0, 25.0, 50.0, 65.0]);

    // increase opinion: level + 1, +20 relation, 50 influence
    s.relation_mut("GPA", "SMA").influence = 60.0;
    do_action(&mut s, &r, &c, today, Action::IncreaseOpinion, "GPA", "SMA", None).unwrap();
    assert_eq!(s.level("GPA", "SMA"), level::CORDIAL);
    assert_eq!(s.relation_value("GPA", "SMA"), 20.0);
    assert_eq!(s.influence("GPA", "SMA"), 10.0);
    assert!(do_action(&mut s, &r, &c, today, Action::IncreaseOpinion, "GPA", "SMA", None).is_err());

    // add to sphere: needs friendly, 100 influence, no leader; level 5
    s.relation_mut("GPA", "SMB").influence = 100.0;
    assert_eq!(check_action(&s, &r, Action::AddToSphere, "GPA", "SMB", None), Err("level below friendly"));
    s.relation_mut("GPA", "SMB").level = level::FRIENDLY;
    do_action(&mut s, &r, &c, today, Action::AddToSphere, "GPA", "SMB", None).unwrap();
    assert_eq!(s.sphere_leader("SMB"), Some("GPA"));
    assert_eq!(s.influence("GPA", "SMB"), 0.0);
    // a second leader cannot add it, a great power cannot be added
    s.relation_mut("GPB", "SMB").influence = 100.0;
    s.relation_mut("GPB", "SMB").level = level::FRIENDLY;
    assert_eq!(check_action(&s, &r, Action::AddToSphere, "GPB", "SMB", None), Err("the target already has a sphere leader"));
    s.relation_mut("GPA", "GPB").influence = 100.0;
    s.relation_mut("GPA", "GPB").level = level::FRIENDLY;
    assert_eq!(check_action(&s, &r, Action::AddToSphere, "GPA", "GPB", None), Err("the target is a great power"));

    // remove from own sphere: infamy and prestige cost, leader level back to friendly
    s.relation_mut("GPA", "SMB").influence = 100.0;
    let fx = do_action(&mut s, &r, &c, today, Action::RemoveFromSphere, "GPA", "SMB", None).unwrap();
    assert_eq!(fx, ActionEffects { infamy: 1.0, prestige: -10.0 });
    assert_eq!(s.sphere_leader("SMB"), None);
    assert_eq!(s.level("GPA", "SMB"), level::FRIENDLY);

    // against a rival: discredit, expel, ban, decrease opinion
    s.relation_mut("GPA", "SMC").influence = 200.0;
    s.relation_mut("GPA", "SMC").level = level::FRIENDLY;
    s.relation_mut("GPB", "SMC").influence = 70.0;
    s.relation_mut("GPB", "SMC").level = level::CORDIAL;
    do_action(&mut s, &r, &c, today, Action::Discredit, "GPA", "SMC", Some("GPB")).unwrap();
    let x = s.relation("GPB", "SMC").unwrap();
    assert_eq!(x.discredited_by.as_deref(), Some("GPA"));
    assert_eq!(x.discredited_until, Some(d(1836, 7, 2)));
    do_action(&mut s, &r, &c, today, Action::DecreaseOpinion, "GPA", "SMC", Some("GPB")).unwrap();
    assert_eq!(s.level("GPB", "SMC"), level::NEUTRAL);
    do_action(&mut s, &r, &c, today, Action::ExpelAdvisors, "GPA", "SMC", Some("GPB")).unwrap();
    assert_eq!(s.influence("GPB", "SMC"), 0.0);
    do_action(&mut s, &r, &c, today, Action::BanEmbassy, "GPA", "SMC", Some("GPB")).unwrap();
    assert_eq!(s.relation("GPB", "SMC").unwrap().banned_until, Some(d(1837, 1, 3)));
    // a rival whose level is above the actor's cannot be targeted
    s.relation_mut("GPB", "SMC").level = level::IN_SPHERE;
    s.relation_mut("GPA", "SMC").influence = 100.0;
    assert_eq!(check_action(&s, &r, Action::Discredit, "GPA", "SMC", Some("GPB")), Err("the rival's level is above the actor's"));
}

#[test]
fn expelling_lifts_a_discredit_the_same_rival_put_on_the_actor() {
    let (r, mut s, c) = (rules(), world(), cal());
    s.relation_mut("GPA", "SMA").influence = 60.0;
    s.relation_mut("GPA", "SMA").level = level::CORDIAL;
    s.relation_mut("GPA", "SMA").discredited_until = Some(d(1836, 5, 1));
    s.relation_mut("GPA", "SMA").discredited_by = Some("GPB".into());
    s.relation_mut("GPB", "SMA").influence = 30.0;
    do_action(&mut s, &r, &c, d(1836, 1, 3), Action::ExpelAdvisors, "GPA", "SMA", Some("GPB")).unwrap();
    assert_eq!(s.relation("GPA", "SMA").unwrap().discredited_until, None);
}

#[test]
fn becoming_a_great_power_and_falling() {
    let (r, mut s) = (rules(), world());
    // SMA is a vassal in GPA's sphere; everybody holds influence in it
    treaties_vassal(&mut s, "GPA", "SMA");
    enter_sphere(&mut s, "GPA", "SMA");
    s.relation_mut("GPB", "SMA").influence = 33.0;
    s.relation_mut("GPB", "SMA").level = level::CORDIAL;
    becomes_great_power(&mut s, &r, "SMA");
    assert!(s.is_great_power("SMA"));
    assert_eq!(s.sphere_leader("SMA"), None);
    assert_eq!(s.overlord_of("SMA"), None);
    assert_eq!(s.influence("GPB", "SMA"), 0.0);
    assert_eq!(s.level("GPB", "SMA"), level::NEUTRAL);
    // the old leader's level was set to friendly on leaving, then reset to neutral by the wipe
    assert_eq!(s.level("GPA", "SMA"), level::NEUTRAL);

    // a fallen power loses its sphere: levels neutral, influence there 0, elsewhere kept
    enter_sphere(&mut s, "GPB", "SMB");
    s.relation_mut("GPB", "SMB").influence = 70.0;
    s.relation_mut("GPB", "SMC").influence = 20.0;
    dissolve_sphere(&mut s, "GPB");
    assert!(!s.is_great_power("GPB"));
    assert_eq!(s.sphere_leader("SMB"), None);
    assert_eq!(s.influence("GPB", "SMB"), 0.0);
    assert_eq!(s.influence("GPB", "SMC"), 20.0);
}

fn treaties_vassal(s: &mut DiplomacyState, o: &str, t: &str) {
    assert!(create_vassal(s, o, t));
}

#[test]
fn alliance_offer_gate_acceptance_cancel_and_end() {
    let (r, mut s) = (rules(), world());
    assert!(can_offer_alliance(&s, &r, "GPA", "SMA", false, false).is_ok());
    assert_eq!(can_offer_alliance(&s, &r, "GPA", "GPA", false, false), Err("not to itself"));
    assert_eq!(can_offer_alliance(&s, &r, "GPA", "SMA", true, false), Err("at war with each other"));
    assert_eq!(can_offer_alliance(&s, &r, "GPA", "GPB", false, true), Err("two great powers in a crisis"));
    make_alliance(&mut s, &r, "GPA", "SMA");
    assert!(s.allied("SMA", "GPA"));
    assert_eq!(s.relation_value("GPA", "SMA"), 100.0);
    assert_eq!(s.points("GPA"), 4.0);
    assert_eq!(can_offer_alliance(&s, &r, "GPA", "SMA", false, false), Err("already allied"));
    // a refused offer
    decline_alliance(&mut s, &r, "GPA", "SMB");
    assert_eq!(s.relation_value("GPA", "SMB"), -50.0);
    assert_eq!(s.points("GPA"), 3.0);
    // cannot cancel after fighting together
    assert_eq!(can_cancel_alliance(&s, "GPA", "SMA", true), Err("they fought on the same side"));
    assert!(can_cancel_alliance(&s, "GPA", "SMA", false).is_ok());
    cancel_alliance(&mut s, &r, "GPA", "SMA");
    assert!(!s.allied("GPA", "SMA"));
    assert_eq!(s.relation_value("GPA", "SMA"), 80.0);
    // no points: no offer
    s.countries.get_mut("SMC").unwrap().points = 0.0;
    assert_eq!(can_offer_alliance(&s, &r, "SMC", "SMA", false, false), Err("not enough diplomatic points"));
}

#[test]
fn subjects_may_only_deal_with_their_overlord() {
    let (r, mut s) = (rules(), world());
    assert!(create_vassal(&mut s, "GPA", "SMA"));
    assert!(!create_vassal(&mut s, "GPA", "SMA"));
    assert_eq!(can_offer_alliance(&s, &r, "SMA", "SMB", false, false), Err("a subject may only ally its overlord"));
    assert_eq!(can_offer_alliance(&s, &r, "SMB", "SMA", false, false), Err("a subject can only be addressed by its overlord"));
    assert!(can_offer_alliance(&s, &r, "GPA", "SMA", false, false).is_ok());
    assert_eq!(can_cancel_alliance(&s, "SMA", "GPA", false), Err("no alliance"));
}

#[test]
fn military_access_flags_costs_and_end() {
    let (r, mut s) = (rules(), world());
    grant_access(&mut s, &r, "GPA", "SMA", true);
    assert!(s.has_access("GPA", "SMA") && !s.has_access("SMA", "GPA"));
    assert_eq!(s.relation_value("GPA", "SMA"), 30.0);
    assert_eq!(s.points("GPA"), 4.0);
    grant_access(&mut s, &r, "SMB", "SMC", false);
    assert!(s.has_access("SMB", "SMC"));
    assert_eq!(s.points("SMC"), 4.0);
    assert!(end_access(&mut s, &r, "GPA", "SMA", true));
    assert!(!s.has_access("GPA", "SMA"));
    assert!(!end_access(&mut s, &r, "GPA", "SMA", true));
}

#[test]
fn making_and_ending_vassals() {
    let (r, mut s) = (rules(), world());
    make_alliance(&mut s, &r, "SMA", "SMB");
    // create_vassal drops every alliance and old link of the new vassal
    assert!(create_vassal(&mut s, "GPA", "SMA"));
    assert!(!s.allied("SMA", "SMB"));
    assert_eq!(s.overlord_of("SMA"), Some("GPA"));
    assert_eq!(s.subjects[0].kind, SubjectKind::Vassal);
    // release: a vassal link, the sphere for a great power, relations +500 clamped
    let (infamy, prestige) = release_vassal(&mut s, &r, "GPB", "SMC", false);
    assert_eq!((infamy, prestige), (-5.0, 0.0));
    assert_eq!(s.overlord_of("SMC"), Some("GPB"));
    assert_eq!(s.sphere_leader("SMC"), Some("GPB"));
    assert_eq!(s.relation_value("GPB", "SMC"), 200.0);
    assert!(end_vassal(&mut s, "SMC"));
    assert!(!end_vassal(&mut s, "SMC"));
    erase_country(&mut s, &r, "SMA");
    assert!(!s.is_subject("SMA"));
}

fn opinion() -> OpinionInput {
    OpinionInput { scale: 1.0, s_civilised: true, o_civilised: true, ..OpinionInput::default() }
}

#[test]
fn the_ai_base_opinion_terms() {
    let r = rules();
    let a = &r.treaties.ai;
    assert_eq!(base_opinion(a, &opinion()), 0.0);
    // relation / 4
    assert_eq!(base_opinion(a, &OpinionInput { relation: 80.0, ..opinion() }), 20.0);
    // the hostile lists: (befriend - antagonize) / 4 clamped to +-100, scaled by B
    assert_eq!(base_opinion(a, &OpinionInput { befriend: 40.0, antagonize: 8.0, ..opinion() }), 8.0);
    assert_eq!(base_opinion(a, &OpinionInput { befriend: 4000.0, scale: 0.5, ..opinion() }), 50.0);
    // one great power: 10 x level - 20 (friendly +20)
    assert_eq!(base_opinion(a, &OpinionInput { level_if_one_great: Some(4), ..opinion() }), 20.0);
    assert_eq!(base_opinion(a, &OpinionInput { level_if_one_great: Some(0), ..opinion() }), -20.0);
    assert_eq!(base_opinion(a, &OpinionInput { o_civilised: false, ..opinion() }), -80.0);
    assert_eq!(base_opinion(a, &OpinionInput { s_civilised: false, ..opinion() }), -50.0);
    assert_eq!(base_opinion(a, &OpinionInput { same_sphere: true, ..opinion() }), 25.0);
    assert_eq!(base_opinion(a, &OpinionInput { other_sphere: true, ..opinion() }), -25.0);
    assert_eq!(base_opinion(a, &OpinionInput { o_is_overlord: true, ..opinion() }), 1000.0);
    // infamy 12.5 of a limit of 25: minus half of 50
    assert_eq!(base_opinion(a, &OpinionInput { o_infamy: 12.5, ..opinion() }), -25.0);
    assert_eq!(base_opinion(a, &OpinionInput { o_over_limit_human: true, ..opinion() }), -1000.0);
}

fn alliance(opinion: f64) -> AllianceInput {
    AllianceInput { opinion, relation: 10.0, strength_ratio: 0.0, ..AllianceInput::default() }
}

#[test]
fn the_ai_alliance_score_hard_results_and_sum() {
    let r = rules();
    let t = &r.treaties;
    assert_eq!(alliance_score(t, &AllianceInput { overlord_of_vassal: true, ..alliance(0.0) }), 1000.0);
    for hard in [
        AllianceInput { not_overlord_of_vassal: true, ..alliance(500.0) },
        AllianceInput { other_leads_another_sphere: true, ..alliance(500.0) },
        AllianceInput { at_war_with_sphere_leader: true, ..alliance(500.0) },
        AllianceInput { other_holds_cores: true, ..alliance(500.0) },
        AllianceInput { relation: -1.0, ..alliance(500.0) },
        AllianceInput { too_many_great_allies: true, ..alliance(500.0) },
        AllianceInput { justifying_cb_on_other: true, ..alliance(500.0) },
    ] {
        assert_eq!(alliance_score(t, &hard), -1000.0);
        assert!(!ai_accepts_alliance(t, &hard));
    }
    // the sum: opinion 40, two great powers +50, conflict 2 x -10, ally of ally +20, a ladder step
    let sum = AllianceInput { s_great: true, other_great: true, conflict_counter: 2.0, ally_of_ally: true, ..alliance(40.0) };
    assert_eq!(alliance_score(t, &sum), 40.0 + 50.0 - 20.0 + 20.0);
    // war terms only count when negative; casus belli -100 each side
    assert_eq!(alliance_score(t, &AllianceInput { wars: 30.0, ..alliance(60.0) }), 60.0);
    assert_eq!(alliance_score(t, &AllianceInput { wars: -15.0, cb_either_way: 1, ..alliance(160.0) }), 45.0);
    // the threshold
    assert!(ai_accepts_alliance(t, &alliance(50.0)));
    assert!(!ai_accepts_alliance(t, &alliance(49.9)));
    // the ladder adds the highest step reached
    assert_eq!(alliance_score(t, &AllianceInput { strength_ratio: 2.0, ..alliance(0.0) }), t.ai.ladder_points[3]);
}

#[test]
fn the_ai_access_scores() {
    let r = rules();
    let t = &r.treaties;
    assert_eq!(ask_access_score(t, 10.0, 5.0, true, false), 115.0);
    assert_eq!(ask_access_score(t, 10.0, 5.0, true, true), -1000.0);
    assert_eq!(give_access_score(t, 115.0, false), 85.0);
    assert_eq!(give_access_score(t, 115.0, true), -1000.0);
    assert!(ai_accepts_access(t, 50.0) && !ai_accepts_access(t, 49.0));
}

#[test]
fn the_save_loader_reads_the_diplomacy_block_and_relation_records() {
    use std::collections::HashSet;
    let text = "date=\"1836.1.2\"\nGPA=\n{\n\tdiplomatic_points=3.500\n\tSMA=\n\t{\n\t\tvalue=50\n\t\tlevel=5\n\t\tinfluence_value=12.5\n\t\tmilitary_access=yes\n\t\ttruce_until=\"1840.1.1\"\n\t}\n}\nSMA=\n{\n}\ndiplomacy=\n{\n\tvassal=\n\t{\n\t\tfirst=\"GPA\"\n\t\tsecond=\"SMA\"\n\t\tstart_date=\"1800.1.1\"\n\t}\n\talliance=\n\t{\n\t\tfirst=\"GPA\"\n\t\tsecond=\"SMA\"\n\t}\n\tsubstate=\n\t{\n\t\tfirst=\"GPA\"\n\t\tsecond=\"SMB\"\n\t}\n}\ngreat_nations=\n{\n1 }\n";
    let save = wiktoria_data::save::parse_save(text.as_bytes(), &HashSet::new(), &HashSet::new()).unwrap();
    let s = DiplomacyState::from_save(&save);
    assert_eq!((s.alliances.len(), s.subjects.len()), (1, 2));
    assert!(s.is_substate("SMB") && !s.is_substate("SMA"));
    assert_eq!(s.overlord_of("SMA"), Some("GPA"));
    assert_eq!(s.points("GPA"), 3.5);
    assert!(s.is_great_power("GPA") && !s.is_great_power("SMA"));
    let r = s.relation("GPA", "SMA").unwrap();
    assert_eq!((r.value, r.level, r.influence, r.access), (50.0, 5, 12.5, true));
    assert_eq!(r.truce_until, Some(d(1840, 1, 1)));
    assert_eq!(s.sphere_leader("SMA"), Some("GPA"));
    assert!(s.truce_active("GPA", "SMA", d(1836, 1, 2)));
}

#[test]
fn the_save_loader_reads_the_influence_priorities() {
    use std::collections::HashSet;
    let text = "date=\"1836.2.9\"\nGPA=\n{\n\tinfluence=\n\t{\n\"SMA\"=\"3\"\n\"SMB\"=\"1\"\n\t}\n}\nSMA=\n{\n}\nSMB=\n{\n}\ngreat_nations=\n{\n1 }\n";
    let save = wiktoria_data::save::parse_save(text.as_bytes(), &HashSet::new(), &HashSet::new()).unwrap();
    let s = DiplomacyState::from_save(&save);
    assert_eq!(s.priorities.get(&("GPA".to_string(), "SMA".to_string())), Some(&3));
    assert_eq!(s.priorities.get(&("GPA".to_string(), "SMB".to_string())), Some(&1));
    assert_eq!(s.priorities.len(), 2);
}

#[test]
fn the_ai_round_comes_once_per_cycle_for_each_power() {
    let r = rules();
    let days = r.ai_influence.days;
    assert_eq!(days, 10);
    for p in ["GPA", "GPB", "ENG"] {
        let hits: Vec<u64> = (0..3 * days).filter(|d| ai_round(&r.ai_influence, p, *d)).collect();
        assert_eq!(hits.len(), 3);
        assert!(hits.windows(2).all(|w| w[1] - w[0] == days));
    }
}

fn target(neighbour: bool, relation: f64, other_continent: bool) -> TargetInfo {
    TargetInfo { owns_land: true, gain: GainInput { neighbour, relation, other_continent, population: 100_000.0, target_score: 1.0, power_score: 100.0, ..GainInput::default() } }
}

#[test]
fn the_ai_prefers_neighbours_it_likes_and_skips_its_safe_sphere() {
    let (r, mut s) = (rules(), world());
    let today = d(1836, 1, 10);
    let mut t = std::collections::BTreeMap::new();
    t.insert("SMA".to_string(), target(true, 100.0, false));
    t.insert("SMB".to_string(), target(false, 0.0, true));
    t.insert("SMC".to_string(), target(false, 50.0, false));
    t.insert("GPB".to_string(), target(true, 200.0, false));
    // score: factor + 0.5 neighbour + relation/200: SMA 2.59+0.5+0.5, SMC 2.04+0.25, SMB 1.49
    let p = ai_priorities(&s, &r, "GPA", today, 2, &t);
    assert_eq!(p, vec![("SMA".to_string(), 1), ("SMC".to_string(), 1)]);
    // its own sphere member drops out until a rival stands friendly there
    s.relation_mut("GPA", "SMA").level = level::IN_SPHERE;
    assert_eq!(ai_priorities(&s, &r, "GPA", today, 2, &t)[0].0, "SMC");
    s.relation_mut("GPB", "SMA").level = level::FRIENDLY;
    assert_eq!(ai_priorities(&s, &r, "GPA", today, 2, &t)[0].0, "SMA");
}

#[test]
fn the_ai_spends_on_the_next_step_and_discredits_rivals_with_a_priority() {
    let (r, mut s, c) = (rules(), world(), cal());
    let today = d(1836, 1, 10);
    // SMA: friendly, 100 points, no leader -> into the sphere
    s.relation_mut("GPA", "SMA").level = level::FRIENDLY;
    s.relation_mut("GPA", "SMA").influence = 100.0;
    // SMB: GPA's own sphere, GPB works on it -> GPA discredits GPB
    s.relation_mut("GPA", "SMB").level = level::IN_SPHERE;
    s.relation_mut("GPA", "SMB").influence = 30.0;
    s.relation_mut("GPB", "SMB").influence = 0.5;
    s.priorities.insert(("GPB".into(), "SMB".into()), 1);
    // SMC: on GPA's own list, GPA ahead of GPB -> saved for the next step
    s.relation_mut("GPA", "SMC").level = level::CORDIAL;
    s.relation_mut("GPA", "SMC").influence = 40.0;
    s.relation_mut("GPB", "SMC").influence = 10.0;
    s.priorities.insert(("GPA".into(), "SMC".into()), 1);
    s.priorities.insert(("GPB".into(), "SMC".into()), 1);
    let done = ai_spend(&mut s, &r, &c, today, "GPA");
    let what: Vec<(Action, &str)> = done.iter().map(|(a, t, _, _)| (*a, t.as_str())).collect();
    assert_eq!(what, vec![(Action::AddToSphere, "SMA"), (Action::Discredit, "SMB")]);
    assert_eq!(s.sphere_leader("SMA"), Some("GPA"));
    assert_eq!(s.relation("GPB", "SMB").unwrap().discredited_by.as_deref(), Some("GPA"));
    assert_eq!(s.influence("GPA", "SMC"), 40.0);
    // once GPB is ahead in SMC, GPA discredits it there too
    s.relation_mut("GPB", "SMC").influence = 45.0;
    let done = ai_spend(&mut s, &r, &c, today, "GPA");
    assert_eq!(done.iter().map(|(a, t, g, _)| (*a, t.as_str(), g.as_deref())).collect::<Vec<_>>(), vec![(Action::Discredit, "SMC", Some("GPB"))]);
    // a rival without a priority there is left alone
    s.relation_mut("GPA", "SMB").influence = 30.0;
    s.relation_mut("GPB", "SMB").discredited_until = None;
    s.priorities.remove(&("GPB".to_string(), "SMB".to_string()));
    assert!(ai_spend(&mut s, &r, &c, today, "GPA").is_empty());
}
