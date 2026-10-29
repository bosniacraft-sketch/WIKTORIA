//! `wiktoria-cli check-diplomacy <save.v2>`
//!
//! Loads the diplomacy state of an original save and prints the counts that can be recounted from
//! the save's text: alliances, vassals, substates, guarantees, spheres per great power, influence
//! and diplomatic points of the great powers, military access links and truces.

use std::collections::HashSet;

use wiktoria_simulation::diplomacy::relations::level;
use wiktoria_simulation::diplomacy::DiplomacyState;

use crate::{load_world, Args};

pub fn check_diplomacy(args: &Args) -> Result<(), String> {
    let path = args.save.as_ref().or(args.positional.first()).ok_or("usage: check-diplomacy <save.v2>")?;
    let w = load_world(args)?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
    let d = DiplomacyState::from_save(&save);

    println!("{path}: {}", save.date);
    println!("alliances {}, vassals {}, substates {}, guarantees {}", d.alliances.len(), d.subjects.iter().filter(|s| s.kind == wiktoria_simulation::diplomacy::SubjectKind::Vassal).count(), d.subjects.iter().filter(|s| s.kind == wiktoria_simulation::diplomacy::SubjectKind::Substate).count(), d.guarantees.len());
    let access = d.relations.values().filter(|r| r.access).count();
    let truces = d.relations.values().filter(|r| r.truce_until.is_some_and(|t| t >= save.date)).count();
    println!("relation records {}, military access links {access}, running truces (one direction each) {truces}", d.relations.len());
    println!();
    println!("great powers");
    println!("{:<5} {:>7} {:>7} {:>9} {:>10} {:>8}", "tag", "sphere", "vassals", "influence", "in >=50", "points");
    let mut sphere_sum = 0;
    for g in d.great_powers() {
        let sphere = d.sphere_members(g).count();
        let vassals = d.subjects_of(g).filter(|s| s.kind == wiktoria_simulation::diplomacy::SubjectKind::Vassal).count();
        let held: Vec<f64> = d.relations.iter().filter(|((a, _), _)| a == g).map(|(_, r)| r.influence).collect();
        println!("{g:<5} {sphere:>7} {vassals:>7} {:>9.1} {:>10} {:>8.2}", held.iter().sum::<f64>(), held.iter().filter(|x| **x >= 50.0).count(), d.points(g));
        sphere_sum += sphere;
    }
    println!("sum   {sphere_sum:>7}");
    let others = d.relations.iter().filter(|((a, _), r)| r.level == level::IN_SPHERE && !d.is_great_power(a)).count();
    println!("level-5 records held by countries that are not great powers: {others}");
    println!();
    let overlords: std::collections::BTreeMap<&str, usize> = d.subjects.iter().fold(Default::default(), |mut m, s| {
        *m.entry(s.overlord.as_str()).or_default() += 1;
        m
    });
    println!("subjects by overlord: {}", overlords.iter().map(|(t, n)| format!("{t} {n}")).collect::<Vec<_>>().join(", "));
    if !d.warnings.is_empty() {
        println!("{} loader warnings, first: {}", d.warnings.len(), d.warnings[0]);
    }
    Ok(())
}
