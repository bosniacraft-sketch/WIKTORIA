//! `wiktoria-cli run --save <file.v2> [--days N]`: the combined game day (wiktoria_simulation::game::Game) for N days
//! (default 30), with a line block a month: great powers, wars, battles, sieges, occupation, rebels,
//! factories, colonies. Ends with checks for broken numbers (NaN, negative pops, runaway money).

use std::collections::HashSet;

use wiktoria_simulation::game::Game;
use wiktoria_simulation::military::battle::CombatKind;

use crate::{config, load_world, root, Args};

#[derive(Default)]
struct Month {
    battles: usize,
    sieges: usize,
    controller_changes: usize,
    men_lost: f64,
    orders: usize,
    risings: usize,
    rebel_regiments: usize,
    /// Pops calmed by a beaten rebel army (REDUCTION_AFTER_DEFEAT).
    calmed: usize,
    raised: usize,
    peaces: Vec<String>,
    mobilised: Vec<String>,
    built: usize,
    declared: Vec<String>,
    transferred: usize,
    goals_added: usize,
    justified: usize,
    /// AI influence actions by kind.
    influence: std::collections::BTreeMap<String, usize>,
}

pub fn run_game(args: &Args) -> Result<(), String> {
    let usage = "usage: run --save <file.v2> [--days N] [--hash] [--profile] [--why-peace] [--ai-off LIST] [--armies TAG] [--spread TAG,TAG]";
    let save_path = args.save.as_ref().ok_or(usage)?;
    let mut days = 30usize;
    let (hash, profile) = (args.positional.iter().any(|a| a == "--hash"), args.positional.iter().any(|a| a == "--profile"));
    let why_peace = args.positional.iter().any(|a| a == "--why-peace");
    let mut off = wiktoria_simulation::military::ai::AiOff::default();
    let mut armies: Option<String> = None;
    let mut spread: Vec<String> = Vec::new();
    // --budget TAG,TAG: each country's day (taxes by strata, duties, spending lines, treasury)
    // on every 1 January and at the end, in the units of a save's `incomes` and `expenses`.
    let mut budget: Vec<String> = Vec::new();
    // --influence: print every influence gain of each day with its inputs; --dip-dump FILE:
    // write every great power's influence, opinion level and priority per target at the end and
    // on every 1 January (tab separated: date power target influence level relation priority).
    let influence = args.positional.iter().any(|a| a == "--influence");
    let mut dip_dump: Option<String> = None;
    // --pop-dump FILE: every pop's mood, needs and mood inputs (check-day's columns) on every
    // 1 January and at the end, each line led by the date and followed by the pop's cash (as
    // stored in a save) and whether its owner is civilised.
    let mut pop_dump: Option<String> = None;
    // --goods-dump FILE: every day, per good, the price for tomorrow, supply and paid demand, and
    // their split: made by farms and mines, factories, artisans; bought by the states, factories,
    // pops for life, everyday and luxury needs, luxury goods over the needs, artisans' inputs.
    let mut goods_dump: Option<String> = None;
    // --pop-dump-monthly: on every 1st of the month instead of every 1 January.
    let monthly = args.positional.iter().any(|a| a == "--pop-dump-monthly");
    let mut it = args.positional.iter();
    while let Some(a) = it.next() {
        if a == "--days" {
            days = it.next().and_then(|n| n.parse().ok()).ok_or("--days needs a number")?;
        }
        if a == "--spread" {
            spread = it.next().ok_or("--spread needs tags")?.split(',').map(str::to_string).collect();
        }
        if a == "--budget" {
            budget = it.next().ok_or("--budget needs tags")?.split(',').map(str::to_string).collect();
        }
        if a == "--dip-dump" {
            dip_dump = Some(it.next().ok_or("--dip-dump needs a file")?.clone());
        }
        if a == "--goods-dump" {
            goods_dump = Some(it.next().ok_or("--goods-dump needs a file")?.clone());
        }
        if a == "--pop-dump" {
            pop_dump = Some(it.next().ok_or("--pop-dump needs a file")?.clone());
        }
        if a == "--armies" {
            armies = Some(it.next().ok_or("--armies needs a country tag")?.clone());
        }
        if a == "--ai-off" {
            off = wiktoria_simulation::military::ai::AiOff::parse(it.next().ok_or("--ai-off needs a list of rules")?)?;
        }
    }
    let w = load_world(args)?;
    let engine_rules = config::load_engine_rules(&root())?;
    let install = config::install_dir(&root(), args.install.clone())?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let bytes = std::fs::read(save_path).map_err(|e| format!("cannot read {save_path}: {e}"))?;
    let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
    let t = std::time::Instant::now();
    let mut game = Game::new(w, &engine_rules, &install, &save)?;
    eprintln!("game built in {:.2?}", t.elapsed());
    game.core.ai.off = off;
    if profile {
        game.profile = Some(Default::default());
        game.core.profile = Some(Default::default());
        wiktoria_simulation::prof::enable();
    }
    let people0: f64 = game.state.provinces.values().flat_map(|p| &p.pops).map(|q| q.size).sum();
    let by_type = |s: &wiktoria_simulation::script::State| {
        let mut m: std::collections::BTreeMap<String, f64> = std::collections::BTreeMap::new();
        for q in s.provinces.values().flat_map(|p| &p.pops) {
            *m.entry(q.pop_type.clone()).or_default() += q.money;
        }
        m
    };
    let pop_money0 = by_type(&game.state);
    let money0: f64 = game.state.countries.values().map(|c| c.money).sum();
    println!("{save_path}: {} for {days} days, all systems", game.state.date);
    report(&game, &Month::default());
    let mut month = Month::default();
    let (mut total_declared, mut total_peaces, mut total_transferred, mut total_goals) = (0usize, 0usize, 0usize, 0usize);
    let t = std::time::Instant::now();
    let mut dump = String::new();
    let mut goods = String::from("date,good,price,supply,demand,farms,factories,artisans,states,factory_inputs,life,everyday,luxury,over_luxury,artisan_inputs\n");
    let mut pops = format!("date,{},money,civilized\n", crate::check_day::MOOD_HEADER);
    for n in 0..days {
        if influence {
            game.core.influence_log = Some(Vec::new());
        }
        let day = game.day();
        if !budget.is_empty() && ((game.state.date.month == 1 && game.state.date.day == 1) || n + 1 == days) {
            print_budget(&game, &day.money, &budget);
        }
        if goods_dump.is_some() {
            goods_rows(&game, &day.money, &mut goods);
        }
        let log = day.core;
        for line in &log.crisis {
            println!("    {line}");
        }
        for (_, _, a, _, _) in game.core.influence_actions.drain(..) {
            *month.influence.entry(format!("{a:?}")).or_default() += 1;
        }
        if let Some(l) = game.core.influence_log.take() {
            for (p, t, g, i) in &l {
                println!("    influence {p} {t} gain {g:.5} {i:?}");
            }
        }
        if dip_dump.is_some() && ((game.state.date.month == 1 && game.state.date.day == 1) || n + 1 == days) {
            dump_dip(&game, &mut dump);
        }
        if pop_dump.is_some() && (((game.state.date.month == 1 || monthly) && game.state.date.day == 1) || n + 1 == days) {
            let date = game.state.date;
            let civilized: std::collections::HashSet<&String> = game.state.countries.iter().filter(|(_, c)| c.civilized).map(|(t, _)| t).collect();
            let owners: std::collections::BTreeMap<u16, &String> = game.state.provinces.iter().filter_map(|(id, p)| Some((*id, p.owner.as_ref()?))).collect();
            crate::check_day::mood_rows(&game.state, &game.world, &mut |id, b, line| pops += &format!("{date},{line},{:.3},{}\n", b.money, owners.get(&id).is_some_and(|o| civilized.contains(o)) as u8));
        }
        month.battles += log.battles_started;
        month.sieges += log.sieges_started;
        month.controller_changes += log.controller_changes;
        month.men_lost += log.men_lost;
        month.orders += log.orders;
        month.risings += log.risings;
        month.calmed += log.rebels_calmed;
        month.rebel_regiments += log.rebel_regiments;
        month.raised += log.regiments_raised;
        month.built += log.buildings_done;
        total_declared += log.wars_declared.len();
        total_peaces += log.peaces.len();
        total_transferred += log.provinces_transferred;
        total_goals += log.goals_added;
        month.declared.extend(log.wars_declared);
        month.transferred += log.provinces_transferred;
        month.goals_added += log.goals_added;
        month.justified += log.cbs_justified;
        month.peaces.extend(log.peaces);
        if why_peace {
            for line in &log.peace_trace {
                println!("    peace {line}");
            }
        }
        month.mobilised.extend(log.mobilised);
        if game.state.date.day == 1 || n + 1 == days {
            report(&game, &month);
            if let Some(tag) = &armies {
                print_armies(&game, tag);
            }
            if !spread.is_empty() {
                print_spread(&game, &spread);
            }
            if hash {
                println!("    hash {:016x}", checksum(&game));
            }
            month = Month::default();
        }
    }
    if let Some(f) = &goods_dump {
        std::fs::write(f, &goods).map_err(|e| format!("cannot write {f}: {e}"))?;
    }
    if let Some(f) = &pop_dump {
        std::fs::write(f, &pops).map_err(|e| format!("cannot write {f}: {e}"))?;
    }
    if let Some(f) = &dip_dump {
        std::fs::write(f, &dump).map_err(|e| format!("cannot write {f}: {e}"))?;
    }
    let secs = t.elapsed().as_secs_f64();
    println!();
    println!("ran {days} days in {secs:.1}s ({:.1}s per simulated year)", secs / days.max(1) as f64 * 365.0);
    if let Some(p) = &game.profile {
        let fine = wiktoria_simulation::prof::sums();
        let mut rows: Vec<_> = p.iter().chain(game.core.profile.iter().flatten()).chain(fine.iter()).collect();
        rows.sort_by(|a, b| b.1.cmp(a.1));
        println!("profile (ms per day):");
        for (name, d) in rows {
            println!("  {name:<26} {:9.2}", d.as_secs_f64() * 1000.0 / days.max(1) as f64);
        }
    }
    println!("war AI: wars declared {total_declared}, peaces {total_peaces}, provinces transferred {total_transferred}, goals added {total_goals}");
    if why_peace {
        // Every running war: its age, score and what each side holds of the other.
        let scores = game.core.day.war_scores(&game.core.mil, &game.state);
        for w in game.core.mil.wars.iter().filter(|w| w.active) {
            let held = |of: &[String], by: &[String]| game.core.mil.provinces.values().filter(|p| p.owner.as_ref().is_some_and(|o| of.contains(o)) && p.controller.as_ref().is_some_and(|c| by.contains(c))).count();
            let score = scores.iter().find(|(n, _)| *n == w.name).map_or(0.0, |(_, s)| *s);
            let since = w.started().map_or(String::new(), |d| d.to_string());
            let goals: Vec<String> = w.goals.iter().map(|g| format!("{} {}->{}", g.casus_belli, g.actor, g.receiver)).collect();
            println!("  war {}: since {since}, score {score:+.1}, attackers hold {} provinces of the defenders, defenders hold {} of the attackers, goals {goals:?}", w.name, held(&w.defenders, &w.attackers), held(&w.attackers, &w.defenders));
        }
    }
    // Anything that looks broken.
    let s = &game.state;
    let nan_pops = s.provinces.values().flat_map(|p| &p.pops).filter(|q| !q.size.is_finite() || !q.money.is_finite() || !q.militancy.is_finite()).count();
    let neg_pops = s.provinces.values().flat_map(|p| &p.pops).filter(|q| q.size < 0.0).count();
    let nan_countries: Vec<&String> = s.countries.iter().filter(|(_, c)| !c.money.is_finite() || !c.prestige.is_finite() || !c.war_exhaustion.is_finite()).map(|(t, _)| t).collect();
    let people1: f64 = s.provinces.values().flat_map(|p| &p.pops).map(|q| q.size).sum();
    let money1: f64 = s.countries.values().map(|c| c.money).sum();
    let mut rich: Vec<(&String, f64)> = s.countries.iter().map(|(t, c)| (t, c.money)).collect();
    rich.sort_by(|a, b| b.1.total_cmp(&a.1));
    println!("checks: pops with NaN {nan_pops}, negative pops {neg_pops}, countries with NaN {}{}", nan_countries.len(), if nan_countries.is_empty() { String::new() } else { format!(" ({:?})", nan_countries) });
    println!("  population {people0:.0} -> {people1:.0} ({:+.2}%); treasuries {money0:.0} -> {money1:.0} (x{:.2}); richest {}", (people1 / people0 - 1.0) * 100.0, money1 / money0.max(1.0), rich.iter().take(5).map(|(t, m)| format!("{t} {m:.0}")).collect::<Vec<_>>().join(", "));
    let pop_money1 = by_type(s);
    println!("  pop cash by type (start -> end): {}", pop_money0.iter().map(|(t, a)| format!("{t} {a:.0}->{:.0}", pop_money1.get(t).copied().unwrap_or(0.0))).collect::<Vec<_>>().join(", "));
    let mut by_owner: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for f in &s.factories {
        *by_owner.entry(f.owner.as_str()).or_default() += 1;
    }
    let mut owners: Vec<(&str, usize)> = by_owner.into_iter().collect();
    owners.sort_by_key(|x| std::cmp::Reverse(x.1));
    println!("  factories by owner ({} in all): {}", s.factories.len(), owners.iter().take(15).map(|(t, n)| format!("{t} {n}")).collect::<Vec<_>>().join(", "));
    let mut sites: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    let mut site_owners: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for (t, x) in &game.core.sites {
        *sites.entry(x.building.as_str()).or_default() += 1;
        *site_owners.entry(t.as_str()).or_default() += 1;
    }
    let mut site_owners: Vec<(&str, usize)> = site_owners.into_iter().collect();
    site_owners.sort_by_key(|x| std::cmp::Reverse(x.1));
    let levels = |kind: &str| s.provinces.values().map(|p| p.buildings.get(kind).copied().unwrap_or(0.0)).sum::<f64>();
    println!("  province buildings: levels naval_base {}, fort {}, railroad {}; sites {:?}, most {}", levels("naval_base"), levels("fort"), levels("railroad"), sites, site_owners.iter().take(8).map(|(t, n)| format!("{t} {n}")).collect::<Vec<_>>().join(", "));
    // People under half their life needs, civilised and not (original: 3.1% on 1836.2.8,
    // 15.7-21.2% on 1837.1.1).
    let mut hungry = [(0.0, 0.0); 2];
    for p in s.provinces.values() {
        let civ = p.owner.as_ref().and_then(|o| s.countries.get(o)).is_some_and(|c| c.civilized);
        for q in &p.pops {
            let h = &mut hungry[usize::from(civ)];
            h.1 += q.size;
            if q.life_needs < 0.5 {
                h.0 += q.size;
            }
        }
    }
    let share = |h: (f64, f64)| 100.0 * h.0 / h.1.max(1.0);
    println!("  people under half life needs: {:.1}% (civilised {:.1}%, uncivilised {:.1}%)", share((hungry[0].0 + hungry[1].0, hungry[0].1 + hungry[1].1)), share(hungry[1]), share(hungry[0]));
    // The computer players' budgets, counted as on the original's yearly saves.
    let landed: HashSet<&String> = s.provinces.values().filter_map(|p| p.owner.as_ref()).collect();
    let ai: Vec<&wiktoria_simulation::script::CountryState> = s.countries.iter().filter(|(t, c)| landed.contains(t) && !c.human).map(|(_, c)| c).collect();
    let get = |c: &wiktoria_simulation::script::CountryState, k: &str| c.spending.get(k).copied().unwrap_or(0.0);
    let count = |f: &dyn Fn(&wiktoria_simulation::script::CountryState) -> bool| ai.iter().filter(|c| f(c)).count();
    println!(
        "  budgets of {} computer players: poor tax >= 0.95 {}; education and administration at 0.2 with a tariff {}; no tariff, education > 0.3 {}; both {}",
        ai.len(),
        count(&|c| get(c, "poor_tax") >= 0.95),
        count(&|c| (get(c, "education_spending") - 0.2).abs() < 0.02 && (get(c, "crime_fighting") - 0.2).abs() < 0.02 && get(c, "tariffs") > 0.0),
        count(&|c| get(c, "tariffs") == 0.0 && get(c, "education_spending") > 0.3),
        count(&|c| get(c, "tariffs") > 0.0 && get(c, "education_spending") > 0.3)
    );
    let m = &game.core.mil;
    let building = |t: &str, land: bool| m.constructions.iter().filter(|c| c.owner == t && game.core.day.units.get(&c.kind).is_some_and(|u| u.land == land)).count();
    println!("  units by country (regiments/ships/building land+sea): {}", ["ENG", "FRA", "RUS", "PRU", "AUS", "USA", "TUR", "SPA", "JAP", "CHI", "EGY", "MEX", "SAR", "SWE"].iter().map(|t| format!("{t} {}/{}/{}+{}", m.regiment_count(t), m.ship_count(t), building(t, true), building(t, false))).collect::<Vec<_>>().join(", "));
    // Where the armies stand: regiments by province, largest first (the original's computer
    // players hold some stacks of about 10 at the capital and border fronts).
    for t in ["FRA", "AUS", "RUS", "PRU", "SPA"] {
        let mut at: std::collections::BTreeMap<u16, usize> = std::collections::BTreeMap::new();
        for a in m.armies_of(t) {
            *at.entry(a.location).or_default() += a.regiments.len();
        }
        let mut v: Vec<(u16, usize)> = at.into_iter().collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.1));
        println!("  armies of {t} (capital {}): {} provinces: {}", s.countries.get(t).and_then(|c| c.capital).unwrap_or(0), v.len(), v.iter().map(|(p, n)| format!("{p} {n}")).collect::<Vec<_>>().join(", "));
    }
    if let Some((_, u)) = &game.core.unrest {
        let angry = s.provinces.values().flat_map(|p| &p.pops).filter(|q| q.militancy > 7.0).count();
        let members: usize = u.factions.iter().map(|f| f.pops.len()).sum();
        println!("  unrest: pops with militancy > 7 {angry}; factions {} with {members} pops; movements {} with {} pops", u.factions.len(), u.movements.len(), u.pop_movement.len());
        let mut kinds: std::collections::BTreeMap<&str, Vec<String>> = std::collections::BTreeMap::new();
        for f in &u.factions {
            kinds.entry(f.kind.as_str()).or_default().push(format!("{}>{}:{} org {:.3}", f.country, f.creates.as_deref().unwrap_or("---"), f.pops.len(), f.organization));
        }
        let mut ms: Vec<&wiktoria_simulation::unrest::Movement> = u.movements.iter().collect();
        ms.sort_by(|a, b| b.support.total_cmp(&a.support));
        println!("    movements: {}", ms.iter().take(20).map(|m| format!("{} {} {:.0} ({:.1})", m.country, m.target, m.support, m.radicalism)).collect::<Vec<_>>().join(", "));
        println!("    factions: {}", kinds.iter().map(|(k, v)| format!("{k} [{}]", v.join(" "))).collect::<Vec<_>>().join("; "));
    }
    println!("  regiments {} in {} armies ({} rebel armies); building sites open {}", game.core.mil.armies.iter().map(|a| a.regiments.len()).sum::<usize>(), game.core.mil.armies.len(), game.core.mil.armies_of(&game.core.day.rebel_tag).count(), game.core.sites.len());
    Ok(())
}

/// One line per (great power, target) with influence, a level other than neutral or a priority.
fn dump_dip(game: &Game, out: &mut String) {
    use std::fmt::Write;
    let Some((_, dip)) = &game.core.dip else { return };
    let date = game.state.date;
    for g in dip.great_powers() {
        for ((a, b), r) in dip.relations.range((g.to_string(), String::new())..).take_while(|((a, _), _)| a == g) {
            let pri = dip.priorities.get(&(a.clone(), b.clone())).copied().unwrap_or(0);
            if r.influence > 0.0 || r.level != 2 || pri > 0 {
                let _ = writeln!(out, "{date}\t{a}\t{b}\t{:.3}\t{}\t{:.0}\t{pri}", r.influence, r.level, r.value);
            }
        }
    }
}

/// FNV-1a over the debug print of the state and the military: equal runs give equal numbers, and any
/// changed float bit changes the print.
fn checksum(game: &Game) -> u64 {
    struct Fnv(u64);
    impl std::fmt::Write for Fnv {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            for b in s.bytes() {
                self.0 = (self.0 ^ b as u64).wrapping_mul(0x100000001b3);
            }
            Ok(())
        }
    }
    let mut h = Fnv(0xcbf29ce484222325);
    let _ = std::fmt::Write::write_fmt(&mut h, format_args!("{:?}{:?}", game.state, game.core.mil));
    h.0
}

/// `--spread TAGS`: per country the regiments standing in its own land (held / occupied by the
/// enemy) and in other countries' land, and how many of its provinces others hold.
fn print_spread(game: &Game, tags: &[String]) {
    let mil = &game.core.mil;
    let mut line = Vec::new();
    for tag in tags {
        let (mut own, mut own_occupied, mut abroad, mut armies) = (0, 0, 0, 0);
        for a in mil.armies_of(tag) {
            armies += 1;
            let p = mil.provinces.get(&a.location);
            match (p.and_then(|p| p.owner.as_deref()), p.and_then(|p| p.controller.as_deref())) {
                (Some(o), Some(c)) if o == tag && c == tag => own += a.regiments.len(),
                (Some(o), _) if o == tag => own_occupied += a.regiments.len(),
                _ => abroad += a.regiments.len(),
            }
        }
        let held = mil.provinces.values().filter(|p| p.owner.as_deref() == Some(tag.as_str()) && p.controller != p.owner).count();
        line.push(format!("{tag}: {armies} armies, regiments own {own} own-occupied {own_occupied} abroad {abroad}; provinces held by others {held}"));
    }
    println!("    spread: {}", line.join("; "));
}

/// `--armies TAG`: where each army of the country stands, what it is doing and where the AI sends it.
fn print_armies(game: &Game, tag: &str) {
    let (mil, core) = (&game.core.mil, &game.core);
    let sieging: HashSet<u64> = mil.sieges().flat_map(|c| c.attacker.units.iter().copied()).collect();
    let fighting: HashSet<u64> = mil.combats.iter().filter(|c| c.kind != CombatKind::Siege).flat_map(|c| c.attacker.units.iter().chain(&c.defender.units).copied()).collect();
    for a in mil.armies_of(tag) {
        let men: f64 = a.regiments.iter().map(|g| g.strength).sum::<f64>() * 1000.0;
        let org = a.regiments.iter().map(|g| g.organisation).sum::<f64>() / a.regiments.len().max(1) as f64;
        let holder = mil.provinces.get(&a.location).and_then(|p| p.controller.clone()).unwrap_or_default();
        let doing = if fighting.contains(&a.id) {
            "in battle"
        } else if sieging.contains(&a.id) {
            "besieging"
        } else if core.ai_resting.contains(&a.id) {
            "resting"
        } else if !a.path.is_empty() {
            "marching"
        } else if a.carried_by.is_some() {
            "aboard"
        } else {
            "standing"
        };
        println!(
            "    army {:>6} of {tag}: {:>2} regiments, {:>6.0} men, organisation {:>5.1}, in {} ({holder}), {doing}{}{}",
            a.id,
            a.regiments.len(),
            men,
            org,
            a.location,
            a.path.last().map_or(String::new(), |p| format!(", path of {} to {p}", a.path.len())),
            core.ai_targets.get(&a.id).map_or(String::new(), |t| format!(", target {t}"))
        );
    }
}

fn report(game: &Game, m: &Month) {
    let (s, mil) = (&game.state, &game.core.mil);
    let gps: Vec<String> = s.rank_order.iter().filter(|t| s.great_powers.contains(t)).chain(s.great_powers.iter().filter(|t| !s.rank_order.contains(t))).filter_map(|t| s.countries.get(t).map(|c| format!("{t} ${:.0} p{:.0}", c.money, c.prestige))).collect();
    println!("{}: great powers {}", s.date, gps.join(", "));
    let scores = game.core.day.war_scores(mil, s);
    let wars: Vec<String> = mil.wars.iter().filter(|w| w.active).map(|w| format!("{} {}v{} {:+.0}", w.name, w.attackers.join("+"), w.defenders.join("+"), scores.iter().find(|(n, _)| *n == w.name).map_or(0.0, |x| x.1))).collect();
    println!("    wars {}: {}", wars.len(), if wars.is_empty() { "none".into() } else { wars.join("; ") });
    let battles = mil.combats.iter().filter(|c| c.kind != CombatKind::Siege).count();
    println!(
        "    battles now {battles} (started {}), sieges now {} (started {}), occupied provinces {}, controller changes {}, men lost {:.0}, AI orders {}",
        m.battles,
        mil.sieges().count(),
        m.sieges,
        mil.occupied().count(),
        m.controller_changes,
        m.men_lost,
        m.orders
    );
    let colonies = s.provinces.values().filter(|p| p.colonial > 0).count();
    println!(
        "    risings {} ({} rebel regiments, {} pops calmed by defeats), mobilised {:?} (+{} regiments), peaces {:?}, factories {}, buildings done {}, colonial provinces {}, races {}",
        m.risings,
        m.rebel_regiments,
        m.calmed,
        m.mobilised,
        m.raised,
        m.peaces,
        s.factories.len(),
        m.built,
        colonies,
        game.core.races.len()
    );
    println!("    declared {:?}, casus belli started {}, goals added {}, provinces transferred {}", m.declared, m.justified, m.goals_added, m.transferred);
    if let Some((_, dip)) = &game.core.dip {
        let spheres: Vec<String> = dip.great_powers().map(|g| format!("{g} {}", dip.sphere_members(g).count())).collect();
        println!("    spheres {}; influence actions {:?}", spheres.join(", "), m.influence);
    }
    if let Some((r, c)) = &game.core.crisis {
        let hot = c.flashpoints.iter().filter(|f| f.tension >= r.tension_max).count();
        let top = c.flashpoints.iter().max_by(|a, b| a.tension.total_cmp(&b.tension)).map_or(String::new(), |f| format!(", hottest {} in {} {:.1}", f.tag, f.owner, f.tension));
        let now = c.crisis.as_ref().map_or("none".to_string(), |x| format!("{} v {} temperature {:.1}, backers {:?} / {:?}, undecided {}", x.attacker, x.defender, x.temperature, x.attacker_backers, x.defender_backers, x.interested.len()));
        println!("    flashpoints {} ({hot} at full tension{top}); crisis {now}; last ended {:?}", c.flashpoints.len(), c.last_end.map(|d| d.to_string()));
    }
}

/// One day of each named country's budget (see `--budget`).
fn print_budget(game: &Game, money: &wiktoria_simulation::economy::FarmMoney, tags: &[String]) {
    let strata: std::collections::BTreeMap<&str, &str> = game.world.defs.pop_types.iter().map(|t| (t.name.as_str(), t.strata.as_str())).collect();
    for tag in tags {
        let mut tax: std::collections::BTreeMap<&str, f64> = std::collections::BTreeMap::new();
        for ((t, kind), row) in &money.taxed_by {
            if t == tag {
                *tax.entry(strata.get(kind.as_str()).copied().unwrap_or("?")).or_default() += row[2];
            }
        }
        let spent: Vec<String> = money.spent_on.iter().filter(|((t, _), _)| t == tag).map(|((_, line), x)| format!("{line} {x:.1}")).collect();
        let treasury = game.state.countries.get(tag).map_or(0.0, |c| c.money);
        println!(
            "  budget {} {tag}: tax poor {:.1} middle {:.1} rich {:.1}, duties {:.1} | {} | treasury {treasury:.0}",
            game.state.date,
            tax.get("poor").copied().unwrap_or(0.0),
            tax.get("middle").copied().unwrap_or(0.0),
            tax.get("rich").copied().unwrap_or(0.0),
            money.duties.get(tag).copied().unwrap_or(0.0),
            spent.join(", ")
        );
    }
}

/// One `--goods-dump` line per good for the day just run.
fn goods_rows(game: &Game, m: &wiktoria_simulation::economy::FarmMoney, out: &mut String) {
    let mut made: std::collections::BTreeMap<&str, [f64; 3]> = std::collections::BTreeMap::new();
    let mut wanted: std::collections::BTreeMap<&str, [f64; 3]> = std::collections::BTreeMap::new();
    for ((_, good), x) in &m.made_by {
        let e = made.entry(good).or_default();
        for i in 0..3 {
            e[i] += x[i];
        }
    }
    for ((_, good), x) in &m.wanted_by {
        let e = wanted.entry(good).or_default();
        for i in 0..2 {
            e[i] += x[i];
        }
    }
    let date = game.state.date;
    let goods: std::collections::BTreeSet<&String> = m.supply.keys().chain(m.demand.keys()).collect();
    for good in goods {
        let s = m.supply.get(good).copied().unwrap_or(0.0);
        let (a, b, p) = (made.get(good.as_str()).copied().unwrap_or_default(), wanted.get(good.as_str()).copied().unwrap_or_default(), m.pop_wanted.get(good).copied().unwrap_or_default());
        let price = game.state.prices.get(good).copied().unwrap_or(0.0);
        let d = m.demand.get(good).copied().unwrap_or(0.0);
        out.push_str(&format!("{date},{good},{price:.4},{s:.3},{d:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}\n", a[0], a[1], a[2], b[0], b[1], p[0], p[1], p[2], p[3], p[4]));
    }
}
