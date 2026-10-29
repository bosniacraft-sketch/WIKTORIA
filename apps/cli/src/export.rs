//! State tables (D10): a small, flat format for analysis and for the scoreboard.
//!
//! An original save is 18-23 MB of nested text. `wiktoria-cli export` reduces each save to four CSV
//! tables in a folder named after the save's date, which load in a moment in any tool:
//!
//!   pops.csv       province,owner,type,id,size,culture,religion,money,literacy,con,mil,life,everyday,luxury
//!   provinces.csv  province,owner,controller,pops,population,rgo_good,rgo_employed,rgo_income
//!   countries.csv  tag,money,tax_base,prestige,plurality,civilized
//!   goods.csv      good,price,supply,demand
//!
//! Needs fulfilment columns hold the save's value; a tier the save leaves out is written empty
//! (the game omits `life_needs` when it is fully met). The port's simulation is meant to write
//! the same tables, so a port run and a game run can be compared without the port having to
//! produce an original-format save.
//!
//!   wiktoria-cli export <save.v2 or folder of saves> <output folder>

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use wiktoria_data::script::{Block, Value};

use crate::{load_world, Args};

fn text(b: &Block, key: &str) -> String {
    b.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// The four tables of one save, as CSV text.
pub fn tables(save: &wiktoria_data::save::SaveGame, pop_types: &HashSet<String>) -> [(&'static str, String); 4] {
    let root = &save.doc.root;
    let mut pops = String::from("province,owner,type,id,size,culture,religion,money,literacy,con,mil,life,everyday,luxury\n");
    let mut provinces = String::from("province,owner,controller,pops,population,rgo_good,rgo_employed,rgo_income\n");
    for (id, prov) in &save.provinces {
        let owner = prov.owner.as_deref().unwrap_or("");
        let Some(block) = root.get(&id.to_string()).and_then(Value::as_block) else { continue };
        let known: BTreeMap<u64, &wiktoria_data::save::SavePop> = prov.pops.iter().map(|p| (p.id, p)).collect();
        for item in &block.items {
            let (Some(kind), Some(p)) = (item.key.as_deref(), item.value.as_block()) else { continue };
            if !pop_types.contains(kind) {
                continue;
            }
            let pid = p.get("id").and_then(Value::as_f64).unwrap_or(0.0) as u64;
            let (culture, religion) = known.get(&pid).map_or(("", ""), |k| (k.culture.as_str(), k.religion.as_str()));
            let _ = writeln!(
                pops,
                "{id},{owner},{kind},{pid},{},{culture},{religion},{},{},{},{},{},{},{}",
                text(p, "size"),
                text(p, "money"),
                text(p, "literacy"),
                text(p, "con"),
                text(p, "mil"),
                text(p, "life_needs"),
                text(p, "everyday_needs"),
                text(p, "luxury_needs"),
            );
        }
        let rgo = block.get("rgo").and_then(Value::as_block);
        let employed: f64 = rgo
            .and_then(|r| r.get("employment"))
            .and_then(Value::as_block)
            .and_then(|e| e.get("employees"))
            .and_then(Value::as_block)
            .map(|list| list.items.iter().filter_map(|i| i.value.as_block()).filter_map(|b| b.get("count").and_then(Value::as_f64)).sum())
            .unwrap_or(0.0);
        let _ = writeln!(
            provinces,
            "{id},{owner},{},{},{},{},{employed},{}",
            prov.controller.as_deref().unwrap_or(""),
            prov.pops.len(),
            prov.population(),
            rgo.map(|r| text(r, "goods_type")).unwrap_or_default(),
            rgo.map(|r| text(r, "last_income")).unwrap_or_default(),
        );
    }
    let mut countries = String::from("tag,money,tax_base,prestige,plurality,civilized\n");
    for tag in &save.countries {
        let Some(c) = root.get(tag).and_then(Value::as_block) else { continue };
        let _ = writeln!(countries, "{tag},{},{},{},{},{}", text(c, "money"), text(c, "tax_base"), text(c, "prestige"), text(c, "plurality"), text(c, "civilized"));
    }
    let mut goods = String::from("good,price,supply,demand\n");
    if let Some(wm) = root.get("worldmarket").and_then(Value::as_block) {
        let pool = |name: &str| -> BTreeMap<String, String> {
            wm.get(name)
                .and_then(Value::as_block)
                .map(|b| b.entries().filter_map(|(k, v, _)| Some((k.to_string(), v.as_str()?.to_string()))).collect())
                .unwrap_or_default()
        };
        let (price, supply, demand) = (pool("price_pool"), pool("supply_pool"), pool("real_demand"));
        let names: std::collections::BTreeSet<&String> = price.keys().chain(supply.keys()).chain(demand.keys()).collect();
        let empty = String::new();
        for g in names {
            let _ = writeln!(goods, "{g},{},{},{}", price.get(g).unwrap_or(&empty), supply.get(g).unwrap_or(&empty), demand.get(g).unwrap_or(&empty));
        }
    }
    [("pops.csv", pops), ("provinces.csv", provinces), ("countries.csv", countries), ("goods.csv", goods)]
}

pub fn export(args: &Args) -> Result<(), String> {
    let (Some(input), Some(out)) = (args.positional.first(), args.positional.get(1)) else {
        return Err("usage: export <save.v2 or folder of saves> <output folder>".into());
    };
    let w = load_world(args)?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let input = Path::new(input);
    let mut files: Vec<PathBuf> = if input.is_dir() {
        std::fs::read_dir(input)
            .map_err(|e| format!("cannot read {}: {e}", input.display()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("v2")))
            .collect()
    } else {
        vec![input.to_path_buf()]
    };
    files.sort();
    for path in &files {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures).map_err(|e| format!("{}: {e}", path.display()))?;
        let d = save.date;
        let dir = Path::new(out).join(format!("{:04}_{:02}_{:02}", d.year, d.month, d.day));
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let mut total = 0;
        for (name, body) in tables(&save, &pop_types) {
            total += body.len();
            std::fs::write(dir.join(name), body).map_err(|e| format!("cannot write {name}: {e}"))?;
        }
        println!("{} -> {} ({} KB from {} KB)", path.display(), dir.display(), total / 1024, bytes.len() / 1024);
    }
    Ok(())
}
