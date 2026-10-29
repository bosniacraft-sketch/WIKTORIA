//! Army AI decisions on synthetic fixtures.

use std::collections::{BTreeMap, BTreeSet};
use wiktoria_simulation::military::ai::*;
use wiktoria_simulation::military::movement::MoveMap;
use wiktoria_simulation::military::war::War;

fn s(x: &str) -> String {
    x.to_string()
}

fn rules() -> AiRules {
    AiRules {
        attack_limit: 0.9,
        org_floor: 0.25,
        siege_men_per_regiment: 3.0,
        own_lift_factor: 0.5,
        goal_factor: 0.2,
        enemy_army_factor: 0.5,
        mobilise_ratio: 1.5,
        area_size_cap: 10.0,
        area_capital: 50.0,
        area_goal: 250.0,
        blockade_ratio: 3.0,
        blockade_max_fleet: 12,
        siege_min: 3.0,
        siege_max: 10.0,
        rebel_tag: s("REB"),
        power_experience: 0.01,
        power_weak_strength: 0.1,
        power_weak_org: 1.0,
        power_weak_factor: 0.5,
        safe_step_ratio: 1.2,
        safe_step_min_friends: 0.001,
        recover_below: 0.34,
        recover_until: 0.75,
        recover_homeland_lost: 0.3,
        recover_capital: 15.0,
        recover_spread: 10.0,
        recover_spread_steps: 5,
        recover_hostile_owner: 5.0,
        recover_hostile_holder: 20.0,
        regroup_share: 0.33,
        regroup_max_neighbours: 20,
        needed_min: 3.0,
        needed_empty: 1.0,
        needed_support: 2.0,
        needed_others_share: 0.1,
        post_regiments: 10.0,
        post_spacing: 3,
        post_days: 15,
        post_hostile_relation: -50.0,
        post_hostile_factor: 2.0,
        gather_share: 0.5,
        gather_steps: 6,
        off: AiOff::default(),
    }
}

/// A line of provinces 1 - 2 - 3 - 4 - 5, each border 2 long.
fn line() -> MoveMap {
    let mut adj = vec![BTreeSet::new(); 6];
    for a in 1..5usize {
        adj[a].insert((a + 1) as u16);
        adj[a + 1].insert(a as u16);
    }
    MoveMap::from_adjacency(&adj, &[], &|_, _| 2.0)
}

fn prov(owner: &str, controller: &str) -> AiProvince {
    AiProvince { owner: Some(s(owner)), controller: Some(s(controller)), ..Default::default() }
}

/// A (owner of 1..=3) against B (4, 5).
fn provinces() -> BTreeMap<u16, AiProvince> {
    BTreeMap::from([(1, prov("A", "A")), (2, prov("A", "A")), (3, prov("A", "A")), (4, prov("B", "B")), (5, prov("B", "B"))])
}

fn army(id: u64, owner: &str, at: u16, strength: f64) -> AiArmy {
    // Full, rested regiments of 3000 men: the strength measure is their number.
    AiArmy { id, owner: s(owner), location: at, strength, regiments: (strength / 3.0).ceil() as usize, org: 1.0, power: (strength / 3.0).ceil(), ..Default::default() }
}

fn sides() -> Sides {
    let w = War { active: true, attackers: vec![s("A")], defenders: vec![s("B")], ..Default::default() };
    Sides::from_wars(&[w], "REB")
}

fn supply(_: &str, _: u16) -> i64 {
    30
}

fn run(provs: &BTreeMap<u16, AiProvince>, armies: &[AiArmy], tag: &str) -> Vec<(u64, u16)> {
    let (map, sd) = (line(), sides());
    let sup: &dyn Fn(&str, u16) -> i64 = &supply;
    let w = AiWorld { map: &map, provinces: provs, armies, sides: &sd, supply: sup };
    plan_armies(&rules(), &w, tag)
}

#[test]
fn rules_load_from_engine_rules_and_defines() {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/engine_rules.json");
    let d = wiktoria_data::defines::Defines::parse("defines = { military = { SIEGE_BRIGADES_MIN = 3, SIEGE_BRIGADES_MAX = 10, }, }").unwrap();
    let m = wiktoria_data::engine_rules::EngineRules::from_json(&std::fs::read_to_string(p).unwrap()).unwrap();
    let r = AiRules::new(&m, &d).unwrap();
    assert_eq!((r.siege_min, r.siege_max, r.mobilise_ratio), (3.0, 10.0, 1.5));
    assert_eq!((r.safe_step_ratio, r.recover_below, r.recover_until, r.regroup_share, r.needed_min), (1.2, 0.34, 0.75, 0.33, 3.0));
    assert!(r.enemy_army_factor > 0.0);
}

// (a) attack enemy armies only when expecting to win, otherwise besiege enemy provinces.

#[test]
fn a_weaker_army_besieges_instead_of_attacking() {
    let armies = [army(1, "A", 3, 9.0), army(2, "B", 4, 30.0)];
    let mut provs = provinces();
    provs.get_mut(&5).unwrap().controller = Some(s("B"));
    // Province 4 holds a stronger enemy army: not attacked. 5 is behind it and the only other target.
    let orders = run(&provs, &armies, "A");
    assert!(!orders.contains(&(1, 4)), "{orders:?}");
}

#[test]
fn a_weaker_army_takes_an_empty_enemy_province() {
    let armies = [army(1, "A", 3, 9.0), army(2, "B", 4, 30.0)];
    let mut provs = provinces();
    provs.insert(6, prov("B", "B"));
    let map = {
        let mut adj = vec![BTreeSet::new(); 7];
        for (a, b) in [(1, 2), (2, 3), (3, 4), (4, 5), (3, 6)] {
            adj[a].insert(b as u16);
            adj[b].insert(a as u16);
        }
        MoveMap::from_adjacency(&adj, &[], &|_, _| 2.0)
    };
    let sd = sides();
    let sup: &dyn Fn(&str, u16) -> i64 = &supply;
    let w = AiWorld { map: &map, provinces: &provs, armies: &armies, sides: &sd, supply: sup };
    assert_eq!(plan_armies(&rules(), &w, "A"), vec![(1, 6)]);
}

#[test]
fn a_stronger_army_attacks_the_enemy_army() {
    let armies = [army(1, "A", 3, 30.0), army(2, "B", 4, 9.0)];
    assert_eq!(run(&provinces(), &armies, "A"), vec![(1, 4)]);
}

#[test]
fn strength_test_is_about_one_point_one_to_one() {
    let r = rules();
    // Equal strength and regiments: R = 1.0, above 0.9, no attack.
    assert!(!attack_ok(&r, 1.0, (30.0, 10), (0.0, 0), (30.0, 10)));
    // 1.2 to 1: R about 0.83, attack.
    assert!(attack_ok(&r, 1.0, (36.0, 12), (0.0, 0), (30.0, 10)));
    // Tired army at 80 percent organisation needs more (limit 0.7).
    assert!(!attack_ok(&r, 0.8, (36.0, 12), (0.0, 0), (30.0, 10)));
    // Below the floor never.
    assert!(!attack_ok(&r, 0.2, (300.0, 100), (0.0, 0), (1.0, 1)));
}

#[test]
fn siege_takes_three_to_ten_regiments() {
    let r = rules();
    assert!(siege_room(&r, 30, 0, 3));
    assert!(!siege_room(&r, 100, 10, 3), "at the maximum no more");
    // From the minimum on the supply limit must carry all: 3 x (4 + 3) = 21 over limit 18.
    assert!(!siege_room(&r, 18, 4, 3));
    assert!(siege_room(&r, 30, 4, 3));
}

#[test]
fn siege_is_not_overfilled_by_orders() {
    // Twelve armies of 3 regiments in 3, one enemy-held province 4: only so many go there as the maximum allows.
    let armies: Vec<AiArmy> = (1..=5).map(|i| army(i, "A", 3, 9.0)).collect();
    let mut provs = provinces();
    provs.remove(&5);
    let orders = run(&provs, &armies, "A");
    let regs: usize = orders.iter().filter(|(_, p)| *p == 4).count() * 3;
    assert!((3..=12).contains(&regs), "{orders:?}");
    assert!(regs < 15, "{orders:?}");
}

// (b) lift sieges of own territory first.

#[test]
fn own_occupied_province_is_lifted_before_a_nearer_enemy_province() {
    let mut provs = provinces();
    provs.get_mut(&1).unwrap().controller = Some(s("B"));
    // From 3: enemy 4 is 2 away, own occupied 1 is 4 away.
    let armies = [army(1, "A", 3, 12.0)];
    assert_eq!(run(&provs, &armies, "A"), vec![(1, 1)]);
}

#[test]
fn enemy_province_is_taken_when_nothing_of_ours_is_held() {
    let armies = [army(1, "A", 3, 12.0)];
    assert_eq!(run(&provinces(), &armies, "A"), vec![(1, 4)]);
}

// (c) push toward the enemy, no back and forth.

#[test]
fn army_keeps_a_valid_target() {
    let mut a = army(1, "A", 2, 12.0);
    a.target = Some(5);
    // 3 is not enemy; 4 is nearer to the army than 5 but the target stays.
    assert_eq!(run(&provinces(), &[a], "A"), vec![]);
}

#[test]
fn invalid_target_is_replaced() {
    let mut a = army(1, "A", 3, 12.0);
    a.target = Some(2); // our own quiet province: not a target any more
    assert_eq!(run(&provinces(), &[a], "A"), vec![(1, 4)]);
}

#[test]
fn orders_are_stable_from_day_to_day() {
    let mut a = army(1, "A", 3, 12.0);
    let first = run(&provinces(), &[a.clone()], "A");
    a.target = Some(first[0].1);
    for _ in 0..5 {
        assert!(run(&provinces(), &[a.clone()], "A").is_empty());
    }
}

#[test]
fn armies_never_head_for_the_province_they_came_from() {
    // After the first order is given and the army stands in 4 besieging, no new order sends it back to 3.
    let mut a = army(1, "A", 4, 12.0);
    a.sieging = true;
    assert!(run(&provinces(), &[a], "A").is_empty());
}

#[test]
fn merging_never_goes_both_ways() {
    let (map, sd) = (line(), sides());
    let sup: &dyn Fn(&str, u16) -> i64 = &supply;
    let armies = [army(1, "A", 1, 9.0), army(2, "A", 2, 9.0)];
    let provs = provinces();
    let w = AiWorld { map: &map, provinces: &provs, armies: &armies, sides: &sd, supply: sup };
    let orders = plan_stacks(&w, "A");
    assert_eq!(orders.len(), 1, "{orders:?}");
}

// (d) stacks stay near the supply limit, no fixed cap.

#[test]
fn stacks_merge_up_to_the_supply_limit() {
    let (map, sd) = (line(), sides());
    let provs = provinces();
    let armies = [army(1, "A", 1, 12.0), army(2, "A", 2, 15.0)];
    let sup30: &dyn Fn(&str, u16) -> i64 = &|_, _| 30;
    let w = AiWorld { map: &map, provinces: &provs, armies: &armies, sides: &sd, supply: sup30 };
    assert_eq!(plan_stacks(&w, "A"), vec![(1, 2)], "27 fits in 30");
    let sup24: &dyn Fn(&str, u16) -> i64 = &|_, _| 24;
    let w = AiWorld { map: &map, provinces: &provs, armies: &armies, sides: &sd, supply: sup24 };
    assert!(plan_stacks(&w, "A").is_empty(), "27 does not fit in 24");
    let sup60: &dyn Fn(&str, u16) -> i64 = &|_, _| 60;
    let w = AiWorld { map: &map, provinces: &provs, armies: &armies, sides: &sd, supply: sup60 };
    assert_eq!(plan_stacks(&w, "A").len(), 1, "a richer province carries a bigger stack");
}

// Mobilisation, areas, admiral.

#[test]
fn mobilise_under_one_and_a_half_times_the_enemy() {
    let r = rules();
    let sd = sides();
    let at = |a: f64, b: f64| mobilise_decisions(&r, &sd, &BTreeMap::from([(s("A"), a), (s("B"), b)]));
    // A has 100, B 80: 100 < 1.5 x 80, mobilise.
    assert_eq!(at(100.0, 80.0), vec![(s("A"), true), (s("B"), true)]);
    // A has 200 against 100: no.
    assert!(!at(200.0, 100.0)[0].1);
    // Exactly 1.5 times: no.
    assert!(!at(150.0, 100.0)[0].1);
    // At peace nobody mobilises.
    let peace = mobilise_decisions(&r, &Sides::default(), &BTreeMap::from([(s("A"), 1.0), (s("B"), 100.0)]));
    assert!(peace.iter().all(|(_, m)| !m));
}

#[test]
fn allies_count_on_the_side() {
    let r = rules();
    let w = War { active: true, attackers: vec![s("A"), s("C")], defenders: vec![s("B")], ..Default::default() };
    let sd = Sides::from_wars(&[w], "REB");
    let m = mobilise_decisions(&r, &sd, &BTreeMap::from([(s("A"), 100.0), (s("B"), 100.0), (s("C"), 100.0)]));
    assert!(!m[0].1, "200 against 100");
    assert!(m[1].1, "100 against 200");
}

#[test]
fn fronts_and_area_values() {
    let r = rules();
    let mut provs = provinces();
    provs.get_mut(&5).unwrap().capital = true;
    provs.get_mut(&4).unwrap().war_goal = true;
    let (map, sd) = (line(), sides());
    let sup: &dyn Fn(&str, u16) -> i64 = &supply;
    let w = AiWorld { map: &map, provinces: &provs, armies: &[], sides: &sd, supply: sup };
    let all = areas(&map, &provs);
    assert_eq!(all.len(), 2);
    assert!(all.iter().all(|a| is_front(&w, &all, a)));
    let b = all.iter().find(|a| a.controller.as_deref() == Some("B")).unwrap();
    assert_eq!(area_value(&r, &w, b), 2.0 + 50.0 + 250.0);
}

#[test]
fn blockade_needs_three_to_one() {
    let r = rules();
    assert!(blockade_allowed(&r, 31, 10, 5, false));
    assert!(!blockade_allowed(&r, 30, 10, 5, false));
    assert!(!blockade_allowed(&r, 100, 10, 13, false), "one enemy fleet above twelve");
    assert!(blockade_allowed(&r, 30, 10, 5, true), "an existing one goes on under half");
}

// The AI's strength measure.

fn reg(strength: f64, organisation: f64, experience: f64) -> AiRegiment {
    AiRegiment { strength, max_strength: 3.0, organisation, max_organisation: 30.0, experience }
}

#[test]
fn a_regiment_counts_strength_share_times_organisation_share() {
    let r = rules();
    assert_eq!(regiment_power(&r, &reg(3.0, 30.0, 0.0)), 1.0);
    assert_eq!(regiment_power(&r, &reg(1.5, 30.0, 0.0)), 0.5);
    assert_eq!(regiment_power(&r, &reg(3.0, 15.0, 0.0)), 0.5, "half organisation counts half");
    assert!((regiment_power(&r, &reg(3.0, 30.0, 50.0)) - 1.5).abs() < 1e-9, "experience adds");
    // Under the strength floor or under 1 organisation the value is halved.
    assert!((regiment_power(&r, &reg(0.06, 30.0, 0.0)) - 0.01).abs() < 1e-9);
    assert!((regiment_power(&r, &reg(3.0, 0.6, 0.0)) - 0.01).abs() < 1e-9);
    assert_eq!(stack_power(&r, &[reg(3.0, 30.0, 0.0), reg(3.0, 15.0, 0.0)]), 1.5);
}

#[test]
fn a_worn_stack_does_not_count_as_a_fresh_one() {
    // Ten regiments at 40 percent strength measure against six fresh ones: 4 to 6, no attack.
    let mut worn = army(1, "A", 3, 30.0);
    worn.power = 4.0;
    let armies = [worn, army(2, "B", 4, 18.0)];
    assert!(!run(&provinces(), &armies, "A").contains(&(1, 4)));
}

// The safe next step.

fn marching(mut a: AiArmy, next: u16, days: f64) -> AiArmy {
    (a.next, a.arrive, a.destination) = (Some(next), days, Some(next));
    a
}

fn world_check(armies: &[AiArmy], check: &dyn Fn(&AiWorld) -> bool) -> bool {
    let (map, sd, provs) = (line(), sides(), provinces());
    let sup: &dyn Fn(&str, u16) -> i64 = &supply;
    check(&AiWorld { map: &map, provinces: &provs, armies, sides: &sd, supply: sup })
}

#[test]
fn next_step_is_unsafe_from_one_point_two_to_one() {
    let r = rules();
    let me = marching(army(1, "A", 3, 30.0), 4, 5.0);
    let safe = |armies: &[AiArmy]| world_check(armies, &|w| safe_step(&r, w, &armies[0], 4, 5.0));
    assert!(safe(std::slice::from_ref(&me)), "nobody there");
    assert!(safe(&[me.clone(), army(2, "B", 4, 33.0)]), "11 against 10");
    assert!(!safe(&[me.clone(), army(2, "B", 4, 36.0)]), "12 against 10");
    // Friends there or marching there count on our side, hostiles marching in on theirs.
    assert!(safe(&[me.clone(), army(2, "B", 4, 36.0), marching(army(3, "A", 3, 9.0), 4, 9.0)]));
    assert!(!safe(&[me.clone(), army(2, "B", 4, 21.0), marching(army(3, "B", 5, 15.0), 4, 2.0)]));
    // A hostile that marches out before we arrive is not counted, one that leaves later is.
    assert!(safe(&[me.clone(), marching(army(2, "B", 4, 60.0), 5, 3.0)]));
    assert!(!safe(&[me.clone(), marching(army(2, "B", 4, 60.0), 5, 7.0)]));
    // A retreating hostile is not counted.
    let mut beaten = army(2, "B", 4, 60.0);
    beaten.retreating = true;
    assert!(safe(&[me, beaten]));
}

#[test]
fn an_unsafe_march_is_cancelled() {
    let r = rules();
    let armies = [marching(army(1, "A", 3, 30.0), 4, 5.0), marching(army(2, "A", 2, 30.0), 3, 5.0), army(3, "B", 4, 60.0)];
    assert!(world_check(&armies, &|w| unsafe_marches(&r, w, "A") == vec![1]));
}

// Sieges.

#[test]
fn the_excess_leaves_a_siege() {
    // Our own design: in every war a big stack leaves a detachment of the siege maximum.
    let r = rules();
    assert_eq!(siege_excess(&r, Some("REB"), 14), 4);
    assert_eq!(siege_excess(&r, Some("REB"), 10), 0);
    assert_eq!(siege_excess(&r, Some("B"), 14), 4);
}

/// Gathering at war (TST_MIL_AI_GATHER): A's small armies walk to the main army at 1 as far as its
/// province feeds them; one over half the main army's power keeps its own way.
#[test]
fn small_field_armies_join_the_main_army() {
    let r = rules();
    let armies = [army(1, "A", 1, 24.0), army(2, "A", 2, 3.0), army(3, "A", 3, 6.0), army(4, "A", 2, 15.0)];
    assert!(world_check(&armies, &|w| plan_gather(&r, w, "A") == vec![(2, 1)]), "army 3 does not fit the supply limit of 30; army 4 is over half the main army");
}

#[test]
fn a_besieging_or_resting_stack_takes_no_order() {
    let mut a = army(1, "A", 4, 30.0);
    a.sieging = true;
    assert!(run(&provinces(), &[a], "A").is_empty());
    let mut a = army(1, "A", 3, 30.0);
    a.resting = true;
    assert!(run(&provinces(), &[a], "A").is_empty());
}

// Recover.

/// A holds 1 to 3, B holds 4 and 5; 1 is A's capital.
fn recover_world(armies: &[AiArmy], provs: &BTreeMap<u16, AiProvince>, check: &dyn Fn(&AiWorld, &AreaIndex) -> bool) -> bool {
    let (map, sd) = (line(), sides());
    let sup: &dyn Fn(&str, u16) -> i64 = &supply;
    check(&AiWorld { map: &map, provinces: provs, armies, sides: &sd, supply: sup }, &AreaIndex::new(&map, provs))
}

#[test]
fn a_worn_stack_rests_in_a_quiet_own_province_until_it_is_rested() {
    let r = rules();
    let mut provs = provinces();
    provs.get_mut(&1).unwrap().capital = true;
    let mut worn = army(11, "A", 3, 30.0);
    worn.org = 0.3;
    // Province 3 touches the enemy (+5 owner, +20 holder), 2 does not: walk 2 + spread 30 beats 0 + 40 + 25;
    // the capital is 4 away with spread 20 and bonus 15: 9, the lowest.
    assert!(recover_world(&[worn.clone()], &provs, &|w, ar| recover_province(&r, w, ar, &worn) == Some(1)));
    assert!(recover_world(&[worn.clone()], &provs, &|w, ar| plan_recovery(&r, w, ar, "A").go == vec![(11, 1)]));
    // At 34 percent and above nobody goes.
    let mut tired = worn.clone();
    tired.org = 0.34;
    assert!(recover_world(&[tired.clone()], &provs, &|w, ar| plan_recovery(&r, w, ar, "A") == Recovery::default()));
    // A resting stack stays until 75 percent.
    let mut resting = worn.clone();
    (resting.resting, resting.org) = (true, 0.74);
    assert!(recover_world(&[resting.clone()], &provs, &|w, ar| plan_recovery(&r, w, ar, "A") == Recovery::default()));
    resting.org = 0.75;
    assert!(recover_world(&[resting], &provs, &|w, ar| plan_recovery(&r, w, ar, "A").done == vec![11]));
    // A besieging stack stays in its siege.
    let mut besieging = worn.clone();
    besieging.sieging = true;
    assert!(recover_world(&[besieging], &provs, &|w, ar| plan_recovery(&r, w, ar, "A").go.is_empty()));
}

#[test]
fn nobody_rests_with_a_third_of_the_homeland_occupied() {
    let r = rules();
    let mut provs = provinces();
    provs.get_mut(&1).unwrap().capital = true;
    assert!(recover_world(&[], &provs, &|w, ar| homeland_lost(w, ar, "A") == 0.0));
    // B holds province 3 of A's three: a third is lost.
    provs.get_mut(&3).unwrap().controller = Some(s("B"));
    assert!(recover_world(&[], &provs, &|w, ar| (homeland_lost(w, ar, "A") - 1.0 / 3.0).abs() < 1e-9));
    let mut worn = army(11, "A", 2, 30.0);
    worn.org = 0.2;
    assert!(recover_world(&[worn.clone()], &provs, &|w, ar| plan_recovery(&r, w, ar, "A").go.is_empty()));
    // A resting stack is called back.
    worn.resting = true;
    assert!(recover_world(&[worn], &provs, &|w, ar| plan_recovery(&r, w, ar, "A").done == vec![11]));
}

// Grouping.

#[test]
fn armies_standing_together_merge() {
    let r = rules();
    let merges = |armies: &[AiArmy]| {
        let out = std::cell::RefCell::new(Vec::new());
        world_check(armies, &|w| {
            *out.borrow_mut() = plan_merges(&r, w, "A");
            true
        });
        out.into_inner()
    };
    // The larger keeps, ties go to the lowest id; a marching army does not merge.
    assert_eq!(merges(&[army(1, "A", 2, 9.0), army(2, "A", 2, 12.0), army(3, "A", 2, 9.0)]), vec![(2, 1), (2, 3)]);
    assert_eq!(merges(&[army(1, "A", 2, 9.0), marching(army(2, "A", 2, 12.0), 3, 4.0)]), vec![]);
    // A besieging army keeps.
    let mut besieger = army(5, "A", 4, 6.0);
    besieger.sieging = true;
    assert_eq!(merges(&[army(1, "A", 4, 9.0), besieger]), vec![(5, 1)]);
    // Not over the supply limit of the province (30), not while a battle runs there.
    assert_eq!(merges(&[army(1, "A", 2, 18.0), army(2, "A", 2, 15.0)]), vec![]);
    let mut fighting = army(3, "B", 2, 9.0);
    (fighting.fighting, fighting.busy) = (true, true);
    assert_eq!(merges(&[army(1, "A", 2, 9.0), army(2, "A", 2, 9.0), fighting]), vec![]);
}

#[test]
fn rebel_held_land_is_no_place_to_merge() {
    let r = rules();
    let (map, sd, mut provs) = (line(), sides(), provinces());
    provs.get_mut(&2).unwrap().controller = Some(s("REB"));
    let sup: &dyn Fn(&str, u16) -> i64 = &supply;
    let armies = [army(1, "A", 2, 9.0), army(2, "A", 2, 9.0)];
    let w = AiWorld { map: &map, provinces: &provs, armies: &armies, sides: &sd, supply: sup };
    assert!(plan_merges(&r, &w, "A").is_empty());
}

#[test]
fn a_small_stack_waits_for_those_marching_to_it() {
    let r = rules();
    let wait = |armies: &[AiArmy]| world_check(armies, &|w| regroup_wait(&r, w, &armies[0]));
    // 3 regiments with 9 on their way: a quarter, it waits and takes no target.
    let (small, big) = (army(1, "A", 3, 9.0), marching(army(2, "A", 2, 27.0), 3, 2.0));
    assert!(wait(&[small.clone(), big.clone()]));
    assert!(run(&provinces(), &[small.clone(), big], "A").iter().all(|(id, _)| *id != 1));
    // With 6 coming it is a third: it goes on. Nobody coming: it goes on.
    assert!(!wait(&[small.clone(), marching(army(2, "A", 2, 18.0), 3, 2.0)]));
    assert!(!wait(&[small.clone(), army(2, "A", 2, 27.0)]));
    assert_eq!(run(&provinces(), &[small], "A"), vec![(1, 4)]);
}

// Needed regiments.

#[test]
fn an_area_needs_its_hostile_regiments_at_least_three() {
    let r = rules();
    let needed = |armies: &[AiArmy], tag: &str, entry: u16| {
        let n = std::cell::Cell::new(0);
        recover_world(armies, &provinces(), &|w, ar| {
            n.set(needed_regiments(&r, w, ar, tag, entry));
            true
        });
        n.get()
    };
    // Empty but hostile-owned: one front. Empty and our own: nothing.
    assert_eq!(needed(&[], "A", 4), 1);
    assert_eq!(needed(&[], "A", 1), 0);
    // One or two hostile regiments count as three; more count as they are.
    assert_eq!(needed(&[army(1, "B", 5, 6.0)], "A", 4), 3);
    assert_eq!(needed(&[army(1, "B", 5, 21.0), army(2, "B", 4, 9.0)], "A", 4), 10);
    // Our own regiments next door do not lower what stands inside.
    assert_eq!(needed(&[army(1, "B", 5, 21.0), army(3, "A", 3, 30.0)], "A", 4), 7);
}

#[test]
fn a_measuring_run_can_switch_rules_off_by_name() {
    assert_eq!(AiOff::parse("").unwrap(), AiOff::default());
    let off = AiOff::parse("merge, safe-step").unwrap();
    assert!(off.merge && off.safe_step && !off.recover && !off.regroup && !off.siege_split && !off.power);
    let all = AiOff::parse("all").unwrap();
    assert!(all.merge && all.recover && all.safe_step && all.regroup && all.siege_split && all.power);
    assert!(AiOff::parse("mergee").is_err());
    assert!(AiOff::parse("enemy-rank").unwrap().enemy_rank && all.enemy_rank);
    assert_eq!(AiOff::parse("power,enemy-rank").unwrap().names_off(), vec!["power", "enemy-rank"]);
    // With the strength measure off a regiment counts by its men alone.
    let mut r = rules();
    let tired = AiRegiment { strength: 1.5, max_strength: 3.0, organisation: 5.0, max_organisation: 20.0, experience: 0.0 };
    let with = regiment_power(&r, &tired);
    r.off.power = true;
    assert_eq!(regiment_power(&r, &tired), 0.5);
    assert!(with < 0.5);
}

// A beatable enemy army ranks with sieges (MIL_AI_002).

/// A holds 1 to 3 and 6, B holds 4 and 5; 6 hangs off 3 by a border 3 long, the others are 2 long.
/// From 3: the siege of 4 scores 2 / 2 = 1, the beatable B army in 6 scores 3 / 2 = 1.5 before its factor.
fn choose_between_siege_and_army(r: &AiRules) -> Vec<(u64, u16)> {
    let mut provs = provinces();
    provs.insert(6, prov("A", "A"));
    let map = {
        let mut adj = vec![BTreeSet::new(); 7];
        for (a, b) in [(1, 2), (2, 3), (3, 4), (4, 5), (3, 6)] {
            adj[a].insert(b as u16);
            adj[b].insert(a as u16);
        }
        MoveMap::from_adjacency(&adj, &[], &|a, b| if a == 6 || b == 6 { 3.0 } else { 2.0 })
    };
    let armies = [army(1, "A", 3, 30.0), army(2, "B", 6, 9.0)];
    let sd = sides();
    let sup: &dyn Fn(&str, u16) -> i64 = &supply;
    let w = AiWorld { map: &map, provinces: &provs, armies: &armies, sides: &sd, supply: sup };
    plan_armies(r, &w, "A")
}

#[test]
fn a_beatable_enemy_army_competes_with_a_siege_at_a_similar_distance() {
    let mut r = rules();
    // Factor 0.5: the army scores 0.75 against the siege's 1 and goes for the enemy army.
    assert_eq!(choose_between_siege_and_army(&r), vec![(1, 6)]);
    // Factor 1: the nearer siege wins on distance alone.
    r.enemy_army_factor = 1.0;
    assert_eq!(choose_between_siege_and_army(&r), vec![(1, 4)]);
    // The old ranking: sieges first whatever the factor.
    r.enemy_army_factor = 0.5;
    r.off.enemy_rank = true;
    assert_eq!(choose_between_siege_and_army(&r), vec![(1, 4)]);
}

#[test]
fn idle_warships_blockade_the_nearest_enemy_ports_when_strong_enough() {
    let r = rules();
    let w = War { active: true, attackers: vec![s("A")], defenders: vec![s("B")], ..War::default() };
    let sd = Sides::from_wars(&[w], "REB");
    let fleet = |id: u64, owner: &str, at: u16, warships: usize, idle: bool| AiFleet { id, owner: s(owner), location: at, warships, idle };
    // B's ports lie on the seas 20, 21 and 22; C is not in the war.
    let ports = vec![AiPort { province: 1, sea: 20, controller: s("B") }, AiPort { province: 2, sea: 21, controller: s("B") }, AiPort { province: 3, sea: 22, controller: s("B") }, AiPort { province: 4, sea: 23, controller: s("C") }];
    // From A's home sea 10 the seas lie at 1, 2, 3 and 0.5.
    let reach = |_: u16| BTreeMap::from([(20, 1.0), (21, 2.0), (22, 3.0), (23, 0.5)]);
    let tags = vec![s("A"), s("B")];
    // A: 9 warships in three fleets, one already off port 22, a busy one and a transport-only one; B: 2.
    let fleets = vec![fleet(1, "A", 10, 3, true), fleet(2, "A", 22, 3, true), fleet(3, "A", 10, 3, false), fleet(4, "A", 10, 0, true), fleet(5, "A", 10, 1, true), fleet(9, "B", 30, 2, true)];
    let plan = plan_blockades(&r, &sd, &tags, &fleets, &ports, &reach);
    // 10 warships against 2 (over 3 to 1): fleet 1 takes the nearest free sea, fleet 5 the next; 22 is held.
    assert_eq!(plan, vec![(1, 20), (5, 21)]);
    // B, outnumbered, blockades nothing; nor does A once B has more than a third of its count.
    let even = vec![fleet(1, "A", 10, 3, true), fleet(9, "B", 30, 2, true)];
    assert!(plan_blockades(&r, &sd, &tags, &even, &ports, &reach).is_empty());
    // Not at war: no blockade.
    assert!(plan_blockades(&r, &Sides::default(), &tags, &fleets, &ports, &reach).is_empty());
}

#[test]
fn an_army_with_no_land_target_lands_over_the_sea() {
    // 1 - 2 (A) | 3 sea | 4 - 5 (B).
    let mut map = line();
    map.provinces.entry(3).or_default().sea = true;
    let provs: BTreeMap<u16, AiProvince> = BTreeMap::from([(1, prov("A", "A")), (2, prov("A", "A")), (4, prov("B", "B")), (5, prov("B", "B"))]);
    let armies = [army(1, "A", 2, 12.0)];
    let sd = sides();
    let sup: &dyn Fn(&str, u16) -> i64 = &supply;
    let w = AiWorld { map: &map, provinces: &provs, armies: &armies, sides: &sd, supply: sup };
    assert!(plan_armies(&rules(), &w, "A").is_empty());
    assert_eq!(plan_landings(&rules(), &w, "A", vec![6], &BTreeSet::new()), vec![(1, vec![3, 4])]);
    // No fleet carries all four regiments: it stays.
    assert!(plan_landings(&rules(), &w, "A", vec![2, 2], &BTreeSet::new()).is_empty());
    let mut off = rules();
    off.off.landing = true;
    assert!(plan_landings(&off, &w, "A", vec![6], &BTreeSet::new()).is_empty());
}

/// Guard posts in peace (TST_MIL_AI_POSTS): A holds 1..=5 with its capital at 1, B holds 6 and 7.
/// Twenty regiments make two posts: the capital and the border province 5; the lone armies
/// inland go there, the one at the capital stays.
#[test]
fn peace_armies_hold_the_capital_and_the_threatened_border() {
    let mut adj = vec![BTreeSet::new(); 8];
    for a in 1..7usize {
        adj[a].insert((a + 1) as u16);
        adj[a + 1].insert(a as u16);
    }
    let map = MoveMap::from_adjacency(&adj, &[], &|_, _| 2.0);
    let provs: BTreeMap<u16, AiProvince> = (1..=7).map(|i| (i, if i <= 5 { prov("A", "A") } else { prov("B", "B") })).collect();
    let armies = [army(1, "A", 1, 30.0), army(2, "A", 3, 15.0), army(3, "A", 2, 15.0), army(9, "B", 7, 30.0)];
    let sd = Sides::default();
    let sup: &dyn Fn(&str, u16) -> i64 = &supply;
    let w = AiWorld { map: &map, provinces: &provs, armies: &armies, sides: &sd, supply: sup };
    let orders = plan_posts(&rules(), &w, "A", Some(1), &|o| if o == "B" { 10.0 } else { 0.0 });
    assert_eq!(orders, vec![(2, 5), (3, 5)]);
    // A friend next door is no threat: everything stays at the capital.
    let orders = plan_posts(&rules(), &w, "A", Some(1), &|_| 0.0);
    assert_eq!(orders, vec![(2, 1), (3, 1)]);
}
