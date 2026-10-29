//! `wiktoria-cli check-day --save <day N.v2> <day N+1.v2>`
//!
//! Moves the port one day on from the first save (pop updates only, no events) and compares
//! every pop that was due that day with the original's save of the next day.

use std::collections::{BTreeMap, HashSet};

use wiktoria_simulation::pops::{grow_due_pops, GrowthRules};
use wiktoria_simulation::script::State;
use wiktoria_simulation::{Calendar, PopSchedule};

use crate::{config, load_world, root, Args};

pub fn check_day(args: &Args) -> Result<(), String> {
    let usage = "usage: check-day --save <day N.v2> <day N+1.v2>";
    let save_path = args.save.as_ref().ok_or(usage)?;
    let after_path = args.positional.first().ok_or(usage)?;
    let w = load_world(args)?;
    let engine_rules = config::load_engine_rules(&root())?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let load = |path: &str| -> Result<State, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
        let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
        let mut state = State::from_save(&save);
        state.attach_world(&w, &save);
        Ok(state)
    };
    let (before, game) = (load(save_path)?, load(after_path)?);
    let (cal, schedule, rules) = (Calendar::from_engine_rules(&engine_rules)?, PopSchedule::from_engine_rules(&engine_rules)?, GrowthRules { growth_scale: 1.0, ..GrowthRules::new(&w.defines, &engine_rules)? });
    let mut port = before.clone();
    port.date = cal.to_date(cal.next_day(cal.to_game(before.date).ok_or("the save's date is not in the calendar")?));
    if port.date != game.date {
        return Err(format!("the second save is dated {}, not the next day {}", game.date, port.date));
    }
    let (updated, added) = grow_due_pops(&mut port, &rules, &schedule, &cal, &w.defs.modifiers);
    println!("{} -> {}: {updated} pops due, {added:+.0} people by growth in the port", before.date, game.date);

    if let Some(out) = args.positional.get(1) {
        let mood = wiktoria_simulation::pops::MoodRules::new(&w.defines, &engine_rules)?.with_reforms(&w.defs.issues);
        dump_mood(&before, &game, &w, &schedule, &cal, &mood, out)?;
    }
    // Due pops, by what happened to them in the game.
    let (mut exact, mut close, mut off, mut gone) = (0usize, 0usize, 0usize, 0usize);
    let (mut game_sum, mut port_sum) = (0.0, 0.0);
    // Pops that did not lose people in the game: growth alone, no other process took anyone.
    let (mut grew, mut grew_exact) = (0usize, 0usize);
    let mut worst: Vec<(f64, String)> = Vec::new();
    let mut by_type: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (id, p) in &before.provinces {
        for (i, b) in p.pops.iter().enumerate().filter(|(_, q)| schedule.is_due_in(q.id, port.date.day, cal.month_len(port.date.month))) {
            if p.owner.is_none() {
                continue;
            }
            let Some(g) = game.provinces[id].pops.iter().find(|q| q.id == b.id && q.pop_type == b.pop_type) else {
                gone += 1;
                continue;
            };
            let q = &port.provinces[id].pops[i];
            let (dg, dp) = (g.size - b.size, q.size - b.size);
            game_sum += dg;
            port_sum += dp;
            let e = by_type.entry(b.pop_type.clone()).or_default();
            e.0 += 1;
            if dg >= 0.0 && b.life_needs >= rules.starvation_limit {
                grew += 1;
                grew_exact += (dg == dp) as usize;
            }
            if dg == dp {
                exact += 1;
                e.1 += 1;
            } else if (dg - dp).abs() <= 1.0 {
                close += 1;
            } else {
                off += 1;
                worst.push(((dg - dp).abs() / b.size.max(1.0), format!("province {id} {} of {}: size {} life needs {:.2} life rating {}, game {dg:+} port {dp:+}", b.pop_type, p.owner.as_deref().unwrap_or("-"), b.size, b.life_needs, p.life_rating)));
            }
        }
    }
    let total = exact + close + off;
    println!("due pops still there in the game: {total} ({gone} gone or changed type)");
    println!("  same change as the game:  {exact} ({:.1}%)", exact as f64 / total.max(1) as f64 * 100.0);
    println!("  within one person:        {close}");
    println!("  different:                {off}");
    println!("of the {grew} pops that did not shrink in the game (growth alone): {grew_exact} the same ({:.1}%)", grew_exact as f64 / grew.max(1) as f64 * 100.0);
    println!("people added to these pops: game {game_sum:+.0}, port {port_sum:+.0}");
    println!("exact by pop type: {}", by_type.iter().map(|(t, (n, ok))| format!("{t} {ok}/{n}")).collect::<Vec<_>>().join(", "));
    worst.sort_by(|a, b| b.0.total_cmp(&a.0));
    let shown = if args.all_warnings { worst.len() } else { 12 };
    println!("largest differences, relative to size:");
    for (_, line) in worst.iter().take(shown) {
        println!("  {line}");
    }
    Ok(())
}

/// Writes one line per due pop with its change of militancy and consciousness in the original and
/// in the port's own step, and everything a rule for them could depend on, for fitting outside
/// the port (`check-day --save a.v2 b.v2 mood.csv`).
fn dump_mood(before: &State, game: &State, w: &wiktoria_data::world::World, schedule: &PopSchedule, cal: &Calendar, mood: &wiktoria_simulation::pops::MoodRules, out: &str) -> Result<(), String> {
    let lib = &w.defs.modifiers;
    // The port's own monthly step on the same pops, for the last two columns.
    let mut port = before.clone();
    port.date = cal.to_date(cal.next_day(cal.to_game(before.date).ok_or("date")?));
    wiktoria_simulation::pops::mood_due_pops(&mut port, mood, schedule, cal, lib);
    let day = port.date;
    let mut text = format!("{MOOD_HEADER},dmil,dcon,port_dmil,port_dcon\n");
    mood_rows(before, w, &mut |id, b, line| {
        if !schedule.is_due_in(b.id, day.day, cal.month_len(day.month)) {
            return;
        }
        let (Some(g), Some(q)) = (game.provinces[&id].pops.iter().find(|q| q.id == b.id && q.pop_type == b.pop_type), port.provinces[&id].pops.iter().find(|q| q.id == b.id)) else { return };
        text += &format!("{line},{:.5},{:.5},{:.5},{:.5}\n", g.militancy - b.militancy, g.consciousness - b.consciousness, q.militancy - b.militancy, q.consciousness - b.consciousness);
    });
    std::fs::write(out, text).map_err(|e| format!("cannot write {out}: {e}"))
}

/// The columns of `mood_rows`.
pub const MOOD_HEADER: &str = "province,id,owner,type,size,mil,con,life,everyday,luxury,accepted,conservative,ruling,political_want,social_want,global_mod,core_mod,non_accepted_mod,province_mod,is_core,colonial,literacy,plurality,war_exhaustion,culture";

/// One line (`MOOD_HEADER`) for every pop of an owned province: the pop's mood and needs and what
/// the militancy and consciousness rules read for it.
pub fn mood_rows(state: &State, w: &wiktoria_data::world::World, f: &mut dyn FnMut(u16, &wiktoria_simulation::script::PopState, String)) {
    let lib = &w.defs.modifiers;
    let conservative = state.ideologies.iter().position(|i| i == "conservative").map(|i| i as u32 + 1);
    for (id, p) in &state.provinces {
        let Some(owner) = p.owner.as_ref() else { continue };
        let Some(c) = state.countries.get(owner) else { continue };
        let ruling = c.ruling_party.and_then(|n| state.party_ideologies.get((n as usize).wrapping_sub(1))).and_then(|name| state.ideologies.iter().position(|i| i == name)).map(|i| i as u32 + 1);
        let sum = |key: &str| -> f64 { wiktoria_simulation::pops::country_modifier(c, lib, key) };
        let (global, core, non_accepted) = (sum("global_pop_militancy_modifier"), sum("core_pop_militancy_modifier"), sum("non_accepted_pop_militancy_modifier"));
        let province_mod: f64 = p.modifiers.iter().filter_map(|m| lib.event_modifiers.get(&m.name)).filter_map(|v| v.get("pop_militancy_modifier")).sum();
        for b in &p.pops {
            // Share of the pop's issue support that goes to reform options the country does not have.
            let want = |category: &str| -> f64 {
                let mut n = 0u32;
                let mut total = 0.0;
                for group in &w.defs.issues {
                    for option in &group.options {
                        n += 1;
                        if group.category == category && c.reforms.get(&group.name) != Some(option) {
                            total += b.issues.get(&n).copied().unwrap_or(0.0);
                        }
                    }
                }
                total / 100.0
            };
            let share = |n: Option<u32>| n.and_then(|n| b.ideology.get(&n)).copied().unwrap_or(0.0) / 100.0;
            let accepted = c.primary_culture.as_deref() == Some(b.culture.as_str()) || c.accepted_cultures.contains(&b.culture);
            let line = format!("{id},{},{owner},{},{},{:.5},{:.5},{:.4},{:.4},{:.4},{},{:.4},{:.4},{:.4},{:.4},{global},{core},{non_accepted},{province_mod},{},{},{:.4},{:.3},{:.3},{}",
                b.id, b.pop_type, b.size, b.militancy, b.consciousness, b.life_needs, b.everyday_needs, b.luxury_needs, accepted as u8,
                share(conservative), share(ruling), want("political_reforms"), want("social_reforms"), p.cores.contains(owner) as u8, p.colonial, b.literacy, c.plurality, c.war_exhaustion, b.culture);
            f(*id, b, line);
        }
    }
}
