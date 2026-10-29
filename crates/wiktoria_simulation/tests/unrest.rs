//! Rebels and movements (unrest.rs) on synthetic numbers: the defines and rebel types are made up
//! here, rules file literals come from the repo's engine_rules.

use std::collections::HashSet;

use wiktoria_data::date::Date;
use wiktoria_data::defines::Defines;
use wiktoria_data::defs::{parse_cultures, parse_issues};
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_simulation::unrest::*;

const DEFINES: &str = r#"
defines = {
	country = {
		MOVEMENT_RADICALISM_BASE = 25,
		MOVEMENT_RADICALISM_PASSED_REFORM_EFFECT = -2,
		MOVEMENT_RADICALISM_NATIONALISM_FACTOR = 1.0,
		SUPPRESSION_POINTS_GAIN_BASE = 170,
		SUPPRESS_BUREAUCRAT_FACTOR = 0.5,
		SUPPRESSION_RADICALISATION_HIT = 10,
		MAX_SUPPRESSION = 100,
		PRESTIGE_HIT_ON_BREAK_COUNTRY = -0.20,
		MIL_TO_JOIN_REBEL = 7,
		MIL_TO_JOIN_RISING = 8,
		MIL_TO_AUTORISE = 9,
		REDUCTION_AFTER_RISEING = 0.0,
		REDUCTION_AFTER_DEFEAT = 3.0,
		ISSUE_MOVEMENT_JOIN_LIMIT = 8,
		ISSUE_MOVEMENT_LEAVE_LIMIT = 7,
		MOVEMENT_CON_FACTOR = 0.05,
		MOVEMENT_LIT_FACTOR = 0.3,
		MIL_ON_REB_MOVE = 8,
		POPULATION_SUPPRESSION_FACTOR = 0.0,
		POPULATION_MOVEMENT_RADICAL_FACTOR = 300,
		NATIONALIST_MOVEMENT_MIL_CAP = 3.0,
		MOVEMENT_SUPPORT_UH_FACTOR = 3,
		REBEL_OCCUPATION_STRENGTH_BONUS = 0.01,
	},
	military = {
		POP_MIN_SIZE_FOR_REGIMENT = 1000,
		POP_SIZE_PER_REGIMENT = 3000,
	},
}
"#;

const TYPES: &str = r#"
reactionary_rebels = {
	area = nation
	break_alliance_on_win = yes
	government = { democracy = presidential_dictatorship absolute_monarchy = absolute_monarchy }
	defection = none
	independence = none
	defect_delay = 18
	ideology = reactionary
	allow_all_cultures = yes
	allow_all_religions = yes
	allow_all_ideologies = yes
	unit_transfer = no
	occupation_mult = 1.0
	demands_enforced_effect = { political_reform = none_voting }
}
nationalist_rebels = {
	area = nation_culture
	defection = culture
	independence = culture
	defect_delay = 12
	allow_all_cultures = no
	allow_all_religions = yes
	allow_all_ideologies = yes
	unit_transfer = yes
	occupation_mult = 5
}
pan_nationalist_rebels = {
	area = culture_group
	government = { democracy = democracy absolute_monarchy = democracy }
	defection = pan_nationalist
	independence = none
	defect_delay = 12
	ideology = liberal
	allow_all_cultures = yes
	allow_all_religions = yes
	allow_all_ideologies = yes
	allow_all_culture_groups = no
	occupation_mult = 3
	demands_enforced_effect = { random_country = { limit = { is_culture_group = THIS } country_event = 11100 } }
}
"#;

const CULTURES: &str = r#"
german = {
	union = GER
	north_german = { color = { 1 1 1 } radicalism = 5 }
	south_german = { color = { 1 1 1 } radicalism = 15 }
}
latin = {
	north_italian = { color = { 1 1 1 } radicalism = 10 }
}
nounion = {
	oddlings = { color = { 1 1 1 } }
}
"#;

fn rules() -> UnrestRules {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/engine_rules.json");
    let m = EngineRules::from_json(&std::fs::read_to_string(p).unwrap()).unwrap();
    UnrestRules::new(&m, &Defines::parse(DEFINES).unwrap()).unwrap()
}

fn types() -> Vec<RebelType> {
    RebelType::parse_file(TYPES.as_bytes()).unwrap()
}

fn kind<'a>(ts: &'a [RebelType], n: &str) -> &'a RebelType {
    ts.iter().find(|t| t.name == n).unwrap()
}

fn pop(militancy: f64) -> PopFacts {
    PopFacts { id: 1, size: 5000.0, literacy: 0.5, consciousness: 3.0, militancy, culture: "north_german".into(), culture_group: "german".into(), religion: "protestant".into(), ..Default::default() }
}

#[test]
fn rules_come_from_defines_and_engine_rules() {
    let r = rules();
    assert_eq!(r.mil_to_join_rebel, 7.0);
    assert_eq!(r.radicalism_base, 25.0);
    assert_eq!(r.random_range, 1000);
    assert_eq!(r.rise_day_mod, 30);
}

#[test]
fn types_are_read_from_the_file() {
    let ts = types();
    assert_eq!(ts.len(), 3);
    let n = kind(&ts, "nationalist_rebels");
    assert_eq!((n.area, n.defection, n.independence, n.defect_delay), (Area::NationCulture, Aim::Culture, Aim::Culture, 12));
    assert!(n.unit_transfer && !n.allow_all_cultures);
    let p = kind(&ts, "pan_nationalist_rebels");
    assert_eq!((p.area, p.defection), (Area::CultureGroup, Aim::PanNationalist));
    assert!(p.demands_enforced_effect.is_some());
    assert_eq!(kind(&ts, "reactionary_rebels").government["democracy"], "presidential_dictatorship");
}

#[test]
fn join_and_leave_thresholds() {
    let r = rules();
    assert!(!may_join_faction(&r, &pop(7.0)));
    assert!(may_join_faction(&r, &pop(7.1)));
    let mut p = pop(9.0);
    p.in_movement = true;
    assert!(!may_join_faction(&r, &p));
    p.in_movement = false;
    p.in_faction = true;
    assert!(!may_join_faction(&r, &p));
    assert!(leaves_faction(&r, &pop(6.9)));
    assert!(!leaves_faction(&r, &pop(7.0)));
}

#[test]
fn highest_spawn_value_wins_and_zero_is_skipped() {
    let ts = types();
    let mut p = pop(9.0);
    p.top_ideology = Some("liberal".into());
    let value = |t: &RebelType| match t.name.as_str() {
        "reactionary_rebels" => 1.0,
        "pan_nationalist_rebels" => 1.5,
        "nationalist_rebels" => 0.0,
        _ => 0.0,
    };
    let got = pick_rebel_type(&ts, &p, &value, &|_| true).unwrap();
    assert_eq!(got.name, "pan_nationalist_rebels");
    // equal values: the first in file order keeps it
    let tie = |_: &RebelType| 1.0;
    assert_eq!(pick_rebel_type(&ts, &p, &tie, &|_| true).unwrap().name, "reactionary_rebels");
    // a type that wants a country is out when none is found
    let want_nat = |t: &RebelType| if t.name == "nationalist_rebels" { 9.0 } else { 0.0 };
    assert!(pick_rebel_type(&ts, &p, &want_nat, &|_| false).is_none());
    assert_eq!(pick_rebel_type(&ts, &p, &want_nat, &|_| true).unwrap().name, "nationalist_rebels");
}

#[test]
fn ideology_restriction_applies_unless_all_allowed() {
    let src = "a = { ideology = fascist } b = { ideology = liberal allow_all_ideologies = yes }";
    let ts = RebelType::parse_file(src.as_bytes()).unwrap();
    let mut p = pop(9.0);
    p.top_ideology = Some("liberal".into());
    assert_eq!(pick_rebel_type(&ts, &p, &|_| 1.0, &|_| true).unwrap().name, "b");
}

#[test]
fn factions_accept_by_culture_group_culture_religion_and_country() {
    let ts = types();
    let nat = kind(&ts, "nationalist_rebels");
    let f = Faction { kind: "nationalist_rebels".into(), culture: "north_german".into(), culture_group: "german".into(), religion: "protestant".into(), creates: Some("HAN".into()), ..Default::default() };
    let p = pop(9.0);
    assert!(faction_accepts(nat, &f, &p, Some("HAN")));
    assert!(!faction_accepts(nat, &f, &p, Some("GER")));
    let mut other = pop(9.0);
    other.culture = "south_german".into();
    assert!(!faction_accepts(nat, &f, &other, Some("HAN")), "nationalists need the same culture");
    let pan = kind(&ts, "pan_nationalist_rebels");
    let pf = Faction { kind: "pan_nationalist_rebels".into(), culture: "north_german".into(), culture_group: "german".into(), religion: "protestant".into(), ..Default::default() };
    assert!(faction_accepts(pan, &pf, &other, None), "pan-nationalists take the whole culture group");
    let mut italian = pop(9.0);
    italian.culture = "north_italian".into();
    italian.culture_group = "latin".into();
    assert!(!faction_accepts(pan, &pf, &italian, None));
}

#[test]
fn joining_dilutes_organisation() {
    assert!((organisation_after_join(0.6, 3, 1) - 0.45).abs() < 1e-12);
    assert_eq!(organisation_after_join(0.6, 0, 0), 0.6);
}

#[test]
fn brigades_follow_pop_size() {
    let r = rules();
    assert_eq!(brigades(&r, 999.0), 0);
    assert_eq!(brigades(&r, 1000.0), 1);
    assert_eq!(brigades(&r, 6000.0), 3);
}

#[test]
fn organisation_gain_uses_money_literacy_militancy_and_brigades() {
    let r = rules();
    let m = |money: f64, mil: f64| Member { money_today: money, literacy: 0.5, militancy: mil, brigades: 2 };
    let calm = organisation_gain(&r, &[m(10.0, 6.0)], 0.0, 0.0);
    let angry = organisation_gain(&r, &[m(10.0, 8.5)], 0.0, 0.0);
    let furious = organisation_gain(&r, &[m(10.0, 9.5)], 0.0, 0.0);
    assert!((angry / calm - 5.0).abs() < 1e-9);
    assert!((furious / calm - 10.0).abs() < 1e-9);
    assert!(organisation_gain(&r, &[m(10.0, 6.0)], 1.0, 0.0) < calm, "administration spending slows it");
    assert!(organisation_gain(&r, &[m(10.0, 6.0)], 0.0, 1.0) > calm, "technology speeds it");
    assert_eq!(organisation_gain(&r, &[Member { brigades: 0, ..m(10.0, 9.0) }], 0.0, 0.0), 0.0);
    assert_eq!(organisation_gain(&r, &[m(-5.0, 9.0)], 0.0, 0.0), 0.0, "a negative day is skipped");
    assert_eq!(organisation_gain(&r, &[Member { money_today: 1e9, ..m(0.0, 9.5) }], 0.0, 0.0), 1.0, "kept in 0..1");
}

#[test]
fn rising_day_and_chance() {
    let r = rules();
    let mut seq = [60u64, 30, 47].into_iter();
    assert_eq!(rising_day(&r, || seq.next().unwrap()), 17, "redrawn while 0");
    assert_eq!(rising_chance(&r, 0.5, 0, 10), None, "no roll without angry brigades");
    // org 0.5, R = 4, G = 10: 0.5 x 1000 x 4 / 200 + 25 - 20 = 15
    assert!((rising_chance(&r, 0.5, 4, 10).unwrap() - 15.0).abs() < 1e-9);
    // a big army shrinks the first term to nothing, leaving 50 x org - 20
    assert!((rising_chance(&r, 1.0, 1, 100_000).unwrap() - 30.0).abs() < 0.1);
    assert_eq!(rising_chance(&r, 0.0, 3, 10).unwrap(), 0.0, "kept at 0");
    assert_eq!(rising_chance(&r, 1.0, 1000, 1).unwrap(), 1000.0, "kept at 1000");
    assert!(rises(&r, 15.0, 1014));
    assert!(!rises(&r, 15.0, 1015));
}

#[test]
fn a_rising_orders_regiments_and_leaves_militancy() {
    let r = rules();
    let f = Faction { id: 7, organization: 0.5, ..Default::default() };
    let members = [
        RisingPop { pop_id: 1, size: 9000.0, militancy: 9.0, brigades_in_service: 0, province: 100, rebel_supply_limit: 30.0 },
        RisingPop { pop_id: 2, size: 9000.0, militancy: 7.9, brigades_in_service: 0, province: 101, rebel_supply_limit: 30.0 },
        RisingPop { pop_id: 3, size: 500.0, militancy: 10.0, brigades_in_service: 0, province: 102, rebel_supply_limit: 30.0 },
        RisingPop { pop_id: 4, size: 9000.0, militancy: 8.0, brigades_in_service: 2, province: 103, rebel_supply_limit: 3.0 },
        RisingPop { pop_id: 5, size: 90_000.0, militancy: 8.0, brigades_in_service: 0, province: 104, rebel_supply_limit: 9.0 },
    ];
    let out = rise(&r, &f, &members);
    // pop 1: 9000/3000+1 = 4 brigades x 0.5 = 2; pop 2 below 8; pop 3 too small; pop 4: (4-2) x .5 = 1 cut to round(3/3)=1;
    // pop 5: 31 x .5 = 15 cut to 3
    assert_eq!(
        out.orders,
        vec![
            SpawnOrder { faction: 7, pop_id: 1, province: 100, regiments: 2 },
            SpawnOrder { faction: 7, pop_id: 4, province: 103, regiments: 1 },
            SpawnOrder { faction: 7, pop_id: 5, province: 104, regiments: 3 },
        ]
    );
    assert_eq!(out.organisation, 0.0);
    assert_eq!(out.militancy[&1], 9.0, "a rising does not reset militancy");
    assert!(!out.militancy.contains_key(&2));
}

#[test]
fn occupation_bonus_skips_the_multiplier_for_culture_defection() {
    let r = rules();
    let ts = types();
    assert_eq!(occupation_bonus(&r, kind(&ts, "nationalist_rebels")), 0.01);
    assert!((occupation_bonus(&r, kind(&ts, "pan_nationalist_rebels")) - 0.03).abs() < 1e-12);
}

#[test]
fn finished_factions() {
    let ts = types();
    let nat = kind(&ts, "nationalist_rebels");
    let f = Faction { creates: Some("HAN".into()), pops: vec![1], ..Default::default() };
    assert!(!faction_finished(nat, &f, true, false, true, false));
    assert!(faction_finished(nat, &f, false, false, true, false), "its country owns nothing");
    assert!(faction_finished(nat, &f, true, true, true, false), "the country it wants exists");
    assert!(faction_finished(nat, &Faction { pops: vec![], ..f.clone() }, true, false, true, false), "no members");
    let pan = kind(&ts, "pan_nationalist_rebels");
    assert!(faction_finished(pan, &Faction { pops: vec![1], ..Default::default() }, true, false, true, true), "union is its own country");
}

#[test]
fn pan_nationalist_faction_creates_the_union_of_the_culture_group() {
    let cultures = parse_cultures(CULTURES).unwrap();
    let ts = types();
    let pan = kind(&ts, "pan_nationalist_rebels");
    assert_eq!(union_tag(&cultures, "south_german").as_deref(), Some("GER"));
    assert_eq!(union_tag(&cultures, "oddlings"), None);
    assert_eq!(union_tag(&cultures, "unknown"), None);
    assert_eq!(faction_country(pan, "north_german", None, &cultures).as_deref(), Some("GER"));
    assert_eq!(faction_country(pan, "north_german", Some("---"), &cultures).as_deref(), Some("GER"));
    // nationalists create what the faction names
    assert_eq!(faction_country(kind(&ts, "nationalist_rebels"), "north_german", Some("HAN"), &cultures).as_deref(), Some("HAN"));
    assert_eq!(faction_country(kind(&ts, "reactionary_rebels"), "north_german", Some("---"), &cultures), None);
}

#[test]
fn winning_hands_the_types_effect_to_the_script_runner() {
    let r = rules();
    let cultures = parse_cultures(CULTURES).unwrap();
    let ts = types();
    let f = Faction { kind: "pan_nationalist_rebels".into(), culture: "north_german".into(), ..Default::default() };
    let d = demands_enforced(&r, kind(&ts, "pan_nationalist_rebels"), &f, "absolute_monarchy", &cultures);
    assert_eq!(d.government.as_deref(), Some("democracy"));
    assert_eq!(d.creates.as_deref(), Some("GER"));
    assert_eq!(d.prestige_hit, -0.20);
    let effect = d.effect.expect("the event comes from the type's own effect block");
    assert!(effect.get("random_country").is_some());
    // the government stays when the type wants what it already is
    let re = Faction { kind: "reactionary_rebels".into(), ..Default::default() };
    let d = demands_enforced(&r, kind(&ts, "reactionary_rebels"), &re, "absolute_monarchy", &cultures);
    assert_eq!(d.government, None);
    assert!(d.break_alliances && !d.unit_transfer);
    let d = demands_enforced(&r, kind(&ts, "reactionary_rebels"), &re, "democracy", &cultures);
    assert_eq!(d.government.as_deref(), Some("presidential_dictatorship"));
    assert!((prestige_after_break(&r, 100.0, 20.0) - 76.0).abs() < 1e-9);
}

#[test]
fn defection_waits_defect_delay_months() {
    let ts = types();
    let nat = kind(&ts, "nationalist_rebels");
    let since = Date::new(1850, 3, 10);
    assert!(!defection_due(nat, since, Date::new(1851, 3, 9)));
    assert!(defection_due(nat, since, Date::new(1851, 3, 10)));
    assert!(!defection_due(kind(&ts, "reactionary_rebels"), since, Date::new(1900, 1, 1)), "defection = none never defects");
    assert_eq!(months_between(Date::new(1850, 12, 31), Date::new(1851, 1, 1)), 0);
}

#[test]
fn defection_goes_to_the_best_matching_country_never_the_owner() {
    let ts = types();
    let nat = kind(&ts, "nationalist_rebels");
    let f = Faction { culture: "north_german".into(), culture_group: "german".into(), religion: "protestant".into(), ..Default::default() };
    let c = |tag: &str, cult: &str, grp: &str| Candidate { tag: tag.into(), primary_culture: cult.into(), culture_group: grp.into(), religion: "protestant".into(), ..Default::default() };
    let list = vec![c("AUS", "south_german", "german"), c("HAN", "north_german", "german"), c("FRA", "french", "latin"), c("PRU", "north_german", "german")];
    assert_eq!(defection_target(nat, &f, "PRU", &list).unwrap().tag, "HAN");
    assert_eq!(defection_target(nat, &f, "HAN", &list).unwrap().tag, "PRU");
    assert!(defection_target(kind(&ts, "reactionary_rebels"), &f, "X", &list).is_none());
    let pan = kind(&ts, "pan_nationalist_rebels");
    let mut union = c("GER", "north_german", "german");
    union.union_of_group = true;
    let list = vec![c("AUS", "south_german", "german"), union];
    assert_eq!(defection_target(pan, &f, "AUS", &list).unwrap().tag, "GER");
}

#[test]
fn issue_movement_join_rule() {
    let r = rules();
    let mut p = pop(1.0);
    p.literacy = 0.8;
    p.consciousness = 6.0;
    let opts = vec![
        IssueOption { name: "a".into(), support: 8.0, available: true },
        IssueOption { name: "b".into(), support: 20.0, available: true },
        IssueOption { name: "c".into(), support: 40.0, available: false },
        IssueOption { name: "d".into(), support: 12.0, available: true },
    ];
    let (o, n) = issue_choice(&r, &p, &opts).unwrap();
    assert_eq!(o.name, "b", "highest available support strictly above the join limit");
    // (0.8 x 0.3 + 6 x 0.05) x 20 x 9 = 97.2
    assert_eq!(n, 97.0);
    assert!(joins_issue(&r, n, 96));
    assert!(!joins_issue(&r, n, 97));
    assert!(!joins_issue(&r, 10.0, 0), "N must exceed 10");
    let weak = vec![IssueOption { name: "a".into(), support: 8.0, available: true }];
    assert!(issue_choice(&r, &p, &weak).is_none(), "support must be strictly above 8");
}

#[test]
fn staying_and_leaving_movements() {
    let r = rules();
    assert!(stays_in_issue_movement(&r, 7.1));
    assert!(!stays_in_issue_movement(&r, 7.0));
    assert!(stays_in_independence_movement(&r, 3.0));
    assert!(!stays_in_independence_movement(&r, 2.9));
}

#[test]
fn independence_movement_join_rule() {
    let r = rules();
    let mut p = pop(3.5);
    p.culture = "north_italian".into();
    assert!(joins_independence(&r, &p, "north_german", 19));
    assert!(!joins_independence(&r, &p, "north_german", 20));
    assert!(!joins_independence(&r, &p, "north_italian", 5), "the owner's own culture does not");
    p.militancy = 3.0;
    assert!(!joins_independence(&r, &p, "north_german", 5));
}

#[test]
fn radicalism_formula() {
    let r = rules();
    // base 25, three reform steps passed -6, size share 1 % x 300 = 3
    let i = RadicalismInput { reform_steps_passed: 3.0, size: 1000.0, country_population: 100_000.0, ..Default::default() };
    assert!((radicalism(&r, &i) - 22.0).abs() < 1e-9);
    // suppression adds
    let i2 = RadicalismInput { suppression_part: 10.0, ..i.clone() };
    assert!((radicalism(&r, &i2) - 32.0).abs() < 1e-9);
    // independence: culture value and nationalism
    let i3 = RadicalismInput { independence: Some((10.0, 14.0)), ..i.clone() };
    assert!((radicalism(&r, &i3) - 46.0).abs() < 1e-9);
    // never below 0
    let low = RadicalismInput { reform_steps_passed: 40.0, ..Default::default() };
    assert_eq!(radicalism(&r, &low), 0.0);
}

#[test]
fn reform_steps_count_the_index_of_the_option_in_force() {
    let issues = "political_reforms = { slavery = { yes_slavery = {} no_slavery = {} } vote = { none = {} landed = {} wealth = {} } } social_reforms = { x = { a = {} b = {} } }";
    let groups = parse_issues(issues).unwrap();
    let steps = reform_steps_passed(&groups, &|g| match g {
        "slavery" => Some("no_slavery".into()),
        "vote" => Some("wealth".into()),
        "x" => Some("b".into()),
        _ => None,
    });
    assert_eq!(steps, 3.0, "1 + 2, social reforms do not count");
}

#[test]
fn strength_and_suppression() {
    let r = rules();
    assert!((movement_strength(&r, 30_000.0, 100_000.0) - 0.9).abs() < 1e-12);
    assert_eq!(movement_strength(&r, 5.0, 0.0), 0.0);
    // 3 % bureaucrats x 0.5 x 170 x (1 + 0.2 + 0) = 3.06
    assert!((suppression_gain(&r, 3000.0, 100_000.0, 0.2, 0.0) - 3.06).abs() < 1e-9);
    assert_eq!(add_suppression(&r, 99.0, 5.0), 100.0);
    assert_eq!(suppression_cost(&r, 19.0, 5000.0), 20.0);
    let s = suppress(&r, 50.0, 19.0, 5000.0).unwrap();
    assert_eq!(s.points, 30.0);
    assert_eq!(s.radicalism_added, 10.0);
    assert_eq!(s.member_consciousness, 1.0);
    assert!(suppress(&r, 19.5, 19.0, 5000.0).is_none());
}

#[test]
fn suppression_cost_with_a_population_factor() {
    let mut r = rules();
    r.population_suppression_factor = 1000.0;
    assert_eq!(suppression_cost(&r, 20.0, 5000.0), 100.0);
    assert_eq!(suppression_cost(&r, 20.0, 10.0), 21.0);
}

#[test]
fn movement_day_and_turning_rebel() {
    let r = rules();
    assert_eq!(movement_day(&r, 99.9, false), MovementDay::Keep);
    assert_eq!(movement_day(&r, 99.9, true), MovementDay::Dissolve);
    assert_eq!(movement_day(&r, 100.0, false), MovementDay::TurnRebel);
    assert_eq!(movement_day(&r, 120.0, true), MovementDay::TurnRebel);
    assert_eq!(turn_rebel_militancy(&r, 2.0), 8.0);
    assert_eq!(turn_rebel_militancy(&r, 9.5), 9.5);
}

const SAVE: &str = r#"date="1850.1.1"
rebel_faction=
{
	id={ id=5 type=38 }
	type="pan_nationalist_rebels"
	name="German Pan-Nationalists"
	country="DEN"
	independence="---"
	culture="north_german"
	religion="protestant"
	government="hms_government"
	province=10
	leader={ id=9 type=37 }
	organization=0.25
	pop={ id=100 type=46 }
	pop={ id=101 type=46 }
}
DEN=
{
	primary_culture="north_german"
	suppression=12.5
	vote_franschise="landed_voting"
	movement={ issue="landed_voting" support=3000 cache=19.0 }
	movement={ tag="GER" support=500 cache=30.0 }
}
10=
{
	name="Kiel"
	owner="DEN"
	controller="DEN"
	core="GER"
	nationalism=7.5
	farmers={ id=100 size=500 north_german=protestant mil=9.0 faction={ id=5 type=38 } movement_tag="GER" }
	farmers={ id=101 size=400 north_german=protestant mil=9.0 faction={ id=5 type=38 } }
	farmers={ id=102 size=3000 north_german=protestant mil=1.0 movement_issue="landed_voting" }
}
"#;

#[test]
fn state_loads_from_a_save() {
    let pop_types: HashSet<String> = ["farmers".to_string()].into();
    let cults: HashSet<String> = ["north_german".to_string()].into();
    let save = wiktoria_data::save::parse_save(SAVE.as_bytes(), &pop_types, &cults).unwrap();
    let cultures = parse_cultures(CULTURES).unwrap();
    let issues = parse_issues("political_reforms = { vote_franschise = { none_voting = {} landed_voting = {} } }").unwrap();
    let s = UnrestState::from_save(&save, &cultures, &issues);
    assert_eq!(s.factions.len(), 1);
    let f = &s.factions[0];
    assert_eq!((f.kind.as_str(), f.country.as_str(), f.culture_group.as_str()), ("pan_nationalist_rebels", "DEN", "german"));
    assert_eq!(f.creates, None);
    assert_eq!(f.pops, vec![100, 101]);
    assert_eq!(f.leader, Some(9));
    assert!((f.organization - 0.25).abs() < 1e-12);
    assert_eq!(s.movements.len(), 2);
    assert_eq!(s.movements_of("DEN").count(), 2);
    assert_eq!(s.recount_support("DEN", MovementKind::Issue, "landed_voting"), 3000.0);
    assert_eq!(s.recount_support("DEN", MovementKind::Independence, "GER"), 500.0);
    assert_eq!(s.pop_faction.len(), 2);
    assert_eq!(s.suppression["DEN"], 12.5);
    assert_eq!(s.reform_steps["DEN"], 1.0);
    assert_eq!(s.nationalism_for("DEN", "GER"), 7.5);
    assert_eq!(s.nationalism_for("DEN", "FRA"), 0.0);
    assert_eq!(s.population["DEN"], 3900.0);
}
