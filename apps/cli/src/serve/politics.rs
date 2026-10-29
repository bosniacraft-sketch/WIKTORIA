//! The Politics window (apps/web/API.md: Politics): the seat's government, parties, upper house,
//! the ideologies of its voters and people, the issues they ask for, its reforms, movements,
//! rebels and the countries it could release. Read-only: the engine has no player order for
//! reforms or releasing a country yet.

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_data::date::Date;
use wiktoria_simulation::game::Game;
use wiktoria_simulation::politics::{self, PoliticsRules};
use wiktoria_simulation::reforms::{self, ReformRules};
use wiktoria_simulation::script::State;
use wiktoria_simulation::unrest::{MovementKind, UnrestState};

use super::{country_name, json_num, json_opt, json_opt_str, json_str, Shared};

/// How many issues the important issues table lists.
const ISSUES_SHOWN: usize = 10;

/// How keys become the player's words: localised names, country names, party names by number.
pub(super) struct Names<'a> {
    pub name: &'a dyn Fn(&str) -> String,
    pub country: &'a dyn Fn(&str) -> String,
    pub party: &'a dyn Fn(u32) -> Option<String>,
}

/// What the builder reads, apart from the state.
pub(super) struct Inputs<'a> {
    pub politics: &'a PoliticsRules,
    pub reforms: &'a ReformRules,
    pub unrest: Option<&'a UnrestState>,
    /// Rebel armies standing in the country's provinces: (armies, regiments).
    pub rebel_armies: (usize, usize),
    pub rebel_tag: &'a str,
}

/// `GET /api/politics` for the seat.
pub(super) fn politics_json(sh: &Shared, g: &Game, tag: &str) -> String {
    let rebels = &g.core.day.rebel_tag;
    let mut armies = (0, 0);
    for a in g.core.mil.armies_of(rebels).filter(|a| g.state.provinces.get(&a.location).is_some_and(|p| p.owner.as_deref() == Some(tag))) {
        armies.0 += 1;
        armies.1 += a.regiments.len();
    }
    let name = |k: &str| sh.name(k);
    let country = |t: &str| country_name(g, t);
    let party = |n: u32| sh.parties.get((n as usize).checked_sub(1)?).map(|k| sh.name(k));
    let names = Names { name: &name, country: &country, party: &party };
    let inputs = Inputs { politics: &g.econ.politics_rules, reforms: &g.econ.reform_rules, unrest: g.core.unrest.as_ref().map(|(_, u)| u), rebel_armies: armies, rebel_tag: rebels };
    build(&g.state, &inputs, &names, tag)
}

fn months_after(d: Date, months: i32) -> Date {
    let at = d.year * 12 + i32::from(d.month) - 1 + months;
    Date::new(at.div_euclid(12), at.rem_euclid(12) as u8 + 1, d.day.min(28))
}

/// `[{"key":"liberal","share":0.4}, ...]` in the ideologies' file order, every ideology listed
/// (so the page gives each the same colour in every pie); `null` when there is no count.
fn ideology_shares(s: &State, shares: Option<&BTreeMap<String, f64>>) -> String {
    let Some(shares) = shares else { return "null".into() };
    let rows: Vec<String> = s.ideologies.iter().map(|i| format!("[{},{}]", json_str(i), json_num(shares.get(i).copied().unwrap_or(0.0)))).collect();
    format!("[{}]", rows.join(","))
}

fn date_or_null(d: Option<Date>) -> String {
    d.map_or("null".into(), |d| json_str(&d.to_string()))
}

/// The answer, from the state and the rules alone (tested without an install).
pub(super) fn build(s: &State, inp: &Inputs, n: &Names, tag: &str) -> String {
    let Some(c) = s.countries.get(tag) else { return "null".into() };
    let pr = inp.politics;
    let rr = inp.reforms;

    // ---- government and parties
    let government = c.government.as_deref().map_or("null".into(), |k| {
        let elections = pr.governments.get(k).map(|g| g.election);
        format!("{{\"key\":{},\"name\":{},\"elections\":{}}}", json_str(k), json_str(&(n.name)(k)), elections.map_or("null".into(), |e| e.to_string()))
    });
    let group_of = |option: &str| s.issues.iter().find(|(o, _)| o == option).map(|(_, g)| g.clone());
    let policy = |option: &String| {
        let group = group_of(option);
        format!(
            "{{\"group\":{},\"group_name\":{},\"option\":{},\"option_name\":{}}}",
            json_opt_str(group.as_deref()),
            json_opt_str(group.as_deref().map(|g| (n.name)(g)).as_deref()),
            json_str(option),
            json_str(&(n.name)(option))
        )
    };
    let today = s.date;
    let allowed = c.government.as_deref().and_then(|g| pr.governments.get(g)).map(|g| &g.ideologies);
    let parties: Vec<String> = pr
        .parties
        .get(tag)
        .into_iter()
        .flatten()
        .filter(|p| p.start.is_none_or(|d| d <= today) && p.end.is_none_or(|d| today < d))
        .map(|p| {
            let ruling = c.ruling_party == Some(p.id);
            format!(
                "{{\"id\":{},\"name\":{},\"ideology\":{},\"ideology_name\":{},\"ruling\":{ruling},\"allowed\":{},\"policies\":[{}]}}",
                p.id,
                json_opt_str((n.party)(p.id).as_deref()),
                json_str(&p.ideology),
                json_str(&(n.name)(&p.ideology)),
                allowed.is_some_and(|a| a.contains(&p.ideology)),
                p.positions.iter().map(policy).collect::<Vec<_>>().join(",")
            )
        })
        .collect();
    let ruling = c.ruling_party.map_or("null".into(), |id| {
        let ideology = s.party_ideologies.get((id as usize).saturating_sub(1)).cloned();
        format!(
            "{{\"id\":{id},\"name\":{},\"ideology\":{},\"ideology_name\":{}}}",
            json_opt_str((n.party)(id).as_deref()),
            json_opt_str(ideology.as_deref()),
            json_opt_str(ideology.as_deref().map(|i| (n.name)(i)).as_deref())
        )
    });
    let ideologies: Vec<String> = s.ideologies.iter().map(|i| format!("{{\"key\":{},\"name\":{}}}", json_str(i), json_str(&(n.name)(i)))).collect();

    // ---- the people: ideology, mood, issues
    let (mut people, mut militancy, mut consciousness) = (0.0, 0.0, 0.0);
    let mut by_ideology: BTreeMap<String, f64> = BTreeMap::new();
    let mut pop_size: BTreeMap<u64, f64> = BTreeMap::new();
    for p in s.provinces.values().filter(|p| p.owner.as_deref() == Some(tag)) {
        for q in &p.pops {
            people += q.size;
            militancy += q.size * q.militancy;
            consciousness += q.size * q.consciousness;
            pop_size.insert(q.id, q.size);
            for (k, share) in &q.ideology {
                if let Some(i) = k.checked_sub(1).and_then(|i| s.ideologies.get(i as usize)) {
                    *by_ideology.entry(i.clone()).or_default() += q.size * share / 100.0;
                }
            }
        }
    }
    let ideology_total: f64 = by_ideology.values().sum();
    by_ideology.values_mut().for_each(|v| *v /= ideology_total.max(1e-12));
    let people_ideology = (ideology_total > 0.0).then_some(&by_ideology);
    let avg = |x: f64| if people > 0.0 { Some(x / people) } else { None };
    let voters = politics::voter_ideologies(s, pr, tag);
    let house = (!c.upper_house.is_empty()).then_some(&c.upper_house);

    let wanted = reforms::wanted(s, tag);
    let mut issues: Vec<(&String, f64)> = wanted.iter().map(|(o, v)| (o, *v)).filter(|(_, v)| *v > 0.0).collect();
    issues.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
    let issues: Vec<String> = issues
        .iter()
        .take(ISSUES_SHOWN)
        .map(|(o, share)| {
            let group = group_of(o);
            format!("{{\"key\":{},\"name\":{},\"group\":{},\"group_name\":{},\"share\":{}}}", json_str(o), json_str(&(n.name)(o)), json_opt_str(group.as_deref()), json_opt_str(group.as_deref().map(|g| (n.name)(g)).as_deref()), json_num(*share))
        })
        .collect();

    // ---- reforms
    let groups: Vec<String> = rr
        .groups
        .iter()
        .map(|g| {
            let current = c.reforms.get(&g.name);
            let open: BTreeSet<String> = reforms::open_options(s, g, tag).into_iter().collect();
            let options: Vec<String> = g
                .options
                .iter()
                .map(|(o, _)| format!("{{\"key\":{},\"name\":{},\"current\":{},\"open\":{},\"wanted\":{}}}", json_str(o), json_str(&(n.name)(o)), current == Some(o), open.contains(o), json_num(wanted.get(o).copied().unwrap_or(0.0))))
                .collect();
            format!(
                "{{\"key\":{},\"name\":{},\"political\":{},\"next_step_only\":{},\"current\":{},\"options\":[{}]}}",
                json_str(&g.name),
                json_str(&(n.name)(&g.name)),
                g.political,
                g.next_step_only,
                json_opt_str(current.map(String::as_str)),
                options.join(",")
            )
        })
        .collect();
    let next_reform = c.last_reform.map(|d| months_after(d, rr.min_delay));
    let reforms = format!(
        "{{\"house_political\":{},\"house_social\":{},\"support_needed\":{},\"last_reform\":{},\"next_possible\":{},\"groups\":[{}]}}",
        json_num(reforms::house_support(s, rr, tag, true)),
        json_num(reforms::house_support(s, rr, tag, false)),
        json_num(rr.support_needed),
        date_or_null(c.last_reform),
        date_or_null(next_reform),
        groups.join(",")
    );

    // ---- movements and rebels
    let movements: Vec<String> = inp
        .unrest
        .into_iter()
        .flat_map(|u| u.movements_of(tag))
        .map(|m| {
            let (kind, target_name) = match m.kind {
                MovementKind::Issue => ("issue", (n.name)(&m.target)),
                MovementKind::Independence => ("independence", (n.country)(&m.target)),
            };
            format!("{{\"kind\":\"{kind}\",\"target\":{},\"target_name\":{},\"support\":{},\"radicalism\":{}}}", json_str(&m.target), json_str(&target_name), json_num(m.support), json_num(m.radicalism))
        })
        .collect();
    let factions: Vec<String> = inp
        .unrest
        .into_iter()
        .flat_map(|u| u.factions_of(tag))
        .map(|f| {
            let size: f64 = f.pops.iter().filter_map(|id| pop_size.get(id)).sum();
            let name = if f.name.is_empty() { (n.name)(&f.kind) } else { f.name.clone() };
            format!(
                "{{\"id\":{},\"kind\":{},\"kind_name\":{},\"name\":{},\"creates\":{},\"organisation\":{},\"pops\":{},\"people\":{}}}",
                f.id,
                json_str(&f.kind),
                json_str(&(n.name)(&f.kind)),
                json_str(&name),
                json_opt_str(f.creates.as_deref()),
                json_num(f.organization),
                f.pops.len(),
                json_num(size)
            )
        })
        .collect();

    // ---- countries it could release: cores on its land of countries that own none
    let owners: BTreeSet<&str> = s.provinces.values().filter_map(|p| p.owner.as_deref()).collect();
    let mut release: BTreeMap<&str, usize> = BTreeMap::new();
    for p in s.provinces.values().filter(|p| p.owner.as_deref() == Some(tag)) {
        for core in p.cores.iter().filter(|t| t.as_str() != tag && t.as_str() != inp.rebel_tag && !owners.contains(t.as_str())) {
            *release.entry(core.as_str()).or_default() += 1;
        }
    }
    let release: Vec<String> = release.iter().map(|(t, count)| format!("{{\"tag\":{},\"name\":{},\"provinces\":{count}}}", json_str(t), json_str(&(n.country)(t)))).collect();

    format!(
        "{{\"tag\":{},\"played\":{},\"date\":{},\"government\":{government},\"ruling_party\":{ruling},\"national_value\":{},\"last_election\":{},\"next_election\":{},\
\"population\":{},\"militancy\":{},\"consciousness\":{},\"ideologies\":[{}],\"upper_house\":{},\"voters\":{},\"people\":{},\"parties\":[{}],\"issues\":[{}],\
\"reforms\":{reforms},\"movements\":[{}],\"rebels\":[{}],\"rebel_armies\":{{\"armies\":{},\"regiments\":{}}},\"release\":[{}],\"can_reform\":false,\"can_release\":false}}",
        json_str(tag),
        c.human,
        json_str(&today.to_string()),
        json_opt_str(c.national_value.as_deref().map(|v| (n.name)(v)).as_deref()),
        date_or_null(c.last_election),
        date_or_null(politics::next_election(s, pr, tag)),
        json_num(people),
        json_opt(avg(militancy)),
        json_opt(avg(consciousness)),
        ideologies.join(","),
        ideology_shares(s, house),
        ideology_shares(s, voters.as_ref()),
        ideology_shares(s, people_ideology),
        parties.join(","),
        issues.join(","),
        movements.join(","),
        factions.join(","),
        inp.rebel_armies.0,
        inp.rebel_armies.1,
        release.join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiktoria_simulation::politics::{Government, PartyInfo};
    use wiktoria_simulation::reforms::ReformGroup;
    use wiktoria_simulation::script::{CountryState, PopState, ProvinceState};

    fn state() -> State {
        let mut s = State::new(Date::new(1840, 3, 5));
        s.ideologies = vec!["ideo_a".into(), "ideo_b".into(), "ideo_c".into()];
        s.issues = vec![("opt_one".into(), "group_x".into()), ("opt_two".into(), "group_x".into()), ("pol_one".into(), "party_y".into())];
        s.party_ideologies = vec!["ideo_a".into(), "ideo_b".into()];
        s.strata = [("workers".to_string(), "poor".to_string()), ("owners".to_string(), "rich".to_string())].into();
        let c = CountryState {
            government: Some("gov_one".into()),
            ruling_party: Some(2),
            last_election: Some(Date::new(1838, 1, 1)),
            reforms: [("group_x".to_string(), "opt_one".to_string()), ("franchise".to_string(), "some_vote".to_string())].into(),
            upper_house: [("ideo_a".to_string(), 0.75), ("ideo_b".to_string(), 0.25)].into(),
            human: true,
            ..Default::default()
        };
        s.countries.insert("AAA".into(), c);
        s.countries.insert("BBB".into(), CountryState::default());
        let pop = |id, t: &str, size, ideo: &[(u32, f64)]| PopState { id, pop_type: t.into(), size, militancy: 2.0, consciousness: 1.0, ideology: ideo.iter().copied().collect(), issues: [(2, 40.0), (3, 10.0)].into(), ..Default::default() };
        let p1 = ProvinceState { owner: Some("AAA".into()), cores: vec!["AAA".into(), "CCC".into()], pops: vec![pop(1, "workers", 3000.0, &[(1, 100.0)]), pop(2, "owners", 1000.0, &[(2, 100.0)])], ..Default::default() };
        let p2 = ProvinceState { owner: Some("BBB".into()), cores: vec!["BBB".into()], ..Default::default() };
        s.provinces.insert(1, p1);
        s.provinces.insert(2, p2);
        s
    }

    fn rules() -> (PoliticsRules, ReformRules) {
        let mut pr = PoliticsRules { campaign_months: 6, franchise_reform: "franchise".into(), ..Default::default() };
        pr.franchise.insert("some_vote".into(), [0.0, 0.5, 1.0]);
        pr.governments.insert("gov_one".into(), Government { election: true, duration: 48, ideologies: ["ideo_a".to_string(), "ideo_b".to_string()].into() });
        let party = |id, ideology: &str| PartyInfo { id, ideology: ideology.into(), start: None, end: None, min_tax: 0.0, max_tax: 1.0, max_military: 1.0, min_tariff: -1.0, max_tariff: 1.0, positions: vec!["pol_one".into()], citizenship: Default::default() };
        pr.parties.insert("AAA".into(), vec![party(1, "ideo_a"), party(2, "ideo_b")]);
        let rr = ReformRules { min_delay: 12, support_needed: 0.5, groups: vec![ReformGroup { name: "group_x".into(), political: false, next_step_only: true, options: vec![("opt_one".into(), None), ("opt_two".into(), None), ("opt_three".into(), None)] }], ..Default::default() };
        (pr, rr)
    }

    #[test]
    fn the_politics_answer_holds_what_the_state_has() {
        let s = state();
        let (pr, rr) = rules();
        let name = |k: &str| format!("N:{k}");
        let country = |t: &str| format!("C:{t}");
        let party = |id: u32| Some(format!("P{id}"));
        let names = Names { name: &name, country: &country, party: &party };
        let inp = Inputs { politics: &pr, reforms: &rr, unrest: None, rebel_armies: (1, 3), rebel_tag: "REB" };
        let j = build(&s, &inp, &names, "AAA");
        let has = |part: &str| assert!(j.contains(part), "{part} not in {j}");
        has("\"government\":{\"key\":\"gov_one\",\"name\":\"N:gov_one\",\"elections\":true}");
        has("\"ruling_party\":{\"id\":2,\"name\":\"P2\",\"ideology\":\"ideo_b\",\"ideology_name\":\"N:ideo_b\"}");
        // 1838.1 + 48 months + 6 months of campaign.
        has("\"next_election\":\"1842.7.1\"");
        has("\"upper_house\":[[\"ideo_a\",0.75],[\"ideo_b\",0.25],[\"ideo_c\",0.0]]");
        // People: 3000 of ideo_a, 1000 of ideo_b. Voters: the poor have no vote here.
        has("\"people\":[[\"ideo_a\",0.75],[\"ideo_b\",0.25],[\"ideo_c\",0.0]]");
        has("\"voters\":[[\"ideo_a\",0.0],[\"ideo_b\",1.0],[\"ideo_c\",0.0]]");
        has("\"population\":4000.0,\"militancy\":2.0,\"consciousness\":1.0");
        has("{\"key\":\"opt_two\",\"name\":\"N:opt_two\",\"group\":\"group_x\",\"group_name\":\"N:group_x\",\"share\":0.4}");
        has("\"policies\":[{\"group\":\"party_y\",\"group_name\":\"N:party_y\",\"option\":\"pol_one\",\"option_name\":\"N:pol_one\"}]");
        // Next step only: the second option is open, the third is not.
        has("{\"key\":\"opt_one\",\"name\":\"N:opt_one\",\"current\":true,\"open\":false,\"wanted\":0.0}");
        has("{\"key\":\"opt_two\",\"name\":\"N:opt_two\",\"current\":false,\"open\":true,\"wanted\":0.4}");
        has("{\"key\":\"opt_three\",\"name\":\"N:opt_three\",\"current\":false,\"open\":false,\"wanted\":0.0}");
        has("\"release\":[{\"tag\":\"CCC\",\"name\":\"C:CCC\",\"provinces\":1}]");
        has("\"rebel_armies\":{\"armies\":1,\"regiments\":3}");
        has("\"can_reform\":false,\"can_release\":false");
        assert_eq!(build(&s, &inp, &names, "ZZZ"), "null");
    }
}
