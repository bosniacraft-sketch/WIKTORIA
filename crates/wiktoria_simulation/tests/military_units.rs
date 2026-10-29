//! Units and logistics rules on made-up numbers.

use wiktoria_data::date::Date;
use wiktoria_data::defines::Defines;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_simulation::military::logistics::*;
use wiktoria_simulation::military::units::{Army, Regiment};

/// A made-up defines.lua with the keys the rules read from the install.
const DEFINES: &str = "defines = {\n country = { MIN_MOBILIZE_LIMIT = 10, MOBILIZATION_SPEED_BASE = 0.1, MOBILIZATION_SPEED_RAILS_MULT = 1.0, }\n pops = { MIL_TO_AUTORISE = 1, }\n military = { SIEGE_ATTRITION = 2, SUPPLY_RANGE = 6, REINFORCE_SPEED = 0.1, POP_MIN_SIZE_FOR_REGIMENT = 1000, POP_SIZE_PER_REGIMENT = 3000, POP_MIN_SIZE_FOR_REGIMENT_PROTECTORATE_MULTIPLIER = 2, POP_MIN_SIZE_FOR_REGIMENT_COLONY_MULTIPLIER = 4, POP_MIN_SIZE_FOR_REGIMENT_NONCORE_MULTIPLIER = 2, LEADER_RECRUIT_COST = 50, LEADER_AGE_DEATH_FACTOR = 10, LEADER_MAX_RANDOM_PRESTIGE = 0, LEADER_PRESTIGE_TO_MORALE_FACTOR = 0.01, LEADER_PRESTIGE_TO_MAX_ORG_FACTOR = 0.005, }\n}\n";

/// Engine literals of the rules file, made-up values.
const FIELDS: &[(&str, f64)] = &[
    ("SUPPLY_OWN", 2.5),
    ("SUPPLY_FRIENDLY", 2.0),
    ("SUPPLY_HOSTILE", 1.0),
    ("SHIP_SUNK_BELOW", 0.01),
    ("ORG_REGAIN", 0.2),
    ("ORG_SLIDER_FLOOR", 0.25),
    ("MOBILISED_REFILL_SHARE", 0.5),
    ("DELETE_BELOW_STRENGTH", 0.001),
    ("LEADER_SAFE_YEARS", 25.0),
    ("MAX_EXPERIENCE", 100.0),
    ("NAVAL_ATTRITION_PER_MONTH", 2.0),
];

fn rules() -> LogisticsRules {
    let rows: Vec<String> = FIELDS.iter().map(|(k, v)| format!("{{\"field_id\":\"FLD_MIL_LOG_{k}\",\"value\":{v}}}")).collect();
    let json = format!("{{\"schema_version\":1,\"source\":{{\"sha256\":\"x\"}},\"tables\":{{\"Fields\":[{}]}}}}", rows.join(","));
    LogisticsRules::new(&EngineRules::from_json(&json).unwrap(), &Defines::parse(DEFINES).unwrap()).unwrap()
}

fn reg(id: u64, strength: f64) -> Regiment {
    Regiment { id, name: String::new(), kind: "infantry".into(), pop: Some(id), pop_type: Some("farmers".into()), organisation: 0.0, strength, experience: 0.0, name_count: 0 }
}

fn army(id: u64, regs: Vec<Regiment>) -> Army {
    Army { id, name: String::new(), owner: "AAA".into(), leader: None, location: 1, previous: None, movement_progress: 0.0, path: vec![], target: None, base: None, dig_in: 0.0, dig_in_date: None, supplies: 0.0, regiments: regs, carried_by: None }
}

#[test]
fn supply_limit_by_access() {
    let r = rules();
    assert_eq!(supply_limit(&r, 0.0, SupplyAccess::OwnControlled, 0.0), 2);
    assert_eq!(supply_limit(&r, 0.0, SupplyAccess::OwnControlled, 0.2), 3);
    assert_eq!(supply_limit(&r, 1.0, SupplyAccess::OwnControlled, 0.0), 5);
    assert_eq!(supply_limit(&r, 0.0, SupplyAccess::OwnedOccupied, 0.0), 2);
    assert_eq!(supply_limit(&r, 0.0, SupplyAccess::Hostile, 0.0), 1);
    assert_eq!(supply_limit(&r, 0.0, SupplyAccess::Sea, 0.0), 1);
}

/// The province share is terrain + land block + infrastructure block x infrastructure.
/// The numbers are the readings of that row: a land block of 5, an infrastructure block of 4.
#[test]
fn supply_limit_adds_terrain_land_block_and_railway() {
    let r = rules();
    let limit = |terrain: f64, infrastructure: f64, access: SupplyAccess, tech: f64| supply_limit(&r, province_supply_modifier(terrain, 5.0, 4.0, infrastructure), access, tech);
    assert_eq!(limit(6.0, 0.0, SupplyAccess::OwnControlled, 0.0), 30);
    assert_eq!(limit(10.0, 0.0, SupplyAccess::OwnControlled, 0.0), 40);
    assert_eq!(limit(6.0, 0.0, SupplyAccess::Hostile, 0.0), 12);
    assert_eq!(limit(10.0, 0.0, SupplyAccess::Hostile, 0.0), 16);
    assert_eq!(limit(2.0, 0.0, SupplyAccess::Hostile, 0.0), 8);
    // One railway level of 0.16: 16.64 x 2.5 = 41.6, the whole part counts.
    assert_eq!(province_supply_modifier(10.0, 5.0, 4.0, 0.16), 15.64);
    assert_eq!(limit(10.0, 0.16, SupplyAccess::OwnControlled, 0.0), 41);
    // Not a reading: the technology factor on top, 16 x 2.5 x 1.25.
    assert_eq!(limit(10.0, 0.0, SupplyAccess::OwnControlled, 0.25), 50);
}

/// The cap is the province's whole `max_attrition`: land block + terrain + climate + continent.
/// The two readings stand in a climate without one, the land block gives 5.
#[test]
fn attrition_is_capped_by_the_land_block_and_the_climate() {
    let r = rules();
    let cap = province_max_attrition(5.0, 0.0, 0.0, 0.0);
    assert_eq!(cap, 5.0);
    let pct = |weight: f64, supply_limit: i64, cap: f64| land_attrition_percent(&r, &LandAttrition { weight, supply_limit, province_max_attrition: cap, ..Default::default() });
    // 52.95 thousand on a limit of 40: 12.95 over, capped at 5. 36 on 30: 6 over, capped at 5.
    assert_eq!(pct(52.95, 40, cap), 5.0);
    assert_eq!(pct(36.0, 30, cap), 5.0);
    assert_eq!(pct(33.0, 30, cap), 3.0);
    assert_eq!(pct(30.0, 30, cap), 0.0);
    // A climate with 5 of its own lifts the cap to 10; without the land block nothing is taken.
    assert_eq!(pct(52.95, 40, province_max_attrition(5.0, 0.0, 5.0, 0.0)), 10.0);
    assert_eq!(pct(52.95, 40, province_max_attrition(0.0, 0.0, 0.0, 0.0)), 0.0);
}

/// no stack cap, a stack over the supply limit takes attrition and
/// the loss grows with every extra brigade, so stacks settle near the limit.
#[test]
fn stack_over_supply_limit_takes_attrition() {
    let r = rules();
    let limit = supply_limit(&r, 9.0, SupplyAccess::OwnControlled, 0.0); // a 25k limit province
    let mut last = -1.0;
    for brigades in [8usize, 10, 12, 15] {
        let a = army(1, (0..brigades).map(|i| reg(i as u64, 3.0)).collect());
        let weight = stack_weight(&[&a]);
        let att = LandAttrition { weight, supply_limit: limit, province_max_attrition: 30.0, ..Default::default() };
        let v = land_attrition_percent(&r, &att);
        if weight <= limit as f64 {
            assert_eq!(v, 0.0);
        } else {
            assert!(v > last);
        }
        last = v;
    }
    // 30k men on a 25k limit: 5 percent, 45k on it: 20 percent capped by max attrition.
    let thirty = LandAttrition { weight: 30.0, supply_limit: 25, province_max_attrition: 10.0, ..Default::default() };
    assert_eq!(land_attrition_percent(&r, &thirty), 5.0);
    let big = LandAttrition { weight: 45.0, supply_limit: 25, province_max_attrition: 10.0, ..Default::default() };
    assert_eq!(land_attrition_percent(&r, &big), 10.0);
    assert!((attrition_loss(3.0, 5.0) - 0.15).abs() < 1e-9);
}

#[test]
fn attrition_terms() {
    let r = rules();
    let base = LandAttrition { weight: 10.0, supply_limit: 8, province_attrition: 1.0, enemy_fort: 2.0, province_max_attrition: 9.7, ..Default::default() };
    assert_eq!(land_attrition_percent(&r, &base), 5.0);
    assert_eq!(land_attrition_percent(&r, &LandAttrition { exiled: true, ..base }), 0.0);
    assert_eq!(land_attrition_percent(&r, &LandAttrition { in_siege: true, province_max_attrition: 1.0, ..base }), 3.0); // after the cap
    assert_eq!(land_attrition_percent(&r, &LandAttrition { weight: 1.0, ..base }), 0.0);
    assert!(ship_hit(3, 7, 2.0) == ((21 % 10) < 2));
    assert!(!ship_hit(3, 7, 0.0));
    assert!(ship_sinks(&r, 0.005) && !ship_sinks(&r, 0.5));
}

#[test]
fn naval_attrition() {
    let r = rules();
    let n = NavalAttrition { months_out_of_supply: 0.0, ..Default::default() };
    assert_eq!(naval_attrition_percent(&r, &n), 1.0);
    assert_eq!(naval_attrition_percent(&r, &NavalAttrition { months_out_of_supply: 1.0, ..n }), 3.0);
    assert_eq!(naval_attrition_percent(&r, &NavalAttrition { months_out_of_supply: 2.0, ..n }), 5.0);
    assert_eq!(naval_attrition_percent(&r, &NavalAttrition { in_port: true, ..n }), 0.0);
    assert_eq!(naval_attrition_percent(&r, &NavalAttrition { in_supply_range: true, ..n }), 0.0);
    assert_eq!(fleet_supply_range(&r, 0.5), 9.0);
}

#[test]
fn organisation_and_supply() {
    let r = rules();
    assert!((max_organisation(&r, 20.0, 0.0, 0.0, 1.0, false) - 20.0).abs() < 1e-9);
    assert!((max_organisation(&r, 20.0, 0.0, 0.0, 0.0, false) - 5.0).abs() < 1e-9);
    assert!((max_organisation(&r, 20.0, 0.0, 0.0, 0.0, true) - 20.0).abs() < 1e-9);
    assert!((regain_organisation(&r, 1.0, 20.0, 1.0, 0.0, 0.0, 0.0) - 1.2).abs() < 1e-9);
    assert!((regain_organisation(&r, 1.0, 20.0, 0.5, 0.0, 0.0, 0.0) - 1.1).abs() < 1e-9);
    assert_eq!(regain_organisation(&r, 19.95, 20.0, 1.0, 0.0, 0.0, 0.0), 20.0);
    assert_eq!(supply_status(&[1.0, 0.0], false), 0.5);
    assert_eq!(supply_status(&[0.0], true), 1.0);
    assert!((supply_need(2.0, 0.5, 1.0, 0.1, 0.5, 1.0) - 1.5).abs() < 1e-9);
}

#[test]
fn reinforcement() {
    let r = rules();
    assert_eq!(reinforce_location_factor(ReinforcePlace::Owned), 2.0);
    assert_eq!(reinforce_location_factor(ReinforcePlace::Exiled), 0.0);
    assert_eq!(reinforce_location_factor(ReinforcePlace::Hostile { friendly_land_neighbour: false, coastal_free: false }), 0.1);
    assert_eq!(reinforce_location_factor(ReinforcePlace::Rebel { reinforcing: false, rebel_controlled: true }), 0.0);
    let rate = reinforce_rate(&r, 0.0, 2.0, 0.0);
    assert!((rate - 0.2).abs() < 1e-9);
    // Three regiments at supply s: gains 1 : s : s^2 (times s once: the product carries).
    let mk = || Refill { strength: 0.0, max_strength: 3.0, experience: 10.0, share: 1.0 };
    let mut v = [mk(), mk(), mk()];
    reinforce(rate, 0.5, &mut v);
    assert!((v[0].strength - 3.0 * 0.2 * 0.5).abs() < 1e-9);
    assert!((v[1].strength / v[0].strength - 0.5).abs() < 1e-9);
    assert!((v[2].strength / v[1].strength - 0.5).abs() < 1e-9);
    assert!(v[0].experience < 10.0);
    // Mobilised regiments refill at half, a full one is skipped.
    assert_eq!(pop_refill_share(&r, false, 3, 1), 0.5);
    assert_eq!(pop_refill_share(&r, true, 1, 2), 0.5);
    assert_eq!(pop_refill_share(&r, true, 4, 2), 1.0);
    let mut full = [Refill { strength: 3.0, max_strength: 3.0, experience: 5.0, share: 1.0 }];
    assert_eq!(reinforce(rate, 1.0, &mut full), 0.0);
}

#[test]
fn regiments_per_pop() {
    let r = rules();
    assert_eq!(regiments_supported(&r, 999.0, PopStatus::Normal, false), 0);
    assert_eq!(regiments_supported(&r, 1000.0, PopStatus::Normal, false), 1);
    assert_eq!(regiments_supported(&r, 9000.0, PopStatus::Normal, false), 4);
    assert_eq!(regiments_supported(&r, 9000.0, PopStatus::Colony, false), 1);
    assert_eq!(regiments_supported(&r, 9000.0, PopStatus::Colony, true), 4);
}

#[test]
fn regiment_pop_check() {
    let r = rules();
    let ok = RegimentCheck { rebel_unit: false, has_pop: true, pop_in_rebel_faction: false, pop_militancy: 0.0, province_changed_owner: false, soldier_pop: true, at_war: false, supports: 2, strength: 3.0, experience: 5.0, start_experience: 5.0 };
    assert_eq!(regiment_check(&r, &ok), RegimentVerdict::Keep);
    assert_eq!(regiment_check(&r, &RegimentCheck { has_pop: false, ..ok }), RegimentVerdict::FindPop);
    assert_eq!(regiment_check(&r, &RegimentCheck { pop_in_rebel_faction: true, pop_militancy: 5.0, ..ok }), RegimentVerdict::Defect);
    assert_eq!(regiment_check(&r, &RegimentCheck { rebel_unit: true, ..ok }), RegimentVerdict::Delete);
    assert_eq!(regiment_check(&r, &RegimentCheck { province_changed_owner: true, ..ok }), RegimentVerdict::Delete);
    assert_eq!(regiment_check(&r, &RegimentCheck { province_changed_owner: true, at_war: true, ..ok }), RegimentVerdict::MoveToNewOwner);
    assert_eq!(regiment_check(&r, &RegimentCheck { supports: 0, ..ok }), RegimentVerdict::Keep);
    assert_eq!(regiment_check(&r, &RegimentCheck { supports: 0, strength: 0.0005, ..ok }), RegimentVerdict::Delete);
    assert_eq!(regiment_check(&r, &RegimentCheck { experience: 1.0, ..ok }), RegimentVerdict::RaiseExperience(5.0));
}

#[test]
fn recruiting() {
    let r = rules();
    assert_eq!(build_days(30.0, 0.0), 30);
    assert_eq!(build_days(30.0, -0.5), 15);
    assert_eq!(build_days(0.0, 0.0), 1);
    assert!(build_done(30, 30) && !build_done(30, 29));
    assert_eq!(start_experience(&r, 90.0, 20.0), 100.0);
}

fn pops() -> Vec<MobilisablePop> {
    vec![MobilisablePop { id: 1, size: 9000.0, has: 1 }, MobilisablePop { id: 2, size: 1500.0, has: 0 }, MobilisablePop { id: 3, size: 1500.0, has: 0 }, MobilisablePop { id: 4, size: 30000.0, has: 0 }]
}

#[test]
fn mobilisation() {
    let r = rules();
    assert_eq!(mobilise_limit(&r, 5, 0.0), 10);
    assert_eq!(mobilise_limit(&r, 40, 0.5), 60);
    // size share 1: pop 1 wants 3 minus 1 held = 2; pops 2 and 3 pool 0.5 + 0.5 into one for pop 3.
    let s = mobilise_schedule(&r, &pops(), 1.0, 100);
    assert_eq!(s, vec![(1, 2), (3, 1), (4, 10)]);
    // A tight limit stops the walk, the pop that passes it is not scheduled.
    let s = mobilise_schedule(&r, &pops(), 1.0, 5);
    assert_eq!(s, vec![(1, 2), (3, 1)]);
    let rails = mobilisation_rail_factor(&r, &[2.0, 0.0], 4.0);
    assert!((rails - 1.25).abs() < 1e-9);
    let mut p = 0.0;
    for _ in 0..8 {
        p = mobilisation_progress(&r, p, rails, false);
    }
    assert_eq!(p, 1.0);
    assert_eq!(mobilisation_progress(&r, 0.4, rails, true), 0.4);
    assert_eq!(mobilised_unit(true, "infantry", "irregular", &r, 5.0, 2.0), ("infantry", 7.0));
    assert_eq!(mobilised_unit(false, "infantry", "irregular", &r, 5.0, 2.0), ("irregular", 2.0));
    assert!((mobilisation_production_factor(true, 0.5, 0.4) - 0.8).abs() < 1e-9);
    assert_eq!(mobilisation_production_factor(false, 0.5, 0.4), 1.0);
}

#[test]
fn demobilising_deletes_non_soldier_regiments() {
    let mut a = army(1, vec![reg(1, 3.0), reg(2, 3.0)]);
    a.regiments[1].pop_type = Some("soldiers".into());
    let gone = demobilise(&mut a, &|t| t == "soldiers");
    assert_eq!(gone, vec![1]);
    assert_eq!(a.regiments.len(), 1);
    assert_eq!(a.regiments[0].id, 2);
}

#[test]
fn transports() {
    assert_eq!(fleet_room(&[1.0, 1.0, 1.0]), 3);
    assert!(may_enter_sea(2, 1, 3) && !may_enter_sea(2, 2, 3));
    assert!(may_board(true, false, 1, 2, 3) && !may_board(true, false, 2, 2, 3) && !may_board(true, true, 0, 1, 3));
    let mut carried = vec![army(1, vec![reg(1, 3.0), reg(2, 3.0)]), army(2, vec![reg(3, 3.0)])];
    let lost = trim_over_capacity(&mut carried, 1);
    assert_eq!(lost, 2);
    assert_eq!(carried.len(), 1);
    assert_eq!(carried[0].id, 2);
    let mut carried = vec![army(1, vec![reg(1, 3.0)]), army(2, vec![reg(3, 3.0)])];
    trim_over_capacity(&mut carried, 1);
    assert_eq!(carried.len(), 1); // the first army lost its only regiment and is gone
    assert_eq!(carried[0].id, 2);
}

#[test]
fn leaders() {
    let r = rules();
    // Two pop types: one at its optimum share (full), one at half (half value).
    let pts = leadership_points(&[(1.0, 0.02, 0.01), (2.0, 0.005, 0.01), (0.0, 0.5, 0.1)], 0.5, 1.0);
    assert!((pts - (1.0 + 1.0 + 0.5) * 2.0).abs() < 1e-9);
    assert_eq!(add_leadership(&r, 140.0, 30.0), 150.0);
    assert!(can_create_leader(&r, 50.0) && !can_create_leader(&r, 49.0));
    assert_eq!(pay_leader(&r, 30.0), 0.0);
    assert_eq!(pay_leader(&r, 120.0), 70.0);
    assert_eq!(draw_trait(5, 0), 1);
    assert_eq!(draw_trait(5, 3), 4);
    assert_eq!(draw_trait(5, 4), 1);
    assert_eq!(draw_trait(1, 9), 0);
    assert_eq!(new_leader_prestige(&r, 0.7), 0.0);
    let (m, o) = prestige_bonus(&r, 150.0);
    assert!((m - 1.0).abs() < 1e-9 && (o - 0.5).abs() < 1e-9);
    assert_eq!(whole_years(Date::new(1800, 6, 1), Date::new(1836, 5, 31)), 35);
    assert_eq!(whole_years(Date::new(1800, 6, 1), Date::new(1836, 6, 1)), 36);
}

#[test]
fn leader_death() {
    let r = rules();
    assert!(!leader_dies(&r, 24, false, 0));
    assert!(leader_dies(&r, 25, true, 0));
    // 60 years: chance 7 in 10950, doubled in battle.
    assert!(leader_dies(&r, 60, false, 6) && !leader_dies(&r, 60, false, 7));
    assert!(leader_dies(&r, 60, true, 13) && !leader_dies(&r, 60, true, 14));
    assert!(leader_dies(&r, 10, true, 0) || !leader_dies(&r, 10, true, 5)); // tested at any age in battle
}

