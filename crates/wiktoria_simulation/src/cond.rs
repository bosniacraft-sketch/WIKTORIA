//! Script conditions compiled once (a speed-up of `script::conditions_hold`, same answers).
//!
//! A trigger block becomes a small tree: `AND` / `OR` / `NOT` and the owner scope change become
//! nodes, the numbers are parsed once, and the conditions read most often (pop needs, averages,
//! war, the number of provinces...) become typed leaves. Every other entry stays the written key
//! and value and is asked of the string evaluator (`script::condition`), so behaviour is the
//! string evaluator's for anything this module does not know. Errors (unknown conditions) come
//! out exactly where the string evaluator gives them: an `AND` stops at the first entry that
//! fails, an `OR` / `NOT` checks every entry.
//!
//! `Facts` keeps figures that many conditions read and that cannot change while the state is
//! only read (provinces per owner, the pop averages of each country). It must be made again
//! whenever the state changes; callers make one for a batch of tests on an unchanged state.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock, RwLock};

use wiktoria_data::date::Date;
use wiktoria_data::script::{Block, Value};

use crate::script::{average, condition, country_of, is_tag, yes, CountryState, PopState, ProvinceState, Scope, State};

#[derive(Debug, Clone, Copy, PartialEq)]
enum PopField {
    Consciousness,
    Militancy,
    Literacy,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Need {
    Life,
    Everyday,
    Luxury,
}

fn field(p: &PopState, f: PopField) -> f64 {
    match f {
        PopField::Consciousness => p.consciousness,
        PopField::Militancy => p.militancy,
        PopField::Literacy => p.literacy,
    }
}

#[derive(Debug, Clone)]
enum Node {
    And(Vec<Node>),
    Or(Vec<Node>),
    Not(Vec<Node>),
    /// `owner = { }` / `country = { }`.
    Owner(Vec<Node>),
    /// consciousness / militancy / literacy (and the `average_` forms) with a number.
    PopAverage { key: String, field: PopField, at_least: f64 },
    /// life_needs / everyday_needs / luxury_needs with a number.
    PopNeed { key: String, need: Need, at_least: f64 },
    War { key: String, want: bool },
    NumOfCities { at_least: f64 },
    NationalValue { key: String, text: String },
    Civilized { key: String, want: bool },
    GreatPower { key: String, want: bool },
    /// `poor_strata_militancy = x` and the like: average over one strata's pops of the
    /// province (for a pop: of its province). Other scopes: the string evaluator.
    StrataAverage { key: String, value: Value, strata: usize, what: usize, at_least: f64 },
    /// `exists = TAG` / `exists = yes|no`.
    Exists { text: String },
    ColonialNation { key: String, want: bool },
    /// `TAG = { }`: the conditions asked of that country.
    Tag(Scope, Vec<Node>),
    /// `political_reform_want = x` / `social_reform_want = x` (`social`) of a country. Other
    /// scopes: the string evaluator.
    ReformWant { key: String, value: Value, social: bool, at_least: f64 },
    /// `is_primary_culture = yes|no` / `is_accepted_culture` (`accepted`) of a province: its
    /// largest culture against its owner's. Other scopes: the string evaluator.
    MajorityCulture { key: String, value: Value, accepted: bool, want: bool },
    Year { at_least: f64 },
    ProvinceId { key: String, id: f64 },
    TagIs { text: String },
    GlobalFlag { text: String },
    CountryFlag { key: String, text: String },
    CountryModifier { key: String, text: String },
    ProvinceModifier { key: String, text: String },
    /// `produces = good` of a province or a pop's province. Other scopes: the string evaluator.
    Produces { key: String, value: Value, text: String },
    /// `liberal = 10`, `jingoism = 5`: a number asked of a name the string evaluator reads as an
    /// ideology or issue when the state has one of that name (`reads_share`). Of a country: the
    /// support in percent among its people. Other scopes and other names: the string evaluator.
    Share { key: String, value: Value, lower: String, at_least: f64 },
    /// Anything else: the string evaluator.
    Raw(String, Value),
}

/// A compiled condition block (entries joined by "and", `factor` skipped) and its `factor`.
#[derive(Debug, Clone)]
pub struct Conditions {
    nodes: Vec<Node>,
    pub factor: Option<f64>,
}

impl Conditions {
    pub fn new(block: &Block) -> Conditions {
        Conditions { nodes: compile_items(block, true), factor: block.get("factor").and_then(Value::as_f64) }
    }

    /// The same answer as `script::conditions_hold(state, block, scope)`.
    pub fn hold(&self, state: &State, facts: Option<&Facts>, scope: &Scope) -> Result<bool, String> {
        all(state, facts, &self.nodes, scope)
    }
}

/// `factor` times the factors of the `modifier`s whose conditions hold (events' weight).
#[derive(Debug, Clone)]
pub struct Weight {
    pub factor: Option<f64>,
    pub modifiers: Vec<Conditions>,
}

impl Weight {
    pub fn new(block: &Block) -> Weight {
        Weight { factor: block.get("factor").and_then(Value::as_f64), modifiers: block.get_all("modifier").filter_map(Value::as_block).map(Conditions::new).collect() }
    }
}

fn compile_items(block: &Block, skip_factor: bool) -> Vec<Node> {
    block.items.iter().filter_map(|item| {
        let key = item.key.as_deref()?;
        if skip_factor && key == "factor" {
            return None;
        }
        Some(compile(key, &item.value))
    }).collect()
}

fn compile(key: &str, value: &Value) -> Node {
    let lower = key.to_ascii_lowercase();
    let raw = || Node::Raw(key.to_string(), value.clone());
    if let Value::Block(inner) = value {
        return match lower.as_str() {
            "and" => Node::And(compile_items(inner, true)),
            "or" => Node::Or(compile_items(inner, false)),
            "not" => Node::Not(compile_items(inner, false)),
            "owner" | "country" => Node::Owner(compile_items(inner, true)),
            _ if is_tag(key) => Node::Tag(Scope::Country(key.to_string()), compile_items(inner, true)),
            _ => raw(),
        };
    }
    let Some(text) = value.as_str() else { return raw() };
    // THIS / FROM stand for a country chosen when asked: the string evaluator.
    if text == "THIS" || text == "FROM" {
        return raw();
    }
    let number = value.as_f64();
    let want = yes(value);
    let k = key.to_string();
    match (lower.as_str(), number, want) {
        ("consciousness" | "average_consciousness", Some(x), _) => Node::PopAverage { key: k, field: PopField::Consciousness, at_least: x },
        ("militancy" | "average_militancy", Some(x), _) => Node::PopAverage { key: k, field: PopField::Militancy, at_least: x },
        ("literacy", Some(x), _) => Node::PopAverage { key: k, field: PopField::Literacy, at_least: x },
        ("life_needs", Some(x), _) => Node::PopNeed { key: k, need: Need::Life, at_least: x },
        ("everyday_needs", Some(x), _) => Node::PopNeed { key: k, need: Need::Everyday, at_least: x },
        ("luxury_needs", Some(x), _) => Node::PopNeed { key: k, need: Need::Luxury, at_least: x },
        ("war", _, Some(w)) => Node::War { key: k, want: w },
        ("num_of_cities", Some(x), _) => Node::NumOfCities { at_least: x },
        ("nationalvalue", _, _) => Node::NationalValue { key: k, text: text.to_string() },
        ("civilized", _, Some(w)) => Node::Civilized { key: k, want: w },
        ("is_greater_power", _, Some(w)) => Node::GreatPower { key: k, want: w },
        ("exists", _, _) => Node::Exists { text: text.to_string() },
        ("colonial_nation", _, Some(w)) => Node::ColonialNation { key: k, want: w },
        ("political_reform_want" | "social_reform_want", Some(x), _) => Node::ReformWant { key: k, value: value.clone(), social: lower.starts_with("social"), at_least: x },
        ("is_primary_culture" | "is_accepted_culture", _, Some(w)) => Node::MajorityCulture { key: k, value: value.clone(), accepted: lower == "is_accepted_culture", want: w },
        ("year", Some(x), _) => Node::Year { at_least: x },
        ("province_id", Some(x), _) => Node::ProvinceId { key: k, id: x },
        ("tag", _, _) => Node::TagIs { text: text.to_string() },
        ("has_global_flag", _, _) => Node::GlobalFlag { text: text.to_string() },
        ("has_country_flag", _, _) => Node::CountryFlag { key: k, text: text.to_string() },
        ("has_country_modifier", _, _) => Node::CountryModifier { key: k, text: text.to_string() },
        ("has_province_modifier", _, _) => Node::ProvinceModifier { key: k, text: text.to_string() },
        ("produces", _, _) => Node::Produces { key: k, value: value.clone(), text: text.to_string() },
        (s, Some(x), _) if strata_key(s).is_some() => {
            let (strata, what) = strata_key(s).unwrap_or((0, 0));
            Node::StrataAverage { key: k, value: value.clone(), strata, what, at_least: x }
        }
        (s, Some(x), _) if reads_share(s) => Node::Share { key: k, value: value.clone(), lower: lower.clone(), at_least: x },
        _ => raw(),
    }
}

/// Whether the string evaluator reads `name = number` as the support for an ideology or issue
/// called `name`, and not as one of its fixed conditions (they come first there). Asked of the
/// evaluator itself on a made-up state: one country with one pop that supports an ideology of
/// that name by a planted share. Only a reading of that share follows the planted numbers. The
/// state has the figures set that switch some fixed conditions on, so those stay fixed here.
/// Kept per name.
fn reads_share(lower: &str) -> bool {
    static KNOWN: Mutex<BTreeMap<String, bool>> = Mutex::new(BTreeMap::new());
    if let Some(known) = KNOWN.lock().ok().and_then(|m| m.get(lower).copied()) {
        return known;
    }
    let ask = |share: f64, x: f64| {
        let mut s = State::new(Date::new(1, 1, 1));
        s.ideologies = vec![lower.to_string()];
        (s.badboy_limit, s.colonial_rank) = (1.0, 1.0);
        s.culture_groups.insert("culture".to_string(), "group".to_string());
        s.countries.insert("AAA".to_string(), CountryState::default());
        let pop = PopState { size: 1.0, ideology: BTreeMap::from([(1, share)]), ..Default::default() };
        s.provinces.insert(1, ProvinceState { owner: Some("AAA".to_string()), pops: vec![pop], ..Default::default() });
        condition(&s, lower, &Value::Scalar(x.to_string()), &Scope::Country("AAA".to_string()))
    };
    let reads = ask(37.0, 37.0) == Ok(true) && ask(37.0, 38.0) == Ok(false) && ask(91.0, 38.0) == Ok(true) && ask(91.0, 92.0) == Ok(false);
    if let Ok(mut m) = KNOWN.lock() {
        m.insert(lower.to_string(), reads);
    }
    reads
}

const STRATA: [&str; 3] = ["poor", "middle", "rich"];
const STRATA_WHAT: [&str; 4] = ["militancy", "life_needs", "everyday_needs", "luxury_needs"];

fn strata_key(lower: &str) -> Option<(usize, usize)> {
    let (s, w) = lower.split_once("_strata_")?;
    Some((STRATA.iter().position(|x| *x == s)?, STRATA_WHAT.iter().position(|x| *x == w)?))
}

/// `conditions_hold`: stops at the first entry that fails or is unknown.
fn all(state: &State, facts: Option<&Facts>, nodes: &[Node], scope: &Scope) -> Result<bool, String> {
    for n in nodes {
        if !eval(state, facts, n, scope)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `any_holds`: every entry is checked, so an unknown one is always reported.
fn any(state: &State, facts: Option<&Facts>, nodes: &[Node], scope: &Scope) -> Result<bool, String> {
    let mut found = false;
    for n in nodes {
        found |= eval(state, facts, n, scope)?;
    }
    Ok(found)
}

fn pop_of<'a>(state: &'a State, scope: &Scope) -> Option<&'a PopState> {
    match scope {
        Scope::Pop(id, i) => state.provinces.get(id).and_then(|p| p.pops.get(*i)),
        _ => None,
    }
}

fn eval(state: &State, facts: Option<&Facts>, node: &Node, scope: &Scope) -> Result<bool, String> {
    let country = || country_of(state, scope).and_then(|t| state.countries.get(t));
    let known = |answer: Option<bool>, key: &str| answer.ok_or_else(|| key.to_string());
    match node {
        Node::And(nodes) => all(state, facts, nodes, scope),
        Node::Or(nodes) => any(state, facts, nodes, scope),
        Node::Not(nodes) => Ok(!any(state, facts, nodes, scope)?),
        Node::Owner(nodes) => match (scope, country_of(state, scope)) {
            (Scope::Province(_) | Scope::Pop(..) | Scope::State(..), Some(tag)) => all(state, facts, nodes, &Scope::Country(tag.to_string())),
            _ => Ok(false),
        },
        Node::PopAverage { key, field: f, at_least } => {
            let value = match (pop_of(state, scope), scope, facts) {
                (Some(p), _, _) => Some(field(p, *f)),
                (None, Scope::Country(t), Some(facts)) => Some(facts.country_average(state, t, *f)),
                _ => average(state, scope, |p| field(p, *f)),
            };
            known(value.map(|a| a >= *at_least), key)
        }
        Node::PopNeed { key, need, at_least } => known(
            pop_of(state, scope).map(|p| {
                let x = match need {
                    Need::Life => p.life_needs,
                    Need::Everyday => p.everyday_needs,
                    Need::Luxury => p.luxury_needs,
                };
                x >= *at_least
            }),
            key,
        ),
        Node::War { key, want } => known(country_of(state, scope).map(|t| !state.enemies(t).is_empty() == *want), key),
        Node::NumOfCities { at_least } => {
            let own = match scope {
                Scope::Country(t) => Some(t.as_str()),
                Scope::Pop(..) => country_of(state, scope),
                _ => None,
            };
            let n = own.map_or(0, |t| match facts {
                Some(f) => f.owned_count(state, t),
                None => state.owned(t).len(),
            });
            Ok(n as f64 >= *at_least)
        }
        Node::NationalValue { key, text } => known(country().map(|c| c.national_value.as_deref() == Some(text.as_str())), key),
        Node::Civilized { key, want } => known(country().map(|c| c.civilized == *want), key),
        Node::GreatPower { key, want } => known(country_of(state, scope).map(|t| state.great_powers.iter().any(|g| g == t) == *want), key),
        Node::StrataAverage { key, value, strata, what, at_least } => match (scope, facts) {
            (Scope::Country(t), Some(f)) => {
                let [people, sums @ ..] = f.country_strata_sums(state, t, *strata);
                Ok(people > 0.0 && sums[*what] / people >= *at_least)
            }
            (Scope::Province(id) | Scope::Pop(id, _), Some(f)) => {
                // The string evaluator: no province at all -> no pops -> false.
                let [people, sums @ ..] = f.strata_sums(state, *id, *strata);
                Ok(people > 0.0 && sums[*what] / people >= *at_least)
            }
            _ => condition(state, key, value, scope),
        },
        Node::Raw(key, value) => condition(state, key, value, scope),
        Node::Share { key, value, lower, at_least } => {
            // A technology or a pop type of that name comes first in the string evaluator; an
            // ideology goes before an issue.
            let which = match (scope, facts) {
                (Scope::Country(t), Some(f)) if !state.technologies.contains(lower) && !state.strata.contains_key(lower) => {
                    state.ideologies.iter().position(|i| i == lower).map(|n| (f, t, false, n)).or_else(|| state.issues.iter().position(|(option, _)| option == lower).map(|n| (f, t, true, n)))
                }
                _ => None,
            };
            match which {
                Some((f, tag, issue, n)) => Ok(f.share(state, tag, issue, n) >= *at_least),
                None => condition(state, key, value, scope),
            }
        }
        Node::Exists { text } => {
            let exists = |t: &str| match facts {
                Some(f) => f.owned_count(state, t) > 0,
                None => state.existing().contains(t),
            };
            Ok(if is_tag(text) { exists(text) } else { country_of(state, scope).is_some_and(exists) == (text != "no") })
        }
        Node::Tag(country, nodes) => all(state, facts, nodes, country),
        Node::ReformWant { key, value, social, at_least } => match (scope, facts) {
            (Scope::Country(t), Some(f)) => Ok(f.reform_want(state, t, *social) >= *at_least),
            _ => condition(state, key, value, scope),
        },
        Node::MajorityCulture { key, value, accepted, want } => match (scope, facts) {
            (Scope::Province(id), Some(f)) => known(
                f.majority_culture(state, *id).zip(country()).map(|(culture, c)| if *accepted { c.accepted_cultures.iter().any(|a| a == culture) } else { c.primary_culture.as_deref() == Some(culture) } == *want),
                key,
            ),
            _ => condition(state, key, value, scope),
        },
        Node::Year { at_least } => Ok(state.date.year as f64 >= *at_least),
        Node::ProvinceId { key, id } => match scope {
            Scope::Province(here) | Scope::Pop(here, _) => Ok(*here as f64 == *id),
            _ => Err(key.clone()),
        },
        // A province or a state has no country of its own: false there.
        Node::TagIs { text } => Ok(matches!(scope, Scope::Country(_) | Scope::Pop(..)) && country_of(state, scope) == Some(text.as_str())),
        Node::GlobalFlag { text } => Ok(state.global_flags.contains(text)),
        Node::CountryFlag { key, text } => known(country().map(|c| c.flags.contains(text)), key),
        Node::CountryModifier { key, text } => known(country().map(|c| c.modifiers.iter().any(|m| m.name == *text)), key),
        Node::ProvinceModifier { key, text } => match scope {
            Scope::Province(id) | Scope::Pop(id, _) => known(state.provinces.get(id).map(|p| p.modifiers.iter().any(|m| m.name == *text)), key),
            _ => Err(key.clone()),
        },
        Node::Produces { key, value, text } => match scope {
            Scope::Province(id) | Scope::Pop(id, _) => Ok(state.provinces.get(id).is_some_and(|p| p.trade_goods.as_deref() == Some(text.as_str()))),
            _ => condition(state, key, value, scope),
        },
        Node::ColonialNation { key, want } => known(
            country_of(state, scope).map(|t| {
                let colonial = match facts {
                    Some(f) => f.colonial_owner(state, t),
                    None => state.owned(t).iter().any(|id| state.provinces[id].colonial != 0),
                };
                colonial == *want
            }),
            key,
        ),
    }
}

/// Figures read by many conditions, made on first use. Valid only while the state they were
/// read from does not change.
#[derive(Default)]
pub struct Facts {
    owned: OnceLock<BTreeMap<String, usize>>,
    /// Per owner: (sum of size x consciousness, x militancy, x literacy, people), summed over its
    /// provinces in id order and their pops in order, as `script::average` sums them.
    averages: OnceLock<BTreeMap<String, [f64; 4]>>,
    /// Per province and strata: (people, size x militancy, x life, x everyday, x luxury needs),
    /// summed over the province's pops in order.
    strata: OnceLock<BTreeMap<u16, [[f64; 5]; 3]>>,
    /// Owners of at least one colonial province.
    colonial: OnceLock<std::collections::BTreeSet<String>>,
    /// Per owner and strata: as `strata`, summed over its provinces in id order.
    country_strata: OnceLock<BTreeMap<String, [[f64; 5]; 3]>>,
    /// Per owner asked so far: (sum of size x share wanting political reform, x social reform,
    /// people), summed as `script::average` sums them. Made country by country: the sums are
    /// costly and few countries are asked.
    reform_want: RwLock<BTreeMap<String, [f64; 3]>>,
    /// Per province: the culture with the most people.
    majority_culture: OnceLock<BTreeMap<u16, Option<String>>>,
    /// Per ideology or issue (`true`; by position in the state's list) and owner asked so far:
    /// the support in percent among the owner's people, averaged as `script::average` does.
    shares: RwLock<BTreeMap<(bool, usize), BTreeMap<String, f64>>>,
    /// Per owner: its provinces in id order.
    lands: OnceLock<BTreeMap<String, Vec<u16>>>,
}

/// The strata sums of `Facts::strata` with the pops of one more province added.
fn add_strata(state: &State, p: &ProvinceState, s: &mut [[f64; 5]; 3]) {
    for q in &p.pops {
        let k = STRATA.iter().position(|x| *x == state.strata.get(&q.pop_type).map_or("poor", String::as_str));
        let Some(k) = k else { continue };
        let e = &mut s[k];
        e[0] += q.size;
        e[1] += q.size * q.militancy;
        e[2] += q.size * q.life_needs;
        e[3] += q.size * q.everyday_needs;
        e[4] += q.size * q.luxury_needs;
    }
}

impl Facts {
    pub fn new() -> Facts {
        Facts::default()
    }

    fn colonial_owner(&self, state: &State, tag: &str) -> bool {
        self.colonial.get_or_init(|| state.provinces.values().filter(|p| p.colonial != 0).filter_map(|p| p.owner.clone()).collect()).contains(tag)
    }

    fn strata_sums(&self, state: &State, id: u16, strata: usize) -> [f64; 5] {
        let map = self.strata.get_or_init(|| {
            state
                .provinces
                .iter()
                .map(|(id, p)| {
                    let mut s = [[0.0; 5]; 3];
                    add_strata(state, p, &mut s);
                    (*id, s)
                })
                .collect()
        });
        map.get(&id).map_or([0.0; 5], |s| s[strata])
    }

    fn country_strata_sums(&self, state: &State, tag: &str, strata: usize) -> [f64; 5] {
        let map = self.country_strata.get_or_init(|| {
            let mut m: BTreeMap<String, [[f64; 5]; 3]> = BTreeMap::new();
            for p in state.provinces.values() {
                if let Some(o) = &p.owner {
                    add_strata(state, p, m.entry(o.clone()).or_insert([[0.0; 5]; 3]));
                }
            }
            m
        });
        map.get(tag).map_or([0.0; 5], |s| s[strata])
    }

    /// The share (0..1) of a country's people asking for political or social reform: the
    /// average over its pops of their issue shares for options of those reforms.
    fn reform_want(&self, state: &State, tag: &str, social: bool) -> f64 {
        let found = self.reform_want.read().ok().and_then(|m| m.get(tag).copied());
        let s = found.unwrap_or_else(|| {
            let mut e = [0.0; 3];
            for q in self.pops_of(state, tag) {
                // A pop's shares for political and for social reforms, each summed in the
                // issues' order.
                let mut want = [0.0; 2];
                for (n, share) in &q.issues {
                    match n.checked_sub(1).and_then(|i| state.issue_categories.get(i as usize)).map(String::as_str) {
                        Some("political_reforms") => want[0] += share,
                        Some("social_reforms") => want[1] += share,
                        _ => {}
                    }
                }
                e[0] += want[0] / 100.0 * q.size;
                e[1] += want[1] / 100.0 * q.size;
                e[2] += q.size;
            }
            if let Ok(mut m) = self.reform_want.write() {
                m.insert(tag.to_string(), e);
            }
            e
        });
        if s[2] > 0.0 { s[social as usize] / s[2] } else { 0.0 }
    }

    /// The support in percent for an ideology or issue (by position in the state's list) among a
    /// country's people.
    fn share(&self, state: &State, tag: &str, issue: bool, n: usize) -> f64 {
        if let Some(found) = self.shares.read().ok().and_then(|m| m.get(&(issue, n))?.get(tag).copied()) {
            return found;
        }
        let number = n as u32 + 1;
        let (mut sum, mut people) = (0.0, 0.0);
        for q in self.pops_of(state, tag) {
            sum += if issue { &q.issues } else { &q.ideology }.get(&number).copied().unwrap_or(0.0) * q.size;
            people += q.size;
        }
        let found = if people > 0.0 { sum / people } else { 0.0 };
        if let Ok(mut m) = self.shares.write() {
            m.entry((issue, n)).or_default().insert(tag.to_string(), found);
        }
        found
    }

    /// The pops of a country: its provinces in id order, their pops in order.
    fn pops_of<'a>(&self, state: &'a State, tag: &str) -> impl Iterator<Item = &'a PopState> + use<'a, '_> {
        let lands = self.lands.get_or_init(|| {
            let mut m: BTreeMap<String, Vec<u16>> = BTreeMap::new();
            for (id, p) in &state.provinces {
                if let Some(o) = &p.owner {
                    m.entry(o.clone()).or_default().push(*id);
                }
            }
            m
        });
        lands.get(tag).into_iter().flatten().filter_map(|id| state.provinces.get(id)).flat_map(|p| &p.pops)
    }

    /// The culture with the most people in a province (of equal ones the last by name), as the
    /// string evaluator picks it.
    fn majority_culture(&self, state: &State, id: u16) -> Option<&str> {
        let map = self.majority_culture.get_or_init(|| {
            state
                .provinces
                .iter()
                .map(|(id, p)| {
                    let mut sizes: BTreeMap<&str, f64> = BTreeMap::new();
                    for q in &p.pops {
                        *sizes.entry(q.culture.as_str()).or_default() += q.size;
                    }
                    (*id, sizes.into_iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|(c, _)| c.to_string()))
                })
                .collect()
        });
        map.get(&id)?.as_deref()
    }

    fn owned_count(&self, state: &State, tag: &str) -> usize {
        let map = self.owned.get_or_init(|| {
            let mut m: BTreeMap<String, usize> = BTreeMap::new();
            for p in state.provinces.values() {
                if let Some(o) = &p.owner {
                    *m.entry(o.clone()).or_default() += 1;
                }
            }
            m
        });
        map.get(tag).copied().unwrap_or(0)
    }

    fn country_average(&self, state: &State, tag: &str, f: PopField) -> f64 {
        let map = self.averages.get_or_init(|| {
            let mut m: BTreeMap<String, [f64; 4]> = BTreeMap::new();
            for p in state.provinces.values() {
                let Some(o) = &p.owner else { continue };
                let e = m.entry(o.clone()).or_insert([0.0; 4]);
                for q in &p.pops {
                    e[0] += q.consciousness * q.size;
                    e[1] += q.militancy * q.size;
                    e[2] += q.literacy * q.size;
                    e[3] += q.size;
                }
            }
            m
        });
        let Some(s) = map.get(tag) else { return 0.0 };
        let sum = match f {
            PopField::Consciousness => s[0],
            PopField::Militancy => s[1],
            PopField::Literacy => s[2],
        };
        if s[3] > 0.0 { sum / s[3] } else { 0.0 }
    }
}

/// The weight of `Weight`: `factor` (or `default`) combined with the factors of the modifiers
/// that hold. `product`: multiply (events, migration); Err names an unknown condition when
/// `strict`, else such a modifier is left out.
pub fn product(state: &State, facts: Option<&Facts>, w: &Weight, scope: &Scope) -> Result<f64, String> {
    let mut x = w.factor.unwrap_or(1.0);
    for m in &w.modifiers {
        if m.hold(state, facts, scope)? {
            x *= m.factor.unwrap_or(1.0);
        }
    }
    Ok(x)
}

/// `pops::product_weight` compiled: unknown conditions leave their modifier out; never below 0.
pub fn product_lenient(state: &State, facts: Option<&Facts>, w: &Weight, scope: &Scope) -> f64 {
    let mut x = w.factor.unwrap_or(1.0);
    for m in &w.modifiers {
        if m.hold(state, facts, scope) == Ok(true) {
            x *= m.factor.unwrap_or(1.0);
        }
    }
    x.max(0.0)
}

// ---------------------------------------------------------------- deep trees (the pop rules)
//
// The weight blocks of the monthly pop rules compiled one step further than above: the tree also
// follows `state_scope` and `location`, answers the conditions those blocks use most without the
// string evaluator (`Quick`), looks up the province, pop and country of a scope once (`Asked`)
// and keeps what was worked out for a country or a state while the state does not change
// (`Memo`). Everything else is still asked of `eval` / `script::condition`, so the answers are
// the string evaluator's.

use std::cell::{Cell, OnceCell};
use std::collections::HashMap;


type Answer = Result<bool, String>;

/// An answer whose "unknown" names the key without copying it where it can.
type Told<'t> = Result<bool, std::borrow::Cow<'t, str>>;

fn told<'t>(answer: Answer) -> Told<'t> {
    answer.map_err(std::borrow::Cow::Owned)
}

fn answer(told: Told) -> Answer {
    told.map_err(std::borrow::Cow::into_owned)
}

/// What is kept for the states of one region: one entry per owner.
type ByOwner<T> = Vec<(String, T)>;

/// Per strata: people, size x militancy, x life, x everyday, x luxury needs.
type StrataSums = [[f64; 5]; 3];

/// The strata sums of the pops of these provinces, province after province and pop after pop, as
/// the string evaluator adds them up for one strata at a time.
fn strata_sums<'a>(kinds: &[(String, Option<usize>)], provinces: impl Iterator<Item = &'a ProvinceState>) -> StrataSums {
    let mut s = [[0.0; 5]; 3];
    for q in provinces.flat_map(|p| &p.pops) {
        // A pop type the install does not list counts as poor.
        let Some(k) = kinds.iter().find(|x| x.0 == q.pop_type).map_or(Some(0), |x| x.1) else { continue };
        let e = &mut s[k];
        e[0] += q.size;
        e[1] += q.size * q.militancy;
        e[2] += q.size * q.life_needs;
        e[3] += q.size * q.everyday_needs;
        e[4] += q.size * q.luxury_needs;
    }
    s
}

/// The provinces of a state in the order `State::provinces_of` lists them: those of the region
/// that `tag` owns.
fn state_provinces<'a>(state: &'a State, region: u16, tag: &'a str) -> impl Iterator<Item = &'a ProvinceState> {
    let listed = state.region_members.get(&region);
    let unlisted = if listed.is_none() { Some(state.provinces.values().filter(move |p| p.region == Some(region))) } else { None };
    listed.into_iter().flatten().filter_map(|id| state.provinces.get(id)).chain(unlisted.into_iter().flatten()).filter(move |p| p.owner.as_deref() == Some(tag))
}

/// Answers kept while the state does not change (like `Facts`): what an entry asked of a country
/// or a state said, so that the next pop of the same place does not ask again, and the figures of
/// a state or a country that many entries read. Entries with the same text share their answers,
/// whichever weight they were written in. Make a new one whenever the state changes.
#[derive(Default)]
pub struct Memo {
    countries: RwLock<HashMap<u32, HashMap<String, Answer>>>,
    states: RwLock<HashMap<(u32, u16), ByOwner<Answer>>>,
    provinces: RwLock<HashMap<(u32, u16), Answer>>,
    /// Province -> (pop type, the share of its people of that type without work), and the share
    /// of all its people.
    unemployment: RwLock<HashMap<u16, Vec<(String, f64)>>>,
    province_unemployment: RwLock<HashMap<u16, f64>>,
    /// Region -> (owner, the share of the state's factory workers that its factories employ).
    factory_rates: RwLock<HashMap<u16, Vec<(String, f64)>>>,
    /// Region -> (owner, the share of the state's people without work).
    state_unemployment: RwLock<HashMap<u16, Vec<(String, f64)>>>,
    /// Country -> the share of its people without work.
    country_unemployment: RwLock<HashMap<String, f64>>,
    /// Region -> (owner, the strata sums of the state), and the same for one province.
    state_strata: RwLock<HashMap<u16, Vec<(String, StrataSums)>>>,
    province_strata: RwLock<HashMap<u16, StrataSums>>,
    /// Pop type -> the place of its strata in `STRATA` (None: a strata of another name).
    kinds: OnceLock<Vec<(String, Option<usize>)>>,
}

fn locked<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn reading<T>(m: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    m.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn writing<T>(m: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    m.write().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Memo {
    pub fn new() -> Memo {
        Memo::default()
    }

    fn country(&self, number: u32, tag: &str, ask: impl FnOnce() -> Answer) -> Answer {
        let found = reading(&self.countries).get(&number).and_then(|m| m.get(tag)).cloned();
        found.unwrap_or_else(|| {
            let answer = ask();
            writing(&self.countries).entry(number).or_default().insert(tag.to_string(), answer.clone());
            answer
        })
    }

    fn state(&self, number: u32, region: u16, tag: &str, ask: impl FnOnce() -> Answer) -> Answer {
        let found = reading(&self.states).get(&(number, region)).and_then(|v| v.iter().find(|x| x.0 == tag)).map(|x| x.1.clone());
        found.unwrap_or_else(|| {
            let answer = ask();
            writing(&self.states).entry((number, region)).or_default().push((tag.to_string(), answer.clone()));
            answer
        })
    }

    fn province(&self, number: u32, province: u16, ask: impl FnOnce() -> Answer) -> Answer {
        let found = reading(&self.provinces).get(&(number, province)).cloned();
        found.unwrap_or_else(|| {
            let answer = ask();
            writing(&self.provinces).insert((number, province), answer.clone());
            answer
        })
    }

    /// `State::factory_rate`: the share (0..1) of a state's factory workers that its open
    /// factories employ, each factory counting the places it has staffed.
    fn factory_rate(&self, state: &State, owner: Option<&str>, region: Option<u16>) -> f64 {
        let (Some(owner), Some(region)) = (owner, region) else { return 0.0 };
        let found = reading(&self.factory_rates).get(&region).and_then(|v| v.iter().find(|x| x.0 == owner)).map(|x| x.1);
        found.unwrap_or_else(|| {
            let staffed = |f: &crate::script::FactoryState| {
                let places = f.level * state.factory_types.get(&f.kind).map_or(0.0, |t| t.1);
                f.staff.map_or(places, |s| s.min(places))
            };
            let jobs: f64 = state.factories.iter().filter(|f| f.owner == owner && f.region == Some(region) && f.level >= 1.0 && !f.closed).map(staffed).sum();
            let people: f64 = state_provinces(state, region, owner).flat_map(|p| &p.pops).filter(|q| state.factory_worker_types.contains(&q.pop_type)).map(|q| q.size).sum();
            let rate = if people > 0.0 { (jobs / people).min(1.0) } else { 0.0 };
            writing(&self.factory_rates).entry(region).or_default().push((owner.to_string(), rate));
            rate
        })
    }

    /// `State::unemployment` of the people of one province, of one pop type or of all: the sums
    /// of `State::employment` for one province. The pop types that work in factories share the
    /// staffed places of their state's factories (looked up once per state here), the type that
    /// works the province's farm or mine (and slaves) the people it employs; everybody else has
    /// work.
    fn employment_in(&self, state: &State, p: &ProvinceState, kind: Option<&str>) -> f64 {
        let worker = p.trade_goods.as_ref().and_then(|g| state.rgo_workers.get(g));
        let on_farm = |q: &PopState| state.slave_types.contains(&q.pop_type) || worker == Some(&q.pop_type);
        let farm_people: f64 = p.pops.iter().filter(|q| on_farm(q)).map(|q| q.size).sum();
        let farm_rate = match p.rgo_staff {
            Some(staff) if farm_people > 0.0 => (staff / farm_people).clamp(0.0, 1.0),
            _ => 1.0,
        };
        let mut factory_rate = None;
        let (mut employed, mut people) = (0.0, 0.0);
        for q in p.pops.iter().filter(|q| kind.is_none_or(|k| q.pop_type == k)) {
            people += q.size;
            employed += q.size * if state.factory_worker_types.contains(&q.pop_type) { *factory_rate.get_or_insert_with(|| self.factory_rate(state, p.owner.as_deref(), p.region)) } else if on_farm(q) { farm_rate } else { 1.0 };
        }
        if people > 0.0 { 1.0 - employed / people } else { 0.0 }
    }

    /// `State::unemployment` of the people of one pop type in one province.
    fn unemployment(&self, state: &State, province: u16, kind: &str) -> f64 {
        let found = reading(&self.unemployment).get(&province).and_then(|v| v.iter().find(|x| x.0 == kind)).map(|x| x.1);
        found.unwrap_or_else(|| {
            let share = match state.provinces.get(&province) {
                Some(p) => self.employment_in(state, p, Some(kind)),
                None => state.unemployment(&[province], Some(kind)),
            };
            writing(&self.unemployment).entry(province).or_default().push((kind.to_string(), share));
            share
        })
    }

    /// `State::unemployment` of all the people of one province.
    fn province_unemployment(&self, state: &State, id: u16, p: &ProvinceState) -> f64 {
        let found = reading(&self.province_unemployment).get(&id).copied();
        found.unwrap_or_else(|| {
            let share = self.employment_in(state, p, None);
            writing(&self.province_unemployment).insert(id, share);
            share
        })
    }

    /// `State::unemployment` of all the people of one state.
    fn state_unemployment(&self, state: &State, region: u16, tag: &str) -> f64 {
        let found = reading(&self.state_unemployment).get(&region).and_then(|v| v.iter().find(|x| x.0 == tag)).map(|x| x.1);
        found.unwrap_or_else(|| {
            let share = state.unemployment(&state.provinces_of(&Scope::State(region, tag.to_string())).unwrap_or_default(), None);
            writing(&self.state_unemployment).entry(region).or_default().push((tag.to_string(), share));
            share
        })
    }

    /// `State::unemployment` of all the people of one country.
    fn country_unemployment(&self, state: &State, tag: &str) -> f64 {
        let found = reading(&self.country_unemployment).get(tag).copied();
        found.unwrap_or_else(|| {
            let share = state.unemployment(&state.owned(tag), None);
            writing(&self.country_unemployment).insert(tag.to_string(), share);
            share
        })
    }

    fn kinds(&self, state: &State) -> &[(String, Option<usize>)] {
        self.kinds.get_or_init(|| state.strata.iter().map(|(kind, strata)| (kind.clone(), STRATA.iter().position(|x| x == strata))).collect())
    }

    fn state_strata(&self, state: &State, region: u16, tag: &str) -> StrataSums {
        let found = reading(&self.state_strata).get(&region).and_then(|v| v.iter().find(|x| x.0 == tag)).map(|x| x.1);
        found.unwrap_or_else(|| {
            let s = strata_sums(self.kinds(state), state_provinces(state, region, tag));
            writing(&self.state_strata).entry(region).or_default().push((tag.to_string(), s));
            s
        })
    }

    fn province_strata(&self, state: &State, id: u16) -> StrataSums {
        let found = reading(&self.province_strata).get(&id).copied();
        found.unwrap_or_else(|| {
            let s = strata_sums(self.kinds(state), state.provinces.get(&id).into_iter());
            writing(&self.province_strata).insert(id, s);
            s
        })
    }
}

/// A scope with what it points at looked up once: its province, its pop, its country; and, made
/// when first asked for, the scopes the pop rules step to from it (the owner, the state, the
/// province of a pop) and its strata sums. Make one per pop (or province, or country) and ask
/// all its weights with it.
pub struct Asked<'a> {
    scope: Scope,
    province: Option<&'a ProvinceState>,
    pop: Option<&'a PopState>,
    country: Option<&'a CountryState>,
    owner: OnceCell<Option<Box<Asked<'a>>>>,
    in_state: OnceCell<Option<Box<Asked<'a>>>>,
    location: OnceCell<Box<Asked<'a>>>,
    strata: Cell<Option<StrataSums>>,
    /// The country is at war (`State::enemies` is not empty).
    war: Cell<Option<bool>>,
    /// The number of the ideology of the country's ruling party.
    ruling: Cell<Option<Option<u32>>>,
    /// The pop's issue support on political and on social reform options, in points of 100.
    reform_want: Cell<Option<[f64; 2]>>,
}

impl<'a> Asked<'a> {
    pub fn new(state: &'a State, scope: &Scope) -> Asked<'a> {
        let province = match scope {
            Scope::Province(id) | Scope::Pop(id, _) => state.provinces.get(id),
            _ => None,
        };
        let pop = match scope {
            Scope::Pop(_, i) => province.and_then(|p| p.pops.get(*i)),
            _ => None,
        };
        let tag = match scope {
            Scope::Country(tag) | Scope::State(_, tag) => Some(tag.as_str()),
            _ => province.and_then(|p| p.owner.as_deref()),
        };
        Asked { scope: scope.clone(), province, pop, country: tag.and_then(|t| state.countries.get(t)), owner: OnceCell::new(), in_state: OnceCell::new(), location: OnceCell::new(), strata: Cell::new(None), war: Cell::new(None), ruling: Cell::new(None), reform_want: Cell::new(None) }
    }

    /// `script::country_of`.
    fn tag(&self) -> Option<&str> {
        match &self.scope {
            Scope::Country(tag) | Scope::State(_, tag) => Some(tag),
            _ => self.province.and_then(|p| p.owner.as_deref()),
        }
    }

    /// The scope `owner = { }` / `country = { }` steps to: the country of a province, a pop or a
    /// state. None from a country, or without an owner.
    fn owner(&self, state: &'a State) -> Option<&Asked<'a>> {
        self.owner.get_or_init(|| if matches!(self.scope, Scope::Country(_)) { None } else { self.tag().map(|t| Box::new(Asked::new(state, &Scope::Country(t.to_string())))) }).as_deref()
    }

    /// The scope `state_scope = { }` steps to from a province or a pop: its region and owner.
    /// None when it has no region or no owner.
    fn in_state(&self, state: &'a State) -> Option<&Asked<'a>> {
        self.in_state.get_or_init(|| self.province.and_then(|p| Some(Box::new(Asked::new(state, &Scope::State(p.region?, p.owner.clone()?)))))).as_deref()
    }

    /// The scope `location = { }` steps to from a pop: its province.
    fn location(&self, state: &'a State, id: u16) -> &Asked<'a> {
        self.location.get_or_init(|| Box::new(Asked::new(state, &Scope::Province(id))))
    }

    fn at_war(&self, state: &State) -> Option<bool> {
        if self.war.get().is_none() {
            // A war with the country on one side and somebody on the other.
            self.war.set(self.tag().map(|t| state.wars.iter().any(|(a, d)| (a.contains(t) && !d.is_empty()) || (d.contains(t) && !a.is_empty()))));
        }
        self.war.get()
    }

    fn ruling_ideology(&self, state: &State) -> Option<u32> {
        if self.ruling.get().is_none() {
            self.ruling.set(Some(self.country.and_then(|c| {
                let name = state.party_ideologies.get(c.ruling_party?.checked_sub(1)? as usize)?;
                Some(state.ideologies.iter().position(|i| i == name)? as u32 + 1)
            })));
        }
        self.ruling.get().flatten()
    }

    fn reform_want(&self, state: &State, pop: &PopState, political: bool) -> f64 {
        if self.reform_want.get().is_none() {
            let mut sums = [-0.0; 2];
            for (n, share) in &pop.issues {
                match n.checked_sub(1).and_then(|i| state.issue_categories.get(i as usize)).map(String::as_str) {
                    Some("political_reforms") => sums[0] += share,
                    Some("social_reforms") => sums[1] += share,
                    _ => {}
                }
            }
            self.reform_want.set(Some(sums));
        }
        self.reform_want.get().map_or(0.0, |sums| sums[usize::from(!political)])
    }

    /// The strata sums of the state, or of the province (for a pop: of its province).
    fn strata(&self, state: &State, memo: &Memo) -> Option<StrataSums> {
        if self.strata.get().is_none() {
            self.strata.set(match &self.scope {
                Scope::State(region, tag) => Some(memo.state_strata(state, *region, tag)),
                Scope::Province(id) | Scope::Pop(id, _) => Some(memo.province_strata(state, *id)),
                Scope::Country(_) => None,
            });
        }
        self.strata.get()
    }
}

/// The number of a written entry: entries with the same text get the same number, so their
/// answers are kept together in a `Memo`.
fn entry_number(key: &str, value: &Value) -> u32 {
    fn write(key: Option<&str>, value: &Value, out: &mut String) {
        out.push_str(key.unwrap_or(""));
        out.push('\u{1}');
        match value {
            Value::Scalar(s) => out.push_str(s),
            Value::Block(b) => {
                out.push('\u{2}');
                for item in &b.items {
                    write(item.key.as_deref(), &item.value, out);
                }
                out.push('\u{3}');
            }
        }
        out.push('\u{4}');
    }
    static NUMBERS: Mutex<BTreeMap<String, u32>> = Mutex::new(BTreeMap::new());
    let mut text = String::new();
    write(Some(key), value, &mut text);
    let mut numbers = locked(&NUMBERS);
    let next = numbers.len() as u32;
    *numbers.entry(text).or_insert(next)
}

/// Every condition `script::condition` knows by name (the entries with a plain value). A key on
/// this list is never taken for a technology, a pop type, an ideology, an issue or a reform; a
/// key that is none of all these is unknown to the evaluator.
const NAMED: &[&str] = &[
    "tag", "is_next_reform", "agree_with_ruling_party", "produces", "has_building", "unemployment", "badboy", "num_of_revolts", "revolt_percentage", "lost_national", "rank", "is_secondary_power", "industrial_score",
    "military_score", "total_score", "involved_in_crisis", "crisis_exist", "truce_with", "constructing_cb_progress", "has_recently_lost_war", "administration_spending", "has_national_minority", "minorities", "is_overseas",
    "controlled_by_rebels", "has_empty_adjacent_province", "empty", "pop_majority_culture", "pop_majority_religion", "pop_majority_ideology", "pop_majority_issue", "is_cultural_union", "this_culture_union", "substate_of",
    "exists", "civilized", "is_greater_power", "war", "war_with", "is_ideology_enabled", "is_coastal", "life_rating", "is_state_religion", "recruited_percentage", "political_movement_strength", "social_movement_strength",
    "has_culture_core", "can_build_factory", "has_factories", "education_spending", "crime_fighting", "social_spending", "military_spending", "can_nationalize", "ai", "invention", "ruling_party_ideology", "nationalvalue",
    "tech_school", "part_of_sphere", "in_sphere", "is_sphere_leader_of", "is_vassal", "is_independant", "military_access", "is_our_vassal", "vassal_of", "alliance_with", "colonial_nation", "capital", "is_capital", "trade_goods",
    "controlled_by", "province_control_days", "has_country_flag", "always", "has_recent_imigration", "constructing_cb_type", "has_global_flag", "has_country_modifier", "has_province_modifier", "has_province_flag", "government", "prestige", "plurality",
    "war_exhaustion", "money", "num_of_cities", "neighbour", "year", "month", "province_id", "continent", "is_colonial", "is_core", "owned_by", "owns", "has_pop_culture", "culture", "religion", "strata", "political_reform_want",
    "social_reform_want", "poor_strata_militancy", "middle_strata_militancy", "rich_strata_militancy", "poor_strata_life_needs", "middle_strata_life_needs", "rich_strata_life_needs", "poor_strata_everyday_needs",
    "middle_strata_everyday_needs", "rich_strata_everyday_needs", "poor_strata_luxury_needs", "middle_strata_luxury_needs", "rich_strata_luxury_needs", "revanchism", "life_needs", "everyday_needs", "luxury_needs", "total_pops",
    "culture_has_union_tag", "has_pop_religion", "has_pop_type", "pop_type", "type", "is_slave", "is_primary_culture", "is_accepted_culture", "primary_culture", "is_culture_group", "consciousness", "average_consciousness",
    "militancy", "average_militancy", "literacy",
];

/// What a key that is not a named condition stands for in one install.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Meaning {
    /// A technology: `electricity = 1`.
    Technology,
    /// A pop type with a number: its share of the people.
    PopShare,
    /// An ideology (first number) or an issue option (second) with a number: its support.
    Support(Option<u32>, Option<u32>),
    /// A reform or party issue group: `slavery = no_slavery`.
    Reform,
    /// Nothing the evaluator knows.
    Unknown,
}

/// Which lists of names a `Meaning` was worked out from.
fn names_stamp(state: &State) -> [usize; 5] {
    [state.issues.as_ptr() as usize, state.issues.len(), state.ideologies.len(), state.technologies.len(), state.strata.len()]
}

/// The order `script::condition` tries the names in.
fn meaning(state: &State, key: &str, number: bool) -> Meaning {
    let (ideology, option) = (state.ideologies.iter().position(|i| i == key), state.issues.iter().position(|(o, _)| o == key));
    if state.technologies.contains(key) {
        Meaning::Technology
    } else if number && state.strata.contains_key(key) {
        Meaning::PopShare
    } else if number && (ideology.is_some() || option.is_some()) {
        Meaning::Support(ideology.map(|n| n as u32 + 1), option.map(|n| n as u32 + 1))
    } else if state.issues.iter().any(|(_, g)| g == key) {
        Meaning::Reform
    } else {
        Meaning::Unknown
    }
}

/// Conditions of the pop rules answered here without the string evaluator, for the scopes named
/// with each; any other scope is left to the evaluator.
#[derive(Debug, Clone)]
enum Quick {
    /// Pop: its strata.
    Strata(String),
    /// Pop: its culture / religion (`this` = the same as its country's).
    Culture(String),
    Religion(String),
    /// A budget slider of the country.
    Spending(String, f64),
    AgreeWithRulingParty(f64),
    /// Things the port does not have (revanchism, reform movements, recruited regiments): only
    /// `= 0` holds.
    Fixed(bool),
    /// Pop: its culture is the country's primary / an accepted one.
    PrimaryCulture(bool),
    AcceptedCulture(bool),
    /// The option, and its place in `State::issues` (with the lists it was looked up in).
    NextReform(String, OnceLock<([usize; 5], Option<usize>)>),
    IdeologyEnabled(String),
    WarExhaustion(f64),
    Plurality(f64),
    /// Pop: its religion is the country's.
    StateReligion(bool),
    /// Pop: the share of its issue support on political (true) / social reform options.
    ReformWant(bool, f64),
    RulingPartyIdeology(String),
    /// Pop: its type. With `true` also a province: it has a pop of the type.
    PopType(String, bool),
    /// Province or pop.
    Continent(String),
    Tag(String),
    Year(f64),
    Month(f64),
    ProvinceId(f64),
    LifeRating(f64),
    Coastal(bool),
    /// Province or pop: it is colonial or not, as asked. State: one of its provinces is.
    Colonial(bool),
    /// Province, pop or state.
    HasFactories(bool),
    /// Pop: a slave type. Province or state: in a slave state.
    Slave(bool),
    ProvinceModifier(String),
    CountryModifier(String),
    CountryFlag(String),
    GlobalFlag(String),
    Government(String),
    /// Province: its people.
    TotalPops(f64),
    /// Province or pop: what its farm or mine makes.
    Produces(String),
    /// Province: the capital of its owner.
    Capital(bool),
    /// The share of people without work: of a pop's type in its province, of a province, of a
    /// state, of a country.
    Unemployment(f64),
    /// State, province or pop (its province): `poor_strata_militancy = x` and the like (strata,
    /// what, number).
    StrataAverage(usize, usize, f64),
    /// Pop: its own consciousness, militancy or literacy; how far a tier of its needs is met.
    PopNumber(PopField, f64),
    PopNeed(Need, f64),
    Civilized(bool),
    War(bool),
    NationalValue(String),
    GreatPower(bool),
    /// `controlled_by = owner`.
    ControlledByOwner,
    /// A pop only (unknown anywhere else).
    HasCultureCore(bool),
    CanBuildFactory(bool),
    /// `any_neighbor_country = { }`: unknown anywhere but in a country.
    CountryOnly,
    /// Province: `unemployment_by_type = { type = x value = y }`, the share of the people of
    /// one pop type in it without work.
    UnemploymentByType(String, f64),
    /// Not a named condition: lower-case key, written text and number, and what the key stands
    /// for (with the lists it was looked up in).
    Name { lower: String, text: String, number: Option<f64>, meaning: OnceLock<([usize; 5], Meaning)> },
}

fn quick_of(lower: &str, value: &Value) -> Option<Quick> {
    if value.as_block().is_some() {
        return match (lower, value.as_block().and_then(|b| b.get("type")).and_then(Value::as_str), value.as_block().and_then(|b| b.get("value")).and_then(Value::as_f64)) {
            ("any_neighbor_country", _, _) => Some(Quick::CountryOnly),
            ("unemployment_by_type", Some(kind), Some(x)) => Some(Quick::UnemploymentByType(kind.to_string(), x)),
            _ => None,
        };
    }
    let text = value.as_str()?;
    if text == "THIS" || text == "FROM" {
        return None;
    }
    let t = || text.to_string();
    Some(match (lower, value.as_f64(), yes(value)) {
        ("strata", _, _) => Quick::Strata(t()),
        ("culture", _, _) => Quick::Culture(t()),
        ("religion", _, _) => Quick::Religion(t()),
        ("education_spending" | "crime_fighting" | "social_spending" | "military_spending", Some(x), _) => Quick::Spending(lower.to_string(), x),
        ("agree_with_ruling_party", Some(x), _) => Quick::AgreeWithRulingParty(x),
        ("revanchism" | "recruited_percentage" | "political_movement_strength" | "social_movement_strength", Some(x), _) => Quick::Fixed(x <= 0.0),
        ("is_primary_culture", _, Some(w)) => Quick::PrimaryCulture(w),
        ("is_accepted_culture", _, Some(w)) => Quick::AcceptedCulture(w),
        ("is_next_reform", _, _) => Quick::NextReform(t(), OnceLock::new()),
        ("is_ideology_enabled", _, _) => Quick::IdeologyEnabled(t()),
        ("war_exhaustion", Some(x), _) => Quick::WarExhaustion(x),
        ("plurality", Some(x), _) => Quick::Plurality(x),
        ("is_state_religion", _, Some(w)) => Quick::StateReligion(w),
        ("political_reform_want" | "social_reform_want", Some(x), _) => Quick::ReformWant(lower.starts_with("political"), x),
        ("ruling_party_ideology", _, _) => Quick::RulingPartyIdeology(t()),
        ("type", _, _) => Quick::PopType(t(), false),
        ("has_pop_type" | "pop_type", _, _) => Quick::PopType(t(), true),
        ("continent", _, _) => Quick::Continent(t()),
        ("tag", _, _) => Quick::Tag(t()),
        ("year", Some(x), _) => Quick::Year(x),
        ("month", Some(x), _) => Quick::Month(x),
        ("province_id", Some(x), _) => Quick::ProvinceId(x),
        ("life_rating", Some(x), _) => Quick::LifeRating(x),
        ("is_coastal", _, Some(w)) => Quick::Coastal(w),
        ("is_colonial", _, Some(w)) => Quick::Colonial(w),
        ("has_factories", _, Some(w)) => Quick::HasFactories(w),
        ("is_slave", _, Some(w)) => Quick::Slave(w),
        ("has_province_modifier", _, _) => Quick::ProvinceModifier(t()),
        ("has_country_modifier", _, _) => Quick::CountryModifier(t()),
        ("has_country_flag", _, _) => Quick::CountryFlag(t()),
        ("has_global_flag", _, _) => Quick::GlobalFlag(t()),
        ("government", _, _) => Quick::Government(t()),
        ("total_pops", Some(x), _) => Quick::TotalPops(x),
        ("produces", _, _) => Quick::Produces(t()),
        ("is_capital", _, Some(w)) => Quick::Capital(w),
        ("unemployment", Some(x), _) => Quick::Unemployment(x),
        (s, Some(x), _) if strata_key(s).is_some() => {
            let (strata, what) = strata_key(s).unwrap_or((0, 0));
            Quick::StrataAverage(strata, what, x)
        }
        ("consciousness" | "average_consciousness", Some(x), _) => Quick::PopNumber(PopField::Consciousness, x),
        ("militancy" | "average_militancy", Some(x), _) => Quick::PopNumber(PopField::Militancy, x),
        ("literacy", Some(x), _) => Quick::PopNumber(PopField::Literacy, x),
        ("life_needs", Some(x), _) => Quick::PopNeed(Need::Life, x),
        ("everyday_needs", Some(x), _) => Quick::PopNeed(Need::Everyday, x),
        ("luxury_needs", Some(x), _) => Quick::PopNeed(Need::Luxury, x),
        ("civilized", _, Some(w)) => Quick::Civilized(w),
        ("war", _, Some(w)) => Quick::War(w),
        ("nationalvalue", _, _) => Quick::NationalValue(t()),
        ("is_greater_power", _, Some(w)) => Quick::GreatPower(w),
        ("controlled_by", _, _) if text == "owner" => Quick::ControlledByOwner,
        ("has_culture_core", _, Some(w)) => Quick::HasCultureCore(w),
        ("can_build_factory", _, Some(w)) => Quick::CanBuildFactory(w),
        (k, number, _) if !NAMED.contains(&k) => Quick::Name { lower: k.to_string(), text: t(), number, meaning: OnceLock::new() },
        _ => return None,
    })
}

/// What a quick form says: the answer, "unknown" (the evaluator's error for the entry), or
/// nothing (ask the evaluator).
enum Said {
    Is(bool),
    Unknown,
    Ask,
}

/// The answer of `script::condition` for the cases named at `Quick`.
fn quick(state: &State, memo: &Memo, q: &Quick, at: &Asked) -> Said {
    let scope = &at.scope;
    let (province, pop, country) = (at.province, at.pop, at.country);
    let place = matches!(scope, Scope::Province(_) | Scope::Pop(..));
    // For a state: its provinces.
    let in_state = || match scope {
        Scope::State(region, tag) => Some(state_provinces(state, *region, tag)),
        _ => None,
    };
    let answer: Option<bool> = match q {
        Quick::Strata(text) => pop.map(|p| state.strata.get(&p.pop_type).map(String::as_str) == Some(text.as_str())),
        Quick::Culture(text) if text.eq_ignore_ascii_case("this") => pop.zip(country).map(|(p, c)| c.primary_culture.as_deref() == Some(p.culture.as_str())),
        Quick::Culture(text) => pop.map(|p| p.culture == *text),
        Quick::Religion(text) if text.eq_ignore_ascii_case("this") => pop.zip(country).map(|(p, c)| c.religion.as_deref() == Some(p.religion.as_str())),
        Quick::Religion(text) => pop.map(|p| p.religion == *text),
        Quick::Spending(key, x) => country.and_then(|c| c.spending.get(key)).map(|s| s >= x),
        Quick::AgreeWithRulingParty(x) => Some(pop.zip(at.ruling_ideology(state)).is_some_and(|(p, n)| p.ideology.get(&n).copied().unwrap_or(0.0) / 100.0 >= *x)),
        Quick::Fixed(answer) => Some(*answer),
        Quick::PrimaryCulture(want) => pop.zip(country).map(|(p, c)| (c.primary_culture.as_deref() == Some(p.culture.as_str())) == *want),
        Quick::AcceptedCulture(want) => pop.zip(country).map(|(p, c)| c.accepted_cultures.contains(&p.culture) == *want),
        Quick::NextReform(text, kept) => Some(country.is_some_and(|c| {
            let place = || state.issues.iter().position(|(o, _)| o == text);
            let stamp = names_stamp(state);
            let (made_for, mut found) = *kept.get_or_init(|| (stamp, place()));
            if made_for != stamp {
                found = place();
            }
            found.filter(|i| *i > 0).is_some_and(|i| {
                let (before, this) = (&state.issues[i - 1], &state.issues[i]);
                before.1 == this.1 && state.issue_categories.get(i).is_some_and(|k| k == "political_reforms" || k == "social_reforms") && c.reforms.get(&this.1) == Some(&before.0)
            })
        })),
        Quick::IdeologyEnabled(text) => state.ideologies.iter().any(|i| i == text).then(|| state.enabled_ideologies.contains(text) || state.ideology_dates.get(text).is_none_or(|d| state.date >= *d)),
        Quick::WarExhaustion(x) => country.map(|c| c.war_exhaustion >= *x),
        Quick::Plurality(x) => country.map(|c| c.plurality >= *x),
        Quick::StateReligion(want) => pop.zip(country).map(|(p, c)| (c.religion.as_deref() == Some(p.religion.as_str())) == *want),
        Quick::ReformWant(political, x) => pop.map(|p| at.reform_want(state, p, *political) / 100.0 >= *x),
        Quick::RulingPartyIdeology(text) => country.and_then(|c| Some(state.party_ideologies.get(c.ruling_party?.checked_sub(1)? as usize)? == text)),
        Quick::PopType(text, in_province) => match scope {
            Scope::Pop(..) => pop.map(|p| p.pop_type == *text),
            Scope::Province(_) if *in_province => province.map(|p| p.pops.iter().any(|q| q.pop_type == *text)),
            _ => None,
        },
        Quick::Continent(text) => province.map(|p| p.continent.as_deref() == Some(text.as_str())),
        Quick::Tag(text) => Some(matches!(scope, Scope::Country(_) | Scope::Pop(..)) && at.tag() == Some(text.as_str())),
        Quick::Year(x) => Some(state.date.year as f64 >= *x),
        Quick::Month(x) => Some(state.date.month as f64 >= *x),
        Quick::ProvinceId(x) => match scope {
            Scope::Province(id) | Scope::Pop(id, _) => Some(*id as f64 == *x),
            _ => None,
        },
        Quick::LifeRating(x) => province.map(|p| p.life_rating >= *x),
        Quick::Coastal(want) => province.map(|p| p.coastal == *want),
        Quick::Colonial(want) => province.map(|p| (p.colonial != 0) == *want).or_else(|| in_state().map(|mut all| all.any(|p| (p.colonial != 0) == *want))),
        Quick::HasFactories(want) => province.map(|p| p.has_factory == *want).or_else(|| in_state().map(|mut all| all.any(|p| p.has_factory) == *want)),
        Quick::Slave(want) => match scope {
            Scope::Pop(..) => pop.map(|p| state.slave_types.contains(&p.pop_type) == *want),
            _ => province.map(|p| p.slave_state == *want).or_else(|| in_state().map(|mut all| all.any(|p| p.slave_state) == *want)),
        },
        Quick::ProvinceModifier(text) => province.map(|p| p.modifiers.iter().any(|m| m.name == *text)),
        Quick::CountryModifier(text) => country.map(|c| c.modifiers.iter().any(|m| m.name == *text)),
        Quick::CountryFlag(text) => country.map(|c| c.flags.contains(text)),
        Quick::GlobalFlag(text) => Some(state.global_flags.contains(text)),
        Quick::Government(text) => country.map(|c| c.government.as_deref() == Some(text.as_str())),
        Quick::TotalPops(x) => match scope {
            Scope::Province(_) => province.map(|p| p.pops.iter().map(|q| q.size).sum::<f64>() >= *x),
            _ => None,
        },
        Quick::Produces(text) => place.then(|| province.is_some_and(|p| p.trade_goods.as_deref() == Some(text.as_str()))),
        Quick::Capital(want) => match scope {
            Scope::Province(id) => country.map(|c| (c.capital == Some(*id)) == *want),
            _ => None,
        },
        Quick::Unemployment(x) => match scope {
            Scope::Pop(id, _) => pop.map(|p| memo.unemployment(state, *id, &p.pop_type) >= *x),
            Scope::State(region, tag) => Some(memo.state_unemployment(state, *region, tag) >= *x),
            Scope::Country(tag) => Some(memo.country_unemployment(state, tag) >= *x),
            Scope::Province(id) => province.map(|p| memo.province_unemployment(state, *id, p) >= *x),
        },
        Quick::StrataAverage(strata, what, x) => at.strata(state, memo).map(|sums| {
            let [people, sums @ ..] = sums[*strata];
            people > 0.0 && sums[*what] / people >= *x
        }),
        Quick::PopNumber(f, x) => pop.map(|p| field(p, *f) >= *x),
        Quick::PopNeed(need, x) => pop.map(|p| {
            let met = match need {
                Need::Life => p.life_needs,
                Need::Everyday => p.everyday_needs,
                Need::Luxury => p.luxury_needs,
            };
            met >= *x
        }),
        Quick::Civilized(want) => country.map(|c| c.civilized == *want),
        Quick::War(want) => at.at_war(state).map(|war| war == *want),
        Quick::NationalValue(text) => country.map(|c| c.national_value.as_deref() == Some(text.as_str())),
        Quick::GreatPower(want) => at.tag().map(|t| state.great_powers.iter().any(|g| g == t) == *want),
        Quick::ControlledByOwner => province.map(|p| p.owner.is_some() && p.controller == p.owner),
        Quick::HasCultureCore(want) => {
            return match pop.zip(province) {
                Some((q, p)) => Said::Is(p.cores.iter().any(|t| state.countries.get(t).is_some_and(|c| c.primary_culture.as_deref() == Some(q.culture.as_str()))) == *want),
                None => Said::Unknown,
            };
        }
        // rules file: FLD_CAN_BUILD_FACTORY_CASH - see `script::condition`.
        Quick::CanBuildFactory(want) => return pop.zip(country).map_or(Said::Unknown, |(q, c)| Said::Is((q.money >= 6.25 && c.civilized) == *want)),
        Quick::CountryOnly => return if matches!(scope, Scope::Country(_)) { Said::Ask } else { Said::Unknown },
        Quick::UnemploymentByType(kind, x) => match scope {
            Scope::Province(id) => Some(memo.unemployment(state, *id, kind) >= *x),
            _ => None,
        },
        Quick::Name { lower, text, number, meaning: kept } => {
            let stamp = names_stamp(state);
            let (made_for, mut what) = *kept.get_or_init(|| (stamp, meaning(state, lower, number.is_some())));
            if made_for != stamp {
                what = meaning(state, lower, number.is_some());
            }
            match (what, number) {
                (Meaning::Unknown, _) => return Said::Unknown,
                (Meaning::Technology, Some(x)) => country.map(|c| c.technologies.contains(lower) == (*x >= 1.0)),
                (Meaning::PopShare, Some(x)) if place || matches!(scope, Scope::State(..)) => {
                    let (mut of_kind, mut all) = (0.0, 0.0);
                    for q in province.into_iter().chain(in_state().into_iter().flatten()).flat_map(|p| &p.pops) {
                        all += q.size;
                        if q.pop_type == *lower {
                            of_kind += q.size;
                        }
                    }
                    Some(all > 0.0 && of_kind / all >= *x)
                }
                (Meaning::Support(ideology, option), Some(x)) => pop.map(|p| {
                    let share = match (ideology, option) {
                        (Some(n), _) => p.ideology.get(&n).copied().unwrap_or(0.0),
                        (None, Some(n)) => p.issues.get(&n).copied().unwrap_or(0.0),
                        (None, None) => 0.0,
                    };
                    share >= *x
                }),
                (Meaning::Reform, _) => country.map(|c| c.reforms.get(lower).map(String::as_str) == Some(text.as_str())),
                _ => None,
            }
        }
    };
    answer.map_or(Said::Ask, Said::Is)
}

/// A condition tree that also follows `state_scope` and `location`.
#[derive(Debug, Clone)]
enum Deep {
    And(Vec<Deep>),
    Or(Vec<Deep>),
    Not(Vec<Deep>),
    /// `owner = { }` / `country = { }`, with its entry number.
    Owner(u32, Vec<Deep>),
    /// `state_scope = { }` (asked of a province or a pop) and `location = { }` (of a pop), each
    /// with the key as written.
    StateScope(String, Vec<Deep>),
    Location(String, Vec<Deep>),
    /// `any_owned_province = { }` (asked of a country or a state), with its entry number and
    /// the key as written: one of the provinces passes. Every province is asked.
    AnyOwnedProvince(u32, String, Vec<Deep>),
    Leaf(Box<Leaf>),
}

/// Any other entry: its number, its key as written, whether its answer for a province is worth
/// keeping, the quick form when there is one, and the entry as `compile` reads it.
#[derive(Debug, Clone)]
struct Leaf {
    number: u32,
    key: String,
    slow_in_province: bool,
    quick: Option<Quick>,
    node: Node,
}

/// Entries whose answer for a province takes a walk over its pops.
const SLOW_IN_PROVINCE: [&str; 5] = ["is_primary_culture", "is_accepted_culture", "culture", "unemployment", "minorities"];

fn deep_items(block: &Block, skip_factor: bool) -> Vec<Deep> {
    block.items.iter().filter_map(|item| {
        let key = item.key.as_deref()?;
        if skip_factor && key == "factor" {
            return None;
        }
        Some(deep(key, &item.value))
    }).collect()
}

fn deep(key: &str, value: &Value) -> Deep {
    let lower = key.to_ascii_lowercase();
    if let Value::Block(inner) = value {
        match lower.as_str() {
            "and" => return Deep::And(deep_items(inner, true)),
            "or" => return Deep::Or(deep_items(inner, false)),
            "not" => return Deep::Not(deep_items(inner, false)),
            "owner" | "country" => return Deep::Owner(entry_number(key, value), deep_items(inner, true)),
            "state_scope" => return Deep::StateScope(key.to_string(), deep_items(inner, true)),
            "location" => return Deep::Location(key.to_string(), deep_items(inner, true)),
            "any_owned_province" => return Deep::AnyOwnedProvince(entry_number(key, value), key.to_string(), deep_items(inner, true)),
            _ => {}
        }
    }
    let slow_in_province = matches!(value, Value::Block(_)) || SLOW_IN_PROVINCE.contains(&lower.as_str());
    Deep::Leaf(Box::new(Leaf { number: entry_number(key, value), key: key.to_string(), slow_in_province, quick: quick_of(&lower, value), node: compile(key, value) }))
}

fn deep_all<'a, 't>(state: &'a State, facts: Option<&Facts>, memo: &Memo, nodes: &'t [Deep], at: &Asked<'a>) -> Told<'t> {
    for n in nodes {
        if !deep_eval(state, facts, memo, n, at)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn deep_any<'a, 't>(state: &'a State, facts: Option<&Facts>, memo: &Memo, nodes: &'t [Deep], at: &Asked<'a>) -> Told<'t> {
    let mut found = false;
    for n in nodes {
        found |= deep_eval(state, facts, memo, n, at)?;
    }
    Ok(found)
}

fn deep_eval<'a, 't>(state: &'a State, facts: Option<&Facts>, memo: &Memo, node: &'t Deep, at: &Asked<'a>) -> Told<'t> {
    match node {
        Deep::And(nodes) => deep_all(state, facts, memo, nodes, at),
        Deep::Or(nodes) => deep_any(state, facts, memo, nodes, at),
        Deep::Not(nodes) => Ok(!deep_any(state, facts, memo, nodes, at)?),
        Deep::Owner(number, nodes) => match at.owner(state) {
            Some(owner) => told(memo.country(*number, owner.tag().unwrap_or(""), || answer(deep_all(state, facts, memo, nodes, owner)))),
            None => Ok(false),
        },
        Deep::StateScope(key, nodes) => match at.scope {
            Scope::Province(_) | Scope::Pop(..) => match at.in_state(state) {
                Some(inner) => deep_all(state, facts, memo, nodes, inner),
                None => Ok(false),
            },
            _ => Err(key.as_str().into()),
        },
        Deep::Location(key, nodes) => match at.scope {
            Scope::Pop(id, _) => deep_all(state, facts, memo, nodes, at.location(state, id)),
            _ => Err(key.as_str().into()),
        },
        Deep::AnyOwnedProvince(number, key, nodes) => {
            let any = || {
                let mut any = false;
                for id in state.provinces_of(&at.scope).unwrap_or_default() {
                    any |= answer(deep_all(state, facts, memo, nodes, &Asked::new(state, &Scope::Province(id))))?;
                }
                Ok(any)
            };
            match &at.scope {
                Scope::Country(tag) => told(memo.country(*number, tag, any)),
                Scope::State(region, tag) => told(memo.state(*number, *region, tag, any)),
                _ => Err(key.as_str().into()),
            }
        }
        Deep::Leaf(leaf) => {
            match leaf.quick.as_ref().map_or(Said::Ask, |q| quick(state, memo, q, at)) {
                Said::Is(answer) => return Ok(answer),
                Said::Unknown => return Err(leaf.key.as_str().into()),
                Said::Ask => {}
            }
            let ask = || eval(state, facts, &leaf.node, &at.scope);
            told(match &at.scope {
                Scope::Country(tag) => memo.country(leaf.number, tag, ask),
                Scope::State(region, tag) => memo.state(leaf.number, *region, tag, ask),
                Scope::Province(id) if leaf.slow_in_province => memo.province(leaf.number, *id, ask),
                _ => ask(),
            })
        }
    }
}

/// A condition block compiled as a deep tree (entries joined by "and", `factor` skipped) and its
/// `factor`.
#[derive(Debug, Clone)]
pub struct DeepConditions {
    nodes: Vec<Deep>,
    pub factor: Option<f64>,
}

impl DeepConditions {
    pub fn new(block: &Block) -> DeepConditions {
        DeepConditions { nodes: deep_items(block, true), factor: block.get("factor").and_then(Value::as_f64) }
    }

    /// The same answer as `script::conditions_hold(state, block, scope)` for the scope of `at`.
    /// `at` must have been made from `state`.
    pub fn hold<'a>(&self, state: &'a State, facts: Option<&Facts>, memo: &Memo, at: &Asked<'a>) -> Result<bool, String> {
        answer(self.told(state, facts, memo, at))
    }

    fn told<'a>(&self, state: &'a State, facts: Option<&Facts>, memo: &Memo, at: &Asked<'a>) -> Told<'_> {
        deep_all(state, facts, memo, &self.nodes, at)
    }
}

/// `pops::product_weight` compiled: `factor` times the factors of the `modifier`s that hold.
#[derive(Debug, Clone)]
pub struct DeepProduct {
    factor: Option<f64>,
    modifiers: Vec<DeepConditions>,
}

impl DeepProduct {
    pub fn new(block: &Block) -> DeepProduct {
        DeepProduct { factor: block.get("factor").and_then(Value::as_f64), modifiers: block.get_all("modifier").filter_map(Value::as_block).map(DeepConditions::new).collect() }
    }

    /// The same number as `pops::product_weight(state, block, scope)`: unknown conditions leave
    /// their modifier out; never below 0.
    pub fn weight<'a>(&self, state: &'a State, facts: Option<&Facts>, memo: &Memo, at: &Asked<'a>) -> f64 {
        let mut x = self.factor.unwrap_or(1.0);
        for m in &self.modifiers {
            if matches!(m.told(state, facts, memo, at), Ok(true)) {
                x *= m.factor.unwrap_or(1.0);
            }
        }
        x.max(0.0)
    }
}

/// One entry of an additive weight: a loose `modifier`, or a `group` with its own factor and
/// its modifiers in order.
#[derive(Debug, Clone)]
enum Part {
    Modifier(DeepConditions),
    Group(f64, Vec<DeepConditions>),
}

/// `pops::additive_weight` compiled: the base plus the factors of the modifiers that hold, a
/// group stopping at its first modifier that fails.
#[derive(Debug, Clone, Default)]
pub struct DeepSum {
    /// The block's own `factor`.
    pub base: f64,
    parts: Vec<Part>,
}

impl DeepSum {
    pub fn new(block: &Block) -> DeepSum {
        let factor = |b: &Block| b.get("factor").and_then(Value::as_f64).unwrap_or(0.0);
        let parts = block.items.iter().filter_map(|item| match (item.key.as_deref(), item.value.as_block()) {
            (Some("modifier"), Some(m)) => Some(Part::Modifier(DeepConditions::new(m))),
            (Some("group"), Some(g)) => Some(Part::Group(factor(g), g.get_all("modifier").filter_map(Value::as_block).map(DeepConditions::new).collect())),
            _ => None,
        });
        DeepSum { base: factor(block), parts: parts.collect() }
    }

    /// The same number as `pops::additive_weight(state, block, scope)`.
    pub fn weight<'a>(&self, state: &'a State, facts: Option<&Facts>, memo: &Memo, at: &Asked<'a>) -> f64 {
        let mut total = self.base;
        for part in &self.parts {
            match part {
                Part::Modifier(m) => {
                    if matches!(m.told(state, facts, memo, at), Ok(true)) {
                        total += m.factor.unwrap_or(0.0);
                    }
                }
                Part::Group(factor, modifiers) => {
                    total += factor;
                    for m in modifiers {
                        match m.told(state, facts, memo, at) {
                            Ok(true) => total += m.factor.unwrap_or(0.0),
                            Ok(false) => break,
                            Err(_) => {}
                        }
                    }
                }
            }
        }
        total
    }
}
