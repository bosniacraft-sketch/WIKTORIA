//! `wiktoria-cli scenario <file> [--days N] [--strict] [--save <file.v2>]`: a test scenario run
//! headless, with a PASS / FAIL line per check and exit code 1 on a FAIL (docs/SCENARIOS.md).
//! `wiktoria-cli scenario --list` lists the scenarios in `scenarios/`. `wiktoria-cli serve --scenario <file>`
//! starts the browser game with the scenario applied (serve.rs calls [`load_for_serve`]).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use wiktoria_simulation::game::{DayLog, Game};
use wiktoria_simulation::scenario::{self, Run, Scenario};

use crate::{config, load_world, root, Args};

/// The scenario file: as given, or by name in the `scenarios/` folder (`.txt` may be left out).
pub fn find_file(name: &str) -> Result<PathBuf, String> {
    let dir = root().join("scenarios");
    let tries = [PathBuf::from(name), dir.join(name), dir.join(format!("{name}.txt"))];
    tries.iter().find(|p| p.is_file()).cloned().ok_or_else(|| format!("no scenario file {name} (looked at {})", tries.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")))
}

pub fn read(path: &Path) -> Result<Scenario, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    scenario::parse(&wiktoria_data::script::decode_windows1252(&bytes)).map_err(|e| format!("{}: {e}", path.display()))
}

/// The base save: `--save` if given, else the file's `base_save` looked up next to the scenario
/// file, in the game folder (WIKTORIA_ROOT or the source tree) and in the install.
pub fn base_save(args: &Args, sc: &Scenario, file: &Path) -> Result<PathBuf, String> {
    if let Some(s) = &args.save {
        return Ok(PathBuf::from(s));
    }
    let name = sc.base_save.as_deref().ok_or("the scenario names no base_save; give one with --save <file.v2>")?;
    if name == "history" {
        return Err("base_save = history (the install's own 1836 start without a save) is not supported yet: the game is built from a save; give one with --save".into());
    }
    let mut tries = vec![PathBuf::from(name)];
    if let Some(dir) = file.parent() {
        tries.push(dir.join(name));
    }
    tries.push(root().join(name));
    if let Ok(install) = config::install_dir(&root(), args.install.clone()) {
        tries.push(install.join(name));
    }
    tries.iter().find(|p| p.is_file()).cloned().ok_or_else(|| format!("base save {name} not found (looked at {}); put it there or give --save <file.v2>", tries.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")))
}

/// The game of a save, built as `run` builds it.
fn build(args: &Args, save_path: &Path) -> Result<Game, String> {
    let w = load_world(args)?;
    let engine_rules = config::load_engine_rules(&root())?;
    let install = config::install_dir(&root(), args.install.clone())?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let bytes = std::fs::read(save_path).map_err(|e| format!("cannot read {}: {e}", save_path.display()))?;
    let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
    Game::new(w, &engine_rules, &install, &save)
}

fn list() -> Result<(), String> {
    let dir = root().join("scenarios");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "txt") && p.file_name().is_some_and(|n| n != "README.txt")).collect();
    files.sort();
    for f in &files {
        let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        match read(f) {
            Ok(sc) => {
                let seat = if sc.observe { "observed".to_string() } else { format!("seat {}", sc.seat.as_deref().unwrap_or("none")) };
                println!("{stem:<24} {} ({seat}, {} days, {} checks)", sc.name, sc.days, sc.checks.len());
                if !sc.description.is_empty() {
                    println!("{:<24} {}", "", sc.description.lines().next().unwrap_or_default());
                }
                for b in &sc.known_bugs {
                    println!("{:<24} known bug: {b}", "");
                }
            }
            Err(e) => println!("{stem:<24} cannot be read: {e}"),
        }
    }
    println!("{} scenarios in {}", files.len(), dir.display());
    Ok(())
}

pub fn scenario_cmd(args: &Args) -> Result<(), String> {
    let usage = "usage: scenario <file> [--days N] [--strict] [--save <file.v2>] | scenario --list";
    if args.positional.iter().any(|a| a == "--list") {
        return list();
    }
    let (mut file, mut days, mut strict) = (None, None, false);
    let mut it = args.positional.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--days" => days = Some(it.next().and_then(|n| n.parse::<u32>().ok()).ok_or("--days needs a number")?),
            "--strict" => strict = true,
            _ if file.is_none() => file = Some(a.clone()),
            other => return Err(format!("unexpected argument {other}; {usage}")),
        }
    }
    let path = find_file(&file.ok_or(usage)?)?;
    let sc = read(&path)?;
    let save_path = base_save(args, &sc, &path)?;
    let days = days.unwrap_or(sc.days);
    println!("scenario {} ({})", sc.name, path.display());
    if !sc.description.is_empty() {
        println!("  {}", sc.description);
    }
    for b in &sc.known_bugs {
        println!("  known bug: {b}");
    }
    let t = std::time::Instant::now();
    let mut game = build(args, &save_path)?;
    wiktoria_simulation::player::observe_all(&mut game.state, &mut game.core);
    eprintln!("game built in {:.2?} from {}", t.elapsed(), save_path.display());
    for line in scenario::prepare(&mut game, &sc)? {
        println!("  setup: {line}");
    }
    println!("{}: running {days} days headless", game.state.date);
    let t = std::time::Instant::now();
    let r = scenario::run(&mut game, &sc, days, |g: &Game, log: &DayLog| {
        let c = &log.core;
        if g.state.date.day == 1 {
            println!("{}: wars {}, battles now {}, sieges now {}, peaces {:?}, declared {:?}", g.state.date, g.core.mil.wars.iter().filter(|w| w.active).count(), g.core.mil.combats.iter().filter(|x| x.kind == wiktoria_simulation::military::battle::CombatKind::Land).count(), g.core.mil.sieges().count(), c.peaces, c.wars_declared);
        }
    });
    eprintln!("ran {days} days in {:.1}s", t.elapsed().as_secs_f64());
    report(&r, strict)
}

fn report(r: &Run, strict: bool) -> Result<(), String> {
    println!();
    for v in &r.verdicts {
        println!("{}", v.line());
    }
    let pass = r.verdicts.iter().filter(|v| v.pass).count();
    let known = r.verdicts.iter().filter(|v| !v.pass && v.known_bug.is_some()).count();
    let failed = r.failed(strict);
    println!("verdict: {pass} PASS, {} FAIL, {known} KNOWN (fails a known bug explains){}", r.verdicts.len() - pass - known, if strict { "; --strict counts KNOWN as FAIL" } else { "" });
    if failed > 0 {
        return Err(format!("{failed} checks failed"));
    }
    Ok(())
}

/// For `serve --scenario <file>`: the scenario and the save to start from.
pub fn load_for_serve(args: &Args, name: &str) -> Result<(Scenario, PathBuf), String> {
    let path = find_file(name)?;
    let sc = read(&path)?;
    let save = base_save(args, &sc, &path)?;
    Ok((sc, save))
}

/// The browser game's day: the checks that fell due, as news lines for the seat.
pub fn serve_day(live: &mut Option<(Scenario, Run)>, g: &Game, log: &DayLog) -> Vec<String> {
    let Some((sc, run)) = live.as_mut() else { return Vec::new() };
    let before = run.verdicts.len();
    if run.due(sc, g.state.date) {
        run.day(sc, &log.core, &g.state, &g.core, &scenario::game_probe(g, Some(log), sc));
    } else {
        run.day(sc, &log.core, &g.state, &g.core, &scenario::Probe::default());
    }
    // The checks for the end are evaluated on the scenario's last day.
    if run.days == sc.days {
        run.finish(sc, &g.state, &g.core, &scenario::game_probe(g, Some(log), sc));
    }
    run.verdicts[before..].iter().map(|v| format!("Scenario check: {}", v.line())).collect()
}
