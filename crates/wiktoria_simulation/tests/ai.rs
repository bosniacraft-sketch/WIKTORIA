//! The computer player's national focuses and research weights, on made-up numbers and worlds.

use std::collections::BTreeMap;

use wiktoria_data::date::Date;
use wiktoria_simulation::ai::{focus_score, plan_focuses, region_candidate, research_weight, Candidate, FocusAi, FocusInput, FocusKind, FocusNumbers, FocusWeights, ResearchWeights};
use wiktoria_simulation::script::{CountryState, FactoryState, PopState, ProvinceState, State};

fn weights() -> FocusWeights {
    FocusWeights { administrator: 10.0, worker: 10.0, educator: 10.0, soldier: 30.0, soldier_fraction: 0.03, capitalist_fraction: 0.001, max_clergy: 0.04, max_bureaucracy: 0.01, bureaucracy_increment: 0.001, numbers: numbers() }
}

/// The rules file's focus numbers (FLD_AI_*).
fn numbers() -> FocusNumbers {
    FocusNumbers { never: NEVER, capital_factor: 10.0, capital_share: 0.01, capitalist_min_figure: 0.3, rail_base: 2.0, rail_factories_per_point: 2.0, soldier_target_army_school: 0.01, soldier_target_culture_school: -0.005 }
}

const NEVER: f64 = -100.0;

fn rw() -> ResearchWeights {
    ResearchWeights { overdue_per_year: 0.005, min_top_cost: 3600.0 }
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} is not {b}");
}

#[test]
fn bureaucrat_score_follows_the_province_shortfall() {
    let w = weights();
    let i = FocusInput { people: 1000.0, bureaucrat_share: 0.004, ..Default::default() };
    close(focus_score(&w, &FocusKind::Bureaucrats, &i), 6.0);
    // Administrative efficiency bonuses raise the share that counts; reform levels raise the need.
    close(focus_score(&w, &FocusKind::Bureaucrats, &FocusInput { admin_bonus: 0.25, ..i.clone() }), 5.0);
    close(focus_score(&w, &FocusKind::Bureaucrats, &FocusInput { admin_reform_levels: 10.0, ..i.clone() }), 8.0);
    // Neither foreign culture nor a colonial state changes it.
    close(focus_score(&w, &FocusKind::Bureaucrats, &FocusInput { foreign_share: 1.0, colonial: true, ..i.clone() }), 6.0);
    // Above the need the score is negative.
    assert!(focus_score(&w, &FocusKind::Bureaucrats, &FocusInput { bureaucrat_share: 0.02, ..i }) < 0.0);
}

#[test]
fn bureaucrat_score_is_tenfold_only_in_a_capital_under_one_percent() {
    let w = weights();
    let i = FocusInput { people: 1000.0, bureaucrat_share: 0.004, admin_reform_levels: 10.0, ..Default::default() };
    close(focus_score(&w, &FocusKind::Bureaucrats, &FocusInput { capital: true, ..i.clone() }), 80.0);
    // A capital with 1.5% bureaucrats (need 2%): no factor.
    close(focus_score(&w, &FocusKind::Bureaucrats, &FocusInput { capital: true, bureaucrat_share: 0.015, ..i.clone() }), 2.5);
    close(focus_score(&w, &FocusKind::Bureaucrats, &i), 8.0);
}

#[test]
fn soldier_score_uses_the_country_share_and_the_culture_factor() {
    let w = weights();
    let i = FocusInput { people: 1000.0, country_soldier_share: 0.01, ..Default::default() };
    close(focus_score(&w, &FocusKind::Soldiers, &i), 20.0);
    close(focus_score(&w, &FocusKind::Soldiers, &FocusInput { foreign_share: 0.75, ..i.clone() }), 5.0);
    // The school moves the target: 4% wanted.
    close(focus_score(&w, &FocusKind::Soldiers, &FocusInput { soldier_target_shift: 0.01, ..i.clone() }), 22.5);
    assert!(focus_score(&w, &FocusKind::Soldiers, &FocusInput { country_soldier_share: 0.05, ..i.clone() }) < 0.0);
    assert_eq!(focus_score(&w, &FocusKind::Soldiers, &FocusInput { colonial: true, ..i.clone() }), NEVER);
    assert_eq!(focus_score(&w, &FocusKind::Soldiers, &FocusInput { people: 0.0, ..i }), NEVER, "an empty province never");
}

#[test]
fn clergy_score_follows_the_province_shortfall_to_four_percent() {
    let w = weights();
    let i = FocusInput { people: 1000.0, clergy_share: 0.01, ..Default::default() };
    close(focus_score(&w, &FocusKind::Clergy, &i), 7.5);
    close(focus_score(&w, &FocusKind::Clergy, &FocusInput { foreign_share: 0.5, ..i.clone() }), 3.75);
    assert_eq!(focus_score(&w, &FocusKind::Clergy, &FocusInput { colonial: true, ..i }), NEVER);
}

#[test]
fn worker_score_counts_the_unfilled_places_of_running_factories() {
    let w = weights();
    let kind = FocusKind::Workers("makers".into());
    let i = FocusInput { people: 1000.0, state_factories: 2, unfilled: [("makers".to_string(), 1.2)].into(), ..Default::default() };
    close(focus_score(&w, &kind, &i), 12.0);
    close(focus_score(&w, &kind, &FocusInput { foreign_share: 0.5, ..i.clone() }), 6.0);
    // Another worker type has its own places; a state without factories gives 0.
    close(focus_score(&w, &FocusKind::Workers("scribes".into()), &i), 0.0);
    close(focus_score(&w, &kind, &FocusInput { state_factories: 0, ..i.clone() }), 0.0);
    assert_eq!(focus_score(&w, &kind, &FocusInput { colonial: true, ..i }), NEVER);
}

#[test]
fn capitalist_score_has_its_gates() {
    let w = weights();
    let i = FocusInput { people: 1000.0, country_capitalist_share: 0.0005, civilized: true, capitalist_figure: 0.5, capital_state_has_factory: true, ..Default::default() };
    // The worker weight: 10 x (1 - 0.05% / 0.1%).
    close(focus_score(&w, &FocusKind::Capitalists, &i), 5.0);
    assert!(focus_score(&w, &FocusKind::Capitalists, &FocusInput { country_capitalist_share: 0.002, ..i.clone() }) < 0.0);
    assert_eq!(focus_score(&w, &FocusKind::Capitalists, &FocusInput { civilized: false, ..i.clone() }), NEVER);
    assert_eq!(focus_score(&w, &FocusKind::Capitalists, &FocusInput { capitalist_figure: 0.2, ..i.clone() }), NEVER);
    assert_eq!(focus_score(&w, &FocusKind::Capitalists, &FocusInput { colonial: true, ..i.clone() }), NEVER);
    // No factory in the capital's state: only the capital itself may score.
    assert_eq!(focus_score(&w, &FocusKind::Capitalists, &FocusInput { capital_state_has_factory: false, ..i.clone() }), NEVER);
    close(focus_score(&w, &FocusKind::Capitalists, &FocusInput { capital_state_has_factory: false, capital: true, ..i }), 5.0);
}

#[test]
fn railroad_score_grows_with_the_factories_of_the_state() {
    let w = weights();
    let i = FocusInput { people: 1000.0, state_factories: 5, rail_possible: true, ..Default::default() };
    close(focus_score(&w, &FocusKind::Railroad, &i), 4.0);
    assert_eq!(focus_score(&w, &FocusKind::Railroad, &FocusInput { state_factories: 0, ..i.clone() }), NEVER);
    assert_eq!(focus_score(&w, &FocusKind::Railroad, &FocusInput { rail_possible: false, ..i.clone() }), NEVER);
    assert_eq!(focus_score(&w, &FocusKind::Railroad, &FocusInput { colonial: true, ..i.clone() }), NEVER);
    close(focus_score(&w, &FocusKind::Other, &i), 0.0);
}

#[test]
fn a_region_offers_its_best_average_of_positive_scores() {
    // Two own provinces in a region of four: focus 0 scores 8 and -100, focus 1 scores 3 and 3.
    let scores = vec![vec![8.0, 3.0], vec![-100.0, 3.0]];
    assert_eq!(region_candidate(&scores, 4), Some((2.0, 0)), "negative scores are left out, the sum is divided by all four provinces");
    assert_eq!(region_candidate(&scores, 2), Some((4.0, 0)));
    assert_eq!(region_candidate(&[vec![-1.0, 0.0]], 1), None, "nothing above 0: no candidate");
    assert_eq!(region_candidate(&[vec![2.0, 2.0]], 1), Some((2.0, 0)), "of equals the first focus");
}

fn candidate(region: u16, average: f64, focus: &str) -> Candidate {
    Candidate { region, average, first_score: average, focus: focus.into() }
}

#[test]
fn the_best_regions_get_the_focuses_and_the_weakest_other_is_cancelled() {
    let current: BTreeMap<u16, String> = [(1, "desks".to_string()), (2, "desks".to_string())].into();
    let scores: BTreeMap<u16, f64> = [(1, 1.0), (2, 0.5)].into();
    // Two points. Region 3 is best, region 1 keeps its focus, region 2 falls out: no stickiness.
    let (cancel, set) = plan_focuses(vec![candidate(1, 5.0, "desks"), candidate(2, 4.0, "desks"), candidate(3, 9.0, "guns")], &current, &scores, 2, NEVER);
    assert_eq!((cancel, set), (vec![2], vec![(3, "guns".to_string())]));
    // With a third point nothing has to go.
    let (cancel, set) = plan_focuses(vec![candidate(1, 5.0, "desks"), candidate(2, 4.0, "desks"), candidate(3, 9.0, "guns")], &current, &scores, 3, NEVER);
    assert_eq!((cancel, set), (vec![], vec![(3, "guns".to_string())]));
    // A region among the best that carries another focus gets the new one in its place.
    let (cancel, set) = plan_focuses(vec![candidate(1, 5.0, "guns"), candidate(2, 4.0, "desks")], &current, &scores, 2, NEVER);
    assert_eq!((cancel, set), (vec![], vec![(1, "guns".to_string())]));
    // Everything standing is kept: the newcomer below the top two is not even looked at.
    let (cancel, set) = plan_focuses(vec![candidate(1, 5.0, "desks"), candidate(2, 4.0, "desks"), candidate(3, 3.0, "guns")], &current, &scores, 2, NEVER);
    assert_eq!((cancel, set), (vec![], vec![]));
    // A candidate whose first province alone scores nothing is passed over for the next one.
    let weak = Candidate { first_score: 0.0, ..candidate(3, 9.0, "guns") };
    let (cancel, set) = plan_focuses(vec![weak, candidate(4, 8.0, "guns"), candidate(1, 5.0, "desks")], &current, &scores, 2, NEVER);
    assert_eq!((cancel, set), (vec![2], vec![(4, "guns".to_string())]));
}

/// Country AAA: region 0 (capital, few bureaucrats), region 1 (plenty of everything), region 2
/// (no clergy); one focus point, standing on region 1.
fn world() -> (State, FocusAi) {
    let mut s = State::new(Date::new(1836, 2, 1));
    let pop = |kind: &str, size: f64| PopState { pop_type: kind.into(), size, culture: "alpha".into(), ..Default::default() };
    let province = |region: u16, pops: Vec<PopState>| ProvinceState { owner: Some("AAA".into()), region: Some(region), pops, ..Default::default() };
    s.provinces.insert(1, province(0, vec![pop("farmers", 100_000.0), pop("bureaucrats", 200.0), pop("clergymen", 5_000.0), pop("soldiers", 5_000.0)]));
    s.provinces.insert(2, province(1, vec![pop("farmers", 100_000.0), pop("bureaucrats", 2_000.0), pop("clergymen", 5_000.0), pop("soldiers", 5_000.0)]));
    s.provinces.insert(3, province(2, vec![pop("farmers", 100_000.0), pop("bureaucrats", 2_000.0), pop("soldiers", 5_000.0)]));
    s.countries.insert("AAA".into(), CountryState { capital: Some(1), primary_culture: Some("alpha".into()), civilized: true, national_focus: [(1, "promote_bureaucrats".to_string())].into(), ..Default::default() });
    let ai = FocusAi {
        weights: weights(),
        kinds: vec![("promote_bureaucrats".into(), FocusKind::Bureaucrats), ("promote_clergymen".into(), FocusKind::Clergy), ("promote_soldiers".into(), FocusKind::Soldiers), ("promote_makers".into(), FocusKind::Workers("makers".into()))],
        factory_jobs: [("mill".to_string(), (10_000.0, vec![("makers".to_string(), 0.8)]))].into(),
        points: [("AAA".to_string(), 1)].into(),
        soldiers_type: "soldiers".into(),
        bureaucrats_type: "bureaucrats".into(),
        clergy_type: "clergymen".into(),
        rail_kind: "infrastructure".into(),
        ..Default::default()
    };
    (s, ai)
}

#[test]
fn the_monthly_round_moves_the_focus_to_the_best_region() {
    let (mut s, mut ai) = world();
    // The capital with 0.18% bureaucrats scores 10 x 10 x (1 - 0.18): far above the rest.
    let done = ai.month(&mut s);
    assert_eq!(done, vec![("AAA".to_string(), 1, None), ("AAA".to_string(), 0, Some("promote_bureaucrats".to_string()))]);
    assert_eq!(s.countries["AAA"].national_focus, [(0, "promote_bureaucrats".to_string())].into());
    assert_eq!((ai.set, ai.cancelled), (1, 1));
    // Once the capital has its bureaucrats, the region without clergy is best (10 against a
    // soldier score of 30 x (1 - 4.6% / 3%) < 0), and the focus moves again.
    s.provinces.get_mut(&1).unwrap().pops[1].size = 2_000.0;
    ai.month(&mut s);
    assert_eq!(s.countries["AAA"].national_focus, [(2, "promote_clergymen".to_string())].into());
    // Nothing changes in a month in which the ranking is the same.
    assert!(ai.month(&mut s).is_empty());
    assert!(ai.summary(&s, false).starts_with("national focuses: 1 in the world (promote_clergymen 1); set 2 times and cancelled 2 times"));
}

#[test]
fn a_person_keeps_their_own_focuses() {
    let (mut s, mut ai) = world();
    s.countries.get_mut("AAA").unwrap().human = true;
    assert!(ai.month(&mut s).is_empty());
    assert_eq!(s.countries["AAA"].national_focus, [(1, "promote_bureaucrats".to_string())].into());
}

#[test]
fn an_empty_factory_draws_the_worker_focus() {
    let (mut s, mut ai) = world();
    // Everything else is satisfied; region 2 gets two running mills and only 4,000 makers for
    // 20,000 places: each mill has 0.8 - 0.2 of its places unfilled, score 2 x 0.6 x 10 = 12.
    // The capital, a little short of clergy, scores about 5.
    s.provinces.get_mut(&1).unwrap().pops[1].size = 2_000.0;
    s.provinces.get_mut(&1).unwrap().pops[2].size = 2_000.0;
    let p = s.provinces.get_mut(&3).unwrap();
    p.pops.push(PopState { pop_type: "clergymen".into(), size: 5_000.0, culture: "alpha".into(), ..Default::default() });
    p.pops.push(PopState { pop_type: "makers".into(), size: 4_000.0, culture: "alpha".into(), ..Default::default() });
    let mill = FactoryState { owner: "AAA".into(), region: Some(2), kind: "mill".into(), level: 1.0, ..Default::default() };
    s.factories = vec![mill.clone(), mill.clone(), FactoryState { closed: true, ..mill }];
    ai.month(&mut s);
    assert_eq!(s.countries["AAA"].national_focus, [(2, "promote_makers".to_string())].into());
    // People of a culture the country does not accept count against the region: the capital wins.
    for q in &mut s.provinces.get_mut(&3).unwrap().pops {
        q.culture = "beta".into();
    }
    ai.month(&mut s);
    assert_eq!(s.countries["AAA"].national_focus, [(0, "promote_clergymen".to_string())].into());
}

#[test]
fn research_weight_likes_cheap_overdue_and_school_favoured_technologies() {
    close(research_weight(&rw(), 1.0, 0.0, 0, 3600.0, 3600.0), 1.0);
    close(research_weight(&rw(), 2.0, 0.1, 10, 1800.0, 3600.0), 2.0 * 1.1 * 1.05 * 1.5);
    // The dearest technology counts as at least 3600; a technology of the future is not overdue.
    close(research_weight(&rw(), 1.0, 0.0, -5, 900.0, 1800.0), 1.75);
    close(research_weight(&rw(), 0.0, 0.5, 3, 900.0, 3600.0), 0.0);
}
