//! Movement rules on synthetic fixtures.

use std::collections::BTreeSet;
use wiktoria_data::defines::Defines;
use wiktoria_data::geography::ExtraAdjacency;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_simulation::military::movement::*;
use wiktoria_simulation::military::siege::ProvinceMilitary;

fn rules() -> MovementRules {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/engine_rules.json");
    let d = Defines::parse("defines = { military = { LAND_SPEED_MODIFIER = 0.5, NAVAL_SPEED_MODIFIER = 2.0, }, }").unwrap();
    MovementRules::new(&d, &EngineRules::from_json(&std::fs::read_to_string(p).unwrap()).unwrap()).unwrap()
}

fn s(x: &str) -> String {
    x.to_string()
}

fn adj(pairs: &[(usize, usize)], n: usize) -> Vec<BTreeSet<u16>> {
    let mut v = vec![BTreeSet::new(); n];
    for &(a, b) in pairs {
        v[a].insert(b as u16);
        v[b].insert(a as u16);
    }
    v
}

fn map(pairs: &[(usize, usize)], extra: &[ExtraAdjacency]) -> MoveMap {
    MoveMap::from_adjacency(&adj(pairs, 8), extra, &|a, b| (a as f64 - b as f64).abs() * 2.0)
}

fn prov(owner: Option<&str>, ctl: Option<&str>) -> ProvinceMilitary {
    ProvinceMilitary { owner: owner.map(s), controller: ctl.map(s), garrison: 0.0, fort: 0.0, naval_base: 0.0, last_controller_change: None }
}

fn rel() -> Relations {
    let mut r = Relations { rebel_tag: s("REB"), ..Relations::default() };
    r.wars.insert((s("AAA"), s("BBB")));
    r
}

fn speed_in() -> SpeedInput {
    SpeedInput { is_army: true, from_sea: false, to_sea: false, unit_speed: 4.0, terrain_entry: 0.0, crossing_entry: 0.0, infrastructure: 0.5, leader_speed: 0.0 }
}

#[test]
fn speed_and_distance() {
    let r = rules();
    let sp = unit_speed(&r, &speed_in());
    assert!((sp - 4.0 * 1.5 * r.land_speed_modifier).abs() < 1e-9);
    let sea = SpeedInput { to_sea: true, ..speed_in() };
    assert_eq!(unit_speed(&r, &sea), r.sea_land_speed);
    assert_eq!(step_distance(&r, true, false, true, 5.0, 0.0), r.sea_land_distance);
    assert_eq!(step_distance(&r, true, false, false, 10.0, 0.5), 15.0);
    assert!((step_distance(&r, true, false, false, 10.0, -2.0) - 10.0 * r.min_distance_factor).abs() < 1e-9);
}

#[test]
fn daily_step_keeps_surplus_and_stops_in_battle() {
    assert_eq!(daily_step(3.0, 2.0, 4.0, false, 0, 0, false), DailyStep::Step { progress: 1.0 });
    assert_eq!(daily_step(1.0, 2.0, 4.0, false, 0, 0, false), DailyStep::Wait { progress: 3.0 });
    assert_eq!(daily_step(3.0, 2.0, 4.0, true, 0, 0, false), DailyStep::Wait { progress: 4.0 });
    assert_eq!(daily_step(3.0, 2.0, 4.0, false, 10, 20, false), DailyStep::Wait { progress: 5.0 });
    assert_eq!(daily_step(3.0, 2.0, 4.0, false, 0, 0, true), DailyStep::Cancel);
}

#[test]
fn arrival_hours_and_orders() {
    assert_eq!(arrival_hour(100, 10.0, 2.0, 3.0, 0), Some(100 + 3 * 24));
    assert_eq!(arrival_hour(100, 10.0, 2.0, 3.0, 500), Some(500));
    assert_eq!(arrival_hour(0, 1.0, 0.0, 0.0, 0), None);
    assert_eq!(group_arrival(&[Some(5), Some(9)]), Some(9));
    assert_eq!(progress_after_order(&[3, 4], &[3, 5], 2.0), 2.0);
    assert_eq!(progress_after_order(&[3, 4], &[6], 2.0), 0.0);
    let r = rules();
    assert!(!may_take_order(&r, true, r.order_battle_min_days));
    assert!(may_take_order(&r, true, r.order_battle_min_days + 1.0));
    assert!(may_take_order(&r, false, 0.0));
    assert_eq!(lock_tick(lock_after_step(&r)), 0.0);
}

#[test]
fn borders_close() {
    let ex = [
        ExtraAdjacency { from: 1, to: 5, kind: s("impassable"), through: 0 },
        ExtraAdjacency { from: 2, to: 6, kind: s("strait"), through: 7 },
        ExtraAdjacency { from: 3, to: 4, kind: s("canal"), through: 4 },
    ];
    let mut m = map(&[(1, 2), (2, 3)], &ex);
    let mut ctx = BorderContext::default();
    assert!(border_closed(&m, &ctx, false, 1, 5));
    assert!(!border_closed(&m, &ctx, false, 1, 2));
    assert!(border_closed(&m, &ctx, false, 1, 7)); // no border at all
    assert!(!border_closed(&m, &ctx, false, 2, 6));
    ctx.hostile_fleet_seas.insert(7);
    assert!(border_closed(&m, &ctx, false, 2, 6));
    assert!(!border_closed(&m, &BorderContext::default(), false, 3, 4));
    ctx.hostile_controlled.insert(4);
    assert!(border_closed(&m, &ctx, false, 3, 4));
    m.canals_open = false;
    assert!(border_closed(&m, &BorderContext::default(), false, 3, 4));
    assert!(safe_closed(5.0, 3.0, false) && safe_closed(0.0, 3.0, true) && !safe_closed(1.0, 3.0, false));
}

#[test]
fn fleets_only_touch_their_port() {
    let mut m = map(&[(1, 2), (2, 3)], &[]);
    m.provinces.insert(1, MoveProvince { port_sea: Some(2), ..Default::default() });
    m.provinces.insert(2, MoveProvince { sea: true, ..Default::default() });
    m.provinces.insert(3, MoveProvince { sea: true, ..Default::default() });
    let c = BorderContext::default();
    assert!(!border_closed(&m, &c, true, 2, 1));
    assert!(!border_closed(&m, &c, true, 1, 2));
    assert!(!border_closed(&m, &c, true, 2, 3));
    m.provinces.insert(1, MoveProvince { port_sea: Some(3), ..Default::default() });
    assert!(border_closed(&m, &c, true, 2, 1));
}

#[test]
fn may_enter() {
    let mut r = rel();
    let home = prov(Some("AAA"), Some("AAA"));
    assert!(may_enter_land(&r, "AAA", false, &home, &prov(Some("BBB"), Some("BBB")))); // at war
    assert!(!may_enter_land(&r, "AAA", false, &home, &prov(Some("CCC"), Some("CCC"))));
    assert!(may_enter_land(&r, "AAA", false, &home, &prov(None, None))); // unowned
    assert!(may_enter_land(&r, "AAA", true, &home, &prov(Some("CCC"), Some("CCC")))); // exiled
    assert!(may_enter_land(&r, "AAA", false, &home, &prov(Some("AAA"), Some("AAA"))));
    r.friendly.insert((s("AAA"), s("CCC")));
    assert!(may_enter_land(&r, "AAA", false, &home, &prov(Some("CCC"), Some("CCC")))); // access
    assert!(may_enter_land(&r, "REB", false, &home, &prov(Some("DDD"), Some("DDD"))));
    // overlord at war: vassal's land is open
    r.overlord.insert(s("EEE"), s("BBB"));
    assert!(may_enter_land(&r, "AAA", false, &home, &prov(Some("EEE"), Some("EEE"))));
    assert!(may_enter_sea(2, 1, 3) && !may_enter_sea(3, 1, 3));
}

#[test]
fn path_finding_is_deterministic() {
    // 1-2-4 and 1-3-4 are equally long: lowest id wins. 1-5 is closed.
    let m = MoveMap::from_adjacency(&adj(&[(1, 2), (1, 3), (2, 4), (3, 4), (1, 5), (5, 4)], 8), &[], &|_, _| 1.0);
    let open = |_: u16, _: u16| true;
    let p = find_path(&m, 1, 4, &open).unwrap();
    assert_eq!(p, vec![2, 4]);
    assert_eq!(find_path(&m, 1, 4, &open), Some(p));
    assert_eq!(find_path(&m, 1, 4, &|_, b| b != 2), Some(vec![3, 4]));
    assert_eq!(find_path(&m, 1, 1, &open), Some(vec![]));
    assert_eq!(find_path(&m, 1, 7, &open), None);
    // a long way round is taken when the short one is closed
    let m2 = MoveMap::from_adjacency(&adj(&[(1, 2), (2, 3), (1, 4), (4, 5), (5, 3)], 8), &[], &|_, _| 1.0);
    assert_eq!(find_path(&m2, 1, 3, &open), Some(vec![2, 3]));
    assert_eq!(find_path(&m2, 1, 3, &|a, b| (a, b) != (2, 3)), Some(vec![4, 5, 3]));
}

#[test]
fn embark_picks_fleet_with_room() {
    let fl = [
        FleetRoom { id: 9, owner: s("AAA"), free: 5 },
        FleetRoom { id: 4, owner: s("AAA"), free: 2 },
        FleetRoom { id: 2, owner: s("BBB"), free: 9 },
    ];
    assert_eq!(embark("AAA", 3, &fl), Some(9));
    assert_eq!(embark("AAA", 1, &fl), Some(4));
    assert_eq!(embark("AAA", 6, &fl), None);
    let r = rel();
    let coast = prov(Some("BBB"), Some("BBB"));
    assert!(may_disembark(&r, "AAA", false, &coast, &coast));
    assert!(!may_disembark(&r, "CCC", false, &prov(None, Some("DDD")), &coast));
}

fn mover(owner: &str) -> Mover {
    Mover { id: 1, owner: s(owner), is_army: true, exiled: false, retreating: false, strength: 3.0, on_moving_fleet: false, at_sea: false, standing: true, lock_days: 0.0 }
}

fn stand(id: u64, owner: &str) -> Standing {
    Standing { id, owner: s(owner), is_army: true, exiled: false, retreating: false, at_sea: false, carried: false, in_port: false }
}

fn battle(index: usize, att: &[&str], def: &[&str]) -> CombatView {
    CombatView { index, siege: false, land: true, attacker_countries: att.iter().map(|x| s(x)).collect(), defender_countries: def.iter().map(|x| s(x)).collect() }
}

#[test]
fn arrival_dogpile_then_siege_then_new_battle() {
    let r = rel();
    // BBB attacks CCC? only AAA-BBB at war. AAA arrives where BBB fights CCC: joins vs BBB.
    let m = mover("AAA");
    assert_eq!(arrival(&r, &m, Some("BBB"), &[stand(7, "BBB")], &[battle(0, &["BBB"], &["CCC"])]), Arrival::JoinBattle { combat: 0, side: Side::Defender });
    // BBB defends against CCC: AAA joins the attacker side (not hostile to CCC).
    assert_eq!(arrival(&r, &m, Some("BBB"), &[stand(7, "BBB")], &[battle(3, &["CCC"], &["BBB"])]), Arrival::JoinBattle { combat: 3, side: Side::Attacker });
    // A battle beats a siege and an existing enemy.
    let siege = CombatView { siege: true, ..battle(1, &["CCC"], &["BBB"]) };
    assert_eq!(arrival(&r, &m, Some("BBB"), &[stand(7, "BBB")], &[siege.clone(), battle(2, &["CCC"], &["BBB"])]), Arrival::JoinBattle { combat: 2, side: Side::Attacker });
    assert_eq!(arrival(&r, &m, Some("BBB"), &[stand(7, "BBB")], &[siege]), Arrival::JoinSiege { combat: 1, side: Side::Attacker });
    // No combat: new battle, arriver attacks.
    assert_eq!(arrival(&r, &m, Some("BBB"), &[stand(8, "CCC"), stand(7, "BBB")], &[]), Arrival::StartBattle { attacker: 1, defender: 7 });
    // No enemy standing: siege of a hostile province.
    assert_eq!(arrival(&r, &m, Some("BBB"), &[stand(8, "CCC")], &[]), Arrival::StartSiege { besieger: 1 });
    // Own or friendly province: nothing. Moving unit does not besiege.
    assert_eq!(arrival(&r, &m, Some("AAA"), &[], &[]), Arrival::Nothing);
    assert_eq!(arrival(&r, &Mover { standing: false, ..m.clone() }, Some("BBB"), &[], &[]), Arrival::Nothing);
}

#[test]
fn arrival_exceptions() {
    let r = rel();
    let m = mover("AAA");
    let foe = [stand(7, "BBB")];
    assert_eq!(arrival(&r, &Mover { lock_days: 1.0, ..m.clone() }, None, &foe, &[]), Arrival::Nothing);
    assert_eq!(arrival(&r, &Mover { exiled: true, ..m.clone() }, None, &foe, &[]), Arrival::Nothing);
    assert_eq!(arrival(&r, &Mover { on_moving_fleet: true, ..m.clone() }, None, &foe, &[]), Arrival::Nothing);
    assert_eq!(arrival(&r, &Mover { retreating: true, ..m.clone() }, None, &foe, &[]), Arrival::Nothing);
    assert_eq!(arrival(&r, &m, None, &[Standing { retreating: true, ..stand(7, "BBB") }], &[]), Arrival::Nothing);
    assert_eq!(arrival(&r, &m, None, &[Standing { carried: true, ..stand(7, "BBB") }], &[]), Arrival::Nothing);
    // Fleets do not meet armies, and a fleet in port does not fight.
    let fleet = Mover { is_army: false, ..m.clone() };
    assert_eq!(arrival(&r, &fleet, None, &foe, &[]), Arrival::Nothing);
    let port = Standing { is_army: false, in_port: true, ..stand(7, "BBB") };
    assert_eq!(arrival(&r, &fleet, None, &[port], &[]), Arrival::Nothing);
    // Rebels fight everyone.
    assert_eq!(arrival(&r, &mover("REB"), None, &[stand(7, "CCC")], &[]), Arrival::StartBattle { attacker: 1, defender: 7 });
}
