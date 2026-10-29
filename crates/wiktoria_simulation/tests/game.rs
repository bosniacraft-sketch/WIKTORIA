//! The combined day (wiktoria_simulation::game::Core) on the small synthetic world of military_day.rs: one war, a
//! siege that completes and changes the controller, a battle that ends with a retreat. Defines and
//! unit types are made up here, rules file literals come from the repo's engine_rules.

use std::collections::BTreeMap;

use wiktoria_data::date::Date;
use wiktoria_data::defines::Defines;
use wiktoria_simulation::calendar::Calendar;
use wiktoria_simulation::military::battle::BattleRules;
use wiktoria_simulation::military::day::{DayParts, MilitaryDay};
use wiktoria_simulation::military::logistics::LogisticsRules;
use wiktoria_simulation::military::movement::{MoveMap, MovementRules};
use wiktoria_simulation::military::siege::{ProvinceMilitary, SiegeRules};
use wiktoria_simulation::military::units::{Army, Regiment, UnitDef};
use wiktoria_simulation::military::war::{War, WarEvent, WarEventKind, WarRules};
use wiktoria_simulation::military::{MilitaryCountry, MilitaryState};
use wiktoria_simulation::game::{Core, CoreLog, GameNumbers};
use wiktoria_simulation::military::ai::AiRules;
use wiktoria_simulation::script::{PopState, ProvinceState, State};

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

fn infantry() -> UnitDef {
    UnitDef { name: "infantry".into(), land: true, kind: "infantry".into(), max_strength: 3.0, organisation: 30.0, speed: 4.0, attack: 4.0, defence: 4.0, discipline: 1.0, maneuver: 1.0, active: true, ..UnitDef::default() }
}

fn parts() -> DayParts {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let engine_rules = wiktoria_data::config::load_engine_rules(&root).unwrap();
    let defines = Defines::parse(DEFINES).unwrap();
    // A line of provinces 1 - 2 - 3 - 4 - 5 and a side road 3 - 6. Each border is 4 long: one day for infantry.
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
        units: BTreeMap::from([("infantry".to_string(), infantry())]),
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

fn army(id: u64, owner: &str, at: u16, path: Vec<u16>, regiments: Vec<Regiment>) -> Army {
    Army { id, name: format!("army {id}"), owner: owner.into(), location: at, path, regiments, supplies: 1.0, ..Army::default() }
}

fn prov(owner: &str) -> ProvinceMilitary {
    ProvinceMilitary { owner: Some(owner.into()), controller: Some(owner.into()), garrison: 100.0, ..ProvinceMilitary::default() }
}

/// Provinces 1 and 2 are BBB's, 3 to 7 AAA's. BBB (attacker) fights AAA.
fn world(armies: Vec<Army>) -> (MilitaryState, State) {
    let d = Date::new(1840, 1, 2);
    let mut mil = MilitaryState {
        date: d,
        countries: BTreeMap::from([("AAA".to_string(), MilitaryCountry::default()), ("BBB".to_string(), MilitaryCountry::default())]),
        leaders: vec![],
        armies,
        navies: vec![],
        constructions: vec![],
        wars: vec![War {
            name: "Test War".into(),
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
        mil.provinces.insert(p, prov(owner));
        state.provinces.insert(p, ProvinceState { owner: Some(owner.into()), controller: Some(owner.into()), cores: vec![owner.into()], ..ProvinceState::default() });
    }
    (mil, state)
}

fn core(seed: u64) -> (Core, State) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let engine_rules = wiktoria_data::config::load_engine_rules(&root).unwrap();
    let defines = Defines::parse(DEFINES).unwrap();
    let (mil, mut state) = world(vec![army(1, "BBB", 1, vec![], regs(100, 6)), army(2, "AAA", 5, vec![], regs(200, 4))]);
    // Soldier pops behind the regiments, so casualties have somewhere to go.
    for (p, first) in [(1u16, 100u64), (5, 200)] {
        let n = if first == 100 { 6 } else { 4 };
        state.provinces.get_mut(&p).unwrap().pops = (0..n).map(|i| PopState { pop_type: "soldiers".into(), id: first * 10 + i, size: 3000.0, ..PopState::default() }).collect();
    }
    let c = Core::new(mil, MilitaryDay::from_parts(parts()), AiRules::new(&engine_rules, &defines).unwrap(), GameNumbers::from_engine_rules(&engine_rules).unwrap(), seed);
    (c, state)
}

fn run_core(c: &mut Core, state: &mut State, days: usize) -> Vec<CoreLog> {
    let cal = c.day.calendar.clone();
    (0..days)
        .map(|_| {
            let log = c.day(state);
            state.date = cal.to_date(cal.next_day(cal.to_game(state.date).unwrap()));
            log
        })
        .collect()
}

#[test]
fn same_seed_same_run_and_the_ai_fights() {
    let (mut c1, mut s1) = core(42);
    let (mut c2, mut s2) = core(42);
    let (l1, l2) = (run_core(&mut c1, &mut s1, 120), run_core(&mut c2, &mut s2, 120));
    assert_eq!(l1, l2);
    assert_eq!(c1.mil, c2.mil);
    assert_eq!(s1, s2);
    assert!(l1.iter().map(|l| l.orders).sum::<usize>() > 0, "the army AI gives orders at war");
    // Casualties came off the soldier pops, which never go negative.
    let lost: f64 = l1.iter().map(|l| l.men_lost).sum();
    let left: f64 = s1.provinces.values().flat_map(|p| &p.pops).map(|q| q.size).sum();
    assert!(left >= 0.0 && (lost == 0.0 || left < 30_000.0));
}

#[test]
fn occupation_reaches_the_economy_state() {
    let (mut c, mut s) = core(7);
    run_core(&mut c, &mut s, 200);
    for (id, p) in &c.mil.provinces {
        assert_eq!(p.controller, s.provinces[id].controller, "province {id}: the state follows the military controller");
    }
}

/// Made-up colonial numbers: upkeep 5 a colony province, a non-core overseas base counts x 0.3.
const COLONY_DEFINES: &str = "defines = {
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

#[test]
fn colonial_points_come_from_technologies_naval_bases_and_ships_within_supply() {
    use wiktoria_simulation::buildings::{BuildingDef, BuildingNumbers, BuildingRules};
    use wiktoria_simulation::colonies::{ColonyCountry, ColonyHolding, ColonyProvince, ColonyRules, ColonyState, RaceRecord, Status};
    use wiktoria_simulation::military::units::{Navy, Ship};
    let (mut c, mut s) = core(1);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let engine_rules = wiktoria_data::config::load_engine_rules(&root).unwrap();
    // AAA: home state (provinces 3 to 5, a level 2 base in 3), a colony overseas with a level 1
    // base (7, no core) and an inland state overseas (6, no core).
    for (id, p) in s.provinces.iter_mut() {
        (p.region, p.continent) = match *id {
            7 => (Some(2), Some("far".to_string())),
            6 => (Some(3), Some("far".to_string())),
            _ => (Some(1), Some("home".to_string())),
        };
        if *id >= 6 {
            p.cores = vec![];
        }
    }
    s.provinces.get_mut(&7).unwrap().colonial = 2;
    s.countries.insert("AAA".into(), wiktoria_simulation::script::CountryState { capital: Some(3), technologies: ["sails".to_string()].into(), ..Default::default() });
    let rules = ColonyRules::from_defines(&Defines::parse(COLONY_DEFINES).unwrap(), &engine_rules).unwrap().with_technologies([("sails", 40.0), ("oars", 9.0)].into_iter());
    let base = |level| ColonyProvince { owner: Some("AAA".into()), naval_base: level, ..Default::default() };
    let colonies = ColonyState {
        date: s.date,
        countries: BTreeMap::from([("AAA".to_string(), ColonyCountry { great_power: true, races: BTreeMap::from([(9, 1.0)]) })]),
        provinces: BTreeMap::from([(3, base(2.0)), (7, ColonyProvince { status: Some(Status::Colony), ..base(1.0) })]),
        states: vec![ColonyHolding { owner: "AAA".into(), provinces: vec![7], status: Status::Colony, factories: 0 }],
        races: vec![RaceRecord { region: 9, contenders: vec![("AAA".into(), 1.0, 40.0, None)], ..Default::default() }],
    };
    c.attach_colonies(rules, colonies);
    c.buildings = Some(BuildingRules {
        defs: vec![BuildingDef { name: "harbour".into(), kind: "naval_base".into(), colonial_points: vec![7.0, 11.0], ..Default::default() }],
        n: BuildingNumbers { naval_supply_base: 10.0, naval_supply_empty: 2.0, naval_supply_non_core: 0.3, ..Default::default() },
    });
    c.day.units.insert("frigate".into(), UnitDef { name: "frigate".into(), colonial_points: 2.0, supply_consumption_score: 5.0, ..UnitDef::default() });
    c.day.units.insert("barge".into(), UnitDef { name: "barge".into(), supply_consumption_score: 8.0, ..UnitDef::default() });
    let ship = |kind: &str| Ship { kind: kind.into(), ..Ship::default() };
    c.mil.navies.push(Navy { owner: "AAA".into(), ships: vec![ship("frigate"), ship("frigate"), ship("frigate")], ..Navy::default() });

    let p = c.colonial_points_of(&s, "AAA", 1.0).unwrap();
    assert_eq!(p.techs, vec![("sails".to_string(), 40.0)], "only the technologies it has");
    assert_eq!(p.bases, vec![(3, 2, 11.0)], "a core base: the list entry of its level");
    assert_eq!(p.overseas, vec![(Some(2), 1.5)], "the state with a port overseas: the flat amount; the inland one nothing");
    assert_eq!(p.ships, vec![("frigate".to_string(), 3.0, 2.0)]);
    assert_eq!((p.supply_used, p.supply_limit, p.overuse), (15.0, 23.0, 0.0), "supply: 20 for the level 2 base, 10 x 0.3 overseas");
    assert!((p.total - (40.0 + 12.5 + 6.0)).abs() < 1e-9);
    assert_eq!((p.races, p.upkeep, p.used), (40.0, 5.0, 45.0), "points in the race and 5 for the colony province");
    assert!((p.free() - 13.5).abs() < 1e-9);

    // Transports give no points but use supply: at 1.5 x the limit the ships' points are halved.
    c.mil.navies[0].ships.extend([ship("barge"), ship("barge")]);
    c.day.units.get_mut("barge").unwrap().supply_consumption_score = 9.75;
    let p = c.colonial_points_of(&s, "AAA", 1.0).unwrap();
    assert_eq!((p.supply_used, p.overuse), (34.5, 0.5));
    assert!((p.total - (40.0 + 12.5 + 3.0)).abs() < 1e-9);
    let p = c.colonial_points_of(&s, "AAA", 17.0).unwrap();
    assert_eq!((p.total, p.used), (0.0, 0.0), "below the colonial rank: nothing");
}

/// D20: a played country gets no army orders from the computer, its own orders are carried out
/// from the queue, and once handed back the computer plans for it again.
#[test]
fn a_played_country_is_left_to_its_player() {
    use wiktoria_simulation::player::{self, Order, Players, Slider};
    let with_countries = |seed| {
        let (mut c, mut s) = core(seed);
        // The war has no goal: keep the foreign minister away, so it is not ended in a white peace.
        (c.numbers.foreign_minister_days, c.days) = (1_000_000, 1);
        for t in ["BBB", "AAA"] {
            s.countries.insert(t.into(), wiktoria_simulation::script::CountryState { civilized: true, ..Default::default() });
        }
        (c, s)
    };
    // The computer alone sends BBB's army somewhere.
    let (mut c, mut s) = with_countries(42);
    let moved = (0..30).any(|_| {
        run_core(&mut c, &mut s, 1);
        c.ai_targets.contains_key(&1) || c.mil.armies.iter().find(|a| a.id == 1).is_some_and(|a| !a.path.is_empty() || a.location != 1)
    });
    assert!(moved, "the AI orders BBB's army");
    // Played, it stands until its player orders it.
    let (mut c, mut s) = with_countries(42);
    player::set_player(&mut s, &mut c, "BBB", true).unwrap();
    assert_eq!(player::players(&s), ["BBB"]);
    run_core(&mut c, &mut s, 3);
    let army = |c: &Core| c.mil.armies.iter().find(|a| a.id == 1).cloned().unwrap();
    assert!(!c.ai_targets.contains_key(&1) && army(&c).path.is_empty() && army(&c).location == 1);
    let mut queue = Players::default();
    queue.push("BBB", Order::MoveArmy { army: 1, to: 3 });
    queue.push("AAA", Order::Budget { slider: Slider::PoorTax, value: 0.5 });
    queue.push("BBB", Order::Budget { slider: Slider::PoorTax, value: 0.5 });
    let (research, politics) = (Default::default(), Default::default());
    let out = player::apply(&mut queue, &mut s, &mut c, &research, &politics);
    assert!(out[0].result.is_ok() && out[1].result.is_err() && out[2].result.is_ok(), "{out:?}");
    assert!(queue.pending().is_empty());
    assert_eq!(army(&c).path, vec![2, 3]);
    assert_eq!(s.countries["BBB"].spending["poor_tax"], 0.5);
    assert!(!s.countries["AAA"].spending.contains_key("poor_tax"), "AAA is the computer's: no order of a person reaches it");
    // Back to the computer: its plans are made again from the state as it is.
    player::set_player(&mut s, &mut c, "BBB", false).unwrap();
    assert!(player::players(&s).is_empty());
    for a in c.mil.armies.iter_mut() {
        a.path.clear();
    }
    let planned = (0..30).any(|_| {
        run_core(&mut c, &mut s, 1);
        c.ai_targets.contains_key(&1) || c.mil.armies.iter().find(|a| a.id == 1).is_some_and(|a| !a.path.is_empty())
    });
    assert!(planned, "handed back, BBB's army gets orders from the computer again");
    assert!(player::set_player(&mut s, &mut c, "REB", true).is_err());
    assert!(player::set_player(&mut s, &mut c, "ZZZ", true).is_err());
}

/// Taken over, a country's armies stop the marches the computer ordered; the player's go on.
#[test]
fn a_taken_over_country_keeps_no_march_of_the_computer() {
    use wiktoria_simulation::player::{self, Order, Players};
    let (mut c, mut s) = core(42);
    (c.numbers.foreign_minister_days, c.days) = (1_000_000, 1);
    for t in ["BBB", "AAA"] {
        s.countries.insert(t.into(), wiktoria_simulation::script::CountryState { civilized: true, ..Default::default() });
    }
    let army = |c: &Core| c.mil.armies.iter().find(|a| a.id == 1).cloned().unwrap();
    let marching = (0..30).any(|_| {
        run_core(&mut c, &mut s, 1);
        !army(&c).path.is_empty()
    });
    assert!(marching, "the computer sends BBB's army off");
    c.ai_resting.insert(1);
    player::set_player(&mut s, &mut c, "BBB", true).unwrap();
    let a = army(&c);
    assert!(a.path.is_empty() && a.target.is_none(), "the computer's march stops: {:?}", a.path);
    assert!(!c.ai_targets.contains_key(&1) && !c.ai_resting.contains(&1) && !c.ai_moved.contains(&1));
    // The player's own order is kept when the country is taken again.
    let mut queue = Players::default();
    let to = if a.location == 3 { 1 } else { 3 };
    queue.push("BBB", Order::MoveArmy { army: 1, to });
    let out = player::apply(&mut queue, &mut s, &mut c, &Default::default(), &Default::default());
    assert!(out[0].result.is_ok(), "{out:?}");
    let path = army(&c).path;
    assert!(!path.is_empty());
    player::set_player(&mut s, &mut c, "BBB", true).unwrap();
    assert_eq!(army(&c).path, path);
}
