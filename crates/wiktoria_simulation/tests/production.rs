//! RGO production rule on synthetic numbers.

use std::collections::BTreeMap;

use wiktoria_data::production::{Job, JobEffect, ProductionEffects, RgoType};
use wiktoria_simulation::fixed::Fixed;
use wiktoria_simulation::modifiers::Totals;
use wiktoria_simulation::production::*;

fn rules() -> RgoRules {
    let l = |s: &str| s.split(',').map(str::to_string).collect::<Vec<_>>();
    RgoRules {
        scale: 1000,
        min_size: Fixed(300),
        base_factor: 1500,
        national_keys: l("n_in,n_out,n_thr"),
        local_keys: l("l_in,l_out,l_thr"),
        size_keys: l("f_size,m_size"),
        eff_keys: l("f_eff,m_eff"),
        good_size_suffix: "_size".into(),
        good_eff_suffix: "_eff".into(),
        good_bonus_keys: l("g_size,g_in,g_out,g_thr"),
        tech_eff_all_factors: true,
    }
}

fn farm() -> RgoType {
    RgoType {
        name: "rock_farm".into(),
        good: "rock".into(),
        value: 2.0,
        workforce: 1000.0,
        farm: true,
        mine: false,
        owner: Some(Job { pop_type: "lords".into(), effect: JobEffect::Output, amount: 1.0, multiplier: 1.0 }),
        employees: vec![
            Job { pop_type: "diggers".into(), effect: JobEffect::Throughput, amount: 1.0, multiplier: 1.0 },
            Job { pop_type: "serfs".into(), effect: JobEffect::Output, amount: 1.0, multiplier: 1.0 },
        ],
    }
}

fn totals(v: &[(&str, i64)]) -> Totals {
    v.iter().map(|(k, x)| (k.to_string(), *x)).collect()
}

#[test]
fn size_adds_bonus_and_province_size() {
    let r = rules();
    let rgo = farm();
    let mut bonus = ProductionBonus::default();
    let mut e = ProductionEffects::default();
    e.values.insert("f_size".into(), 0.1);
    e.per_good.insert("g_size".into(), BTreeMap::from([("rock".into(), 0.2)]));
    bonus.add(&r, &e);
    let province = totals(&[("rock_size", 100), ("f_size", 100), ("m_size", 900)]);
    let employed = BTreeMap::new();
    let none = Totals::new();
    let inp = RgoInputs { rgo: &rgo, base: 4, bonus: &bonus, real_country: true, country: &none, province: &province, employed: &employed, owner_share: (0, 0), mobilisation: Fixed::ONE, overseas_penalty: Fixed::ZERO };
    // 4 x (1 + 0.3 + 0.2): each thousandths group truncated to fixed point separately.
    let expect = (r.fx(300) + r.fx(200) + Fixed::ONE) * Fixed(4 << 15);
    assert_eq!(rgo_size(&r, &inp), expect);
    // Minimum size.
    let tiny = RgoInputs { base: 0, ..inp };
    assert_eq!(rgo_size(&r, &tiny), Fixed(300));
}

#[test]
fn setup_base_employs_everyone_then_scales() {
    let r = rules();
    let rgo = farm();
    let none = Totals::new();
    // 2500 workers need 3 steps of 1000 -> 3 x 1.5 = 4.
    assert_eq!(setup_base_size(&r, &rgo, &none, 2500, true), 4);
    assert_eq!(setup_base_size(&r, &rgo, &none, 1000, true), 1);
    // Owner not in control: one step at most.
    assert_eq!(setup_base_size(&r, &rgo, &none, 2500, false), 3);
}

#[test]
fn factors_and_output() {
    let r = rules();
    let rgo = farm();
    let bonus = ProductionBonus::default();
    let employed = BTreeMap::from([("diggers".to_string(), 1500i64), ("serfs".to_string(), 500)]);
    let country = totals(&[("n_out", 100), ("f_eff", 50)]);
    let province = totals(&[("l_thr", 200)]);
    let size = Fixed(2 << 15);
    let inp = RgoInputs { rgo: &rgo, base: 2, bonus: &bonus, real_country: true, country: &country, province: &province, employed: &employed, owner_share: (100, 1000), mobilisation: Fixed::ONE, overseas_penalty: Fixed::ZERO };
    let f = rgo_factors(&r, &inp, size);
    // The country's farm efficiency (0.05) counts for every factor.
    assert_eq!(f[0], r.fx(50) + Fixed::ONE);
    // Output: (1 + 500/2000) x (1 + 0.1 + 0.05 + owner 0.1).
    let owner = Fixed((100i64 << 30) / (1000 << 15));
    assert_eq!(f[1], (r.fx(100) + r.fx(50) + owner + Fixed::ONE) * (Fixed::ONE + Fixed((500i64 << 30) / (2000 << 15))));
    // Throughput: 1500/2000 x (1 + 0.2 + 0.05).
    let thr = (r.fx(200) + r.fx(50) + Fixed::ONE) * Fixed((1500i64 << 30) / (2000 << 15));
    assert_eq!(f[2], thr);
    assert_eq!(rgo_output(&rgo, f, size), f[2] * f[1] * Fixed::from_f64(2.0) * size);
    // Overseas penalty reduces throughput only; never below zero.
    let over = RgoInputs { overseas_penalty: Fixed(8192), ..inp.clone() };
    assert_eq!(rgo_factors(&r, &over, size)[2], thr * Fixed(32768 - 8192));
    let gone = RgoInputs { overseas_penalty: Fixed(40000), ..inp };
    assert_eq!(rgo_factors(&r, &gone, size)[2], Fixed::ZERO);
}

fn hire_rules() -> HireRules {
    HireRules {
        fire_above: Fixed::from_f32(1.05),
        hire_hi_below: Fixed::from_f32(0.9),
        hire_lo_below: Fixed::ONE,
        hire_hi: 200,
        hire_lo: 20,
        fire: 400,
        hire_lowest: 1,
        fire_lowest: 1,
        min_step: 50,
        fire_divisor: 4,
        recent_days: 14,
        recent_factor: 25,
        scale: 1000,
    }
}

#[test]
fn hiring_follows_supply_and_demand() {
    let r = hire_rules();
    let one = Fixed::ONE;
    // Oversupply 1.5: fire at 0.4 x 0.5.
    let (hire, rate) = hiring_order(&r, Fixed(3 << 15), Fixed(2 << 15), false, None);
    assert!(!hire);
    assert_eq!(rate, Fixed((400 << 15) / 1000) * Fixed(1 << 14));
    // Shortage 0.5: hire at 0.2 x 0.5; recently conquered: 0.2 x 25 unscaled.
    assert_eq!(hiring_order(&r, Fixed(1 << 15), Fixed(2 << 15), false, None), (true, Fixed((200 << 15) / 1000) * Fixed(1 << 14)));
    assert_eq!(hiring_order(&r, Fixed(1 << 15), Fixed(2 << 15), false, Some(3)), (true, Fixed((5000 << 15) / 1000)));
    // Balanced or no demand.
    assert_eq!(hiring_order(&r, one, one, false, None), (true, Fixed::ZERO));
    assert_eq!(hiring_order(&r, one, Fixed::ZERO, false, None), (true, one));
    assert_eq!(hiring_order(&r, Fixed(9 << 15), one, true, None), (true, one));
}

#[test]
fn employment_steps_have_floors_and_stop_at_target() {
    let r = hire_rules();
    let half = Fixed(1 << 14);
    assert_eq!(employment_step(&r, 1000, 3000, half), 1000);
    assert_eq!(employment_step(&r, 1000, 1010, half), 10, "never past the target");
    assert_eq!(employment_step(&r, 1000, 1100, Fixed(100)), 50, "at least 50");
    // Firing: quarter speed, floor 50, not below the target.
    assert_eq!(employment_step(&r, 10_000, 0, half), -1250);
    assert_eq!(employment_step(&r, 10_000, 9_990, half), -10);
    assert_eq!(employment_step(&r, 1000, 0, Fixed(1)), -50);
}

