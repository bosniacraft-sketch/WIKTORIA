//! `wiktoria-cli check-event <id> --save <before.v2> <after.v2> [<TAG>]`
//!
//! Fires the event on the "before" save and compares every value it touches with the game's
//! own "after" save (made by firing the same event from the console while paused).

use std::collections::{BTreeMap, BTreeSet, HashSet};

use wiktoria_simulation::script::{run_effects, CountryState, EffectRules, PopState, Report, Scope, State};
use wiktoria_simulation::Calendar;

use crate::{config, fire_event, load_world, root, Args};

pub fn check_event(args: &Args) -> Result<(), String> {
    let usage = "usage: check-event <id> --save <before.v2> <after.v2> [<TAG>]";
    let save_path = args.save.as_ref().ok_or(usage)?;
    let id: u64 = args.positional.first().and_then(|s| s.parse().ok()).ok_or(usage)?;
    let after_path = args.positional.get(1).ok_or(usage)?;
    let w = load_world(args)?;
    let root = root();
    let engine_rules = config::load_engine_rules(&root)?;
    let install = config::install_dir(&root, args.install.clone())?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let load = |path: &str| -> Result<(wiktoria_data::save::SaveGame, State), String> {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
        let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
        let mut state = State::from_save(&save);
        state.attach_world(&w, &save);
        Ok((save, state))
    };
    let (save, before) = load(save_path)?;
    let (_, game) = load(after_path)?;
    let tag = args.positional.get(2).cloned().or(save.player.clone()).ok_or("the save names no player; give a TAG")?;
    let (_, options) = fire_event::find_event(&install, id)?;
    let rules = EffectRules::from_engine_rules(&engine_rules)?;
    let mut port = before.clone();
    port.seed = rules.seed;
    port.this = Some(Scope::Country(tag.clone()));
    let mut report = Report::default();
    run_effects(&mut port, &Calendar::from_engine_rules(&engine_rules)?, &rules, options.first().ok_or("the event has no option")?, &Scope::Country(tag.clone()), &mut report);

    println!("event {id} for {tag}: before {} -> game {} / port", before.date, game.date);
    let (b, g, p) = (&before.countries[&tag], &game.countries[&tag], &port.countries[&tag]);
    println!("{:<28} {:>12} {:>12} {:>12}", "country value", "before", "game", "port");
    let row = |name: &str, b: f64, g: f64, p: f64| println!("{name:<28} {b:>12.3} {g:>12.3} {p:>12.3}{}", if (g - p).abs() > 0.002 * g.abs().max(1.0) { "   <-- differs" } else { "" });
    row("plurality", b.plurality, g.plurality, p.plurality);
    row("badboy", b.badboy, g.badboy, p.badboy);
    row("war_exhaustion", b.war_exhaustion, g.war_exhaustion, p.war_exhaustion);
    row("money", b.money, g.money, p.money);
    let rel = |c: &CountryState, t: &str| c.relations.get(t).copied().unwrap_or(0.0);
    for t in ["RUS", "PRU"] {
        row(&format!("relation with {t}"), rel(b, t), rel(g, t), rel(p, t));
        row(&format!("  {t} towards {tag}"), rel(&before.countries[t], &tag), rel(&game.countries[t], &tag), rel(&port.countries[t], &tag));
    }
    let names: BTreeSet<&String> = b.upper_house.keys().chain(g.upper_house.keys()).chain(p.upper_house.keys()).collect();
    for n in names {
        let v = |c: &CountryState| c.upper_house.get(n).copied().unwrap_or(0.0);
        row(&format!("upper house {n}"), v(b), v(g), v(p));
    }
    if let Some(cap) = b.capital {
        row("capital life rating", before.provinces[&cap].life_rating, game.provinces[&cap].life_rating, port.provinces[&cap].life_rating);
    }

    // Who the group scopes reached.
    println!();
    for flag in ["wiktoria_near", "wiktoria_gp", "wiktoria_random"] {
        let with = |s: &State| -> BTreeSet<String> { s.countries.iter().filter(|(_, c)| c.flags.contains(flag)).map(|(t, _)| t.clone()).collect() };
        let (g, p) = (with(&game), with(&port));
        println!("{flag}: game {} countries, port {}; only in game: {:?}; only in port: {:?}", g.len(), p.len(), g.difference(&p).collect::<Vec<_>>(), p.difference(&g).collect::<Vec<_>>());
    }
    let flagged = |s: &State| -> BTreeSet<u16> { s.provinces.iter().filter(|(_, p)| p.flags.contains("wiktoria_state")).map(|(id, _)| *id).collect() };
    let (g, p) = (flagged(&game), flagged(&port));
    println!("wiktoria_state: game {} provinces, port {}; only in game: {:?}; only in port: {:?}", g.len(), p.len(), g.difference(&p).collect::<Vec<_>>(), p.difference(&g).collect::<Vec<_>>());
    let blight = |s: &State| -> Vec<u16> { s.provinces.iter().filter(|(_, p)| p.modifiers.iter().any(|m| m.name == "giga_blight")).map(|(id, _)| *id).collect() };
    println!("random_owned blight: game province {:?}, port province {:?} (the port rolls its own dice)", blight(&game), blight(&port));

    // Pops: per pop type and field, how many changed and how far game and port are apart.
    println!();
    println!("{:<14} {:<14} {:>6} {:>8} {:>8} {:>11} {:>11} {:>10}", "pop type", "field", "pops", "game chg", "port chg", "game mean d", "port mean d", "max apart");
    let liberal = port.ideologies.iter().position(|i| i == "liberal").map(|n| n as u32 + 1).unwrap_or(0);
    let free_trade = port.issues.iter().position(|(o, _)| o == "free_trade").map(|n| n as u32 + 1).unwrap_or(0);
    let values = |q: &PopState| -> [(&'static str, f64); 6] {
        [
            ("militancy", q.militancy),
            ("consciousness", q.consciousness),
            ("money", q.money),
            ("literacy", q.literacy),
            ("liberal %", q.ideology.get(&liberal).copied().unwrap_or(0.0)),
            ("free_trade %", q.issues.get(&free_trade).copied().unwrap_or(0.0)),
        ]
    };
    // (pops, changed in game, changed in port, summed game change, summed port change, largest gap)
    type Tally = (usize, usize, usize, f64, f64, f64);
    let mut rows: BTreeMap<(String, &str), Tally> = BTreeMap::new();
    for (pid, pb) in before.provinces.iter().filter(|(_, p)| p.owner.as_deref() == Some(tag.as_str())) {
        let (pg, pp) = (&game.provinces[pid], &port.provinces[pid]);
        for (i, b) in pb.pops.iter().enumerate() {
            let (Some(g), Some(p)) = (pg.pops.iter().find(|q| q.id == b.id), pp.pops.get(i)) else { continue };
            for (((field, b_v), (_, g_v)), (_, p_v)) in values(b).into_iter().zip(values(g)).zip(values(p)) {
                let e = rows.entry((b.pop_type.clone(), field)).or_default();
                e.0 += 1;
                e.1 += ((g_v - b_v).abs() > 1e-4) as usize;
                e.2 += ((p_v - b_v).abs() > 1e-4) as usize;
                e.3 += g_v - b_v;
                e.4 += p_v - b_v;
                e.5 = e.5.max((g_v - p_v).abs());
            }
        }
    }
    for ((kind, field), (n, gc, pc, gd, pd, apart)) in &rows {
        if *gc > 0 || *pc > 0 {
            println!("{kind:<14} {field:<14} {n:>6} {gc:>8} {pc:>8} {:>11.4} {:>11.4} {apart:>10.4}{}", gd / *n as f64, pd / *n as f64, if *apart > 0.01 { "   <-- differs" } else { "" });
        }
    }
    println!("unsupported: {}", if report.unsupported.is_empty() { "nothing".to_string() } else { format!("{:?}", report.unsupported) });
    Ok(())
}
