//! Battle rules on synthetic fixtures. Nothing here is Victoria 2 data.

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_simulation::military::battle::*;
use wiktoria_simulation::military::movement::{MovementRules, Border, BorderContext, BorderKind, MoveMap, Relations};
use wiktoria_simulation::military::siege::ProvinceMilitary;
use wiktoria_simulation::military::units::UnitDef;

fn rules() -> BattleRules {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let engine_rules = wiktoria_data::config::load_engine_rules(&root).unwrap();
    BattleRules::from_engine_rules(&engine_rules).unwrap()
}

/// A regiment that hardly gets hurt (huge discipline), so a battle can run for weeks.
fn fighter(id: u64, army: u64, country: &str) -> Fighter {
    Fighter { id, army, country: country.into(), attack: 3.0, defence: 3.0, discipline: 1000.0, maneuver: 0.0, max_strength: 3.0, strength: 3.0, organisation: 10.0, pop: Some(id * 100), ..Fighter::default() }
}

fn country(tag: &str, width_modifier: f64) -> CountryInfo {
    // A big tactics value divides strength damage away.
    CountryInfo { tag: tag.into(), width_modifier, tactics_modifier: 1.0e6, exhaustion_count: 1.0, ..CountryInfo::default() }
}

fn ctx(att: &[(&str, f64)], def: &[(&str, f64)]) -> LandContext {
    LandContext {
        attacker: SideInfo { countries: att.iter().map(|(t, w)| country(t, *w)).collect(), ..SideInfo::default() },
        defender: SideInfo { countries: def.iter().map(|(t, w)| country(t, *w)).collect(), ..SideInfo::default() },
        ..LandContext::default()
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn constants_come_from_the_engine_rules() {
    let r = rules();
    assert_eq!((r.dice_period_days, r.dice_sides, r.lock_days), (5.0, 10, 11.0));
    assert_eq!(r.table.len(), 18);
    assert_eq!((r.table[0], r.table[7], r.table[17]), (0.02, 0.20, 0.90));
    // The table index is die + modifiers + 2, clamped.
    assert_eq!(r.table_factor(-9.0), 0.02);
    assert_eq!(r.table_factor(5.0), 0.20);
    assert_eq!(r.table_factor(40.0), 0.90);
}

#[test]
fn width_takes_the_smallest_country_and_the_terrain() {
    let r = rules();
    assert_eq!(width(&r, &ctx(&[("AAA", 0.0)], &[("BBB", 0.0)])), 30);
    assert_eq!(width(&r, &ctx(&[("AAA", 0.0)], &[("BBB", -28.0)])), 2);
    let mut c = ctx(&[("AAA", -28.0)], &[("BBB", 0.0)]);
    c.terrain_width = 0.5;
    assert_eq!(width(&r, &c), 3);
    // Never below 2.
    assert_eq!(width(&r, &ctx(&[("AAA", -100.0)], &[])), 2);
    assert_eq!(slot_order()[..5], [16, 15, 17, 14, 18]);
}

#[test]
fn crossing_penalty_strait_beats_river() {
    assert_eq!(crossing_penalty(&[Entry::Plain, Entry::River]), -1);
    assert_eq!(crossing_penalty(&[Entry::River, Entry::Landing]), -2);
    assert_eq!(crossing_penalty(&[Entry::Strait]), -2);
    assert_eq!(crossing_penalty(&[Entry::Plain]), 0);
}

#[test]
fn attacker_modifier_has_leader_terrain_river_and_dig_in() {
    let r = rules();
    let mut a = fighter(11, 1, "AAA");
    let mut d = fighter(21, 2, "BBB");
    d.dig_in = 3.0;
    let mut c = Combat::new(CombatKind::Land, 1, vec![1], vec![2]);
    c.attacker.reserve = vec![11];
    c.defender.reserve = vec![21];
    let mut context = ctx(&[("AAA", 0.0)], &[("BBB", 0.0)]);
    context.attacker.leader_attack = 2.7;
    context.defender.leader_defence = 1.9;
    context.terrain_defence = 1.0;
    context.crossing = -1;
    let mut f = BTreeMap::from([(11, a.clone()), (21, d.clone())]);
    // 2 - 1 - 1 - dig-in 3 (no reconnaissance).
    assert_eq!(side_modifier(&r, &c, &f, &context, BattleSide::Attacker), -3.0);
    assert_eq!(side_modifier(&r, &c, &f, &context, BattleSide::Defender), 1.0);
    // Reconnaissance 2 over the whole strength divides the dig-in by floor(1 + 2) = 3.
    a.recon = 2.0;
    f.insert(11, a);
    assert_eq!(side_modifier(&r, &c, &f, &context, BattleSide::Attacker), -1.0);
    // The LOWEST dig-in of the defenders counts.
    c.defender.reserve.push(22);
    f.insert(22, fighter(22, 2, "BBB"));
    assert_eq!(side_modifier(&r, &c, &f, &context, BattleSide::Attacker), 2.0 - 1.0 - 1.0);
}

#[test]
fn gas_adds_to_the_dice_of_a_side_whose_enemy_has_no_defence() {
    let r = rules();
    let mut c = Combat::new(CombatKind::Land, 1, vec![1], vec![2]);
    c.attacker.reserve = vec![11];
    c.defender.reserve = vec![21];
    let f = BTreeMap::from([(11, fighter(11, 1, "AAA")), (21, fighter(21, 2, "BBB"))]);
    let mut context = ctx(&[("AAA", 0.0)], &[("BBB", 0.0)]);
    context.gas_attack_modifier = 3.0;
    context.defender.countries[0].gas_attack = true;
    assert_eq!(side_modifier(&r, &c, &f, &context, BattleSide::Defender), 3.0);
    assert_eq!(side_modifier(&r, &c, &f, &context, BattleSide::Attacker), 0.0);
    // Gas defence on the other side takes it away.
    context.attacker.countries[0].gas_defence = true;
    assert_eq!(side_modifier(&r, &c, &f, &context, BattleSide::Defender), 0.0);
    // Gas attack on both sides and no defence gives it to both.
    context.attacker.countries[0].gas_defence = false;
    context.attacker.countries[0].gas_attack = true;
    assert_eq!(side_modifier(&r, &c, &f, &context, BattleSide::Attacker), 3.0);
    assert_eq!(side_modifier(&r, &c, &f, &context, BattleSide::Defender), 3.0);
}

#[test]
fn damage_of_one_regiment_follows_the_formula() {
    let r = rules();
    let mut a = fighter(1, 1, "AAA");
    a.attack = 2.0;
    a.strength = 3.0;
    let mut t = fighter(2, 2, "BBB");
    t.discipline = 4.0;
    // table[5 + 2] = 0.2; x = 0.2 * (1 + 0.2) * 3 = 0.72; organisation 0.72 * 10 / 4 * 0.2.
    let (s, o) = regiment_damage(&r, 5.0, true, 1.0, &a, &t, 0.0, 1.0);
    assert!(near(o, 0.72 * 10.0 / 4.0 * 0.2), "{o}");
    assert!(near(s, 0.72 / (r.base_tactics + 1.0) * 0.2), "{s}");
    // A target whose country has no tactics at all is not divided, not even by the base.
    let (s, _) = regiment_damage(&r, 5.0, true, 1.0, &a, &t, 0.0, 0.0);
    assert!(near(s, 0.72 * 0.2), "{s}");
    // Support fire is scaled, experience and forts divide.
    let (s2, _) = regiment_damage(&r, 5.0, true, 0.5, &a, &t, 0.0, 0.0);
    assert!(near(s2, s / 2.0));
    t.experience = 100.0;
    let (s3, _) = regiment_damage(&r, 5.0, true, 1.0, &a, &t, 2.0, 0.0);
    assert!(near(s3, s / 2.0 / 1.2));
    // The defender uses its defence stat.
    a.defence = 0.0;
    let (s4, _) = regiment_damage(&r, 5.0, false, 1.0, &a, &fighter(3, 2, "BBB"), 0.0, 0.0);
    assert!(near(s4, 0.2 * 1.0 * 3.0 * 0.2));
}

struct Field {
    c: Combat,
    f: BTreeMap<u64, Fighter>,
    ctx: LandContext,
    rng: BattleRng,
}

/// Army 1 (AAA) attacks army 2 (BBB), two regiments each, full width.
fn field() -> Field {
    let mut f = BTreeMap::new();
    for (id, army, tag) in [(11, 1, "AAA"), (12, 1, "AAA"), (21, 2, "BBB"), (22, 2, "BBB")] {
        f.insert(id, fighter(id, army, tag));
    }
    let mut c = Combat::new(CombatKind::Land, 1, vec![1], vec![2]);
    c.attacker.reserve = vec![11, 12];
    c.defender.reserve = vec![21, 22];
    Field { c, f, ctx: ctx(&[("AAA", 0.0), ("CCC", 0.0)], &[("BBB", 0.0)]), rng: BattleRng(7) }
}

#[test]
fn dice_hold_for_five_days_then_reroll() {
    let r = rules();
    let mut x = field();
    let mut dice = Vec::new();
    let mut rerolls = Vec::new();
    for day in 1..=11 {
        let rep = battle_day(&r, &mut x.c, &mut x.f, &x.ctx, &mut x.rng);
        assert!(rep.outcome.is_none());
        assert!(rep.dice.iter().all(|d| (0.0..10.0).contains(d) && d.fract() == 0.0));
        dice.push(rep.dice);
        if rep.rerolled {
            rerolls.push(day);
        }
    }
    assert_eq!(rerolls, [6, 11]);
    assert!(dice[..5].iter().all(|d| *d == dice[0]));
    assert!(dice[5..10].iter().all(|d| *d == dice[5]));
    assert_eq!(x.c.duration, 11.0);
}

#[test]
fn same_seed_same_battle() {
    let r = rules();
    let run = || {
        let mut x = field();
        (1..=8).map(|_| battle_day(&r, &mut x.c, &mut x.f, &x.ctx, &mut x.rng).dice).collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}

#[test]
fn first_day_deploys_the_reserve_into_the_front_line() {
    let r = rules();
    let mut x = field();
    battle_day(&r, &mut x.c, &mut x.f, &x.ctx, &mut x.rng);
    assert_eq!(x.c.attacker.front.values().copied().collect::<BTreeSet<_>>(), BTreeSet::from([11, 12]));
    assert_eq!(x.c.attacker.front.keys().copied().collect::<Vec<_>>(), [15, 16]);
    assert!(x.c.attacker.reserve.is_empty());
}

#[test]
fn a_side_with_nothing_in_front_is_overrun_on_day_one() {
    let r = rules();
    let mut x = field();
    x.c.defender.reserve.clear();
    x.f.remove(&21);
    x.f.remove(&22);
    let rep = battle_day(&r, &mut x.c, &mut x.f, &x.ctx, &mut x.rng);
    let o = rep.outcome.expect("overrun");
    assert_eq!(o.winner, BattleSide::Attacker);
    assert!(o.loser_destroyed);
    assert_eq!(o.destroyed, [2]);
}

#[test]
fn joining_a_battle_in_progress_fills_the_front_as_slots_free_up() {
    let r = rules();
    let mut f = BTreeMap::new();
    for (id, army, tag) in [(11, 1, "AAA"), (12, 1, "AAA"), (31, 3, "CCC"), (32, 3, "CCC"), (21, 2, "BBB"), (22, 2, "BBB")] {
        f.insert(id, fighter(id, army, tag));
    }
    let mut c = Combat::new(CombatKind::Land, 1, vec![1], vec![2]);
    c.attacker.reserve = vec![11, 12];
    c.defender.reserve = vec![21, 22];
    // Width 2: only two regiments a side fight at a time.
    let context = ctx(&[("AAA", -28.0), ("CCC", -28.0)], &[("BBB", 0.0)]);
    let mut rng = BattleRng(3);
    battle_day(&r, &mut c, &mut f, &context, &mut rng);
    assert_eq!(c.attacker.front.len(), 2);
    // C arrives in the middle of the fight: army added, regiments wait in the reserve.
    join_battle(&mut c, BattleSide::Attacker, &[JoinUnit { army: 3, regiments: vec![31, 32] }]);
    assert_eq!(c.attacker.units, [1, 3]);
    assert_eq!(c.attacker.reserve, [31, 32]);
    // Joining twice changes nothing.
    join_battle(&mut c, BattleSide::Attacker, &[JoinUnit { army: 3, regiments: vec![31, 32] }]);
    assert_eq!(c.attacker.reserve, [31, 32]);
    // A front regiment is worn out; before the morale lock one unfit regiment goes back a day.
    f.get_mut(&11).unwrap().organisation = 0.5;
    battle_day(&r, &mut c, &mut f, &context, &mut rng);
    battle_day(&r, &mut c, &mut f, &context, &mut rng);
    let front: Vec<u64> = c.attacker.front.values().copied().collect();
    assert!(front.contains(&31), "reserve regiment took the free slot: {front:?}");
    assert!(front.contains(&12));
    assert!(!front.contains(&11));
    assert!(c.attacker.front.len() <= 2);
}

#[test]
fn rotation_swap_then_the_beaten_side_has_nowhere_to_go() {
    let r = rules();
    let mut x = field();
    for (id, army, tag) in [(31, 3, "CCC"), (32, 3, "CCC")] {
        x.f.insert(id, fighter(id, army, tag));
    }
    let mv = movement_rules();
    // D17: one week (the value is movement's FLD_MIL_MOVE_ORDER_BATTLE_MIN_DAYS). Orders are refused while the battle has lasted at most `lock` days.
    let lock = mv.order_battle_min_days as u32;
    assert!(lock >= 5, "the test needs a lock of at least 5 days, rules file has {lock}");
    for day in 1..=lock + 1 {
        let rep = battle_day(&r, &mut x.c, &mut x.f, &x.ctx, &mut x.rng);
        assert!(rep.outcome.is_none(), "day {day}");
        if day == 2 {
            join_battle(&mut x.c, BattleSide::Attacker, &[JoinUnit { army: 3, regiments: vec![31, 32] }]);
        }
        if day == 5 || day == lock {
            // No order out of the battle before the lock has passed (D17).
            assert!(matches!(leave_battle(&mv, &mut x.c, &x.f, 1), Err(LeaveError::Locked { .. })), "day {day}");
        }
    }
    assert_eq!(leave_battle(&mv, &mut x.c, &x.f, 9), Err(LeaveError::NotInBattle));
    // Fresh C stands in the line by now.
    assert!(x.c.attacker.front.values().any(|id| *id == 31 || *id == 32));
    let left = leave_battle(&mv, &mut x.c, &x.f, 1).expect("lock is over after the lock");
    assert_eq!(left.side, BattleSide::Attacker);
    assert!(left.retreating, "the enemy still has units");
    assert_eq!(left.regiments, [11, 12]);
    assert_eq!(x.c.attacker.units, [3]);
    let in_lines: BTreeSet<u64> = x.c.attacker.front.values().chain(x.c.attacker.back.values()).chain(&x.c.attacker.reserve).copied().collect();
    assert_eq!(in_lines, BTreeSet::from([31, 32]));
    // C goes on alone. The battle's own morale lock: nobody breaks while total days <= 11
    // so B can break on day 12 at the earliest.
    for day in lock + 2..=11 {
        for id in [21, 22] {
            x.f.get_mut(&id).unwrap().organisation = 0.5;
        }
        assert!(battle_day(&r, &mut x.c, &mut x.f, &x.ctx, &mut x.rng).outcome.is_none(), "day {day}");
    }
    // B is worn out and now breaks.
    for id in [21, 22] {
        x.f.get_mut(&id).unwrap().organisation = 0.5;
    }
    let rep = battle_day(&r, &mut x.c, &mut x.f, &x.ctx, &mut x.rng);
    let o = rep.outcome.expect("B has lost");
    assert_eq!(o.winner, BattleSide::Attacker);
    assert!(!o.loser_destroyed, "B holds more than a tenth of the winner");
    assert_eq!(o.retreating, [2]);
    // What the end gives: equal prestige up and down, exhaustion, a record for the war score.
    let p = |tag: &str| o.result.prestige.iter().find(|(t, _)| t == tag).map(|(_, v)| *v).unwrap();
    assert!(p("AAA") > 0.0 && near(p("AAA"), p("CCC")) && near(p("BBB"), -p("AAA")));
    assert!(o.result.exhaustion.iter().all(|(_, v)| (0.0..=r.war_exhaustion_max).contains(v)));
    assert!(o.result.record.attacker_won && !o.result.record.naval && o.result.record.score > 0.0);
    // B has no retreat province: every neighbour holds a hostile army, so it is deleted.
    let map = MoveMap {
        borders: BTreeMap::from([((1, 2), border()), ((2, 1), border())]),
        ..MoveMap::default()
    };
    let provinces = BTreeMap::from([(2, province("AAA", "AAA"))]);
    let rel = relations();
    let ctx_b = BorderContext::default();
    let hostile = BTreeSet::from([2]);
    let none = BTreeSet::new();
    let facts = RetreatFacts { map: &map, border_ctx: &ctx_b, relations: &rel, provinces: &provinces, hostile_armies: &hostile, exposed: &none, sea_room: &none };
    let unit = RetreatUnit { owner: "BBB".into(), from: 1, mean_organisation: 5.0, at_sea: false, rebel_can_retreat: false };
    assert_eq!(retreat_target(&unit, &facts, &mut x.rng), None);
}

fn movement_rules() -> MovementRules {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let m = wiktoria_data::config::load_engine_rules(&root).unwrap();
    // Only the battle lock matters here; the rest of movement needs the install's defines.
    MovementRules { sea_land_distance: 0.0, sea_land_speed: 0.0, min_distance_factor: 0.0, land_speed_modifier: 1.0, naval_speed_modifier: 1.0, order_battle_min_days: wiktoria_simulation::num_field(&m, "FLD_MIL_MOVE_ORDER_BATTLE_MIN_DAYS").unwrap(), just_moved_lock_days: 0.0, regiments_per_transport: 1.0 }
}

fn border() -> Border {
    Border { kind: BorderKind::Land, distance: 1.0, through: None }
}

fn province(owner: &str, controller: &str) -> ProvinceMilitary {
    ProvinceMilitary { owner: Some(owner.into()), controller: Some(controller.into()), ..ProvinceMilitary::default() }
}

fn relations() -> Relations {
    Relations { rebel_tag: "REB".into(), wars: BTreeSet::from([("AAA".to_string(), "BBB".to_string())]), ..Relations::default() }
}

#[test]
fn a_side_under_a_tenth_of_the_winner_is_destroyed_and_its_pops_pay() {
    let r = rules();
    let mut x = field();
    x.f.get_mut(&21).unwrap().strength = 0.2;
    x.c.defender.reserve = vec![21];
    x.f.remove(&22);
    x.c.attacker.reserve = vec![11, 12];
    x.c.duration = 12.0;
    x.c.day = 3.0;
    // B's front line is already in place and worn out.
    x.f.get_mut(&21).unwrap().organisation = 0.5;
    let rep = battle_day(&r, &mut x.c, &mut x.f, &x.ctx, &mut x.rng);
    let o = rep.outcome.expect("ends");
    assert!(o.loser_destroyed);
    assert_eq!(o.destroyed, [2]);
    assert!(o.retreating.is_empty());
    let cas = rep.casualties.iter().find(|c| c.pop == 2100).expect("soldier pop of regiment 21");
    // 0.2 of 3.0 strength is 1/15 of the pop size of a regiment, times the soldier damage share.
    assert!(near(cas.men, 0.2 / 3.0 * r.pop_size_per_regiment * r.soldier_to_pop_damage));
}

#[test]
fn losses_come_back_as_casualties_per_pop() {
    let r = rules();
    // Attackers hurt for real: discipline and tactics are ordinary.
    let mut x = field();
    for f in x.f.values_mut() {
        f.discipline = 5.0;
        f.attack = 6.0;
        f.defence = 6.0;
    }
    for c in x.ctx.attacker.countries.iter_mut().chain(x.ctx.defender.countries.iter_mut()) {
        c.tactics_modifier = 0.0;
    }
    let rep = battle_day(&r, &mut x.c, &mut x.f, &x.ctx, &mut x.rng);
    assert!(!rep.casualties.is_empty());
    assert!(rep.casualties.iter().all(|c| c.men > 0.0 && [1100, 1200, 2100, 2200].contains(&c.pop)));
    // Strength lost and men lost agree.
    let lost: f64 = x.f.values().map(|f| 3.0 - f.strength).sum();
    let men: f64 = rep.casualties.iter().map(|c| c.men).sum();
    assert!(near(men, lost / 3.0 * r.pop_size_per_regiment * r.soldier_to_pop_damage));
}

fn two_provinces_with_a_free_neighbour() -> (MoveMap, BTreeMap<u16, ProvinceMilitary>) {
    let map = MoveMap { borders: BTreeMap::from([((1, 2), border()), ((2, 1), border()), ((1, 3), border()), ((3, 1), border())]), ..MoveMap::default() };
    let provinces = BTreeMap::from([(2, province("AAA", "AAA")), (3, province("BBB", "BBB"))]);
    (map, provinces)
}

#[test]
fn retreat_goes_to_own_land_and_never_into_hostile_armies() {
    let (map, provinces) = two_provinces_with_a_free_neighbour();
    let rel = relations();
    let (ctx_b, none) = (BorderContext::default(), BTreeSet::new());
    let unit = RetreatUnit { owner: "BBB".into(), from: 1, mean_organisation: 5.0, at_sea: false, rebel_can_retreat: false };
    let facts = RetreatFacts { map: &map, border_ctx: &ctx_b, relations: &rel, provinces: &provinces, hostile_armies: &none, exposed: &none, sea_room: &none };
    // Own province 3 scores 4 + 0..3, enemy-held 2 scores 1 + 0..3 (a tie goes to the lower id):
    // ours wins in most draws, and a way out always exists.
    let ours = (0..40).filter(|seed| retreat_target(&unit, &facts, &mut BattleRng(*seed)) == Some(3)).count();
    assert!(ours >= 28, "{ours}");
    assert!((0..40).all(|seed| retreat_target(&unit, &facts, &mut BattleRng(seed)).is_some()));
    // A worn-out army may not enter an enemy-controlled province (only 3 and 2 are left).
    let weak = RetreatUnit { mean_organisation: 0.5, ..unit.clone() };
    let hostile = BTreeSet::from([3]);
    let facts = RetreatFacts { hostile_armies: &hostile, ..facts };
    assert_eq!(retreat_target(&weak, &facts, &mut BattleRng(1)), None);
    // Rebels that may not retreat are deleted at once.
    let reb = RetreatUnit { owner: "REB".into(), ..unit };
    assert_eq!(retreat_target(&reb, &facts, &mut BattleRng(1)), None);
}

#[test]
fn naval_battle_is_decided_by_the_stronger_fleet() {
    let r = rules();
    let def = |hull: f64, strength: f64| UnitDef { max_strength: strength, organisation: 30.0, speed: 10.0, hull, gun_power: 12.0, fire_range: 1.0, evasion: 0.2, ..UnitDef::default() };
    let run = |seed: u64| {
        let mut ships = Vec::new();
        for i in 0..3 {
            ships.push(NavalShip::new(100 + i, 1, BattleSide::Attacker, ShipClass::Big, &def(5.0, 10.0), 10.0, 30.0, 0.0));
        }
        for i in 0..2 {
            ships.push(NavalShip::new(200 + i, 2, BattleSide::Defender, ShipClass::Light, &def(5.0, 5.0), 5.0, 30.0, 0.0));
        }
        let mut st = NavalState { ships, fleeing: [false; 2], attacker: NavalSideInfo::default(), defender: NavalSideInfo::default() };
        let mut c = Combat::new(CombatKind::Naval, 1000, vec![1], vec![2]);
        let context = LandContext::default();
        let mut rng = BattleRng(seed);
        for day in 1..=600 {
            let rep = naval_day(&r, &mut c, &mut st, &context, &mut rng);
            if let Some(o) = rep.outcome {
                return (day, o.winner, o.result.record.naval, st.ships.iter().filter(|s| s.state == ShipState::Gone).count(), o.result.exhaustion.is_empty());
            }
        }
        panic!("the naval battle never ended");
    };
    let first = run(5);
    assert_eq!(first, run(5), "same seed, same battle");
    let (_, winner, naval, _, no_exhaustion) = first;
    assert_eq!(winner, BattleSide::Attacker);
    assert!(naval);
    assert!(no_exhaustion, "no war exhaustion from naval battles");
    assert!((1..=4u64).any(|s| run(s).1 == BattleSide::Attacker));
}

#[test]
fn naval_first_day_overrun_destroys_a_tiny_fleet() {
    let r = rules();
    let def = UnitDef { max_strength: 10.0, organisation: 30.0, speed: 10.0, hull: 5.0, gun_power: 12.0, fire_range: 1.0, ..UnitDef::default() };
    let ships = vec![NavalShip::new(1, 1, BattleSide::Attacker, ShipClass::Big, &def, 10.0, 30.0, 0.0), NavalShip::new(2, 2, BattleSide::Defender, ShipClass::Big, &def, 0.5, 30.0, 0.0)];
    let mut st = NavalState { ships, fleeing: [false; 2], attacker: NavalSideInfo::default(), defender: NavalSideInfo::default() };
    let mut c = Combat::new(CombatKind::Naval, 1000, vec![1], vec![2]);
    let rep = naval_day(&r, &mut c, &mut st, &LandContext::default(), &mut BattleRng(1));
    let o = rep.outcome.expect("overrun");
    assert_eq!((o.winner, o.loser_destroyed, rep.sunk.clone()), (BattleSide::Attacker, true, vec![2]));
    assert_eq!(o.destroyed, [2]);
}
