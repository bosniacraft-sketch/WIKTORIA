//! Test scenarios (wiktoria_simulation::scenario, docs/SCENARIOS.md): the file format, the setup commands and
//! the checks, on the small synthetic world of game.rs (a line of provinces 1 - 2 - 3 - 4 - 5 and
//! a side road 3 - 6; BBB owns 1 and 2, AAA 3 to 7). Defines and unit types are made up here,
//! rules file literals come from the repo's engine_rules.

use std::collections::BTreeMap;

use wiktoria_data::date::Date;
use wiktoria_data::defines::Defines;
use wiktoria_simulation::calendar::Calendar;
use wiktoria_simulation::game::{Core, GameNumbers};
use wiktoria_simulation::military::ai::AiRules;
use wiktoria_simulation::military::battle::BattleRules;
use wiktoria_simulation::military::day::{DayParts, MilitaryDay};
use wiktoria_simulation::military::logistics::LogisticsRules;
use wiktoria_simulation::military::movement::{MoveMap, MovementRules};
use wiktoria_simulation::military::siege::{ProvinceMilitary, SiegeRules};
use wiktoria_simulation::military::units::{Army, Regiment, UnitDef};
use wiktoria_simulation::military::war::{War, WarEvent, WarEventKind, WarRules};
use wiktoria_simulation::military::{MilitaryCountry, MilitaryState};
use wiktoria_simulation::scenario::{self, Ctx, Place, Probe, Run, Scenario, Step, Want, When};
use wiktoria_simulation::script::{CountryState, EffectRules, PopState, ProvinceState, State};

const DEFINES: &str = r#"
defines = {
	country = {
		BADBOY_LIMIT = 25,
		CREATED_CB_VALID_TIME = 12,
		MIN_MOBILIZE_LIMIT = 3,
		MOBILIZATION_SPEED_BASE = 0.12,
		MOBILIZATION_SPEED_RAILS_MULT = 3.0,
	},
	military = {
		COMBATLOSS_WAR_EXHAUSTION = 3,
		SIEGE_ATTRITION = 2,
		SUPPLY_RANGE = 250,
		REINFORCE_SPEED = 0.2,
		POP_MIN_SIZE_FOR_REGIMENT = 1000,
		POP_SIZE_PER_REGIMENT = 3000,
		POP_MIN_SIZE_FOR_REGIMENT_PROTECTORATE_MULTIPLIER = 8,
		POP_MIN_SIZE_FOR_REGIMENT_COLONY_MULTIPLIER = 5,
		POP_MIN_SIZE_FOR_REGIMENT_NONCORE_MULTIPLIER = 3,
		LEADER_RECRUIT_COST = 20,
		LEADER_AGE_DEATH_FACTOR = 4,
		LEADER_MAX_RANDOM_PRESTIGE = 0.05,
		LEADER_PRESTIGE_TO_MORALE_FACTOR = 0.8,
		LEADER_PRESTIGE_TO_MAX_ORG_FACTOR = 0.7,
		LAND_SPEED_MODIFIER = 1,
		NAVAL_SPEED_MODIFIER = 2,
		ENGINEER_UNIT_RATIO = 0.1,
		RECON_UNIT_RATIO = 0.1,
		RECON_SIEGE_EFFECT = 0.5,
		SIEGE_BRIGADES_MIN = 3,
		SIEGE_BRIGADES_MAX = 13,
		SIEGE_BRIGADES_BONUS = 0.5,
	},
	diplomacy = {
		PEACE_COST_ANNEX = 85,
		PEACE_COST_DEMAND_STATE = 10,
		PEACE_COST_STATUS_QUO = 1,
		INFAMY_ANNEX = 10,
		INFAMY_DEMAND_STATE = 5,
		PRESTIGE_ANNEX_BASE = 2,
		PRESTIGE_ANNEX = 0.02,
		BREAKTRUCE_INFAMY_ANNEX = 1,
		BREAKTRUCE_PRESTIGE_ANNEX = -20,
		BREAKTRUCE_MILITANCY_ANNEX = 2,
		WAR_FAILED_GOAL_MILITANCY = 2,
		WAR_FAILED_GOAL_PRESTIGE_BASE = -10,
		WAR_FAILED_GOAL_PRESTIGE = -0.1,
		MAX_WARSCORE_FROM_BATTLES = 50,
		WARGOAL_JINGOISM_REQUIREMENT = 0.07,
		GW_WARGOAL_JINGOISM_REQUIREMENT_MOD = 0.25,
		BASE_TRUCE_MONTHS = 60,
		GOOD_PEACE_REFUSAL_MILITANCY = 1.0,
		GOOD_PEACE_REFUSAL_WAREXH = 5.0,
		CB_GENERATION_BASE_SPEED = 0.5,
		CB_DETECTION_CHANCE_BASE = 15,
		ON_CB_DETECTED_RELATION_CHANGE = -50,
		CB_GENERATION_SPEED_BONUS_ON_COLONY_COMPETITION = 1.0,
		CB_GENERATION_SPEED_BONUS_ON_COLONY_COMPETITION_TROOPS_PRESENCE = 1.0,
		GW_JUSTIFY_CB_BADBOY_IMPACT = 0.33,
		GW_CB_CONSTRUCTION_SPEED = 0.25,
		GW_WARSCORE_COST_MOD = 0.65,
		GW_WARSCORE_COST_MOD_2 = 0.3,
		TWS_FULFILLED_SPEED = 0.1,
		TWS_NOT_FULFILLED_SPEED = 0.1,
		TWS_GRACE_PERIOD_DAYS = 730,
		TWS_CB_LIMIT_DEFAULT = 100,
		TWS_FULFILLED_IDLE_SPACE = 0.75,
		TWS_BATTLE_MIN_COUNT = 5,
		TWS_BATTLE_MAX_ASPECT = 8.0,
	},
	pops = {
		MIL_TO_AUTORISE = 1,
		PEACE_BASE_RELUCTANCE = 20,
		PEACE_TIME_MONTHS = 30,
		PEACE_TIME_FACTOR = 0.6,
		PEACE_TIME_FACTOR_NO_GOALS = 2.0,
		PEACE_WAR_EXHAUSTION_FACTOR = 0.5,
		PEACE_WAR_DIRECTION_FACTOR = 1.0,
		PEACE_WAR_DIRECTION_WINNING_MULT = 5.0,
		PEACE_FORCE_BALANCE_FACTOR = 0.3,
		PEACE_ALLY_BASE_RELUCTANCE_MULT = 2.0,
		PEACE_ALLY_TIME_MULT = 0,
		PEACE_ALLY_WAR_EXHAUSTION_MULT = 1.0,
		PEACE_ALLY_WAR_DIRECTION_MULT = 0,
		PEACE_ALLY_FORCE_BALANCE_MULT = 0,
	},
}
"#;

fn engine_rules() -> wiktoria_data::engine_rules::EngineRules {
    wiktoria_data::config::load_engine_rules(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap()
}

fn infantry() -> UnitDef {
    UnitDef { name: "infantry".into(), land: true, kind: "infantry".into(), max_strength: 3.0, organisation: 30.0, speed: 4.0, attack: 4.0, defence: 4.0, discipline: 1.0, maneuver: 1.0, active: true, ..UnitDef::default() }
}

fn frigate() -> UnitDef {
    UnitDef { name: "frigate".into(), land: false, kind: "big_ship".into(), max_strength: 100.0, organisation: 30.0, speed: 10.0, active: true, ..UnitDef::default() }
}

fn parts() -> DayParts {
    let engine_rules = engine_rules();
    let defines = Defines::parse(DEFINES).unwrap();
    let mut adjacency = vec![std::collections::BTreeSet::new(); 8];
    for (a, b) in [(1usize, 2usize), (2, 3), (3, 4), (4, 5), (3, 6)] {
        adjacency[a].insert(b as u16);
        adjacency[b].insert(a as u16);
    }
    DayParts {
        logistics: LogisticsRules::new(&engine_rules, &defines).unwrap(),
        movement: MovementRules::new(&defines, &engine_rules).unwrap(),
        battle: BattleRules::from_engine_rules(&engine_rules).unwrap(),
        siege: SiegeRules::from_engine_rules(&engine_rules, &defines).unwrap(),
        war: WarRules::from_engine_rules(&defines, &engine_rules).unwrap(),
        calendar: Calendar::from_engine_rules(&engine_rules).unwrap(),
        map: MoveMap::from_adjacency(&adjacency, &[], &|_, _| 4.0),
        units: BTreeMap::from([("infantry".to_string(), infantry()), ("frigate".to_string(), frigate())]),
        cb_types: BTreeMap::new(),
        rebel_tag: "REB".into(),
        men_per_strength: 1000.0,
        supply_status: 1.0,
        dig_in_days: 5.0,
        soldier_type: "soldiers".into(),
        leadership: BTreeMap::new(),
        terrain: BTreeMap::new(),
        techs: BTreeMap::new(),
        infrastructure: None,
        content: Default::default(),
        supply_land: 0.0,
        supply_infrastructure: 0.0,
        seed: 7,
    }
}

fn regs(first_id: u64, n: usize) -> Vec<Regiment> {
    (0..n).map(|i| Regiment { id: first_id + i as u64, name: String::new(), kind: "infantry".into(), pop: Some(first_id * 10 + i as u64), pop_type: Some("soldiers".into()), organisation: 30.0, strength: 3.0, experience: 0.0, name_count: 0 }).collect()
}

/// The world of game.rs with one war (BBB attacks AAA), an AAA army of 4 regiments in 5, a BBB
/// army of 2 in 1, the two countries in the script state (AAA's capital is 4) and soldier and
/// farmer pops.
fn world() -> (Core, State) {
    let engine_rules = engine_rules();
    let defines = Defines::parse(DEFINES).unwrap();
    let d = Date::new(1840, 1, 2);
    let mut mil = MilitaryState {
        date: d,
        countries: BTreeMap::from([("AAA".to_string(), MilitaryCountry::default()), ("BBB".to_string(), MilitaryCountry::default())]),
        leaders: vec![],
        armies: vec![
            Army { id: 1, name: "old BBB army".into(), owner: "BBB".into(), location: 1, regiments: regs(100, 2), supplies: 1.0, ..Army::default() },
            Army { id: 2, name: "old AAA army".into(), owner: "AAA".into(), location: 5, regiments: regs(200, 4), supplies: 1.0, ..Army::default() },
        ],
        navies: vec![],
        constructions: vec![],
        wars: vec![War {
            name: "Old War".into(),
            active: true,
            attackers: vec!["BBB".into()],
            defenders: vec!["AAA".into()],
            original_attacker: Some("BBB".into()),
            original_defender: Some("AAA".into()),
            history: vec![WarEvent { date: d, kind: WarEventKind::AddAttacker("BBB".into()) }, WarEvent { date: d, kind: WarEventKind::AddDefender("AAA".into()) }],
            ..War::default()
        }],
        combats: vec![],
        provinces: BTreeMap::new(),
        warnings: vec![],
    };
    let mut state = State::new(d);
    for p in 1u16..=7 {
        let owner = if p <= 2 { "BBB" } else { "AAA" };
        mil.provinces.insert(p, ProvinceMilitary { owner: Some(owner.into()), controller: Some(owner.into()), garrison: 100.0, ..ProvinceMilitary::default() });
        let pops = vec![
            PopState { pop_type: "farmers".into(), id: 9000 + p as u64, size: 10_000.0, money: 50.0, militancy: 1.0, ..PopState::default() },
            PopState { pop_type: "capitalists".into(), id: 9100 + p as u64, size: 1_000.0, money: 10.0, militancy: 3.0, ..PopState::default() },
        ];
        state.provinces.insert(p, ProvinceState { owner: Some(owner.into()), controller: Some(owner.into()), cores: vec![owner.into()], pops, ..ProvinceState::default() });
    }
    state.countries.insert("AAA".into(), CountryState { capital: Some(4), money: 1000.0, prestige: 10.0, ..CountryState::default() });
    state.countries.insert("BBB".into(), CountryState { capital: Some(1), money: 2000.0, prestige: 20.0, ..CountryState::default() });
    state.prices.insert("grain".into(), 2.2);
    let core = Core::new(mil, MilitaryDay::from_parts(parts()), AiRules::new(&engine_rules, &defines).unwrap(), GameNumbers::from_engine_rules(&engine_rules).unwrap(), 11);
    (core, state)
}

fn setup(sc: &Scenario, core: &mut Core, state: &mut State) -> Result<Vec<String>, String> {
    let m = engine_rules();
    let (cal, effects) = (Calendar::from_engine_rules(&m).unwrap(), EffectRules::from_engine_rules(&m).unwrap());
    scenario::apply_setup(sc, &mut Ctx { state, core, cal: &cal, effects: &effects, politics: None })
}

/// Runs the core `days` days with the scenario's checks, as `scenario::run` does for a whole game.
fn run(sc: &Scenario, core: &mut Core, state: &mut State, days: u32) -> Run {
    let cal = core.day.calendar.clone();
    let probe = Probe { pop_money_scale: EffectRules::from_engine_rules(&engine_rules()).unwrap().pop_money_scale, ..Probe::default() };
    let mut r = Run::begin(sc, state, core, &probe);
    for _ in 0..days {
        let log = core.day(state);
        state.date = cal.to_date(cal.next_day(cal.to_game(state.date).unwrap()));
        r.day(sc, &log, state, core, &probe);
    }
    r.finish(sc, state, core, &probe);
    r
}

const SIEGE: &str = r#"
# A made-up scenario on the synthetic world.
name = "Siege test"
description = "BBB besieges AAA's capital with a big stack."
what_to_look_for = "The siege starts at once and the province changes hands."
base_save = "start/1836.v2"
seat = BBB
days = 200
speed = 4
known_bug = "none, a plain test"
setup = {
    end_wars = all
    clear_armies = AAA
    war = { attacker = BBB defender = AAA goal = conquest }
    spawn_army = { owner = BBB at = capital:AAA regiments = 12 unit = infantry name = "Big stack" }
    fort = { province = capital:AAA level = 1 }
    AAA = { treasury = 500 prestige = 5 }
    3 = { any_pop = { militancy = 2 } }
    pop_money = { tag = AAA pop_type = capitalists money = 250 }
    set_money = { tag = BBB money = 12345 }
    budget = { tag = BBB rich = 1.0 poor = 0.9 tariff = -3 }
}
check = { name = "a siege began" what = sieges_started min = 1 day = 3 }
check = { name = "war is on" what = war a = AAA b = BBB is = yes day = 1 }
check = { name = "capital falls" what = controller province = capital:AAA is = BBB day = 60 }
check = { name = "stack still there" what = regiments tag = BBB province = 4 min = 6 }
check = { name = "attrition or losses" what = men tag = BBB change_max = 0 }
check = { name = "AAA treasury" what = money tag = AAA min = 1500 max = 1500 day = 1 }
check = { name = "rich capitalists" what = pop_money tag = AAA pop_type = capitalists min = 1250 max = 1250 day = 1 }
check = { name = "angry 3" what = militancy province = 3 pop_type = farmers min = 3 max = 3 day = 1 }
check = { name = "not AAA's" what = controller province = 4 not = AAA day = 60 }
check = { name = "a bug shows" what = price good = grain min = 100 known_bug = "made up for the test" }
"#;

#[test]
fn a_scenario_file_is_read_with_its_setup_and_checks() {
    let sc = scenario::parse(SIEGE).unwrap();
    assert_eq!(sc.name, "Siege test");
    assert_eq!((sc.seat.as_deref(), sc.days, sc.speed, sc.base_save.as_deref()), (Some("BBB"), 200, Some(4), Some("start/1836.v2")));
    assert_eq!(sc.known_bugs, vec!["none, a plain test".to_string()]);
    assert_eq!(sc.setup.len(), 10);
    assert_eq!(sc.setup[0], Step::EndWars(None));
    assert_eq!(sc.setup[3], Step::SpawnArmy { owner: "BBB".into(), at: Place::Capital("AAA".into()), regiments: 12, unit: "infantry".into(), name: "Big stack".into() });
    assert!(matches!(&sc.setup[5], Step::CountryEffects { tag, .. } if tag == "AAA"));
    assert!(matches!(&sc.setup[6], Step::ProvinceEffects { at: Place::Id(3), .. }));
    assert_eq!(sc.checks.len(), 10);
    assert_eq!(sc.checks[0].when, When::Day(3));
    assert_eq!(sc.checks[0].want, Want::Range { min: Some(1.0), max: None });
    assert_eq!(sc.checks[2].when, When::Day(60));
    assert_eq!(sc.checks[3].when, When::End);
    assert_eq!(sc.checks[2].want, Want::Is("BBB".into()));
    assert_eq!(sc.checks[4].want, Want::Change { min: None, max: Some(0.0) });
    assert_eq!(sc.checks[9].known_bug.as_deref(), Some("made up for the test"));
}

#[test]
fn mistakes_in_a_file_are_named_with_their_line() {
    let bad = |text: &str| scenario::parse(&format!("name = x\n{text}")).unwrap_err();
    assert!(bad("setup = { teleport = { } }").contains("'teleport' is no setup command"));
    assert!(bad("check = { what = happiness tag = AAA min = 1 }").contains("unknown quantity 'happiness'"));
    assert!(bad("check = { what = money tag = AAA }").contains("one of: is, not"));
    assert!(bad("check = { what = money tag = AAA min = 1 change_min = 2 }").contains("one of: is, not"));
    assert!(bad("check = { what = money tag = AAA min = 1 date = 1840.13.1 }").contains("not Y.M.D"));
    assert!(bad("setup = {\n spawn_army = { owner = AAA at = 3 unit = infantry } }").starts_with("line 3: spawn_army needs regiments"));
    assert!(bad("setup = { spawn_army = { owner = AAA at = nowhere regiments = 2 unit = infantry } }").contains("not a province"));
    assert!(bad("setup = { budget = { tag = AAA gold = 1 } }").contains("unknown slider gold"));
    assert!(bad("weather = rain").contains("unknown entry 'weather'"));
    assert!(scenario::parse("days = 5").unwrap_err().contains("needs a name"));
    assert_eq!(scenario::parse("name = x").unwrap().days, scenario::DEFAULT_DAYS);
}

#[test]
fn the_setup_places_armies_starts_wars_and_sets_values() {
    let sc = scenario::parse(SIEGE).unwrap();
    let (mut core, mut state) = world();
    let lines = setup(&sc, &mut core, &mut state).unwrap();
    assert!(lines.iter().all(|l| !l.starts_with("warning")), "{lines:?}");
    // The old war ended, the new one runs with its goal.
    assert_eq!(core.mil.wars.len(), 1);
    let w = &core.mil.wars[0];
    assert!(w.active && w.attackers == ["BBB"] && w.defenders == ["AAA"]);
    assert_eq!(w.goals[0].casus_belli, "conquest");
    assert_eq!(state.wars.len(), 1, "the script state knows the war");
    // AAA's armies are gone; the new stack stands in AAA's capital at full strength.
    assert_eq!(core.mil.armies_of("AAA").count(), 0);
    let stack = core.mil.armies.iter().find(|a| a.name == "Big stack").unwrap();
    assert_eq!((stack.location, stack.regiments.len(), stack.regiments[0].strength, stack.regiments[0].organisation), (4, 12, 3.0, 30.0));
    let mut ids: Vec<u64> = core.mil.armies.iter().flat_map(|a| std::iter::once(a.id).chain(a.regiments.iter().map(|r| r.id))).collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n, "new ids do not clash");
    assert_eq!(core.mil.provinces[&4].fort, 1.0);
    // Effects: AAA's treasury and prestige added to; every pop of province 3 one point angrier.
    assert_eq!((state.countries["AAA"].money, state.countries["AAA"].prestige), (1500.0, 15.0));
    assert!(state.provinces[&3].pops.iter().all(|q| q.militancy == if q.pop_type == "farmers" { 3.0 } else { 5.0 }));
    // pop_money sets money units (times the stored scale) on AAA's capitalists only.
    let scale = EffectRules::from_engine_rules(&engine_rules()).unwrap().pop_money_scale;
    assert!(state.provinces.values().filter(|p| p.owner.as_deref() == Some("AAA")).flat_map(|p| &p.pops).filter(|q| q.pop_type == "capitalists").all(|q| q.money == 250.0 * scale));
    assert_eq!(state.provinces[&1].pops[1].money, 10.0);
    assert_eq!(state.countries["BBB"].money, 12345.0);
    let sp = &state.countries["BBB"].spending;
    assert_eq!((sp["rich_tax"], sp["poor_tax"], sp["tariffs"]), (1.0, 0.9, -1.0), "sliders kept inside their range");
    assert!(state.countries["BBB"].human, "the seat is played");
    assert!(!state.countries["AAA"].human);
}

#[test]
fn a_big_stack_takes_the_capital_and_the_checks_say_so() {
    let sc = scenario::parse(SIEGE).unwrap();
    let (mut core, mut state) = world();
    setup(&sc, &mut core, &mut state).unwrap();
    let r = run(&sc, &mut core, &mut state, sc.days);
    let by = |label: &str| r.verdicts.iter().find(|v| v.label == label).unwrap_or_else(|| panic!("no verdict {label}"));
    for v in &r.verdicts {
        println!("{}", v.line());
    }
    assert_eq!(r.verdicts.len(), sc.checks.len());
    for label in ["a siege began", "war is on", "capital falls", "stack still there", "attrition or losses", "AAA treasury", "rich capitalists", "angry 3", "not AAA's"] {
        assert!(by(label).pass, "{}", by(label).line());
    }
    assert_eq!(by("a siege began").date, Date::new(1840, 1, 5), "evaluated on day 3");
    assert_eq!(by("capital falls").actual, "BBB");
    let bug = by("a bug shows");
    assert!(!bug.pass && bug.word() == "KNOWN" && bug.line().contains("known bug: made up"));
    assert_eq!((r.failed(false), r.failed(true)), (0, 1), "a known bug fails only when strict");
    assert!(r.tally.sieges_started >= 1);
}

#[test]
fn checks_fail_with_the_value_they_measured() {
    let sc = scenario::parse(
        r#"name = "fails"
setup = { clear_armies = AAA }
check = { name = c1 what = controller province = 4 is = BBB }
check = { name = c2 what = regiments tag = AAA max = -1 }
check = { name = c3 what = money tag = ZZZ min = 0 }
check = { name = c4 what = controller province = 3 not = AAA }
check = { name = c5 what = owner province = 3 is = capital:AAA }
check = { name = c6 what = armies tag = AAA is = 0 }
check = { name = c7 what = ships tag = BBB is = 0 }
check = { name = c8 what = provinces tag = AAA min = 5 max = 5 }
check = { name = c9 what = population province = 4 min = 11000 max = 11000 }
check = { name = c10 what = wars_started tag = BBB max = 0 }
"#,
    )
    .unwrap();
    let (mut core, mut state) = world();
    setup(&sc, &mut core, &mut state).unwrap();
    let r = run(&sc, &mut core, &mut state, 1);
    let v: BTreeMap<&str, (bool, &str)> = r.verdicts.iter().map(|v| (v.label.as_str(), (v.pass, v.actual.as_str()))).collect();
    assert_eq!(v["c1"], (false, "AAA"));
    assert_eq!(v["c2"], (false, "0"));
    assert!(!v["c3"].0 && v["c3"].1.contains("no country ZZZ"));
    assert_eq!(v["c4"], (false, "AAA"));
    assert!(!v["c5"].0, "a tag is not a province");
    assert_eq!(v["c6"], (true, "0"));
    assert_eq!(v["c7"], (true, "0"));
    assert_eq!(v["c8"], (true, "5"));
    assert_eq!(v["c9"], (true, "11000"));
    assert_eq!(v["c10"], (true, "0"), "the war of the save is not counted");
    assert_eq!(r.failed(false), 5);
}

#[test]
fn setup_errors_name_the_step() {
    let try_setup = |text: &str| {
        let sc = scenario::parse(&format!("name = x\nsetup = {{ {text} }}")).unwrap();
        let (mut core, mut state) = world();
        setup(&sc, &mut core, &mut state)
    };
    assert!(try_setup("spawn_army = { owner = AAA at = 3 regiments = 2 unit = cavalry }").unwrap_err().contains("'cavalry' is no land unit type (have: infantry)"));
    assert!(try_setup("spawn_army = { owner = AAA at = 3 regiments = 2 unit = frigate }").unwrap_err().contains("no land unit"));
    assert!(try_setup("spawn_navy = { owner = AAA at = 3 ships = 2 unit = infantry }").unwrap_err().contains("no ship type"));
    assert!(try_setup("spawn_army = { owner = ZZZ at = 3 regiments = 2 unit = infantry }").unwrap_err().starts_with("setup step 1: no country ZZZ"));
    assert!(try_setup("spawn_army = { owner = AAA at = 99 regiments = 2 unit = infantry }").unwrap_err().contains("province 99 is not in the save"));
    assert!(try_setup("spawn_army = { owner = AAA at = capital:ZZZ regiments = 2 unit = infantry }").unwrap_err().contains("no country ZZZ"));
    assert!(try_setup("move_army = { army = ghost to = 3 }").unwrap_err().contains("no army 'ghost'"));
    assert!(try_setup("play = REB").is_err(), "rebels are not played");
    // An effect the engine does not know is reported, not fatal.
    let lines = try_setup("AAA = { become_a_unicorn = yes }").unwrap();
    assert!(lines.iter().any(|l| l.starts_with("warning: effect not understood") && l.contains("become_a_unicorn")), "{lines:?}");
}

#[test]
fn controllers_owners_fleets_casus_belli_and_orders() {
    let sc = scenario::parse(
        r#"name = "misc"
setup = {
    controller = { of = AAA to = BBB }
    owner = { province = 7 to = BBB }
    end_wars = AAA
    spawn_navy = { owner = BBB at = 2 ships = 3 unit = frigate name = "Fleet" }
    spawn_army = { owner = BBB at = 1 regiments = 3 unit = infantry name = "Marchers" }
    move_army = { army = "Marchers" to = 2 }
    casus_belli = { actor = AAA target = BBB type = conquest }
    play = AAA
}
check = { name = arrived what = regiments tag = BBB province = 2 min = 3 }
check = { name = gone what = regiments tag = BBB province = 1 min = 2 max = 2 }
check = { name = fleet what = ships tag = BBB is = 3 }
check = { name = peace what = wars is = 0 }
check = { name = owner7 what = owner province = 7 is = BBB }
check = { name = occupied what = occupied tag = AAA is = 0 }
"#,
    )
    .unwrap();
    let (mut core, mut state) = world();
    let lines = setup(&sc, &mut core, &mut state).unwrap();
    // end_wars gave back every province the war's sides occupied (all of AAA's, by the controller step).
    assert!(lines.iter().any(|l| l.contains("ended 1 wars of AAA, 4 provinces back")), "{lines:?}");
    assert_eq!(state.provinces[&4].controller.as_deref(), Some("AAA"));
    assert_eq!(core.mil.provinces[&4].controller.as_deref(), Some("AAA"));
    assert_eq!((state.provinces[&7].owner.as_deref(), core.mil.provinces[&7].owner.as_deref(), core.mil.provinces[&7].controller.as_deref()), (Some("BBB"), Some("BBB"), Some("BBB")));
    let a = core.mil.armies.iter().find(|a| a.name == "Marchers").unwrap();
    assert_eq!(a.path, vec![2], "ordered to the next province (AAA's land is closed in peace)");
    assert_eq!(core.held_cbs[&("AAA".to_string(), "BBB".to_string())][0].cb_type, "conquest");
    assert!(state.countries["AAA"].human);
    // The orders are the scenario's: BBB's guard posts in peace would send the march elsewhere.
    core.ai.off.posts = true;
    let r = run(&sc, &mut core, &mut state, 30);
    for v in &r.verdicts {
        assert!(v.pass, "{}", v.line());
    }
}

#[test]
fn the_shipped_scenarios_parse() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios");
    let mut n = 0;
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_none_or(|x| x != "txt") || p.file_name().is_some_and(|f| f == "README.txt") {
            continue;
        }
        let sc = scenario::parse(&std::fs::read_to_string(&p).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        assert!(!sc.description.is_empty() && !sc.what_to_look_for.is_empty() && sc.base_save.is_some() && !sc.checks.is_empty(), "{}", p.display());
        n += 1;
    }
    assert!((8..=20).contains(&n), "8 to 20 scenarios, found {n}");
}


#[test]
fn an_observed_scenario_plays_nobody_and_watches_the_ai() {
    assert!(scenario::parse("name = x\nobserve = yes\nseat = AAA").unwrap_err().contains("plays no country"));
    let sc = scenario::parse(
        r#"name = "watch"
observe = yes
setup = { BBB = { prestige = 1 } }
check = { name = moved what = armies_moved min = 1 }
check = { name = changed what = wars_changed min = 1 }
check = { name = nan what = nan_countries is = 0 }
check = { name = gp what = lowest_gp_money min = 1500 }
"#,
    )
    .unwrap();
    assert!(sc.observe);
    let (mut core, mut state) = world();
    state.great_powers = vec!["AAA".into(), "BBB".into()];
    setup(&sc, &mut core, &mut state).unwrap();
    assert!(state.countries.values().all(|c| !c.human));
    // A fresh war with a goal, so the computer's armies march.
    core.mil.wars[0].goals.push(wiktoria_simulation::military::war::WarGoal { casus_belli: "conquest".into(), actor: "BBB".into(), receiver: "AAA".into(), ..Default::default() });
    let r = run(&sc, &mut core, &mut state, 60);
    let v: BTreeMap<&str, (bool, &str)> = r.verdicts.iter().map(|v| (v.label.as_str(), (v.pass, v.actual.as_str()))).collect();
    assert_eq!(v["nan"], (true, "0"));
    assert_eq!(v["gp"].1, "1000", "AAA's treasury is the lowest");
    assert!(!v["gp"].0);
    assert_eq!(v["moved"].0, r.verdicts.iter().find(|x| x.label == "moved").unwrap().actual != "0");
}
