//! `wiktoria-cli check-unrest <save.v2>`: factions and movements per country, from the save's own text.
//!
//! Each movement's size is recounted from the pops that carry it (`movement_issue` / `movement_tag`)
//! and each faction's members from the pops that carry its id, then compared with what the save
//! writes (`support`, the faction's `pop` list). The radicalism the rules give is printed next to
//! the save's cached value.

use std::collections::{BTreeMap, HashSet};

use wiktoria_simulation::unrest::{self, MovementKind, UnrestRules, UnrestState};

use crate::{config, load_world, root, Args};

pub fn check_unrest(args: &Args) -> Result<(), String> {
    let usage = "usage: check-unrest <save.v2>";
    let path = args.save.as_ref().or(args.positional.first()).ok_or(usage)?;
    let w = load_world(args)?;
    let engine_rules = config::load_engine_rules(&root())?;
    let install = config::install_dir(&root(), args.install.clone())?;
    let rules = UnrestRules::new(&engine_rules, &w.defines)?;
    let types = unrest::RebelType::parse_file(&std::fs::read(install.join("common/rebel_types.txt")).map_err(|e| format!("rebel_types.txt: {e}"))?)?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
    let s = UnrestState::from_save(&save, &w.defs.cultures, &w.defs.issues);

    println!("{path}: {} ({} rebel types in the install)", s.date.map(|d| d.to_string()).unwrap_or_default(), types.len());
    println!();
    println!("factions per country");
    let mut by_country: BTreeMap<&str, Vec<&unrest::Faction>> = BTreeMap::new();
    for f in &s.factions {
        by_country.entry(f.country.as_str()).or_default().push(f);
    }
    let mut member_mismatch = 0;
    let mut unknown_type = 0;
    for (tag, fs) in &by_country {
        let line: Vec<String> = fs
            .iter()
            .map(|f| {
                let carried = s.pop_faction.values().filter(|id| **id == f.id).count();
                if carried != f.pops.len() {
                    member_mismatch += 1;
                }
                if !types.iter().any(|t| t.name == f.kind) {
                    unknown_type += 1;
                }
                let creates = types
                    .iter()
                    .find(|t| t.name == f.kind)
                    .and_then(|t| unrest::faction_country(t, &f.culture, f.creates.as_deref(), &w.defs.cultures))
                    .map(|c| format!(">{c}"))
                    .unwrap_or_default();
                format!("{}[{} pops, org {:.2}{creates}]", f.kind, f.pops.len(), f.organization)
            })
            .collect();
        println!("  {tag}: {} {}", fs.len(), line.join(" "));
    }
    let members: usize = s.factions.iter().map(|f| f.pops.len()).sum();
    println!("factions {} in {} countries, member pops {} (pops carrying a faction id: {}), mismatches {member_mismatch}, types unknown to the install {unknown_type}", s.factions.len(), by_country.len(), members, s.pop_faction.len());
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for f in &s.factions {
        *kinds.entry(f.kind.as_str()).or_default() += 1;
    }
    println!("by type: {}", kinds.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", "));

    println!();
    println!("movements per country (issue / independence, size recounted from pops vs save, radicalism rule vs save cache)");
    let mut by_c: BTreeMap<&str, Vec<&unrest::Movement>> = BTreeMap::new();
    for m in &s.movements {
        by_c.entry(m.country.as_str()).or_default().push(m);
    }
    let mut size_mismatch = 0;
    let mut radical_off = 0;
    for (tag, ms) in &by_c {
        let pop = s.population.get(*tag).copied().unwrap_or(0.0);
        let steps = s.reform_steps.get(*tag).copied().unwrap_or(0.0);
        let supp = s.suppression.get(*tag).copied().unwrap_or(0.0);
        let issues = ms.iter().filter(|m| m.kind == MovementKind::Issue).count();
        println!("  {tag}: {} issue, {} independence (population {:.0}, reform steps {steps}, suppression {supp:.1})", issues, ms.len() - issues, pop);
        for m in ms {
            let n = s.recount_support(tag, m.kind, &m.target);
            let ok = (n - m.support).abs() < 1.0;
            if !ok {
                size_mismatch += 1;
            }
            let independence = (m.kind == MovementKind::Independence).then(|| {
                let culture_value = s.primary_culture.get(&m.target).and_then(|c| w.defs.cultures.index.get(c)).and_then(|i| w.defs.cultures.cultures.get(i as usize)).map(|c| c.radicalism).unwrap_or(0.0);
                (culture_value, s.nationalism_for(tag, &m.target))
            });
            let rad = unrest::radicalism(&rules, &unrest::RadicalismInput { suppression_part: 0.0, reform_steps_passed: steps, size: n, country_population: pop, independence });
            if (rad - m.radicalism).abs() > 0.5 {
                radical_off += 1;
            }
            println!("      {:<28} size {:>9.0} (save {:>9.0}){} radicalism {:>6.2} (cache {:>6.2})", m.target, n, m.support, if ok { "" } else { " MISMATCH" }, rad, m.radicalism);
        }
    }
    let issue = s.movements.iter().filter(|m| m.kind == MovementKind::Issue).count();
    println!("movements {} ({issue} issue, {} independence) in {} countries, size mismatches {size_mismatch}; pops in a movement {}", s.movements.len(), s.movements.len() - issue, by_c.len(), s.pop_movement.len());
    println!("radicalism (base, reform steps, size share, and for independence the culture and nationalism terms) differs from the cache by more than 0.5 in {radical_off} movements");
    if !s.warnings.is_empty() {
        println!("{} loader warnings, first: {}", s.warnings.len(), s.warnings[0]);
    }
    Ok(())
}
