//! `wiktoria-cli check-scores --save <file.v2> [--tag FRA]`
//!
//! Loads a save of the original and prints, without moving time, three numbers the port computes
//! for one country with the parts they are made of, so they can be held against the original's
//! tooltips: the industrial score (per state, per country invested in), the colonial points
//! (technologies, naval bases, ships, supply, races, upkeep) and the cost of turning each of its
//! colonies into a state. The country is the save's player unless `--tag` names another.

use std::collections::HashSet;

use wiktoria_data::localisation::Localisation;
use wiktoria_data::script::Value;
use wiktoria_simulation::colonies::{self, Status};
use wiktoria_simulation::game::Game;

use crate::{config, load_world, root, Args};

pub fn check_scores(args: &Args) -> Result<(), String> {
    let usage = "usage: check-scores --save <file.v2> [--tag TAG]";
    let path = args.save.as_ref().or(args.positional.first()).ok_or(usage)?;
    let mut wanted = None;
    let mut it = args.positional.iter();
    while let Some(a) = it.next() {
        if a == "--tag" {
            wanted = Some(it.next().ok_or("--tag needs a country tag")?.to_uppercase());
        }
    }
    let w = load_world(args)?;
    let engine_rules = config::load_engine_rules(&root())?;
    let install = config::install_dir(&root(), args.install.clone())?;
    let mut loc = Localisation::default();
    for f in wiktoria_data::install_fs::InstallFs::with_mods(&install, &[])?.dir_files(engine_rules.require_str("FLD_LOCALISATION_DIR")?, "csv")? {
        loc.add_file(&wiktoria_data::script::decode_windows1252(&std::fs::read(&f).map_err(|e| format!("cannot read {}: {e}", f.display()))?));
    }
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
    let tag = wanted.or_else(|| save.doc.root.get("player").and_then(Value::as_str).map(str::to_string)).ok_or("the save names no player: give --tag")?;
    let game = Game::new(w, &engine_rules, &install, &save)?;
    let state = &game.state;
    let country = state.countries.get(&tag).ok_or(format!("no country {tag} in the save"))?;
    let name = |key: &str| loc.get(key).unwrap_or(key).to_string();
    let province_name = |id: u16| name(&format!("PROV{id}"));
    let state_name = |region: Option<u16>| region.and_then(|r| state.regions.get(r as usize)).map_or("(no state)".to_string(), |k| name(k));

    println!("{path}: {} on {}", name(&tag), state.date);

    // ---- industrial score ----
    let rules = &game.econ.rank_rules;
    let numbers = &state.economy_numbers;
    println!();
    println!("INDUSTRIAL SCORE");
    println!("  a state gives {} x factory levels x the share of its factory places its craftsmen and clerks could fill", numbers.industrial_score_per_level);
    println!("  (people and places in whole hundreds, the share between {} and 1)", numbers.industrial_score_min_staff);
    println!("  {:<28} {:>6} {:>9} {:>9} {:>7} {:>8}", "state", "levels", "people", "places", "share", "score");
    let mut rows: Vec<(String, f64, f64, f64, f64)> = state.industrial_states(&tag).into_iter().map(|(region, (levels, places, people))| (state_name(region), levels, places, people, wiktoria_simulation::economy::industrial_state_score(numbers, levels, places, people))).collect();
    rows.sort_by(|a, b| b.4.total_cmp(&a.4).then(a.0.cmp(&b.0)));
    for (state, levels, places, people, score) in &rows {
        let share = if *places >= 100.0 { (people / 100.0).floor() / (places / 100.0).floor() } else { 0.0 };
        println!("  {state:<28} {levels:>6} {people:>9.0} {places:>9.0} {share:>7.3} {score:>8.3}");
    }
    let from_states: f64 = rows.iter().map(|r| r.4).sum();
    println!("  {:<28} {:>43.3}", "all states", from_states);
    let mut abroad = wiktoria_simulation::rank::invested_abroad(state, &tag);
    abroad.sort_by(|a, b| b.1.total_cmp(&a.1));
    println!("  money invested abroad counts x {} (0.01 x INVESTMENT_SCORE_FACTOR {})", wiktoria_simulation::rank::investment_score(rules, 1.0), rules.investment_score_factor);
    for (host, money) in &abroad {
        println!("  {:<28} {:>34.2} {:>8.3}", format!("invested in {}", name(host)), money, wiktoria_simulation::rank::investment_score(rules, *money));
    }
    let from_abroad: f64 = abroad.iter().map(|(_, m)| wiktoria_simulation::rank::investment_score(rules, *m)).sum();
    let all = wiktoria_simulation::rank::scores(state, rules, &game.world.defs.modifiers);
    let ranked = all.get(&tag).map_or(0.0, |s| s[0]);
    println!("  INDUSTRIAL SCORE {:>55.3}   (the ranking uses {ranked:.3}; the original shows it cut to a whole number: {})", from_states + from_abroad, (from_states + from_abroad).floor());

    // ---- colonial points ----
    let order = wiktoria_simulation::economy::rank_order(all.iter().filter_map(|(t, x)| Some((t.clone(), state.countries.get(t)?.civilized, x.iter().sum::<f64>()))).collect());
    let rank = order.iter().position(|t| *t == tag).map_or(f64::INFINITY, |i| (i + 1) as f64);
    println!();
    println!("COLONIAL POINTS (overall rank {rank} by the port's scores)");
    let p = game.core.colonial_points_of(state, &tag, rank).ok_or("the colonial rules are not attached")?;
    for (tech, points) in &p.techs {
        println!("  technology {:<34} {points:>8.1}", name(tech));
    }
    for (province, level, points) in &p.bases {
        println!("  naval base {:<22} level {level}      {points:>8.1}", province_name(*province));
    }
    for (region, points) in &p.overseas {
        println!("  coastal state overseas, no core: {:<12} {points:>8.1}", state_name(*region));
    }
    println!("  {:<45} {:>8.1}", "all naval bases and states overseas", p.bases.iter().map(|b| b.2).sum::<f64>() + p.overseas.iter().map(|b| b.1).sum::<f64>());
    for (kind, n, each) in &p.ships {
        println!("  ships {:<24} {n:>4} x {each:<8} {:>8.1}", name(kind), n * each);
    }
    let ship_points: f64 = p.ships.iter().map(|s| s.1 * s.2).sum();
    println!("  naval supply: the ships need {} of {} available: {:.1}% over, so the ships' {ship_points} points count x {:.3}", p.supply_used, p.supply_limit, p.overuse * 100.0, 1.0 - p.overuse);
    println!("    naval supply by state (best base level; * = no core, on another continent than the capital):");
    for (region, level, far, supply) in &p.supply_states {
        println!("      {:<28} level {level} {} {supply:>6.1}", state_name(*region), if *far { "*" } else { " " });
    }
    println!("  TOTAL {:>48.1}", p.total);
    println!("  points standing in open races {:>24.1}", p.races + 0.0);
    if let Some((_, c)) = &game.core.colonies {
        for r in c.races.iter().filter(|r| r.contenders.iter().any(|x| x.0 == tag)) {
            for x in r.contenders.iter().filter(|x| x.0 == tag) {
                println!("    race for region {} (stage {:?}): level {}, {} points invested", r.region, r.stage, x.1, x.2);
            }
        }
        let (colony, protectorate) = c.counts_by_province(&tag);
        println!("  upkeep ({colony} colony provinces, {protectorate} protectorate provinces, with factories and railways) {:>8.1}", p.upkeep);
    }
    println!("  USED {:>49.1}", p.used);
    println!("  FREE {:>49.1}", p.free());

    // ---- cost of turning a colony into a state ----
    println!();
    println!("COST OF TURNING A COLONY INTO A STATE");
    let Some((crules, c)) = &game.core.colonies else { return Ok(()) };
    println!("  provinces x {} x min(1, distance / {}), distance = straight line between province centres (middle of the bounding box) in map pixels / {}; nothing on the capital's continent", crules.state_cost, crules.state_distance, crules.state_distance_pixels);
    let centres = game.world.map.bounds_centres();
    let centre = |id: u16| centres.get(id as usize).map(|c| (f64::from(c.0), f64::from(c.1)));
    let (Some(capital), Some(home)) = (country.capital, country.capital.and_then(centre)) else {
        println!("  {tag} has no capital");
        return Ok(());
    };
    let continent = |id: u16| state.provinces.get(&id).and_then(|p| p.continent.clone());
    println!("  capital {} at pixel ({:.0}, {:.0})", province_name(capital), home.0, home.1);
    println!("  every holding outside the capital's continent; a full state is listed with what it would have cost");
    println!("  {:<24} {:<12} {:>9} {:<18} {:>7} {:>9} {:>7}", "holding", "now", "provinces", "its first province", "pixels", "distance", "cost");
    for h in c.states.iter().filter(|s| s.owner == tag) {
        let Some(first) = h.provinces.first().copied() else { continue };
        let Some(at) = centre(first) else { continue };
        if continent(first) == continent(capital) && h.status != Status::Colony {
            continue;
        }
        let distance = colonies::state_distance(crules, at, home);
        let cost = colonies::state_cost(crules, h.provinces.len(), distance, continent(first) == continent(capital));
        let region = state.provinces.get(&first).and_then(|p| p.region);
        let now = match h.status {
            Status::State => "state",
            Status::Protectorate => "protectorate",
            Status::Colony => "colony",
        };
        println!("  {:<24} {now:<12} {:>9} {:<18} {:>7.0} {:>9.1} {:>7.1}", state_name(region), h.provinces.len(), province_name(first), distance * crules.state_distance_pixels, distance, cost);
    }
    Ok(())
}
