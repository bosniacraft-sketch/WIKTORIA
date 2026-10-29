//! `wiktoria-cli check-battle --save <day N.v2> <day N+1.v2>` or `check-battle --dir <folder of daily saves>`
//!
//! Fights every land battle of the first save for one day in the port (that battle alone, with the
//! original's dice of the next day where the port would roll) and compares the regiments, the losses
//! and the lines with the original's save of the next day. With `--dir` every pair of consecutive
//! saves is checked and one line a day is printed.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use wiktoria_simulation::military::battle::{self, BattleSide, Combat, CombatKind, LandContext, LandReport};
use wiktoria_simulation::military::day::MilitaryDay;
use wiktoria_simulation::military::{MilitaryRules, MilitaryState};
use wiktoria_simulation::script::State;

use crate::{config, load_world, root, Args};

/// The save writes strength and organisation with three decimals.
const SAVE_STEP: f64 = 0.001;
/// "Within 1%" of the regiment's value on day N.
const CLOSE_SHARE: f64 = 0.01;
const SIDES: [BattleSide; 2] = [BattleSide::Attacker, BattleSide::Defender];
const WHAT: [&str; 2] = ["strength", "organisation"];

struct Day {
    mil: MilitaryState,
    state: State,
}

/// One regiment of a battle: strength and organisation on day N, and on day N+1 in the game (none
/// when the regiment is gone there) and in the port.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    id: u64,
    side: BattleSide,
    kind: String,
    before: [f64; 2],
    game: Option<[f64; 2]>,
    port: [f64; 2],
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Tally {
    same: usize,
    close: usize,
    off: usize,
}

impl Tally {
    fn add(&mut self, o: Tally) {
        self.same += o.same;
        self.close += o.close;
        self.off += o.off;
    }
}

/// Port against game for strength (`k` 0) or organisation (`k` 1) of the regiments still in the game.
fn tally(rows: &[Row], k: usize) -> Tally {
    let mut t = Tally::default();
    for r in rows {
        let Some(g) = r.game else { continue };
        let d = (r.port[k] - g[k]).abs();
        if d <= SAVE_STEP + 1e-9 {
            t.same += 1;
        } else if d <= CLOSE_SHARE * r.before[k] {
            t.close += 1;
        } else {
            t.off += 1;
        }
    }
    t
}

/// What the regiments of a side lost that day, (game, port), over those still in the game.
fn lost(rows: &[Row], side: BattleSide, k: usize) -> (f64, f64) {
    rows.iter().filter(|r| r.side == side).filter_map(|r| Some((r.before[k] - r.game?[k], r.before[k] - r.port[k]))).fold((0.0, 0.0), |a, x| (a.0 + x.0, a.1 + x.1))
}

/// Regiments whose strength fell: (in the game, in the port, in both).
fn hit(rows: &[Row], side: BattleSide) -> (usize, usize, usize) {
    let fell = |before: f64, after: f64| before - after > SAVE_STEP + 1e-9;
    let mut n = (0, 0, 0);
    for r in rows.iter().filter(|r| r.side == side) {
        let Some(g) = r.game else { continue };
        let (a, b) = (fell(r.before[0], g[0]), fell(r.before[0], r.port[0]));
        n = (n.0 + a as usize, n.1 + b as usize, n.2 + (a && b) as usize);
    }
    n
}

/// Slots of one line that hold the same regiment in both (or are empty in both), out of the slots
/// either uses, and the ones that differ in words.
fn line_diff(game: &BTreeMap<u32, u64>, port: &BTreeMap<u32, u64>) -> (usize, usize, Vec<String>) {
    let slots: BTreeSet<u32> = game.keys().chain(port.keys()).copied().collect();
    let name = |x: Option<&u64>| x.map_or("empty".to_string(), |id| id.to_string());
    let differ: Vec<String> = slots.iter().filter(|p| game.get(p) != port.get(p)).map(|p| format!("slot {p}: game {}, port {}", name(game.get(p)), name(port.get(p)))).collect();
    (slots.len() - differ.len(), slots.len(), differ)
}

fn label(s: BattleSide) -> &'static str {
    if s == BattleSide::Attacker { "attacker" } else { "defender" }
}

fn share(port: f64, game: f64) -> String {
    if game.abs() < SAVE_STEP { "-".to_string() } else { format!("{:.0}%", port / game * 100.0) }
}

/// One battle fought a day in the port, next to the game's state of the next day.
struct Check {
    location: u16,
    countries: [Vec<String>; 2],
    before: Combat,
    game: Combat,
    port: Combat,
    report: LandReport,
    ctx: LandContext,
    rows: Vec<Row>,
    /// Dig-in of the defending armies on day N, as the save has it.
    dig_in: Vec<f64>,
    /// Things about this battle the port does not know or that changed outside the round.
    notes: Vec<String>,
}

impl Check {
    fn game_rolled(&self) -> bool {
        self.game.day <= self.before.day
    }

    fn port_rolled(&self) -> bool {
        self.report.rerolled || self.before.day < 0.0
    }

    fn lines_same(&self) -> (usize, usize) {
        let mut n = (0, 0);
        for s in SIDES {
            for (g, p) in [(&self.game.side(s).front, &self.port.side(s).front), (&self.game.side(s).back, &self.port.side(s).back)] {
                let (same, all, _) = line_diff(g, p);
                n = (n.0 + same, n.1 + all);
            }
        }
        n
    }
}

fn owners(mil: &MilitaryState, units: &[u64]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in mil.armies.iter().filter(|a| units.contains(&a.id)) {
        if !out.contains(&a.owner) {
            out.push(a.owner.clone());
        }
    }
    out
}

fn check_one(day: &MilitaryDay, w: &wiktoria_data::world::World, a: &Day, b: &Day, ci: usize, game: &Combat) -> Check {
    let before = a.mil.combats[ci].clone();
    let (port, fighters, ctx, report) = day.land_round_alone(&a.mil, &a.state, ci, [game.attacker.dice, game.defender.dice]);
    let now: BTreeMap<u64, [f64; 2]> = b.mil.armies.iter().flat_map(|x| &x.regiments).map(|r| (r.id, [r.strength, r.organisation])).collect();
    let mut rows = Vec::new();
    let mut notes = Vec::new();
    for s in SIDES {
        let units = &before.side(s).units;
        for army in a.mil.armies.iter().filter(|x| units.contains(&x.id)) {
            for r in &army.regiments {
                if let Some(f) = fighters.get(&r.id) {
                    rows.push(Row { id: r.id, side: s, kind: r.kind.clone(), before: [r.strength, r.organisation], game: now.get(&r.id).copied(), port: [f.strength, f.organisation] });
                }
            }
        }
        let leader = day.side_leader(&a.mil, &before, s);
        let t = day.content.leader_traits(leader);
        let who = leader.map_or("no leader, the stand-in's traits".to_string(), |l| format!("{} ({}, {}; prestige {})", l.name, l.personality, l.background, l.prestige));
        notes.push(format!("{} leader: {who}: attack {:+}, defence {:+}; the {} counts", label(s), t.attack, t.defence, if s == BattleSide::Attacker { "attack" } else { "defence" }));
        let (was, is) = (units, &game.side(s).units);
        let joined: Vec<String> = is.iter().filter(|u| !was.contains(u)).map(u64::to_string).collect();
        let left: Vec<String> = was.iter().filter(|u| !is.contains(u)).map(u64::to_string).collect();
        if !joined.is_empty() || !left.is_empty() {
            notes.push(format!("{} armies changed in the game that day (joined: {}; left: {}): the port's round does not know, their regiments are not compared", label(s), if joined.is_empty() { "none".into() } else { joined.join(" ") }, if left.is_empty() { "none".into() } else { left.join(" ") }));
        }
    }
    for army in a.mil.armies.iter().filter(|x| before.attacker.units.contains(&x.id)) {
        let from = army.previous.map_or("no previous province".to_string(), |p| format!("province {p}, {} river pixels on the border", w.river_borders.get(&(p.min(before.location), p.max(before.location))).copied().unwrap_or(0)));
        notes.push(format!("attacking army {} came from {from}: {:?}", army.id, day.entry(army, before.location)));
    }
    if !day.terrain.contains_key(&before.location) {
        notes.push("the port has no terrain for this province: no terrain penalty, plain width".to_string());
    }
    let countries = [owners(&a.mil, &before.attacker.units), owners(&a.mil, &before.defender.units)];
    let dig_in = a.mil.armies.iter().filter(|x| before.defender.units.contains(&x.id)).map(|x| x.dig_in).collect();
    Check { location: before.location, countries, before, game: game.clone(), port, report, ctx, rows, dig_in, notes }
}

/// The land battles of one day: those in both saves checked, the others named.
struct Pair {
    checks: Vec<Check>,
    /// Battles of day N that are gone on day N+1, with what the port's round made of them.
    ended: Vec<String>,
    began: Vec<u16>,
    naval: usize,
}

fn check_pair(day: &MilitaryDay, w: &wiktoria_data::world::World, a: &Day, b: &Day) -> Pair {
    let mut pair = Pair { checks: Vec::new(), ended: Vec::new(), began: Vec::new(), naval: a.mil.combats.iter().filter(|c| c.kind == CombatKind::Naval).count() };
    for (ci, c) in a.mil.combats.iter().enumerate().filter(|(_, c)| c.kind == CombatKind::Land) {
        match b.mil.combats.iter().find(|g| g.kind == CombatKind::Land && g.location == c.location) {
            Some(game) => pair.checks.push(check_one(day, w, a, b, ci, game)),
            None => {
                // No next dice to borrow: the day N dice stand in if the port wants to roll.
                let (_, _, _, rep) = day.land_round_alone(&a.mil, &a.state, ci, [c.attacker.dice, c.defender.dice]);
                let port = match rep.outcome {
                    Some(o) => format!("the port ends it too, the {} wins", label(o.winner)),
                    None => "the port fights on: MISMATCH".to_string(),
                };
                pair.ended.push(format!("province {}: over in the game after {} days; {port}", c.location, c.duration));
            }
        }
    }
    pair.began = b.mil.combats.iter().filter(|g| g.kind == CombatKind::Land && !a.mil.combats.iter().any(|c| c.kind == CombatKind::Land && c.location == g.location)).map(|g| g.location).collect();
    pair
}

fn print_check(day: &MilitaryDay, x: &Check, shown: usize) {
    println!();
    println!("land battle in province {}: {} attack {}, day {} of the battle", x.location, x.countries[0].join("+"), x.countries[1].join("+"), x.game.duration);
    let dice = |c: &Combat| format!("{} against {}", c.attacker.dice, c.defender.dice);
    match (x.game_rolled(), x.port_rolled()) {
        (true, true) => println!("  dice: rolled anew this day in the game and in the port; the game's {} are used", dice(&x.game)),
        (false, false) if dice(&x.game) == dice(&x.before) => println!("  dice: held in both ({})", dice(&x.game)),
        (false, false) => println!("  dice: MISMATCH, the game's changed from {} to {} with no new phase; the port kept the old", dice(&x.before), dice(&x.game)),
        (true, false) => println!("  dice: MISMATCH, the game rolled anew ({}), the port held {}", dice(&x.game), dice(&x.before)),
        (false, true) => println!("  dice: MISMATCH, the port rolled anew, the game held {}", dice(&x.game)),
    }
    if (x.port.day, x.port.duration) == (x.game.day, x.game.duration) {
        println!("  day of the dice {} and days fought {}: the same", x.game.day, x.game.duration);
    } else {
        println!("  MISMATCH of the counters: day of the dice game {} port {}, days fought game {} port {}", x.game.day, x.port.day, x.game.duration, x.port.duration);
    }
    let t = day.terrain.get(&x.location).copied().unwrap_or_default();
    println!("  the port's inputs: added to the dice {:+} attacker (crossing {:+} in it), {:+} defender; width {}; terrain defence {} width {:+}; fort {}{}; dig-in of the defending armies {}", x.report.modifiers[0], x.ctx.crossing, x.report.modifiers[1], battle::width(&day.battle, &x.ctx), t.defence, t.width, x.ctx.fort, if x.ctx.attacker.fort_applies { " (counts against the attacker)" } else { "" }, x.dig_in.iter().map(f64::to_string).collect::<Vec<_>>().join(", "));
    if let Some(o) = &x.report.outcome {
        println!("  MISMATCH: the port ends the battle this day (the {} wins{}), the game fights on", label(o.winner), if o.loser_destroyed { ", the loser destroyed" } else { "" });
    }
    for s in SIDES {
        let mine: Vec<Row> = x.rows.iter().filter(|r| r.side == s).cloned().collect();
        let gone = mine.iter().filter(|r| r.game.is_none()).count();
        println!("  {} ({} regiments{})", label(s), mine.len(), if gone > 0 { format!(", {gone} gone in the game and left out") } else { String::new() });
        for (k, what) in WHAT.iter().enumerate() {
            let (g, p) = lost(&x.rows, s, k);
            println!("    {what} lost: game {g:.3}, port {p:.3} ({})", share(p, g));
        }
        let (g, p) = (x.game.side(s).losses - x.before.side(s).losses, x.port.side(s).losses - x.before.side(s).losses);
        println!("    men added to the battle's losses: game {g:.0}, port {p:.0} ({})", share(p, g));
        let (hg, hp, both) = hit(&x.rows, s);
        println!("    regiments that lost strength: game {hg}, port {hp}, the same ones {both}");
        for (k, what) in WHAT.iter().enumerate() {
            let t = tally(&mine, k);
            println!("    {what} per regiment: the same {}, within 1% {}, different {}", t.same, t.close, t.off);
        }
        for (name, g, p) in [("front line", &x.game.side(s).front, &x.port.side(s).front), ("second line", &x.game.side(s).back, &x.port.side(s).back)] {
            let (same, all, differ) = line_diff(g, p);
            if differ.is_empty() {
                println!("    {name}: the same ({all} slots)");
            } else {
                println!("    {name}: {same} of {all} slots the same; {}{}", differ.iter().take(shown).cloned().collect::<Vec<_>>().join("; "), if differ.len() > shown { "; ..." } else { "" });
            }
        }
        let waiting = |c: &Combat| -> BTreeSet<u64> { c.side(s).reserve.iter().chain(&c.side(s).broken).copied().collect() };
        let (g, p) = (waiting(&x.game), waiting(&x.port));
        if g == p {
            println!("    reserve: the same {} regiments", g.len());
        } else {
            println!("    reserve: game {:?}, port {:?}", g, p);
        }
    }
    for (k, what) in WHAT.iter().enumerate() {
        let mut worst: Vec<(f64, &Row, [f64; 2])> = x.rows.iter().filter_map(|r| Some(((r.port[k] - r.game?[k]).abs(), r, r.game?))).filter(|(d, _, _)| *d > SAVE_STEP + 1e-9).collect();
        worst.sort_by(|a, b| b.0.total_cmp(&a.0));
        if worst.is_empty() {
            continue;
        }
        println!("  largest differences in {what}:");
        for (_, r, g) in worst.iter().take(shown) {
            println!("    regiment {} ({}, {}): day before {:.3}, game {:.3}, port {:.3}", r.id, r.kind, label(r.side), r.before[k], g[k], r.port[k]);
        }
    }
    for n in &x.notes {
        println!("  note: {n}");
    }
}

fn print_others(p: &Pair) {
    for e in &p.ended {
        println!("{e}");
    }
    if !p.began.is_empty() {
        println!("began in the game this day, not compared (the check does not march armies): province {}", p.began.iter().map(u16::to_string).collect::<Vec<_>>().join(", "));
    }
    if p.naval > 0 {
        println!("{} naval battles not compared: the port's naval round draws chance for every ship and a save is not read for where the ships stand", p.naval);
    }
}

/// One line for a day: every battle of the pair added up.
fn one_line(p: &Pair) -> String {
    if p.checks.is_empty() {
        return format!("no land battle in both saves{}", if p.ended.is_empty() { String::new() } else { format!(" ({} ended)", p.ended.len()) });
    }
    let rows: Vec<Row> = p.checks.iter().flat_map(|c| c.rows.iter().cloned()).collect();
    let mut sums = [(0.0, 0.0); 2];
    let mut tallies = [Tally::default(); 2];
    for k in 0..2 {
        for s in SIDES {
            let (g, q) = lost(&rows, s, k);
            sums[k] = (sums[k].0 + g, sums[k].1 + q);
        }
        tallies[k].add(tally(&rows, k));
    }
    let lines = p.checks.iter().map(Check::lines_same).fold((0, 0), |a, x| (a.0 + x.0, a.1 + x.1));
    let dice_off = p.checks.iter().filter(|c| c.game_rolled() != c.port_rolled()).count();
    let port_ends = p.checks.iter().filter(|c| c.report.outcome.is_some()).count();
    let mut text = format!("{} battles, {} regiments; strength lost game {:.2} port {:.2} ({}); organisation lost game {:.2} port {:.2} ({})", p.checks.len(), rows.len(), sums[0].0, sums[0].1, share(sums[0].1, sums[0].0), sums[1].0, sums[1].1, share(sums[1].1, sums[1].0));
    for (k, what) in WHAT.iter().enumerate() {
        text += &format!("; {what} same {} within 1% {} different {}", tallies[k].same, tallies[k].close, tallies[k].off);
    }
    text += &format!("; line slots the same {} of {}", lines.0, lines.1);
    if dice_off > 0 {
        text += &format!("; dice phase differs in {dice_off}");
    }
    if port_ends > 0 {
        text += &format!("; the port ends {port_ends} the game fights on");
    }
    if !p.ended.is_empty() {
        text += &format!("; {} ended in the game ({} of them fight on in the port)", p.ended.len(), p.ended.iter().filter(|e| e.contains("MISMATCH")).count());
    }
    text
}

pub fn check_battle(args: &Args) -> Result<(), String> {
    let usage = "usage: check-battle --save <day N.v2> <day N+1.v2>  or  check-battle --dir <folder of daily saves>  [--gas TAG]...";
    let w = load_world(args)?;
    let engine_rules = config::load_engine_rules(&root())?;
    let install = config::install_dir(&root(), args.install.clone())?;
    let rules = MilitaryRules::new(&install, &engine_rules)?;
    let mut day = MilitaryDay::from_world(&w, &engine_rules, &install, &rules, 0)?;
    // `--gas TAG`: the country has gas attack and gas defence, for saves of a game with other inventions.
    let mut positional = args.positional.clone();
    while let Some(i) = positional.iter().position(|a| a == "--gas") {
        day.content.gas_override.insert(positional.get(i + 1).ok_or(usage)?.clone());
        positional.drain(i..=i + 1);
    }
    let dir = positional.iter().position(|a| a == "--dir").map(|i| positional.get(i + 1).ok_or(usage)).transpose()?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let load = |path: &str| -> Result<Day, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
        let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
        let mut state = State::from_save(&save);
        state.attach_world(&w, &save);
        Ok(Day { mil: MilitaryState::from_save(&save, &rules), state })
    };
    let next_day = |a: &Day, b: &Day| -> Result<(), String> {
        let want = day.calendar.to_date(day.calendar.next_day(day.calendar.to_game(a.mil.date).ok_or("the save's date is not in the calendar")?));
        if b.mil.date == want { Ok(()) } else { Err(format!("the second save is dated {}, not the next day {want}", b.mil.date)) }
    };
    if let Some(dir) = dir {
        let mut files: Vec<_> = std::fs::read_dir(dir).map_err(|e| format!("cannot read {dir}: {e}"))?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "v2")).collect();
        files.sort();
        let mut before: Option<Day> = None;
        for f in &files {
            let now = load(&f.to_string_lossy())?;
            if let Some(a) = &before {
                match next_day(a, &now) {
                    Ok(()) => println!("{} -> {}: {}", a.mil.date, now.mil.date, one_line(&check_pair(&day, &w, a, &now))),
                    Err(e) => println!("{} -> {}: skipped, {e}", a.mil.date, now.mil.date),
                }
            }
            before = Some(now);
        }
    } else {
        let (a, b) = (load(args.save.as_ref().ok_or(usage)?)?, load(positional.first().ok_or(usage)?)?);
        next_day(&a, &b)?;
        let pair = check_pair(&day, &w, &a, &b);
        println!("{} -> {}: {} land battles in both saves, each fought one day in the port on its own", a.mil.date, b.mil.date, pair.checks.len());
        println!("per regiment: the same = within the save's {SAVE_STEP}; within 1% = of the regiment's value the day before");
        for x in &pair.checks {
            print_check(&day, x, if args.all_warnings { usize::MAX } else { 6 });
        }
        println!();
        print_others(&pair);
    }
    println!("what the port does not have for a battle taken from a save (differences can come from these):");
    println!("  which leader a side fights with: the one whose attack and defence add up highest among its armies' leaders is taken (the original picks by a value not identified yet)");
    println!("  gas: from the inventions of the install in the order the save numbers them, or for the countries named with --gas");
    println!("  river crossing: a border counts as a river border from the river pixels lying on it (a stand-in, how the original marks one is not known)");
    println!("  unit statistics, combat width and tactics: the unit files plus the country's technologies and inventions (inventions by the save's numbers in the install's order; no other country modifiers)");
    println!("  whatever else the game does to a regiment that day (reinforcement, attrition, supply) is not run here");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u64, side: BattleSide, before: [f64; 2], game: Option<[f64; 2]>, port: [f64; 2]) -> Row {
        Row { id, side, kind: "foot".into(), before, game, port }
    }

    #[test]
    fn regiments_are_graded_and_losses_summed_per_side() {
        let (a, d) = (BattleSide::Attacker, BattleSide::Defender);
        let rows = vec![
            // Strength the same to the save's step, organisation within 1% of 30.
            row(1, a, [3.0, 30.0], Some([2.9, 28.0]), [2.9004, 28.2]),
            // Both different.
            row(2, a, [3.0, 30.0], Some([2.5, 20.0]), [3.0, 30.0]),
            // Gone in the game: in no count.
            row(3, a, [1.0, 10.0], None, [0.5, 5.0]),
            row(4, d, [2.0, 20.0], Some([1.99, 20.0]), [1.98, 20.0]),
        ];
        assert_eq!(tally(&rows, 0), Tally { same: 1, close: 1, off: 1 });
        assert_eq!(tally(&rows, 1), Tally { same: 1, close: 1, off: 1 });
        let (g, p) = lost(&rows, a, 0);
        assert!((g - 0.6).abs() < 1e-9 && (p - 0.0996).abs() < 1e-9, "{g} {p}");
        let (g, p) = lost(&rows, d, 0);
        assert!((g - 0.01).abs() < 1e-9 && (p - 0.02).abs() < 1e-9, "{g} {p}");
        assert_eq!(hit(&rows, a), (2, 1, 1));
        assert_eq!(share(0.5, 1.0), "50%");
        assert_eq!(share(0.5, 0.0), "-");
    }

    #[test]
    fn lines_are_compared_slot_by_slot() {
        let game = BTreeMap::from([(15, 7), (16, 8), (17, 9)]);
        let port = BTreeMap::from([(15, 7), (16, 9), (18, 8)]);
        let (same, all, differ) = line_diff(&game, &port);
        assert_eq!((same, all), (1, 4));
        assert_eq!(differ, vec!["slot 16: game 8, port 9", "slot 17: game 9, port empty", "slot 18: game empty, port 8"]);
    }
}
