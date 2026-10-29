//! Synthetic tests for the day-to-day pop comparison (no game files).

use std::collections::{BTreeMap, HashMap, HashSet};

use wiktoria_data::date::Date;
use wiktoria_data::popdiff::{self, DayBasis, Field, NewPopSource, Snapshot};
use wiktoria_data::save::{parse_save, SavePop};

fn pop(id: u64, ty: &str, size: f64, religion: &str, ideology: f64) -> SavePop {
    SavePop {
        pop_type: ty.into(),
        id,
        size,
        culture: "alpha".into(),
        religion: religion.into(),
        money: 0.0,
        literacy: 0.1,
        militancy: 0.0,
        consciousness: 0.0,
        ideology: BTreeMap::from([(1, ideology)]),
        issues: BTreeMap::new(),
        life_needs: 1.0,
        everyday_needs: 1.0,
        luxury_needs: 1.0,
    }
}

fn snap(date: Date, pops: Vec<(u16, SavePop)>) -> Snapshot {
    Snapshot { date, pops: pops.into_iter().map(|(prov, p)| (p.id, (prov, p))).collect::<HashMap<_, _>>() }
}

#[test]
fn reads_ideology_and_issues_from_a_save() {
    let text = "date=\"1900.3.4\"\n1={\n name=\"A\"\n owner=\"AAA\"\n workers={\n  id=7\n  size=100\n  alpha=sun\n  money=1.5\n  ideology={\n2=10.5\n3=89.5\n  }\n  issues={\n1=4.0\n  }\n  con=2.0\n  literacy=0.5\n }\n}\n";
    let types = HashSet::from(["workers".to_string()]);
    let cultures = HashSet::from(["alpha".to_string()]);
    let s = parse_save(text.as_bytes(), &types, &cultures).unwrap();
    let p = &s.provinces[&1].pops[0];
    assert_eq!(p.ideology, BTreeMap::from([(2, 10.5), (3, 89.5)]));
    assert_eq!(p.issues, BTreeMap::from([(1, 4.0)]));
    assert_eq!((p.id, p.consciousness), (7, 2.0));
}

#[test]
fn counts_daily_changes() {
    let a = snap(Date::new(1900, 1, 1), vec![(1, pop(1, "workers", 100.0, "sun", 50.0)), (1, pop(2, "workers", 50.0, "sun", 50.0)), (2, pop(3, "nobles", 5.0, "sun", 50.0))]);
    let b = snap(Date::new(1900, 1, 2), vec![(1, pop(1, "workers", 104.0, "sun", 60.0)), (1, pop(2, "workers", 50.0, "sun", 50.0)), (1, pop(4, "workers", 3.0, "moon", 50.0))]);
    let d = popdiff::diff(&a, &b);
    assert_eq!((d.pops, d.new, d.gone, d.grew, d.shrank), (3, 1, 1, 1, 0));
    assert_eq!(d.changed.get(&Field::Ideology), Some(&1));
    assert_eq!(d.changed.get(&Field::Size), Some(&1));
    assert_eq!(d.changed.get(&Field::Money), None);
}

#[test]
fn finds_the_pop_update_schedule() {
    // 62 pops; pop i updates its ideology on day-of-month d when (d - i) mod 31 == 1.
    let mut series = Vec::new();
    let mut values: Vec<f64> = vec![0.0; 62];
    for day in 1..=20u8 {
        let mut pops = Vec::new();
        for (i, v) in values.iter_mut().enumerate() {
            if day > 1 && (i64::from(day) - i as i64).rem_euclid(31) == 1 {
                *v += 1.0;
            }
            pops.push((1, pop(i as u64, "workers", 10.0, "sun", *v)));
        }
        series.push(snap(Date::new(1900, 1, day), pops));
    }
    let fit = popdiff::fit_schedule(&series, Field::Ideology, DayBasis::DayOfMonth, &[29, 30, 31, 32], None).unwrap();
    assert_eq!((fit.modulus, fit.residue, fit.share), (31, 1, 1.0));
    assert_eq!(fit.changes, 38); // days 2..=20, two pops each
}

#[test]
fn running_days_cross_month_ends() {
    let s = |m, d| snap(Date::new(1900, m, d), vec![]);
    let series = vec![s(1, 30), s(1, 31), s(2, 1), s(2, 2)];
    assert_eq!(popdiff::running_days(&series), vec![0, 1, 2, 3]);
}

#[test]
fn classifies_new_pops() {
    let a = snap(Date::new(1900, 1, 1), vec![(1, pop(1, "workers", 100.0, "sun", 0.0)), (2, pop(2, "workers", 100.0, "sun", 0.0))]);
    let b = snap(
        Date::new(1900, 1, 2),
        vec![
            (1, pop(1, "workers", 93.0, "sun", 0.0)),
            (1, pop(10, "workers", 7.0, "moon", 0.0)), // conversion split
            (2, pop(2, "workers", 100.0, "sun", 0.0)),
            (2, pop(11, "workers", 40.0, "sun", 0.0)), // nobody in province 2 shrank
        ],
    );
    let mut got: Vec<_> = popdiff::new_pop_sources(&a, &b).into_iter().map(|(s, _)| s).collect();
    got.sort();
    assert_eq!(got, vec![NewPopSource::SplitFrom { differs: vec!["religion"] }, NewPopSource::NoLocalDonor]);
}
