//! `wiktoria-cli check-money --save <day N.v2> <day N+1.v2> [TAG,TAG,...]`
//!
//! With a third save (`<day N+2.v2>`) the port runs two days and its second day is compared with
//! the original's N+1 -> N+2, so what a freshly loaded game lacks on its first day (yesterday's
//! sales, pay) does not count.
//!
//! Runs the whole port one day on from the first save and compares where the money went with the
//! original's save of the next day: each pop type's cash and bank (summed per country), and each
//! country's treasury. Pops are matched by id. Prints, per pop type, the day's change in the original
//! and in the port, so a wrong flow shows as a change of the wrong size or sign.

use std::collections::{BTreeMap, HashSet};

use wiktoria_data::script::Value;
use wiktoria_simulation::script::State;
use wiktoria_simulation::game::Game;

use crate::{config, load_world, root, Args};

#[derive(Default, Clone, Copy)]
struct Sum {
    pops: usize,
    size: f64,
    money: f64,
}

/// Money per (country, pop type), only for the pops whose ids are in `ids` (when given).
fn money_by_type(state: &State, ids: Option<&HashSet<u64>>) -> BTreeMap<(String, String), Sum> {
    let mut out: BTreeMap<(String, String), Sum> = BTreeMap::new();
    for p in state.provinces.values() {
        let Some(owner) = p.owner.as_deref() else { continue };
        for q in &p.pops {
            if ids.is_some_and(|ids| !ids.contains(&q.id)) {
                continue;
            }
            let s = out.entry((owner.to_string(), q.pop_type.clone())).or_default();
            s.pops += 1;
            s.size += q.size;
            s.money += q.money;
        }
    }
    out
}

/// Savings (in budget units) per (country, pop type), only for the pops whose ids are in `ids`.
fn savings_by_type(state: &State, ids: &HashSet<u64>) -> BTreeMap<(String, String), f64> {
    let mut out: BTreeMap<(String, String), f64> = BTreeMap::new();
    for p in state.provinces.values() {
        let Some(owner) = p.owner.as_deref() else { continue };
        for q in p.pops.iter().filter(|q| ids.contains(&q.id)) {
            *out.entry((owner.to_string(), q.pop_type.clone())).or_default() += state.money.savings.get(&q.id).map_or(0.0, |x| x.1);
        }
    }
    out
}

fn ids(state: &State) -> HashSet<u64> {
    state.provinces.values().flat_map(|p| p.pops.iter().map(|q| q.id)).collect()
}

pub fn check_money(args: &Args) -> Result<(), String> {
    let usage = "usage: check-money --save <day N.v2> <day N+1.v2> [<day N+2.v2>] [TAG,TAG,...]";
    let save_path = args.save.as_ref().ok_or(usage)?;
    let after_path = args.positional.first().ok_or(usage)?;
    let warm = args.positional.get(1).filter(|p| p.to_lowercase().ends_with(".v2")).cloned();
    let only: Option<HashSet<String>> = args.positional.get(if warm.is_some() { 2 } else { 1 }).map(|t| t.split(',').map(|x| x.trim().to_uppercase()).collect());
    let w = load_world(args)?;
    let engine_rules = config::load_engine_rules(&root())?;
    let install = config::install_dir(&root(), args.install.clone())?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let read = |path: &str| -> Result<wiktoria_data::save::SaveGame, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
        wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)
    };
    let (save, after) = (read(save_path)?, read(after_path)?);
    let third = warm.as_deref().map(read).transpose()?;
    let mut game = Game::new(w, &engine_rules, &install, &save)?;
    let state_of = |g: &wiktoria_data::save::SaveGame, game: &Game| {
        let mut s = State::from_save(g);
        s.attach_world(&game.world, g);
        s
    };
    // (original before, original after, port before, port after) of the compared day.
    let (orig_before, original, before) = match &third {
        // Day one warms the port up; day two is compared with the original's N+1 -> N+2.
        Some(third) => {
            game.day();
            (state_of(&after, &game), state_of(third, &game), game.state.clone())
        }
        None => (state_of(&save, &game), state_of(&after, &game), game.state.clone()),
    };
    let t0 = game.state.money.totals.clone();
    let log = game.day();
    let t1 = &game.state.money.totals;
    println!(
        "port's compared day, world totals (budget units): tax {:.1}, tariffs {:.1}, salaries charged {:.1}, social {:.1}, stockpile {:.1}, interest {:.1}, deposited {:.1}, withdrawn {:.1}, invested {:.1}, needs paid {:.1}",
        t1.tax - t0.tax, t1.tariffs - t0.tariffs, t1.salaries_charged - t0.salaries_charged, t1.social - t0.social, t1.stockpile - t0.stockpile,
        t1.interest - t0.interest, t1.deposited - t0.deposited, t1.withdrawn - t0.withdrawn, t1.invested - t0.invested, t1.needs_paid - t0.needs_paid
    );
    println!("artisans, world: sold {:.1}, inputs bought {:.1}, made {:.1} of {:.1} possible", log.money.artisan_income, log.money.artisan_inputs, log.money.artisan_made, log.money.artisan_capacity);
    let world_treasury = |s: &State| s.countries.values().map(|c| c.money).sum::<f64>();
    println!("world treasuries change: original {:.1}, port {:.1}", world_treasury(&original) - world_treasury(&orig_before), world_treasury(&game.state) - world_treasury(&before));
    if game.state.date != original.date {
        return Err(format!("the port reached {}, the last save is dated {}", game.state.date, original.date));
    }
    // Only pops that exist in all four, so births, moves and merges do not count as money flows.
    let mut common = ids(&before);
    for s in [&game.state, &original, &orig_before] {
        let other = ids(s);
        common.retain(|i| other.contains(i));
    }
    let (b, p, ob, o) = (money_by_type(&before, Some(&common)), money_by_type(&game.state, Some(&common)), money_by_type(&orig_before, Some(&common)), money_by_type(&original, Some(&common)));
    let mut orig_before = orig_before;
    let mut original = original;
    orig_before.money = wiktoria_simulation::economy::MoneyState::from_save(&orig_before, third.as_ref().map_or(&save, |_| &after));
    original.money = wiktoria_simulation::economy::MoneyState::from_save(&original, third.as_ref().unwrap_or(&after));
    let (sb, sp, sob, so) = (savings_by_type(&before, &common), savings_by_type(&game.state, &common), savings_by_type(&orig_before, &common), savings_by_type(&original, &common));
    let keep = |tag: &str| only.as_ref().is_none_or(|s| s.contains(tag));
    let get = |m: &BTreeMap<(String, String), Sum>, k: &(String, String)| m.get(k).map_or(0.0, |x| x.money);

    println!("{} -> {}: {} pops followed (money as stored in the save){}", orig_before.date, original.date, common.len(), if third.is_some() { ", the port's second day" } else { "" });
    println!("\nWorld, per pop type: money before (original) | change in the original | change in the port | port/original");
    let mut world: BTreeMap<String, (f64, f64, f64)> = BTreeMap::new();
    for ((tag, ty), s) in &b {
        if !keep(tag) {
            continue;
        }
        let e = world.entry(ty.clone()).or_default();
        let key = (tag.clone(), ty.clone());
        e.0 += get(&ob, &key);
        e.1 += get(&o, &key) - get(&ob, &key);
        e.2 += get(&p, &key) - s.money;
    }
    for (ty, (m, dorig, dport)) in &world {
        let ratio = if dorig.abs() > 1e-9 { format!("{:.2}", dport / dorig) } else { "-".into() };
        println!("  {ty:<12} {m:>16.1} {dorig:>14.1} {dport:>14.1} {ratio:>8}");
    }
    println!("\nTreasuries: before | change in the original | change in the port");
    let mut rows: Vec<(String, f64, f64, f64)> = before
        .countries
        .iter()
        .filter(|(t, _)| keep(t))
        .filter_map(|(t, c)| Some((t.clone(), c.money, original.countries.get(t)?.money - orig_before.countries.get(t)?.money, game.state.countries.get(t)?.money - c.money)))
        .collect();
    rows.sort_by(|a, b| (b.2 - b.3).abs().total_cmp(&(a.2 - a.3).abs()));
    for (t, m, dorig, dport) in rows.iter().take(25) {
        println!("  {t:<4} {m:>14.1} {dorig:>12.1} {dport:>12.1}");
    }
    if only.is_some() {
        for (t, c) in game.state.countries.iter().filter(|(t, _)| keep(t)) {
            let base = game.state.money.countries.get(t).map_or(0.0, |b| b.tax_base);
            println!("\n{t} in the port: taxes {:.1}, tax base {base:.1}, duties {:.1}, treasury {:.1}", c.tax_income, log.money.duties.get(t).copied().unwrap_or(0.0), c.money);
        }
        println!("\nThe port's state spending at full pay: country | line | amount");
        for ((t, line), x) in log.money.spent_on.iter().filter(|((t, _), _)| keep(t)) {
            println!("  {t} {line:<18} {x:>12.1}");
        }
        // Farms and mines: the port's day against the original's `last_income` (x 1000 in the save).
        let rgo_income = |g: &wiktoria_data::save::SaveGame, id: u16| -> Option<f64> {
            g.doc.root.get(&id.to_string())?.as_block()?.get("rgo")?.as_block()?.get("last_income")?.as_f64().map(|x| x / 1000.0)
        };
        let mut by_good: BTreeMap<(String, String), (f64, f64)> = BTreeMap::new();
        for (id, (good, _, income, _, _)) in &log.money.rgo {
            let Some(owner) = game.state.provinces.get(id).and_then(|p| p.owner.clone()).filter(|t| keep(t)) else { continue };
            let e = by_good.entry((owner, good.clone())).or_default();
            e.0 += rgo_income(third.as_ref().unwrap_or(&after), *id).unwrap_or(0.0);
            e.1 += income;
        }
        let employed = |g: &wiktoria_data::save::SaveGame, id: u16| -> f64 {
            let rgo = g.doc.root.get(&id.to_string()).and_then(Value::as_block).and_then(|b| b.get("rgo")).and_then(Value::as_block);
            let employees = rgo.and_then(|r| r.get("employment")).and_then(Value::as_block).and_then(|e| e.get("employees")).and_then(Value::as_block);
            employees.map_or(0.0, |e| e.items.iter().filter_map(|i| i.value.as_block()?.get("count")?.as_f64()).sum())
        };
        println!("\nFarms and mines 25% or more off: province | good | income original, port | employed original, port, port's places | port units");
        for (id, (good, units, income, at_work, places)) in &log.money.rgo {
            let Some(_) = game.state.provinces.get(id).filter(|p| p.owner.as_deref().is_some_and(&keep)) else { continue };
            let o = rgo_income(third.as_ref().unwrap_or(&after), *id).unwrap_or(0.0);
            if (o - income).abs() > 0.25 * o.max(*income) && o.max(*income) > 5.0 {
                println!("  {id:>5} {good:<16} {o:>9.1} {income:>9.1} | {:>9.0} {at_work:>9.0} {places:>9.0} | {units:.3}", employed(third.as_ref().unwrap_or(&after), *id));
            }
        }
        println!("\nFarms and mines, per country and good: the original's income | the port's");
        for ((t, good), (o, p)) in &by_good {
            println!("  {t} {good:<16} {o:>10.1} {p:>10.1}");
        }
        // Needs met (life, everyday, luxury), weighted by size, in the original and in the port.
        let needs = |st: &State| -> BTreeMap<(String, String), [f64; 4]> {
            let mut out: BTreeMap<(String, String), [f64; 4]> = BTreeMap::new();
            for p in st.provinces.values() {
                let Some(owner) = p.owner.clone().filter(|t| keep(t)) else { continue };
                for q in p.pops.iter().filter(|q| common.contains(&q.id)) {
                    let e = out.entry((owner.clone(), q.pop_type.clone())).or_default();
                    (e[0], e[1], e[2], e[3]) = (e[0] + q.size * q.life_needs, e[1] + q.size * q.everyday_needs, e[2] + q.size * q.luxury_needs, e[3] + q.size);
                }
            }
            out
        };
        let (no, np) = (needs(&original), needs(&game.state));
        println!("\nNeeds met: life, everyday, luxury in the original | in the port");
        for (key, o) in &no {
            let p = np.get(key).copied().unwrap_or_default();
            let f = |x: [f64; 4]| (0..3).map(|i| format!("{:>6.2}", if x[3] > 0.0 { x[i] / x[3] } else { 0.0 })).collect::<String>();
            println!("  {} {:<12} {} | {}", key.0, key.1, f(*o), f(p));
        }
        // Artisans per craft: the save's production_income and last_spending (x 1000) per pop.
        let mut crafts: BTreeMap<(String, String), [f64; 4]> = BTreeMap::new();
        let last = third.as_ref().unwrap_or(&after);
        for (id, p) in &original.provinces {
            let Some(owner) = p.owner.clone().filter(|t| keep(t)) else { continue };
            let Some(block) = last.doc.root.get(&id.to_string()).and_then(Value::as_block) else { continue };
            for q in block.get_all("artisans").filter_map(Value::as_block) {
                let (Some(kind), num) = (q.get("production_type").and_then(Value::as_str), |k: &str| q.get(k).and_then(Value::as_f64).unwrap_or(0.0)) else { continue };
                let e = crafts.entry((owner.clone(), kind.to_string())).or_default();
                (e[0], e[1], e[2]) = (e[0] + num("production_income") / 1000.0, e[1] + num("last_spending") / 1000.0, e[2] + num("size"));
            }
        }
        println!("\nArtisans per craft: sales, inputs, people in the original | sales, inputs, people in the port");
        for (key, o) in &crafts {
            let p = log.money.artisans_by.get(key).copied().unwrap_or_default();
            println!("  {} {:<28} {:>9.1} {:>9.1} {:>9.0} | {:>9.1} {:>9.1} {:>9.0}", key.0, key.1, o[0], o[1], o[2], p[0], p[1], p[3]);
        }
        println!("\nThe port's day, per country and pop type: income | tax base | tax | inputs | needs | invested | saved | full cost of life, everyday, luxury");
        for ((t, ty), f) in log.money.taxed_by.iter().filter(|((t, _), _)| keep(t)) {
            println!("  {t} {ty:<12} {}", f.iter().map(|x| format!("{x:>10.1}")).collect::<String>());
        }
        println!("\nPer country and pop type: change in the original | change in the port (money as stored) | savings: change in the original | in the port (budget units)");
        for (key, s) in &b {
            if !keep(&key.0) {
                continue;
            }
            let dorig = get(&o, key) - get(&ob, key);
            let dport = get(&p, key) - s.money;
            let sv = |m: &BTreeMap<(String, String), f64>| m.get(key).copied().unwrap_or(0.0);
            println!("  {} {:<12} {:>5} pops {:>12.0} people  {:>12.1} {:>12.1} | {:>10.2} {:>10.2}", key.0, key.1, s.pops, s.size, dorig, dport, sv(&so) - sv(&sob), sv(&sp) - sv(&sb));
        }
    }
    Ok(())
}

