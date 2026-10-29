//! Scoreboard (D9): how close a candidate run is to a reference run of the real game.
//!
//! Both runs are folders of saves in the original format, matched by date. Each save is reduced
//! to a set of aggregate numbers in families (prices, supply, population per country, ...). A
//! number's closeness is 1 - |candidate - reference| / |reference|, never below 0. A family's
//! score is the weighted mean closeness of its numbers (per-country families weigh each country
//! by its size in the reference, so large countries count more; per-good families weigh goods
//! equally). The overall score is the plain mean of the family scores.
//!
//!   wiktoria-cli score <reference folder>                      null model: nothing changes after the first save
//!   wiktoria-cli score <reference folder> <candidate folder>   e.g. a second run of the game, or the port
//!   wiktoria-cli score <reference> <candidate> <ceiling>       also reports the gap closed (D10): for each
//!       family, how far the candidate got from the null model (0%) to the ceiling run (100%)
//!
//! The null model is the floor the port has to beat; two runs of the real game from the same
//! start give the ceiling, because the game itself takes a different random path every time.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use wiktoria_data::date::Date;
use wiktoria_data::script::{Block, Value};

use crate::{load_world, Args};

/// family -> item (good or country tag) -> value.
pub type Metrics = BTreeMap<&'static str, BTreeMap<String, f64>>;

/// Families whose items are weighted equally; all others are weighted by the reference value.
const EQUAL_WEIGHT: [&str; 2] = ["price", "supply"];

fn num(block: &Block, key: &str) -> Option<f64> {
    block.get(key).and_then(Value::as_f64)
}

/// Aggregate numbers of one save.
pub fn metrics(save: &wiktoria_data::save::SaveGame) -> Metrics {
    let mut m: Metrics = BTreeMap::new();
    let root = &save.doc.root;
    if let Some(wm) = root.get("worldmarket").and_then(Value::as_block) {
        for (family, pool) in [("price", "price_pool"), ("supply", "supply_pool")] {
            if let Some(b) = wm.get(pool).and_then(Value::as_block) {
                let out = m.entry(family).or_default();
                for (good, v, _) in b.entries() {
                    if let Some(x) = v.as_f64() {
                        out.insert(good.to_string(), x);
                    }
                }
            }
        }
    }
    for (id, prov) in &save.provinces {
        let Some(owner) = &prov.owner else { continue };
        let mut add = |family: &'static str, x: f64| *m.entry(family).or_default().entry(owner.clone()).or_insert(0.0) += x;
        add("population", prov.pops.iter().map(|p| p.size).sum());
        add("pop_money", prov.pops.iter().map(|p| p.money).sum());
        add("pop_count", prov.pops.len() as f64);
        let Some(rgo) = root.get(&id.to_string()).and_then(Value::as_block).and_then(|b| b.get("rgo")).and_then(Value::as_block) else { continue };
        add("rgo_income", num(rgo, "last_income").unwrap_or(0.0));
        let employed: f64 = rgo
            .get("employment")
            .and_then(Value::as_block)
            .and_then(|e| e.get("employees"))
            .and_then(Value::as_block)
            .map(|list| list.items.iter().filter_map(|i| i.value.as_block()).filter_map(|b| num(b, "count")).sum())
            .unwrap_or(0.0);
        add("rgo_employed", employed);
    }
    for tag in &save.countries {
        let Some(c) = root.get(tag).and_then(Value::as_block) else { continue };
        for (family, key) in [("treasury", "money"), ("tax_base", "tax_base")] {
            if let Some(x) = num(c, key) {
                m.entry(family).or_default().insert(tag.clone(), x);
            }
        }
    }
    m
}

/// Closeness of one number: 1 when equal, 0 when off by the reference's own size or more.
pub fn closeness(reference: f64, candidate: f64) -> f64 {
    let scale = reference.abs();
    if scale < 1e-9 {
        return if candidate.abs() < 1e-9 { 1.0 } else { 0.0 };
    }
    (1.0 - (candidate - reference).abs() / scale).max(0.0)
}

/// Score of one family: weighted mean closeness over the reference's items, and the signed mean
/// relative error (bias, D9). An item missing in the candidate counts as 0.
pub fn family_score(family: &str, reference: &BTreeMap<String, f64>, candidate: &BTreeMap<String, f64>) -> (f64, f64) {
    let equal = EQUAL_WEIGHT.contains(&family);
    let (mut total, mut sum, mut bias) = (0.0, 0.0, 0.0);
    for (item, r) in reference {
        let w = if equal { 1.0 } else { r.abs() };
        if w <= 0.0 {
            continue;
        }
        let c = candidate.get(item).copied().unwrap_or(0.0);
        total += w;
        sum += w * closeness(*r, c);
        if r.abs() > 1e-9 {
            bias += w * ((c - r) / r.abs()).clamp(-1.0, 1.0);
        }
    }
    if total == 0.0 { (1.0, 0.0) } else { (sum / total, bias / total) }
}

/// How far a candidate got from the floor to the ceiling, as a fraction. None when the two
/// anchors are too close together for the family to say anything (less than 3 points apart).
pub fn gap_closed(floor: f64, candidate: f64, ceiling: f64) -> Option<f64> {
    (ceiling - floor >= 0.03).then(|| (candidate - floor) / (ceiling - floor))
}

/// Reads every `.v2` save of a folder; when several have the same date the newest file wins.
fn read_run(dir: &Path, pop_types: &HashSet<String>, cultures: &HashSet<String>) -> Result<BTreeMap<Date, Metrics>, String> {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("v2")))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    files.sort();
    let mut out = BTreeMap::new();
    for (_, path) in files {
        let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let save = wiktoria_data::save::parse_save(&bytes, pop_types, cultures).map_err(|e| format!("{}: {e}", path.display()))?;
        out.insert(save.date, metrics(&save));
    }
    if out.is_empty() {
        return Err(format!("no .v2 saves in {}", dir.display()));
    }
    Ok(out)
}

pub fn score(args: &Args) -> Result<(), String> {
    let reference_dir = args.positional.first().ok_or("usage: score <reference folder> [<candidate folder>]")?;
    let w = load_world(args)?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let reference = read_run(Path::new(reference_dir), &pop_types, &cultures)?;
    let (start, baseline) = reference.iter().next().map(|(d, m)| (*d, m.clone())).ok_or("empty reference")?;
    let candidate = match args.positional.get(1) {
        Some(dir) => Some(read_run(Path::new(dir), &pop_types, &cultures)?),
        None => None,
    };
    println!("reference: {reference_dir} ({} saves from {start})", reference.len());
    println!("candidate: {}", args.positional.get(1).map_or("null model (the first save, unchanged)", |s| s.as_str()));

    let families: Vec<&'static str> = baseline.keys().copied().collect();
    print!("{:<12}", "date");
    for f in &families {
        print!("{f:>13}");
    }
    println!("{:>10}", "overall");
    let empty = BTreeMap::new();
    let (mut last, mut sums, mut rows) = (None, vec![0.0; families.len() + 1], 0usize);
    for (date, r) in reference.iter().skip(1) {
        let c = match &candidate {
            Some(run) => match run.get(date) {
                Some(m) => m,
                None => continue,
            },
            None => &baseline,
        };
        let scores: Vec<(f64, f64)> = families.iter().map(|f| family_score(f, r.get(f).unwrap_or(&empty), c.get(f).unwrap_or(&empty))).collect();
        let overall = scores.iter().map(|s| s.0).sum::<f64>() / scores.len() as f64;
        print!("{:<12}", date.to_string());
        for (i, s) in scores.iter().enumerate() {
            print!("{:>12.1}%", s.0 * 100.0);
            sums[i] += s.0;
        }
        println!("{:>9.1}%", overall * 100.0);
        sums[families.len()] += overall;
        rows += 1;
        last = Some((*date, scores, overall));
    }
    let Some((date, scores, overall)) = last else { return Err("no dates in common after the first save".into()) };
    print!("{:<12}", "mean");
    for s in &sums {
        print!("{:>12.1}%", s / rows as f64 * 100.0);
    }
    println!();
    println!("\nlast date {date}: overall {:.1}%. Signed mean error per family (candidate vs reference; a one-sided error compounds):", overall * 100.0);
    for (f, s) in families.iter().zip(&scores) {
        println!("  {f:<13} score {:>5.1}%   bias {:>+6.1}%", s.0 * 100.0, s.1 * 100.0);
    }
    if let Some(dir) = args.positional.get(2) {
        let ceiling = read_run(Path::new(dir), &pop_types, &cultures)?;
        let (Some(r), Some(top)) = (reference.get(&date), ceiling.get(&date)) else { return Err(format!("ceiling run has no save for {date}")) };
        println!("\ngap closed on {date} (0% = null model, 100% = ceiling run {dir}):");
        let mut closed = Vec::new();
        for (f, s) in families.iter().zip(&scores) {
            let floor = family_score(f, r.get(f).unwrap_or(&empty), baseline.get(f).unwrap_or(&empty)).0;
            let ceil = family_score(f, r.get(f).unwrap_or(&empty), top.get(f).unwrap_or(&empty)).0;
            match gap_closed(floor, s.0, ceil) {
                Some(g) => {
                    println!("  {f:<13} floor {:>5.1}%  candidate {:>5.1}%  ceiling {:>5.1}%  -> {:>6.1}%", floor * 100.0, s.0 * 100.0, ceil * 100.0, g * 100.0);
                    closed.push(g);
                }
                None => println!("  {f:<13} floor {:>5.1}%  candidate {:>5.1}%  ceiling {:>5.1}%  -> not a target (floor and ceiling within 3 points)", floor * 100.0, s.0 * 100.0, ceil * 100.0),
            }
        }
        if !closed.is_empty() {
            println!("  mean over target families: {:.1}%", closed.iter().sum::<f64>() / closed.len() as f64 * 100.0);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(items: &[(&str, f64)]) -> BTreeMap<String, f64> {
        items.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn closeness_is_relative_and_bounded() {
        assert_eq!(closeness(100.0, 100.0), 1.0);
        assert!((closeness(100.0, 95.0) - 0.95).abs() < 1e-12);
        assert!((closeness(100.0, 105.0) - 0.95).abs() < 1e-12);
        assert_eq!(closeness(100.0, 250.0), 0.0);
        assert_eq!(closeness(0.0, 0.0), 1.0);
        assert_eq!(closeness(0.0, 3.0), 0.0);
        assert!((closeness(-10.0, -9.0) - 0.9).abs() < 1e-12);
    }

    #[test]
    fn gap_closed_between_anchors() {
        assert_eq!(gap_closed(0.60, 0.60, 0.90), Some(0.0));
        assert!((gap_closed(0.60, 0.75, 0.90).unwrap() - 0.5).abs() < 1e-12);
        assert!((gap_closed(0.60, 0.90, 0.90).unwrap() - 1.0).abs() < 1e-12);
        assert_eq!(gap_closed(0.99, 0.995, 1.0), None, "anchors too close: not a target");
    }

    #[test]
    fn country_families_weigh_by_size_and_goods_equally() {
        let reference = map(&[("BIG", 900.0), ("SMALL", 100.0)]);
        let candidate = map(&[("BIG", 900.0), ("SMALL", 50.0)]);
        let (s, bias) = family_score("population", &reference, &candidate);
        assert!((s - 0.95).abs() < 1e-12, "small country is 10% of the weight and half right");
        assert!((bias + 0.05).abs() < 1e-12);
        let (s, _) = family_score("price", &reference, &candidate);
        assert!((s - 0.75).abs() < 1e-12, "goods count equally");
        let (s, _) = family_score("population", &reference, &map(&[("BIG", 900.0)]));
        assert!((s - 0.9).abs() < 1e-12, "a missing item scores 0");
        assert_eq!(family_score("population", &BTreeMap::new(), &candidate), (1.0, 0.0));
    }
}
