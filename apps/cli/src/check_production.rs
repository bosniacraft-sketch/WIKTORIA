//! `wiktoria-cli check-production --save <file.v2>`
//!
//! Computes every province's farm or mine (RGO) output for one day from a save alone: the
//! workers the save lists as employed, the port's own modifier totals (wiktoria_simulation::modifiers) and
//! the measured production rule (wiktoria_simulation::production). Output times the save's price is compared
//! with the income the original recorded for the province (`last_income`), per good and for the
//! world. It runs on any save; it is a check of
//! shape (D9: country and world scale), not of exact values.
//!
//! Not modelled here: the overseas penalty, mobilisation, secondary-power rank.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use wiktoria_data::script::{Block, Value};
use wiktoria_simulation::modifiers::{country_totals, province_totals, CountryModifierInputs, CountryModifierRules, ProvinceModifierInputs, ProvinceModifierRules, ProvinceSources, Rank, Totals};
use wiktoria_simulation::production::{rgo_factors, rgo_output, rgo_size, setup_base_size, ProductionBonus, RgoInputs, RgoRules};
use wiktoria_simulation::Fixed;

use crate::{config, load_world, root, Args};

fn timed(block: &Block) -> Vec<String> {
    block.get_all("modifier").filter_map(Value::as_block).filter_map(|m| m.get("modifier").and_then(Value::as_str)).map(str::to_string).collect()
}

fn numbers(block: Option<&Block>) -> Vec<usize> {
    block.map(|b| b.bare_scalars().iter().filter_map(|s| s.parse().ok()).collect()).unwrap_or_default()
}

pub fn check_production(args: &Args) -> Result<(), String> {
    let save_path = args.save.as_ref().ok_or("usage: check-production --save <file.v2>")?;
    let engine_rules = config::load_engine_rules(&root())?;
    let (rules, crules, prules) = (RgoRules::from_engine_rules(&engine_rules)?, CountryModifierRules::from_engine_rules(&engine_rules)?, ProvinceModifierRules::from_engine_rules(&engine_rules)?);
    let w = load_world(args)?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let bytes = std::fs::read(save_path).map_err(|e| format!("cannot read {save_path}: {e}"))?;
    let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
    let doc = &save.doc.root;

    // Prices of the day.
    let mut price: BTreeMap<String, f64> = BTreeMap::new();
    if let Some(pool) = doc.get("worldmarket").and_then(Value::as_block).and_then(|wm| wm.get("price_pool")).and_then(Value::as_block) {
        for (good, v, _) in pool.entries() {
            if let Some(x) = v.as_f64() {
                price.insert(good.to_string(), x);
            }
        }
    }

    // Country side: modifier totals, production bonuses, states.
    let great: BTreeSet<usize> = numbers(doc.get("great_nations").and_then(Value::as_block)).into_iter().collect();
    let at_war: BTreeSet<String> = doc.get_all("active_war").filter_map(Value::as_block).flat_map(|b| b.get_all("attacker").chain(b.get_all("defender")).filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>()).collect();
    let mut literate: BTreeMap<&str, (f64, f64)> = BTreeMap::new();
    for p in save.provinces.values() {
        let Some(owner) = p.owner.as_deref() else { continue };
        let e = literate.entry(owner).or_default();
        for pop in &p.pops {
            e.0 += pop.size;
            e.1 += pop.size * pop.literacy;
        }
    }
    let parties: Vec<&wiktoria_data::countries::Party> = w.countries.iter().flat_map(|c| c.country.parties.iter()).collect();
    let party_groups: BTreeSet<&str> = w.defs.issues.iter().filter(|g| g.category == crules.party_issue_category).map(|g| g.name.as_str()).collect();
    let mut country: BTreeMap<String, (Totals, ProductionBonus)> = BTreeMap::new();
    let mut state_of: BTreeMap<u16, usize> = BTreeMap::new();
    let mut states: Vec<Vec<u16>> = Vec::new();
    for (i, c) in w.countries.iter().enumerate() {
        let tag = c.country.tag.as_str();
        let Some(block) = doc.get(tag).and_then(Value::as_block) else { continue };
        let text = |k: &str| block.get(k).and_then(Value::as_str).map(str::to_string);
        let num = |k: &str| block.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        let rank = if great.contains(&(i + 1)) {
            Rank::GreatPower
        } else if text("civilized").as_deref() == Some("yes") {
            Rank::Civilized
        } else {
            Rank::Uncivilized
        };
        let inventions = numbers(block.get("active_inventions").and_then(Value::as_block));
        let input = CountryModifierInputs {
            rank: Some(rank),
            at_war: at_war.contains(tag),
            plurality: num("plurality"),
            literacy: literate.get(tag).filter(|(n, _)| *n > 0.0).map_or(0.0, |(n, l)| l / n),
            war_exhaustion: num("war_exhaustion"),
            badboy: num("badboy"),
            national_value: text("nationalvalue"),
            school: text("schools"),
            reforms: w.defs.issues.iter().filter_map(|g| Some((g.name.clone(), text(&g.name)?))).collect(),
            party_policies: match block.get("ruling_party").and_then(Value::as_f64) {
                Some(n) if n >= 1.0 => parties.get(n as usize - 1).map(|p| p.policies.iter().filter(|(g, _)| party_groups.contains(g.as_str())).map(|(_, o)| o.clone()).collect()).unwrap_or_default(),
                _ => Vec::new(),
            },
            inventions: inventions.iter().filter_map(|n| w.defs.modifiers.inventions.get(n.wrapping_sub(1))).map(|(name, _)| name.clone()).collect(),
            event_modifiers: timed(block),
        };
        let mut bonus = ProductionBonus::default();
        for (t, _, _) in block.get("technology").and_then(Value::as_block).map(|b| b.entries().collect::<Vec<_>>()).unwrap_or_default() {
            if let Some(e) = w.defs.tech_production.get(t) {
                bonus.add(&rules, e);
            }
        }
        for n in &inventions {
            if let Some(e) = w.defs.invention_production.get(n.wrapping_sub(1)) {
                bonus.add(&rules, e);
            }
        }
        country.insert(tag.to_string(), (country_totals(&w.defs.modifiers, &w.defs.issues, &crules, &input), bonus));
        for s in block.get_all("state").filter_map(Value::as_block) {
            let members: Vec<u16> = numbers(s.get("provinces").and_then(Value::as_block)).into_iter().map(|p| p as u16).collect();
            for p in &members {
                state_of.insert(*p, states.len());
            }
            states.push(members);
        }
    }

    let src = ProvinceSources {
        statics: &w.defs.modifiers.static_modifiers,
        terrain: &w.defs.terrain,
        climates: &w.defs.climates,
        continents: &w.defs.continents,
        crimes: &w.defs.crimes,
        buildings: &w.defs.buildings,
        events: &w.defs.modifiers.event_modifiers,
    };
    // Per good: provinces, workers, the original's income, the port's value of output.
    let mut by_good: BTreeMap<String, (usize, i64, f64, f64)> = BTreeMap::new();
    let mut by_owner: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    let (mut close, mut compared) = (0usize, 0usize);
    for (id, sp) in &save.provinces {
        let (Some(owner), Some(block), Some(def)) = (sp.owner.as_ref(), doc.get(&id.to_string()).and_then(Value::as_block), w.province(*id)) else { continue };
        let Some(rgo_block) = block.get("rgo").and_then(Value::as_block) else { continue };
        let Some(good) = rgo_block.get("goods_type").and_then(Value::as_str) else { continue };
        let Some(rgo) = w.defs.rgo_types.iter().find(|r| r.good == good) else { continue };
        let income = rgo_block.get("last_income").and_then(Value::as_f64).unwrap_or(0.0) / 1000.0;
        let mut employed: BTreeMap<String, i64> = BTreeMap::new();
        let list = rgo_block.get("employment").and_then(Value::as_block).and_then(|e| e.get("employees")).and_then(Value::as_block);
        for e in list.map(|e| e.items.iter().filter_map(|i| i.value.as_block()).collect::<Vec<_>>()).unwrap_or_default() {
            let kind = e.get("province_pop_id").and_then(Value::as_block).and_then(|p| p.get("type")).and_then(Value::as_f64);
            if let Some(t) = kind.and_then(|t| w.defs.pop_types.get((t as usize).wrapping_sub(1))) {
                *employed.entry(t.name.clone()).or_default() += e.get("count").and_then(Value::as_f64).unwrap_or(0.0) as i64;
            }
        }
        let workers: i64 = employed.values().sum();
        if workers == 0 {
            continue;
        }
        let level = |k: &str| block.get(k).and_then(Value::as_block).and_then(|b| b.bare_scalars().first().and_then(|s| s.parse::<f64>().ok()));
        let input = ProvinceModifierInputs {
            is_sea: false,
            coastal_sea: false,
            terrain: def.terrain.clone(),
            climate: def.climate.clone(),
            continent: def.continent.clone(),
            crime: block.get("crime").and_then(Value::as_f64).map(|c| c as usize),
            buildings: w.defs.buildings.iter().filter_map(|b| Some((b.name.clone(), level(&b.name).filter(|l| *l > 0.0)?))).collect(),
            besieged: false,
            event_modifiers: timed(block),
        };
        let province = province_totals(&src, &prules, &input);
        let empty = (Totals::new(), ProductionBonus::default());
        let (ctot, bonus) = country.get(owner).unwrap_or(&empty);
        // Base size as fixed at setup: the history's good and pops (x).
        let setup_rgo = def.trade_goods.as_deref().and_then(|g| w.defs.rgo_types.iter().find(|r| r.good == g)).unwrap_or(rgo);
        let worker_types: HashSet<&str> = setup_rgo.employees.iter().map(|j| j.pop_type.as_str()).collect();
        let setup_workers: i64 = def.pops.iter().filter(|p| worker_types.contains(w.defs.pop_types[p.pop_type as usize].name.as_str())).map(|p| p.size as i64).sum();
        let base = setup_base_size(&rules, setup_rgo, &province, setup_workers, sp.controller.as_deref() == Some(owner.as_str()));
        let owner_type = rgo.owner.as_ref().map(|o| o.pop_type.clone()).unwrap_or_default();
        let (mut owners, mut people) = (0i64, 0i64);
        for q in state_of.get(id).map(|s| states[*s].clone()).unwrap_or_else(|| vec![*id]) {
            for pop in save.provinces.get(&q).map(|p| p.pops.as_slice()).unwrap_or_default() {
                people += pop.size as i64;
                if pop.pop_type == owner_type {
                    owners += pop.size as i64;
                }
            }
        }
        let inp = RgoInputs { rgo, base, bonus, real_country: true, country: ctot, province: &province, employed: &employed, owner_share: (owners, people), mobilisation: Fixed::ONE, overseas_penalty: Fixed::ZERO };
        let size = rgo_size(&rules, &inp);
        let value = rgo_output(rgo, rgo_factors(&rules, &inp, size), size).to_f64() * price.get(good).copied().unwrap_or(0.0);
        if args.positional.first().and_then(|x| x.parse::<u16>().ok()) == Some(*id) {
            let f = rgo_factors(&rules, &inp, size);
            println!("province {id} {good} of {owner}: base {base}, size {:.3}, workers {workers} of {}, factors input {:.4} output {:.4} throughput {:.4}", size.to_f64(), wiktoria_simulation::production::max_workers(rgo, size), f[0].to_f64(), f[1].to_f64(), f[2].to_f64());
            println!("  game income {income:.3}, port value {value:.3}, price {:?}, setup workers {setup_workers}, owners {owners} of {people}", price.get(good));
            println!("  bonus: {bonus:?}");
            println!("  country totals: {:?}", ctot.iter().filter(|(k, v)| **v != 0 && (k.contains("rgo") || k.contains("mine") || k.contains("farm") || k.contains("throughput") || k.contains("output") || k.contains("input"))).collect::<Vec<_>>());
            println!("  province totals: {:?}", province.iter().filter(|(_, v)| **v != 0).collect::<Vec<_>>());
            println!("  buildings: {:?}", input.buildings);
        }
        let e = by_good.entry(good.to_string()).or_default();
        (e.0, e.1, e.2, e.3) = (e.0 + 1, e.1 + workers, e.2 + income, e.3 + value);
        let o = by_owner.entry(owner.clone()).or_default();
        (o.0, o.1) = (o.0 + income, o.1 + value);
        if income > 0.0 {
            compared += 1;
            close += ((value / income - 1.0).abs() <= 0.05) as usize;
        }
    }
    println!("{}: farm and mine output of one day, port against the income the original recorded", save.date);
    println!("{:<16}{:>6}{:>12}{:>14}{:>14}{:>8}", "good", "prov.", "workers", "game income", "port value", "ratio");
    let (mut game, mut port) = (0.0, 0.0);
    for (good, (n, workers, income, value)) in &by_good {
        println!("{good:<16}{n:>6}{workers:>12}{income:>14.1}{value:>14.1}{:>8.3}", value / income.max(1e-9));
        game += income;
        port += value;
    }
    println!("world: game {game:.1}, port {port:.1}, ratio {:.3}", port / game.max(1e-9));
    println!("provinces within 5% of the original: {close} of {compared} ({:.1}%)", close as f64 / compared.max(1) as f64 * 100.0);
    let mut owners: Vec<_> = by_owner.into_iter().filter(|(_, (g, _))| *g > 0.0).collect();
    owners.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
    println!("largest producers (port / game): {}", owners.iter().take(14).map(|(t, (g, p))| format!("{t} {:.3}", p / g)).collect::<Vec<_>>().join(", "));
    Ok(())
}
