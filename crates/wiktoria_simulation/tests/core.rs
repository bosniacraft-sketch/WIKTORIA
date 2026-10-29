//! wiktoria_simulation against the repo's own rules file (generated engine_rules) and synthetic numbers.

use wiktoria_data::date::Date;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_simulation::{Calendar, Fixed, GameDate, GoodMarket, PopSchedule, PriceRules};

fn engine_rules() -> EngineRules {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/engine_rules.json");
    EngineRules::from_json(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn fixed_point_arithmetic() {
    assert_eq!(Fixed::ONE.0, 32768);
    assert_eq!(Fixed::from_f64(2.5).0, 81920);
    assert_eq!(Fixed::from_f64(-0.01).0, -328, "rounds half away from zero");
    // 17.5 + two steps of 328, as a save prints it.
    assert_eq!(Fixed::from_save_text("17.52002").unwrap().0, 573_440 + 656);
    let a = Fixed::from_f64(3.0);
    let b = Fixed::from_f64(1.5);
    assert_eq!(a * b, Fixed::from_f64(4.5));
    assert_eq!(a.checked_div(b), Some(Fixed::from_f64(2.0)));
    assert_eq!(Fixed(1).checked_div(Fixed(3)), Some(Fixed(10922)), "(1 << 15) / 3, truncated");
    assert_eq!(a.checked_div(Fixed::ZERO), None);
    assert_eq!(Fixed::from_f64(0.25).sqrt(), Fixed::from_f64(0.5));
    assert_eq!(Fixed::from_f64(17.5).to_string(), "17.50000");
}

#[test]
fn engine_rules_price_constants_match_the_game_values() {
    let r = PriceRules::from_engine_rules(&engine_rules()).unwrap();
    // Raw values the original holds for these constants (007, 008).
    assert_eq!(r.step.0, 328);
    assert_eq!((r.min_factor.0, r.max_factor.0), (6554, 163_840));
    let raw: Vec<(i64, i64)> = r.table.iter().map(|(t, v)| (t.0, v.0)).collect();
    assert_eq!(&raw[..4], &[(1638, 7307), (2163, 8454), (2720, 9437), (3277, 10355)]);
    assert_eq!(raw.len(), 13);
    assert_eq!(r.fixed_good_flag, "money");
    assert_eq!(r.setup_updates, 2);
}

#[test]
fn price_factor_is_a_stepped_square_root() {
    let r = PriceRules::from_engine_rules(&engine_rules()).unwrap();
    assert_eq!(r.factor(Fixed::from_f64(0.01)), Fixed::from_f32(0.223), "below the first threshold");
    assert_eq!(r.factor(Fixed::from_f64(0.4)), Fixed::from_f32(0.7), "0.333 <= r < 0.5");
    assert_eq!(r.factor(Fixed::from_f64(1.0)), Fixed::ONE, "exact sqrt above the table");
    assert_eq!(r.factor(Fixed::from_f64(4.0)), Fixed::from_f64(2.0));
}

#[test]
fn price_moves_one_step_towards_the_target_within_limits() {
    let r = PriceRules::from_engine_rules(&engine_rules()).unwrap();
    let base = Fixed::from_f64(10.0);
    let mut g = GoodMarket::new(base, false);
    // demand = supply -> target = base: no move.
    g.supply = Fixed::from_f64(100.0);
    g.real_demand = Fixed::from_f64(100.0);
    g.update_price(&r);
    assert_eq!((g.price, g.price_change), (base, Fixed::ZERO));
    // demand 4x supply -> target 2x base: one step up per day.
    g.real_demand = Fixed::from_f64(400.0);
    g.update_price(&r);
    g.update_price(&r);
    assert_eq!(g.price, base + Fixed(656));
    assert_eq!(g.price_change, r.step);
    // Within one step of the target: no move.
    let mut near = GoodMarket::new(base, false);
    near.price = Fixed::from_f64(20.0) - Fixed(100);
    near.supply = Fixed::from_f64(1.0);
    near.real_demand = Fixed::from_f64(4.0);
    near.update_price(&r);
    assert_eq!(near.price_change, Fixed::ZERO);
    // Clamped at 5x base.
    let mut top = GoodMarket::new(base, false);
    top.price = Fixed::from_f64(50.0);
    top.supply = Fixed::from_f64(1.0);
    top.real_demand = Fixed::from_f64(10_000.0);
    top.update_price(&r);
    assert_eq!(top.price, Fixed::from_f64(50.0));
    // No supply or no demand: unchanged. Fixed-price goods: base cost.
    let mut idle = GoodMarket::new(base, false);
    idle.price = Fixed::from_f64(12.0);
    idle.real_demand = Fixed::from_f64(5.0);
    idle.update_price(&r);
    assert_eq!(idle.price, Fixed::from_f64(12.0));
    let mut gold = GoodMarket::new(base, true);
    gold.price = Fixed::from_f64(9.0);
    gold.supply = Fixed::from_f64(100.0);
    gold.real_demand = Fixed::from_f64(1.0);
    gold.update_price(&r);
    assert_eq!(gold.price, base);
}

#[test]
fn calendar_round_trips_and_steps_days() {
    let c = Calendar::from_engine_rules(&engine_rules()).unwrap();
    assert_eq!(c.days_per_year(), 365);
    let start = c.to_game(Date::new(1836, 1, 1)).unwrap();
    assert_eq!(start, GameDate(43_800_000 + 1836 * 365 * 24));
    // Value seen in a test log for 1836.6.22.
    assert_eq!(c.to_game(Date::new(1836, 6, 22)), Some(GameDate(59_887_488)));
    assert_eq!(c.to_date(c.next_day(c.to_game(Date::new(1836, 2, 28)).unwrap())), Date::new(1836, 3, 1), "no leap day");
    assert_eq!(c.to_date(c.next_day(c.to_game(Date::new(1836, 12, 31)).unwrap())), Date::new(1837, 1, 1));
    assert_eq!(c.to_game(Date::new(1836, 2, 29)), None);
}

#[test]
fn pop_schedule_matches_the_january_measurement() {
    let s = PopSchedule::from_engine_rules(&engine_rules()).unwrap();
    // pops with id mod 31 == 30 updated on 1836.1.31, id mod 31 == 0 on the 1st.
    assert!(s.is_due(30, 31) && s.is_due(61, 31));
    assert!(s.is_due(31, 1) && !s.is_due(31, 2));
    for day in 1..=31u8 {
        assert_eq!((0..31u64).filter(|id| s.is_due(*id, day)).count(), 1, "one id group per day");
        // In a 28-day month the groups follow the month length.
        if day <= 28 {
            assert_eq!((0..28u64).filter(|id| s.is_due_in(*id, day, 28)).count(), 1);
            assert!(s.is_due_in(u64::from(day) - 1, day, 28) && s.is_due_in(u64::from(day) + 27, day, 28));
        }
    }
}
