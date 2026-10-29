//! `wiktoria-cli check-occupation --dir <folder of dated saves of one game> [--every N] [--ai-off LIST]`:
//! the port is started from the first save and run to the dates of the later ones; at each of those
//! dates every war that ran in the first save is printed twice, as the original's save has it and as
//! the port has it: war score (the port's own sum, applied to both), provinces each side holds of the
//! other, battles fought and men lost in them since the first save, and men under arms.
//! It answers "does a war go as fast here as there". A war the original's player took part in is
//! marked: there a person moved the armies.

use std::collections::{BTreeMap, HashSet};

use wiktoria_data::date::Date;
use wiktoria_simulation::game::Game;
use wiktoria_simulation::military::ai::AiOff;
use wiktoria_simulation::military::day::MilitaryDay;
use wiktoria_simulation::military::war::War;
use wiktoria_simulation::military::{MilitaryRules, MilitaryState};
use wiktoria_simulation::script::State;

use crate::{config, load_world, root, Args};

/// One war on one day, from one of the two games.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WarPicture {
    pub active: bool,
    pub ended: Option<Date>,
    pub score: f64,
    /// Provinces of the defenders the attackers hold, and how many the defenders own; then the other way round.
    pub attackers_hold: (usize, usize),
    pub defenders_hold: (usize, usize),
    /// Battles in the war's history, and the men lost in them by the war's attackers and defenders.
    pub battles: usize,
    pub lost: [f64; 2],
    /// Men in the armies of each side.
    pub men: [f64; 2],
}

/// The whole map on one day.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MapPicture {
    pub occupied: usize,
    pub rebel_held: usize,
    pub wars: usize,
}

/// The countries of a war as the first save has them: a war that ended has none left in its own list.
#[derive(Debug, Clone, PartialEq)]
pub struct Sides {
    pub name: String,
    pub attackers: Vec<String>,
    pub defenders: Vec<String>,
}

pub fn war_picture(mil: &MilitaryState, men_per_strength: f64, sides: &Sides, score: Option<f64>) -> WarPicture {
    let Some(war) = mil.wars.iter().find(|w| w.name == sides.name) else { return WarPicture::default() };
    let (att, def) = if war.active { (&war.attackers, &war.defenders) } else { (&sides.attackers, &sides.defenders) };
    let held = |of: &[String], by: &[String]| {
        let own = mil.provinces.values().filter(|p| p.owner.as_ref().is_some_and(|o| of.contains(o)));
        (own.clone().filter(|p| p.controller.as_ref().is_some_and(|c| by.contains(c))).count(), own.count())
    };
    let men = |of: &[String]| mil.armies.iter().filter(|a| of.contains(&a.owner)).flat_map(|a| &a.regiments).map(|g| g.strength * men_per_strength).sum::<f64>();
    let mut lost = [0.0; 2];
    for b in war.battles() {
        for s in [&b.attacker, &b.defender] {
            // A battle's attacker is not the war's: each loss goes to the war side its country is on.
            let k = if sides.attackers.contains(&s.country) || att.contains(&s.country) { 0 } else { 1 };
            lost[k] += s.losses;
        }
    }
    WarPicture { active: war.active, ended: war.ended(), score: score.unwrap_or(0.0), attackers_hold: held(def, att), defenders_hold: held(att, def), battles: war.battles().count(), lost, men: [men(att), men(def)] }
}

pub fn map_picture(mil: &MilitaryState, rebel_tag: &str) -> MapPicture {
    MapPicture { occupied: mil.occupied().count(), rebel_held: mil.provinces.values().filter(|p| p.owner.is_some() && p.controller.as_deref() == Some(rebel_tag)).count(), wars: mil.wars.iter().filter(|w| w.active).count() }
}

fn sides_of(w: &War) -> Sides {
    Sides { name: w.name.clone(), attackers: w.attackers.clone(), defenders: w.defenders.clone() }
}

fn k(x: f64) -> String {
    format!("{:.0}k", x / 1000.0)
}

/// One line: the war as one game has it, battles and losses counted from the first save on.
pub fn line(p: &WarPicture, start: &WarPicture) -> String {
    let state = match (p.active, p.ended) {
        (true, _) => format!("score {:+.0}", p.score),
        (false, Some(d)) => format!("over since {d}"),
        (false, None) => "over".to_string(),
    };
    format!(
        "{state:<22} attackers hold {:>3} of {:<4} defenders hold {:>3} of {:<4} battles {:>3}, lost {:>5} / {:<5} under arms {:>6} / {}",
        p.attackers_hold.0,
        p.attackers_hold.1,
        p.defenders_hold.0,
        p.defenders_hold.1,
        p.battles.saturating_sub(start.battles),
        k(p.lost[0] - start.lost[0]),
        k(p.lost[1] - start.lost[1]),
        k(p.men[0]),
        k(p.men[1])
    )
}

pub fn check_occupation(args: &Args) -> Result<(), String> {
    let usage = "usage: check-occupation --dir <folder of dated saves of one game> [--every N] [--ai-off LIST]";
    let pos = &args.positional;
    let after = |flag: &str| pos.iter().position(|a| a == flag).map(|i| pos.get(i + 1).ok_or(usage)).transpose();
    let dir = after("--dir")?.ok_or(usage)?;
    let every: usize = after("--every")?.map(|n| n.parse().map_err(|_| "--every needs a number")).transpose()?.unwrap_or(10).max(1);
    let off = after("--ai-off")?.map(|l| AiOff::parse(l)).transpose()?.unwrap_or_default();
    let mut files: Vec<_> = std::fs::read_dir(dir).map_err(|e| format!("cannot read {dir}: {e}"))?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "v2")).collect();
    files.sort();
    if files.len() < 2 {
        return Err(format!("{dir} holds {} saves; two or more are needed", files.len()));
    }
    let w = load_world(args)?;
    let engine_rules = config::load_engine_rules(&root())?;
    let install = config::install_dir(&root(), args.install.clone())?;
    let rules = MilitaryRules::new(&install, &engine_rules)?;
    let pop_types: HashSet<String> = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures: HashSet<String> = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let parse = |path: &std::path::Path| -> Result<wiktoria_data::save::SaveGame, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)
    };
    let first = parse(&files[0])?;
    let player = first.player.clone();
    let mut game = Game::new(w, &engine_rules, &install, &first)?;
    game.core.ai.off = off;
    let start_date = game.state.date;
    let sides: Vec<Sides> = game.core.mil.wars.iter().filter(|w| w.active).map(sides_of).collect();
    let picture = |day: &MilitaryDay, mil: &MilitaryState, state: &State| -> BTreeMap<String, WarPicture> {
        let scores = day.war_scores(mil, state);
        sides.iter().map(|s| (s.name.clone(), war_picture(mil, rules.men_per_strength, s, scores.iter().find(|(n, _)| *n == s.name).map(|x| x.1)))).collect()
    };
    let start = picture(&game.core.day, &game.core.mil, &game.state);
    let names = off.names_off();
    println!("{dir}: {} saves from {start_date}; {} wars running; army rules off: {}", files.len(), sides.len(), if names.is_empty() { "none".to_string() } else { names.join(", ") });
    println!("per war: who holds how many of the other side's provinces, battles and men lost in them since {start_date} (attackers / defenders), men under arms");
    if let Some(p) = &player {
        println!("a person played {p} in the original: its wars are marked, there the comparison is with a person's moves");
    }
    let mut picks: Vec<usize> = (every..files.len()).step_by(every).collect();
    if picks.last() != Some(&(files.len() - 1)) {
        picks.push(files.len() - 1);
    }
    let mut day_no = 0;
    for i in picks {
        let save = parse(&files[i])?;
        let mut state = State::from_save(&save);
        state.attach_world(&game.world, &save);
        let mil = MilitaryState::from_save(&save, &rules);
        if mil.date <= game.state.date {
            println!("{}: skipped, not later than the port's day {}", mil.date, game.state.date);
            continue;
        }
        let mut steps = 0;
        while game.state.date < mil.date {
            game.day();
            steps += 1;
            day_no += 1;
            if steps > 4000 {
                return Err(format!("the port did not reach {} in 4000 days", mil.date));
            }
        }
        let (theirs, ours) = (picture(&game.core.day, &mil, &state), picture(&game.core.day, &game.core.mil, &game.state));
        let (tm, om) = (map_picture(&mil, &rules.rebel_tag), map_picture(&game.core.mil, &rules.rebel_tag));
        println!();
        println!("{} (day {}): provinces under another country's control {} original / {} port, of them held by rebels {} / {}; wars running {} / {}", mil.date, day_no, tm.occupied, om.occupied, tm.rebel_held, om.rebel_held, tm.wars, om.wars);
        for s in &sides {
            let person = player.as_ref().is_some_and(|p| s.attackers.contains(p) || s.defenders.contains(p));
            println!("  {}: {} against {}{}", s.name, s.attackers.join("+"), s.defenders.join("+"), if person { "  (a person played in the original)" } else { "" });
            println!("    original  {}", line(&theirs[&s.name], &start[&s.name]));
            println!("    port      {}", line(&ours[&s.name], &start[&s.name]));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiktoria_simulation::military::siege::ProvinceMilitary;
    use wiktoria_simulation::military::units::{Army, Regiment};
    use wiktoria_simulation::military::war::{BattleRecord, BattleSide, WarEvent, WarEventKind};

    fn province(owner: &str, controller: &str) -> ProvinceMilitary {
        ProvinceMilitary { owner: Some(owner.to_string()), controller: Some(controller.to_string()), ..Default::default() }
    }

    fn battle(day: u8, a: (&str, f64), d: (&str, f64)) -> WarEvent {
        let date = Date { year: 1850, month: 1, day };
        let side = |x: (&str, f64)| BattleSide { country: x.0.to_string(), losses: x.1, ..Default::default() };
        WarEvent { date, kind: WarEventKind::Battle(BattleRecord { date: Some(date), name: String::new(), location: 1, attacker_won: true, attacker: side(a), defender: side(d) }) }
    }

    fn world() -> (MilitaryState, Sides) {
        let mut mil = MilitaryState { date: Date { year: 1850, month: 1, day: 20 }, countries: BTreeMap::new(), leaders: vec![], armies: vec![], navies: vec![], constructions: vec![], wars: vec![], combats: vec![], provinces: BTreeMap::new(), warnings: vec![] };
        for (id, o, c) in [(1, "AAA", "AAA"), (2, "AAA", "BBB"), (3, "BBB", "AAA"), (4, "BBB", "AAA"), (5, "BBB", "REB"), (6, "CCC", "REB")] {
            mil.provinces.insert(id, province(o, c));
        }
        mil.armies.push(Army { owner: "AAA".into(), regiments: vec![Regiment { strength: 3.0, ..Default::default() }, Regiment { strength: 1.5, ..Default::default() }], ..Default::default() });
        mil.armies.push(Army { owner: "BBB".into(), regiments: vec![Regiment { strength: 2.0, ..Default::default() }], ..Default::default() });
        let war = War { name: "the war".into(), active: true, attackers: vec!["AAA".into()], defenders: vec!["BBB".into()], history: vec![battle(3, ("AAA", 100.0), ("BBB", 300.0)), battle(9, ("BBB", 50.0), ("AAA", 20.0))], ..Default::default() };
        let sides = sides_of(&war);
        mil.wars.push(war);
        (mil, sides)
    }

    #[test]
    fn a_war_is_counted_by_its_sides_not_by_who_attacked_in_a_battle() {
        let (mil, sides) = world();
        let p = war_picture(&mil, 1000.0, &sides, Some(12.0));
        assert_eq!((p.attackers_hold, p.defenders_hold), ((2, 3), (1, 2)));
        assert_eq!((p.battles, p.lost), (2, [120.0, 350.0]));
        assert_eq!(p.men, [4500.0, 2000.0]);
        assert_eq!(map_picture(&mil, "REB"), MapPicture { occupied: 5, rebel_held: 2, wars: 1 });
    }

    #[test]
    fn a_finished_war_keeps_the_sides_of_the_first_save_and_counts_from_there() {
        let (mut mil, sides) = world();
        let start = war_picture(&mil, 1000.0, &sides, None);
        let w = &mut mil.wars[0];
        w.history.push(battle(15, ("AAA", 10.0), ("BBB", 40.0)));
        w.history.push(WarEvent { date: Date { year: 1850, month: 1, day: 18 }, kind: WarEventKind::RemDefender("BBB".into()) });
        (w.active, w.attackers, w.defenders) = (false, vec![], vec![]);
        let p = war_picture(&mil, 1000.0, &sides, None);
        assert_eq!((p.attackers_hold, p.lost), ((2, 3), [130.0, 390.0]));
        let text = line(&p, &start);
        assert!(text.starts_with("over since 1850.1.18"), "{text}");
        assert!(text.contains("battles   1, lost    0k / 0k"), "{text}");
    }
}
