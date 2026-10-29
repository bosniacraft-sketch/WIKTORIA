//! The Population window (apps/web/API.md: Population): the seat's states and provinces as a
//! filter tree, its pops of the chosen place and types, one page of them at a time, and the
//! shares of the chosen pops by type, culture, religion, ideology and issue.

use std::collections::BTreeMap;

use wiktoria_simulation::game::Game;
use wiktoria_simulation::script::{PopState, State};

use super::{json_int, json_num, json_opt, json_opt_str, json_str, param, province_name, region_name, Shared};

/// Most pops one answer lists.
const MAX_LIMIT: usize = 200;
const DEFAULT_LIMIT: usize = 50;
/// Slices kept per pie; the rest are left out (the page draws them as "other").
const PIE_SLICES: usize = 8;

/// What the page asks for (`?state=&province=&type=&sort=&desc=&offset=&limit=`).
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct Ask {
    pub state: Option<u16>,
    pub province: Option<u16>,
    /// Pop types to show; empty = all.
    pub types: Vec<String>,
    pub sort: Sort,
    pub desc: bool,
    pub offset: usize,
    pub limit: usize,
}

/// The column the pops are sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Sort {
    #[default]
    Size,
    Type,
    Culture,
    Religion,
    Location,
    Militancy,
    Consciousness,
    Literacy,
    Unemployment,
    Cash,
    Needs,
}

impl Sort {
    fn of(s: &str) -> Option<Sort> {
        Some(match s {
            "size" => Sort::Size,
            "type" => Sort::Type,
            "culture" => Sort::Culture,
            "religion" => Sort::Religion,
            "location" => Sort::Location,
            "militancy" => Sort::Militancy,
            "consciousness" => Sort::Consciousness,
            "literacy" => Sort::Literacy,
            "unemployment" => Sort::Unemployment,
            "cash" => Sort::Cash,
            "needs" => Sort::Needs,
            _ => return None,
        })
    }
}

/// Reads the query; an unknown sort or a bad number is refused.
pub(super) fn ask_of(q: &[(String, String)]) -> Result<Ask, String> {
    let get = |k: &str| param(q, k).map(str::trim).filter(|v| !v.is_empty() && *v != "null");
    let id = |k: &str| get(k).map(|v| v.parse::<u16>().map_err(|_| format!("{k} must be a number or empty"))).transpose();
    let count = |k: &str, d: usize| get(k).map_or(Ok(d), |v| v.parse::<usize>().map_err(|_| format!("{k} must be a number")));
    Ok(Ask {
        state: id("state")?,
        province: id("province")?,
        types: get("type").map(|t| t.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect()).unwrap_or_default(),
        sort: match get("sort") {
            Some(s) => Sort::of(s).ok_or("sort must be one of size, type, culture, religion, location, militancy, consciousness, literacy, unemployment, cash, needs")?,
            None => Sort::Size,
        },
        desc: !matches!(get("desc"), Some("false" | "0" | "no")),
        offset: count("offset", 0)?,
        limit: count("limit", DEFAULT_LIMIT)?.clamp(1, MAX_LIMIT),
    })
}

/// Names the answer needs, from the install (kept apart from `Shared` so tests can make them).
pub(super) struct Names<'a> {
    /// A key's display name (pop types, cultures, religions, ideologies, issue options, focuses).
    pub key: &'a dyn Fn(&str) -> String,
    pub region: &'a dyn Fn(u16) -> String,
    pub province: &'a dyn Fn(u16) -> String,
}

/// Fixed facts of the install and the country.
pub(super) struct Facts<'a> {
    /// Pop types in file order, with their strata and their colour in the install.
    pub pop_types: &'a [(String, String, [u8; 3])],
    /// A pop's `money` is this many times a pound.
    pub money_scale: f64,
    pub focus_max: Option<usize>,
}

/// `GET /api/population` for the seat.
pub(super) fn population_json(sh: &Shared, g: &Game, tag: &str, ask: &Ask) -> String {
    let key = |k: &str| sh.name(k);
    let region = |r: u16| region_name(sh, g, Some(r)).unwrap_or_else(|| format!("State {r}"));
    let province = |p: u16| province_name(g, p);
    let names = Names { key: &key, region: &region, province: &province };
    let pop_types: Vec<(String, String, [u8; 3])> = g.world.defs.pop_types.iter().map(|t| (t.name.clone(), t.strata.clone(), t.color)).collect();
    let facts = Facts { pop_types: &pop_types, money_scale: g.econ.farms.pop_money_scale(), focus_max: g.econ.focus_ai.points.get(tag).copied() };
    build(&g.state, tag, &names, &facts, ask)
}

/// `[["name",share],...]` of the largest shares, at most `PIE_SLICES`.
fn pie(acc: BTreeMap<String, f64>, total: f64, name: &dyn Fn(&str) -> String) -> String {
    let mut v: Vec<(String, f64)> = acc.into_iter().filter(|(_, x)| *x > 0.0).collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let items: Vec<String> = v.iter().take(PIE_SLICES).map(|(k, x)| format!("[{},{}]", json_str(&name(k)), json_num(if total > 0.0 { x / total } else { 0.0 }))).collect();
    format!("[{}]", items.join(","))
}

/// One state of the tree: its number, name, people and (province, people) list.
type StateNode = (Option<u16>, String, f64, Vec<(u16, f64)>);

/// The answer, from the state alone.
pub(super) fn build(s: &State, tag: &str, n: &Names, f: &Facts, ask: &Ask) -> String {
    let Some(c) = s.countries.get(tag) else {
        return format!("{{\"tag\":{},\"exists\":false}}", json_str(tag));
    };
    // The seat's provinces by state (region), and the state of each.
    let owned: Vec<(u16, &wiktoria_simulation::script::ProvinceState)> = s.provinces.iter().filter(|(_, p)| p.owner.as_deref() == Some(tag)).map(|(id, p)| (*id, p)).collect();
    let mut by_state: BTreeMap<Option<u16>, Vec<(u16, f64)>> = BTreeMap::new();
    for (id, p) in &owned {
        by_state.entry(p.region).or_default().push((*id, p.pops.iter().map(|q| q.size).sum()));
    }
    let people: f64 = by_state.values().flatten().map(|x| x.1).sum();
    let mut states: Vec<StateNode> = by_state
        .into_iter()
        .map(|(r, mut provs)| {
            provs.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            let total = provs.iter().map(|x| x.1).sum();
            (r, r.map_or_else(|| "Other provinces".to_string(), n.region), total, provs)
        })
        .collect();
    states.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.1.cmp(&b.1)));
    let tree: Vec<String> = states
        .iter()
        .map(|(r, name, total, provs)| {
            let focus = r.and_then(|r| c.national_focus.get(&r));
            let provinces: Vec<String> = provs.iter().map(|(id, size)| format!("{{\"id\":{id},\"name\":{},\"population\":{}}}", json_str(&(n.province)(*id)), json_int(*size))).collect();
            format!(
                "{{\"id\":{},\"name\":{},\"population\":{},\"focus\":{},\"provinces\":[{}]}}",
                r.map_or("null".into(), |r| r.to_string()),
                json_str(name),
                json_int(*total),
                json_opt_str(focus.map(|x| (n.key)(x)).as_deref()),
                provinces.join(",")
            )
        })
        .collect();

    // The place asked for: a province of the seat, a state of it, or the whole country.
    let in_place = |id: u16, p: &wiktoria_simulation::script::ProvinceState| match (ask.province, ask.state) {
        (Some(want), _) => id == want,
        (None, Some(r)) => p.region == Some(r),
        (None, None) => true,
    };
    let place: Vec<(u16, &wiktoria_simulation::script::ProvinceState)> = owned.iter().filter(|(id, p)| in_place(*id, p)).copied().collect();
    // People of each type there (for the type toggles).
    let mut by_type: BTreeMap<&str, f64> = BTreeMap::new();
    for q in place.iter().flat_map(|(_, p)| &p.pops) {
        *by_type.entry(q.pop_type.as_str()).or_default() += q.size;
    }
    let types: Vec<String> = f
        .pop_types
        .iter()
        .map(|(t, strata, [r, g, b])| format!("{{\"key\":{},\"name\":{},\"strata\":{},\"colour\":\"#{r:02x}{g:02x}{b:02x}\",\"population\":{},\"shown\":{}}}", json_str(t), json_str(&(n.key)(t)), json_str(strata), json_int(by_type.get(t.as_str()).copied().unwrap_or(0.0)), ask.types.is_empty() || ask.types.contains(t)))
        .collect();

    // The pops asked for, with what each row shows.
    struct Row<'a> {
        province: u16,
        region: Option<u16>,
        q: &'a PopState,
        unemployment: f64,
    }
    let mut out_of_work: BTreeMap<(u16, &str), f64> = BTreeMap::new();
    let mut rows: Vec<Row> = Vec::new();
    for (id, p) in &place {
        for q in p.pops.iter().filter(|q| ask.types.is_empty() || ask.types.contains(&q.pop_type)) {
            let u = *out_of_work.entry((*id, q.pop_type.as_str())).or_insert_with(|| s.unemployment(&[*id], Some(&q.pop_type)));
            rows.push(Row { province: *id, region: p.region, q, unemployment: u });
        }
    }

    // The shares of the chosen pops.
    let total: f64 = rows.iter().map(|r| r.q.size).sum();
    let mut acc: [BTreeMap<String, f64>; 5] = Default::default();
    let (mut lit, mut mil, mut con, mut unemployed, mut cash) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for r in &rows {
        let q = r.q;
        *acc[0].entry(q.pop_type.clone()).or_default() += q.size;
        *acc[1].entry(q.culture.clone()).or_default() += q.size;
        *acc[2].entry(q.religion.clone()).or_default() += q.size;
        for (k, v) in &q.ideology {
            if let Some(name) = (*k as usize).checked_sub(1).and_then(|i| s.ideologies.get(i)) {
                *acc[3].entry(name.clone()).or_default() += v / 100.0 * q.size;
            }
        }
        for (k, v) in &q.issues {
            if let Some((name, _)) = (*k as usize).checked_sub(1).and_then(|i| s.issues.get(i)) {
                *acc[4].entry(name.clone()).or_default() += v / 100.0 * q.size;
            }
        }
        lit += q.literacy * q.size;
        mil += q.militancy * q.size;
        con += q.consciousness * q.size;
        unemployed += r.unemployment * q.size;
        cash += q.money;
    }
    let avg = |x: f64| if total > 0.0 { Some(x / total) } else { None };
    let [p_types, p_culture, p_religion, p_ideology, p_issues] = acc;
    let summary = format!(
        "{{\"population\":{},\"pops\":{},\"literacy\":{},\"militancy\":{},\"consciousness\":{},\"unemployment\":{},\"cash\":{},\"types\":{},\"culture\":{},\"religion\":{},\"ideology\":{},\"issues\":{}}}",
        json_int(total),
        rows.len(),
        json_opt(avg(lit)),
        json_opt(avg(mil)),
        json_opt(avg(con)),
        json_opt(avg(unemployed)),
        json_opt((f.money_scale > 0.0).then(|| cash / f.money_scale)),
        pie(p_types, total, n.key),
        pie(p_culture, total, n.key),
        pie(p_religion, total, n.key),
        pie(p_ideology, total, n.key),
        pie(p_issues, total, n.key),
    );

    // One page, sorted.
    let needs = |q: &PopState| q.life_needs + q.everyday_needs + q.luxury_needs;
    let mut names_of: BTreeMap<u16, String> = BTreeMap::new();
    if ask.sort == Sort::Location {
        for r in &rows {
            names_of.entry(r.province).or_insert_with(|| (n.province)(r.province));
        }
    }
    rows.sort_by(|a, b| {
        let (x, y) = (a.q, b.q);
        let o = match ask.sort {
            Sort::Size => x.size.total_cmp(&y.size),
            Sort::Type => x.pop_type.cmp(&y.pop_type),
            Sort::Culture => x.culture.cmp(&y.culture),
            Sort::Religion => x.religion.cmp(&y.religion),
            Sort::Location => names_of.get(&a.province).cmp(&names_of.get(&b.province)),
            Sort::Militancy => x.militancy.total_cmp(&y.militancy),
            Sort::Consciousness => x.consciousness.total_cmp(&y.consciousness),
            Sort::Literacy => x.literacy.total_cmp(&y.literacy),
            Sort::Unemployment => a.unemployment.total_cmp(&b.unemployment),
            Sort::Cash => x.money.total_cmp(&y.money),
            Sort::Needs => needs(x).total_cmp(&needs(y)),
        };
        let o = if ask.desc { o.reverse() } else { o };
        o.then(y.size.total_cmp(&x.size)).then(x.id.cmp(&y.id))
    });
    let count = rows.len();
    let offset = ask.offset.min(count.saturating_sub(1) / ask.limit * ask.limit);
    let page: Vec<String> = rows
        .iter()
        .skip(offset)
        .take(ask.limit)
        .map(|r| {
            let q = r.q;
            format!(
                "{{\"id\":{},\"type\":{},\"type_name\":{},\"size\":{},\"culture\":{},\"religion\":{},\"province\":{{\"id\":{},\"name\":{}}},\"state\":{},\"militancy\":{},\"consciousness\":{},\"literacy\":{},\"unemployment\":{},\"cash\":{},\"needs\":{{\"life\":{},\"everyday\":{},\"luxury\":{}}}}}",
                q.id,
                json_str(&q.pop_type),
                json_str(&(n.key)(&q.pop_type)),
                json_int(q.size),
                json_str(&(n.key)(&q.culture)),
                json_str(&(n.key)(&q.religion)),
                r.province,
                json_str(&(n.province)(r.province)),
                r.region.map_or("null".into(), |r| r.to_string()),
                json_num(q.militancy),
                json_num(q.consciousness),
                json_num(q.literacy),
                json_num(r.unemployment),
                json_opt((f.money_scale > 0.0).then(|| q.money / f.money_scale)),
                json_num(q.life_needs),
                json_num(q.everyday_needs),
                json_num(q.luxury_needs),
            )
        })
        .collect();
    let sort = format!("{:?}", ask.sort).to_lowercase();
    format!(
        "{{\"tag\":{},\"exists\":true,\"played\":{},\"population\":{},\"focus_used\":{},\"focus_max\":{},\"states\":[{}],\"types\":[{}],\"filter\":{{\"state\":{},\"province\":{},\"types\":[{}]}},\"summary\":{summary},\"pops\":{{\"sort\":{},\"desc\":{},\"offset\":{offset},\"limit\":{},\"count\":{count},\"rows\":[{}]}}}}",
        json_str(tag),
        c.human,
        json_int(people),
        c.national_focus.len(),
        f.focus_max.map_or("null".into(), |m| m.to_string()),
        tree.join(","),
        types.join(","),
        ask.state.map_or("null".into(), |r| r.to_string()),
        ask.province.map_or("null".into(), |p| p.to_string()),
        ask.types.iter().map(|t| json_str(t)).collect::<Vec<_>>().join(","),
        json_str(&sort),
        ask.desc,
        ask.limit,
        page.join(","),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::parse_query;
    use wiktoria_data::date::Date;
    use wiktoria_simulation::script::{CountryState, ProvinceState};

    fn pop(id: u64, kind: &str, size: f64, culture: &str, militancy: f64) -> PopState {
        PopState { pop_type: kind.into(), id, size, culture: culture.into(), religion: "faith_one".into(), money: 500.0, literacy: 0.25, militancy, consciousness: 1.0, ideology: [(1, 60.0), (2, 40.0)].into(), issues: [(1, 100.0)].into(), life_needs: 1.0, everyday_needs: 0.5, ..Default::default() }
    }

    fn world() -> State {
        let mut s = State::new(Date::new(1840, 1, 1));
        s.ideologies = vec!["idea_one".into(), "idea_two".into()];
        s.issues = vec![("option_one".into(), "group_one".into())];
        let mut c = CountryState { human: true, ..Default::default() };
        c.national_focus.insert(1, "focus_one".into());
        s.countries.insert("AAA".into(), c);
        s.countries.insert("BBB".into(), CountryState::default());
        let prov = |owner: &str, region: u16, pops: Vec<PopState>| ProvinceState { owner: Some(owner.into()), region: Some(region), pops, ..Default::default() };
        s.provinces.insert(10, prov("AAA", 1, vec![pop(1, "farmers", 3000.0, "culture_a", 2.0), pop(2, "clergy", 100.0, "culture_a", 0.5)]));
        s.provinces.insert(11, prov("AAA", 1, vec![pop(3, "farmers", 1000.0, "culture_b", 6.0)]));
        s.provinces.insert(20, prov("AAA", 2, vec![pop(4, "craftsmen", 900.0, "culture_b", 1.0)]));
        s.provinces.insert(30, prov("BBB", 3, vec![pop(5, "farmers", 9999.0, "culture_c", 9.0)]));
        s
    }

    fn answer(query: &str) -> String {
        let s = world();
        let key = |k: &str| format!("<{k}>");
        let region = |r: u16| format!("State {r}");
        let province = |p: u16| format!("Province {p}");
        let types = [("farmers".to_string(), "poor".to_string(), [10, 20, 30]), ("craftsmen".to_string(), "poor".to_string(), [0, 0, 0]), ("clergy".to_string(), "middle".to_string(), [255, 255, 255])];
        let ask = ask_of(&parse_query(query)).unwrap();
        build(&s, "AAA", &Names { key: &key, region: &region, province: &province }, &Facts { pop_types: &types, money_scale: 100.0, focus_max: Some(2) }, &ask)
    }

    #[test]
    fn the_seat_s_states_and_pops_are_listed() {
        let a = answer("");
        assert!(a.contains("\"population\":5000,\"focus_used\":1,\"focus_max\":2"), "{a}");
        // States largest first, each with its provinces, the focus named.
        assert!(a.contains("\"states\":[{\"id\":1,\"name\":\"State 1\",\"population\":4100,\"focus\":\"<focus_one>\",\"provinces\":[{\"id\":10,\"name\":\"Province 10\",\"population\":3100},{\"id\":11,"), "{a}");
        assert!(!a.contains("Province 30"), "another country's province: {a}");
        assert!(a.contains("{\"key\":\"clergy\",\"name\":\"<clergy>\",\"strata\":\"middle\",\"colour\":\"#ffffff\",\"population\":100,\"shown\":true}"), "{a}");
        // Largest pop first; cash in pounds.
        assert!(a.contains("\"rows\":[{\"id\":1,\"type\":\"farmers\",\"type_name\":\"<farmers>\",\"size\":3000,"), "{a}");
        assert!(a.contains("\"cash\":5.0"), "{a}");
        assert!(a.contains("\"ideology\":[[\"<idea_one>\",0.6],[\"<idea_two>\",0.4]]"), "{a}");
        assert!(a.contains("\"issues\":[[\"<option_one>\",1.0]]"), "{a}");
        assert!(a.contains("\"count\":4,"), "{a}");
    }

    #[test]
    fn the_filter_and_the_page_are_kept() {
        let a = answer("state=1&type=farmers&sort=militancy&desc=true");
        assert!(a.contains("\"count\":2,"), "{a}");
        assert!(a.contains("\"rows\":[{\"id\":3,"), "most militant first: {a}");
        assert!(a.contains("{\"key\":\"craftsmen\",\"name\":\"<craftsmen>\",\"strata\":\"poor\",\"colour\":\"#000000\",\"population\":0,\"shown\":false}"), "{a}");
        assert!(a.contains("\"types\":[[\"<farmers>\",1.0]]"), "{a}");
        let a = answer("province=20");
        assert!(a.contains("\"count\":1,") && a.contains("\"rows\":[{\"id\":4,"), "{a}");
        let a = answer("limit=2&offset=2&sort=size&desc=false");
        assert!(a.contains("\"offset\":2,\"limit\":2,\"count\":4,\"rows\":[{\"id\":3,"), "{a}");
        // An offset past the end shows the last page.
        let a = answer("limit=3&offset=99");
        assert!(a.contains("\"offset\":3,"), "{a}");
        assert!(ask_of(&parse_query("sort=height")).is_err());
        assert!(ask_of(&parse_query("offset=x")).is_err());
        assert_eq!(ask_of(&parse_query("limit=5000")).unwrap().limit, MAX_LIMIT);
    }
}
