//! `wiktoria-cli run-military <save.v2> [--days N]`: runs the military day step alone (no economy, no
//! AI) from an original save and prints a line a month.

use std::collections::{BTreeMap, HashSet};

use wiktoria_data::date::Date;
use wiktoria_simulation::military::battle::CombatKind;
use wiktoria_simulation::military::day::{DayReport, MilitaryDay};
use wiktoria_simulation::military::{MilitaryRules, MilitaryState};
use wiktoria_simulation::script::State;
use wiktoria_simulation::GameDate;

use crate::{config, load_world, root, Args};

#[derive(Default)]
struct Totals {
    battles_started: usize,
    battles_finished: usize,
    sieges_started: usize,
    sieges_ended: usize,
    controller_changes: usize,
    armies_destroyed: usize,
    fleets_destroyed: usize,
    men_lost: f64,
}

/// The wars' state in one place so the final comparison and the monthly line agree.
fn running(m: &MilitaryState) -> usize {
    m.wars.iter().filter(|w| w.active).count()
}

pub fn run_military(args: &Args) -> Result<(), String> {
    let usage = "usage: run-military <save.v2> [--days N]";
    let mut days = 365usize;
    let mut path: Option<&String> = args.save.as_ref();
    let mut it = args.positional.iter();
    while let Some(a) = it.next() {
        if a == "--days" {
            days = it.next().and_then(|n| n.parse().ok()).ok_or("--days needs a number")?;
        } else if path.is_none() {
            path = Some(a);
        }
    }
    let path = path.ok_or(usage)?;
    let w = load_world(args)?;
    let engine_rules = config::load_engine_rules(&root())?;
    let install = config::install_dir(&root(), args.install.clone())?;
    let rules = MilitaryRules::new(&install, &engine_rules)?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
    let mut mil = MilitaryState::from_save(&save, &rules);
    let mut state = State::from_save(&save);
    state.attach_world(&w, &save);
    let mut day = MilitaryDay::from_world(&w, &engine_rules, &install, &rules, 1836)?;

    println!("{path}: {} for {days} days, military step only (no economy, no AI)", save.date);
    println!("{} armies, {} fleets, {} running wars, {} combats, {} provinces under another controller", mil.armies.len(), mil.navies.len(), running(&mil), mil.combats.len(), mil.occupied().count());
    // Calibration of FLD_MIL_DAY_DISTANCE_SCALE: land borders, in days for an infantry regiment on plains.
    let infantry = rules.units.get("infantry").map_or(4.0, |u| u.speed) * day.movement.land_speed_modifier;
    let mut days_per_step: Vec<f64> = day.map.borders.iter().filter(|((a, b), _)| a < b && day.map.provinces.get(a).is_some_and(|p| !p.sea) && day.map.provinces.get(b).is_some_and(|p| !p.sea)).map(|(_, b)| b.distance / infantry).collect();
    days_per_step.sort_by(f64::total_cmp);
    if !days_per_step.is_empty() {
        let q = |p: f64| days_per_step[((days_per_step.len() - 1) as f64 * p) as usize];
        println!("land border step for infantry (speed {infantry}): {:.1} days median, {:.1} at the 10th and {:.1} at the 90th percentile", q(0.5), q(0.1), q(0.9));
    }
    let mut total = Totals::default();
    let mut month = Totals::default();
    let mut g = day.calendar.to_game(save.date).ok_or("save date outside the calendar")?;
    let t = std::time::Instant::now();
    for n in 0..days {
        let date = day.calendar.to_date(g);
        let rep = day.day(&mut mil, &state, date);
        add(&mut month, &rep);
        add(&mut total, &rep);
        // The caller's side of the day: provinces change controller in the read-only state's copy too.
        for c in &rep.controller_changes {
            if let Some(p) = state.provinces.get_mut(&c.province) {
                p.controller = c.to.clone();
            }
        }
        if date.day == 1 || n + 1 == days {
            print_month(&day, &mil, &state, date, &month);
            month = Totals::default();
        }
        g = GameDate(g.0 + 24);
    }
    eprintln!("ran {days} days in {:.2?}", t.elapsed());
    println!();
    println!("totals over {days} days");
    println!("  battles started {}, finished {} ({} still running)", total.battles_started, total.battles_finished, mil.combats.iter().filter(|c| c.kind != CombatKind::Siege).count());
    println!("  sieges started {}, ended {} ({} still running)", total.sieges_started, total.sieges_ended, mil.sieges().count());
    println!("  provinces that changed controller {} (occupied now: {})", total.controller_changes, mil.occupied().count());
    println!("  armies destroyed {}, fleets destroyed {}, men lost {:.0}", total.armies_destroyed, total.fleets_destroyed, total.men_lost);
    println!("  running wars {} (finished {})", running(&mil), mil.wars.len() - running(&mil));
    println!("  armies {} with {} regiments, fleets {}", mil.armies.len(), mil.armies.iter().map(|a| a.regiments.len()).sum::<usize>(), mil.navies.len());
    let garrisons: Vec<f64> = mil.provinces.values().map(|p| p.garrison).collect();
    println!("  garrison range {:.1} to {:.1}", garrisons.iter().copied().fold(f64::INFINITY, f64::min), garrisons.iter().copied().fold(f64::NEG_INFINITY, f64::max));
    println!();
    println!("occupied provinces at the end");
    let mut by_owner: BTreeMap<String, usize> = BTreeMap::new();
    for (_, p) in mil.occupied() {
        *by_owner.entry(p.owner.clone().unwrap_or_default()).or_default() += 1;
    }
    println!("  {}", if by_owner.is_empty() { "none".to_string() } else { by_owner.iter().map(|(t, n)| format!("{t} {n}")).collect::<Vec<_>>().join(", ") });
    println!("running wars at the end");
    let scores = day.war_scores(&mil, &state);
    for x in mil.wars.iter().filter(|x| x.active) {
        let s = scores.iter().find(|(n, _)| *n == x.name).map_or(0.0, |(_, s)| *s);
        let exh: Vec<String> = x.attackers.iter().chain(&x.defenders).map(|t| format!("{t} {:.1}", mil.countries.get(t).map_or(0.0, |c| c.war_exhaustion))).collect();
        println!("  {}: {} against {}, score {s:+.0}, battles {}, exhaustion {}", x.name, x.attackers.join("+"), x.defenders.join("+"), x.battles().count(), exh.join(" "));
    }
    Ok(())
}

fn add(t: &mut Totals, r: &DayReport) {
    t.battles_started += r.battles_started;
    t.battles_finished += r.battles_finished;
    t.sieges_started += r.sieges_started;
    t.sieges_ended += r.sieges_ended;
    t.controller_changes += r.controller_changes.len();
    t.armies_destroyed += r.armies_destroyed;
    t.fleets_destroyed += r.fleets_destroyed;
    t.men_lost += r.casualties.iter().map(|(_, m)| m).sum::<f64>();
}

fn print_month(day: &MilitaryDay, mil: &MilitaryState, state: &State, date: Date, m: &Totals) {
    let battles = mil.combats.iter().filter(|c| c.kind != CombatKind::Siege).count();
    println!(
        "{date}: wars {} | battles now {battles}, finished {} | sieges now {}, started {} | controller changes {} | armies destroyed {} | men lost {:.0}",
        running(mil),
        m.battles_finished,
        mil.sieges().count(),
        m.sieges_started,
        m.controller_changes,
        m.armies_destroyed,
        m.men_lost,
    );
    let scores = day.war_scores(mil, state);
    if !scores.is_empty() {
        println!("    war score (attackers): {}", scores.iter().map(|(n, s)| format!("{n} {s:+.0}")).collect::<Vec<_>>().join("; "));
    }
    let mut at_war: Vec<(&String, f64)> = mil.countries.iter().filter(|(t, _)| mil.active_wars_of(t).next().is_some()).map(|(t, c)| (t, c.war_exhaustion)).collect();
    at_war.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
    if !at_war.is_empty() {
        println!("    war exhaustion: {}", at_war.iter().map(|(t, e)| format!("{t} {e:.1}")).collect::<Vec<_>>().join(", "));
    }
    let mut big: Vec<String> = Vec::new();
    for (tag, c) in mil.countries.iter().filter(|(_, c)| c.great_power) {
        let max = mil.armies_of(tag).map(|a| a.men(day.men_per_strength)).fold(0.0, f64::max);
        let _ = c;
        big.push(format!("{tag} {max:.0}"));
    }
    println!("    largest army (men): {}", big.join(", "));
}
