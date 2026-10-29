//! The Diplomacy window's overview (apps/web/API.md: Diplomacy window): the great powers, every
//! running war, the crisis, and one row per country with its relation to the seat, the great powers'
//! influence and opinion, and what the selected-country panel shows. It is added to the answer of
//! `GET /api/diplomacy` (play.rs keeps the seat's casus belli, wars, peace and questions).

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_simulation::game::Game;

use super::{country_name, json_int, json_list, json_opt, json_opt_str, json_str, play, rank_order, Shared};

/// One great power's card.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct Power {
    pub tag: String,
    pub name: String,
    pub colour: [u8; 3],
    pub rank: Option<usize>,
    /// (prestige, industry, military)
    pub scores: [Option<f64>; 3],
    pub sphere: usize,
    /// Its relation with the seat, its opinion of the seat, its influence in the seat.
    pub relation: Option<f64>,
    pub opinion: Option<u8>,
    pub influence: Option<f64>,
}

/// One row of the country table and the selected-country panel.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct Nation {
    pub tag: String,
    pub name: String,
    pub colour: [u8; 3],
    pub rank: Option<usize>,
    /// "great", "secondary", "civilised" or "uncivilised".
    pub status: &'static str,
    pub continent: Option<String>,
    pub neighbour: bool,
    pub sphere_of: Option<String>,
    pub overlord: Option<String>,
    pub government: Option<String>,
    pub population: f64,
    pub cultures: Vec<String>,
    pub scores: [Option<f64>; 3],
    /// The seat's opinion of it (a great power's level, 0 hostile .. 5 in sphere).
    pub opinion: Option<u8>,
    /// Influence and opinion of each great power, in the order of `powers`.
    pub influence: Vec<Option<f64>>,
    pub levels: Vec<Option<u8>>,
    pub access: bool,
    pub guaranteed: bool,
}

/// A running war, seen by everybody.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct WarRow {
    pub name: String,
    pub attackers: Vec<String>,
    pub defenders: Vec<String>,
    /// From the attackers' side, -100..100.
    pub score: Option<f64>,
    /// "attacker", "defender" or none: the seat's side.
    pub side: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct Overview {
    pub seat_great_power: bool,
    pub sphere_of_seat: Option<String>,
    pub powers: Vec<Power>,
    pub nations: Vec<Nation>,
    pub wars: Vec<WarRow>,
    pub continents: Vec<String>,
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn opt_u8(v: Option<u8>) -> String {
    v.map_or("null".into(), |x| x.to_string())
}

fn opt_rank(v: Option<usize>) -> String {
    v.map_or("null".into(), |x| x.to_string())
}

fn scores_json(s: &[Option<f64>; 3]) -> String {
    let total = if s.iter().all(Option::is_some) { Some(s.iter().flatten().sum()) } else { None };
    format!("\"prestige\":{},\"industry\":{},\"military\":{},\"total\":{}", json_opt(s[0]), json_opt(s[1]), json_opt(s[2]), json_opt(total))
}

/// The members `GET /api/diplomacy` gets from this module, without the braces.
pub(super) fn overview_members(o: &Overview) -> String {
    let powers: Vec<String> = o
        .powers
        .iter()
        .map(|p| {
            format!(
                "{{\"tag\":{},\"name\":{},\"colour\":{},\"rank\":{},{},\"sphere\":{},\"relation\":{},\"opinion\":{},\"influence\":{}}}",
                json_str(&p.tag),
                json_str(&p.name),
                json_str(&hex(p.colour)),
                opt_rank(p.rank),
                scores_json(&p.scores),
                p.sphere,
                json_opt(p.relation),
                opt_u8(p.opinion),
                json_opt(p.influence)
            )
        })
        .collect();
    let nations: Vec<String> = o
        .nations
        .iter()
        .map(|n| {
            format!(
                "{{\"tag\":{},\"name\":{},\"colour\":{},\"rank\":{},\"status\":\"{}\",\"continent\":{},\"neighbour\":{},\"sphere_of\":{},\"overlord\":{},\"government\":{},\"population\":{},\"cultures\":{},{},\"opinion\":{},\"influence\":[{}],\"levels\":[{}],\"access\":{},\"guaranteed\":{}}}",
                json_str(&n.tag),
                json_str(&n.name),
                json_str(&hex(n.colour)),
                opt_rank(n.rank),
                n.status,
                json_opt_str(n.continent.as_deref()),
                n.neighbour,
                json_opt_str(n.sphere_of.as_deref()),
                json_opt_str(n.overlord.as_deref()),
                json_opt_str(n.government.as_deref()),
                json_int(n.population),
                json_list(&n.cultures),
                scores_json(&n.scores),
                opt_u8(n.opinion),
                n.influence.iter().map(|v| json_opt(*v)).collect::<Vec<_>>().join(","),
                n.levels.iter().map(|v| opt_u8(*v)).collect::<Vec<_>>().join(","),
                n.access,
                n.guaranteed
            )
        })
        .collect();
    let wars: Vec<String> = o
        .wars
        .iter()
        .map(|w| format!("{{\"name\":{},\"attackers\":{},\"defenders\":{},\"score\":{},\"side\":{}}}", json_str(&w.name), json_list(&w.attackers), json_list(&w.defenders), json_opt(w.score), json_opt_str(w.side)))
        .collect();
    format!(
        "\"seat_great_power\":{},\"sphere_of_seat\":{},\"powers\":[{}],\"nations\":[{}],\"all_wars\":[{}],\"crisis\":null,\"continents\":{}",
        o.seat_great_power,
        json_opt_str(o.sphere_of_seat.as_deref()),
        powers.join(","),
        nations.join(","),
        wars.join(","),
        json_list(&o.continents)
    )
}

/// Adds `members` to the JSON object `base` (`null` stays `null`).
fn merged(base: String, members: &str) -> String {
    match base.strip_suffix('}') {
        Some(head) if head.starts_with('{') => format!("{head},{members}}}"),
        _ => base,
    }
}

/// What the overview reads from the running game.
pub(super) fn overview(sh: &Shared, g: &Game, seat: &str) -> Overview {
    let s = &g.state;
    let rebels = &g.core.day.rebel_tag;
    let dip = g.core.dip.as_ref().map(|(_, d)| d);
    let colour = |t: &str| g.world.tag_index.get(t).map_or([128, 128, 128], |i| g.world.country(*i).country.color);
    let order = rank_order(sh, s);
    let rank = |t: &str| order.iter().position(|x| x == t).map(|i| i + 1);
    // Scores: yesterday's, or those worked out at the start until a day has run.
    let scores = |t: &str| -> [Option<f64>; 3] {
        let Some(c) = s.countries.get(t) else { return [None; 3] };
        if s.rank_order.is_empty() {
            sh.first_scores.get(t).map_or([None; 3], |x| [Some(x[2]), Some(x[0]), Some(x[1])])
        } else {
            [Some(c.prestige), Some(s.industrial_score(t)), Some(c.military_score)]
        }
    };
    // One pass over the provinces: owners, people, the seat's neighbours.
    let mut people: BTreeMap<&str, f64> = BTreeMap::new();
    let mut neighbours: BTreeSet<&str> = BTreeSet::new();
    for (id, p) in &s.provinces {
        let Some(owner) = p.owner.as_deref() else { continue };
        *people.entry(owner).or_default() += p.pops.iter().map(|q| q.size).sum::<f64>();
        if owner == seat {
            for n in s.neighbours.get(id).into_iter().flatten() {
                if let Some(b) = s.provinces.get(n).and_then(|q| q.owner.as_deref()).filter(|b| *b != seat) {
                    neighbours.insert(b);
                }
            }
        }
    }
    let great: Vec<&str> = if s.great_powers.is_empty() { dip.map(|d| d.great_powers().collect()).unwrap_or_default() } else { s.great_powers.iter().map(String::as_str).collect() };
    // Influence and opinion: the diplomacy state when it runs, else the save's numbers.
    let influence = |gp: &str, t: &str| -> Option<f64> {
        match dip {
            Some(d) => Some(d.influence(gp, t)),
            None => s.countries.get(gp).map(|c| c.influence.get(t).copied().unwrap_or(0.0)),
        }
    };
    let opinion = |from: &str, t: &str| dip.map(|d| d.level(from, t));
    let sphere_of = |t: &str| -> Option<String> {
        match dip {
            Some(d) => d.sphere_leader(t).map(str::to_string),
            None => great.iter().find(|gp| s.countries.get(**gp).is_some_and(|c| c.sphere.contains(t))).map(|x| x.to_string()),
        }
    };
    let relation = |t: &str| dip.map(|d| d.relation_value(seat, t)).or_else(|| s.countries.get(seat).and_then(|c| c.relations.get(t).copied()));
    let seat_great = great.contains(&seat);
    let powers: Vec<Power> = great
        .iter()
        .map(|gp| Power {
            tag: gp.to_string(),
            name: country_name(g, gp),
            colour: colour(gp),
            rank: rank(gp),
            scores: scores(gp),
            sphere: match dip {
                Some(d) => d.sphere_members(gp).count(),
                None => s.countries.get(*gp).map_or(0, |c| c.sphere.len()),
            },
            relation: if *gp == seat { None } else { relation(gp) },
            opinion: if *gp == seat { None } else { opinion(gp, seat) },
            influence: if *gp == seat { None } else { influence(gp, seat) },
        })
        .collect();
    let continent = |t: &str| s.countries.get(t).and_then(|c| c.capital).and_then(|p| s.provinces.get(&p)).and_then(|p| p.continent.as_deref()).map(|k| sh.name(k));
    let mut continents: BTreeSet<String> = BTreeSet::new();
    let nations: Vec<Nation> = people
        .keys()
        .filter(|t| **t != seat && *t != rebels)
        .map(|t| {
            let c = s.countries.get(*t);
            let status = match c {
                Some(c) if !c.civilized => "uncivilised",
                None => "uncivilised",
                _ if great.contains(t) => "great",
                _ if rank(t).is_some_and(|r| r as f64 <= g.econ.rank_rules.colonial_rank) => "secondary",
                _ => "civilised",
            };
            let cont = continent(t);
            if let Some(x) = &cont {
                continents.insert(x.clone());
            }
            let cultures = c.map(|c| c.primary_culture.iter().chain(&c.accepted_cultures).map(|k| sh.name(k)).collect()).unwrap_or_default();
            Nation {
                tag: t.to_string(),
                name: country_name(g, t),
                colour: colour(t),
                rank: rank(t),
                status,
                continent: cont,
                neighbour: neighbours.contains(t),
                sphere_of: sphere_of(t),
                overlord: dip.and_then(|d| d.overlord_of(t)).map(str::to_string),
                government: c.and_then(|c| c.government.as_deref()).map(|k| sh.name(k)),
                population: people.get(t).copied().unwrap_or(0.0),
                cultures,
                scores: scores(t),
                opinion: if seat_great { opinion(seat, t) } else { None },
                influence: great.iter().map(|gp| if gp == t { None } else { influence(gp, t) }).collect(),
                levels: great.iter().map(|gp| if gp == t { None } else { opinion(gp, t) }).collect(),
                access: dip.is_some_and(|d| d.has_access(seat, t)),
                guaranteed: dip.is_some_and(|d| d.guaranteed_by(seat, t) || d.guaranteed_by(t, seat)),
            }
        })
        .collect();
    let war_scores: BTreeMap<String, f64> = g.core.day.war_scores(&g.core.mil, s).into_iter().collect();
    let wars = g
        .core
        .mil
        .wars
        .iter()
        .filter(|w| w.active && !w.attackers.is_empty() && !w.defenders.is_empty())
        .map(|w| WarRow {
            name: w.name.clone(),
            attackers: w.attackers.clone(),
            defenders: w.defenders.clone(),
            score: war_scores.get(&w.name).copied(),
            side: if w.attackers.iter().any(|t| t == seat) {
                Some("attacker")
            } else if w.defenders.iter().any(|t| t == seat) {
                Some("defender")
            } else {
                None
            },
        })
        .collect();
    Overview { seat_great_power: seat_great, sphere_of_seat: sphere_of(seat), powers, nations, wars, continents: continents.into_iter().collect() }
}

/// `GET /api/diplomacy`: play.rs's answer with the overview added.
pub(super) fn diplomacy_json(sh: &Shared, g: &Game, seat: &str) -> String {
    merged(play::diplomacy_json(sh, g, seat), &overview_members(&overview(sh, g, seat)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiktoria_simulation::diplomacy::relations::level;

    fn parse(s: &str) -> bool {
        // A light check that the text is balanced JSON-ish: braces and brackets pair up outside strings.
        let (mut depth, mut inside, mut escaped) = (0i32, false, false);
        for c in s.chars() {
            match (inside, escaped, c) {
                (true, true, _) => escaped = false,
                (true, false, '\\') => escaped = true,
                (_, false, '"') => inside = !inside,
                (false, _, '{' | '[') => depth += 1,
                (false, _, '}' | ']') => depth -= 1,
                _ => {}
            }
            if depth < 0 {
                return false;
            }
        }
        depth == 0 && !inside
    }

    #[test]
    fn the_overview_is_written_as_json_members() {
        let o = Overview {
            seat_great_power: true,
            sphere_of_seat: None,
            powers: vec![Power { tag: "AAA".into(), name: "Country A".into(), colour: [200, 16, 1], rank: Some(1), scores: [Some(50.0), Some(120.5), Some(30.0)], sphere: 2, relation: None, opinion: None, influence: None }],
            nations: vec![Nation {
                tag: "BBB".into(),
                name: "Country \"B\"".into(),
                colour: [0, 0, 255],
                rank: None,
                status: "uncivilised",
                continent: Some("Continent One".into()),
                neighbour: true,
                sphere_of: Some("AAA".into()),
                population: 1234.4,
                cultures: vec!["Culture One".into()],
                scores: [Some(1.0), None, Some(2.0)],
                opinion: Some(level::IN_SPHERE),
                influence: vec![Some(12.5)],
                levels: vec![Some(5)],
                ..Default::default()
            }],
            wars: vec![WarRow { name: "War One".into(), attackers: vec!["AAA".into()], defenders: vec!["BBB".into()], score: Some(-12.0), side: Some("attacker") }],
            continents: vec!["Continent One".into()],
        };
        let m = overview_members(&o);
        let whole = merged("{\"tag\":\"AAA\"}".into(), &m);
        assert!(parse(&whole), "{whole}");
        assert!(whole.starts_with("{\"tag\":\"AAA\",\"seat_great_power\":true,"));
        assert!(m.contains("\"colour\":\"#c81001\""), "colours as #rrggbb");
        assert!(m.contains("\"prestige\":50.0,\"industry\":120.5,\"military\":30.0,\"total\":200.5"));
        assert!(m.contains("\"military\":2.0,\"total\":null"), "a missing score leaves the total a dash");
        assert!(m.contains("\"name\":\"Country \\\"B\\\"\""));
        assert!(m.contains("\"population\":1234,"));
        assert!(m.contains("\"influence\":[12.5],\"levels\":[5]"));
        assert!(m.contains("\"score\":-12.0,\"side\":\"attacker\""));
        assert!(m.contains("\"crisis\":null"));
        assert_eq!(merged("null".into(), &m), "null", "no seat: still null");
    }
}
