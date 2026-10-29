//! Colonisation rules on made-up numbers (no install, no save from the game).

use std::collections::HashSet;

use wiktoria_data::defines::Defines;
use wiktoria_data::save::parse_save;
use wiktoria_simulation::colonies::*;

const DEFINES: &str = "defines = {
country = {
 COLONIAL_RANK = 16, COLONY_TO_STATE_PRESTIGE_GAIN = 10, COLONIAL_LIFERATING = 35, COLONY_FORMED_PRESTIGE = 1,
 NAVAL_BASE_SUPPLY_SCORE_BASE = 10, NAVAL_BASE_SUPPLY_SCORE_EMPTY = 2, NAVAL_BASE_NON_CORE_SUPPLY_SCORE = 0.3,
 COLONIAL_POINTS_FROM_SUPPLY_FACTOR = 1, COLONIAL_POINTS_FOR_NON_CORE_BASE = 1.5,
 COLONIZATION_INTEREST_LEAD = 3, COLONIZATION_INFLUENCE_LEAD = 3, COLONIZATION_MONTHS_TO_COLONIZE = 6,
 COLONIZATION_DAYS_BETWEEN_INVESTMENT = 90, COLONIZATION_DAYS_FOR_INITIAL_INVESTMENT = 270,
 COLONIZATION_PROTECTORATE_PROVINCE_MAINTAINANCE = 4, COLONIZATION_COLONY_PROVINCE_MAINTAINANCE = 5,
 COLONIZATION_COLONY_INDUSTRY_MAINTAINANCE = 1, COLONIZATION_COLONY_RAILWAY_MAINTAINANCE = 0.1,
 COLONIZATION_INTEREST_COST_INITIAL = 100, COLONIZATION_INTEREST_COST_NEIGHBOR_MODIFIER = -20,
 COLONIZATION_INTEREST_COST = 20, COLONIZATION_INFLUENCE_COST = 20, COLONIZATION_EXTRA_GUARD_COST = 10,
 COLONIZATION_CREATE_STATE_COST = 300, COLONIZATION_CREATE_PROTECTORATE_COST = 5, COLONIZATION_CREATE_COLONY_COST = 4,
 COLONIZATION_COLONY_STATE_DISTANCE = 400, COLONIZATION_INFLUENCE_TEMPERATURE_PER_DAY = 0.08,
 COLONIZATION_INFLUENCE_TEMPERATURE_PER_LEVEL = 0.005,
},
diplomacy = { TENSION_WHILE_CRISIS = -1.0, AT_WAR_TENSION_DECAY = -0.12, PEACE_COST_COLONY = 10, INFAMY_COLONY = 0, },
}";

fn engine_rules() -> wiktoria_data::engine_rules::EngineRules {
    wiktoria_data::config::load_engine_rules(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap()
}

fn rules() -> ColonyRules {
    ColonyRules::from_defines(&Defines::parse(DEFINES).unwrap(), &engine_rules()).unwrap()
}

#[test]
fn rules_read_from_defines_and_missing_key_is_named() {
    let r = rules();
    assert_eq!((r.colonial_rank, r.days_between_investment, r.colony_cost), (16.0, 90, 4.0));
    let bad = DEFINES.replace("COLONIAL_RANK = 16,", "");
    let e = ColonyRules::from_defines(&Defines::parse(&bad).unwrap(), &engine_rules()).unwrap_err();
    assert!(e.contains("COLONIAL_RANK"), "{e}");
}

#[test]
fn naval_base_scores_and_points() {
    let r = rules();
    assert_eq!(naval_base_score(&r, 0, true, true), 2.0);
    assert_eq!(naval_base_score(&r, 1, true, true), 10.0);
    assert_eq!(naval_base_score(&r, 3, true, true), 40.0);
    assert_eq!(naval_base_score(&r, 1, false, false), 3.0);
    assert_eq!(naval_base_score(&r, 0, false, false), 1.0, "at least 1");
    assert_eq!(naval_base_score(&r, 1, false, true), 10.0, "same continent as the capital is not scaled");
    assert_eq!(overuse(150.0, 100.0), 0.5);
    assert_eq!(overuse(500.0, 100.0), 1.0);
    assert_eq!(overuse(50.0, 100.0), 0.0);
    let i = PointsInput { rank: 5.0, tech_points: 3.0, base_points: 20.0, ship_points: 10.0, supply_overuse: 0.5, bonus: 2.0 };
    assert_eq!(total_points(&r, &i), 3.0 + 20.0 + 5.0 + 2.0, "only the ships' points are cut by the overuse");
    assert_eq!(total_points(&r, &PointsInput { supply_overuse: 1.0, ..i }), 3.0 + 20.0 + 2.0, "a navy at twice its supply gives nothing");
    assert_eq!(total_points(&r, &PointsInput { rank: 17.0, ..i }), 0.0, "rank above the limit: nothing");
}

#[test]
fn a_naval_base_gives_the_points_of_its_level() {
    let list = [7.0, 11.0, 13.0];
    assert_eq!(naval_base_points(&list, 0), 0.0, "no base, no points");
    assert_eq!(naval_base_points(&list, 1), 7.0);
    assert_eq!(naval_base_points(&list, 2), 11.0);
    assert_eq!(naval_base_points(&list, 5), 13.0, "past the list: its last entry");
    assert_eq!(naval_base_points(&[], 2), 0.0);
    assert_eq!(rules().non_core_base_points, 1.5);
}

#[test]
fn technologies_with_points_are_kept_by_name() {
    let r = rules().with_technologies([("sails", 40.0), ("ropes", 0.0)].into_iter());
    assert_eq!(r.tech_points.len(), 1);
    assert_eq!(r.tech_points["sails"], 40.0);
}

#[test]
fn the_state_distance_is_the_straight_line_in_pixels_over_four() {
    let r = rules();
    assert_eq!(state_distance(&r, (100.0, 50.0), (400.0, 450.0)), 125.0);
    assert_eq!(state_cost(&r, 3, state_distance(&r, (0.0, 0.0), (0.0, 800.0)), false), 450.0);
}

#[test]
fn upkeep_by_status_and_rounding() {
    let r = rules();
    let prot = Holding { status: Status::Protectorate, provinces: 3, factories: 2, railway_levels: 5.0 };
    let col = Holding { status: Status::Colony, provinces: 2, factories: 3, railway_levels: 4.0 };
    let st = Holding { status: Status::State, provinces: 9, factories: 9, railway_levels: 9.0 };
    assert_eq!(upkeep_of(&r, &prot), 12.0, "protectorate: provinces only");
    assert!((upkeep_of(&r, &col) - (10.0 + 3.0 + 0.4)).abs() < 1e-9);
    assert_eq!(upkeep_of(&r, &st), 0.0);
    assert_eq!(points_used(&r, 3.0, 20.0, &[prot, col, st]), 45.0);
    assert_eq!(points_used(&r, 30.0, 20.0, &[]), 0.0);
    assert_eq!(free_points(50.0, 45.0), 5.0);
}

#[test]
fn only_states_allow_factories() {
    assert!(Status::State.allows_factories());
    assert!(!Status::Colony.allows_factories());
    assert!(!Status::Protectorate.allows_factories());
    assert_eq!(Status::from_save(2.0), Status::Colony);
    assert_eq!(Status::Protectorate.to_save(), 1);
}

fn cand() -> Candidate {
    Candidate { rank: 3.0, great_power: true, best_life_rating: 40.0, life_rating_limit: 35.0, at_great_war: false, region_is_crisis: false, in_reach: true, borders_region: false }
}

#[test]
fn who_may_step() {
    let r = rules();
    let race = Race::default();
    assert_eq!(may_step(&r, &cand(), &race, "ENG", 1000), Ok(()));
    assert_eq!(may_step(&r, &Candidate { rank: 20.0, great_power: false, ..cand() }, &race, "X", 1000), Err(Refusal::NotGreatOrSecondary));
    assert_eq!(may_step(&r, &Candidate { rank: 20.0, great_power: true, ..cand() }, &race, "X", 1000), Ok(()), "a great power is always allowed");
    assert_eq!(may_step(&r, &Candidate { best_life_rating: 30.0, ..cand() }, &race, "ENG", 1000), Err(Refusal::LifeRating));
    assert_eq!(may_step(&r, &Candidate { best_life_rating: 30.0, life_rating_limit: 25.0, ..cand() }, &race, "ENG", 1000), Ok(()), "technology lowers the limit");
    assert_eq!(may_step(&r, &Candidate { region_is_crisis: true, ..cand() }, &race, "ENG", 1000), Err(Refusal::CrisisRegion));
    assert_eq!(may_step(&r, &Candidate { at_great_war: true, ..cand() }, &race, "ENG", 1000), Err(Refusal::AtWar));
    assert_eq!(may_step(&r, &Candidate { in_reach: false, ..cand() }, &race, "ENG", 1000), Err(Refusal::OutOfReach));
}

#[test]
fn step_costs() {
    let r = rules();
    assert_eq!(step_cost(&r, Stage::Open, false, false, 0.0), 100.0);
    assert_eq!(step_cost(&r, Stage::Open, false, true, 0.0), 80.0, "neighbour discount");
    assert_eq!(step_cost(&r, Stage::Interest, true, true, 2.0), 20.0);
    assert_eq!(step_cost(&r, Stage::Influence, true, false, 4.0), 20.0);
    assert_eq!(step_cost(&r, Stage::Influence, true, false, 6.0), 40.0, "extra guard above level 4");
    assert_eq!(step_cost(&r, Stage::Won, true, false, 6.0), 0.0);
}

#[test]
fn spacing_between_steps_and_race_slots() {
    let r = rules();
    let mut race = Race::default();
    assert!(race.add_step(&r, "ENG", 100.0, 0));
    assert_eq!(may_step(&r, &cand(), &race, "ENG", 500), Err(Refusal::AlreadyIn), "stage 0: a lone contender waits");
    assert_eq!(may_step(&r, &cand(), &race, "FRA", 10), Ok(()), "a newcomer is not held back");
    assert!(race.add_step(&r, "FRA", 100.0, 5));
    race.stage = Stage::Interest;
    assert_eq!(may_step(&r, &cand(), &race, "ENG", 50), Err(Refusal::TooSoon));
    assert_eq!(may_step(&r, &cand(), &race, "ENG", 90), Ok(()));
    for t in ["RUS", "USA"] {
        assert!(race.add_step(&r, t, 100.0, 5));
    }
    assert!(!race.add_step(&r, "SPA", 100.0, 5), "four slots only");
}

fn two_way(a: f64, b: f64) -> Race {
    Race {
        stage: Stage::Interest,
        date: 0,
        temperature: 0.0,
        contenders: vec![Contender { tag: "ENG".into(), level: a, points: 0.0, last_step: 0 }, Contender { tag: "FRA".into(), level: b, points: 0.0, last_step: 0 }],
    }
}

#[test]
fn a_lone_contender_wins_after_the_initial_period() {
    let r = rules();
    let mut race = Race::default();
    race.add_step(&r, "ENG", 100.0, 0);
    assert_eq!(race.contenders[0].level, r.first_step_level);
    assert_eq!(race.contenders[0].points, 100.0);
    race.daily(&r, 270, 30, &Climate::default());
    assert_eq!(race.stage, Stage::Open);
    race.daily(&r, 271, 30, &Climate::default());
    assert_eq!(race.stage, Stage::Won);
    assert_eq!(race.contenders[0].level, 3.0, "level set to the interest lead");
    assert_eq!(race.contenders[0].points, 0.0, "the invested points are released when the race is won");
    // the win lapses after six months
    race.daily(&r, 271 + 180, 30, &Climate::default());
    assert_eq!(race.stage, Stage::Won);
    race.daily(&r, 271 + 181, 30, &Climate::default());
    assert_eq!(race, Race::default());
}

#[test]
fn second_contender_opens_interest_stage() {
    let r = rules();
    let mut race = Race::default();
    race.add_step(&r, "ENG", 100.0, 0);
    race.add_step(&r, "FRA", 100.0, 1);
    race.daily(&r, 2, 30, &Climate::default());
    assert_eq!(race.stage, Stage::Interest);
}

#[test]
fn interest_narrows_to_two_with_influence_stage() {
    let r = rules();
    // lead of three over the other: narrows, second is set to top - 1, leader's points to the influence cost
    let mut race = two_way(4.0, 1.0);
    race.daily(&r, 10, 30, &Climate::default());
    assert_eq!(race.stage, Stage::Influence);
    assert_eq!(race.contenders[0].level, 4.0);
    assert_eq!(race.contenders[1].level, 3.0);
    assert_eq!(race.contenders[0].points, 20.0);
    // exactly two contenders and the top at the lead: also narrows
    let mut race = two_way(3.0, 2.0);
    race.daily(&r, 10, 30, &Climate::default());
    assert_eq!(race.stage, Stage::Influence);
    // below the lead nothing happens
    let mut race = two_way(2.0, 1.0);
    race.daily(&r, 10, 30, &Climate::default());
    assert_eq!(race.stage, Stage::Interest);
}

#[test]
fn three_contenders_throw_out_the_trailing() {
    let r = rules();
    let mut race = two_way(5.0, 4.0);
    race.contenders.push(Contender { tag: "RUS".into(), level: 1.0, points: 0.0, last_step: 0 });
    race.daily(&r, 10, 30, &Climate::default());
    assert_eq!(race.contenders.len(), 2);
    assert!(race.contender("RUS").is_none());
    assert_eq!(race.stage, Stage::Influence);
}

#[test]
fn influence_ends_on_the_lead_and_temperature_runs() {
    let r = rules();
    let mut race = two_way(4.0, 3.0);
    race.stage = Stage::Influence;
    let t0 = Climate::default();
    race.daily(&r, 10, 30, &t0);
    assert_eq!(race.stage, Stage::Influence);
    assert!((race.temperature - (0.08 + 0.005 * 4.0)).abs() < 1e-9);
    // a crisis lowers it, never below zero
    let c = Climate { crisis_running: true, ..Climate::default() };
    race.daily(&r, 11, 30, &c);
    assert_eq!(race.temperature, 0.0);
    // the crisis region sits at 100
    race.daily(&r, 12, 30, &Climate { is_crisis_region: true, ..Climate::default() });
    assert_eq!(race.temperature, 100.0);
    // the lead of three ends it
    race.contenders[0].level = 6.0;
    race.daily(&r, 13, 30, &t0);
    assert_eq!(race.stage, Stage::Won);
    assert_eq!(race.contenders.len(), 1);
    assert_eq!(race.contenders[0].tag, "ENG");
    assert_eq!(race.temperature, 0.0, "temperature only in stage 2");
}

#[test]
fn stage_two_admits_nobody_new() {
    let mut race = two_way(4.0, 3.0);
    race.stage = Stage::Influence;
    let r = rules();
    assert!(!race.add_step(&r, "RUS", 20.0, 5));
    assert_eq!(may_step(&r, &cand(), &race, "RUS", 500), Err(Refusal::NotInRace));
    assert_eq!(may_step(&r, &cand(), &race, "ENG", 500), Ok(()));
    assert!(race.can_step_by_day(&r, "ENG", 90));
    assert!(!race.can_step_by_day(&r, "ENG", 89));
}

#[test]
fn creating_protectorate_colony_and_state() {
    let r = rules();
    let mut race = Race { stage: Stage::Won, date: 5, temperature: 0.0, contenders: vec![Contender { tag: "ENG".into(), level: 3.0, points: 0.0, last_step: 0 }] };
    assert_eq!(create_protectorate(&r, &mut race.clone(), "FRA", 4, 100.0), Err(Refusal::RaceClosed));
    assert_eq!(create_protectorate(&r, &mut race.clone(), "ENG", 4, 4.0), Err(Refusal::NotEnoughPoints));
    let c = create_protectorate(&r, &mut race, "ENG", 4, 5.0).unwrap();
    assert_eq!((c.cost, c.prestige), (5.0, 4.0), "prestige for each province");
    assert_eq!(race, Race::default());

    assert_eq!(create_colony(&r, Status::Colony, 3, 100.0), Err(Refusal::WrongStatus));
    assert_eq!(create_colony(&r, Status::Protectorate, 3, 11.0), Err(Refusal::NotEnoughPoints));
    assert_eq!(create_colony(&r, Status::Protectorate, 3, 12.0).unwrap().cost, 12.0);

    assert_eq!(state_cost(&r, 2, 200.0, false), 300.0);
    assert_eq!(state_cost(&r, 2, 900.0, false), 600.0, "distance factor caps at 1");
    assert_eq!(state_cost(&r, 2, 900.0, true), 0.0, "own continent is free");
    assert_eq!(create_state(&r, Status::Colony, true, true, 10.0, 100.0), Err(Refusal::AtWar));
    assert_eq!(create_state(&r, Status::Protectorate, false, true, 10.0, 100.0), Err(Refusal::WrongStatus));
    assert_eq!(create_state(&r, Status::Colony, false, false, 10.0, 100.0), Err(Refusal::ConditionsNotMet));
    assert_eq!(create_state(&r, Status::Colony, false, true, 10.0, 5.0), Err(Refusal::NotEnoughPoints));
    let s = create_state(&r, Status::Colony, false, true, 10.0, 10.0).unwrap();
    assert_eq!(s.prestige, 10.0);
}

#[test]
fn failed_colony_goal_loses_the_claim() {
    let r = rules();
    let mut race = two_way(2.0, 1.0);
    assert!(race.lose_claim("ENG"));
    assert_eq!(race.stage, Stage::Open, "one contender is left: back to the open stage");
    assert!(!race.lose_claim("ENG"));
    assert!(race.lose_claim("FRA"));
    assert_eq!(race, Race::default());
    // the winner of a stage-3 race loses it too
    let mut won = Race { stage: Stage::Won, date: 1, temperature: 0.0, contenders: vec![Contender { tag: "ENG".into(), level: 3.0, points: 0.0, last_step: 0 }] };
    assert!(won.lose_claim("ENG"));
    assert_eq!(won, Race::default());
    // po_colony resets the race and reports the consequences from the defines
    let mut race = two_way(3.0, 1.0);
    let p = po_colony(&r, &mut race, 5);
    assert_eq!((p.provinces, p.prestige, p.infamy, p.war_score_cost), (5, 5.0, 0.0, 10.0));
    assert_eq!(race, Race::default());
}

#[test]
fn ai_picks_the_cheapest_per_province_it_can_pay() {
    let o = |region, cost, provinces| RegionOption { region, cost, provinces };
    let opts = [o(1, 100.0, 2), o(2, 100.0, 6), o(3, 500.0, 40), o(4, 80.0, 6)];
    assert_eq!(ai_pick_region(&opts, 150.0), Some(4));
    assert_eq!(ai_pick_region(&opts, 50.0), None);
    assert_eq!(ai_pick_region(&opts, 1000.0), Some(3));
}

const SAVE: &str = r#"
date="1850.1.1"
great_nations=
{
1 }
region=
{
	index=6
	phase=0
	date="1849.3.1"
	temperature=0.000
}
region=
{
	index=7
	phase=1
	temperature=0.500
	colony=
	{
		tag="ENG"
		points=2
		invest=120
		date="1849.10.3"
	}
	colony=
	{
		tag="FRA"
		points=1
		invest=80
		date="1849.11.5"
	}
}
region=
{
	index=9
	phase=0
	temperature=0.000
	colony=
	{
		tag="ENG"
		points=1
		invest=100
		date="1849.12.1"
	}
}
ENG=
{
	colonize=
	{
		7=2
	}
	state=
	{
		provinces=
		{
2 3 		}
		is_colonial=2
		state_buildings=
		{
			id=1
		}
	}
	state=
	{
		provinces=
		{
1 		}
		is_colonial=0
	}
}
FRA=
{
	state=
	{
		provinces=
		{
4 		}
		is_colonial=1
	}
}
1=
{
	name="A"
	owner="ENG"
}
2=
{
	name="B"
	owner="ENG"
	colonial=2
	railroad=
	{
 3.000 3.000 	}
}
3=
{
	name="C"
	owner="ENG"
	colonial=2
	naval_base=
	{
 1.000 1.000 	}
}
4=
{
	name="D"
	owner="FRA"
	colonial=1
}
"#;

#[test]
fn state_from_a_made_up_save() {
    let save = parse_save(SAVE.as_bytes(), &HashSet::new(), &HashSet::new()).unwrap();
    let c = ColonyState::from_save(&save);
    assert!(c.countries["ENG"].great_power);
    assert!(!c.countries["FRA"].great_power);
    assert_eq!(c.countries["ENG"].races.get(&7), Some(&2.0));
    assert_eq!(c.counts_by_province("ENG"), (2, 0));
    assert_eq!(c.counts_by_province("FRA"), (0, 1));
    assert_eq!(c.counts_by_state("ENG"), (1, 0));
    assert_eq!(c.counts_by_state("FRA"), (0, 1));
    let h = c.holdings_of("ENG");
    assert_eq!(h.len(), 1);
    assert_eq!((h[0].provinces, h[0].factories, h[0].railway_levels), (2, 1, 3.0));
    assert_eq!(c.provinces[&3].naval_base, 1.0);
    assert_eq!(c.races.len(), 3);
    assert_eq!((c.races[1].region, c.races[1].stage, c.races[1].temperature), (7, Stage::Interest, 0.5));
    assert_eq!(c.races[1].contenders[1], ("FRA".to_string(), 1.0, 80.0, wiktoria_data::date::Date::parse("1849.11.5")));
    assert_eq!(c.races[0].date, wiktoria_data::date::Date::parse("1849.3.1"));
    assert_eq!((c.invested("ENG"), c.invested("FRA"), c.invested("RUS")), (220.0, 80.0, 0.0));
    let r = rules();
    assert_eq!(upkeep_of(&r, &h[0]), 10.0 + 1.0 + 0.3);
}
