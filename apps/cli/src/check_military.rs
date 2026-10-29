//! `wiktoria-cli check-military <save.v2>` 
//!
//! Loads the military state of an original save and prints counts per great power (armies,
//! regiments, ships, leaders, running wars, army size) and for the whole world, so they can be
//! compared with the save's own text and, year by year, with a run of the port.

use std::collections::{BTreeMap, HashSet};

use wiktoria_simulation::military::{MilitaryRules, MilitaryState};

use crate::{config, load_world, root, Args};

pub fn check_military(args: &Args) -> Result<(), String> {
    let usage = "usage: check-military <save.v2>";
    let path = args.save.as_ref().or(args.positional.first()).ok_or(usage)?;
    let w = load_world(args)?;
    let engine_rules = config::load_engine_rules(&root())?;
    let install = config::install_dir(&root(), args.install.clone())?;
    let rules = MilitaryRules::new(&install, &engine_rules)?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
    let m = MilitaryState::from_save(&save, &rules);

    println!("{path}: {} ({} unit types in the install)", m.date, rules.units.len());
    println!();
    println!("great powers (save order)");
    println!("{:<5} {:>6} {:>9} {:>6} {:>7} {:>7} {:>10} {:>10}  active wars", "tag", "armies", "regiments", "ships", "capital", "leaders", "avg men", "max men");
    let mut total = Totals::default();
    for tag in m.countries.iter().filter(|(_, c)| c.great_power).map(|(t, _)| t) {
        let row = row(&m, &rules, tag);
        let wars: Vec<&str> = m.active_wars_of(tag).map(|x| x.name.as_str()).collect();
        println!("{tag:<5} {:>6} {:>9} {:>6} {:>7} {:>6} {:>10.0} {:>10.0}  {} {}", row.armies, row.regiments, row.ships, row.capital, row.leaders, row.avg_men, row.max_men, wars.len(), if m.countries[tag].mobilized { "(mobilised)" } else { "" });
        total.add(&row);
    }
    println!("{:<5} {:>6} {:>9} {:>6} {:>7} {:>6}", "sum", total.armies, total.regiments, total.ships, total.capital, total.leaders);

    println!();
    println!("world");
    let mut all = Totals::default();
    for tag in m.countries.keys() {
        all.add(&row(&m, &rules, tag));
    }
    let carried = m.armies.iter().filter(|a| a.carried_by.is_some()).count();
    println!("countries with forces: {}", m.countries.keys().filter(|t| m.regiment_count(t) + m.ship_count(t) + m.leaders_of(t).count() > 0).count());
    println!("armies {} (of them on ships {carried}), regiments {}, fleets {}, ships {}, leaders {}", all.armies, all.regiments, m.navies.len(), all.ships, all.leaders);
    println!("units under construction {}", m.constructions.len());
    let active = m.wars.iter().filter(|x| x.active).count();
    println!("wars: {active} running, {} finished; goals in force {}; battles in the histories {}", m.wars.len() - active, m.wars.iter().map(|x| x.goals.len()).sum::<usize>(), m.wars.iter().map(|x| x.battles().count()).sum::<usize>());
    println!("combats in progress: {} land, {} naval, {} sieges", count(&m, wiktoria_simulation::military::battle::CombatKind::Land), count(&m, wiktoria_simulation::military::battle::CombatKind::Naval), count(&m, wiktoria_simulation::military::battle::CombatKind::Siege));
    println!("provinces under another controller than their owner: {}", m.occupied().count());
    let mobilised: Vec<&String> = m.countries.iter().filter(|(_, c)| c.mobilized).map(|(t, _)| t).collect();
    println!("mobilised: {}", if mobilised.is_empty() { "none".to_string() } else { mobilised.iter().map(|t| t.as_str()).collect::<Vec<_>>().join(" ") });

    println!();
    println!("running wars");
    for x in m.wars.iter().filter(|x| x.active) {
        println!("  {}: {} against {}  since {}, {} goals, {} battles", x.name, x.attackers.join("+"), x.defenders.join("+"), x.started().map(|d| d.to_string()).unwrap_or_default(), x.goals.len(), x.battles().count());
    }

    println!();
    println!("regiments by unit type (all countries)");
    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    for r in m.armies.iter().flat_map(|a| &a.regiments) {
        *by_kind.entry(r.kind.as_str()).or_default() += 1;
    }
    println!("  {}", by_kind.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", "));
    let unknown: Vec<&str> = by_kind.keys().copied().filter(|k| !rules.units.contains_key(*k)).collect();
    if !unknown.is_empty() {
        println!("  unit types the install does not define: {}", unknown.join(", "));
    }
    if !m.warnings.is_empty() {
        println!();
        println!("{} loader warnings, first: {}", m.warnings.len(), m.warnings[0]);
    }
    Ok(())
}

fn count(m: &MilitaryState, kind: wiktoria_simulation::military::battle::CombatKind) -> usize {
    m.combats.iter().filter(|c| c.kind == kind).count()
}

#[derive(Default)]
struct Row {
    armies: usize,
    regiments: usize,
    ships: usize,
    capital: usize,
    leaders: usize,
    avg_men: f64,
    max_men: f64,
}

#[derive(Default)]
struct Totals {
    armies: usize,
    regiments: usize,
    ships: usize,
    capital: usize,
    leaders: usize,
}

impl Totals {
    fn add(&mut self, r: &Row) {
        self.armies += r.armies;
        self.regiments += r.regiments;
        self.ships += r.ships;
        self.capital += r.capital;
        self.leaders += r.leaders;
    }
}

/// Counts for one country. Armies on ships are counted as armies of the country; the average
/// and the largest army are in men (strength x men per strength point).
fn row(m: &MilitaryState, rules: &MilitaryRules, tag: &str) -> Row {
    let men: Vec<f64> = m.armies_of(tag).map(|a| a.men(rules.men_per_strength)).collect();
    Row {
        armies: men.len(),
        regiments: m.regiment_count(tag),
        ships: m.ship_count(tag),
        capital: m.navies_of(tag).flat_map(|n| &n.ships).filter(|s| rules.units.get(&s.kind).is_some_and(|u| u.capital)).count(),
        leaders: m.leaders_of(tag).count(),
        avg_men: if men.is_empty() { 0.0 } else { men.iter().sum::<f64>() / men.len() as f64 },
        max_men: men.iter().copied().fold(0.0, f64::max),
    }
}
