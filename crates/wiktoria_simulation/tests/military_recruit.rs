//! Units under construction and the computer's unit choice, on made-up numbers.

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_simulation::military::recruit::*;
use wiktoria_simulation::military::units::UnitDef;

fn unit(name: &str, kind: &str, priority: f64, active: bool) -> UnitDef {
    UnitDef { name: name.into(), land: true, kind: kind.into(), priority, active, build_cost: [("small_arms".to_string(), 10.0), ("canned_food".to_string(), 10.0)].into(), ..UnitDef::default() }
}

fn goods(list: &[(&str, f64)]) -> BTreeMap<String, f64> {
    list.iter().map(|(g, x)| (g.to_string(), *x)).collect()
}

#[test]
fn a_unit_is_ordered_with_its_cost_times_two_less_efficiency() {
    // Measured: Britain's artillery regiment carries 17.45911 of each good at cost 10.
    let g = order_goods(&unit("infantry", "infantry", 10.0, true), 0.254089);
    assert!((g["small_arms"] - 17.45911).abs() < 1e-5);
}

#[test]
fn sites_take_what_the_stockpile_has_and_ask_a_tenth_of_one_list() {
    let mut lacks = goods(&[("artillery", 17.0), ("wine", 1.0)]);
    let mut stock = goods(&[("wine", 3.0)]);
    assert!(!site_takes(&mut lacks, &mut stock));
    assert_eq!(lacks, goods(&[("artillery", 17.0)]));
    assert!((stock["wine"] - 2.0).abs() < 1e-9);
    let full = goods(&[("artillery", 20.0), ("wine", 20.0)]);
    let other = goods(&[("artillery", 1.0)]);
    let ask = sites_ask([(&lacks, &full), (&other, &full)].into_iter(), 0.1);
    // The largest single ask, at most what the site still lacks.
    assert_eq!(ask, goods(&[("artillery", 2.0)]));
}

#[test]
fn the_best_unlocked_unit_of_a_class_is_picked() {
    let units: BTreeMap<String, UnitDef> = [unit("irregular", "infantry", 5.0, true), unit("infantry", "infantry", 10.0, false), UnitDef { primary_culture_only: true, ..unit("guard", "infantry", 18.0, false) }].into_iter().map(|u| (u.name.clone(), u)).collect();
    let mut unlocked: BTreeSet<String> = ["irregular".to_string()].into();
    assert_eq!(best_unit(&units, &unlocked, "infantry").map(|u| u.name.as_str()), Some("irregular"));
    unlocked.extend(["infantry".to_string(), "guard".to_string()]);
    assert_eq!(best_unit(&units, &unlocked, "infantry").map(|u| u.name.as_str()), Some("infantry"));
}

#[test]
fn the_land_mix_fills_artillery_then_cavalry() {
    assert_eq!(land_class(50, 1, 6, true), "support");
    assert_eq!(land_class(70, 18, 6, true), "cavalry");
    assert_eq!(land_class(60, 25, 15, true), "infantry");
    // Without artillery: a tenth cavalry.
    assert_eq!(land_class(20, 0, 0, false), "cavalry");
    assert_eq!(sea_class(0, 0, 0), "light_ship");
}
