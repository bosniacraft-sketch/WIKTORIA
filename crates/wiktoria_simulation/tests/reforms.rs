//! Reforms passed by the upper house, on a made-up country.

use std::collections::BTreeMap;

use wiktoria_data::date::Date;
use wiktoria_data::script;
use wiktoria_simulation::reforms::{house_support, pick_reform, reform_day, ReformGroup, ReformRules};
use wiktoria_simulation::script::{CountryState, PopState, ProvinceState, State};

fn rules() -> ReformRules {
    let block = |src: &str| Some(script::parse(src).unwrap().root);
    let group = |name: &str, political: bool, next_step_only: bool, options: &[&str]| ReformGroup { name: name.into(), political, next_step_only, options: options.iter().map(|o| (o.to_string(), None)).collect() };
    ReformRules {
        min_delay: 6,
        ai_support: 0.05,
        support_needed: 0.5,
        con_impact: -50.0,
        mil_impact: 2.0,
        wrong_impact: 1.0,
        conservative_increase: 0.25,
        groups: vec![group("vote", true, true, &["no_vote", "some_vote", "all_vote"]), group("press", true, false, &["state_press", "censored_press", "free_press"]), group("schools", false, true, &["no_schools", "some_schools"])],
        wants: [
            ("blue".to_string(), (block("base = 1"), block("base = 0"))),
            // The reds come round with militancy: 0.2 for each step.
            // The blacks are against political reform.
            ("black".to_string(), (block("base = -1"), block("base = 0"))),
            ("red".to_string(), (block("base = 0 group = { modifier = { factor = 0.2 militancy = 1 } modifier = { factor = 0.2 militancy = 2 } modifier = { factor = 0.2 militancy = 3 } }"), block("base = 0"))),
        ]
        .into(),
    }
}

/// 40% blue and 60% red in the upper house; the pops ask for `all_vote` (30%) and `free_press` (20%).
fn world(militancy: f64) -> State {
    let mut s = State::new(Date::new(1840, 3, 1));
    s.issues = ["no_vote", "some_vote", "all_vote", "state_press", "censored_press", "free_press", "no_schools", "some_schools"].iter().zip(["vote", "vote", "vote", "press", "press", "press", "schools", "schools"]).map(|(o, g)| (o.to_string(), g.to_string())).collect();
    s.ideologies = vec!["blue".into(), "red".into(), "black".into()];
    let pop = PopState { pop_type: "farmers".into(), size: 100_000.0, militancy, ideology: [(1, 50.0), (2, 50.0)].into(), issues: [(3, 30.0), (6, 20.0), (8, 50.0)].into(), ..Default::default() };
    s.provinces.insert(1, ProvinceState { owner: Some("AAA".into()), pops: vec![pop], ..Default::default() });
    let reforms: BTreeMap<String, String> = [("vote", "no_vote"), ("press", "state_press"), ("schools", "no_schools")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    s.countries.insert("AAA".into(), CountryState { civilized: true, reforms, upper_house: [("blue".to_string(), 0.4), ("red".to_string(), 0.6)].into(), last_reform: Some(Date::new(1839, 9, 1)), ..Default::default() });
    s
}

#[test]
fn militancy_brings_the_upper_house_round() {
    let r = rules();
    assert!((house_support(&world(0.0), &r, "AAA", true) - 0.4).abs() < 1e-12, "the blues alone");
    assert!((house_support(&world(2.5), &r, "AAA", true) - (0.4 + 0.6 * 0.4)).abs() < 1e-12, "two steps of militancy");
    assert_eq!(house_support(&world(9.0), &r, "AAA", false), 0.0, "nobody wants social reform");
}

#[test]
fn the_most_wanted_reform_is_passed_one_step_at_a_time() {
    let r = rules();
    // Voting is asked for most (30%): one step only. The press would go straight to free_press.
    assert_eq!(pick_reform(&world(0.0), &r, "AAA", true), Some(("vote".to_string(), "some_vote".to_string())));
    let mut s = world(2.5);
    assert_eq!(reform_day(&mut s, &r), vec![("AAA".to_string(), "vote".to_string(), "some_vote".to_string())]);
    assert_eq!(s.countries["AAA"].last_reform, Some(Date::new(1840, 3, 1)));
    // Militancy: the pop is half blue (weight 1) and half red (weight 0.4 at this militancy), so
    // 2 x (0.5 x 1 + 0.5 x 0.4) = 1.4 less, and 1 less for not being in a movement for something
    // else. Nobody asked for `some_vote` itself, so consciousness stays.
    assert!((s.provinces[&1].pops[0].militancy - 0.1).abs() < 1e-12);
    assert_eq!(s.provinces[&1].pops[0].consciousness, 0.0);
    // The upper house: the blues 0.4 / 1.25 = 0.32; there are no conservatives here to gain.
    assert!((s.countries["AAA"].upper_house["blue"] - 0.4).abs() < 1e-12);
    // Not again within six months.
    s.date = Date::new(1840, 8, 1);
    assert!(reform_day(&mut s, &r).is_empty());
    // With militancy back up the next one passes: `all_vote`, which 30% asked for.
    s.date = Date::new(1840, 9, 1);
    let pop = &mut s.provinces.get_mut(&1).unwrap().pops[0];
    (pop.militancy, pop.consciousness) = (2.5, 20.0);
    assert_eq!(reform_day(&mut s, &r).len(), 1);
    assert!((s.provinces[&1].pops[0].militancy - 0.1).abs() < 1e-12);
    assert!((s.provinces[&1].pops[0].consciousness - 5.0).abs() < 1e-12, "20 - 50 x 0.3");
    // Without militancy the house does not agree; not on other days than the 1st.
    assert!(reform_day(&mut world(0.0), &r).is_empty());
    let mut s = world(2.5);
    s.date = Date::new(1840, 3, 2);
    assert!(reform_day(&mut s, &r).is_empty());
    // A reform nobody asks for is not taken.
    let mut s = world(2.5);
    s.provinces.get_mut(&1).unwrap().pops[0].issues = [(8, 50.0)].into();
    assert!(reform_day(&mut s, &r).is_empty());
}

#[test]
fn a_reform_needs_five_percent_of_the_people_unless_the_country_is_restless() {
    let r = rules();
    let few = |militancy: f64| {
        let mut s = world(militancy);
        s.provinces.get_mut(&1).unwrap().pops[0].issues = [(3, 4.0)].into();
        s
    };
    assert_eq!(pick_reform(&few(0.0), &r, "AAA", true), None, "4% ask for it: not enough");
    assert_eq!(pick_reform(&few(4.5), &r, "AAA", true), Some(("vote".to_string(), "some_vote".to_string())), "average militancy above 4: 0.1% is enough");
}

#[test]
fn support_counts_options_further_on_and_subtracts_those_behind() {
    let r = rules();
    let mut s = world(0.0);
    s.countries.get_mut("AAA").unwrap().reforms.insert("press".into(), "censored_press".into());
    // 25% want the state press back, 20% a free press: -5% for going on.
    s.provinces.get_mut(&1).unwrap().pops[0].issues = [(4, 25.0), (6, 20.0)].into();
    assert_eq!(pick_reform(&s, &r, "AAA", true), None);
    s.provinces.get_mut(&1).unwrap().pops[0].issues = [(4, 10.0), (6, 20.0)].into();
    assert_eq!(pick_reform(&s, &r, "AAA", true), Some(("press".to_string(), "free_press".to_string())));
    // From the state press both further options are open; the nearer one also counts those who
    // want to go further, so it is at least as well supported and comes first.
    s.countries.get_mut("AAA").unwrap().reforms.insert("press".into(), "state_press".into());
    assert_eq!(pick_reform(&s, &r, "AAA", true), Some(("press".to_string(), "censored_press".to_string())));
}

#[test]
fn social_reforms_are_looked_at_first() {
    let mut r = rules();
    r.wants.get_mut("blue").unwrap().1 = Some(script::parse("base = 2").unwrap().root);
    let mut s = world(0.0);
    assert_eq!(reform_day(&mut s, &r), vec![("AAA".to_string(), "schools".to_string(), "some_schools".to_string())]);
    // Consciousness: 50% asked for it, so -50 x 0.5, not below 0.
    assert_eq!(s.provinces[&1].pops[0].consciousness, 0.0);
}

#[test]
fn a_reform_angers_those_against_it_and_seats_conservatives() {
    let r = rules();
    let mut s = world(2.5);
    s.ideologies.push("conservative".into());
    s.provinces.get_mut(&1).unwrap().pops[0].ideology = [(3, 100.0)].into();
    assert_eq!(reform_day(&mut s, &r).len(), 1);
    // An all-black pop: 2 x 1 more, then 1 less.
    assert!((s.provinces[&1].pops[0].militancy - 3.5).abs() < 1e-12);
    // The upper house: conservatives +0.25, then everything / 1.25.
    let house = &s.countries["AAA"].upper_house;
    assert!((house["conservative"] - 0.2).abs() < 1e-12 && (house["blue"] - 0.32).abs() < 1e-12 && (house["red"] - 0.48).abs() < 1e-12);
}
