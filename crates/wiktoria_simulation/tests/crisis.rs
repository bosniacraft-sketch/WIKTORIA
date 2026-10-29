//! Flashpoints and crises on synthetic numbers: the defines are made up here (with the
//! install's values), rules file literals come from the repo's engine_rules.

use std::collections::HashSet;

use wiktoria_data::date::Date;
use wiktoria_data::defines::Defines;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_simulation::diplomacy::crisis::*;

const DEFINES: &str = r#"
defines = {
	diplomacy = {
		TENSION_DECAY = -0.08,
		TENSION_FROM_CB = 0.04,
		TENSION_FROM_MOVEMENT = 0.02,
		TENSION_FROM_MOVEMENT_MAX = 1.2,
		AT_WAR_TENSION_DECAY = -0.12,
		TENSION_ON_CB_DISCOVERED = 20,
		TENSION_ON_REVOLT = 50,
		TENSION_WHILE_CRISIS = -1.0,
		CRISIS_COOLDOWN_MONTHS = 60,
		CRISIS_BASE_CHANCE = 20,
		CRISIS_TEMPERATURE_INCREASE = 0.05,
		CRISIS_TEMPERATURE_PARTICIPANT_FACTOR = 10,
		CRISIS_TEMPERATURE_ON_MOBILIZE = 10,
		CRISIS_INTEREST_WAR_EXHAUSTION_LIMIT = 20,
		CRISIS_DID_NOT_TAKE_SIDE_PRESTIGE_FACTOR_BASE = -0.1,
		CRISIS_DID_NOT_TAKE_SIDE_PRESTIGE_FACTOR_YEAR = -0.002,
		CRISIS_WINNER_PRESTIGE_FACTOR_BASE = 10,
		CRISIS_WINNER_PRESTIGE_FACTOR_YEAR = 0.4,
		CRISIS_WINNER_RELATIONS_IMPACT = 25,
		RANK_1_TENSION_DECAY = -0.5, RANK_2_TENSION_DECAY = -0.4, RANK_3_TENSION_DECAY = -0.3, RANK_4_TENSION_DECAY = -0.2,
		RANK_5_TENSION_DECAY = -0.08, RANK_6_TENSION_DECAY = -0.06, RANK_7_TENSION_DECAY = -0.04, RANK_8_TENSION_DECAY = -0.02,
	},
}
"#;

fn rules() -> CrisisRules {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/engine_rules.json");
    let m = EngineRules::from_json(&std::fs::read_to_string(p).unwrap()).unwrap();
    CrisisRules::new(&m, &Defines::parse(DEFINES).unwrap()).unwrap()
}

fn d(y: i32, m: u8, dd: u8) -> Date {
    Date::new(y, m, dd)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn the_save_gives_the_flashpoints_and_the_crisis_manager() {
    let text = "date=\"1846.1.1\"\nTUR=\n{\n\tstate=\n\t{\n\t\tid=\n\t\t{\n\t\t\tid=1\n\t\t\ttype=47\n\t\t}\n\t\tprovinces=\n\t\t{\n843 844 \t\t}\n\t\tflashpoint=yes\n\t\tcrisis=\"GRE\"\n\t}\n\tstate=\n\t{\n\t\tprovinces=\n\t\t{\n900 \t\t}\n\t}\n}\nGRE=\n{\n}\ncrisis_manager=\n{\n\tdate=\"1840.4.23\"\n\tcrisis=\n\t{\n\t\ttemperature=2.636\n\t\tstate=843\n\t\tattacker=\"GRE\"\n\t\tdefender=\"TUR\"\n\t\tattacker_backer=\"RUS\"\n\t\tinterested=\"ENG\"\n\t\tinterested=\"FRA\"\n\t}\n}\n";
    let save = wiktoria_data::save::parse_save(text.as_bytes(), &HashSet::new(), &HashSet::new()).unwrap();
    let s = CrisisState::from_save(&save);
    assert_eq!(s.flashpoints, vec![Flashpoint { province: 843, owner: "TUR".into(), tag: "GRE".into(), tension: 0.0 }]);
    assert_eq!(s.last_end, Some(d(1840, 4, 23)));
    let c = s.crisis.unwrap();
    assert_eq!((c.province, c.attacker.as_str(), c.defender.as_str()), (843, "GRE", "TUR"));
    assert_eq!((c.attacker_backers, c.interested), (vec!["RUS".to_string()], vec!["ENG".to_string(), "FRA".to_string()]));
    assert!(close(c.temperature, 2.636));
}

#[test]
fn tension_terms_add_up() {
    let r = rules();
    // decay only
    assert!(close(tension_change(&r, &TensionInput::default()), -0.08));
    // owner ranked 1st: the great powers' extra decay
    assert!(close(tension_change(&r, &TensionInput { owner_rank: 1, ..Default::default() }), -0.58));
    // two casus belli and a movement of 30,000 (0.6), at war share one half
    let i = TensionInput { owner_rank: 7, casus_belli: 2, movement: 30_000.0, at_war_share: 0.5, crisis_running: false };
    assert!(close(tension_change(&r, &i), -0.08 - 0.04 + 0.08 + 0.6 - 0.06));
    // a movement of 234,000 is capped at 1.2; a running crisis takes 1 a day
    let i = TensionInput { movement: 234_000.0, crisis_running: true, ..Default::default() };
    assert!(close(tension_change(&r, &i), -0.08 + 1.2 - 1.0));
    let mut f = Flashpoint { province: 1, owner: "TUR".into(), tag: "GRE".into(), tension: 99.5 };
    add_tension(&r, &mut f, 1.12);
    assert_eq!(f.tension, r.tension_max);
    add_tension(&r, &mut f, -500.0);
    assert_eq!(f.tension, 0.0);
}

#[test]
fn a_crisis_waits_for_the_cooldown_and_full_tension_at_an_existing_tag() {
    let r = rules();
    let mut s = CrisisState { last_end: Some(d(1846, 4, 23)), ..Default::default() };
    assert!(!cooldown_over(&r, &s, d(1851, 4, 22)));
    assert!(cooldown_over(&r, &s, d(1851, 4, 23)));
    s.last_end = None;
    assert!(cooldown_over(&r, &s, d(1836, 1, 1)));
    assert!(close(start_chance(&r), 20.0 / 365.0));
    s.flashpoints = vec![
        Flashpoint { province: 1, owner: "TUR".into(), tag: "GRE".into(), tension: 100.0 },
        Flashpoint { province: 2, owner: "ENG".into(), tag: "IRE".into(), tension: 100.0 },
        Flashpoint { province: 3, owner: "TUR".into(), tag: "BUL".into(), tension: 80.0 },
    ];
    let exists = |t: &str| t == "GRE" || t == "BUL";
    assert_eq!(crisis_candidate(&r, &s, &exists).map(|f| f.province), Some(1));
    s.flashpoints[0].tension = 99.0;
    // IRE does not exist (a country to liberate: no crisis, STAND-IN), BUL is not at full tension
    assert_eq!(crisis_candidate(&r, &s, &exists), None);
}

#[test]
fn temperature_rises_faster_with_sides_taken_and_answers_follow_ties_and_relations() {
    let r = rules();
    let f = Flashpoint { province: 843, owner: "TUR".into(), tag: "GRE".into(), tension: 100.0 };
    let mut c = start_crisis(&r, &f, "RUS", vec!["ENG".into(), "FRA".into(), "AUS".into(), "PRU".into(), "NET".into()], d(1845, 12, 24));
    assert_eq!(c.temperature, 2.5);
    // one of six has taken a side: 0.05 x (1 + 10/6)
    assert!(close(temperature_rise(&r, &c), 0.05 * (1.0 + 10.0 / 6.0)));
    let a = |ra: f64, rd: f64, ta: bool, td: bool| answer(&AnswerInput { relation_attacker: ra, relation_defender: rd, tied_attacker: ta, tied_defender: td });
    assert_eq!(a(-50.0, 100.0, true, false), Some(Side::Attacker));
    assert_eq!(a(100.0, -50.0, false, true), Some(Side::Defender));
    assert_eq!(a(10.0, 50.0, false, false), Some(Side::Defender));
    assert_eq!(a(-10.0, -50.0, false, false), None);
    record_answer(&mut c, "ENG", Some(Side::Defender));
    record_answer(&mut c, "FRA", None);
    assert_eq!((c.participants(), c.interested.len(), c.declined.len()), (2, 3, 1));
    assert!(close(temperature_rise(&r, &c), 0.05 * (1.0 + 10.0 * 2.0 / 6.0)));
    assert!(c.involves("ENG") && c.involves("GRE") && !c.involves("FRA"));
    assert_eq!(may_be_interested(&r, &[("ENG".into(), 0.0, false), ("FRA".into(), 25.0, false), ("AUS".into(), 0.0, true), ("TUR".into(), 0.0, false)], "GRE", "TUR"), vec!["ENG".to_string()]);
}

#[test]
fn the_weaker_side_gives_in_and_a_defender_that_does_cedes_the_state() {
    let r = rules();
    let f = Flashpoint { province: 843, owner: "TUR".into(), tag: "GRE".into(), tension: 100.0 };
    let mut c = start_crisis(&r, &f, "RUS", vec!["ENG".into(), "FRA".into()], d(1845, 12, 24));
    record_answer(&mut c, "ENG", Some(Side::Defender));
    record_answer(&mut c, "FRA", None);
    assert!(!settles(&r, &c));
    c.temperature = r.settle_temperature;
    assert!(settles(&r, &c));
    let strength = |t: &str| match t {
        "RUS" => 60.0,
        "GRE" => 5.0,
        "TUR" => 20.0,
        "ENG" => 30.0,
        _ => 0.0,
    };
    let prestige = |t: &str| if t == "FRA" { 100.0 } else { 0.0 };
    let s = settle(&r, &c, d(1846, 4, 23), &strength, &prestige);
    // GRE + RUS 65 against TUR + ENG 50: the defender gives in, the state is ceded
    assert_eq!(s.winner, Side::Attacker);
    assert!(s.state_ceded);
    // winners: 10 + 0.4 x 10 years; FRA stayed out: 100 x (-0.1 - 0.002 x 10)
    assert!(close(s.prestige["GRE"], 14.0) && close(s.prestige["RUS"], 14.0));
    assert!(close(s.prestige["FRA"], -12.0));
    assert!(!s.prestige.contains_key("TUR"));
    assert_eq!(s.relations, vec![("RUS".to_string(), "GRE".to_string(), 25.0), ("ENG".to_string(), "TUR".to_string(), -25.0)]);
    // a stronger defender keeps the state
    let s = settle(&r, &c, d(1846, 4, 23), &|t| if t == "TUR" { 100.0 } else { strength(t) }, &prestige);
    assert_eq!(s.winner, Side::Defender);
    assert!(!s.state_ceded);
}
