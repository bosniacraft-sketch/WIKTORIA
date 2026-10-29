//! `wiktoria-cli run-events --save <file.v2> [<days>]`
//!
//! Starts from a save and lets events, decisions and pop growth happen for the given number of
//! days (default 365). The economy is switched off: each pop keeps the needs fulfilment it has
//! in the save, or, with a folder of state tables (`wiktoria-cli export`) as the second argument,
//! the fulfilment the original had at each table's date:
//! `wiktoria-cli run-events --save <file.v2> <days> <tables folder>`. Prints which events fired, which decisions
//! were taken, which modifiers and flags they left behind, and what the engine could not judge.
//! The tally is what gets compared with a year of the original game.

use std::collections::{BTreeMap, HashSet};

use wiktoria_data::script;
use wiktoria_simulation::game::Game;
use wiktoria_simulation::script::State;

use crate::{config, load_world, root, Args};

/// Needs fulfilment read from one `pops.csv` of the state tables: per pop, and averaged per
/// (province, pop type) and per (owner, pop type) for pops the table does not know.
#[derive(Default)]
struct NeedsTable {
    by_pop: BTreeMap<(u16, u64), [f64; 3]>,
    by_province: BTreeMap<(u16, String), ([f64; 3], f64)>,
    by_owner: BTreeMap<(String, String), ([f64; 3], f64)>,
}

fn read_needs(path: &std::path::Path) -> Result<NeedsTable, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut t = NeedsTable::default();
    for line in text.lines().skip(1) {
        // province,owner,type,id,size,culture,religion,money,literacy,con,mil,life,everyday,luxury
        let c: Vec<&str> = line.split(',').collect();
        let (Some(province), Some(id), Some(size)) = (c.first().and_then(|x| x.parse::<u16>().ok()), c.get(3).and_then(|x| x.parse::<u64>().ok()), c.get(4).and_then(|x| x.parse::<f64>().ok())) else { continue };
        // An empty cell is a tier that was fully met, or one left empty because the tier
        // below it was not met.
        let need = |i: usize| c.get(i).and_then(|x| x.parse::<f64>().ok());
        let life = need(11).unwrap_or(1.0);
        let everyday = need(12).unwrap_or(if life < 1.0 { 0.0 } else { 1.0 });
        let needs = [life, everyday, need(13).unwrap_or(if life < 1.0 || everyday < 1.0 { 0.0 } else { 1.0 })];
        t.by_pop.insert((province, id), needs);
        for entry in [t.by_province.entry((province, c[2].to_string())).or_default(), t.by_owner.entry((c[1].to_string(), c[2].to_string())).or_default()] {
            for (sum, x) in entry.0.iter_mut().zip(needs) {
                *sum += x * size;
            }
            entry.1 += size;
        }
    }
    Ok(t)
}

/// Gives every pop the needs fulfilment of the table: its own, else the average of its type in
/// the province, else of its type in the country.
fn apply_needs(state: &mut State, t: &NeedsTable) {
    for (id, p) in &mut state.provinces {
        let owner = p.owner.clone().unwrap_or_default();
        for q in &mut p.pops {
            let average = |e: &([f64; 3], f64)| [e.0[0] / e.1, e.0[1] / e.1, e.0[2] / e.1];
            let found = t.by_pop.get(&(*id, q.id)).copied().or_else(|| t.by_province.get(&(*id, q.pop_type.clone())).filter(|e| e.1 > 0.0).map(average)).or_else(|| t.by_owner.get(&(owner.clone(), q.pop_type.clone())).filter(|e| e.1 > 0.0).map(average));
            if let Some([life, everyday, luxury]) = found {
                (q.life_needs, q.everyday_needs, q.luxury_needs) = (life, everyday, luxury);
            }
        }
    }
}

fn top(map: &BTreeMap<String, usize>, n: usize) -> String {
    let mut list: Vec<_> = map.iter().collect();
    list.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let more = list.len().saturating_sub(n);
    let shown: Vec<String> = list.iter().take(n).map(|(k, v)| format!("{k} x{v}")).collect();
    if shown.is_empty() {
        "nothing".into()
    } else if more > 0 {
        format!("{} ... and {more} more", shown.join(", "))
    } else {
        shown.join(", ")
    }
}

pub fn run_events(args: &Args) -> Result<(), String> {
    let save_path = args.save.as_ref().ok_or("usage: run-events --save <file.v2> [<days>]")?;
    let days: usize = args.positional.first().and_then(|d| d.parse().ok()).unwrap_or(365);
    let w = load_world(args)?;
    let root = root();
    let engine_rules = config::load_engine_rules(&root)?;
    let install = config::install_dir(&root, args.install.clone())?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let bytes = std::fs::read(save_path).map_err(|e| format!("cannot read {save_path}: {e}"))?;
    let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
    // The day itself lives in wiktoria_simulation::game::Game. run-events keeps its meaning: events, pops and the
    // economy only, without the military and the cross-system effects (`wiktoria-cli run` has those).
    let mut game = Game::new(w, &engine_rules, &install, &save)?;
    game.core_on = false;
    // GAMEMIX_NO_DRIFT=1 leaves pops' ideologies as they are (to compare runs).
    game.econ.no_drift = std::env::var("GAMEMIX_NO_DRIFT").is_ok();
    println!("{} events ({} fire at random), {} decisions", game.econ.runner.events.len(), game.econ.random_events, game.econ.runner.decisions.len());
    // Scores, ranking and great-power seats. GAMEMIX_RANK_CHECK=1: rank the save as it is,
    // compare with its own list of great powers, then stop.
    if std::env::var("GAMEMIX_RANK_CHECK").is_ok() {
        println!("{}", wiktoria_simulation::rank::check(&mut game.state, &game.econ.rank_rules, &game.world.defs.modifiers));
        return Ok(());
    }
    let before = game.state.clone();
    let started = std::time::Instant::now();
    {
    let (state, mobility, mobility_rules, w) = (&game.state, &game.econ.mobility, &game.econ.mobility_rules, &game.world);
    if args.cmd == "why" {
        let ids: Vec<u64> = args.positional.iter().filter_map(|a| a.parse().ok()).collect();
        if !ids.is_empty() {
            print!("{}", crate::why::why(state, mobility, mobility_rules, &ids));
        }
        let read = |field: &str| -> Result<wiktoria_data::script::Document, String> {
            let path = install.join(engine_rules.require_str(field)?);
            script::parse_bytes(&std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?).map_err(|e| e.to_string())
        };
        let reform_rules = wiktoria_simulation::reforms::ReformRules::new(w, &engine_rules, &read("FLD_ISSUES_FILE")?.root, &read("FLD_IDEOLOGIES_FILE")?.root)?;
        for tag in args.positional.iter().filter(|a| a.parse::<u64>().is_err()) {
            print!("{}", crate::why::why_country(state, &reform_rules, &wiktoria_simulation::politics::PoliticsRules::new(w, &read("FLD_GOVERNMENTS_FILE")?.root, &read("FLD_ISSUES_FILE")?.root)?, tag));
        }
        return Ok(());
    }
    // WIKTORIA_MOBILITY_DUMP=<file>: every pop's promotion and demotion chance and its weight for
    // each target type, as the save has it (before the port's own setup), then stop.
    if let Ok(path) = std::env::var("WIKTORIA_MOBILITY_DUMP") {
        let mut out = String::from("province,owner,id,type,size,promotion,demotion,targets,ideology,mil,con,lit,culture,issues
");
        for (id, p) in &state.provinces {
            for (i, q) in p.pops.iter().enumerate() {
                let scope = wiktoria_simulation::script::Scope::Pop(*id, i);
                let targets = mobility.promote_to.get(&q.pop_type).map(|t| t.iter().map(|(n, b)| format!("{n}={:.3}", wiktoria_simulation::pops::additive_weight(state, b, &scope))).collect::<Vec<_>>().join(";")).unwrap_or_default();
                let ideo = mobility.ideologies.get(&q.pop_type).map(|t| t.iter().map(|(n, b)| format!("{n}={:.4}", wiktoria_simulation::pops::product_weight(state, b, &scope))).collect::<Vec<_>>().join(";")).unwrap_or_default();
                let iss = mobility.issues.get(&q.pop_type).map(|t| t.iter().map(|(n, b)| format!("{n}={:.4}", wiktoria_simulation::pops::product_weight(state, b, &scope))).collect::<Vec<_>>().join(";")).unwrap_or_default();
                let targets = format!("{targets},{ideo},{:.3},{:.3},{:.3},{},{iss}", q.militancy, q.consciousness, q.literacy, q.culture);
                out.push_str(&format!("{id},{},{},{},{},{:.3},{:.3},{targets}
", p.owner.as_deref().unwrap_or(""), q.id, q.pop_type, q.size, wiktoria_simulation::pops::additive_weight(state, &mobility.promotion_chance, &scope), wiktoria_simulation::pops::additive_weight(state, &mobility.demotion_chance, &scope)));
            }
        }
        std::fs::write(&path, out).map_err(|e| format!("cannot write {path}: {e}"))?;
        return Ok(());
    }
    }
    let mood = |s: &State| -> (f64, f64) {
        let (mut con, mut mil, mut all) = (0.0, 0.0, 0.0);
        for q in s.provinces.values().flat_map(|p| &p.pops) {
            con += q.consciousness * q.size;
            mil += q.militancy * q.size;
            all += q.size;
        }
        (con / all.max(1.0), mil / all.max(1.0))
    };
    let literacy = |s: &State| -> f64 {
        let (mut sum, mut all) = (0.0, 0.0);
        for q in s.provinces.values().flat_map(|p| &p.pops) {
            sum += q.literacy * q.size;
            all += q.size;
        }
        sum / all.max(1.0) * 100.0
    };
    let mut flows: BTreeMap<String, usize> = BTreeMap::new();
    let (mut assimilated, mut moved_within) = (0.0, 0.0);
    let (mut emigrated, mut arrivals): (BTreeMap<String, usize>, BTreeMap<String, usize>) = Default::default();
    let cultures_of = |s: &State| -> BTreeMap<String, f64> {
        let mut out = BTreeMap::new();
        for q in s.provinces.values().flat_map(|p| &p.pops) {
            *out.entry(q.culture.clone()).or_insert(0.0) += q.size;
        }
        out
    };
    let by_type = |s: &State| -> BTreeMap<String, f64> {
        let mut out = BTreeMap::new();
        for q in s.provinces.values().flat_map(|p| &p.pops) {
            *out.entry(q.pop_type.clone()).or_insert(0.0) += q.size;
        }
        out
    };
    let people = |s: &State, tag: Option<&str>| -> f64 { s.provinces.values().filter(|p| tag.is_none() || p.owner.as_deref() == tag).flat_map(|p| &p.pops).map(|q| q.size).sum() };
    // Needs tables of the original, oldest first, each with its date.
    let mut needs_tables: Vec<(wiktoria_data::date::Date, std::path::PathBuf)> = Vec::new();
    if let Some(folder) = args.positional.get(1) {
        for entry in std::fs::read_dir(folder).map_err(|e| format!("cannot read {folder}: {e}"))?.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            let parts: Vec<i32> = name.split('_').filter_map(|x| x.parse().ok()).collect();
            if let [y, m, d] = parts[..] {
                needs_tables.push((wiktoria_data::date::Date::new(y, m as u8, d as u8), entry.path().join("pops.csv")));
            }
        }
        needs_tables.sort();
        println!("needs fulfilment taken from {} tables of the original ({folder})", needs_tables.len());
    }
    let (mut discovered, mut discovered_names): (BTreeMap<String, usize>, BTreeMap<String, usize>) = (BTreeMap::new(), BTreeMap::new());
    let farm_goods: std::collections::BTreeSet<String> = game.world.defs.rgo_types.iter().map(|r| r.good.clone()).collect();
    let report_farms = |w: &wiktoria_data::world::World, farms: &wiktoria_simulation::economy::Farms, state: &State, tables: &[(wiktoria_data::date::Date, std::path::PathBuf)]| {
        let mut ours: BTreeMap<String, f64> = BTreeMap::new();
        for ((_, good), x) in farms.daily_output(w, state) {
            *ours.entry(good).or_default() += x;
        }
        let game: BTreeMap<String, f64> = tables
            .iter()
            .rev()
            .find(|(d, _)| *d <= state.date)
            .and_then(|(_, pops)| std::fs::read_to_string(pops.with_file_name("goods.csv")).ok())
            .map(|text| text.lines().skip(1).filter_map(|l| { let c: Vec<&str> = l.split(',').collect(); Some((c.first()?.to_string(), c.get(2)?.parse().ok()?)) }).collect())
            .unwrap_or_default();
        let (mut port_sum, mut game_sum) = (0.0, 0.0);
        let mut parts = Vec::new();
        for good in &farm_goods {
            let (o, g) = (ours.get(good).copied().unwrap_or(0.0), game.get(good).copied().unwrap_or(0.0));
            if g > 0.0 && good != "precious_metal" {
                port_sum += o;
                game_sum += g;
                parts.push(format!("{good} {:.2}", o / g));
            }
        }
        // Factories and artisans, at what they can make when their inputs are there.
        let (mut factory, mut artisan): (BTreeMap<String, f64>, BTreeMap<String, f64>) = (BTreeMap::new(), BTreeMap::new());
        for ((_, good), (f, a)) in farms.workshop_output(w, state) {
            *factory.entry(good.clone()).or_default() += f;
            *artisan.entry(good).or_default() += a;
        }
        // Artisans as they run today (throttle and stock), next to their capacity above.
        let mut running: BTreeMap<String, f64> = BTreeMap::new();
        for ((_, good), x) in farms.artisan_output_running(w, state) {
            *running.entry(good).or_default() += x;
        }
        let made: std::collections::BTreeSet<&String> = factory.keys().chain(artisan.keys()).collect();
        let (mut shop_sum, mut shop_game, mut shop_factory, mut shop_running) = (0.0, 0.0, 0.0, 0.0);
        let mut shop_parts = Vec::new();
        for good in made {
            let (f, a, g) = (factory.get(good).copied().unwrap_or(0.0), artisan.get(good).copied().unwrap_or(0.0), game.get(good).copied().unwrap_or(0.0));
            if g > 0.0 && !farm_goods.contains(good) {
                shop_sum += f + a;
                shop_game += g;
                shop_factory += f;
                shop_running += f + running.get(good).copied().unwrap_or(0.0);
                shop_parts.push(format!("{good} {:.2}", (f + a) / g));
            }
        }
        // What pops and workshops want to buy, next to the paid demand of the original.
        let game_demand: BTreeMap<String, f64> = tables
            .iter()
            .rev()
            .find(|(d, _)| *d <= state.date)
            .and_then(|(_, pops)| std::fs::read_to_string(pops.with_file_name("goods.csv")).ok())
            .map(|text| text.lines().skip(1).filter_map(|l| { let c: Vec<&str> = l.split(',').collect(); Some((c.first()?.to_string(), c.get(3)?.parse().ok()?)) }).collect())
            .unwrap_or_default();
        let mut wanted: BTreeMap<String, [f64; 4]> = BTreeMap::new();
        for ((_, good), tiers) in farms.pop_needs(w, state) {
            let e = wanted.entry(good).or_default();
            for i in 0..3 {
                e[i] += tiers[i];
            }
        }
        for ((_, good), x) in farms.workshop_inputs(w, state) {
            wanted.entry(good).or_default()[3] += x;
        }
        let demand_parts: Vec<String> = wanted
            .iter()
            .filter_map(|(good, [life, everyday, luxury, inputs])| {
                let g = *game_demand.get(good).filter(|g| **g > 1.0)?;
                Some(format!("{good} {:.2}/{:.2}", (life + everyday + inputs) / g, (life + everyday + luxury + inputs) / g))
            })
            .collect();
        // Needs met of the pops the port pays itself, next to the original's.
        let mut met: BTreeMap<&str, [f64; 4]> = BTreeMap::new();
        for q in state.provinces.values().filter(|p| p.owner.is_some()).flat_map(|p| &p.pops) {
            if let Some(kind) = ["farmers", "labourers", "aristocrats", "artisans", "soldiers", "clergymen", "bureaucrats", "officers"].into_iter().find(|k| *k == q.pop_type) {
                let e = met.entry(kind).or_default();
                (e[0], e[1], e[2], e[3]) = (e[0] + q.life_needs * q.size, e[1] + q.everyday_needs * q.size, e[2] + q.luxury_needs * q.size, e[3] + q.size);
            }
        }
        let mut game_met: BTreeMap<String, [f64; 4]> = BTreeMap::new();
        if let Some(text) = tables.iter().rev().find(|(d, _)| *d <= state.date).and_then(|(_, pops)| std::fs::read_to_string(pops).ok()) {
            for line in text.lines().skip(1) {
                let c: Vec<&str> = line.split(',').collect();
                let (Some(kind), Some(size)) = (c.get(2), c.get(4).and_then(|x| x.parse::<f64>().ok())) else { continue };
                if c.get(1).is_none_or(|o| o.is_empty()) {
                    continue;
                }
                let need = |i: usize| c.get(i).and_then(|x| x.parse::<f64>().ok());
                let life = need(11).unwrap_or(1.0);
                let everyday = need(12).unwrap_or(if life < 1.0 { 0.0 } else { 1.0 });
                let luxury = need(13).unwrap_or(if life < 1.0 || everyday < 1.0 { 0.0 } else { 1.0 });
                let e = game_met.entry(kind.to_string()).or_default();
                (e[0], e[1], e[2], e[3]) = (e[0] + life * size, e[1] + everyday * size, e[2] + luxury * size, e[3] + size);
            }
        }
        let met_line: Vec<String> = met
            .iter()
            .map(|(kind, p)| {
                let g = game_met.get(*kind).copied().unwrap_or([0.0, 0.0, 0.0, 1.0]);
                format!("{kind} life {:.2}/{:.2} everyday {:.2}/{:.2} luxury {:.2}/{:.2}", p[0] / p[3].max(1.0), g[0] / g[3].max(1.0), p[1] / p[3].max(1.0), g[1] / g[3].max(1.0), p[2] / p[3].max(1.0), g[2] / g[3].max(1.0))
            })
            .collect();
        println!("{}: needs met, port/game: {}", state.date, met_line.join("; "));
        {
            // Factories (count/levels) per country, largest first: where the port over- or under-builds.
            let mut by_country: BTreeMap<&String, (usize, f64)> = BTreeMap::new();
            for f in &state.factories {
                let e = by_country.entry(&f.owner).or_default();
                (e.0, e.1) = (e.0 + 1, e.1 + f.level);
            }
            let mut list: Vec<_> = by_country.into_iter().collect();
            list.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
            println!("{}: factories by country (count/levels): {} in all; {}", state.date, state.factories.len(), list.iter().take(12).map(|(t, (n, l))| format!("{t} {n}/{l:.0}")).collect::<Vec<_>>().join(" "));
        }
        // Prices, port against the original.
        let game_price: BTreeMap<String, f64> = tables
            .iter()
            .rev()
            .find(|(d, _)| *d <= state.date)
            .and_then(|(_, pops)| std::fs::read_to_string(pops.with_file_name("goods.csv")).ok())
            .map(|text| text.lines().skip(1).filter_map(|l| { let c: Vec<&str> = l.split(',').collect(); Some((c.first()?.to_string(), c.get(1)?.parse().ok()?)) }).collect())
            .unwrap_or_default();
        let ratios: Vec<(f64, &String)> = state.prices.iter().filter_map(|(good, p)| Some((p / game_price.get(good).filter(|g| **g > 0.0)?, good))).filter(|(_, good)| state.goods_on_sale.is_empty() || state.goods_on_sale.contains(*good)).collect();
        if !ratios.is_empty() {
            let mut sorted = ratios.clone();
            sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
            let close = ratios.iter().filter(|(r, _)| (r - 1.0).abs() <= 0.15).count();
            println!(
                "{}: prices, port/game: {} of {} goods within 15%; lowest {}, highest {}",
                state.date,
                close,
                ratios.len(),
                sorted.iter().take(4).map(|(r, g)| format!("{g} {r:.2}")).collect::<Vec<_>>().join(", "),
                sorted.iter().rev().take(4).map(|(r, g)| format!("{g} {r:.2}")).collect::<Vec<_>>().join(", ")
            );
        }
        // The world's books: who holds the money, port against the original (budget units).
        let mut held: BTreeMap<String, f64> = BTreeMap::new();
        for q in state.provinces.values().filter(|p| p.owner.is_some()).flat_map(|p| &p.pops) {
            *held.entry(q.pop_type.clone()).or_default() += q.money / 1000.0;
        }
        let owners: std::collections::BTreeSet<&String> = state.provinces.values().filter_map(|p| p.owner.as_ref()).collect();
        let treasuries: f64 = owners.iter().filter_map(|t| state.countries.get(*t)).map(|c| c.money).sum();
        let mut game_held: BTreeMap<String, f64> = BTreeMap::new();
        let mut game_treasuries = 0.0;
        if let Some((_, pops)) = tables.iter().rev().find(|(d, _)| *d <= state.date) {
            for line in std::fs::read_to_string(pops).unwrap_or_default().lines().skip(1) {
                let c: Vec<&str> = line.split(',').collect();
                if let (Some(kind), Some(money)) = (c.get(2), c.get(7).and_then(|x| x.parse::<f64>().ok())) && c.get(1).is_some_and(|o| !o.is_empty()) {
                    *game_held.entry(kind.to_string()).or_default() += money / 1000.0;
                }
            }
            for line in std::fs::read_to_string(pops.with_file_name("countries.csv")).unwrap_or_default().lines().skip(1) {
                let c: Vec<&str> = line.split(',').collect();
                if let (Some(tag), Some(money)) = (c.first(), c.get(1).and_then(|x| x.parse::<f64>().ok())) && owners.iter().any(|o| o.as_str() == *tag) {
                    game_treasuries += money;
                }
            }
        }
        let (port_total, game_total) = (treasuries + held.values().sum::<f64>(), game_treasuries + game_held.values().sum::<f64>());
        let mut kinds: Vec<(&String, &f64)> = game_held.iter().collect();
        kinds.sort_by(|a, b| b.1.total_cmp(a.1));
        println!(
            "{}: money held, port/game: total {:.0}/{:.0}; treasuries {:.0}/{:.0} ({:.0}%/{:.0}%); {}",
            state.date,
            port_total,
            game_total,
            treasuries,
            game_treasuries,
            treasuries / port_total.max(1.0) * 100.0,
            game_treasuries / game_total.max(1.0) * 100.0,
            kinds.iter().filter(|(k, _)| k.as_str() == "capitalists").chain(kinds.iter().take(7)).map(|(k, g)| format!("{k} {:.0}/{:.0}", held.get(*k).copied().unwrap_or(0.0), g)).collect::<Vec<_>>().join(", ")
        );
        if game_sum > 0.0 {
            println!("{}: wanted by pops and workshops, port / game paid demand (without / with luxury needs): {}", state.date, demand_parts.join(", "));
            println!("{}: farm and mine output, port / game supply: all {:.3}; {}", state.date, port_sum / game_sum, parts.join(", "));
            println!("{}: factory and artisan output (inputs assumed), port / game supply: all {:.3}, {:.0}% of it from factories; {}", state.date, shop_sum / shop_game.max(1e-9), shop_factory / shop_sum.max(1e-9) * 100.0, shop_parts.join(", "));
            println!("{}: the same with artisans at their own throttle and stock (factories still at full inputs): all {:.3}", state.date, shop_running / shop_game.max(1e-9));
        } else {
            println!("{}: farm and mine output a day: {:.0} units", state.date, ours.values().sum::<f64>());
            let scale = wiktoria_simulation::num_field(&engine_rules, "FLD_POP_MONEY_EFFECT_SCALE").unwrap_or(1.0);
            let (n, cash, income, spending, pay) = state.factories.iter().fold((0, 0.0, 0.0, 0.0, 0.0), |a, f| (a.0 + 1, a.1 + f.money / scale, a.2 + f.income, a.3 + f.spending, a.4 + f.pay));
            println!("{}: factories: {n}, in profit yesterday {}, cash {cash:.0}, sales {income:.1}, inputs {spending:.1}, pay {pay:.2} a day", state.date, state.factories.iter().filter(|f| f.income > f.spending).count());
            {
                // Countries whose poor get at least 90% of their life needs (what stops the
                // epidemics spreading in the events).
                let mut fed: BTreeMap<&String, (f64, f64)> = BTreeMap::new();
                for p in state.provinces.values() {
                    let Some(o) = p.owner.as_ref() else { continue };
                    for q in p.pops.iter().filter(|q| state.strata.get(&q.pop_type).is_none_or(|s| s == "poor") && !state.slave_types.contains(&q.pop_type)) {
                        let e = fed.entry(o).or_default();
                        (e.0, e.1) = (e.0 + q.size * q.life_needs, e.1 + q.size);
                    }
                }
                let well = fed.values().filter(|(got, all)| *all > 0.0 && got / all >= 0.9).count();
                let show = |t: &str| fed.get(&t.to_string()).map_or(String::new(), |(got, all)| format!("{t} {:.2}", got / all.max(1.0)));
                println!("{}: countries whose poor get 90% of their life needs: {well} of {}; {}", state.date, fed.len(), ["ENG", "FRA", "PRU", "RUS", "AUS", "USA", "TUR", "CHI", "JAP", "BRZ", "MEX", "SWE", "SPA", "EGY"].iter().map(|t| show(t)).collect::<Vec<_>>().join(" "));
            }
            // GAMEMIX_FACTORY_LINES=1: every factory on its own line.
            if std::env::var("GAMEMIX_FACTORY_LINES").is_ok() {
                for f in &state.factories {
                    println!("  factory {} {} level {} staff {} cash {:.1} sales {:.2} inputs {:.2} pay {:.3}", f.owner, f.kind, f.level, f.staff.map_or("all".to_string(), |s| format!("{s:.0}")), f.money / scale, f.income, f.spending, f.pay);
                }
            }
        }
    };
    let mut reforms_passed: Vec<(wiktoria_data::date::Date, String, String, String)> = Vec::new();
    let mood_tags = ["ENG", "FRA", "PRU", "RUS", "AUS", "USA", "NET", "SAX", "HOL", "DEN", "SPA", "CHI"];
    let mut elections: Vec<(String, Option<u32>, u32)> = Vec::new();
    let mut researched: BTreeMap<String, usize> = BTreeMap::new();
    let mut farm_money = wiktoria_simulation::economy::FarmMoney::default();
    let mut built = wiktoria_simulation::industry::IndustryMonth::default();
    let mut next_table = 0;
    for _ in 0..days {
        // Before the original's needs of this date are taken over, so the port's own show.
        if game.state.date.day == 1 && game.state.date.month == 1 || game.state.date == before.date {
            report_farms(&game.world, &game.econ.farms, &game.state, &needs_tables);
        }
        while next_table < needs_tables.len() && needs_tables[next_table].0 <= game.state.date {
            apply_needs(&mut game.state, &read_needs(&needs_tables[next_table].1)?);
            next_table += 1;
        }
        let today = game.state.date;
        if today.day == 1 && today.month == 1 {
            let state = &game.state;
            let mood = |tag: &str| {
                let (mut people, mut mil, mut con) = (0.0f64, 0.0f64, 0.0f64);
                for q in state.provinces.values().filter(|p| p.owner.as_deref() == Some(tag)).flat_map(|p| &p.pops) {
                    (people, mil, con) = (people + q.size, mil + q.size * q.militancy, con + q.size * q.consciousness);
                }
                format!("{tag} {:.2}/{:.2}", mil / people.max(1.0), con / people.max(1.0))
            };
            println!("{}.1.1: militancy/consciousness: {}", today.year, mood_tags.iter().map(|t| mood(t)).collect::<Vec<_>>().join(" "));
            println!("{}.1.1: {}", today.year, wiktoria_simulation::rank::report(state));
        }
        let day = game.day();
        for (tag, _) in day.researched {
            *researched.entry(tag).or_default() += 1;
        }
        elections.extend(day.elections);
        reforms_passed.extend(day.reforms.into_iter().map(|(t, g, o)| (today, t, g, o)));
        let money = day.money;
        (farm_money.artisan_income, farm_money.artisan_inputs, farm_money.artisan_made, farm_money.artisan_capacity) = (farm_money.artisan_income + money.artisan_income, farm_money.artisan_inputs + money.artisan_inputs, money.artisan_made, money.artisan_capacity);
        (farm_money.income, farm_money.tax, farm_money.spent, farm_money.salaries) = (farm_money.income + money.income, farm_money.tax + money.tax, farm_money.spent + money.spent, farm_money.salaries + money.salaries);
        farm_money.tariffs += money.tariffs;
        (farm_money.factory_income, farm_money.factory_inputs, farm_money.factory_pay) = (farm_money.factory_income + money.factory_income, farm_money.factory_inputs + money.factory_inputs, farm_money.factory_pay + money.factory_pay);
        (farm_money.salary_cost, farm_money.state_goods, farm_money.banked) = (farm_money.salary_cost + money.salary_cost, farm_money.state_goods + money.state_goods, farm_money.banked + money.banked);
        built.started_by_pops.extend(day.built.started_by_pops);
        built.started_by_states.extend(day.built.started_by_states);
        (built.enlarged, built.opened) = (built.enlarged + day.built.enlarged, built.opened + day.built.opened);
        for (tag, n) in day.discovered {
            *discovered.entry(tag).or_default() += 1;
            *discovered_names.entry(game.state.inventions.get(n as usize - 1).cloned().unwrap_or_default()).or_default() += 1;
        }
        for ((from, to), n) in day.flows {
            *flows.entry(format!("{from} -> {to}")).or_default() += n as usize;
        }
        assimilated += day.assimilated;
        moved_within += day.moved_within;
        for ((from, to), n) in day.emigrated {
            *emigrated.entry(format!("{from} -> {to}")).or_default() += n as usize;
            *arrivals.entry(to).or_default() += n as usize;
        }
    }
    let (w, state) = (&game.world, game.state.clone());
    let (industry_rules, focus_ai, log, event_rules, titles) = (&game.econ.industry_rules, &game.econ.focus_ai, &game.econ.log, &game.econ.event_rules, &game.econ.event_titles);
    let (t0, t1) = (by_type(&before), by_type(&state));
    let mut kinds: Vec<&String> = t0.keys().collect();
    kinds.sort_by(|a, b| t0[*b].total_cmp(&t0[*a]));
    println!("pop types (people, change): {}", kinds.iter().map(|k| format!("{k} {:+.2}%", (t1.get(*k).copied().unwrap_or(0.0) / t0[*k].max(1.0) - 1.0) * 100.0)).collect::<Vec<_>>().join(", "));
    println!("pops: {} -> {}", before.provinces.values().map(|p| p.pops.len()).sum::<usize>(), state.provinces.values().map(|p| p.pops.len()).sum::<usize>());
    let ((c0, m0), (c1, m1)) = (mood(&before), mood(&state));
    println!("average consciousness: {c0:.2} -> {c1:.2}; average militancy: {m0:.2} -> {m1:.2}");
    println!("research points saved up (no technology is researched): {}", ["ENG", "RUS", "FRA", "AUS", "PRU", "USA", "CHI"].iter().filter_map(|t| Some(format!("{t} {:.0} -> {:.0}", before.countries.get(*t)?.research_points, state.countries.get(*t)?.research_points))).collect::<Vec<_>>().join(", "));
    println!("prestige: {}", ["ENG", "RUS", "FRA", "AUS", "PRU", "USA"].iter().filter_map(|t| Some(format!("{t} {:.1} -> {:.1}", before.countries.get(*t)?.prestige, state.countries.get(*t)?.prestige))).collect::<Vec<_>>().join(", "));
    let mut by_group: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_year: BTreeMap<i32, usize> = BTreeMap::new();
    for (d, _, g, _) in &reforms_passed {
        *by_group.entry(g.clone()).or_default() += 1;
        *by_year.entry(d.year).or_default() += 1;
    }
    println!("reforms passed: {} in all; by reform: {}; by year: {}", reforms_passed.len(), top(&by_group, 20), by_year.iter().map(|(y, n)| format!("{y} {n}")).collect::<Vec<_>>().join(", "));
    println!("  first: {}", reforms_passed.iter().take(40).map(|(d, t, _, o)| format!("{}.{} {t} {o}", d.year, d.month)).collect::<Vec<_>>().join(", "));
    println!("elections: {} held, {} changed the ruling party", elections.len(), elections.iter().filter(|e| e.1 != Some(e.2)).count());
    println!("ruling parties: {}", state.countries.iter().filter_map(|(t, c)| Some(format!("{t}={}", c.ruling_party?))).collect::<Vec<_>>().join(" "));
    println!("upper houses: {}", ["ENG", "RUS", "FRA", "AUS", "PRU", "USA"].iter().filter_map(|t| Some(format!("{t} {}", state.countries.get(*t)?.upper_house.iter().filter(|(_, v)| **v >= 0.005).map(|(k, v)| format!("{k} {:.0}%", v * 100.0)).collect::<Vec<_>>().join(" ")))).collect::<Vec<_>>().join("; "));
    println!("technologies researched: {} in all; {}", researched.values().sum::<usize>(), ["ENG", "RUS", "FRA", "AUS", "PRU", "USA", "SWE", "CHI"].iter().map(|t| format!("{t} {} -> {}", before.countries.get(*t).map_or(0, |c| c.technologies.len()), state.countries.get(*t).map_or(0, |c| c.technologies.len()))).collect::<Vec<_>>().join(", "));
    println!("inventions discovered: {} in all; {}", discovered.values().sum::<usize>(), ["ENG", "RUS", "FRA", "AUS", "PRU", "USA", "CHI"].iter().map(|t| format!("{t} {}", discovered.get(*t).copied().unwrap_or(0))).collect::<Vec<_>>().join(", "));
    println!("  most often: {}", top(&discovered_names, 12));
    println!("artisans: sold {:.0}, paid for inputs {:.0}; on the last day they made {:.0}% of what they could at full capacity", farm_money.artisan_income, farm_money.artisan_inputs, farm_money.artisan_made / farm_money.artisan_capacity.max(1e-9) * 100.0);
    let by_owner = |list: &[(String, String)]| {
        let mut n: BTreeMap<String, usize> = BTreeMap::new();
        for (owner, _) in list {
            *n.entry(owner.clone()).or_default() += 1;
        }
        top(&n, 16)
    };
    println!("factories built: {} started by capitalists ({}), {} by governments ({}), {} levels added, {} opened; now {} factories, {} standing empty, {} states with one, investment funds {:.0}", built.started_by_pops.len(), by_owner(&built.started_by_pops), built.started_by_states.len(), by_owner(&built.started_by_states), built.enlarged, built.opened, state.factories.len(), state.factories.iter().filter(|f| f.closed).count(), state.factories.iter().map(|f| (&f.owner, f.region)).collect::<std::collections::BTreeSet<_>>().len(), state.investment.values().sum::<f64>());
    {
        // Per country: its capitalists' funds, its economic policy and how many types it can build.
        let mut funds: BTreeMap<&String, f64> = BTreeMap::new();
        for ((owner, _), x) in &state.investment {
            *funds.entry(owner).or_default() += x;
        }
        let mut owners: BTreeMap<&String, f64> = BTreeMap::new();
        for p in state.provinces.values() {
            if let Some(o) = p.owner.as_ref() {
                *owners.entry(o).or_default() += p.pops.iter().filter(|q| industry_rules.owner_types.contains(&q.pop_type)).map(|q| q.size).sum::<f64>();
            }
        }
        let line = |tag: &str| {
            let Some(c) = state.countries.get(tag) else { return String::new() };
            let policy = c.ruling_party.and_then(|n| industry_rules.party_policy.get((n as usize).wrapping_sub(1))).and_then(|o| o.clone()).unwrap_or_default();
            format!("{tag} {policy} owners {:.0} fund {:.0} types {} factories {}", owners.get(&tag.to_string()).copied().unwrap_or(0.0), funds.get(&tag.to_string()).copied().unwrap_or(0.0), industry_rules.enabled(w, c).len(), state.factories.iter().filter(|f| f.owner == tag).count())
        };
        println!("industry by country: {}", ["ENG", "FRA", "PRU", "AUS", "RUS", "USA", "NET", "TUR", "SWE", "SPA", "BEL", "SIC", "BOL"].iter().map(|t| line(t)).collect::<Vec<_>>().join("; "));
    }
    println!("factories over the run: sales {:.0}, inputs {:.0}, pay to workers and owners {:.0}; tariffs paid by pops {:.0}", farm_money.factory_income, farm_money.factory_inputs, farm_money.factory_pay, farm_money.tariffs);
    println!("money over the run: farm income {:.0}, tax {:.0}, state salaries {:.0} (cost to the states {:.0}), state goods {:.0}, spent on needs {:.0}, deposited by pops {:.0}; treasuries: {}", farm_money.income, farm_money.tax, farm_money.salaries, farm_money.salary_cost, farm_money.state_goods, farm_money.spent, farm_money.banked, ["ENG", "RUS", "FRA", "AUS", "PRU", "USA"].iter().filter_map(|t| Some(format!("{t} {:.0} -> {:.0}", before.countries.get(*t)?.money, state.countries.get(*t)?.money))).collect::<Vec<_>>().join(", "));
    println!("{}", focus_ai.summary(&state, std::env::var("GAMEMIX_FOCUS_LINES").is_ok()));
    println!("{}", state.money.report(&state, wiktoria_simulation::num_field(&engine_rules, "FLD_POP_MONEY_EFFECT_SCALE").unwrap_or(1000.0)));
    println!("plurality:{}", ["ENG", "RUS", "FRA", "AUS", "PRU", "USA"].iter().filter_map(|t| Some(format!("{t} {:.2} -> {:.2}", before.countries.get(*t)?.plurality, state.countries.get(*t)?.plurality))).collect::<Vec<_>>().join(", "));
    println!("average literacy: {:.2}% -> {:.2}%", literacy(&before), literacy(&state));
    println!("assimilated: {assimilated:.0} people; moved within their country: {moved_within:.0}; emigrated: {}", emigrated.values().sum::<usize>());
    println!("  arrivals by country: {}", top(&arrivals, 8));
    println!("  largest flows: {}", top(&emigrated, 8));
    let (c0, c1) = (cultures_of(&before), cultures_of(&state));
    let mut shifts: Vec<(f64, &String)> = c1.iter().map(|(k, v)| (v - c0.get(k).copied().unwrap_or(0.0), k)).collect();
    shifts.sort_by(|a, b| b.0.abs().total_cmp(&a.0.abs()));
    println!("  cultures that changed most: {}", shifts.iter().take(8).map(|(d, k)| format!("{k} {d:+.0}")).collect::<Vec<_>>().join(", "));
    println!("people who changed type: {}", top(&flows, 60));
    let (p0, p1) = (people(&before, None), people(&state, None));
    println!("population: {p0:.0} -> {p1:.0} ({:+.2}%); at that yearly rate x{:.2} in 100 years", (p1 / p0 - 1.0) * 100.0, (p1 / p0).powf(100.0 * 365.0 / days as f64));
    let by_country: Vec<String> = ["ENG", "RUS", "FRA", "AUS", "PRU", "USA", "CHI", "JAP", "TUR"].iter().map(|t| format!("{t} {:+.2}%", (people(&state, Some(t)) / people(&before, Some(t)).max(1.0) - 1.0) * 100.0)).collect();
    println!("  {}", by_country.join(", "));
    println!("{} -> {} ({days} days, checked every {} days) in {:.1}s", before.date, state.date, event_rules.check_days, started.elapsed().as_secs_f64());

    let fired: BTreeMap<String, usize> = log.fired.iter().map(|(id, n)| (format!("{id} {}", titles.get(id).map(String::as_str).unwrap_or("")), *n)).collect();
    let shown = if args.all_warnings { usize::MAX } else { 30 };
    println!("events fired: {} of {} different events", log.fired.values().sum::<usize>(), log.fired.len());
    println!("  {}", top(&fired, shown));
    println!("decisions taken: {}", top(&log.decided, shown));
    println!("modifiers added: {}", top(&log.effects.modifiers_added, shown));
    let mut flags: BTreeMap<String, usize> = BTreeMap::new();
    for (tag, c) in &state.countries {
        for f in c.flags.difference(&before.countries[tag].flags) {
            *flags.entry(f.clone()).or_default() += 1;
        }
    }
    println!("country flags gained: {}", top(&flags, shown));
    println!("global flags gained: {}", state.global_flags.difference(&before.global_flags).cloned().collect::<Vec<_>>().join(", "));
    println!("could not judge (events and decisions blocked by a condition): {}", top(&log.unknown, shown));
    println!("effects not understood: {}", top(&log.effects.unsupported, shown));
    Ok(())
}
