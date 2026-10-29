//! War rules on synthetic numbers: defines are made up here,
//! rules file literals come from the repo's engine_rules.

use std::collections::BTreeMap;

use wiktoria_data::date::Date;
use wiktoria_data::defines::Defines;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_simulation::military::war::*;

const DEFINES: &str = r#"
defines = {
	country = {
		BADBOY_LIMIT = 25,
		CREATED_CB_VALID_TIME = 12,
	},
	military = {
		COMBATLOSS_WAR_EXHAUSTION = 3,
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
		BREAKTRUCE_INFAMY_DEMAND_STATE = 1,
		BREAKTRUCE_PRESTIGE_DEMAND_STATE = -20,
		BREAKTRUCE_MILITANCY_DEMAND_STATE = 2,
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

fn rules() -> WarRules {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/engine_rules.json");
    let m = EngineRules::from_json(&std::fs::read_to_string(p).unwrap()).unwrap();
    WarRules::from_engine_rules(&Defines::parse(DEFINES).unwrap(), &m).unwrap()
}

fn cbs() -> BTreeMap<String, CbType> {
    let mut m = BTreeMap::new();
    let mut add = |c: CbType| {
        m.insert(c.name.clone(), c);
    };
    add(CbType::new("conquest", &["demand_state"]));
    add(CbType::new("annex_cb", &["annex"]));
    add(CbType::new("quo", &["status_quo"]));
    add(CbType::new("humiliate", &["prestige"]));
    m
}

fn goal(cb: &str, actor: &str, receiver: &str) -> WarGoal {
    WarGoal { casus_belli: cb.into(), actor: actor.into(), receiver: receiver.into(), ..WarGoal::default() }
}

/// AAA leads attackers with ABB; DDD leads defenders with DEE.
fn war(goals: Vec<WarGoal>) -> War {
    War {
        name: "w".into(),
        active: true,
        attackers: vec!["AAA".into(), "ABB".into()],
        defenders: vec!["DDD".into(), "DEE".into()],
        goals,
        ..War::default()
    }
}

fn d(y: i32, m: u8, dd: u8) -> Date {
    Date::new(y, m, dd)
}

#[test]
fn casus_belli_builds_in_200_days_and_expires_in_12_months() {
    let r = rules();
    let cb = CbType::new("conquest", &["demand_state"]);
    let gain = cb_daily_progress(&r, &cb, &CbSpeedInput::default());
    assert!((gain - 0.5).abs() < 1e-9);
    let (mut p, mut done, mut days) = (0.0, false, 0);
    while !done {
        (p, done) = cb_build_day(&r, p, gain);
        days += 1;
    }
    assert_eq!((days, p), (200, 100.0));
    // modifiers
    let fast = cb_daily_progress(&r, &cb, &CbSpeedInput { speed_modifier: 0.3, great_war_with_target: true, ..Default::default() });
    assert!((fast - 0.5 * 1.3 * 1.25).abs() < 1e-9);
    assert_eq!(cb_daily_progress(&r, &cb, &CbSpeedInput { speed_modifier: -2.0, ..Default::default() }), 0.0);
    // detection is 1.5 %
    assert!(cb_detected(&r, 14) && !cb_detected(&r, 15));
    // infamy of a detected one: only the unbuilt part
    assert!((cb_detection_infamy(&r, &cb, 75.0, false) - 5.0 * 0.25).abs() < 1e-9);
    // expiry
    let held = cb_gain(&r, "conquest", d(1840, 3, 31));
    assert_eq!(held.end, d(1841, 3, 31));
    assert!(!cb_expired(&held, d(1841, 3, 30)) && cb_expired(&held, d(1841, 3, 31)));
    assert_eq!(add_months(d(1840, 1, 31), 1), d(1840, 2, 29));
}

#[test]
fn declaring_just_unjust_and_breaking_a_truce() {
    let r = rules();
    let cb = CbType::new("conquest", &["demand_state"]);
    let just = declare_war(&r, &cb, &DeclareInput { holds_cb: true, ..Default::default() });
    assert!(just.just && just.infamy == 0.0 && just.consumes_cb);
    let unjust = declare_war(&r, &cb, &DeclareInput::default());
    assert!(!unjust.just && unjust.infamy == 5.0 && !unjust.consumes_cb);
    // great war cuts the infamy
    let gw = declare_war(&r, &cb, &DeclareInput { great_war_with_target: true, ..Default::default() });
    assert!((gw.infamy - 5.0 * 0.33).abs() < 1e-9);
    // no justification needed
    let mut free = cb.clone();
    free.needs_no_justification = true;
    assert!(declare_war(&r, &free, &DeclareInput::default()).just);
    // truce: infamy 1, prestige -20, militancy 2, on top of a just war
    let t = declare_war(&r, &cb, &DeclareInput { holds_cb: true, truce_active: true, ..Default::default() });
    assert_eq!((t.infamy, t.prestige, t.militancy), (1.0, -20.0, 2.0));
    // unjust and truce add up
    let both = declare_war(&r, &cb, &DeclareInput { truce_active: true, ..Default::default() });
    assert_eq!(both.infamy, 6.0);
    // factor of the type scales the infamy
    let mut bad = cb.clone();
    bad.badboy_factor = 2.0;
    assert_eq!(goal_infamy(&r, &bad, false), 10.0);
}

#[test]
fn adding_a_goal_mid_war() {
    let r = rules();
    let ok = AddGoalInput { status_quo_goal: false, score_vs_leader: 10.0, jingoism_share: 0.1, great_war: false };
    assert_eq!(can_add_goal(&r, &ok), Ok(()));
    assert_eq!(can_add_goal(&r, &AddGoalInput { status_quo_goal: true, ..ok }), Err(AddGoalRefusal::StatusQuoGoal));
    assert_eq!(can_add_goal(&r, &AddGoalInput { score_vs_leader: 0.0, ..ok }), Err(AddGoalRefusal::NoWarScore));
    assert_eq!(can_add_goal(&r, &AddGoalInput { jingoism_share: 0.05, ..ok }), Err(AddGoalRefusal::NotEnoughJingoism));
    // a great war lowers the requirement: 0.07 x 0.75 = 0.0525
    assert_eq!(can_add_goal(&r, &AddGoalInput { jingoism_share: 0.06, great_war: true, ..ok }), Ok(()));
    // unjust added goal pays the infamy, a held casus belli does not
    let cb = CbType::new("conquest", &["demand_state"]);
    assert_eq!(add_goal_cost(&r, &cb, false, false, false).infamy, 5.0);
    assert_eq!(add_goal_cost(&r, &cb, true, false, false).infamy, 0.0);
}

#[test]
fn war_score_parts() {
    let r = rules();
    // a land battle with 3000 losses vs 1000: 0.1 + (3000 - 500) / 5000 = 0.6
    assert!((battle_points(&r, 1000.0, 3000.0, false) - 0.6).abs() < 1e-9);
    assert!((battle_points(&r, 4000.0, 1000.0, false) - 0.1).abs() < 1e-9);
    // battles part is capped at 50
    let mut w = war(vec![]);
    for i in 0..300 {
        let rec = BattleRecord { attacker_won: true, attacker: BattleSide { losses: 10.0, ..Default::default() }, defender: BattleSide { losses: 1000.0, ..Default::default() }, location: i, ..Default::default() };
        w.history.push(WarEvent { date: d(1840, 1, 1), kind: WarEventKind::Battle(rec) });
    }
    assert_eq!(battles_part(&r, &w, &|_| false), 50.0);
    // occupation: share of owner total value; blockaded port a quarter
    let v = province_value(&r, &ProvinceValueInput { owner_has_core: true, in_owner_capital_state: true, ..Default::default() });
    assert_eq!(v, 6.0);
    let overseas = province_value(&r, &ProvinceValueInput { owner_has_core: true, overseas: true, ..Default::default() });
    assert_eq!(overseas, 1.0);
    assert_eq!(province_score(v, 60.0), 10.0);
    assert_eq!(occupation_part(&r, &[10.0, 5.0], &[8.0]), 17.0);
    // totals
    let base = ScoreInput { battles: 30.0, occupation: 50.0, ticking: 40.0, ..Default::default() };
    assert_eq!(war_score(&base), 100.0);
    assert_eq!(war_score(&ScoreInput { battles: 10.0, occupation: 5.0, ticking: 2.0, ..Default::default() }), 17.0);
    assert_eq!(war_score(&ScoreInput { a_holds_all_of_b: true, ..Default::default() }), 100.0);
    assert_eq!(war_score(&ScoreInput { b_holds_all_of_a: true, battles: 40.0, ..Default::default() }), -100.0);
}

#[test]
fn ticking_goals_run_at_a_tenth_a_day() {
    let r = rules();
    let mut s = 0.0;
    for _ in 0..100 {
        s = tick_goal(&r, s, 1.0, 0.0);
    }
    assert!((s - 10.0).abs() < 1e-9);
    // below the idle space: positive stands still, negative recovers
    assert_eq!(tick_goal(&r, 5.0, 0.5, 0.0), 5.0);
    assert!((tick_goal(&r, -5.0, 0.5, 0.0) - -4.95).abs() < 1e-9);
    // not fulfilled: falls to 0, then waits out the grace period
    assert!((tick_goal(&r, 5.0, 0.0, 0.0) - 4.9).abs() < 1e-9);
    assert_eq!(tick_goal(&r, 0.0, 0.0, 100.0), 0.0);
    assert!((tick_goal(&r, 0.0, 0.0, 730.0) - -0.1).abs() < 1e-9);
    assert_eq!(tick_goal(&r, 99.95, 1.0, 0.0), 100.0);
    // contribution: whole part x min(cost, limit) / limit
    assert_eq!(goal_score_contribution(&r, 40.7, 50.0), 20.0);
    assert_eq!(goal_score_contribution(&r, 40.0, 500.0), 40.0);
    // battle goals need five battles and a ratio above the factor
    assert_eq!(battle_fulfilment(&r, 4, 10.0, 1.0, 1.0), 0.0);
    assert_eq!(battle_fulfilment(&r, 5, 1.0, 2.0, 1.0), 0.0);
    assert!((battle_fulfilment(&r, 5, 5.0, 1.0, 1.0) - 0.5).abs() < 1e-9);
}

#[test]
fn failed_goal_and_peace_cost() {
    let r = rules();
    let cb = CbType::new("conquest", &["demand_state"]);
    // the larger of 10 and 10 % of prestige
    let low = failed_goal_penalty(&r, &cb, 50.0);
    assert_eq!((low.militancy, low.prestige), (2.0, -10.0));
    assert_eq!(failed_goal_penalty(&r, &cb, 400.0).prestige, -40.0);
    let mut soft = cb.clone();
    soft.penalty_factor = 0.5;
    assert_eq!(failed_goal_penalty(&r, &soft, 50.0).prestige, -5.0);
    // a state: each province clamp(200 x value / total, 1, value) x 2.8
    let one = GoalCostInput { provinces: vec![(4.0, 800.0)], ..Default::default() };
    assert!((goal_cost(&r, &cb, &one) - 1.0 * 2.8).abs() < 1e-9);
    let big = GoalCostInput { provinces: vec![(4.0, 100.0)], ..Default::default() };
    assert!((goal_cost(&r, &cb, &big) - 4.0 * 2.8).abs() < 1e-9);
    // flat annex 85, great war factor, cap
    let annex = CbType::new("annex_cb", &["annex"]);
    let flat = GoalCostInput::default();
    assert_eq!(goal_cost(&r, &annex, &flat), 85.0);
    assert!((goal_cost(&r, &annex, &GoalCostInput { stage: GreatWarStage::GreatWar, ..Default::default() }) - 55.25).abs() < 1e-9);
    let mut dear = annex.clone();
    dear.peace_cost_factor = 3.0;
    assert_eq!(goal_cost(&r, &dear, &flat), 100.0);
    let mut free = annex.clone();
    free.zero_cost = true;
    assert_eq!(goal_cost(&r, &free, &flat), 0.0);
}

#[test]
fn truce_is_base_plus_largest_goal_months() {
    let r = rules();
    let a = CbType::new("a", &["annex"]);
    let mut b = CbType::new("b", &["demand_state"]);
    assert_eq!(truce_months(&r, &[&a]), 60.0);
    b.truce_months = 24.0;
    let mut c = CbType::new("c", &["prestige"]);
    c.truce_months = 12.0;
    assert_eq!(truce_months(&r, &[&a, &b, &c]), 84.0);
}

fn req<'a>(w: &'a War, sender: &'a str, target: &'a str, selected: &'a [usize]) -> PeaceRequest<'a> {
    PeaceRequest { war: w, crisis: false, sender, target, selected }
}

#[test]
fn peace_with_the_whole_enemy_side_through_the_leaders() {
    let r = rules();
    let cbs = cbs();
    let w = war(vec![goal("conquest", "AAA", "DDD"), goal("conquest", "ABB", "DEE")]);
    assert_eq!(peace_allowed(&req(&w, "AAA", "DDD", &[0, 1]), &cbs), Ok(PeaceScope::WholeWar));
    // goal 1 is left out: it fails, its actor pays; everybody leaves and the truce is 60 months
    let out = plan_peace(&r, &req(&w, "AAA", "DDD", &[0]), &cbs, &|_| true, &|_| 200.0).unwrap();
    assert_eq!(out.scope, PeaceScope::WholeWar);
    assert_eq!(out.enforced, vec![0]);
    assert_eq!(out.failed.len(), 1);
    assert_eq!((out.failed[0].0, out.failed[0].1.as_str(), out.failed[0].2.prestige), (1, "ABB", -20.0));
    assert_eq!(out.leaving.len(), 4);
    assert_eq!(out.truce_months, 60.0);
    // a goal no longer possible fails even when selected
    let out = plan_peace(&r, &req(&w, "AAA", "DDD", &[0, 1]), &cbs, &|g| g.actor != "AAA", &|_| 0.0).unwrap();
    assert_eq!((out.enforced.clone(), out.failed.len()), (vec![1], 1));
    // not at war with each other, or on the same side
    assert_eq!(peace_allowed(&req(&w, "AAA", "ABB", &[]), &cbs), Err(PeaceRefusal::NotAtWar));
    assert_eq!(peace_allowed(&req(&w, "AAA", "ZZZ", &[]), &cbs), Err(PeaceRefusal::NotAtWar));
}

#[test]
fn separate_peace_with_one_member() {
    let cbs = cbs();
    let w = war(vec![goal("conquest", "AAA", "DDD"), goal("conquest", "ABB", "DEE"), goal("conquest", "ABB", "DDD")]);
    // needs a leader on one side: AAA (leader) with DEE (member) works, both orders
    assert_eq!(peace_allowed(&req(&w, "AAA", "DEE", &[]), &cbs), Ok(PeaceScope::Separate("DEE".into())));
    assert_eq!(peace_allowed(&req(&w, "DEE", "AAA", &[]), &cbs), Ok(PeaceScope::Separate("DEE".into())));
    // two members, no leader on either side
    assert_eq!(peace_allowed(&req(&w, "ABB", "DEE", &[]), &cbs), Err(PeaceRefusal::NeitherIsLeader));
    // DDD (leader) with ABB (member): ABB has two goals, taking one only leaves some of them
    assert_eq!(peace_allowed(&req(&w, "DDD", "ABB", &[1]), &cbs), Err(PeaceRefusal::LeftoverGoals));
    // all or none of them is fine
    assert_eq!(peace_allowed(&req(&w, "DDD", "ABB", &[1, 2]), &cbs), Ok(PeaceScope::Separate("ABB".into())));
    assert_eq!(peace_allowed(&req(&w, "DDD", "ABB", &[]), &cbs), Ok(PeaceScope::Separate("ABB".into())));
    // crisis wars: leaders only
    let crisis = PeaceRequest { crisis: true, ..req(&w, "AAA", "DEE", &[]) };
    assert_eq!(peace_allowed(&crisis, &cbs), Err(PeaceRefusal::NoSeparatePeaceCrisis));
    let crisis_leaders = PeaceRequest { crisis: true, ..req(&w, "AAA", "DDD", &[]) };
    assert_eq!(peace_allowed(&crisis_leaders, &cbs), Ok(PeaceScope::WholeWar));
}

#[test]
fn separate_peace_is_limited_by_a_status_quo_goal() {
    let r = rules();
    let cbs = cbs();
    // the defenders hold a status quo goal: DDD vs AAA
    let w = war(vec![goal("quo", "DDD", "AAA"), goal("conquest", "AAA", "DEE"), goal("annex_cb", "AAA", "DEE")]);
    // conquest between AAA and the member DEE is not an annexation: refused (LeftoverGoals is
    // tested first when it applies, so take both of AAA's goals... AAA is a leader, DEE a member)
    assert_eq!(peace_allowed(&req(&w, "AAA", "DEE", &[1]), &cbs), Err(PeaceRefusal::StatusQuoLimit));
    // only the annexation between the two
    let w2 = war(vec![goal("quo", "DDD", "AAA"), goal("annex_cb", "AAA", "DEE")]);
    let out = plan_peace(&r, &req(&w2, "AAA", "DEE", &[1]), &cbs, &|_| true, &|_| 0.0).unwrap();
    assert_eq!((out.scope, out.enforced, out.leaving), (PeaceScope::Separate("DEE".into()), vec![1], vec!["DEE".to_string()]));
    // without the status quo goal the conquest is fine
    let w3 = war(vec![goal("conquest", "AAA", "DEE")]);
    assert!(peace_allowed(&req(&w3, "AAA", "DEE", &[0]), &cbs).is_ok());
}

fn ai_input() -> PeaceAnswerInput {
    PeaceAnswerInput {
        m_is_leader: true,
        o_is_leader: true,
        m_score: 0.0,
        o_score: 0.0,
        direction: 0.0,
        months_of_war: 0.0,
        m_exhaustion: 0.0,
        land: 1.0,
        naval: 1.0,
        offer_cost: 0.0,
        own_goals: 0,
        own_granted: 0,
        annexes_m: false,
        has_enemy_obligatory: false,
        has_own_obligatory: false,
        m_infamy: 0.0,
        m_cannot_add_goals: false,
        m_holds_cb: false,
    }
}

#[test]
fn the_ai_answers_peace_offers() {
    let r = rules();
    // willingness: base 20 and time 30 months of stubbornness for a fresh war
    let i = ai_input();
    assert_eq!(ai_willingness(&r, &i), -50.0);
    // month 1 with score 0: a white peace is refused, with or without goals of its own
    assert!(!ai_accepts_peace(&r, &i));
    assert!(!ai_accepts_peace(&r, &PeaceAnswerInput { months_of_war: 1.0, own_goals: 1, ..i }));
    // a demand that annexes the country: 100 points less, refused even at a high enemy score
    let annex = PeaceAnswerInput { annexes_m: true, offer_cost: 85.0, o_score: 90.0, own_goals: 1, ..i };
    assert_eq!(ai_willingness(&r, &annex), -150.0);
    assert!(!ai_accepts_peace(&r, &annex));
    // ... but at an enemy score of 100 anything up to 101 points is signed
    assert!(ai_accepts_peace(&r, &PeaceAnswerInput { o_score: 100.0, ..annex }));
    // an obligatory goal of the enemy costs the same 100; an own one gives 50
    assert_eq!(ai_willingness(&r, &PeaceAnswerInput { has_enemy_obligatory: true, ..i }), -150.0);
    assert_eq!(ai_willingness(&r, &PeaceAnswerInput { has_own_obligatory: true, ..i }), 0.0);
    // a leader does not let a member's enemy keep an obligatory goal for nothing
    assert!(!ai_accepts_peace(&r, &PeaceAnswerInput { has_enemy_obligatory: true, o_is_leader: false, o_score: 100.0, ..i }));
    // exhausted and long war: willing. Time counts 0.6 a month past month 30, twice that without goals.
    let tired = PeaceAnswerInput { m_exhaustion: 100.0, months_of_war: 60.0, ..i };
    assert_eq!(ai_willingness(&r, &tired), 50.0 + 36.0 - 20.0);
    assert_eq!(ai_willingness(&r, &PeaceAnswerInput { own_goals: 1, ..tired }), 50.0 + 18.0 - 20.0);
    assert!(ai_accepts_peace(&r, &tired));
    // every term in whole points
    assert_eq!(ai_willingness(&r, &PeaceAnswerInput { m_exhaustion: 3.9, months_of_war: 31.0, own_goals: 1, ..i }), 1.0 - 20.0);
    // an allied (non-leader) member counts base x2 and ignores a time past month 30, direction and force
    let ally = PeaceAnswerInput { m_is_leader: false, ..tired };
    assert_eq!(ai_willingness(&r, &ally), 50.0 - 40.0);
    assert_eq!(ai_willingness(&r, &PeaceAnswerInput { months_of_war: 10.0, m_exhaustion: 0.0, ..ally }), -20.0 - 40.0);
    // direction: winning raises it x5; losing ground makes willing
    assert_eq!(ai_willingness(&r, &PeaceAnswerInput { direction: 2.0, ..tired }), ai_willingness(&r, &tired) - 10.0);
    assert_eq!(ai_willingness(&r, &PeaceAnswerInput { direction: -7.0, ..tired }), ai_willingness(&r, &tired) + 7.0);
    // force: the land ratio weighs 0.75 (cap 3), the naval one 0.25 (0.2..2)
    assert_eq!(ai_willingness(&r, &PeaceAnswerInput { land: 5.0, naval: 9.0, ..i }), -50.0 - (0.3f64 * 100.0 * (0.75 * 3.0 + 0.25 * 2.0 - 1.0)).trunc());
    assert_eq!(ai_willingness(&r, &PeaceAnswerInput { land: 0.0, naval: 0.0, ..i }), -50.0 - (0.3f64 * 100.0 * (0.25 * 0.2 - 1.0)).trunc());
    // a priced offer: accepted when cost <= W / 2 + the other's score (W = 66, half 33)
    let ok = PeaceAnswerInput { offer_cost: 43.0, o_score: 10.0, ..tired };
    assert!(ai_accepts_peace(&r, &ok));
    assert!(!ai_accepts_peace(&r, &PeaceAnswerInput { offer_cost: 44.0, ..ok }));
    // a fresh war: the loser signs only what is 25 points below the winner's score
    assert!(ai_accepts_peace(&r, &PeaceAnswerInput { offer_cost: 35.0, o_score: 60.0, m_score: -60.0, ..i }));
    assert!(!ai_accepts_peace(&r, &PeaceAnswerInput { offer_cost: 36.0, o_score: 60.0, m_score: -60.0, ..i }));
    // the offer costs something and the other side's score is negative: refused
    assert!(!ai_accepts_peace(&r, &PeaceAnswerInput { o_score: -1.0, offer_cost: 1.0, ..tired }));
    // all of a leader's goals: accepted; all of a member's goals or a part: only when willing and worth the score
    let all = PeaceAnswerInput { own_goals: 2, own_granted: 2, offer_cost: 80.0, ..i };
    assert!(ai_accepts_peace(&r, &all));
    assert!(!ai_accepts_peace(&r, &PeaceAnswerInput { own_granted: 1, ..all }));
    let part = PeaceAnswerInput { own_goals: 2, own_granted: 1, offer_cost: 10.0, m_score: 30.0, ..tired };
    assert_eq!(ai_willingness(&r, &part), 48.0);
    assert!(ai_accepts_peace(&r, &part), "it gets 10, its score less half its willingness is 6");
    assert!(ai_accepts_peace(&r, &PeaceAnswerInput { offer_cost: 6.0, ..part }));
    assert!(!ai_accepts_peace(&r, &PeaceAnswerInput { offer_cost: 5.0, ..part }));
    // white peace, a winning leader that wants nothing: taken once its score is above 50
    assert!(ai_accepts_peace(&r, &PeaceAnswerInput { m_score: 60.0, o_score: -60.0, ..i }));
    assert!(!ai_accepts_peace(&r, &PeaceAnswerInput { m_score: 50.0, o_score: -50.0, ..i }));
    assert!(!ai_accepts_peace(&r, &PeaceAnswerInput { m_score: 60.0, o_score: -60.0, own_goals: 1, ..i }), "not while it still wants a goal");
    // white peace in a long war that goes nowhere: an infamous leader without a casus belli in hand takes it
    let stuck = PeaceAnswerInput { months_of_war: 31.0, land: 3.0, naval: 2.0, ..i };
    assert!(ai_willingness(&r, &stuck) < 1.0 && !ai_accepts_peace(&r, &stuck));
    assert!(ai_accepts_peace(&r, &PeaceAnswerInput { m_infamy: 11.0, ..stuck }));
    assert!(ai_accepts_peace(&r, &PeaceAnswerInput { m_cannot_add_goals: true, ..stuck }));
    assert!(!ai_accepts_peace(&r, &PeaceAnswerInput { m_infamy: 11.0, m_holds_cb: true, ..stuck }));
    assert!(!ai_accepts_peace(&r, &PeaceAnswerInput { m_infamy: 11.0, months_of_war: 30.0, ..stuck }));
    // white peace for the losing side that still has a goal: only when the enemy's score covers its reluctance
    assert!(ai_accepts_peace(&r, &PeaceAnswerInput { own_goals: 1, o_score: 25.0, m_score: -25.0, ..i }));
    assert!(!ai_accepts_peace(&r, &PeaceAnswerInput { own_goals: 1, o_score: 24.0, m_score: -24.0, ..i }));
}

#[test]
fn own_goals_are_counted_within_a_hundred_points() {
    let r = rules();
    // (cost, selected): the third goal lies past 100 points and is not selected, so it is not wanted
    assert_eq!(count_own_goals(&r, &[(40.0, false), (40.0, true), (40.0, false)]), (2, 1));
    assert_eq!(count_own_goals(&r, &[(40.0, false), (40.0, true), (40.0, true)]), (3, 2));
    // an annexation alone (85) is wanted; a goal that costs the full 100 is not, unless selected
    assert_eq!(count_own_goals(&r, &[(85.0, false)]), (1, 0));
    assert_eq!(count_own_goals(&r, &[(100.0, false)]), (0, 0));
    assert_eq!(count_own_goals(&r, &[(100.0, true)]), (1, 1));
    assert_eq!(count_own_goals(&r, &[]), (0, 0));
}

#[test]
fn direction_is_the_score_change_of_a_three_month_window() {
    let r = rules();
    let mut d = Direction::default();
    // first sight: the window starts, direction 0
    update_direction(&r, &mut d, 0.0, Date::new(1861, 7, 1));
    assert_eq!((d.value, d.next), (0.0, Some(Date::new(1861, 10, 1))));
    update_direction(&r, &mut d, 12.7, Date::new(1861, 8, 15));
    update_direction(&r, &mut d, 12.7, Date::new(1861, 10, 1));
    assert_eq!(d.value, 0.0, "nothing inside the window, the end day included");
    update_direction(&r, &mut d, 12.7, Date::new(1861, 10, 2));
    assert_eq!((d.value, d.last_score, d.next), (12.0, 12.0, Some(Date::new(1862, 1, 2))));
    update_direction(&r, &mut d, 5.0, Date::new(1862, 1, 3));
    assert_eq!(d.value, -7.0);
}

#[test]
fn the_force_ratio_counts_other_wars_at_half() {
    let r = rules();
    // even sides, no other war: the enemy side is counted once more at half among M's enemies
    assert!((force_ratio(&r, 10.0, 10.0, 10.0, 0.0) - 11.5 / 16.5).abs() < 1e-9);
    // O is busy with a third country of 20: M's side looks stronger
    assert!((force_ratio(&r, 10.0, 10.0, 10.0, 20.0) - 21.5 / 16.5).abs() < 1e-9);
    // nobody has anything: each sum starts at 1
    assert!((force_ratio(&r, 0.0, 0.0, 0.0, 0.0) - 1.0).abs() < 1e-9);
}

#[test]
fn declining_a_good_peace_only() {
    let r = rules();
    // far ahead and the AI would sign: costs
    let p = decline_penalty(&r, 60.0, true, false).unwrap();
    assert_eq!((p.exhaustion, p.militancy), (5.0, 1.0));
    // far ahead but the AI would not sign: free
    assert!(decline_penalty(&r, 60.0, false, false).is_none());
    // a weak offer: free
    assert!(decline_penalty(&r, 30.0, true, false).is_none());
    // the flag forces it
    assert!(decline_penalty(&r, 0.0, false, true).is_some());
}

#[test]
fn war_exhaustion_every_month() {
    let r = rules();
    let at_war = ExhaustionInput { at_war: true, current: 10.0, ..Default::default() };
    // 74 % occupied: +3.7, as the owner's save showed (+3.69)
    let occ = ExhaustionInput { occupied_share: 0.74, ..at_war };
    assert!((exhaustion_change(&r, &occ) - 3.7).abs() < 1e-9);
    // blockade half: +0.25; both add up
    let both = ExhaustionInput { occupied_share: 0.5, blockade_share: 0.5, ..at_war };
    assert!((exhaustion_monthly(&r, &both) - (10.0 + 2.5 + 0.25)).abs() < 1e-9);
    // fully occupied adds at most 5 and exhaustion stops at 100
    let full = ExhaustionInput { occupied_share: 1.0, current: 98.0, ..at_war };
    assert_eq!(exhaustion_change(&r, &full), 5.0);
    assert_eq!(exhaustion_monthly(&r, &full), 100.0);
    // modifier cap
    assert_eq!(exhaustion_monthly(&r, &ExhaustionInput { modifier_max: Some(20.0), occupied_share: 1.0, current: 19.0, ..at_war }), 20.0);
    // peace: -1 a month, never above 50 afterwards
    let peace = ExhaustionInput { at_war: false, current: 80.0, ..Default::default() };
    assert_eq!(exhaustion_monthly(&r, &peace), 50.0);
    assert_eq!(exhaustion_monthly(&r, &ExhaustionInput { current: 30.0, ..peace }), 29.0);
    assert_eq!(exhaustion_monthly(&r, &ExhaustionInput { current: 0.5, ..peace }), 0.0);
    assert_eq!(battle_exhaustion(&r, 0.5), 1.5);
}

#[test]
fn the_ai_adds_goals_as_the_war_drags_on() {
    let r = rules();
    assert_eq!(ai_goal_threshold(&r, 1, false, false), 15.0);
    assert_eq!(ai_goal_threshold(&r, 3, false, false), 45.0);
    assert_eq!(ai_goal_threshold(&r, 3, true, false), 24.0);
    assert_eq!(ai_goal_threshold(&r, 0, false, true), 5.0);
    assert_eq!(ai_goal_threshold(&r, 0, false, false), 2.0);
    let base = AiGoalInput { exhaustion: 10.0, max_exhaustion: 100.0, land_ratio: 1.5, score: 20.0, own_goals: 1, great_war: false, score_above_demanded: false };
    assert!(ai_wants_goal(&r, &base));
    // not enough score for a second goal, yet enough in a great war
    assert!(!ai_wants_goal(&r, &AiGoalInput { own_goals: 2, ..base }));
    assert!(ai_wants_goal(&r, &AiGoalInput { own_goals: 2, great_war: true, ..base }));
    // exhausted, or not ahead on land
    assert!(!ai_wants_goal(&r, &AiGoalInput { exhaustion: 30.0, ..base }));
    assert!(!ai_wants_goal(&r, &AiGoalInput { land_ratio: 1.0, ..base }));
    assert_eq!(ai_goal_infamy_ceiling(&r), 23.0);
    assert!(ai_goal_value_ok(&r, 21.0) && !ai_goal_value_ok(&r, 20.0));
}
