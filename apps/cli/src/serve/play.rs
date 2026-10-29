//! The answers and orders of the second slice of play (apps/web/API.md: Events and decisions,
//! Diplomacy): held events, open decisions, casus belli, wars, peace and the computer's questions.

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_simulation::game::Game;
use wiktoria_simulation::military::war::{self, PeaceRequest};
use wiktoria_simulation::player::{AskKind, Order};
use wiktoria_simulation::script::Scope;

use super::{country_name, json_list, json_num, json_opt, json_opt_str, json_str, param, plain, province_name, Shared};

/// A localised text, or `None` when the key has none.
fn text_of(sh: &Shared, key: &str) -> Option<String> {
    sh.loc.get(key).filter(|s| !s.is_empty()).map(str::to_string)
}

/// The orders of this slice from a POST's path and query; `None` for another path.
pub(super) fn order_of(path: &str, q: &[(String, String)]) -> Option<Result<Order, String>> {
    let need = |k: &str| param(q, k).map(str::trim).filter(|v| !v.is_empty()).ok_or(format!("{k} missing"));
    let number = |k: &str| need(k).and_then(|v| v.parse::<u64>().map_err(|_| format!("{k} must be a number")));
    let order = || -> Result<Order, String> {
        Ok(match path {
            "/api/event" => Order::ChooseEvent { key: number("key")?, option: number("option")? as usize },
            "/api/decision" => Order::Decide { name: need("name")?.to_string() },
            "/api/justify" => Order::Justify { target: need("target")?.to_uppercase(), cb: need("cb")?.to_string() },
            "/api/war/declare" => Order::DeclareWar { target: need("target")?.to_uppercase(), cb: need("cb")?.to_string() },
            "/api/peace/offer" => {
                let goals = param(q, "goals").unwrap_or("").split(',').map(str::trim).filter(|g| !g.is_empty()).map(|g| g.parse::<usize>().map_err(|_| "goals must be a list of numbers".to_string())).collect::<Result<Vec<_>, _>>()?;
                Order::OfferPeace { war: need("war")?.to_string(), to: need("to")?.to_uppercase(), goals }
            }
            "/api/answer" => {
                let accept = match need("accept")? {
                    "true" | "1" | "yes" => true,
                    "false" | "0" | "no" => false,
                    _ => return Err("accept must be true or false".into()),
                };
                Order::Answer { key: number("key")?, accept }
            }
            _ => unreachable!(),
        })
    };
    PATHS.contains(&path).then(order)
}

/// The order paths of this slice.
pub(super) const PATHS: [&str; 6] = ["/api/event", "/api/decision", "/api/justify", "/api/war/declare", "/api/peace/offer", "/api/answer"];

/// Whether a carried-out order goes into the seat's message log.
pub(super) fn logged(o: &Order) -> bool {
    matches!(o, Order::DeclareWar { .. } | Order::OfferPeace { .. } | Order::Answer { .. } | Order::Decide { .. } | Order::Justify { .. })
}

/// The seat's held events and the decisions open to it (`GET /api/events`).
pub(super) fn events_json(sh: &Shared, g: &mut Game, tag: &str) -> String {
    let runner = &g.econ.runner;
    let pending: Vec<String> = runner
        .held
        .iter()
        .filter(|h| h.tag == tag)
        .map(|h| {
            let e = runner.events.iter().find(|e| e.id == h.id);
            let title = e.and_then(|e| text_of(sh, &e.title)).unwrap_or_else(|| format!("Event {}", h.id));
            let desc = e.and_then(|e| text_of(sh, &e.desc));
            let names = e.map(wiktoria_simulation::events::option_names).unwrap_or_default();
            let options: Vec<String> = names.iter().enumerate().map(|(i, n)| format!("{{\"index\":{i},\"name\":{}}}", json_str(&text_of(sh, n).unwrap_or_else(|| format!("Option {}", i + 1))))).collect();
            let province = match &h.target {
                Scope::Province(p) | Scope::Pop(p, _) => format!("{{\"id\":{p},\"name\":{}}}", json_str(&province_name(g, *p))),
                _ => "null".into(),
            };
            format!(
                "{{\"key\":{},\"id\":{},\"title\":{},\"desc\":{},\"province\":{province},\"fired\":{},\"expires\":{},\"options\":[{}],\"ai_option\":{}}}",
                h.key,
                h.id,
                json_str(&title),
                json_opt_str(desc.as_deref()),
                json_str(&h.fired.to_string()),
                json_str(&h.expires.to_string()),
                options.join(","),
                h.ai_pick
            )
        })
        .collect();
    let played = wiktoria_simulation::player::is_player(&g.state, tag);
    // Decisions are listed for a played country only: the computer takes its own at once.
    let decisions: Vec<String> = if played {
        let g = &mut *g;
        g.econ
            .runner
            .open_decisions(&mut g.state, tag)
            .into_iter()
            .map(|(d, allowed)| {
                let title = text_of(sh, &format!("{}_title", d.name)).unwrap_or_else(|| plain(&d.name));
                format!("{{\"key\":{},\"title\":{},\"desc\":{},\"allowed\":{allowed}}}", json_str(&d.name), json_str(&title), json_opt_str(text_of(sh, &format!("{}_desc", d.name)).as_deref()))
            })
            .collect()
    } else {
        Vec::new()
    };
    format!("{{\"tag\":{},\"played\":{played},\"pending\":[{}],\"decisions\":[{}]}}", json_str(tag), pending.join(","), decisions.join(","))
}

/// The seat's diplomacy: other countries, its casus belli, its wars and the questions it has to
/// answer (`GET /api/diplomacy`).
pub(super) fn diplomacy_json(sh: &Shared, g: &Game, tag: &str) -> String {
    let (s, core) = (&g.state, &g.core);
    let Some(c) = s.countries.get(tag) else { return "null".into() };
    let wr = &core.day.war;
    let dip = core.dip.as_ref().map(|(_, d)| d);
    let cb_name = |k: &str| text_of(sh, k).unwrap_or_else(|| plain(k));
    let today = s.date;
    let justifying = core.justifying.iter().find(|((a, _), _)| a == tag).map_or("null".into(), |((_, t), (k, p, found))| {
        format!("{{\"target\":{},\"cb\":{},\"name\":{},\"progress\":{},\"discovered\":{found}}}", json_str(t), json_str(k), json_str(&cb_name(k)), json_num((p / wr.cb_target_progress.max(1e-9)).clamp(0.0, 1.0)))
    });
    let cbs: Vec<String> = core.justifiable_cbs().into_iter().map(|cb| format!("{{\"key\":{},\"name\":{},\"infamy\":{}}}", json_str(&cb.name), json_str(&cb_name(&cb.name)), json_num(war::goal_infamy(wr, cb, false)))).collect();
    let sides = wiktoria_simulation::military::ai::Sides::from_wars(&core.mil.wars, &core.day.rebel_tag);
    let owners: BTreeSet<&str> = s.provinces.values().filter_map(|p| p.owner.as_deref()).collect();
    let mut others: Vec<(&str, String)> = owners.iter().filter(|t| **t != tag && **t != core.day.rebel_tag).map(|t| (*t, country_name(g, t))).collect();
    others.sort_by(|a, b| a.1.cmp(&b.1));
    let countries: Vec<String> = others
        .iter()
        .map(|(t, name)| {
            let held: Vec<String> = core.held_cbs.get(&(tag.to_string(), t.to_string())).into_iter().flatten().map(|h| format!("{{\"cb\":{},\"name\":{},\"until\":{}}}", json_str(&h.cb_type), json_str(&cb_name(&h.cb_type)), json_str(&h.end.to_string()))).collect();
            format!(
                "{{\"tag\":{},\"name\":{},\"relation\":{},\"allied\":{},\"truce\":{},\"at_war\":{},\"infamy\":{},\"held\":[{}]}}",
                json_str(t),
                json_str(name),
                json_opt(dip.map(|d| d.relation_value(tag, t))),
                dip.is_some_and(|d| d.allied(tag, t)),
                dip.is_some_and(|d| d.truce_active(tag, t, today)),
                sides.at_war_with(tag, t),
                json_num(s.countries.get(*t).map_or(0.0, |x| x.badboy)),
                held.join(",")
            )
        })
        .collect();
    let scores: BTreeMap<String, f64> = core.day.war_scores(&core.mil, s).into_iter().collect();
    let wars: Vec<String> = core
        .mil
        .active_wars_of(tag)
        .map(|w| {
            let attacker = w.attackers.iter().any(|t| t == tag);
            let enemies = if attacker { &w.defenders } else { &w.attackers };
            let talk: Vec<&String> = enemies.iter().filter(|e| war::peace_allowed(&PeaceRequest { war: w, crisis: false, sender: tag, target: e, selected: &[] }, &core.day.cb_types).is_ok()).collect();
            let goals: Vec<String> = w
                .goals
                .iter()
                .enumerate()
                .map(|(i, x)| {
                    let ours = w.attackers.contains(&x.actor) == attacker;
                    let province = x.state_province.map_or("null".into(), |p| format!("{{\"id\":{p},\"name\":{}}}", json_str(&province_name(g, p))));
                    format!("{{\"index\":{i},\"cb\":{},\"name\":{},\"actor\":{},\"receiver\":{},\"province\":{province},\"ours\":{ours}}}", json_str(&x.casus_belli), json_str(&cb_name(&x.casus_belli)), json_str(&x.actor), json_str(&x.receiver))
                })
                .collect();
            let score = scores.get(&w.name).map(|x| if attacker { *x } else { -x });
            format!(
                "{{\"name\":{},\"attacker\":{attacker},\"leader\":{},\"attackers\":{},\"defenders\":{},\"score\":{},\"goals\":[{}],\"talk\":{}}}",
                json_str(&w.name),
                war::is_leader(w, tag),
                json_list(&w.attackers),
                json_list(&w.defenders),
                json_opt(score),
                goals.join(","),
                json_list(talk)
            )
        })
        .collect();
    let questions: Vec<String> = core
        .asks
        .of(tag)
        .map(|a| {
            let (kind, goals, attackers) = match &a.kind {
                AskKind::Peace { goals } => ("peace", goals.iter().map(|x| format!("{{\"cb\":{},\"name\":{},\"actor\":{},\"receiver\":{}}}", json_str(&x.casus_belli), json_str(&cb_name(&x.casus_belli)), json_str(&x.actor), json_str(&x.receiver))).collect::<Vec<_>>(), "null".to_string()),
                AskKind::Call { attackers } => ("call", Vec::new(), attackers.to_string()),
            };
            format!(
                "{{\"key\":{},\"kind\":\"{kind}\",\"from\":{},\"from_name\":{},\"war\":{},\"goals\":[{}],\"attackers\":{attackers},\"made\":{},\"expires\":{}}}",
                a.key,
                json_str(&a.from),
                json_str(&country_name(g, &a.from)),
                json_str(&a.war),
                goals.join(","),
                json_str(&a.made.to_string()),
                json_str(&a.expires.to_string())
            )
        })
        .collect();
    format!(
        "{{\"tag\":{},\"played\":{},\"infamy\":{},\"infamy_limit\":{},\"justifying\":{justifying},\"cbs\":[{}],\"countries\":[{}],\"wars\":[{}],\"questions\":[{}]}}",
        json_str(tag),
        c.human,
        json_num(c.badboy),
        json_num(wr.badboy_limit),
        cbs.join(","),
        countries.join(","),
        wars.join(","),
        questions.join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::parse_query;

    #[test]
    fn the_diplomacy_orders_are_read_from_the_query() {
        let o = |p: &str, q: &str| order_of(p, &parse_query(q));
        assert_eq!(o("/api/event", "key=4&option=1"), Some(Ok(Order::ChooseEvent { key: 4, option: 1 })));
        assert!(o("/api/event", "key=4").unwrap().is_err());
        assert_eq!(o("/api/decision", "name=form_germany"), Some(Ok(Order::Decide { name: "form_germany".into() })));
        assert_eq!(o("/api/justify", "target=fra&cb=conquest"), Some(Ok(Order::Justify { target: "FRA".into(), cb: "conquest".into() })));
        assert_eq!(o("/api/war/declare", "target=FRA&cb=conquest"), Some(Ok(Order::DeclareWar { target: "FRA".into(), cb: "conquest".into() })));
        assert_eq!(o("/api/peace/offer", "war=The+War&to=FRA&goals=0,2"), Some(Ok(Order::OfferPeace { war: "The War".into(), to: "FRA".into(), goals: vec![0, 2] })));
        assert_eq!(o("/api/peace/offer", "war=W&to=FRA"), Some(Ok(Order::OfferPeace { war: "W".into(), to: "FRA".into(), goals: vec![] })), "no goals: a white peace");
        assert!(o("/api/peace/offer", "war=W&to=FRA&goals=a").unwrap().is_err());
        assert_eq!(o("/api/answer", "key=3&accept=false"), Some(Ok(Order::Answer { key: 3, accept: false })));
        assert!(o("/api/answer", "key=3&accept=maybe").unwrap().is_err());
        assert_eq!(o("/api/budget", ""), None);
    }
}
