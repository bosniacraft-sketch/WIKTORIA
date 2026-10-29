//! Scores, ranking and great-power seats on made-up worlds.

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_data::date::Date;
use wiktoria_data::modifiers::ModifierLibrary;
use wiktoria_simulation::rank::*;
use wiktoria_simulation::script::{CountryState, PopState, ProvinceState, State};
use wiktoria_simulation::Calendar;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn unit(name: &str, land: bool, capital: bool, stats: UnitStats) -> UnitType {
    UnitType { name: name.into(), land, capital, stats }
}

/// Three seats, a year of grace, secondary powers down to rank 5; a regiment per 3000 soldiers.
fn rules() -> RankRules {
    RankRules {
        units: vec![unit("foot", true, false, [4.0, 4.0, 1.0, 0.0, 0.0]), unit("horse", true, false, [3.0, 1.0, 1.0, 0.0, 0.0]), unit("liner", false, true, [0.0, 0.0, 0.0, 4.0, 8.0]), unit("sloop", false, false, [0.0, 0.0, 0.0, 3.0, 4.0])],
        tech_units: BTreeMap::new(),
        invention_units: Vec::new(),
        soldier_types: BTreeSet::from(["soldiers".to_string()]),
        peace_block: "peace".into(),
        war_block: "war".into(),
        calendar: Calendar::new(0, vec![31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]).unwrap(),
        great_nations: 3.0,
        greatness_days: 365.0,
        colonial_rank: 5.0,
        investment_score_factor: 0.5,
        regiment_min_pop: 1000.0,
        regiment_pop: 3000.0,
        regiment_protectorate: 8.0,
        regiment_colony: 5.0,
        regiment_noncore: 3.0,
    }
}

/// Adds a country with `states` one-province states (each a core) and the given prestige.
fn add(s: &mut State, tag: &str, states: u16, civilized: bool, prestige: f64) {
    let first = s.provinces.len() as u16 + 1;
    for id in first..first + states {
        s.provinces.insert(id, ProvinceState { owner: Some(tag.into()), cores: vec![tag.into()], region: Some(id), ..Default::default() });
    }
    s.countries.insert(tag.into(), CountryState { civilized, prestige, ..Default::default() });
}

fn soldiers(s: &mut State, province: u16, size: f64) {
    s.provinces.get_mut(&province).unwrap().pops.push(PopState { pop_type: "soldiers".into(), size, ..Default::default() });
}

fn world() -> State {
    let mut s = State::new(Date::new(1840, 1, 1));
    for (tag, prestige) in [("AAA", 50.0), ("BBB", 40.0), ("CCC", 30.0), ("DDD", 20.0), ("EEE", 10.0)] {
        add(&mut s, tag, 2, true, prestige);
    }
    s
}

fn tags(list: &[&str]) -> Vec<String> {
    list.iter().map(|t| t.to_string()).collect()
}

#[test]
fn a_soldier_pop_supports_regiments_by_its_size() {
    let r = rules();
    assert!(close(possible_regiments(&r, 999.0, 1.0), 0.0));
    assert!(close(possible_regiments(&r, 1000.0, 1.0), 1.0));
    assert!(close(possible_regiments(&r, 6500.0, 1.0), 3.0));
    // Three times the people per regiment in a province that is no core.
    assert!(close(possible_regiments(&r, 9500.0, r.regiment_noncore), 2.0));
}

#[test]
fn military_score_is_land_part_plus_capital_ships_plus_leaders() {
    let (r, lib) = (rules(), ModifierLibrary::default());
    let mut s = State::new(Date::new(1840, 1, 1));
    add(&mut s, "AAA", 2, true, 0.0);
    soldiers(&mut s, 1, 30_000.0);
    let c = s.countries.get_mut("AAA").unwrap();
    c.regiments = BTreeMap::from([("soldiers".to_string(), 2.0)]);
    c.ships = BTreeMap::from([("liner".to_string(), 2.0), ("sloop".to_string(), 9.0)]);
    c.leaders = 5.0;
    // Could raise 11, has 2: counts 8. Average of (4 + 4) and (3 + 1) = 6. 48 / 7, two capital
    // ships at 8 x 4 / 250, and as many leaders as regiments.
    assert!(close(scores(&s, &r, &lib)["AAA"][1], 48.0 / 7.0 + 2.0 * 32.0 / 250.0 + 2.0));
    // With regiments to spare the soldier pops are the limit: 11 x 6 = 66.
    s.countries.get_mut("AAA").unwrap().regiments.insert("soldiers".into(), 20.0);
    assert!(close(scores(&s, &r, &lib)["AAA"][1], 66.0 / 7.0 + 2.0 * 32.0 / 250.0 + 5.0));
}

#[test]
fn colonies_and_foreign_land_support_fewer_regiments() {
    let (r, lib) = (rules(), ModifierLibrary::default());
    let mut s = State::new(Date::new(1840, 1, 1));
    add(&mut s, "AAA", 3, true, 0.0);
    for id in 1..=3 {
        soldiers(&mut s, id, 30_000.0);
    }
    s.provinces.get_mut(&2).unwrap().colonial = 2;
    s.provinces.get_mut(&3).unwrap().cores.clear();
    s.countries.get_mut("AAA").unwrap().regiments.insert("soldiers".into(), 100.0);
    // 11 at home, 30000 / 15000 + 1 = 3 in the colony, 30000 / 9000 + 1 = 4 on foreign land.
    assert!(close(scores(&s, &r, &lib)["AAA"][1], (18.0f64 * 6.0).floor() / 7.0));
}

#[test]
fn mobilised_regiments_do_not_count_while_mobilised() {
    let (r, lib) = (rules(), ModifierLibrary::default());
    let mut s = State::new(Date::new(1840, 1, 1));
    add(&mut s, "AAA", 2, true, 0.0);
    soldiers(&mut s, 1, 300_000.0);
    let c = s.countries.get_mut("AAA").unwrap();
    c.regiments = BTreeMap::from([("soldiers".to_string(), 1.0), ("farmers".to_string(), 9.0)]);
    assert!(close(scores(&s, &r, &lib)["AAA"][1], 240.0 / 7.0));
    s.countries.get_mut("AAA").unwrap().mobilized = true;
    assert!(close(scores(&s, &r, &lib)["AAA"][1], 24.0 / 7.0));
}

#[test]
fn technologies_and_supply_modifier_move_the_land_part() {
    let (mut r, mut lib) = (rules(), ModifierLibrary::default());
    r.tech_units.insert("drill".into(), UnitEffects { changes: vec![("army_base".to_string(), [1.0, 0.0, 0.0, 0.0, 0.0]), ("foot".to_string(), [0.0, 2.0, 0.0, 0.0, 0.0])] });
    let mut s = State::new(Date::new(1840, 1, 1));
    add(&mut s, "AAA", 2, true, 0.0);
    soldiers(&mut s, 1, 30_000.0);
    let c = s.countries.get_mut("AAA").unwrap();
    c.regiments.insert("soldiers".into(), 20.0);
    c.technologies.insert("drill".into());
    // (4 + 1 + 4 + 2) and (3 + 1 + 1): average 8, times 11 regiments.
    assert!(close(scores(&s, &r, &lib)["AAA"][1], 88.0 / 7.0));
    // At peace supplies cost half: 44, cut to a whole number before the division.
    lib.static_modifiers.insert("peace".into(), BTreeMap::from([("supply_consumption".to_string(), -0.5)]));
    assert!(close(scores(&s, &r, &lib)["AAA"][1], 44.0 / 7.0));
}

#[test]
fn money_invested_abroad_adds_to_the_industrial_score() {
    let (r, lib) = (rules(), ModifierLibrary::default());
    let mut s = world();
    s.countries.get_mut("BBB").unwrap().foreign_investment.insert("AAA".into(), 1000.0);
    // A host without land does not count.
    s.countries.insert("ZZZ".into(), CountryState { foreign_investment: BTreeMap::from([("AAA".to_string(), 9000.0)]), ..Default::default() });
    assert!(close(scores(&s, &r, &lib)["AAA"][0], 1000.0 * 0.01 * 0.5));
}

#[test]
fn ranking_puts_civilised_first_and_seats_the_best_with_two_states() {
    let (r, lib) = (rules(), ModifierLibrary::default());
    let mut s = world();
    add(&mut s, "UUU", 2, false, 900.0);
    add(&mut s, "ONE", 1, true, 45.0);
    rank_day(&mut s, &r, &lib);
    assert_eq!(s.rank_order, tags(&["AAA", "ONE", "BBB", "CCC", "DDD", "EEE", "UUU"]));
    // The one-state country ranks second and gets no seat.
    assert_eq!(s.great_powers, tags(&["AAA", "BBB", "CCC"]));
    assert!(close(s.countries["AAA"].total_score, 50.0));
    // Secondary power: civilised, no seat, rank 5 or better - also inside the band.
    assert_eq!([status(&s, &r, "AAA"), status(&s, &r, "ONE"), status(&s, &r, "DDD"), status(&s, &r, "EEE"), status(&s, &r, "UUU")], [0, 1, 1, 2, 3]);
}

#[test]
fn a_great_power_out_of_the_band_keeps_its_seat_for_greatness_days() {
    let (r, lib) = (rules(), ModifierLibrary::default());
    let mut s = world();
    rank_day(&mut s, &r, &lib);
    assert_eq!(s.countries["CCC"].last_greatness, Some(Date::new(1840, 1, 1)));
    // DDD overtakes CCC: CCC sits on for a year.
    s.countries.get_mut("DDD").unwrap().prestige = 35.0;
    s.date = Date::new(1841, 1, 1);
    let day = rank_day(&mut s, &r, &lib);
    assert!(day.demoted.is_empty());
    assert_eq!(s.great_powers, tags(&["AAA", "BBB", "CCC"]));
    assert_eq!(s.countries["CCC"].last_greatness, Some(Date::new(1840, 1, 1)));
    s.date = Date::new(1841, 1, 2);
    let day = rank_day(&mut s, &r, &lib);
    assert_eq!((day.promoted, day.demoted), (tags(&["DDD"]), tags(&["CCC"])));
    assert_eq!(s.great_powers, tags(&["AAA", "BBB", "DDD"]));
}

#[test]
fn a_great_power_that_is_not_eligible_loses_its_seat_at_once_if_somebody_can_take_it() {
    let (r, lib) = (rules(), ModifierLibrary::default());
    let mut s = world();
    rank_day(&mut s, &r, &lib);
    // BBB is left with one state.
    s.provinces.get_mut(&4).unwrap().owner = Some("AAA".into());
    let day = rank_day(&mut s, &r, &lib);
    assert_eq!((day.promoted, day.demoted), (tags(&["DDD"]), tags(&["BBB"])));
    assert_eq!(s.great_powers, tags(&["AAA", "CCC", "DDD"]));
    assert_eq!(s.countries["BBB"].last_greatness, None);
    // With nobody to take the seat the loser stays.
    let mut s = State::new(Date::new(1840, 1, 1));
    for (tag, prestige) in [("AAA", 50.0), ("BBB", 40.0), ("CCC", 30.0)] {
        add(&mut s, tag, 2, true, prestige);
    }
    rank_day(&mut s, &r, &lib);
    s.countries.get_mut("CCC").unwrap().civilized = false;
    assert!(rank_day(&mut s, &r, &lib).demoted.is_empty());
    assert_eq!(s.great_powers, tags(&["AAA", "BBB", "CCC"]));
    // A great power that owns no land any more is dropped from the list.
    for p in s.provinces.values_mut().filter(|p| p.owner.as_deref() == Some("CCC")) {
        p.owner = Some("AAA".into());
    }
    rank_day(&mut s, &r, &lib);
    assert_eq!(s.great_powers, tags(&["AAA", "BBB"]));
}

#[test]
fn rising_frees_a_country_and_falling_dissolves_its_sphere() {
    let (r, lib) = (rules(), ModifierLibrary::default());
    let mut s = world();
    rank_day(&mut s, &r, &lib);
    // DDD: vassal of AAA, in BBB's sphere, CCC has influence in it. CCC leads EEE.
    s.vassals.push(("AAA".into(), "DDD".into()));
    s.countries.get_mut("BBB").unwrap().sphere.insert("DDD".into());
    let c = s.countries.get_mut("CCC").unwrap();
    c.sphere.insert("EEE".into());
    c.influence = BTreeMap::from([("DDD".to_string(), 60.0), ("EEE".to_string(), 100.0), ("ZZZ".to_string(), 30.0)]);
    s.countries.get_mut("CCC").unwrap().last_greatness = None;
    s.countries.get_mut("DDD").unwrap().prestige = 35.0;
    let day = rank_day(&mut s, &r, &lib);
    assert_eq!((day.promoted, day.demoted), (tags(&["DDD"]), tags(&["CCC"])));
    assert!(s.vassals.is_empty() && s.countries["BBB"].sphere.is_empty());
    // The fallen power keeps only its influence outside its former sphere, and none in the
    // new great power.
    assert!(s.countries["CCC"].sphere.is_empty());
    assert_eq!(s.countries["CCC"].influence, BTreeMap::from([("ZZZ".to_string(), 30.0)]));
}
