//! Siege and occupation rules on synthetic fixtures.

use wiktoria_data::date::Date;
use wiktoria_data::defines::Defines;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_simulation::military::siege::*;

fn rules() -> SiegeRules {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/engine_rules.json");
    let defines = Defines::parse(
        "defines = {\n military = {\n ENGINEER_UNIT_RATIO = 0.1,\n RECON_UNIT_RATIO = 0.1,\n RECON_SIEGE_EFFECT = 0.1,\n SIEGE_BRIGADES_MIN = 4,\n SIEGE_BRIGADES_MAX = 8,\n SIEGE_BRIGADES_BONUS = 0.25,\n POP_SIZE_PER_REGIMENT = 3000,\n }\n}",
    )
    .unwrap();
    SiegeRules::from_engine_rules(&EngineRules::from_json(&std::fs::read_to_string(p).unwrap()).unwrap(), &defines).unwrap()
}

fn reg(strength: f64, siege: f64, recon: f64) -> SiegeRegiment {
    SiegeRegiment { strength, siege, recon }
}

fn army(n: usize) -> Vec<SiegeRegiment> {
    (0..n).map(|_| reg(3.0, 0.0, 0.0)).collect()
}

fn prov(owner: &str, controller: &str, fort: f64) -> ProvinceMilitary {
    ProvinceMilitary { owner: Some(owner.into()), controller: Some(controller.into()), garrison: 100.0, fort, ..Default::default() }
}

fn war(a: &str, b: &str) -> bool {
    a != b
}

fn ctx<'a>(hostile: &'a dyn Fn(&str, &str) -> bool, core: &'a dyn Fn(&str) -> bool, over: &'a dyn Fn(&str) -> Option<String>) -> SiegeContext<'a> {
    SiegeContext { hostile, core_of: core, overlord: over }
}

const DAY: Date = Date { year: 1840, month: 1, day: 1 };

#[test]
fn die_zero_gives_no_progress_and_five_is_full() {
    let r = rules();
    let a = army(6);
    assert_eq!(siege_progress(&r, 0.0, &a, 0, false, false), 0.0);
    let full = siege_progress(&r, 0.0, &a, 5, false, false);
    assert!(full > 0.0);
    assert!((siege_progress(&r, 0.0, &a, 9, false, false) / full - 1.25).abs() < 1e-9);
    assert!((siege_progress(&r, 0.0, &a, 1, false, false) / full - 0.2).abs() < 1e-9);
}

#[test]
fn forts_slow_and_engineers_beat_forts() {
    let r = rules();
    let a = army(6);
    let p0 = siege_progress(&r, 0.0, &a, 5, false, false);
    let p3 = siege_progress(&r, 3.0, &a, 5, false, false);
    assert!(p3 < p0);
    let mut with_eng = army(5);
    with_eng.push(reg(3.0, 2.0, 0.0));
    let pe = siege_progress(&r, 3.0, &with_eng, 5, false, false);
    assert!(pe > siege_progress(&r, 3.0, &army(6), 5, false, false));
}

#[test]
fn more_brigades_help_up_to_the_maximum_and_recon_helps() {
    let r = rules();
    let p5 = siege_progress(&r, 0.0, &army(5), 5, false, false);
    let p6 = siege_progress(&r, 0.0, &army(6), 5, false, false);
    let p20 = siege_progress(&r, 0.0, &army(20), 5, false, false);
    let p30 = siege_progress(&r, 0.0, &army(30), 5, false, false);
    assert!(p6 > p5);
    assert_eq!(p20, p30);
    let mut scouts = army(5);
    scouts.push(reg(3.0, 0.0, 2.0));
    assert!(siege_progress(&r, 0.0, &scouts, 5, false, false) > p6 * 0.0 + siege_progress(&r, 0.0, &army(6), 5, false, false));
}

#[test]
fn owner_and_core_bonuses() {
    let r = rules();
    let a = army(6);
    let base = siege_progress(&r, 0.0, &a, 5, false, false);
    assert!((siege_progress(&r, 0.0, &a, 5, true, false) / base - r.owner_bonus).abs() < 1e-9);
    assert!((siege_progress(&r, 0.0, &a, 5, false, true) / base - r.core_bonus).abs() < 1e-9);
    assert!((siege_progress(&r, 0.0, &a, 5, true, true) / base - r.owner_bonus).abs() < 1e-9);
}

fn run_until_captured(p: &mut ProvinceMilitary, besiegers: &[Besieger], c: &SiegeContext) -> (SiegeEvent, u32) {
    let r = rules();
    let mut rng = SiegeRng(7);
    for day in 1..2000 {
        let e = siege_day(&r, p, besiegers, c, &mut rng, DAY);
        if !matches!(e, SiegeEvent::Continues(_)) {
            return (e, day);
        }
    }
    panic!("siege never ended");
}

#[test]
fn garrison_zero_changes_controller_and_resets_garrison() {
    let mut p = prov("AAA", "AAA", 1.0);
    let b = [Besieger { country: "BBB".into(), regiments: army(8) }];
    let (h, c, o) = (&war as &dyn Fn(&str, &str) -> bool, &(|_: &str| false) as &dyn Fn(&str) -> bool, &(|_: &str| None) as &dyn Fn(&str) -> Option<String>);
    let (e, days) = run_until_captured(&mut p, &b, &ctx(h, c, o));
    assert!(matches!(e, SiegeEvent::Captured { ref new_controller, liberated: false, .. } if new_controller == "BBB"));
    assert_eq!(p.controller.as_deref(), Some("BBB"));
    assert_eq!(p.garrison, 100.0);
    assert_eq!(p.last_controller_change, Some(DAY));
    assert!(p.is_occupied() && p.rgo_workers_leave());
    assert!(days > 1);
}

#[test]
fn owner_liberates_and_others_earn_relations() {
    let mut p = prov("AAA", "BBB", 0.0);
    let b = [Besieger { country: "AAA".into(), regiments: army(8) }, Besieger { country: "CCC".into(), regiments: army(2) }];
    let (h, c, o) = (&war as &dyn Fn(&str, &str) -> bool, &(|_: &str| false) as &dyn Fn(&str) -> bool, &(|_: &str| None) as &dyn Fn(&str) -> Option<String>);
    let (e, _) = run_until_captured(&mut p, &b, &ctx(h, c, o));
    assert_eq!(e, SiegeEvent::Captured { new_controller: "AAA".into(), liberated: true, relations: vec!["CCC".into()] });
    assert!(!p.is_occupied() && !p.rgo_workers_leave());
}

#[test]
fn ally_of_the_owner_hands_province_back() {
    let mut p = prov("AAA", "BBB", 0.0);
    let b = [Besieger { country: "CCC".into(), regiments: army(8) }];
    // CCC fights BBB but is not hostile to AAA.
    let hostile = |a: &str, b: &str| a != b && !(a == "CCC" && b == "AAA");
    let (e, _) = run_until_captured(&mut p, &b, &ctx(&hostile, &|_| false, &|_| None));
    assert!(matches!(e, SiegeEvent::Captured { liberated: true, .. }));
    assert_eq!(p.controller.as_deref(), Some("AAA"));
}

#[test]
fn vassal_siege_goes_to_overlord() {
    let mut p = prov("AAA", "AAA", 0.0);
    let b = [Besieger { country: "VAS".into(), regiments: army(8) }];
    let over = |c: &str| (c == "VAS").then(|| "BIG".to_string());
    let (e, _) = run_until_captured(&mut p, &b, &ctx(&war, &|_| false, &over));
    assert!(matches!(e, SiegeEvent::Captured { ref new_controller, .. } if new_controller == "BIG"));
}

#[test]
fn siege_ends_when_not_hostile_or_own_controller() {
    let r = rules();
    let mut rng = SiegeRng(1);
    let b = [Besieger { country: "BBB".into(), regiments: army(6) }];
    let mut p = prov("AAA", "BBB", 0.0);
    assert_eq!(siege_day(&r, &mut p, &b, &ctx(&war, &|_| false, &|_| None), &mut rng, DAY), SiegeEvent::Ended);
    let mut q = prov("AAA", "AAA", 0.0);
    assert_eq!(siege_day(&r, &mut q, &b, &ctx(&|_, _| false, &|_| false, &|_| None), &mut rng, DAY), SiegeEvent::Ended);
    assert_eq!(siege_day(&r, &mut q, &[], &ctx(&war, &|_| false, &|_| None), &mut rng, DAY), SiegeEvent::Ended);
}

#[test]
fn same_seed_same_siege() {
    let b = [Besieger { country: "BBB".into(), regiments: army(6) }];
    let c = ctx(&war, &|_| false, &|_| None);
    let (mut p1, mut p2) = (prov("AAA", "AAA", 2.0), prov("AAA", "AAA", 2.0));
    let a = run_until_captured(&mut p1, &b, &c);
    let d = run_until_captured(&mut p2, &b, &c);
    assert_eq!(a.1, d.1);
}

#[test]
fn garrison_regrows_only_without_siege() {
    let r = rules();
    let mut p = prov("AAA", "AAA", 0.0);
    p.garrison = 50.0;
    garrison_regrow_day(&r, &mut p, true);
    assert_eq!(p.garrison, 50.0);
    garrison_regrow_day(&r, &mut p, false);
    assert_eq!(p.garrison, 50.0 + r.regrowth_per_day);
    for _ in 0..500 {
        garrison_regrow_day(&r, &mut p, false);
    }
    assert_eq!(p.garrison, 100.0);
}

#[test]
fn blockade_needs_hostile_free_fleet_with_ships() {
    let f = |owner: &str, ships, in_battle, retreating| BlockadeUnit { owner: owner.into(), ships, in_battle, retreating };
    assert!(port_blockaded(&[f("BBB", 3, false, false)], "AAA", &war));
    assert!(!port_blockaded(&[f("AAA", 3, false, false)], "AAA", &war));
    assert!(!port_blockaded(&[f("BBB", 3, true, false), f("BBB", 2, false, true), f("BBB", 0, false, false)], "AAA", &war));
    assert!(!port_blockaded(&[], "AAA", &war));
}

fn hp(owner: &str, controller: &str, port: bool, home: bool, blockaded: bool) -> HomeProvince {
    HomeProvince { owner: owner.into(), controller: controller.into(), port, home_continent: home, blockaded }
}

#[test]
fn occupation_shares() {
    let provs = vec![
        hp("AAA", "AAA", true, true, true),
        hp("AAA", "BBB", true, true, false),
        hp("AAA", "REB", false, true, false),
        hp("AAA", "AAA", false, true, false),
        hp("AAA", "BBB", false, false, false), // overseas: ignored
        hp("ZZZ", "AAA", false, true, false),  // foreign: ignored
    ];
    let s = occupation_share("AAA", &provs, "REB");
    assert!((s.occupied - 0.25).abs() < 1e-9);
    assert!((s.blockaded - 1.0).abs() < 1e-9);
    let full: Vec<_> = (0..4).map(|_| hp("AAA", "BBB", false, true, false)).collect();
    assert_eq!(occupation_share("AAA", &full, "REB").occupied, 1.0);
    assert_eq!(occupation_share("NOPE", &full, "REB"), OccupationShare::default());
}

/// Ticket #242: the reconnaissance effect and the numbers bonus come from the install's defines
/// (RECON_SIEGE_EFFECT, SIEGE_BRIGADES_BONUS); the rules file's fitted rows are the fallback.
#[test]
fn recon_effect_and_number_bonus_follow_the_defines_and_fall_back_to_the_engine_rules() {
    let r = rules();
    assert_eq!((r.recon_effect, r.number_bonus), (0.1, 0.25), "the defines win");
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/engine_rules.json");
    let m = EngineRules::from_json(&std::fs::read_to_string(p).unwrap()).unwrap();
    let bare = Defines::parse("defines = {\n military = {\n ENGINEER_UNIT_RATIO = 0.1,\n SIEGE_BRIGADES_MIN = 4,\n SIEGE_BRIGADES_MAX = 8,\n POP_SIZE_PER_REGIMENT = 3000,\n }\n}").unwrap();
    let f = SiegeRules::from_engine_rules(&m, &bare).unwrap();
    assert_eq!(f.recon_effect, wiktoria_simulation::num_field(&m, "FLD_MIL_SIEGE_RECON_EFFECT").unwrap());
    assert_eq!(f.number_bonus, wiktoria_simulation::num_field(&m, "FLD_MIL_SIEGE_NUMBER_BONUS").unwrap());
}
