//! `wiktoria-cli fire-event <id> --save <file.v2> [<TAG> [<option number>]]`
//!
//! Loads the state of a save, finds the event with that id in the install's `events` folder and
//! applies one of its options to the given country (default: the save's player, option 1), the
//! way the console command `event <id>` does in the game. Prints what was applied, what the
//! engine does not support yet, and a summary of what changed. With a second folder argument the
//! resulting state is also written as the state tables of `wiktoria-cli export`.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use wiktoria_data::script::{self, Block, Value};
use wiktoria_simulation::script::{run_effects, EffectRules, Report, Scope, State};
use wiktoria_simulation::Calendar;

use crate::{config, load_world, root, Args};

/// The option blocks of the event with this id, and its title, searching every events file.
pub(crate) fn find_event(install: &Path, id: u64) -> Result<(String, Vec<Block>), String> {
    let dir = install.join("events");
    let mut files: Vec<_> = std::fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    files.sort();
    for path in files {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let doc = script::parse_bytes(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        for (key, value, _) in doc.root.entries() {
            let (true, Some(event)) = (key == "country_event" || key == "province_event", value.as_block()) else { continue };
            if event.get("id").and_then(Value::as_f64) != Some(id as f64) {
                continue;
            }
            let title = event.get("title").and_then(Value::as_str).unwrap_or("").to_string();
            let options = event.get_all("option").filter_map(Value::as_block).cloned().collect();
            return Ok((format!("{title} ({key}, {})", path.file_name().unwrap_or_default().to_string_lossy()), options));
        }
    }
    Err(format!("no event with id {id} in {}", dir.display()))
}

pub fn fire_event(args: &Args) -> Result<(), String> {
    let id: u64 = args.positional.first().and_then(|s| s.parse().ok()).ok_or("usage: fire-event <id> --save <file.v2> [<TAG> [<option number>]]")?;
    let save_path = args.save.as_ref().ok_or("--save <file.v2> is required")?;
    let w = load_world(args)?;
    let root = root();
    let engine_rules = config::load_engine_rules(&root)?;
    let install = config::install_dir(&root, args.install.clone())?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let bytes = std::fs::read(save_path).map_err(|e| format!("cannot read {save_path}: {e}"))?;
    let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
    let tag = args.positional.get(1).cloned().or(save.player.clone()).ok_or("the save names no player; give a TAG")?;
    let pick: usize = args.positional.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);

    let (title, options) = find_event(&install, id)?;
    let option = options.get(pick.wrapping_sub(1)).ok_or(format!("event {id} has {} options, not {pick}", options.len()))?;
    let rules = EffectRules::from_engine_rules(&engine_rules)?;
    let mut before = State::from_save(&save);
    before.attach_world(&w, &save);
    before.seed = rules.seed;
    before.this = Some(Scope::Country(tag.clone()));
    let mut state = before.clone();
    let mut report = Report::default();
    run_effects(&mut state, &Calendar::from_engine_rules(&engine_rules)?, &rules, option, &Scope::Country(tag.clone()), &mut report);

    println!("event {id}: {title}");
    println!("fired for {tag} on {}, option {pick} of {}: {}", state.date, options.len(), option.get("name").and_then(Value::as_str).unwrap_or(""));
    println!("applied:     {}", list(&report.applied));
    println!("unsupported: {}", if report.unsupported.is_empty() { "nothing".to_string() } else { list(&report.unsupported) });

    // What changed.
    let mut added: BTreeMap<(String, String), (usize, BTreeSet<String>)> = BTreeMap::new();
    for (pid, p) in &state.provinces {
        for m in p.modifiers.iter().skip(before.provinces[pid].modifiers.len()) {
            let e = added.entry((m.name.clone(), m.expires.map_or("never".into(), |d| d.to_string()))).or_default();
            e.0 += 1;
            e.1.extend(p.owner.clone());
        }
    }
    for ((name, expires), (n, owners)) in &added {
        println!("province modifier {name} (expires {expires}): {n} provinces of {} countries{}", owners.len(), if owners.contains(&tag) { format!(", including {tag}") } else { format!(", none of {tag}") });
    }
    let changed: Vec<&String> = state.countries.keys().filter(|t| state.countries[*t] != before.countries[*t]).collect();
    println!("countries changed: {} of {} ({tag} {})", changed.len(), state.countries.len(), if changed.contains(&&tag) { "changed" } else { "unchanged" });
    for t in ["ENG", "FRA", "RUS", "USA", "SPA", tag.as_str()] {
        if let (Some(b), Some(a)) = (before.countries.get(t), state.countries.get(t)) {
            println!("  {t}: prestige {:.3} -> {:.3}, money {:.0} -> {:.0}", b.prestige, a.prestige, b.money, a.money);
        }
    }
    let (mut pops, mut touched, mut con, mut mil) = (0usize, 0usize, 0.0, 0.0);
    for (pid, p) in &state.provinces {
        for (i, pop) in p.pops.iter().enumerate() {
            pops += 1;
            if *pop != before.provinces[pid].pops[i] {
                touched += 1;
                con += pop.consciousness;
                mil += pop.militancy;
            }
        }
    }
    if touched > 0 {
        println!("pops changed: {touched} of {pops}; their mean consciousness {:.2}, militancy {:.2}", con / touched as f64, mil / touched as f64);
    } else {
        println!("pops changed: 0 of {pops}");
    }
    Ok(())
}

fn list(m: &BTreeMap<String, usize>) -> String {
    m.iter().map(|(k, n)| format!("{k} x{n}")).collect::<Vec<_>>().join(", ")
}

/// Every effect block of the install: (label, block, true when it belongs to a province event).
/// Event options and `immediate` blocks from `events`, and each decision's `effect` from `decisions`.
fn all_effect_blocks(install: &wiktoria_data::install_fs::InstallFs) -> Result<Vec<(String, Block, bool)>, String> {
    let mut out = Vec::new();
    for folder in ["events", "decisions"] {
        let files = install.dir_files(folder, "txt")?;
        for path in files {
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let Ok(doc) = script::parse_bytes(&bytes) else { continue };
            for (key, value, _) in doc.root.entries() {
                let Some(top) = value.as_block() else { continue };
                if key == "country_event" || key == "province_event" {
                    let id = top.get("id").and_then(Value::as_str).unwrap_or("?").to_string();
                    for (i, b) in top.get_all("option").chain(top.get_all("immediate")).filter_map(Value::as_block).enumerate() {
                        out.push((format!("event {id} #{}", i + 1), b.clone(), key == "province_event"));
                    }
                } else {
                    // decisions: political_decisions = { name = { potential allow effect } ... }
                    for (name, d, _) in top.entries() {
                        if let Some(b) = d.as_block().and_then(|d| d.get("effect")).and_then(Value::as_block) {
                            out.push((format!("decision {name}"), b.clone(), false));
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Every condition block of the install that decides whether something may happen:
/// (kind, block, true when it belongs to a province event). An event's `trigger`, and each
/// decision's `potential` and `allow`.
fn all_condition_blocks(install: &wiktoria_data::install_fs::InstallFs) -> Result<Vec<(&'static str, Block, bool)>, String> {
    let mut out = Vec::new();
    for folder in ["events", "decisions"] {
        let files = install.dir_files(folder, "txt")?;
        for path in files {
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let Ok(doc) = script::parse_bytes(&bytes) else { continue };
            for (key, value, _) in doc.root.entries() {
                let Some(top) = value.as_block() else { continue };
                if key == "country_event" || key == "province_event" {
                    out.extend(top.get("trigger").and_then(Value::as_block).map(|b| ("event trigger", b.clone(), key == "province_event")));
                } else {
                    for (_, d, _) in top.entries() {
                        let Some(d) = d.as_block() else { continue };
                        for (kind, word) in [("decision potential", "potential"), ("decision allow", "allow")] {
                            out.extend(d.get(word).and_then(Value::as_block).map(|b| (kind, b.clone(), false)));
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

/// `wiktoria-cli script-coverage --save <file.v2> [<TAG>]`: runs every event option and decision
/// effect of the install once, for the given country (default: the save's player; province
/// events run on its capital), and reports how much the engine understood. The state is
/// carried from one block to the next, so only the tally is meaningful, not the end state.
pub fn script_coverage(args: &Args) -> Result<(), String> {
    let save_path = args.save.as_ref().ok_or("usage: script-coverage --save <file.v2> [<TAG>]")?;
    let w = load_world(args)?;
    let root = root();
    let engine_rules = config::load_engine_rules(&root)?;
    let install = config::install_dir(&root, args.install.clone())?;
    let mod_files: Vec<_> = args.mod_files.iter().map(std::path::PathBuf::from).collect();
    let install_fs = wiktoria_data::install_fs::InstallFs::with_mods(&install, &mod_files)?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let bytes = std::fs::read(save_path).map_err(|e| format!("cannot read {save_path}: {e}"))?;
    let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
    let tag = args.positional.first().cloned().or(save.player.clone()).ok_or("the save names no player; give a TAG")?;
    let mut state = State::from_save(&save);
    state.attach_world(&w, &save);
    let capital = state.countries.get(&tag).and_then(|c| c.capital).ok_or(format!("{tag} has no capital in the save"))?;
    let (cal, rules) = (Calendar::from_engine_rules(&engine_rules)?, EffectRules::from_engine_rules(&engine_rules)?);
    state.seed = rules.seed;

    let blocks = all_effect_blocks(&install_fs)?;
    let (mut clean, mut applied, mut skipped) = (0usize, 0usize, 0usize);
    let mut missing: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (_, block, province_event) in &blocks {
        let scope = if *province_event { Scope::Province(capital) } else { Scope::Country(tag.clone()) };
        let mut report = Report::default();
        state.this = Some(scope.clone());
        // Coverage has no sender: FROM is the same scope.
        state.from = Some(scope.clone());
        run_effects(&mut state, &cal, &rules, block, &scope, &mut report);
        applied += report.applied.len();
        skipped += report.unsupported.len();
        if report.unsupported.is_empty() {
            clean += 1;
        }
        for (k, n) in report.unsupported {
            // "key (in x scope)" -> key; keep "… with limit on cond" as its own line.
            let name = k.split(" (in ").next().unwrap_or(&k).to_string();
            let e = missing.entry(name).or_default();
            e.0 += 1;
            e.1 += n;
        }
    }
    // Conditions first, on the untouched state.
    let conditions = all_condition_blocks(&install_fs)?;
    let mut by_kind: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut unknown: BTreeMap<String, usize> = BTreeMap::new();
    {
        let mut fresh = State::from_save(&save);
        fresh.attach_world(&w, &save);
        for (kind, block, province_event) in &conditions {
            let scope = if *province_event { Scope::Province(capital) } else { Scope::Country(tag.clone()) };
            fresh.this = Some(scope.clone());
            let missing = wiktoria_simulation::script::unknown_conditions(&fresh, block, &scope);
            let e = by_kind.entry(kind).or_default();
            e.0 += 1;
            e.1 += missing.is_empty() as usize;
            for name in missing {
                *unknown.entry(name).or_default() += 1;
            }
        }
    }
    println!("condition blocks checked for {tag}:");
    for (kind, (all, clean)) in &by_kind {
        println!("  {kind:<20} {clean:>5} of {all:>5} fully understood ({:.0}%)", *clean as f64 / (*all).max(1) as f64 * 100.0);
    }
    let mut list: Vec<_> = unknown.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    println!("conditions not understood yet (blocks affected):");
    let shown = if args.all_warnings { usize::MAX } else { 45 };
    for (name, n) in list.iter().take(shown) {
        println!("  {n:>5}  {name}");
    }
    if list.len() > shown {
        println!("  ... {} more (use --all-warnings)", list.len() - shown);
    }
    println!();
    println!("{} effect blocks (event options, immediate blocks, decisions) run for {tag}", blocks.len());
    println!("fully understood: {clean} ({:.0}%)", clean as f64 / blocks.len().max(1) as f64 * 100.0);
    println!("distinct entries applied per block, summed: {applied}; skipped: {skipped}");
    let mut list: Vec<_> = missing.into_iter().collect();
    list.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.0.cmp(&b.0)));
    println!("not understood yet (blocks affected, times met):");
    for (name, (blocks, times)) in list.iter().take(if args.all_warnings { usize::MAX } else { 45 }) {
        println!("  {blocks:>5} {times:>6}  {name}");
    }
    if list.len() > 45 && !args.all_warnings {
        println!("  ... {} more (use --all-warnings)", list.len() - 45);
    }
    Ok(())
}

/// `wiktoria-cli decisions --save <file.v2> [<TAG>]`: the decisions the country would see in its
/// decision list (`potential` holds) and whether it could take each one (`allow` holds).
/// A decision with a condition the engine does not know is listed apart, never guessed.
pub fn decisions(args: &Args) -> Result<(), String> {
    let save_path = args.save.as_ref().ok_or("usage: decisions --save <file.v2> [<TAG>]")?;
    let w = load_world(args)?;
    let root = root();
    let engine_rules = config::load_engine_rules(&root)?;
    let install = config::install_dir(&root, args.install.clone())?;
    let mod_files: Vec<_> = args.mod_files.iter().map(std::path::PathBuf::from).collect();
    let install_fs = wiktoria_data::install_fs::InstallFs::with_mods(&install, &mod_files)?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let bytes = std::fs::read(save_path).map_err(|e| format!("cannot read {save_path}: {e}"))?;
    let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
    let tag = args.positional.first().cloned().or(save.player.clone()).ok_or("the save names no player; give a TAG")?;
    let _ = engine_rules;
    let mut state = State::from_save(&save);
    state.attach_world(&w, &save);
    let scope = Scope::Country(tag.clone());
    state.this = Some(scope.clone());

    let files = install_fs.dir_files("decisions", "txt")?;
    let (mut total, mut hidden) = (0usize, 0usize);
    let (mut can, mut cannot, mut unsure) = (Vec::new(), Vec::new(), Vec::new());
    for path in files {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Ok(doc) = script::parse_bytes(&bytes) else { continue };
        for (_, top, _) in doc.root.entries() {
            let Some(top) = top.as_block() else { continue };
            for (name, d, _) in top.entries() {
                let Some(d) = d.as_block() else { continue };
                total += 1;
                let check = |word: &str| match d.get(word).and_then(Value::as_block) {
                    Some(b) => wiktoria_simulation::script::conditions_hold(&state, b, &scope),
                    None => Ok(true),
                };
                match (check("potential"), check("allow")) {
                    (Ok(false), _) => hidden += 1,
                    (Ok(true), Ok(true)) => can.push(name.to_string()),
                    (Ok(true), Ok(false)) => cannot.push(name.to_string()),
                    (Err(u), _) | (_, Err(u)) => unsure.push(format!("{name} (needs {u})")),
                }
            }
        }
    }
    println!("decisions for {tag} on {}: {total} defined, {hidden} not shown", state.date);
    println!("shown and can be taken ({}): {}", can.len(), can.join(", "));
    println!("shown but not allowed yet ({}): {}", cannot.len(), cannot.join(", "));
    println!("cannot tell ({}): {}", unsure.len(), unsure.join(", "));
    Ok(())
}
