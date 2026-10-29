//! The AI foreign minister's war logic (wiktoria_simulation::military::war_ai and its wiring in game.rs) on a small
//! synthetic world: AAA owns provinces 1-4, BBB 5-7; they border at 4-5. Defines and the casus belli
//! type are made up here, rules file literals come from the repo's engine_rules.

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_data::date::Date;
use wiktoria_data::defines::Defines;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_simulation::calendar::Calendar;
use wiktoria_simulation::diplomacy::{DipCountry, DipRules, DiplomacyState};
use wiktoria_simulation::game::{Core, GameNumbers};
use wiktoria_simulation::military::ai::AiRules;
use wiktoria_simulation::military::battle::BattleRules;
use wiktoria_simulation::military::day::{DayParts, MilitaryDay};
use wiktoria_simulation::military::logistics::LogisticsRules;
use wiktoria_simulation::military::movement::{MoveMap, MovementRules};
use wiktoria_simulation::military::siege::{ProvinceMilitary, SiegeRules};
use wiktoria_simulation::military::units::{Army, Regiment, UnitDef};
use wiktoria_simulation::military::war::{self, CbType, HeldCb, War, WarEvent, WarEventKind, WarGoal, WarRules};
use wiktoria_simulation::military::war_ai::{self, AntagonizeInput, GoalContext, GoalOption, TargetInput, TargetMove, WarAiRules};
use wiktoria_simulation::military::{MilitaryCountry, MilitaryState};
use wiktoria_simulation::script::{CountryState, ProvinceState, State};

const DEFINES: &str = r#"
defines = {
	country = {
		BADBOY_LIMIT = 25,
		CREATED_CB_VALID_TIME = 12,
		MIN_MOBILIZE_LIMIT = 3,
		MOBILIZATION_SPEED_BASE = 0.12,
		MOBILIZATION_SPEED_RAILS_MULT = 3.0,
		BASE_GREATPOWER_DAILY_INFLUENCE = 0.25,
	},
	military = {
		COMBATLOSS_WAR_EXHAUSTION = 3, SIEGE_ATTRITION = 2, SUPPLY_RANGE = 250, REINFORCE_SPEED = 0.2,
		POP_MIN_SIZE_FOR_REGIMENT = 1000, POP_SIZE_PER_REGIMENT = 3000,
		POP_MIN_SIZE_FOR_REGIMENT_PROTECTORATE_MULTIPLIER = 8, POP_MIN_SIZE_FOR_REGIMENT_COLONY_MULTIPLIER = 5,
		POP_MIN_SIZE_FOR_REGIMENT_NONCORE_MULTIPLIER = 3, LEADER_RECRUIT_COST = 20, LEADER_AGE_DEATH_FACTOR = 4,
		LEADER_MAX_RANDOM_PRESTIGE = 0.05, LEADER_PRESTIGE_TO_MORALE_FACTOR = 0.8, LEADER_PRESTIGE_TO_MAX_ORG_FACTOR = 0.7,
		LAND_SPEED_MODIFIER = 1, NAVAL_SPEED_MODIFIER = 2, ENGINEER_UNIT_RATIO = 0.1, RECON_UNIT_RATIO = 0.1,
		RECON_SIEGE_EFFECT = 0.5, SIEGE_BRIGADES_MIN = 3, SIEGE_BRIGADES_MAX = 13, SIEGE_BRIGADES_BONUS = 0.5,
	},
	diplomacy = {
		PEACE_COST_ANNEX = 85, PEACE_COST_DEMAND_STATE = 10, PEACE_COST_STATUS_QUO = 1,
		INFAMY_ANNEX = 10, INFAMY_DEMAND_STATE = 5, PRESTIGE_ANNEX_BASE = 2, PRESTIGE_ANNEX = 0.02,
		PRESTIGE_DEMAND_STATE_BASE = 1, PRESTIGE_DEMAND_STATE = 0.01,
		BREAKTRUCE_INFAMY_ANNEX = 1, BREAKTRUCE_PRESTIGE_ANNEX = -20, BREAKTRUCE_MILITANCY_ANNEX = 2,
		WAR_FAILED_GOAL_MILITANCY = 2, WAR_FAILED_GOAL_PRESTIGE_BASE = -10, WAR_FAILED_GOAL_PRESTIGE = -0.1,
		MAX_WARSCORE_FROM_BATTLES = 50, WARGOAL_JINGOISM_REQUIREMENT = 0.07, GW_WARGOAL_JINGOISM_REQUIREMENT_MOD = 0.25,
		BASE_TRUCE_MONTHS = 60, GOOD_PEACE_REFUSAL_MILITANCY = 1.0, GOOD_PEACE_REFUSAL_WAREXH = 5.0,
		CB_GENERATION_BASE_SPEED = 0.5, CB_DETECTION_CHANCE_BASE = 15, ON_CB_DETECTED_RELATION_CHANGE = -50,
		CB_GENERATION_SPEED_BONUS_ON_COLONY_COMPETITION = 1.0, CB_GENERATION_SPEED_BONUS_ON_COLONY_COMPETITION_TROOPS_PRESENCE = 1.0,
		GW_JUSTIFY_CB_BADBOY_IMPACT = 0.33, GW_CB_CONSTRUCTION_SPEED = 0.25, GW_WARSCORE_COST_MOD = 0.65, GW_WARSCORE_COST_MOD_2 = 0.3,
		TWS_FULFILLED_SPEED = 0.1, TWS_NOT_FULFILLED_SPEED = 0.1, TWS_GRACE_PERIOD_DAYS = 730, TWS_CB_LIMIT_DEFAULT = 100,
		TWS_FULFILLED_IDLE_SPACE = 0.75, TWS_BATTLE_MIN_COUNT = 5, TWS_BATTLE_MAX_ASPECT = 8.0,
		MAX_INFLUENCE = 100, NEIGHBOUR_BONUS_INFLUENCE_PERCENT = 0.50, SPHERE_NEIGHBOUR_BONUS_INFLUENCE_PERCENT = 0.2,
		OTHER_CONTINENT_BONUS_INFLUENCE_PERCENT = -0.50, PUPPET_BONUS_INFLUENCE_PERCENT = 1.0, RELATION_INFLUENCE_MODIFIER = 1000,
		INVESTMENT_INFLUENCE_DEFENSE = 0.5, LARGE_POPULATION_INFLUENCE_PENALTY = -0.4, DISCREDIT_INFLUENCE_GAIN_FACTOR = -0.75,
		DISCREDIT_DAYS = 180, BANEMBASSY_DAYS = 365, REMOVEFROMSPHERE_PRESTIGE_COST = 10, REMOVEFROMSPHERE_INFAMY_COST = 1,
		INCREASEOPINION_RELATION_ON_ACCEPT = 20, INCREASEOPINION_INFLUENCE_COST = 50,
		DECREASEOPINION_RELATION_ON_ACCEPT = -10, DECREASEOPINION_INFLUENCE_COST = 50,
		ADDTOSPHERE_RELATION_ON_ACCEPT = 0, ADDTOSPHERE_INFLUENCE_COST = 100,
		REMOVEFROMSPHERE_RELATION_ON_ACCEPT = -10, REMOVEFROMSPHERE_INFLUENCE_COST = 100,
		DISCREDIT_RELATION_ON_ACCEPT = -5, DISCREDIT_INFLUENCE_COST = 25,
		EXPELADVISORS_RELATION_ON_ACCEPT = -5, EXPELADVISORS_INFLUENCE_COST = 50,
		BANEMBASSY_RELATION_ON_ACCEPT = -10, BANEMBASSY_INFLUENCE_COST = 65,
		ALLIANCE_RELATION_ON_ACCEPT = 100, ALLIANCE_RELATION_ON_DECLINE = -50, ALLIANCE_DIPLOMATIC_COST = 1,
		CANCELALLIANCE_RELATION_ON_ACCEPT = -20, CANCELALLIANCE_DIPLOMATIC_COST = 1,
		ASKMILACCESS_RELATION_ON_ACCEPT = 30, ASKMILACCESS_RELATION_ON_DECLINE = -10, ASKMILACCESS_DIPLOMATIC_COST = 1,
		GIVEMILACCESS_RELATION_ON_ACCEPT = 10, GIVEMILACCESS_RELATION_ON_DECLINE = 0, GIVEMILACCESS_DIPLOMATIC_COST = 1,
		CANCELASKMILACCESS_RELATION_ON_ACCEPT = 0, CANCELASKMILACCESS_DIPLOMATIC_COST = 1,
		CANCELGIVEMILACCESS_RELATION_ON_ACCEPT = -10,
		DISHONORED_CALLALLY_PRESTIGE_PENALTY = -3, RELEASE_NATION_INFAMY = -5, RELEASE_NATION_PRESTIGE = 0,
	},
	pops = {
		MIL_TO_AUTORISE = 1, PEACE_BASE_RELUCTANCE = 20, PEACE_TIME_MONTHS = 30, PEACE_TIME_FACTOR = 0.6,
		PEACE_TIME_FACTOR_NO_GOALS = 2.0, PEACE_WAR_EXHAUSTION_FACTOR = 0.5, PEACE_WAR_DIRECTION_FACTOR = 1.0,
		PEACE_WAR_DIRECTION_WINNING_MULT = 5.0, PEACE_FORCE_BALANCE_FACTOR = 0.3, PEACE_ALLY_BASE_RELUCTANCE_MULT = 2.0,
		PEACE_ALLY_TIME_MULT = 0, PEACE_ALLY_WAR_EXHAUSTION_MULT = 1.0, PEACE_ALLY_WAR_DIRECTION_MULT = 0,
		PEACE_ALLY_FORCE_BALANCE_MULT = 0, LARGE_POPULATION_LIMIT = 9000000, LARGE_POPULATION_INFLUENCE_PENALTY_CHUNK = 250000,
		RELATION_LIMIT_NO_ALLIANCE_OFFER = 0,
	},
}
"#;

fn engine_rules() -> EngineRules {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    wiktoria_data::config::load_engine_rules(&root).unwrap()
}

fn war_rules() -> WarRules {
    WarRules::from_engine_rules(&Defines::parse(DEFINES).unwrap(), &engine_rules()).unwrap()
}

fn ai_rules() -> WarAiRules {
    WarAiRules::from_engine_rules(&engine_rules()).unwrap()
}

fn conquest() -> CbType {
    CbType::new("conquest", &["demand_state"])
}

fn infantry() -> UnitDef {
    UnitDef { name: "infantry".into(), land: true, kind: "infantry".into(), max_strength: 3.0, organisation: 30.0, speed: 4.0, attack: 4.0, defence: 4.0, discipline: 1.0, maneuver: 1.0, active: true, ..UnitDef::default() }
}

fn regs(first_id: u64, n: usize) -> Vec<Regiment> {
    (0..n).map(|i| Regiment { id: first_id + i as u64, name: String::new(), kind: "infantry".into(), pop: None, pop_type: Some("soldiers".into()), organisation: 30.0, strength: 3.0, experience: 0.0, name_count: 0 }).collect()
}

fn army(id: u64, owner: &str, at: u16, n: usize) -> Army {
    Army { id, name: format!("army {id}"), owner: owner.into(), location: at, path: vec![], regiments: regs(id * 100, n), supplies: 1.0, ..Army::default() }
}

/// AAA: provinces 1-4 (regions 0, 1), BBB: 5, 6 (region 2), 7 (region 3, capital). A line 1-2-3-4-5-6-7.
fn core(aaa: usize, bbb: usize, wars: Vec<War>) -> (Core, State) {
    let m = engine_rules();
    let defines = Defines::parse(DEFINES).unwrap();
    let mut adjacency = vec![BTreeSet::new(); 9];
    for a in 1usize..7 {
        adjacency[a].insert(a as u16 + 1);
        adjacency[a + 1].insert(a as u16);
    }
    let parts = DayParts {
        logistics: LogisticsRules::new(&m, &defines).unwrap(),
        movement: MovementRules::new(&defines, &m).unwrap(),
        battle: BattleRules::from_engine_rules(&m).unwrap(),
        siege: SiegeRules::from_engine_rules(&m, &defines).unwrap(),
        war: war_rules(),
        calendar: Calendar::from_engine_rules(&m).unwrap(),
        map: MoveMap::from_adjacency(&adjacency, &[], &|_, _| 4.0),
        units: BTreeMap::from([("infantry".to_string(), infantry())]),
        cb_types: BTreeMap::from([("conquest".to_string(), conquest())]),
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
    };
    let d = Date::new(1840, 1, 2);
    let mut mil = MilitaryState {
        date: d,
        countries: BTreeMap::from([("AAA".to_string(), MilitaryCountry::default()), ("BBB".to_string(), MilitaryCountry::default())]),
        leaders: vec![],
        armies: vec![army(1, "AAA", 1, aaa), army(2, "BBB", 7, bbb)],
        navies: vec![],
        constructions: vec![],
        wars,
        combats: vec![],
        provinces: BTreeMap::new(),
        warnings: vec![],
    };
    let mut state = State::new(d);
    for p in 1u16..=7 {
        let owner = if p <= 4 { "AAA" } else { "BBB" };
        let region = match p {
            1 | 2 => 0,
            3 | 4 => 1,
            5 | 6 => 2,
            _ => 3,
        };
        mil.provinces.insert(p, ProvinceMilitary { owner: Some(owner.into()), controller: Some(owner.into()), garrison: 100.0, ..ProvinceMilitary::default() });
        state.provinces.insert(p, ProvinceState { owner: Some(owner.into()), controller: Some(owner.into()), cores: vec![owner.into()], region: Some(region), ..ProvinceState::default() });
        state.neighbours.insert(p, adjacency[p as usize].clone());
    }
    for (t, cap) in [("AAA", 1u16), ("BBB", 7u16)] {
        state.countries.insert(t.into(), CountryState { capital: Some(cap), civilized: true, prestige: 10.0, ..CountryState::default() });
    }
    let mut dip = DiplomacyState::new(d);
    for t in ["AAA", "BBB"] {
        dip.countries.insert(t.into(), DipCountry { points: 2.0, great_power: false, human: false });
    }
    let dip_rules = DipRules::new(&m, &defines).unwrap();
    dip.set_relation(&dip_rules, "AAA", "BBB", -50.0);
    let mut c = Core::new(mil, MilitaryDay::from_parts(parts), AiRules::new(&m, &defines).unwrap(), GameNumbers::from_engine_rules(&m).unwrap(), 11);
    c.dip = Some((dip_rules, dip));
    c.war_ai = Some(ai_rules());
    (c, state)
}

// ---- pure functions ---------------------------------------------------------------------

#[test]
fn rules_load_from_the_engine_rules() {
    let r = ai_rules();
    assert_eq!(r.list_size, 10);
    assert!((r.strength_ratio - 1.1).abs() < 1e-12);
}

#[test]
fn antagonize_list_keeps_the_ten_highest() {
    let r = ai_rules();
    let base = AntagonizeInput { x_provinces: 3.0, x_regiments: 2.0, own_provinces: 4.0, own_regiments: 10.0, ..Default::default() };
    let near = war_ai::antagonize_value(&r, &AntagonizeInput { neighbour: true, ..base });
    assert!(near > war_ai::antagonize_value(&r, &base), "a neighbour is disliked more");
    assert_eq!(war_ai::antagonize_value(&r, &AntagonizeInput { in_own_sphere: true, ..base }), 0.0);
    let many: Vec<(String, f64)> = (0..15).map(|i| (format!("T{i:02}"), i as f64 + 1.0)).collect();
    let list = war_ai::antagonize_list(&r, many);
    assert_eq!(list.len(), 10);
    assert_eq!(list[0].0, "T14");
}

#[test]
fn strength_test_needs_about_one_point_one_to_one() {
    let (r, w) = (ai_rules(), war_rules());
    let i = TargetInput { neighbour: true, own_regiments: 10.0, own_side: 10.0, target_side: 2.0, antagonize: 150.0, relation: -50.0, holds_cb: true, goal_value: 31.0, ..Default::default() };
    assert!(war_ai::target_score(&r, &w, &i) >= r.target_min_score);
    assert_eq!(war_ai::target_score(&r, &w, &TargetInput { own_side: 10.0, target_side: 9.5, ..i }), 0.0, "10 : 9.5 is under 1.1 : 1");
    // An ally next door to the enemy counts x 1.1: 9 + 1 x 1.1 against 9 passes, 9 + 1 does not reach 1.1.
    let with_ally = war_ai::side_strength(&r, 9.0, &[(1.0, true)]);
    assert!((with_ally - 10.1).abs() < 1e-9);
    assert_eq!(war_ai::target_score(&r, &w, &TargetInput { truce: true, ..i }), 0.0, "no target during a truce");
}

#[test]
fn draw_takes_one_target_in_proportion() {
    let r = ai_rules();
    let scored = vec![("AAA".to_string(), 30.0), ("BBB".to_string(), 10.0), ("CCC".to_string(), 90.0)];
    assert_eq!(war_ai::draw_target(&r, &scored, 0.0).as_deref(), Some("AAA"));
    assert_eq!(war_ai::draw_target(&r, &scored, 0.5).as_deref(), Some("CCC"), "BBB is under the minimum score");
    assert_eq!(war_ai::draw_target(&r, &[("X".into(), 5.0)], 0.5), None);
}

#[test]
fn goal_picker_respects_the_infamy_ceiling_and_cut_down_to_size_wins_against_a_bad_boy() {
    let (r, w) = (ai_rules(), war_rules());
    let conq = conquest();
    let mut humiliate = CbType::new("humiliate", &["prestige"]);
    humiliate.needs_no_justification = false;
    let options = [GoalOption { cb: &conq, held: false, cost: 0.0, present: false }, GoalOption { cb: &humiliate, held: false, cost: 0.0, present: false }];
    let ctx = GoalContext { own_infamy: 0.0, ceiling: 23.0, t_states: 2.0, own_states: 4.0, t_prestige: 100.0, own_prestige: 50.0, t_civilised: true, ..Default::default() };
    // Not over the limit and not alike: the conquest wins.
    let (k, _) = war_ai::pick_goal(&r, &w, &options, &ctx, None).unwrap();
    assert_eq!(options[k].cb.name, "conquest");
    // Against a country over the infamy limit, "cut down to size" (humiliation) counts in full and wins.
    let (k, _) = war_ai::pick_goal(&r, &w, &options, &GoalContext { t_over_limit: true, ..ctx }, None).unwrap();
    assert_eq!(options[k].cb.name, "humiliate");
    // Own infamy at the ceiling: no unjustified goal.
    assert!(war_ai::pick_goal(&r, &w, &options[..1], &GoalContext { own_infamy: 20.0, ..ctx }, None).is_none());
    // Held types only: an unheld conquest is skipped, a held one is taken.
    let held = [GoalOption { cb: &conq, held: true, cost: 0.0, present: false }];
    assert!(war_ai::pick_goal(&r, &w, &options[..1], &GoalContext { held_only: true, ..ctx }, None).is_none());
    assert!(war_ai::pick_goal(&r, &w, &held, &GoalContext { held_only: true, ..ctx }, None).is_some());
}

#[test]
fn target_move_justifies_worsens_then_declares() {
    let r = ai_rules();
    let i = war_ai::MoveInput { relation: 20.0, roll: 0.0, justify_chance: 5.0, justify_goal: Some(("conquest".into(), 31.0)), ..Default::default() };
    assert_eq!(war_ai::target_move(&r, &i), TargetMove::Justify("conquest".into()));
    assert_eq!(war_ai::target_move(&r, &war_ai::MoveInput { roll: 999.0, ..i.clone() }), TargetMove::DecreaseRelation);
    let ready = war_ai::MoveInput { relation: -10.0, has_cb: true, declare_goal: Some("conquest".into()), ..i.clone() };
    assert_eq!(war_ai::target_move(&r, &ready), TargetMove::Declare("conquest".into()));
    assert_eq!(war_ai::target_move(&r, &war_ai::MoveInput { truce: true, ..ready }), TargetMove::Nothing);
}

#[test]
fn peace_offer_drops_goals_from_the_end() {
    // The signer swallows at most two goals; goal 1 is obligatory.
    let signs = |kept: &[usize]| kept.len() <= 2;
    assert_eq!(war_ai::peace_offer(&[0, 1, 2, 3], &|g| g == 1, &signs, &signs), Some(vec![0, 1]));
    // Obligatory goals stay even when refused, and then nothing is sent.
    assert_eq!(war_ai::peace_offer(&[0, 1], &|_| true, &|k: &[usize]| k.is_empty(), &|k: &[usize]| k.is_empty()), None);
    // Everything refused: what is left is a white peace, sent only if both take it.
    assert_eq!(war_ai::peace_offer(&[0, 1], &|_| false, &|k: &[usize]| k.is_empty(), &|_| true), Some(vec![]));
    assert_eq!(war_ai::peace_offer(&[0, 1], &|_| false, &|k: &[usize]| k.is_empty(), &|_| false), None);
    // The signer takes all, but the other party does not: no offer.
    assert_eq!(war_ai::peace_offer(&[0, 1], &|_| false, &|_| true, &|k: &[usize]| k.len() < 2), None);
    // An empty table is a white peace offer.
    assert_eq!(war_ai::peace_offer(&[], &|_| false, &|_| false, &|_| true), Some(vec![]));
}

fn hold_cb(c: &mut Core) {
    c.held_cbs.insert(("AAA".into(), "BBB".into()), vec![HeldCb { cb_type: "conquest".into(), end: Date::new(1841, 1, 1) }]);
}

#[test]
fn a_weak_neighbour_with_bad_relations_gets_a_declaration() {
    let (mut c, mut s) = core(10, 2, vec![]);
    hold_cb(&mut c);
    let log = c.day(&mut s);
    assert_eq!(log.wars_declared.len(), 1, "AAA declares on BBB");
    let w = c.mil.wars.last().unwrap();
    assert!(w.active && w.attackers == ["AAA"] && w.defenders == ["BBB"]);
    assert_eq!(w.goals[0].casus_belli, "conquest");
    assert!(!c.held_cbs.contains_key(&("AAA".into(), "BBB".into())), "the casus belli is used up");
    assert_eq!(s.wars.len(), 1);
}

#[test]
fn a_strong_neighbour_gets_none() {
    let (mut c, mut s) = core(10, 20, vec![]);
    hold_cb(&mut c);
    let log = c.day(&mut s);
    assert!(log.wars_declared.iter().all(|w| !w.starts_with("AAA")), "AAA does not attack a stronger BBB");
}

#[test]
fn an_ally_answers_the_call() {
    let (mut c, mut s) = core(10, 2, vec![]);
    hold_cb(&mut c);
    // CCC owns nothing next door but is allied to AAA with good relations.
    s.countries.insert("CCC".into(), CountryState { civilized: true, ..CountryState::default() });
    let (rules, dip) = c.dip.as_mut().unwrap();
    dip.countries.insert("CCC".into(), DipCountry::default());
    wiktoria_simulation::diplomacy::treaties::make_alliance(dip, rules, "AAA", "CCC");
    c.day(&mut s);
    let w = c.mil.wars.last().unwrap();
    assert_eq!(w.attackers, ["AAA", "CCC"]);
}

#[test]
fn a_winning_war_ends_in_peace_with_its_goal_enforced() {
    let d = Date::new(1838, 1, 2);
    let goal = WarGoal { casus_belli: "conquest".into(), actor: "AAA".into(), receiver: "BBB".into(), state_province: Some(5), date: Some(d), score: 100.0, ..WarGoal::default() };
    let w = War {
        name: "Test War".into(),
        active: true,
        attackers: vec!["AAA".into()],
        defenders: vec!["BBB".into()],
        goals: vec![goal],
        history: vec![WarEvent { date: d, kind: WarEventKind::AddAttacker("AAA".into()) }, WarEvent { date: d, kind: WarEventKind::AddDefender("BBB".into()) }],
        ..War::default()
    };
    let (mut c, mut s) = core(10, 2, vec![w]);
    // AAA holds the wanted state (5, 6).
    for p in [5u16, 6] {
        c.mil.provinces.get_mut(&p).unwrap().controller = Some("AAA".into());
        s.provinces.get_mut(&p).unwrap().controller = Some("AAA".into());
    }
    let score = c.day.war_scores(&c.mil, &s)[0].1;
    assert!(score >= 50.0, "the war stands at {score:+.0} for AAA");
    let log = c.day(&mut s);
    assert_eq!(log.peaces, ["Test War"]);
    assert!(!c.mil.wars[0].active);
    for p in [5u16, 6] {
        assert_eq!(s.provinces[&p].owner.as_deref(), Some("AAA"), "province {p} changes owner");
        assert_eq!(c.mil.provinces[&p].owner.as_deref(), Some("AAA"));
    }
    assert_eq!(s.provinces[&7].owner.as_deref(), Some("BBB"));
    assert_eq!(log.provinces_transferred, 2);
    assert!(s.countries["AAA"].prestige > 10.0 && s.countries["BBB"].prestige < 10.0, "prestige moves with the goal");
    let (_, dip) = c.dip.as_ref().unwrap();
    assert!(dip.truce_active("AAA", "BBB", s.date), "a truce follows");
    let _ = war::truce_months(&war_rules(), &[]);
}

/// Advance `s` one day after `c.day`.
fn next_day(c: &Core, s: &mut State) {
    let cal = &c.day.calendar;
    s.date = cal.to_date(cal.next_day(cal.to_game(s.date).unwrap()));
}

#[test]
fn without_a_casus_belli_the_ai_picks_a_target_justifies_and_declares() {
    let (mut c, mut s) = core(10, 2, vec![]);
    // AAA wants BBB's land, as a save's conquer_prov list gives it (values on the save's scale).
    s.countries.get_mut("AAA").unwrap().ai_lists.conquer_prov = BTreeMap::from([(5, 100.0), (6, 100.0), (7, 80.0)]);
    let fm = c.numbers.foreign_minister_days as usize;
    let (mut justified, mut declared, mut targeted) = (0, 0, false);
    // About 0.5 % per foreign-minister run to start justifying (then months to build it): 3000 runs.
    for _ in 0..fm * 3000 {
        let log = c.day(&mut s);
        targeted |= c.war_targets.get("AAA").is_some_and(|t| t == "BBB");
        justified += log.cbs_justified;
        declared += log.wars_declared.len();
        next_day(&c, &mut s);
        if declared > 0 {
            break;
        }
    }
    assert!(targeted, "BBB becomes AAA's target without a casus belli in hand");
    assert!(justified >= 1, "AAA starts a justification");
    assert_eq!(declared, 1, "and declares once it holds the casus belli");
    assert_eq!(c.mil.wars.last().unwrap().defenders, ["BBB"]);
}

#[test]
fn a_long_war_with_a_clear_winner_ends_in_peace() {
    // Begun four years ago; AAA occupies all of BBB, so the score is clear and BBB concedes.
    let d = Date::new(1836, 1, 2);
    let goal = WarGoal { casus_belli: "conquest".into(), actor: "AAA".into(), receiver: "BBB".into(), state_province: Some(5), date: Some(d), score: 100.0, ..WarGoal::default() };
    let w = War {
        name: "Long War".into(),
        active: true,
        attackers: vec!["AAA".into()],
        defenders: vec!["BBB".into()],
        goals: vec![goal],
        history: vec![WarEvent { date: d, kind: WarEventKind::AddAttacker("AAA".into()) }, WarEvent { date: d, kind: WarEventKind::AddDefender("BBB".into()) }],
        ..War::default()
    };
    let (mut c, mut s) = core(10, 2, vec![w]);
    for p in [5u16, 6, 7] {
        c.mil.provinces.get_mut(&p).unwrap().controller = Some("AAA".into());
        s.provinces.get_mut(&p).unwrap().controller = Some("AAA".into());
    }
    let mut peaces = Vec::new();
    for _ in 0..c.numbers.foreign_minister_days * 3 {
        peaces.extend(c.day(&mut s).peaces);
        next_day(&c, &mut s);
        if !peaces.is_empty() {
            break;
        }
    }
    assert_eq!(peaces, ["Long War"]);
    assert!(!c.mil.wars[0].active);
}

#[test]
fn time_lowers_reluctance_but_does_not_forbid_peace() {
    let w = war_rules();
    let i = war::PeaceAnswerInput {
        m_is_leader: true,
        o_is_leader: true,
        m_score: -60.0,
        o_score: 60.0,
        direction: 0.0,
        months_of_war: 2.0,
        m_exhaustion: 0.0,
        land: 0.3,
        naval: 0.3,
        offer_cost: 30.0,
        own_goals: 0,
        own_granted: 0,
        annexes_m: false,
        has_enemy_obligatory: false,
        has_own_obligatory: false,
        m_infamy: 0.0,
        m_cannot_add_goals: false,
        m_holds_cb: false,
    };
    let young = war::ai_willingness(&w, &i);
    let old = war::ai_willingness(&w, &war::PeaceAnswerInput { months_of_war: 40.0, ..i });
    assert!(old > young, "willingness rises with the war's age");
    // A young war: the winner holds 60 points, the demand costs 30, so the loser signs anyway.
    assert!(war::ai_accepts_peace(&w, &i));
}

#[test]
fn a_civil_war_at_score_zero_is_not_settled_at_the_table() {
    // Both sides want to annex the other; nobody holds anything of the other.
    let d = Date::new(1836, 1, 1);
    let annex = CbType::new("civil", &["annex"]);
    let goal = |a: &str, b: &str| WarGoal { casus_belli: "civil".into(), actor: a.into(), receiver: b.into(), date: Some(d), ..WarGoal::default() };
    let w = War {
        name: "Civil War".into(),
        active: true,
        attackers: vec!["AAA".into()],
        defenders: vec!["BBB".into()],
        goals: vec![goal("BBB", "AAA"), goal("AAA", "BBB")],
        history: vec![WarEvent { date: d, kind: WarEventKind::AddAttacker("AAA".into()) }, WarEvent { date: d, kind: WarEventKind::AddDefender("BBB".into()) }],
        ..War::default()
    };
    let (mut c, mut s) = core(3, 3, vec![w]);
    c.day.cb_types.insert("civil".into(), annex);
    // No declarations on the side: only the peace table is looked at.
    c.war_ai = None;
    let owners: Vec<Option<String>> = s.provinces.values().map(|p| p.owner.clone()).collect();
    // The minister's first round is on the first day.
    let log = c.day(&mut s);
    assert!(log.peaces.is_empty(), "no peace at the table: {:?}", log.peace_trace);
    assert_eq!(log.provinces_transferred, 0);
    assert!(c.mil.wars[0].active);
    assert_eq!(s.provinces.values().map(|p| p.owner.clone()).collect::<Vec<_>>(), owners);
}

#[test]
fn a_war_without_goals_ends_in_a_white_peace() {
    let d = Date::new(1836, 1, 1);
    let w = War {
        name: "Empty War".into(),
        active: true,
        attackers: vec!["AAA".into()],
        defenders: vec!["BBB".into()],
        history: vec![WarEvent { date: d, kind: WarEventKind::AddAttacker("AAA".into()) }, WarEvent { date: d, kind: WarEventKind::AddDefender("BBB".into()) }],
        ..War::default()
    };
    let (mut c, mut s) = core(3, 3, vec![w]);
    c.war_ai = None;
    let log = c.day(&mut s);
    assert_eq!(log.peaces, ["Empty War"]);
    assert_eq!(log.provinces_transferred, 0);
    assert!(log.peace_trace[0].contains("no goal left"), "{:?}", log.peace_trace);
    assert!(!c.mil.wars[0].active);
}

// ---- a country a person plays (player.rs): its own war and peace, questions instead of answers --

fn play(c: &mut Core, s: &mut State, tag: &str) {
    wiktoria_simulation::player::set_player(s, c, tag, true).unwrap();
}

/// Carries out one order of `tag`'s player now.
fn order(c: &mut Core, s: &mut State, tag: &str, o: wiktoria_simulation::player::Order) -> Result<String, String> {
    let mut q = wiktoria_simulation::player::Players::default();
    q.push(tag, o);
    wiktoria_simulation::player::apply(&mut q, s, c, &Default::default(), &Default::default()).remove(0).result
}

fn winning_war(start: Date) -> War {
    let goal = WarGoal { casus_belli: "conquest".into(), actor: "AAA".into(), receiver: "BBB".into(), state_province: Some(5), date: Some(start), score: 100.0, ..WarGoal::default() };
    War {
        name: "Test War".into(),
        active: true,
        attackers: vec!["AAA".into()],
        defenders: vec!["BBB".into()],
        goals: vec![goal],
        history: vec![WarEvent { date: start, kind: WarEventKind::AddAttacker("AAA".into()) }, WarEvent { date: start, kind: WarEventKind::AddDefender("BBB".into()) }],
        ..War::default()
    }
}

/// AAA holds BBB's state 5-6: the war stands well for AAA.
fn occupy(c: &mut Core, s: &mut State) {
    for p in [5u16, 6] {
        c.mil.provinces.get_mut(&p).unwrap().controller = Some("AAA".into());
        s.provinces.get_mut(&p).unwrap().controller = Some("AAA".into());
    }
}

#[test]
fn the_computer_declares_no_war_for_a_played_country() {
    let (mut c, mut s) = core(10, 2, vec![]);
    hold_cb(&mut c);
    play(&mut c, &mut s, "AAA");
    let fm = c.numbers.foreign_minister_days as usize;
    for _ in 0..fm * 5 {
        assert!(c.day(&mut s).wars_declared.is_empty());
        next_day(&c, &mut s);
    }
    assert!(c.mil.wars.is_empty() && !c.justifying.keys().any(|(a, _)| a == "AAA"), "no war and no justification on AAA's behalf");
}

#[test]
fn a_player_justifies_then_declares_war() {
    use wiktoria_simulation::player::Order::{DeclareWar, Justify};
    let (mut c, mut s) = core(10, 2, vec![]);
    play(&mut c, &mut s, "AAA");
    let declare = DeclareWar { target: "BBB".into(), cb: "conquest".into() };
    assert!(order(&mut c, &mut s, "AAA", declare.clone()).unwrap_err().contains("justify"), "no casus belli yet");
    assert!(order(&mut c, &mut s, "AAA", Justify { target: "BBB".into(), cb: "nothing".into() }).is_err());
    assert!(order(&mut c, &mut s, "AAA", Justify { target: "AAA".into(), cb: "conquest".into() }).is_err());
    assert!(order(&mut c, &mut s, "BBB", Justify { target: "AAA".into(), cb: "conquest".into() }).unwrap_err().contains("computer"), "BBB is not played");
    order(&mut c, &mut s, "AAA", Justify { target: "BBB".into(), cb: "conquest".into() }).unwrap();
    assert!(order(&mut c, &mut s, "AAA", Justify { target: "BBB".into(), cb: "conquest".into() }).unwrap_err().contains("already"), "one at a time");
    // The computer's daily progress builds it.
    let mut days = 0;
    while !c.held_cbs.contains_key(&("AAA".into(), "BBB".into())) {
        c.day(&mut s);
        next_day(&c, &mut s);
        days += 1;
        assert!(days < 2000, "the casus belli is never finished");
    }
    let infamy = s.countries["AAA"].badboy;
    let text = order(&mut c, &mut s, "AAA", declare.clone()).unwrap();
    assert!(text.contains("AAA-BBB war"), "{text}");
    let w = c.mil.wars.last().unwrap();
    assert!(w.active && w.attackers == ["AAA"] && w.defenders == ["BBB"] && w.goals[0].casus_belli == "conquest");
    assert_eq!(s.countries["AAA"].badboy, infamy, "a held casus belli: a just war, no infamy");
    assert!(!c.held_cbs.contains_key(&("AAA".into(), "BBB".into())), "used up");
    assert_eq!(s.wars.len(), 1);
    assert!(order(&mut c, &mut s, "AAA", declare).is_err(), "already at war");
}

#[test]
fn a_played_ally_is_asked_to_join() {
    use wiktoria_simulation::player::{AskKind, Order::Answer};
    for accept in [true, false] {
        let (mut c, mut s) = core(10, 2, vec![]);
        hold_cb(&mut c);
        s.countries.insert("CCC".into(), CountryState { civilized: true, prestige: 10.0, ..CountryState::default() });
        let (rules, dip) = c.dip.as_mut().unwrap();
        dip.countries.insert("CCC".into(), wiktoria_simulation::diplomacy::DipCountry::default());
        wiktoria_simulation::diplomacy::treaties::make_alliance(dip, rules, "AAA", "CCC");
        play(&mut c, &mut s, "CCC");
        c.day(&mut s);
        let w = c.mil.wars.last().unwrap().clone();
        assert_eq!(w.attackers, ["AAA"], "the computer does not answer for CCC");
        let ask = c.asks.list[0].clone();
        assert_eq!((ask.to.as_str(), ask.from.as_str(), ask.war.as_str(), &ask.kind), ("CCC", "AAA", w.name.as_str(), &AskKind::Call { attackers: true }));
        assert!(order(&mut c, &mut s, "CCC", Answer { key: ask.key + 7, accept }).is_err());
        order(&mut c, &mut s, "CCC", Answer { key: ask.key, accept }).unwrap();
        assert!(c.asks.list.is_empty());
        let allied = c.dip.as_ref().unwrap().1.allied("AAA", "CCC");
        if accept {
            assert_eq!(c.mil.wars.last().unwrap().attackers, ["AAA", "CCC"]);
            assert!(allied);
        } else {
            assert_eq!(c.mil.wars.last().unwrap().attackers, ["AAA"]);
            assert!(!allied && s.countries["CCC"].prestige < 10.0, "a refused call ends the alliance and costs prestige");
        }
    }
}

#[test]
fn a_player_offers_peace_and_the_computer_answers() {
    use wiktoria_simulation::player::Order::OfferPeace;
    let (mut c, mut s) = core(10, 2, vec![winning_war(Date::new(1838, 1, 2))]);
    occupy(&mut c, &mut s);
    play(&mut c, &mut s, "AAA");
    let offer = |goals: Vec<usize>| OfferPeace { war: "Test War".into(), to: "BBB".into(), goals };
    assert!(order(&mut c, &mut s, "AAA", OfferPeace { war: "No War".into(), to: "BBB".into(), goals: vec![] }).is_err());
    assert!(order(&mut c, &mut s, "AAA", offer(vec![3])).unwrap_err().contains("no goal 3"));
    assert!(order(&mut c, &mut s, "AAA", OfferPeace { war: "Test War".into(), to: "AAA".into(), goals: vec![] }).is_err(), "not with oneself");
    // The computer never makes this peace for AAA: a round of the foreign minister passes.
    assert!(c.day(&mut s).peaces.is_empty());
    next_day(&c, &mut s);
    let text = order(&mut c, &mut s, "AAA", offer(vec![0])).unwrap();
    assert!(text.contains("accepts"), "{text}");
    assert!(!c.mil.wars[0].active && s.wars.is_empty());
    for p in [5u16, 6] {
        assert_eq!(s.provinces[&p].owner.as_deref(), Some("AAA"));
    }
}

#[test]
fn the_computer_asks_a_played_country_before_peace() {
    use wiktoria_simulation::player::{AskKind, Order::Answer};
    for accept in [true, false] {
        let (mut c, mut s) = core(10, 2, vec![winning_war(Date::new(1838, 1, 2))]);
        occupy(&mut c, &mut s);
        play(&mut c, &mut s, "BBB");
        let log = c.day(&mut s);
        assert!(log.peaces.is_empty() && c.mil.wars[0].active, "the peace is not forced on BBB");
        assert_eq!(c.asks.list.len(), 1);
        let ask = c.asks.list[0].clone();
        assert_eq!((ask.to.as_str(), ask.from.as_str()), ("BBB", "AAA"));
        let AskKind::Peace { goals } = &ask.kind else { panic!("a peace offer") };
        assert_eq!(goals.len(), 1, "the computer demands its goal");
        assert!(order(&mut c, &mut s, "AAA", Answer { key: ask.key, accept }).is_err(), "only BBB answers");
        let text = order(&mut c, &mut s, "BBB", Answer { key: ask.key, accept }).unwrap();
        assert_eq!(c.mil.wars[0].active, !accept, "{text}");
        assert_eq!(s.provinces[&5].owner.as_deref(), Some(if accept { "AAA" } else { "BBB" }));
        if !accept {
            // Refused: AAA offers again in this war only after the answer time.
            let mut back = None;
            for day in 1..=c.asks.answer_days + 2 * c.numbers.foreign_minister_days as i64 {
                next_day(&c, &mut s);
                c.day(&mut s);
                if !c.asks.list.is_empty() {
                    back = Some(day);
                    break;
                }
            }
            assert!(back.is_some_and(|d| d >= c.asks.answer_days), "offered again on day {back:?}");
            assert!(c.mil.wars[0].active);
        }
    }
}

#[test]
fn an_unanswered_offer_takes_the_computers_answer() {
    let (mut c, mut s) = core(10, 2, vec![winning_war(Date::new(1838, 1, 2))]);
    occupy(&mut c, &mut s);
    play(&mut c, &mut s, "BBB");
    c.asks.answer_days = 3;
    c.day(&mut s);
    assert_eq!(c.asks.list.len(), 1);
    let mut peaces = Vec::new();
    for _ in 0..3 {
        next_day(&c, &mut s);
        peaces.extend(c.day(&mut s).peaces);
    }
    assert_eq!(peaces, ["Test War"], "after three days the computer signs for BBB, as it would itself");
    assert!(c.asks.list.is_empty());
    // A country handed back to the computer gets its answer at once.
    let (mut c, mut s) = core(10, 2, vec![winning_war(Date::new(1838, 1, 2))]);
    occupy(&mut c, &mut s);
    play(&mut c, &mut s, "BBB");
    c.day(&mut s);
    wiktoria_simulation::player::set_player(&mut s, &mut c, "BBB", false).unwrap();
    next_day(&c, &mut s);
    assert_eq!(c.day(&mut s).peaces, ["Test War"]);
}

#[test]
fn a_war_without_goals_asks_its_played_leader() {
    let d = Date::new(1836, 1, 1);
    let w = War {
        name: "Empty War".into(),
        active: true,
        attackers: vec!["AAA".into()],
        defenders: vec!["BBB".into()],
        history: vec![WarEvent { date: d, kind: WarEventKind::AddAttacker("AAA".into()) }, WarEvent { date: d, kind: WarEventKind::AddDefender("BBB".into()) }],
        ..War::default()
    };
    let (mut c, mut s) = core(3, 3, vec![w]);
    c.war_ai = None;
    play(&mut c, &mut s, "AAA");
    assert!(c.day(&mut s).peaces.is_empty());
    assert_eq!((c.asks.list[0].to.as_str(), c.asks.list[0].from.as_str()), ("AAA", "BBB"));
}

#[test]
fn a_war_without_goals_asks_its_played_leader_once() {
    let d = Date::new(1836, 1, 1);
    let w = War {
        name: "Empty War".into(),
        active: true,
        attackers: vec!["AAA".into()],
        defenders: vec!["BBB".into()],
        history: vec![WarEvent { date: d, kind: WarEventKind::AddAttacker("AAA".into()) }, WarEvent { date: d, kind: WarEventKind::AddDefender("BBB".into()) }],
        ..War::default()
    };
    let (mut c, mut s) = core(3, 3, vec![w]);
    c.war_ai = None;
    c.asks.answer_days = 10_000;
    play(&mut c, &mut s, "AAA");
    c.day(&mut s);
    for _ in 0..3 * c.numbers.foreign_minister_days {
        next_day(&c, &mut s);
        c.day(&mut s);
    }
    assert_eq!(c.asks.list.len(), 1, "the question waits; no new one each round");
}

#[test]
fn an_offer_whose_goals_left_the_war_is_dropped() {
    use wiktoria_simulation::player::Order::Answer;
    for player_answers in [true, false] {
        let (mut c, mut s) = core(10, 2, vec![winning_war(Date::new(1838, 1, 2))]);
        occupy(&mut c, &mut s);
        play(&mut c, &mut s, "BBB");
        c.asks.answer_days = 1;
        c.day(&mut s);
        assert_eq!(c.asks.list.len(), 1);
        let key = c.asks.list[0].key;
        // The demanded goal goes out of the war (as in a separate peace) before the answer.
        c.mil.wars[0].goals.clear();
        if player_answers {
            assert!(order(&mut c, &mut s, "BBB", Answer { key, accept: true }).is_err());
        } else {
            next_day(&c, &mut s);
            assert!(c.day(&mut s).peaces.is_empty(), "no smaller peace is signed for the player");
        }
        assert!(c.asks.list.iter().all(|a| a.key != key), "the question goes");
        assert!(c.mil.wars[0].active, "and the war goes on");
    }
}
