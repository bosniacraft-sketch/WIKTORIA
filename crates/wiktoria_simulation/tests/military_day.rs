//! The military day step on a small synthetic world: one war, a
//! siege that completes and changes the controller, a battle that ends with a retreat. Defines and
//! unit types are made up here, rules file literals come from the repo's engine_rules.

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_data::date::Date;
use wiktoria_data::defines::Defines;
use wiktoria_simulation::calendar::Calendar;
use wiktoria_simulation::military::battle::{BattleRules, Combat, CombatKind};
use wiktoria_simulation::military::day::{DayParts, MilitaryDay, Terrain};
use wiktoria_simulation::military::logistics::{LogisticsRules, SupplyAccess};
use wiktoria_simulation::military::movement::{self, Border, BorderKind, MoveMap, MoveProvince, MovementRules};
use wiktoria_simulation::military::siege::{ProvinceMilitary, SiegeRules};
use wiktoria_simulation::military::units::{Army, Navy, Regiment, Ship, UnitDef};
use wiktoria_simulation::military::war::{War, WarEvent, WarEventKind, WarRules};
use wiktoria_simulation::military::{MilitaryCountry, MilitaryState};
use wiktoria_simulation::script::{ProvinceState, State};

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

fn run(day: &mut MilitaryDay, mil: &mut MilitaryState, state: &State, days: usize, mut stop: impl FnMut(&MilitaryState) -> bool) -> usize {
    let mut g = day.calendar.to_game(mil.date).unwrap();
    for n in 0..days {
        let date = day.calendar.to_date(g);
        let rep = day.day(mil, state, date);
        assert!(rep.casualties.iter().all(|(_, m)| *m > 0.0));
        g = wiktoria_simulation::GameDate(g.0 + 24);
        if stop(mil) {
            return n + 1;
        }
    }
    days
}

#[test]
fn a_standing_army_besieges_takes_the_province_and_the_war_goes_on() {
    // Six brigades of BBB stand in province 3, which AAA owns and controls.
    let (mut mil, state) = world(vec![army(1, "BBB", 3, vec![], regs(100, 6))]);
    let mut day = MilitaryDay::from_parts(parts());
    let mut changes = Vec::new();
    let mut g = day.calendar.to_game(mil.date).unwrap();
    let mut started = false;
    for _ in 0..400 {
        let rep = day.day(&mut mil, &state, day.calendar.to_date(g));
        g = wiktoria_simulation::GameDate(g.0 + 24);
        started |= rep.sieges_started > 0;
        changes.extend(rep.controller_changes);
        if !changes.is_empty() {
            break;
        }
    }
    assert!(started, "a standing army in an enemy province starts a siege");
    assert_eq!(changes.len(), 1, "the siege finishes within 400 days");
    assert_eq!((changes[0].province, changes[0].to.as_deref(), changes[0].from.as_deref()), (3, Some("BBB"), Some("AAA")));
    let p = &mil.provinces[&3];
    assert!(p.is_occupied() && p.rgo_workers_leave());
    assert_eq!(p.garrison, 100.0, "garrison is back at full strength after the capture");
    // The war is still on and its score now counts an occupied province for BBB.
    assert!(mil.wars[0].active);
    let scores = day.war_scores(&mil, &state);
    assert_eq!(scores.len(), 1);
    assert!(scores[0].1 > 0.0 && scores[0].1 <= 100.0, "score {scores:?}");
}

#[test]
fn garrisons_never_leave_their_range_and_regrow_without_a_siege() {
    let (mut mil, state) = world(vec![army(1, "BBB", 3, vec![], regs(100, 6))]);
    mil.provinces.get_mut(&5).unwrap().garrison = 40.0;
    let mut day = MilitaryDay::from_parts(parts());
    run(&mut day, &mut mil, &state, 200, |m| m.provinces.values().any(|p| !(0.0..=100.0).contains(&p.garrison)));
    assert!(mil.provinces.values().all(|p| (0.0..=100.0).contains(&p.garrison)));
    assert_eq!(mil.provinces[&5].garrison, 100.0, "an unbesieged garrison regrows to 100");
}

#[test]
fn an_arriving_army_attacks_and_the_loser_retreats() {
    // BBB marches from 4 into province 3 where a smaller AAA army stands.
    let (mut mil, mut state) = world(vec![army(1, "BBB", 4, vec![3], regs(100, 6)), army(2, "AAA", 3, vec![], regs(200, 4))]);
    // Both countries know some tactics: a country with none takes strength damage undivided.
    let mut p = parts();
    p.techs.insert("drill".to_string(), BTreeMap::from([("military_tactics".to_string(), 1.0)]));
    for tag in ["AAA", "BBB"] {
        state.countries.entry(tag.to_string()).or_default().technologies.insert("drill".to_string());
    }
    let mut day = MilitaryDay::from_parts(p);
    let mut started = 0;
    let mut finished_on = None;
    let mut casualties = 0.0;
    let mut g = day.calendar.to_game(mil.date).unwrap();
    for n in 0..120 {
        let rep = day.day(&mut mil, &state, day.calendar.to_date(g));
        g = wiktoria_simulation::GameDate(g.0 + 24);
        started += rep.battles_started;
        casualties += rep.casualties.iter().map(|(_, m)| *m).sum::<f64>();
        if rep.battles_finished > 0 {
            finished_on = Some(n);
            assert_eq!(rep.armies_destroyed, 0, "the beaten army was not wiped out");
            assert!(rep.rebels_beaten.is_empty(), "no rebels in this battle");
            break;
        }
    }
    assert_eq!(started, 1, "one battle starts when the army arrives");
    assert!(finished_on.is_some(), "the battle ends within 120 days");
    assert!(casualties > 0.0, "the pops behind the regiments pay for it");
    assert!(mil.combats.iter().all(|c| c.kind != CombatKind::Land), "no land battle is left running");
    // The war keeps the result for its score.
    assert_eq!(mil.wars[0].battles().count(), 1);
    assert!(mil.wars[0].battles().next().unwrap().attacker_won, "six brigades beat four");
    // The beaten army is ordered to a neighbouring province and flagged to leave: it is still there with a path.
    let loser = mil.armies.iter().find(|a| a.id == 2).expect("the loser retreats instead of vanishing");
    assert_eq!(loser.path.len(), 1);
    assert_ne!(loser.path[0], 3);
    assert!(day.map.border(3, loser.path[0]).is_some(), "it retreats to a neighbour");
    // Next day it has left and does not fight again.
    let dest = loser.path[0];
    run(&mut day, &mut mil, &state, 3, |_| false);
    let loser = mil.armies.iter().find(|a| a.id == 2).unwrap();
    assert_eq!(loser.location, dest);
    assert!(loser.path.is_empty());
    assert_eq!(mil.combats.iter().filter(|c| c.kind == CombatKind::Land).count(), 0);
}

/// REDUCTION_AFTER_DEFEAT: a beaten rebel army names the pops behind its regiments, once each.
#[test]
fn a_beaten_rebel_army_names_the_pops_behind_it() {
    // AAA marches from 4 into province 3 of its own, where four rebel brigades stand.
    let (mut mil, mut state) = world(vec![army(1, "AAA", 4, vec![3], regs(100, 6)), army(2, "REB", 3, vec![], regs(200, 4))]);
    mil.countries.insert("REB".into(), MilitaryCountry::default());
    mil.provinces.get_mut(&3).unwrap().controller = Some("REB".into());
    let mut p = parts();
    p.techs.insert("drill".to_string(), BTreeMap::from([("military_tactics".to_string(), 1.0)]));
    state.countries.entry("AAA".to_string()).or_default().technologies.insert("drill".to_string());
    let mut day = MilitaryDay::from_parts(p);
    let mut g = day.calendar.to_game(mil.date).unwrap();
    let mut beaten = Vec::new();
    for _ in 0..120 {
        let rep = day.day(&mut mil, &state, day.calendar.to_date(g));
        g = wiktoria_simulation::GameDate(g.0 + 24);
        if rep.battles_finished > 0 {
            beaten = rep.rebels_beaten;
            break;
        }
    }
    assert_eq!(beaten, vec![2000, 2001, 2002, 2003], "the pops of the four rebel regiments");
}

/// The day feeds the supply limit from what it holds: terrain, the two static blocks, the railway of
/// the province and the technologies of the country. A sea province has none of the land shares.
#[test]
fn supply_limit_reads_terrain_static_blocks_railway_and_technology() {
    let mut p = parts();
    p.supply_land = 5.0;
    p.supply_infrastructure = 4.0;
    p.terrain = BTreeMap::from([(3, Terrain { supply_limit: 10.0, ..Terrain::default() }), (4, Terrain { supply_limit: 6.0, ..Terrain::default() })]);
    p.map.provinces.insert(3, MoveProvince { infrastructure: 0.16, ..MoveProvince::default() });
    p.map.provinces.insert(7, MoveProvince { sea: true, ..MoveProvince::default() });
    p.techs = BTreeMap::from([("steam".to_string(), BTreeMap::from([("supply_limit".to_string(), 0.25)]))]);
    let day = MilitaryDay::from_parts(p);
    let (_, mut state) = world(vec![]);
    assert_eq!(day.supply_limit(&state, "AAA", 4, SupplyAccess::OwnControlled), 30);
    assert_eq!(day.supply_limit(&state, "BBB", 4, SupplyAccess::Hostile), 12);
    assert_eq!(day.supply_limit(&state, "AAA", 3, SupplyAccess::OwnControlled), 41);
    assert_eq!(day.supply_limit(&state, "AAA", 7, SupplyAccess::Sea), 1);
    state.countries.entry("AAA".into()).or_default().technologies.insert("steam".into());
    assert_eq!(day.supply_limit(&state, "AAA", 4, SupplyAccess::OwnControlled), 37);
    assert_eq!(day.supply_limit(&state, "BBB", 4, SupplyAccess::Hostile), 12);
}

/// Whose supply limit a country gets in a province: its own, a friend's or a stranger's. Friends are
/// allies, countries that give it military access, the leader of its sphere, and anyone under siege.
#[test]
fn supply_access_knows_own_occupied_friends_and_sieges() {
    let mut p = parts();
    p.map.provinces.insert(7, MoveProvince { sea: true, ..MoveProvince::default() });
    let day = MilitaryDay::from_parts(p);
    let (mut mil, mut state) = world(vec![]);
    let access = |mil: &MilitaryState, state: &State, tag: &str, province: u16| day.supply_access(mil, state, &day.relations(mil, state), tag, province);
    assert_eq!(access(&mil, &state, "AAA", 3), SupplyAccess::OwnControlled);
    assert_eq!(access(&mil, &state, "BBB", 3), SupplyAccess::Hostile);
    assert_eq!(access(&mil, &state, "AAA", 7), SupplyAccess::Sea);
    assert_eq!(access(&mil, &state, "REB", 3), SupplyAccess::Friendly);
    mil.provinces.get_mut(&4).unwrap().controller = Some("BBB".into());
    assert_eq!(access(&mil, &state, "BBB", 4), SupplyAccess::Controlled);
    assert_eq!(access(&mil, &state, "AAA", 4), SupplyAccess::OwnedOccupied);
    // A third country is a stranger until one of the ties holds.
    assert_eq!(access(&mil, &state, "CCC", 3), SupplyAccess::Hostile);
    state.alliances.push(("AAA".into(), "CCC".into()));
    assert_eq!(access(&mil, &state, "CCC", 3), SupplyAccess::Friendly);
    assert_eq!(access(&mil, &state, "CCC", 1), SupplyAccess::Hostile);
    state.alliances.clear();
    state.countries.entry("CCC".into()).or_default().access.insert("AAA".into());
    assert_eq!(access(&mil, &state, "CCC", 3), SupplyAccess::Friendly);
    state.countries.get_mut("CCC").unwrap().access.clear();
    state.countries.entry("AAA".into()).or_default().sphere.insert("CCC".into());
    assert_eq!(access(&mil, &state, "CCC", 3), SupplyAccess::Friendly);
    state.countries.get_mut("AAA").unwrap().sphere.clear();
    assert_eq!(access(&mil, &state, "CCC", 3), SupplyAccess::Hostile);
    mil.combats.push(Combat::new(CombatKind::Siege, 3, vec![], vec![]));
    assert_eq!(access(&mil, &state, "CCC", 3), SupplyAccess::Friendly);
    assert_eq!(access(&mil, &state, "CCC", 5), SupplyAccess::Hostile);
}

#[test]
fn monthly_step_changes_exhaustion_and_never_runs_away() {
    let (mut mil, state) = world(vec![army(1, "BBB", 3, vec![], regs(100, 6))]);
    mil.countries.get_mut("AAA").unwrap().war_exhaustion = 1.0;
    let mut day = MilitaryDay::from_parts(parts());
    let mut monthly = 0;
    let mut g = day.calendar.to_game(mil.date).unwrap();
    for _ in 0..365 {
        let rep = day.day(&mut mil, &state, day.calendar.to_date(g));
        g = wiktoria_simulation::GameDate(g.0 + 24);
        monthly += usize::from(rep.monthly);
        for c in mil.countries.values() {
            assert!((0.0..=100.0).contains(&c.war_exhaustion), "exhaustion {}", c.war_exhaustion);
        }
    }
    assert_eq!(monthly, 12, "the monthly step runs on the 1st");
    assert!(mil.countries["AAA"].war_exhaustion > 1.0, "an occupied country at war grows more tired");
}

#[test]
fn same_seed_same_year() {
    let build = || {
        let (mil, state) = world(vec![army(1, "BBB", 4, vec![3], regs(100, 6)), army(2, "AAA", 3, vec![], regs(200, 4))]);
        (mil, state)
    };
    let (mut m1, s1) = build();
    let (mut m2, s2) = build();
    let (mut d1, mut d2) = (MilitaryDay::from_parts(parts()), MilitaryDay::from_parts(parts()));
    run(&mut d1, &mut m1, &s1, 200, |_| false);
    run(&mut d2, &mut m2, &s2, 200, |_| false);
    assert_eq!(m1, m2);
}

#[test]
fn leader_traits_are_summed_and_an_army_without_a_leader_gets_the_stand_in() {
    use wiktoria_simulation::military::day::BattleContent;
    use wiktoria_simulation::military::units::{parse_traits, Leader};
    // A made-up traits file: two groups of named traits.
    let traits = parse_traits(b"personality = { nobody = { } bold = { attack = 2 defence = 1 reconnaissance = 0.5 } }
background = { untrained = { attack = -2 defence = -2 } gunner = { attack = 1.5 } }").unwrap();
    assert_eq!(traits.len(), 4);
    let content = BattleContent { traits, no_leader: ["nobody".to_string(), "untrained".to_string()], gas_attack_modifier: 3.0, invention_gas: vec![(false, false), (true, false), (false, true)], ..BattleContent::default() };
    let l = Leader { id: 1, personality: "bold".into(), background: "gunner".into(), ..Leader::default() };
    let t = content.leader_traits(Some(&l));
    assert_eq!((t.attack, t.defence, t.reconnaissance), (3.5, 1.0, 0.5));
    let none = content.leader_traits(None);
    assert_eq!((none.attack, none.defence), (-2.0, -2.0));
    // A trait name the file does not have adds nothing.
    let odd = Leader { id: 2, personality: "unknown".into(), background: "gunner".into(), ..Leader::default() };
    assert_eq!(content.leader_traits(Some(&odd)).attack, 1.5);
    // Gas from the inventions a save numbers from 1; an unknown number gives nothing.
    assert_eq!(content.gas("AAA", &BTreeSet::from([1])), (false, false));
    assert_eq!(content.gas("AAA", &BTreeSet::from([2, 9])), (true, false));
    assert_eq!(content.gas("AAA", &BTreeSet::from([2, 3])), (true, true));
    let named = BattleContent { gas_override: BTreeSet::from(["AAA".to_string()]), ..content.clone() };
    assert_eq!((named.gas("AAA", &BTreeSet::new()), named.gas("BBB", &BTreeSet::new())), ((true, true), (false, false)));
}

#[test]
fn a_side_fights_with_its_best_leader_and_the_stand_in_when_it_has_none() {
    use wiktoria_simulation::military::battle::{BattleSide, Combat, CombatKind};
    use wiktoria_simulation::military::day::BattleContent;
    use wiktoria_simulation::military::units::{parse_traits, Leader};
    let (mut mil, state) = world(vec![army(1, "BBB", 3, vec![], regs(100, 2)), army(2, "BBB", 3, vec![], regs(110, 2)), army(3, "AAA", 3, vec![], regs(200, 2))]);
    mil.leaders = vec![Leader { id: 7, owner: "BBB".into(), personality: "shy".into(), background: "plain".into(), prestige: 0.2, ..Leader::default() }, Leader { id: 8, owner: "BBB".into(), personality: "bold".into(), background: "plain".into(), ..Leader::default() }];
    mil.armies[0].leader = Some(7);
    mil.armies[1].leader = Some(8);
    // Both came over the plain border from 4: no crossing penalty in the way.
    mil.armies[0].previous = Some(4);
    mil.armies[1].previous = Some(4);
    mil.combats = vec![Combat::new(CombatKind::Land, 3, vec![1, 2], vec![3])];
    let mut p = parts();
    p.content = BattleContent { traits: parse_traits(b"personality = { shy = { attack = -1 } bold = { attack = 3 } none = { } }
background = { plain = { } raw = { attack = -2 defence = -2 } }").unwrap(), no_leader: ["none".to_string(), "raw".to_string()], ..BattleContent::default() };
    let day = MilitaryDay::from_parts(p);
    assert_eq!(day.side_leader(&mil, &mil.combats[0], BattleSide::Attacker).map(|l| l.id), Some(8));
    assert!(day.side_leader(&mil, &mil.combats[0], BattleSide::Defender).is_none());
    let (_, _, ctx, rep) = day.land_round_alone(&mil, &state, 0, [5.0, 5.0]);
    assert_eq!((ctx.attacker.leader, ctx.attacker.leader_attack, ctx.defender.leader_defence), (Some(8), 3.0, -2.0));
    assert_eq!(rep.modifiers, [3.0, -2.0]);
}

#[test]
fn an_attacker_pays_for_the_border_it_came_over() {
    use wiktoria_data::geography::ExtraAdjacency;
    use wiktoria_simulation::military::battle::{crossing_penalty, Combat, Entry};
    let mut p = parts();
    // The road 2 - 3 crosses a river, 6 - 3 is a strait, 4 - 3 is plain.
    let mut adjacency = vec![std::collections::BTreeSet::new(); 8];
    for (a, b) in [(2usize, 3usize), (3, 4)] {
        adjacency[a].insert(b as u16);
        adjacency[b].insert(a as u16);
    }
    p.map = MoveMap::from_adjacency(&adjacency, &[ExtraAdjacency { from: 6, to: 3, kind: "sea".into(), through: 7 }], &|_, _| 4.0);
    p.map.set_rivers([(2, 3), (3, 6)]);
    let day = MilitaryDay::from_parts(p);
    let from = |owner: &str, previous: Option<u16>| Army { previous, ..army(9, owner, 3, vec![], regs(900, 1)) };
    assert_eq!(day.entry(&from("BBB", Some(2)), 3), Entry::River);
    assert_eq!(day.entry(&from("BBB", Some(4)), 3), Entry::Plain);
    // A strait stays a strait when a river is marked on it.
    assert_eq!(day.entry(&from("BBB", Some(6)), 3), Entry::Strait);
    // No previous province: a landing, except for rebels.
    assert_eq!(day.entry(&from("BBB", None), 3), Entry::Landing);
    assert_eq!(day.entry(&from("REB", None), 3), Entry::Plain);
    // The battle gets the worst of its attackers; the defender's way in does not count.
    let (mut mil, state) = world(vec![army(1, "BBB", 3, vec![], regs(100, 2)), army(2, "BBB", 3, vec![], regs(110, 2)), army(3, "AAA", 3, vec![], regs(200, 2))]);
    mil.armies[0].previous = Some(4);
    mil.armies[1].previous = Some(2);
    mil.combats = vec![Combat::new(CombatKind::Land, 3, vec![1, 2], vec![3])];
    let (_, _, ctx, rep) = day.land_round_alone(&mil, &state, 0, [5.0, 5.0]);
    assert_eq!((ctx.crossing, rep.modifiers), (-1, [-1.0, 0.0]));
    assert_eq!(crossing_penalty(&[Entry::River, Entry::Strait]), -2);
}

#[test]
fn technologies_and_inventions_add_to_unit_types_and_to_tactics() {
    use wiktoria_simulation::military::battle::Combat;
    use wiktoria_simulation::military::day::BattleContent;
    use wiktoria_simulation::military::units::unit_bonus;
    let mut p = parts();
    // A made-up technology and a made-up invention effect; `cart` is no unit type and `cost` no unit block.
    let tech = wiktoria_data::script::parse_bytes(b"cost = 100 army_base = { defence = 1 } infantry = { attack = 2 discipline = 0.5 } cart = { attack = 9 } navy_base = { hull = 4 }").unwrap();
    let effect = wiktoria_data::script::parse_bytes(b"infantry = { attack = 0.5 } military_tactics = 0.25").unwrap();
    let bonus = unit_bonus(&tech.root, &p.units);
    assert_eq!(bonus.keys().map(String::as_str).collect::<Vec<_>>(), ["army_base", "infantry", "navy_base"]);
    p.content = BattleContent {
        tech_units: BTreeMap::from([("drill".to_string(), bonus)]),
        invention_units: vec![BTreeMap::new(), unit_bonus(&effect.root, &p.units)],
        invention_values: vec![BTreeMap::new(), BTreeMap::from([("military_tactics".to_string(), 0.25)])],
        ..BattleContent::default()
    };
    p.techs.insert("drill".to_string(), BTreeMap::from([("military_tactics".to_string(), 1.0)]));
    let base = p.units["infantry"].clone();
    let day = MilitaryDay::from_parts(p);
    let (mut mil, mut state) = world(vec![army(1, "BBB", 3, vec![], regs(100, 1)), army(2, "AAA", 3, vec![], regs(200, 1))]);
    let bbb = state.countries.entry("BBB".to_string()).or_default();
    bbb.technologies.insert("drill".to_string());
    bbb.inventions.insert(2);
    state.countries.entry("AAA".to_string()).or_default();
    // BBB: the type's block, the land base and the invention; the sea base does not reach a land type.
    let d = day.unit_of(&state, "BBB", "infantry");
    assert_eq!((d.attack, d.defence, d.discipline, d.hull), (base.attack + 2.5, base.defence + 1.0, base.discipline + 0.5, base.hull));
    // AAA has neither: the unit file's values.
    assert_eq!(day.unit_of(&state, "AAA", "infantry"), base);
    // The battle sees them: the regiments' statistics and BBB's tactics from technology and invention.
    mil.armies[0].previous = Some(4);
    mil.combats = vec![Combat::new(CombatKind::Land, 3, vec![1], vec![2])];
    let (_, fighters, ctx, _) = day.land_round_alone(&mil, &state, 0, [5.0, 5.0]);
    assert_eq!((fighters[&100].attack, fighters[&200].attack), (base.attack + 2.5, base.attack));
    assert_eq!((ctx.attacker.countries[0].tactics_modifier, ctx.defender.countries[0].tactics_modifier), (1.25, 0.0));
}

/// Ticket #242: the day remembers every land regiment's strength of the evening before, not only
/// those already besieging, so an army whose siege starts tomorrow is booked on yesterday's men.
#[test]
fn every_regiment_is_remembered_for_tomorrows_siege() {
    // Army 1 stands at home (no siege), army 2 besieges province 3.
    let (mut mil, state) = world(vec![army(1, "BBB", 1, vec![], regs(100, 2)), army(2, "BBB", 3, vec![], regs(200, 6))]);
    mil.armies[0].regiments[0].strength = 2.5;
    let mut day = MilitaryDay::from_parts(parts());
    assert_eq!(day.yesterday_strength(100), None, "nothing is remembered before the first day");
    run(&mut day, &mut mil, &state, 1, |_| false);
    for a in &mil.armies {
        for r in &a.regiments {
            assert_eq!(day.yesterday_strength(r.id), Some(r.strength), "regiment {} of army {}", r.id, a.id);
        }
    }
    assert_eq!(day.yesterday_strength(100), Some(2.5));
    assert_eq!(day.yesterday_strength(999), None, "a regiment that does not exist");
}

/// Ticket #242: on the first day after a load the siege is booked on the strengths the save holds
/// (the evening before), not on what is left after that day's battle in the province.
#[test]
fn the_first_day_after_a_load_books_the_siege_on_the_saved_strengths() {
    use wiktoria_simulation::military::siege::{siege_progress, SiegeRegiment};
    // BBB besieges province 3 with six brigades; a lone AAA brigade fights it there.
    let start = || {
        let (mut mil, state) = world(vec![army(1, "BBB", 3, vec![], regs(100, 6)), army(2, "AAA", 3, vec![], regs(200, 1))]);
        mil.combats.push(Combat::new(CombatKind::Siege, 3, vec![1], vec![]));
        mil.combats.push(Combat::new(CombatKind::Land, 3, vec![2], vec![1]));
        (mil, state)
    };
    // The day the battle ends, played straight through.
    let (mut mil, state) = start();
    let mut day = MilitaryDay::from_parts(parts());
    let mut g = day.calendar.to_game(mil.date).unwrap();
    let mut last = None;
    for n in 0..60 {
        let rep = day.day(&mut mil, &state, day.calendar.to_date(g));
        g = wiktoria_simulation::GameDate(g.0 + 24);
        if rep.battles_finished > 0 {
            last = Some(n);
            break;
        }
    }
    let last = last.expect("the battle ends within 60 days");
    // The same game saved on the evening before and loaded into a new day step.
    let (mut mil, state) = start();
    let mut day = MilitaryDay::from_parts(parts());
    let mut g = day.calendar.to_game(mil.date).unwrap();
    for _ in 0..last {
        day.day(&mut mil, &state, day.calendar.to_date(g));
        g = wiktoria_simulation::GameDate(g.0 + 24);
    }
    let garrison = mil.provinces[&3].garrison;
    let saved: Vec<f64> = mil.armies.iter().find(|a| a.id == 1).unwrap().regiments.iter().map(|r| r.strength).collect();
    let mut loaded = MilitaryDay::from_parts(parts());
    loaded.battle_rng = day.battle_rng.clone();
    loaded.siege_rng = day.siege_rng.clone();
    let die = loaded.siege_rng.clone().die();
    let rep = loaded.day(&mut mil, &state, loaded.calendar.to_date(g));
    assert_eq!(rep.battles_finished, 1, "the battle also ends on the first day after the load");
    assert!(mil.combats.iter().any(|c| c.kind == CombatKind::Siege), "the siege goes on");
    let today: Vec<f64> = mil.armies.iter().find(|a| a.id == 1).unwrap().regiments.iter().map(|r| r.strength).collect();
    assert!(today.iter().sum::<f64>() < saved.iter().sum::<f64>(), "BBB lost men in the battle: {saved:?} -> {today:?}");
    let booked = |s: &[f64]| {
        let regs: Vec<SiegeRegiment> = s.iter().map(|&strength| SiegeRegiment { strength, siege: 0.0, recon: 0.0 }).collect();
        siege_progress(&loaded.siege, 0.0, &regs, die, false, false)
    };
    let fall = garrison - mil.provinces[&3].garrison;
    assert!(fall > 0.0, "the siege moved (die {die})");
    assert!((fall - booked(&saved)).abs() < 1e-9, "fall {fall}: saved men give {}, today's {}", booked(&saved), booked(&today));
}

// ---------------------------------------------------------------- over water

fn transport() -> UnitDef {
    UnitDef { name: "transport".into(), land: false, kind: "transport".into(), transport: true, max_strength: 1.0, organisation: 10.0, speed: 2.0, active: true, ..UnitDef::default() }
}

/// BBB's coast 2 lies on the sea 10, AAA's coast 3 on the sea 11: 1 - 2 ~ 10 ~ 11 ~ 3 - 4, every border 4 long.
fn sea_parts() -> DayParts {
    let mut p = parts();
    let mut map = MoveMap::default();
    for (id, sea, port) in [(1, false, None), (2, false, Some(10)), (3, false, Some(11)), (4, false, None), (10, true, None), (11, true, None)] {
        map.provinces.insert(id, MoveProvince { sea, port_sea: port, ..MoveProvince::default() });
    }
    for (a, b) in [(1, 2), (2, 10), (10, 11), (11, 3), (3, 4)] {
        map.borders.insert((a, b), Border { kind: BorderKind::Land, distance: 4.0, through: None });
        map.borders.insert((b, a), Border { kind: BorderKind::Land, distance: 4.0, through: None });
    }
    p.map = map;
    p.units.insert("transport".into(), transport());
    p
}

fn fleet(id: u64, owner: &str, at: u16, transports: usize) -> Navy {
    let ships = (0..transports).map(|i| Ship { id: id * 100 + i as u64, kind: "transport".into(), strength: 1.0, organisation: 10.0, ..Ship::default() }).collect();
    Navy { id, name: format!("fleet {id}"), owner: owner.into(), location: at, ships, supplies: 1.0, ..Navy::default() }
}

/// Day by day: (army location, carried by, fleet location).
fn sea_run(day: &mut MilitaryDay, mil: &mut MilitaryState, state: &State, days: usize) -> Vec<(u16, Option<u64>, Option<u16>)> {
    let mut out = Vec::new();
    run(day, mil, state, days, |m| {
        let a = m.armies.iter().find(|a| a.id == 1).expect("the army lives");
        out.push((a.location, a.carried_by, m.navies.first().map(|n| n.location)));
        false
    });
    out
}

#[test]
fn an_army_boards_sails_and_lands_over_the_sea() {
    let path = movement::army_path(&sea_parts().map, 1, 3).expect("a way by sea");
    assert_eq!(path, (vec![2, 10, 11, 3], true));
    assert_eq!(movement::army_path(&sea_parts().map, 1, 2), Some((vec![2], false)), "land first");
    let (mut mil, state) = world(vec![army(1, "BBB", 1, path.0, regs(100, 2))]);
    mil.navies.push(fleet(50, "BBB", 10, 2));
    let mut day = MilitaryDay::from_parts(sea_parts());
    let days = sea_run(&mut day, &mut mil, &state, 60);
    let boarded = days.iter().position(|d| d.1 == Some(50)).expect("the army boards the fleet");
    let landed = days.iter().position(|d| d.0 == 3).expect("the army lands on the far coast");
    assert!(boarded < landed);
    // Boarding and landing each take the step between land and sea of the rules file.
    let step = (day.movement.sea_land_distance / day.movement.sea_land_speed).ceil() as usize;
    assert!(boarded >= step && landed - boarded >= step, "boarded {boarded}, landed {landed}, step {step}");
    // While on board the army goes where its fleet goes; ashore it is free of it.
    assert!(days[boarded..landed].iter().all(|d| d.1 == Some(50) && Some(d.0) == d.2));
    assert_eq!(days[landed].2, Some(11), "the fleet sailed to the sea off the landing");
    let a = mil.armies.iter().find(|a| a.id == 1).unwrap();
    assert_eq!((a.carried_by, a.previous, a.path.clone(), a.target), (None, Some(11), vec![], None));
    assert!(mil.navies[0].carried.is_empty());
    assert_eq!(day.entry(a, 3), wiktoria_simulation::military::battle::Entry::Landing, "it fights as a landing");
    assert!(mil.combats.iter().any(|c| c.kind == CombatKind::Siege && c.location == 3), "and besieges the enemy coast");
}

#[test]
fn an_army_on_the_coast_calls_a_fleet_and_waits_for_it() {
    let (mut mil, state) = world(vec![army(1, "BBB", 2, vec![10, 11, 3], regs(100, 2))]);
    // The fleet waits off the enemy coast; a fleet too small for the army is nearer and stays put.
    mil.navies.push(fleet(50, "BBB", 11, 3));
    mil.navies.push(fleet(51, "BBB", 10, 1));
    let mut day = MilitaryDay::from_parts(sea_parts());
    run(&mut day, &mut mil, &state, 1, |_| false);
    assert!(day.fleet_called(50) && !day.fleet_called(51));
    assert_eq!(mil.armies[0].path, vec![10, 11, 3], "the army keeps its order while it waits");
    let days = sea_run(&mut day, &mut mil, &state, 60);
    assert!(days.iter().any(|d| d.1 == Some(50)), "it boards the called fleet");
    assert!(days.iter().all(|d| d.1 != Some(51)));
    assert_eq!(mil.armies[0].location, 3);
    assert_eq!(mil.navies.iter().find(|n| n.id == 51).unwrap().location, 10);
}

#[test]
fn an_order_over_water_without_room_is_cancelled() {
    let (mut mil, state) = world(vec![army(1, "BBB", 2, vec![10, 11, 3], regs(100, 2))]);
    // One transport carries one regiment; the army has two. An enemy fleet with room does not help.
    mil.navies.push(fleet(50, "BBB", 10, 1));
    mil.navies.push(fleet(60, "AAA", 10, 5));
    let mut day = MilitaryDay::from_parts(sea_parts());
    run(&mut day, &mut mil, &state, 1, |_| false);
    let a = &mil.armies[0];
    assert_eq!((a.location, a.carried_by, a.path.clone()), (2, None, vec![]));
    assert!(!day.fleet_called(50));
    assert_eq!(day.fleet_free_room(&mil, &mil.navies[0]), 1);
}

#[test]
fn a_landing_where_the_army_may_not_go_leaves_it_aboard() {
    // Province 3 belongs to CCC, at peace with BBB and giving it no access.
    let (mut mil, mut state) = world(vec![army(1, "BBB", 2, vec![10, 11, 3], regs(100, 1))]);
    mil.provinces.insert(3, prov("CCC"));
    state.provinces.insert(3, ProvinceState { owner: Some("CCC".into()), controller: Some("CCC".into()), ..ProvinceState::default() });
    mil.navies.push(fleet(50, "BBB", 10, 1));
    let mut day = MilitaryDay::from_parts(sea_parts());
    sea_run(&mut day, &mut mil, &state, 60);
    let a = &mil.armies[0];
    assert_eq!((a.location, a.carried_by, a.path.is_empty()), (11, Some(50), true));
    assert_eq!(mil.navies[0].carried, vec![1]);
}

#[test]
fn an_army_aboard_a_fleet_that_loses_its_last_ship_goes_down_with_it() {
    // Fleet 50 at sea 10 carries army 1; its ships are all gone (sunk or worn away).
    let mut a = army(1, "BBB", 10, vec![11, 3], regs(100, 1));
    a.carried_by = Some(50);
    let (mut mil, state) = world(vec![a, army(2, "BBB", 1, vec![], regs(200, 1))]);
    let mut f = fleet(50, "BBB", 10, 0);
    f.carried = vec![1];
    mil.navies.push(f);
    let mut day = MilitaryDay::from_parts(sea_parts());
    run(&mut day, &mut mil, &state, 2, |_| false);
    assert!(mil.navies.is_empty(), "the empty fleet is removed");
    assert_eq!(mil.armies.iter().map(|a| a.id).collect::<Vec<_>>(), vec![2], "the army at sea is lost with its fleet");
}

#[test]
fn an_army_aboard_a_shipless_fleet_in_port_walks_ashore() {
    let mut a = army(1, "BBB", 2, vec![], regs(100, 1));
    a.carried_by = Some(50);
    let (mut mil, state) = world(vec![a]);
    let mut f = fleet(50, "BBB", 2, 0);
    f.carried = vec![1];
    mil.navies.push(f);
    let mut day = MilitaryDay::from_parts(sea_parts());
    run(&mut day, &mut mil, &state, 1, |_| false);
    assert!(mil.navies.is_empty());
    let a = &mil.armies[0];
    assert_eq!((a.location, a.carried_by), (2, None));
}

#[test]
fn a_fleet_that_lost_transports_drops_the_regiments_it_cannot_carry() {
    // Two regiments aboard, one transport left.
    let mut a = army(1, "BBB", 10, vec![], regs(100, 2));
    a.carried_by = Some(50);
    let (mut mil, state) = world(vec![a]);
    let mut f = fleet(50, "BBB", 10, 1);
    f.carried = vec![1];
    mil.navies.push(f);
    let mut day = MilitaryDay::from_parts(sea_parts());
    run(&mut day, &mut mil, &state, 1, |_| false);
    let a = &mil.armies[0];
    assert_eq!((a.carried_by, a.regiments.iter().map(|r| r.id).collect::<Vec<_>>()), (Some(50), vec![101]));
}

#[test]
fn an_order_goes_round_a_closed_country_by_sea() {
    // A land bridge 2 - 4 that the army may not cross: the way over land is not taken.
    let mut map = sea_parts().map;
    for (a, b) in [(2, 4), (4, 2)] {
        map.borders.insert((a, b), Border { kind: BorderKind::Land, distance: 4.0, through: None });
    }
    assert_eq!(movement::army_path(&map, 2, 3), Some((vec![4, 3], false)));
    assert_eq!(movement::army_path_entering(&map, 2, 3, &|_, b| b != 4), Some((vec![10, 11, 3], true)));
}
