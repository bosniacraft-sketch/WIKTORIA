//! wiktoria-cli command-line tools.
//!
//!   wiktoria-cli [summary]                 load the start-date world and print a summary
//!   wiktoria-cli province <id>             everything loaded for one province (pops, buildings, ...)
//!   wiktoria-cli country <TAG>             a country's start-date history (reforms, techs, ...)
//!   wiktoria-cli goods                     list trade goods
//!   wiktoria-cli diff-save <file.v2>       compare the loaded start world with an original save
//!   wiktoria-cli diff-days <folder>        day-by-day pop changes across a folder of daily saves
//!   wiktoria-cli check-prices <folder>     replay daily saves through the port's price rule and compare
//!   wiktoria-cli check-day --save <day N.v2> <day N+1.v2>   move the port one day on and compare the pops due that day
//!   wiktoria-cli check-battle --save <day N.v2> <day N+1.v2>   fight each land battle one day in the port and compare with the next save
//!   wiktoria-cli check-battle --dir <folder>               the same over a folder of daily saves, a line a day
//!   wiktoria-cli check-event <id> --save <before.v2> <after.v2> [<TAG>]   fire an event and compare with the game's own save
//!   wiktoria-cli check-colonies <save.v2>   colonies and protectorates per great power, recounted from the save's own text
//!   wiktoria-cli check-scores --save <file.v2> [--tag TAG]   industrial score, colonial points and colony-to-state costs of a country, with their parts
//!   wiktoria-cli check-occupation --dir <folder of dated saves> [--every N] [--ai-off LIST]   run the port from the first save and compare each war with the later saves
//!   wiktoria-cli check-military <save.v2>   armies, regiments, ships, leaders and wars per great power, from the save's own text
//!   wiktoria-cli why --save <file.v2> <pop id>...           the port's reasoning for a pop, laid out like `debug promotiondesc`
//!   wiktoria-cli run --save <file.v2> [--days N] [--ai-off LIST]   every system in one combined day, a summary a month
//!   wiktoria-cli serve --save <file.v2> [--port 8036]      the combined game behind a web page on 127.0.0.1 (apps/web/API.md)
//!   wiktoria-cli serve --scenario <file> [--port 8036]     the same with a test scenario applied and its seat played
//!   wiktoria-cli scenario <file> [--days N] [--strict]     run a test scenario headless: PASS/FAIL per check (docs/SCENARIOS.md)
//!   wiktoria-cli scenario --list                           the scenarios in scenarios/
//!   wiktoria-cli run-events --save <file.v2> [<days>]       let only events and decisions happen for a year
//!   wiktoria-cli decisions --save <file.v2> [<TAG>]         which decisions the country sees and can take
//!   wiktoria-cli script-coverage --save <file.v2> [<TAG>]   how much of the install's events and decisions the effect engine understands
//!   wiktoria-cli fire-event <id> --save <file.v2> [<TAG> [<option>]]   apply an event option to a save's state
//!   wiktoria-cli export <save.v2 or folder> <output folder>   save -> four small CSV state tables per date
//!   wiktoria-cli score <reference folder> [<candidate folder>]   scoreboard: closeness of a run to the real game
//!   wiktoria-cli render OUT.bmp [--mode political|provinces|population|culture|religion|states] [--width N]
//!
//! Every command accepts --install DIR (otherwise vic2_path.txt / VIC2_DIR is used) and
//! --no-setup (skip the game-setup pass and show the history pops exactly as written).

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use wiktoria_data::{config, goods, script, world::World};
use wiktoria_render::{describe, fmt_int, render, Camera, MapMode, Palette, RenderOptions};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
// One command per line, so branches that each add a command merge without conflicts.
const COMMANDS: &[&str] = &[
    "check-military",
    "run",
    "serve",
    "scenario",
    "run-military",
    "why",
    "check-production",
    "check-day",
    "check-battle",
    "check-event",
    "run-events",
    "decisions",
    "script-coverage",
    "fire-event",
    "export",
    "score",
    "summary",
    "province",
    "country",
    "goods",
    "render",
    "diff-save",
    "diff-days",
    "check-prices",
    "check-unrest",
    "check-money",
    "check-colonies",
    "check-diplomacy",
    "check-scores",
    "check-occupation",
];
mod check_colonies;
mod check_diplomacy;
mod check_scores;
mod check_occupation;
mod check_military;
mod check_unrest;
mod check_production;
mod why;
mod export;
mod fire_event;
mod check_battle;
mod check_day;
mod check_money;
mod check_event;
mod run_events;
mod run_game;
mod run_military;
mod scenario;
mod score;
mod serve;

pub(crate) struct Args {
    cmd: String,
    positional: Vec<String>,
    install: Option<String>,
    mode: MapMode,
    width: usize,
    all_warnings: bool,
    no_setup: bool,
    fix_rgo_size: bool,
    slots: Option<String>,
    save: Option<String>,
    mods: Option<String>,
    secondary: Vec<String>,
    mod_files: Vec<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { cmd: "summary".into(), positional: Vec::new(), install: None, mode: MapMode::Political, width: 2808, all_warnings: false, no_setup: false, fix_rgo_size: false, slots: None, save: None, mods: None, secondary: Vec::new(), mod_files: Vec::new() };
    let mut it = std::env::args().skip(1);
    let mut first = true;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--install" => a.install = Some(it.next().ok_or("--install needs a folder")?),
            "--mod" => a.mod_files.push(it.next().ok_or("--mod needs a .mod file")?),
            "--mode" => {
                let m = it.next().unwrap_or_default();
                a.mode = MapMode::from_label(&m).ok_or(format!(
                    "unknown --mode '{m}' (use {})",
                    MapMode::ALL.map(|m| m.label()).join(", ")
                ))?;
            }
            "--all-warnings" => a.all_warnings = true,
            "--no-setup" => a.no_setup = true,
            "--fix-rgo-size" => a.fix_rgo_size = true,
            "--slots" => a.slots = Some(it.next().ok_or("--slots needs a file")?),
            "--mods" => a.mods = Some(it.next().ok_or("--mods needs a file")?),
            "--save" => a.save = Some(it.next().ok_or("--save needs a file")?),
            "--secondary" => a.secondary = it.next().ok_or("--secondary needs TAG,TAG,...")?.split(',').map(|t| t.trim().to_string()).collect(),
            "--width" => a.width = it.next().and_then(|s| s.parse().ok()).ok_or("--width needs a number")?,
            c if first && COMMANDS.contains(&c) => a.cmd = arg,
            _ => a.positional.push(arg),
        }
        first = false;
    }
    Ok(a)
}

/// Repo root: WIKTORIA_ROOT if set (for running a copied binary), else the source tree.
pub(crate) fn root() -> std::path::PathBuf {
    std::env::var("WIKTORIA_ROOT").map(Into::into).unwrap_or_else(|_| Path::new(ROOT).to_path_buf())
}

pub(crate) fn load_world(args: &Args) -> Result<World, String> {
    let root = &root();
    let engine_rules = config::load_engine_rules(root)?;
    let install = config::install_dir(root, args.install.clone())?;
    let t = Instant::now();
    let mod_files: Vec<_> = args.mod_files.iter().map(std::path::PathBuf::from).collect();
    let mut world = World::load_with(wiktoria_data::install_fs::InstallFs::with_mods(&install, &mod_files)?, &engine_rules)?;
    eprintln!("loaded {} in {:.2?} (engine_rules {})", install.display(), t.elapsed(), &engine_rules.sha256[..12]);
    if args.no_setup {
        eprintln!("setup pass skipped (--no-setup)");
    } else {
        let r = wiktoria_data::setup::apply(&mut world, wiktoria_data::setup::SetupSteps::from_engine_rules(&engine_rules)?);
        eprintln!(
            "setup: {} -> {} pops; {} type changes, {} slaves converted ({} kept), {} moved to state capitals, {} merged",
            r.pops_before, r.pops_after, r.type_changes, r.slaves_converted, r.slaves_kept, r.moved_to_capital, r.merged
        );
        world.warnings.extend(r.warnings);
        wiktoria_data::setup::starting_literacy(&mut world, &wiktoria_data::setup::LiteracyRules::from_engine_rules(&engine_rules)?);
    }
    Ok(world)
}

fn top<K: Ord + Clone>(m: &BTreeMap<K, f64>, n: usize) -> Vec<(K, f64)> {
    let mut v: Vec<_> = m.iter().map(|(k, v)| (k.clone(), *v)).collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    v.truncate(n);
    v
}

fn summary(args: &Args) -> Result<(), String> {
    let w = load_world(args)?;
    let provs: Vec<_> = w.provinces.iter().flatten().collect();
    let sea = provs.iter().filter(|p| p.is_sea).count();
    let owned = provs.iter().filter(|p| p.owner.is_some()).count();
    let mut by_owner: BTreeMap<u16, (usize, f64)> = BTreeMap::new();
    let (mut world_pop, mut pop_count) = (0.0, 0);
    let mut by_type: BTreeMap<String, f64> = BTreeMap::new();
    let mut by_culture: BTreeMap<String, f64> = BTreeMap::new();
    let mut by_religion: BTreeMap<String, f64> = BTreeMap::new();
    for p in &provs {
        let pop = p.population();
        world_pop += pop;
        pop_count += p.pops.len();
        if let Some(o) = p.owner {
            let e = by_owner.entry(o).or_default();
            e.0 += 1;
            e.1 += pop;
        }
        for x in &p.pops {
            *by_type.entry(w.defs.pop_types[x.pop_type as usize].name.clone()).or_default() += x.size;
            *by_culture.entry(w.defs.cultures.cultures[x.culture as usize].name.clone()).or_default() += x.size;
            *by_religion.entry(w.defs.religions.religions[x.religion as usize].name.clone()).or_default() += x.size;
        }
    }
    let factories: usize = provs.iter().map(|p| p.state_buildings.len()).sum();
    println!("start date      {} (pops from history/pops/{})", w.start_date, w.pop_folder);
    println!("map             {} x {}, {} state regions", w.map.width, w.map.height, w.regions.len());
    println!("provinces       {} defined, {sea} sea, {} land ({owned} owned)", provs.len(), provs.len() - sea);
    println!("countries       {} in countries.txt, {} own land", w.countries.len(), by_owner.len());
    println!("population      {} in {pop_count} pops", fmt_int(world_pop));
    println!("definitions     {} cultures, {} religions, {} pop types, {} reform groups, {} techs, {} building types",
        w.defs.cultures.cultures.len(), w.defs.religions.religions.len(), w.defs.pop_types.len(),
        w.defs.issues.len(), w.defs.technologies.len(), w.defs.buildings.len());
    println!("buildings       {factories} factories, {} forts, {} naval bases, {} railroads (province levels > 0)",
        provs.iter().filter(|p| p.fort > 0).count(), provs.iter().filter(|p| p.naval_base > 0).count(),
        provs.iter().filter(|p| p.railroad > 0).count());

    let mut owners: Vec<_> = by_owner.into_iter().collect();
    owners.sort_by(|a, b| b.1.1.total_cmp(&a.1.1).then(a.0.cmp(&b.0)));
    println!("\nlargest countries by population:");
    for (o, (n, pop)) in owners.iter().take(10) {
        let c = w.country(*o);
        println!("  {}  {:>13}  {n:>4} provinces  {}", c.country.tag, fmt_int(*pop), c.name);
    }
    let show = |title: &str, m: &BTreeMap<String, f64>| {
        let items: Vec<String> = top(m, 6).iter().map(|(k, v)| format!("{k} {}", fmt_int(*v))).collect();
        println!("{title:<16}{}", items.join(", "));
    };
    println!();
    show("pop types", &by_type);
    show("cultures", &by_culture);
    show("religions", &by_religion);
    print_warnings(&w.warnings, args.all_warnings);
    Ok(())
}

fn province_cmd(args: &Args) -> Result<(), String> {
    let id: u16 = args.positional.first().and_then(|s| s.parse().ok()).ok_or("usage: province <id>")?;
    let w = load_world(args)?;
    let p = w.province(id).ok_or(format!("province {id} is not defined"))?;
    let tag = |i: u16| w.country(i).country.tag.clone();
    println!("{}", describe(&w, id));
    println!("  state       {}", p.region.map_or("-".into(), |r| w.regions[r as usize].name.clone()));
    println!("  controller  {}", p.controller.map_or("-".into(), tag));
    println!("  cores       {}", p.cores.iter().map(|c| tag(*c)).collect::<Vec<_>>().join(" "));
    println!("  life rating {}", p.life_rating.map_or("-".into(), |v| v.to_string()));
    println!("  colonial    {}", p.colonial.map_or("-".into(), |v| v.to_string()));
    println!("  fort {}  naval base {}  railroad {}", p.fort, p.naval_base, p.railroad);
    for b in &p.state_buildings {
        println!("  factory     {} level {}{}", b.building, b.level, if b.upgrade { " (upgrade)" } else { "" });
    }
    println!("  pops ({}, total {}):", p.pops.len(), fmt_int(p.population()));
    for x in &p.pops {
        println!(
            "    {:<12} {:<16} {:<12} {:>9}{}",
            w.defs.pop_types[x.pop_type as usize].name,
            w.defs.cultures.cultures[x.culture as usize].name,
            w.defs.religions.religions[x.religion as usize].name,
            fmt_int(x.size),
            if x.militancy > 0.0 { format!("  militancy {}", x.militancy) } else { String::new() }
        );
    }
    Ok(())
}

fn country_cmd(args: &Args) -> Result<(), String> {
    let tag = args.positional.first().ok_or("usage: country <TAG>")?.to_uppercase();
    let w = load_world(args)?;
    let &idx = w.tag_index.get(&tag).ok_or(format!("unknown tag {tag}"))?;
    let c = w.country(idx);
    let h = &c.history;
    let opt = |v: &Option<String>| v.clone().unwrap_or("-".into());
    let num = |v: Option<f64>| v.map_or("-".into(), |x| x.to_string());
    let provs: Vec<_> = w.provinces.iter().flatten().filter(|p| p.owner == Some(idx)).collect();
    println!("{} [{}]{}", c.name, tag, if c.country.dynamic { " (dynamic tag)" } else { "" });
    println!("  capital        {}", h.capital.map_or("-".into(), |id| describe(&w, id)));
    println!("  provinces      {}, population {}", provs.len(), fmt_int(provs.iter().map(|p| p.population()).sum()));
    println!("  government     {}   ruling party {}", opt(&h.government), opt(&h.ruling_party));
    println!("  culture        {} + {}", opt(&h.primary_culture), h.cultures.join(", "));
    println!("  religion       {}", opt(&h.religion));
    println!("  civilized      {}", h.civilized.map_or("-".into(), |b| b.to_string()));
    println!("  literacy       {}   plurality {}   prestige {}", num(h.literacy), num(h.plurality), num(h.prestige));
    println!("  national value {}", opt(&h.national_value));
    if !h.upper_house.is_empty() {
        let uh: Vec<String> = h.upper_house.iter().map(|(k, v)| format!("{k} {v}")).collect();
        println!("  upper house    {}", uh.join(", "));
    }
    println!("  reforms:");
    for g in &w.defs.issues {
        if let Some(v) = h.reforms.get(&g.name) {
            println!("    {:<28} {v}", g.name);
        }
    }
    println!("  technologies ({}): {}", h.technologies.len(), h.technologies.iter().cloned().collect::<Vec<_>>().join(", "));
    if !h.inventions.is_empty() {
        println!("  inventions ({}): {}", h.inventions.len(), h.inventions.iter().cloned().collect::<Vec<_>>().join(", "));
    }
    if !h.decisions.is_empty() {
        println!("  decisions      {}", h.decisions.join(", "));
    }
    Ok(())
}

fn diff_save(args: &Args) -> Result<(), String> {
    use std::collections::{BTreeMap, HashSet};
    let path = args.positional.first().ok_or("usage: diff-save <file.v2>")?;
    let w = load_world(args)?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let t = Instant::now();
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
    eprintln!("read save in {:.2?}: date {}, player {}, {} provinces, {} countries",
        t.elapsed(), save.date, save.player.as_deref().unwrap_or("-"), save.provinces.len(), save.countries.len());
    let r = wiktoria_data::compare::compare(&w, &save);
    let ex = |n: usize| if args.all_warnings { usize::MAX } else { n };

    println!("compared {} provinces (missing in save: {:?}, missing in world: {:?})",
        r.provinces_compared, r.missing_in_save, r.missing_in_world);
    println!("pops    ours {} pops / {}    save {} pops / {}   (diff {:+})",
        r.pops_ours.0, fmt_int(r.pops_ours.1), r.pops_save.0, fmt_int(r.pops_save.1), r.pops_save.1 - r.pops_ours.1);
    for (title, list) in [("owner", &r.owner), ("controller", &r.controller), ("cores", &r.cores)] {
        println!("\n{title} differences: {}", list.len());
        for (id, a, b) in list.iter().take(ex(10)) {
            println!("  #{id:<5} ours [{a}]  save [{b}]");
        }
    }
    println!("\npop type swaps (same culture/religion/size, different type):");
    for (swap, (n, goods)) in &r.type_swaps {
        let mut g: Vec<_> = goods.iter().collect();
        g.sort_by(|a, b| b.1.cmp(a.1));
        let g: Vec<String> = g.iter().take(12).map(|(k, v)| format!("{k} {v}")).collect();
        println!("  {swap:<24} {n:>5} x   by trade goods: {}", g.join(", "));
    }
    println!("\nsize changes per pop type (groups present in both):");
    let types: std::collections::BTreeSet<&String> = r.size_changes.keys().chain(r.size_equal.keys()).collect();
    for ty in types {
        let eq = r.size_equal.get(ty).copied().unwrap_or(0);
        let ch = r.size_changes.get(ty).map(|v| v.as_slice()).unwrap_or(&[]);
        if ch.is_empty() {
            println!("  {ty:<12} {eq:>5} equal, 0 changed");
            continue;
        }
        let mut ratios: Vec<f64> = ch.iter().map(|(_, a, b)| b / a).collect();
        ratios.sort_by(|a, b| a.total_cmp(b));
        let mut common: BTreeMap<String, usize> = BTreeMap::new();
        for x in &ratios {
            *common.entry(format!("{x:.3}")).or_default() += 1;
        }
        let mut common: Vec<_> = common.into_iter().collect();
        common.sort_by_key(|b| std::cmp::Reverse(b.1));
        let common: Vec<String> = common.iter().take(5).map(|(k, v)| format!("x{k} ({v})")).collect();
        let grow: f64 = ch.iter().map(|(_, a, b)| b - a).sum();
        let total = format!("{}{}", if grow < 0.0 { "-" } else { "+" }, fmt_int(grow.abs()));
        println!("  {ty:<12} {eq:>5} equal, {:>5} changed, total {total}, ratio min {:.3} median {:.3} max {:.3}; common: {}",
            ch.len(), ratios[0], ratios[ratios.len() / 2], ratios[ratios.len() - 1], common.join(", "));
        for (id, a, b) in ch.iter().take(ex(4)) {
            println!("      #{id:<5} {} -> {}", fmt_int(*a), fmt_int(*b));
        }
    }
    for (title, list) in [("only in ours (unexplained)", &r.only_ours), ("only in save (unexplained)", &r.only_save)] {
        println!("\n{title}: {}", list.len());
        let mut by_type: BTreeMap<&str, (usize, f64)> = BTreeMap::new();
        for (_, k, v) in list.iter() {
            let e = by_type.entry(&k.0).or_default();
            e.0 += 1;
            e.1 += v;
        }
        for (ty, (n, total)) in &by_type {
            println!("  {ty:<12} {n:>5} groups, {}", fmt_int(*total));
        }
        for (id, k, v) in list.iter().take(ex(6)) {
            println!("      #{id:<5} {} {} {} {}", k.0, k.1, k.2, fmt_int(*v));
        }
    }
    if !save.warnings.is_empty() {
        print_warnings(&save.warnings, args.all_warnings);
    }
    Ok(())
}

fn diff_days(args: &Args) -> Result<(), String> {
    use std::collections::{BTreeMap, HashSet};
    use wiktoria_data::popdiff::{self, DayBasis, Field, Snapshot};
    let dir = args.positional.first().ok_or("usage: diff-days <folder with daily .v2 saves>")?;
    let w = load_world(args)?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read {dir}: {e}"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("v2")))
        .collect();
    files.sort();
    let t = Instant::now();
    let mut series = Vec::new();
    let mut warnings = Vec::new();
    for f in &files {
        let bytes = std::fs::read(f).map_err(|e| format!("cannot read {}: {e}", f.display()))?;
        let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures).map_err(|e| format!("{}: {e}", f.display()))?;
        warnings.extend(save.warnings.iter().take(5).map(|x| format!("{}: {x}", f.display())));
        series.push(Snapshot::from_save(&save));
    }
    series.sort_by_key(|s| s.date);
    eprintln!("read {} saves in {:.2?}", series.len(), t.elapsed());
    if series.len() < 2 {
        return Err("need at least two saves".into());
    }

    let fields = Field::ALL.map(|f| format!("{:>8}", f.label())).join(" ");
    println!("{:<11} {:>6} {:>12} {:>5} {:>5} {:>5} {:>6} | changed: {fields}", "date", "pops", "population", "+new", "-gone", "grew", "shrank");
    println!("{:<11} {:>6} {:>12}", series[0].date.to_string(), series[0].pops.len(), fmt_int(series[0].population()));
    let mut sources: BTreeMap<String, (usize, Vec<f64>)> = BTreeMap::new();
    for pair in series.windows(2) {
        let d = popdiff::diff(&pair[0], &pair[1]);
        let ch = Field::ALL.map(|f| format!("{:>8}", d.changed.get(&f).copied().unwrap_or(0))).join(" ");
        println!("{:<11} {:>6} {:>12} {:>5} {:>5} {:>5} {:>6} | changed: {ch}",
            pair[1].date.to_string(), d.pops, fmt_int(d.population), d.new, d.gone, d.grew, d.shrank);
        let first_of_month = pair[1].date.day == 1;
        for (src, size) in popdiff::new_pop_sources(&pair[0], &pair[1]) {
            let key = format!("{}{}", src.label(), if first_of_month { "  [1st of month]" } else { "" });
            let e = sources.entry(key).or_default();
            e.0 += 1;
            e.1.push(size);
        }
    }

    println!("
update schedule: share of each field's changes on the best (day - pop id) mod m (1st of month left out)");
    let moduli: Vec<u64> = (28..=32).collect();
    for f in Field::ALL {
        let show = |basis| match popdiff::fit_schedule(&series, f, basis, &moduli, Some(1)) {
            Some(fit) => format!("mod {} = {:<2} {:>5.1}% of {:>7}", fit.modulus, fit.residue, fit.share * 100.0, fit.changes),
            None => "no changes".into(),
        };
        println!("  {:<9} day of month: {:<28} running day: {}", f.label(), show(DayBasis::DayOfMonth), show(DayBasis::Running));
    }

    println!("
new pops by likely source:");
    let mut rows: Vec<_> = sources.into_iter().collect();
    rows.sort_by_key(|b| std::cmp::Reverse(b.1.0));
    for (label, (n, mut sizes)) in rows {
        sizes.sort_by(|a, b| a.total_cmp(b));
        println!("  {n:>6}  {label:<55} median size {}", fmt_int(sizes[sizes.len() / 2]));
    }
    if !warnings.is_empty() {
        print_warnings(&warnings, args.all_warnings);
    }
    Ok(())
}

/// One save's world market: good -> (price, supply_pool, real_demand, price_change), raw fixed point.
type MarketRow = (Option<wiktoria_simulation::Fixed>, wiktoria_simulation::Fixed, wiktoria_simulation::Fixed, Option<wiktoria_simulation::Fixed>);

fn read_market(path: &Path) -> Result<(wiktoria_data::date::Date, BTreeMap<String, MarketRow>), String> {
    use wiktoria_simulation::Fixed;
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let doc = script::parse_bytes(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let date = doc.root.get("date").and_then(script::Value::as_str).and_then(wiktoria_data::date::Date::parse)
        .ok_or(format!("{}: no date", path.display()))?;
    let wm = doc.root.get("worldmarket").and_then(script::Value::as_block).ok_or(format!("{}: no worldmarket", path.display()))?;
    let pool = |name: &str| -> BTreeMap<String, Fixed> {
        wm.get(name)
            .and_then(script::Value::as_block)
            .map(|b| b.entries().filter_map(|(k, v, _)| Some((k.to_string(), Fixed::from_save_text(v.as_str()?)?))).collect())
            .unwrap_or_default()
    };
    let (price, supply, demand, change) = (pool("price_pool"), pool("supply_pool"), pool("real_demand"), pool("price_change"));
    let mut out = BTreeMap::new();
    for good in price.keys().chain(supply.keys()).chain(demand.keys()) {
        out.entry(good.clone()).or_insert_with(|| {
            (price.get(good).copied(), supply.get(good).copied().unwrap_or_default(), demand.get(good).copied().unwrap_or_default(), change.get(good).copied())
        });
    }
    Ok((date, out))
}

fn check_prices(args: &Args) -> Result<(), String> {
    use wiktoria_simulation::{GoodMarket, PriceRules};
    let dir = args.positional.first().ok_or("usage: check-prices <folder with daily .v2 saves>")?;
    let root = &root();
    let engine_rules = config::load_engine_rules(root)?;
    let rules = PriceRules::from_engine_rules(&engine_rules)?;
    let calendar = wiktoria_simulation::Calendar::from_engine_rules(&engine_rules)?;
    let install = config::install_dir(root, args.install.clone())?;
    let gpath = install.join(engine_rules.require_str("FLD_GOODS_FILE")?);
    let gdoc = script::parse_bytes(&std::fs::read(&gpath).map_err(|e| format!("cannot read {}: {e}", gpath.display()))?)
        .map_err(|e| format!("{}: {e}", gpath.display()))?;
    let goods = goods::load_goods(&gdoc)?.goods;
    let fixed_flag = |g: &goods::Good| match rules.fixed_good_flag.as_str() {
        "money" => Ok(g.money),
        "not_tradeable" => Ok(!g.tradeable),
        other => Err(format!("FLD_PRICE_FIXED_GOOD_FLAG '{other}' is not supported (money, not_tradeable)")),
    };

    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read {dir}: {e}"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("v2")))
        .collect();
    files.sort();
    let mut days = Vec::new();
    for f in &files {
        days.push(read_market(f)?);
    }
    days.sort_by_key(|d| d.0);
    let (mut compared, mut exact, mut change_ok, mut change_n, mut skipped) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut misses = Vec::new();
    for pair in days.windows(2) {
        let ((d0, m0), (d1, m1)) = (&pair[0], &pair[1]);
        let consecutive = calendar.to_game(*d0).map(|g| calendar.next_day(g)) == calendar.to_game(*d1);
        if !consecutive {
            skipped += 1;
            continue;
        }
        for g in &goods {
            let (Some((Some(p0), s0, r0, _)), Some((Some(p1), _, _, c1))) = (m0.get(&g.id), m1.get(&g.id)) else { continue };
            let mut gm = GoodMarket::new(wiktoria_simulation::Fixed::from_f64(g.cost), fixed_flag(g)?);
            gm.price = *p0;
            gm.supply = *s0;
            gm.real_demand = *r0;
            gm.update_price(&rules);
            compared += 1;
            if gm.price == *p1 {
                exact += 1;
            } else {
                misses.push(format!("{d1} {:<16} {} -> ours {} / game {}  (supply {}, real demand {})", g.id, p0, gm.price, p1, s0, r0));
            }
            if let Some(c) = c1 {
                change_n += 1;
                change_ok += usize::from(gm.price_change == *c);
            }
        }
    }
    println!("{} saves, {} day pairs ({} not consecutive, skipped)", days.len(), days.len().saturating_sub(1), skipped);
    println!("prices: {exact} of {compared} exactly equal (raw fixed point)");
    println!("price_change: {change_ok} of {change_n} equal");
    for m in misses.iter().take(if args.all_warnings { usize::MAX } else { 20 }) {
        println!("  {m}");
    }
    Ok(())
}


fn print_warnings(warnings: &[String], all: bool) {
    let limit = if all { usize::MAX } else { 40 };
    if warnings.is_empty() {
        eprintln!("\n0 warnings");
        return;
    }
    eprintln!("\n{} warnings:", warnings.len());
    for w in warnings.iter().take(limit) {
        eprintln!("  {w}");
    }
    if warnings.len() > limit {
        eprintln!("  ... {} more (use --all-warnings)", warnings.len() - limit);
    }
}

fn render_cmd(args: &Args) -> Result<(), String> {
    let out = args.positional.first().ok_or("render needs an output path, e.g. render map.bmp")?;
    let w = load_world(args)?;
    let width = args.width.clamp(64, 16384);
    let height = (width as f64 * w.map.height as f64 / w.map.width as f64).round() as usize;
    let cam = Camera::fit(&w.map, width, height);
    let pal = Palette::build(&w, args.mode);
    let mut buf = vec![0u32; width * height];
    render(&w.map, &pal, &cam, &RenderOptions { borders: true, highlight: None }, &mut buf, width, height);
    let img = wiktoria_data::bmp::Image {
        width,
        height,
        rgb: buf.iter().flat_map(|c| [(c >> 16) as u8, (c >> 8) as u8, *c as u8]).collect(),
    };
    std::fs::write(out, wiktoria_data::bmp::encode(&img)).map_err(|e| format!("cannot write {out}: {e}"))?;
    println!("wrote {out} ({width} x {height}, {} mode)", args.mode.label());
    Ok(())
}

fn goods_cmd(args: &Args) -> Result<(), String> {
    let root = &root();
    let engine_rules = config::load_engine_rules(root)?;
    let install = config::install_dir(root, args.install.clone())?;
    let path = install.join(engine_rules.require_str("FLD_GOODS_FILE")?);
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let doc = script::parse_bytes(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let loaded = goods::load_goods(&doc)?;
    println!("{} goods", loaded.goods.len());
    let mut category = "";
    for g in &loaded.goods {
        if g.category != category {
            category = &g.category;
            println!("\n[{category}]");
        }
        let mut flags = Vec::new();
        if !g.available_from_start {
            flags.push("locked at start");
        }
        if !g.tradeable {
            flags.push("not tradeable");
        }
        if g.money {
            flags.push("money");
        }
        if g.overseas_penalty {
            flags.push("overseas penalty");
        }
        println!("  {:<22} cost {:>7.2}  {}", g.id, g.cost, flags.join(", "));
    }
    print_warnings(&loaded.warnings, args.all_warnings);
    Ok(())
}

fn main() -> ExitCode {
    let result = parse_args().and_then(|a| match a.cmd.as_str() {
        "goods" => goods_cmd(&a),
        "render" => render_cmd(&a),
        "province" => province_cmd(&a),
        "country" => country_cmd(&a),
        "diff-save" => diff_save(&a),
        "diff-days" => diff_days(&a),
        "check-prices" => check_prices(&a),
        "check-colonies" => check_colonies::check_colonies(&a),
        "check-military" => check_military::check_military(&a),
        "run" => run_game::run_game(&a),
        "serve" => serve::serve(&a),
        "scenario" => scenario::scenario_cmd(&a),
        "run-military" => run_military::run_military(&a),
        "check-unrest" => check_unrest::check_unrest(&a),
        "check-money" => check_money::check_money(&a),
        "check-diplomacy" => check_diplomacy::check_diplomacy(&a),
        "check-scores" => check_scores::check_scores(&a),
        "check-occupation" => check_occupation::check_occupation(&a),
        "check-production" => check_production::check_production(&a),
        "score" => score::score(&a),
        "export" => export::export(&a),
        "fire-event" => fire_event::fire_event(&a),
        "script-coverage" => fire_event::script_coverage(&a),
        "decisions" => fire_event::decisions(&a),
        "run-events" | "why" => run_events::run_events(&a),
        "check-event" => check_event::check_event(&a),
        "check-day" => check_day::check_day(&a),
        "check-battle" => check_battle::check_battle(&a),
        _ => summary(&a),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
