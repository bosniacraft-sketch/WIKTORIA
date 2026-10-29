//! The Production window's answer (apps/web/API.md: Production): the seat's states with their
//! factories, money invested from abroad, what the country makes per good, the building work
//! under way, and the factory types it could build. Read only: the engine has no player order to
//! build a factory yet, so `can_build` is false.
//!
//! What the engine does not have is sent as null (the page shows a dash): a factory's throttle
//! (the port has no such setting), and the output of a factory that is shut or not yet open.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::{Mutex, OnceLock};

use wiktoria_data::date::Date;
use wiktoria_simulation::game::Game;
use wiktoria_simulation::industry::Project;

use super::{country_name, json_int, json_num, json_opt, json_opt_str, json_str, lock, region_name, Shared};

/// The good groups of the goods grid, in the order shown.
pub(super) const GROUPS: [&str; 4] = ["military_goods", "raw_material_goods", "industrial_goods", "consumer_goods"];

pub(super) struct FactoryRow {
    pub index: usize,
    pub kind: String,
    pub name: String,
    pub good: Option<String>,
    pub good_name: Option<String>,
    pub level: f64,
    /// People at work today and the places it has (`None` while shut or not yet open).
    pub workers: Option<f64>,
    pub places: Option<f64>,
    pub output: Option<f64>,
    /// Yesterday's sales, inputs bought, pay to workers and owners (pounds).
    pub income: f64,
    pub inputs: f64,
    pub pay: f64,
    /// `building`, `opening` (a new one being built), `closed` or `running`.
    pub status: &'static str,
    pub days_left: Option<f64>,
    pub subsidised: bool,
    pub loss_days: u32,
}

pub(super) struct ProjectRow {
    pub state: String,
    pub kind: String,
    pub name: String,
    /// `new`, `expand` or `reopen`.
    pub what: &'static str,
    /// `state` (the government pays) or `capitalists`.
    pub by: &'static str,
    pub progress: Option<f64>,
    pub days_left: Option<f64>,
    pub price: Option<f64>,
    pub paid: Option<f64>,
    pub needs: Vec<(String, String, f64)>,
}

pub(super) struct StateRow {
    pub id: Option<u16>,
    pub name: String,
    pub provinces: usize,
    pub colony: bool,
    /// People of the factory worker types who live there.
    pub workforce: f64,
    pub investment: f64,
    pub factories: Vec<FactoryRow>,
}

pub(super) struct GoodRow {
    pub key: String,
    pub name: String,
    pub group: String,
    /// Its colour in the goods file, `#rrggbb`.
    pub colour: Option<[u8; 3]>,
    pub price: Option<f64>,
    pub rgo: f64,
    pub factories: f64,
    pub artisans: f64,
}

pub(super) struct KindRow {
    pub key: String,
    pub name: String,
    pub good: Option<String>,
    pub good_name: Option<String>,
    pub days: f64,
    pub cost: f64,
    pub goods: Vec<(String, String, f64)>,
    pub inputs: Vec<(String, String, f64)>,
    pub workforce: Option<f64>,
    /// (pop type, its name, share of the places).
    pub jobs: Vec<(String, String, f64)>,
    pub owner: Option<(String, String)>,
    pub available: bool,
}

pub(super) struct View {
    pub tag: String,
    pub name: String,
    pub played: bool,
    pub max_per_state: usize,
    pub treasury: f64,
    /// (state builds, state expands, state reopens, state subsidises, capitalists build, expand, reopen).
    pub policy: Option<[bool; 7]>,
    pub states: Vec<StateRow>,
    /// (tag, name, money) invested here by others, and invested by this country abroad.
    pub invested_here: Vec<(String, String, f64)>,
    pub invested_abroad: Vec<(String, String, f64)>,
    pub groups: Vec<(String, String)>,
    pub goods: Vec<GoodRow>,
    pub projects: Vec<ProjectRow>,
    pub kinds: Vec<KindRow>,
}

fn triples(v: &[(String, String, f64)]) -> String {
    format!("[{}]", v.iter().map(|(k, n, x)| format!("{{\"key\":{},\"name\":{},\"amount\":{}}}", json_str(k), json_str(n), json_num(*x))).collect::<Vec<_>>().join(","))
}

fn investors(v: &[(String, String, f64)]) -> String {
    format!("[{}]", v.iter().map(|(t, n, x)| format!("{{\"tag\":{},\"name\":{},\"amount\":{}}}", json_str(t), json_str(n), json_num(*x))).collect::<Vec<_>>().join(","))
}

/// The answer, from the gathered view.
pub(super) fn view_json(v: &View) -> String {
    let mut out = String::new();
    let policy = v.policy.map_or("null".into(), |p| {
        format!(
            "{{\"build\":{},\"expand\":{},\"reopen\":{},\"subsidise\":{},\"pops_build\":{},\"pops_expand\":{},\"pops_reopen\":{}}}",
            p[0], p[1], p[2], p[3], p[4], p[5], p[6]
        )
    });
    let _ = write!(
        out,
        "{{\"tag\":{},\"name\":{},\"played\":{},\"can_build\":false,\"max_per_state\":{},\"treasury\":{},\"policy\":{policy},\"states\":[",
        json_str(&v.tag),
        json_str(&v.name),
        v.played,
        v.max_per_state,
        json_num(v.treasury)
    );
    for (i, s) in v.states.iter().enumerate() {
        let factories: Vec<String> = s
            .factories
            .iter()
            .map(|f| {
                let profit = f.income - f.inputs - f.pay;
                format!(
                    "{{\"index\":{},\"kind\":{},\"name\":{},\"good\":{},\"good_name\":{},\"level\":{},\"workers\":{},\"places\":{},\"output\":{},\"income\":{},\"inputs\":{},\"pay\":{},\"profit\":{},\"throttle\":null,\"status\":{},\"days_left\":{},\"subsidised\":{},\"loss_days\":{}}}",
                    f.index,
                    json_str(&f.kind),
                    json_str(&f.name),
                    json_opt_str(f.good.as_deref()),
                    json_opt_str(f.good_name.as_deref()),
                    json_num(f.level),
                    f.workers.map_or("null".into(), json_int),
                    f.places.map_or("null".into(), json_int),
                    json_opt(f.output),
                    json_num(f.income),
                    json_num(f.inputs),
                    json_num(f.pay),
                    json_num(profit),
                    json_str(f.status),
                    f.days_left.map_or("null".into(), |d| json_int(d.max(0.0).ceil())),
                    f.subsidised,
                    f.loss_days
                )
            })
            .collect();
        let _ = write!(
            out,
            "{}{{\"id\":{},\"name\":{},\"provinces\":{},\"colony\":{},\"workforce\":{},\"investment\":{},\"factories\":[{}]}}",
            if i > 0 { "," } else { "" },
            s.id.map_or("null".into(), |r| r.to_string()),
            json_str(&s.name),
            s.provinces,
            s.colony,
            json_int(s.workforce),
            json_num(s.investment),
            factories.join(",")
        );
    }
    let _ = write!(out, "],\"foreign_investment\":{{\"here\":{},\"abroad\":{}}}", investors(&v.invested_here), investors(&v.invested_abroad));
    let groups: Vec<String> = v.groups.iter().map(|(k, n)| format!("{{\"key\":{},\"name\":{}}}", json_str(k), json_str(n))).collect();
    let goods: Vec<String> = v
        .goods
        .iter()
        .map(|g| {
            format!(
                "{{\"key\":{},\"name\":{},\"group\":{},\"colour\":{},\"price\":{},\"rgo\":{},\"factories\":{},\"artisans\":{},\"total\":{}}}",
                json_str(&g.key),
                json_str(&g.name),
                json_str(&g.group),
                g.colour.map_or("null".into(), |c| json_str(&format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))),
                json_opt(g.price),
                json_num(g.rgo),
                json_num(g.factories),
                json_num(g.artisans),
                json_num(g.rgo + g.factories + g.artisans)
            )
        })
        .collect();
    let _ = write!(out, ",\"groups\":[{}],\"goods\":[{}]", groups.join(","), goods.join(","));
    let projects: Vec<String> = v
        .projects
        .iter()
        .map(|p| {
            format!(
                "{{\"state\":{},\"kind\":{},\"name\":{},\"what\":{},\"by\":{},\"progress\":{},\"days_left\":{},\"price\":{},\"paid\":{},\"needs\":{}}}",
                json_str(&p.state),
                json_str(&p.kind),
                json_str(&p.name),
                json_str(p.what),
                json_str(p.by),
                json_opt(p.progress.map(|x| x.clamp(0.0, 1.0))),
                p.days_left.map_or("null".into(), |d| json_int(d.max(0.0).ceil())),
                json_opt(p.price),
                json_opt(p.paid),
                triples(&p.needs)
            )
        })
        .collect();
    let kinds: Vec<String> = v
        .kinds
        .iter()
        .map(|k| {
            let jobs: Vec<String> = k.jobs.iter().map(|(t, n, s)| format!("{{\"type\":{},\"name\":{},\"share\":{}}}", json_str(t), json_str(n), json_num(*s))).collect();
            format!(
                "{{\"key\":{},\"name\":{},\"good\":{},\"good_name\":{},\"days\":{},\"cost\":{},\"goods\":{},\"inputs\":{},\"workforce\":{},\"jobs\":[{}],\"owner\":{},\"available\":{}}}",
                json_str(&k.key),
                json_str(&k.name),
                json_opt_str(k.good.as_deref()),
                json_opt_str(k.good_name.as_deref()),
                json_int(k.days),
                json_num(k.cost),
                triples(&k.goods),
                triples(&k.inputs),
                k.workforce.map_or("null".into(), json_int),
                jobs.join(","),
                k.owner.as_ref().map_or("null".into(), |(t, n)| format!("{{\"type\":{},\"name\":{}}}", json_str(t), json_str(n))),
                k.available
            )
        })
        .collect();
    let _ = write!(out, ",\"projects\":[{}],\"kinds\":[{}]}}", projects.join(","), kinds.join(","));
    out
}

/// Good -> its group (the goods file's category) and its colour, read once from the install.
fn categories(sh: &Shared) -> &'static BTreeMap<String, (String, [u8; 3])> {
    static CATS: OnceLock<BTreeMap<String, (String, [u8; 3])>> = OnceLock::new();
    CATS.get_or_init(|| {
        let read = || -> Result<BTreeMap<String, (String, [u8; 3])>, String> {
            let engine_rules = wiktoria_data::config::load_engine_rules(&crate::root())?;
            let path = sh.install.join(engine_rules.require_str("FLD_GOODS_FILE")?);
            let doc = wiktoria_data::script::parse_bytes(&std::fs::read(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            Ok(wiktoria_data::goods::load_goods(&doc)?.goods.into_iter().map(|g| (g.id, (g.category, g.color))).collect())
        };
        read().unwrap_or_else(|e| {
            eprintln!("production window: goods groups left out: {e}");
            BTreeMap::new()
        })
    })
}

/// Gathers the view of the seat `tag`; `None` when it does not exist.
pub(super) fn gather(sh: &Shared, g: &Game, tag: &str) -> Option<View> {
    let (w, s) = (&g.world, &g.state);
    let c = s.countries.get(tag)?;
    let rules = &g.econ.industry_rules;
    let price = |good: &str| s.prices.get(good).copied();
    let cost_of = |kind: &str| rules.kinds.iter().find(|k| k.name == kind).map(|k| k.cost.iter().map(|(good, x)| x * price(good).unwrap_or(0.0)).sum::<f64>());
    let days_of = |kind: &str| rules.kinds.iter().find(|k| k.name == kind).map(|k| k.days);
    let ftype = |kind: &str| w.defs.factory_types.iter().find(|t| t.name == kind);
    // The states: every one the country owns land in, with or without factories.
    let mut states: BTreeMap<Option<u16>, StateRow> = BTreeMap::new();
    for p in s.provinces.values().filter(|p| p.owner.as_deref() == Some(tag)) {
        let row = states.entry(p.region).or_insert_with(|| StateRow {
            id: p.region,
            name: region_name(sh, g, p.region).unwrap_or_else(|| "Unnamed state".into()),
            provinces: 0,
            colony: false,
            workforce: 0.0,
            investment: s.investment.get(&(tag.to_string(), p.region)).copied().unwrap_or(0.0),
            factories: Vec::new(),
        });
        row.provinces += 1;
        row.colony |= p.colonial != 0;
        row.workforce += p.pops.iter().filter(|q| s.factory_worker_types.contains(&q.pop_type)).map(|q| q.size).sum::<f64>();
    }
    let days: BTreeMap<usize, wiktoria_simulation::economy::FactoryDay> = g.econ.farms.factory_days(w, s).into_iter().map(|d| (d.index, d)).collect();
    let money = s.money.countries.get(tag);
    let sites: BTreeMap<usize, &wiktoria_simulation::economy::Construction> = money.map(|m| m.constructions.iter().map(|x| (x.factory, x)).collect()).unwrap_or_default();
    let mut projects = Vec::new();
    for (index, f) in s.factories.iter().enumerate().filter(|(_, f)| f.owner == tag) {
        let t = ftype(&f.kind);
        let good = t.map(|t| t.output_good.clone()).or_else(|| s.factory_types.get(&f.kind).map(|x| x.0.clone()));
        let d = days.get(&index);
        let status = if f.building_days.is_some() {
            if f.level < 1.0 { "opening" } else { "building" }
        } else if f.closed {
            "closed"
        } else {
            "running"
        };
        let name = sh.name(&f.kind);
        let state_name = states.get(&f.region).map_or_else(|| region_name(sh, g, f.region).unwrap_or_default(), |r| r.name.clone());
        if let Some(left) = f.building_days {
            let total = days_of(&f.kind);
            let site = sites.get(&index);
            projects.push(ProjectRow {
                state: state_name,
                kind: f.kind.clone(),
                name: name.clone(),
                what: if f.level < 1.0 { "new" } else { "expand" },
                by: if f.state_built { "state" } else { "capitalists" },
                progress: total.filter(|t| *t > 0.0).map(|t| 1.0 - left / t),
                days_left: Some(left),
                price: None,
                paid: None,
                needs: site.map(|x| x.lacks.iter().filter(|(_, n)| **n > 1e-9).map(|(k, n)| (k.clone(), sh.name(k), *n)).collect()).unwrap_or_default(),
            });
        }
        let row = FactoryRow {
            index,
            kind: f.kind.clone(),
            name,
            good_name: good.as_deref().map(|x| sh.name(x)),
            good,
            level: f.level,
            workers: d.map(|d| d.workers),
            places: d.map(|d| d.places).or_else(|| t.map(|t| t.workforce * f.level.max(1.0))),
            output: d.map(|d| d.units),
            income: f.income,
            inputs: f.spending,
            pay: f.pay,
            status,
            days_left: f.building_days,
            subsidised: f.subsidised,
            loss_days: f.loss_days,
        };
        if let Some(st) = states.get_mut(&f.region) {
            st.factories.push(row);
        }
    }
    // The capitalists' plans, paid for from the state's fund.
    let policy = rules.policy(c);
    for ((owner, region), project) in s.projects.iter().filter(|(k, _)| k.0 == tag) {
        let (what, kind) = match project {
            Project::New(k) => ("new", k.clone()),
            Project::Reopen(i) => ("reopen", s.factories.get(*i).map(|f| f.kind.clone()).unwrap_or_default()),
            Project::Expand(i) => ("expand", s.factories.get(*i).map(|f| f.kind.clone()).unwrap_or_default()),
        };
        let base = cost_of(&kind).map(|x| if what == "reopen" { x * rules.ai.reopen_cost_share } else { x });
        let price = base.map(|x| x * policy.owner_cost);
        let paid = s.investment.get(&(owner.clone(), *region)).copied().unwrap_or(0.0);
        projects.push(ProjectRow {
            state: states.get(region).map_or_else(|| region_name(sh, g, *region).unwrap_or_default(), |r| r.name.clone()),
            name: sh.name(&kind),
            kind,
            what,
            by: "capitalists",
            progress: price.filter(|p| *p > 0.0).map(|p| paid / p),
            days_left: None,
            price,
            paid: Some(paid),
            needs: Vec::new(),
        });
    }
    // What the country makes, per good.
    let cats = categories(sh);
    let groups: Vec<(String, String)> = GROUPS.iter().map(|k| (k.to_string(), sh.name(k))).collect();
    let mut made: BTreeMap<String, [f64; 3]> = BTreeMap::new();
    for (_, owner, good, units) in g.econ.farms.province_output(w, s) {
        if owner == tag {
            made.entry(good).or_default()[0] += units;
        }
    }
    for ((owner, good), (f, a)) in g.econ.farms.workshop_output(w, s) {
        if owner == tag {
            let e = made.entry(good).or_default();
            e[1] += f;
            e[2] += a;
        }
    }
    let all_goods: BTreeSet<&str> = s.prices.keys().map(String::as_str).collect();
    let goods: Vec<GoodRow> = all_goods
        .iter()
        .filter_map(|k| {
            let (group, colour) = cats.get(*k).cloned().map_or((String::from("other"), None), |(g, c)| (g, Some(c)));
            if !GROUPS.contains(&group.as_str()) {
                return None;
            }
            let m = made.get(*k).copied().unwrap_or_default();
            Some(GoodRow { key: k.to_string(), name: sh.name(k), group, colour, price: price(k), rgo: m[0], factories: m[1], artisans: m[2] })
        })
        .collect();
    // The factory types, for the picker.
    let open = rules.enabled(w, c);
    let kinds: Vec<KindRow> = rules
        .kinds
        .iter()
        .map(|k| {
            let t = ftype(&k.name);
            let named = |v: Vec<(&String, &f64)>| v.into_iter().map(|(g, x)| (g.clone(), sh.name(g), *x)).collect::<Vec<_>>();
            KindRow {
                key: k.name.clone(),
                name: sh.name(&k.name),
                good: t.map(|t| t.output_good.clone()),
                good_name: t.map(|t| sh.name(&t.output_good)),
                days: k.days,
                cost: k.cost.iter().map(|(good, x)| x * price(good).unwrap_or(0.0)).sum(),
                goods: k.cost.iter().map(|(g, x)| (g.clone(), sh.name(g), *x)).collect(),
                inputs: t.map(|t| named(t.input_goods.iter().collect())).unwrap_or_default(),
                workforce: t.map(|t| t.workforce),
                jobs: t.map(|t| t.employees.iter().map(|j| (j.pop_type.clone(), sh.name(&j.pop_type), j.amount)).collect()).unwrap_or_default(),
                owner: t.and_then(|t| t.owner.as_ref()).map(|j| (j.pop_type.clone(), sh.name(&j.pop_type))),
                available: open.contains(k.name.as_str()),
            }
        })
        .collect();
    let invested_here: Vec<(String, String, f64)> = c.foreign_investment.iter().filter(|(_, x)| **x > 0.0).map(|(t, x)| (t.clone(), country_name(g, t), *x)).collect();
    let invested_abroad: Vec<(String, String, f64)> = s.countries.iter().filter_map(|(t, o)| o.foreign_investment.get(tag).filter(|x| **x > 0.0).map(|x| (t.clone(), country_name(g, t), *x))).collect();
    Some(View {
        tag: tag.to_string(),
        name: country_name(g, tag),
        played: c.human,
        max_per_state: rules.max_per_state,
        treasury: c.money,
        policy: c.ruling_party.map(|_| [policy.build_factory, policy.expand_factory, policy.open_factory, policy.can_subsidise, policy.pop_build_factory, policy.pop_expand_factory, policy.pop_open_factory]),
        states: states.into_values().collect(),
        invested_here,
        invested_abroad,
        groups,
        goods,
        projects,
        kinds,
    })
}

/// `GET /api/production`: worked out at most once a day per seat (the factories' day goes over
/// every pop), and whenever the seat is taken or let go.
pub(super) fn production_json(sh: &Shared, g: &Game, tag: &str) -> String {
    static KEPT: Mutex<Option<(Date, String, bool, String)>> = Mutex::new(None);
    let played = g.state.countries.get(tag).is_some_and(|c| c.human);
    let mut kept = lock(&KEPT);
    if let Some((_, _, _, body)) = kept.as_ref().filter(|(d, t, p, _)| *d == g.state.date && t == tag && *p == played) {
        return body.clone();
    }
    let body = gather(sh, g, tag).map_or_else(|| format!("{{\"tag\":{},\"exists\":false}}", json_str(tag)), |v| view_json(&v));
    *kept = Some((g.state.date, tag.to_string(), played, body.clone()));
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    fn factory(status: &'static str, workers: Option<f64>) -> FactoryRow {
        FactoryRow {
            index: 3,
            kind: "kind_one".into(),
            name: "Kind One".into(),
            good: Some("good_one".into()),
            good_name: Some("Good One".into()),
            level: 2.0,
            workers,
            places: Some(20000.0),
            output: workers.map(|_| 4.5),
            income: 10.0,
            inputs: 6.0,
            pay: 3.0,
            status,
            days_left: None,
            subsidised: false,
            loss_days: 0,
        }
    }

    #[test]
    fn the_production_answer_shows_what_it_has_and_null_for_the_rest() {
        let v = View {
            tag: "AAA".into(),
            name: "Country A".into(),
            played: true,
            max_per_state: 8,
            treasury: 1000.0,
            policy: None,
            states: vec![
                StateRow { id: Some(4), name: "State One".into(), provinces: 3, colony: false, workforce: 15000.0, investment: 12.5, factories: vec![factory("running", Some(12000.0)), factory("closed", None)] },
                StateRow { id: Some(5), name: "State Two".into(), provinces: 1, colony: true, workforce: 0.0, investment: 0.0, factories: vec![] },
            ],
            invested_here: vec![("BBB".into(), "Country B".into(), 50.0)],
            invested_abroad: vec![],
            groups: vec![("raw_material_goods".into(), "Raw".into())],
            goods: vec![GoodRow { key: "good_one".into(), name: "Good One".into(), group: "raw_material_goods".into(), colour: Some([255, 0, 16]), price: Some(2.0), rgo: 1.0, factories: 2.0, artisans: 0.5 }],
            projects: vec![ProjectRow { state: "State One".into(), kind: "kind_one".into(), name: "Kind One".into(), what: "new", by: "state", progress: Some(1.4), days_left: Some(9.2), price: None, paid: None, needs: vec![("good_two".into(), "Good Two".into(), 3.0)] }],
            kinds: vec![],
        };
        let j = view_json(&v);
        assert!(j.contains("\"can_build\":false"), "{j}");
        assert!(j.contains("\"policy\":null"));
        assert!(j.contains("\"workers\":12000,\"places\":20000,\"output\":4.5"), "{j}");
        assert!(j.contains("\"workers\":null,\"places\":20000,\"output\":null"), "{j}");
        assert!(j.contains("\"profit\":1.0,\"throttle\":null"), "{j}");
        assert!(j.contains("\"name\":\"State Two\",\"provinces\":1,\"colony\":true,\"workforce\":0,\"investment\":0.0,\"factories\":[]"), "{j}");
        assert!(j.contains("\"here\":[{\"tag\":\"BBB\",\"name\":\"Country B\",\"amount\":50.0}],\"abroad\":[]"), "{j}");
        assert!(j.contains("\"total\":3.5"), "{j}");
        assert!(j.contains("\"colour\":\"#ff0010\""), "{j}");
        assert!(j.contains("\"progress\":1.0,\"days_left\":10"), "{j}");
        assert!(j.contains("\"needs\":[{\"key\":\"good_two\",\"name\":\"Good Two\",\"amount\":3.0}]"), "{j}");
        // Balanced brackets: the parts are joined right.
        assert_eq!(j.matches('{').count(), j.matches('}').count());
        assert_eq!(j.matches('[').count(), j.matches(']').count());
    }
}
