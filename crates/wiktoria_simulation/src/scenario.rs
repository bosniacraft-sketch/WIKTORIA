//! Test scenarios: small, ready-to-load setups that each exercise one mechanic (docs/SCENARIOS.md).
//!
//! A scenario file is written in the install's script syntax (`wiktoria_data::script`). It names a base
//! save, a seat, how long to run, a `setup` block applied after the save is loaded and before the
//! first day, and `check` blocks evaluated on given days or at the end, each giving PASS or FAIL
//! with the value it measured.
//!
//! The setup reuses the script effect runner (`script::run_effects`) for everything the effect
//! language has (money, prestige, modifiers, relations, militancy through pop scopes...). The few
//! commands it lacks (place an army or a fleet, start a war, set a controller or a fort, set a
//! pop's money, give a held casus belli, play a country) are implemented here. Nothing here
//! changes how the simulation runs: the setup only edits the state before day 1, and the checks
//! only read it.

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_data::date::Date;
use wiktoria_data::script::{Block, Value};

use crate::calendar::Calendar;
use crate::game::{Core, CoreLog, DayLog, Game};
use crate::military::battle::CombatKind;
use crate::military::units::{Army, Navy, Regiment, Ship};
use crate::military::war::{self, War, WarEvent, WarEventKind, WarGoal};
use crate::player::Slider;
use crate::politics::PoliticsRules;
use crate::script::{EffectRules, Report, Scope, State};

/// Days a scenario runs headless when the file does not say.
pub const DEFAULT_DAYS: u32 = 90;

/// A province named in a scenario: by its number, or as the capital of a country
/// (`capital:BEL`), which stays right when ids are not at hand.
#[derive(Debug, Clone, PartialEq)]
pub enum Place {
    Id(u16),
    Capital(String),
}

impl Place {
    pub fn parse(s: &str) -> Result<Place, String> {
        if let Some(tag) = s.strip_prefix("capital:") {
            return Ok(Place::Capital(tag.to_uppercase()));
        }
        s.parse().map(Place::Id).map_err(|_| format!("'{s}' is not a province (a number or capital:TAG)"))
    }

    /// The province number in this state.
    pub fn resolve(&self, state: &State) -> Result<u16, String> {
        match self {
            Place::Id(id) if state.provinces.contains_key(id) => Ok(*id),
            Place::Id(id) => Err(format!("province {id} is not in the save")),
            Place::Capital(tag) => state.countries.get(tag).ok_or_else(|| format!("no country {tag} in the save"))?.capital.ok_or_else(|| format!("{tag} has no capital")),
        }
    }
}

impl std::fmt::Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Place::Id(id) => write!(f, "{id}"),
            Place::Capital(tag) => write!(f, "capital:{tag}"),
        }
    }
}

/// One entry of the `setup` block.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// `TAG = { ... }` or `<province> = { ... }`: effects in the install's language.
    CountryEffects { tag: String, block: Block },
    ProvinceEffects { at: Place, block: Block },
    SpawnArmy { owner: String, at: Place, regiments: u32, unit: String, name: String },
    SpawnNavy { owner: String, at: Place, ships: u32, unit: String, name: String },
    War { attackers: Vec<String>, defenders: Vec<String>, goal: String },
    /// Ends every running war of the tag (`all`: every war), and gives back what they occupied.
    EndWars(Option<String>),
    ClearArmies(String),
    /// One province, or every province owned by `of`, controlled by `to`.
    Controller { at: Option<Place>, of: Option<String>, to: String },
    Owner { at: Place, to: String },
    Fort { at: Place, level: f64 },
    /// Sets the money of every matching pop (in money units, as the `money` effect counts them).
    PopMoney { tag: Option<String>, at: Option<Place>, pop_type: Option<String>, money: f64 },
    SetMoney { tag: String, money: f64 },
    Budget { tag: String, sliders: Vec<(Slider, f64)> },
    MoveArmy { army: String, to: Place },
    CasusBelli { actor: String, target: String, kind: String },
    Play(String),
}

/// When a check is evaluated.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum When {
    End,
    /// After this many days.
    Day(u32),
    /// On the first day at or after this date.
    Date(Date),
}

/// What a check expects of the value it measures.
#[derive(Debug, Clone, PartialEq)]
pub enum Want {
    Is(String),
    Not(String),
    /// Bounds on the value (inclusive).
    Range { min: Option<f64>, max: Option<f64> },
    /// Bounds on the value minus its value right after the setup.
    Change { min: Option<f64>, max: Option<f64> },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Args {
    pub tag: Option<String>,
    pub province: Option<Place>,
    pub pop_type: Option<String>,
    pub good: Option<String>,
    pub a: Option<String>,
    pub b: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub label: String,
    pub what: String,
    pub args: Args,
    pub want: Want,
    pub when: When,
    /// A known open bug this check is expected to expose: its FAIL is not a regression.
    pub known_bug: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scenario {
    pub name: String,
    pub description: String,
    pub what_to_look_for: String,
    pub base_save: Option<String>,
    pub seat: Option<String>,
    /// No country is played: every one is the computer's (`observe = yes`).
    pub observe: bool,
    pub days: u32,
    /// Starting speed of the browser game (1 to 5).
    pub speed: Option<u8>,
    /// Known open bugs the scenario as a whole is expected to expose.
    pub known_bugs: Vec<String>,
    pub setup: Vec<Step>,
    pub checks: Vec<Check>,
}

/// The quantities a check can measure, with what they need and mean (docs/SCENARIOS.md).
pub const QUANTITIES: &[(&str, &str)] = &[
    ("controller", "province: the tag controlling it"),
    ("owner", "province: the tag owning it"),
    ("war", "a, b: yes when a running war has them on opposite sides"),
    ("at_war", "tag: yes when it fights a running war"),
    ("wars", "tag (optional): running wars (of the tag)"),
    ("wars_started", "tag: wars it declared during the run (setup wars not counted)"),
    ("regiments", "tag, province (optional): its regiments (standing in the province)"),
    ("men", "tag, province (optional): men in its regiments (standing in the province)"),
    ("armies", "tag, province (optional): its armies (standing in the province)"),
    ("ships", "tag: its ships"),
    ("sieges", "province (optional): sieges going on (there)"),
    ("battles", "province (optional): land battles going on (there)"),
    ("rebels", "tag or province (optional): rebel regiments (in the tag's land, in the province)"),
    ("occupied", "tag: its provinces controlled by somebody else"),
    ("provinces", "tag: provinces it owns"),
    ("colonies", "tag: colonial provinces it owns"),
    ("money", "tag: treasury"),
    ("prestige", "tag"),
    ("war_exhaustion", "tag"),
    ("badboy", "tag: infamy"),
    ("factories", "tag: open factories (level 1 or more)"),
    ("factory_levels", "tag: sum of factory levels"),
    ("population", "tag or province, pop_type (optional): people"),
    ("pop_money", "tag or province, pop_type (optional): money of the pops, in money units"),
    ("militancy", "tag or province, pop_type (optional): average, weighted by size"),
    ("consciousness", "tag or province, pop_type (optional): average, weighted by size"),
    ("price", "good: today's price"),
    ("supply", "good: world supply on the last market day"),
    ("demand", "good: world paid demand on the last market day"),
    ("rgo_output", "tag or province, good (optional): farm and mine output a day"),
    ("battles_started", "run total"),
    ("battles_finished", "run total"),
    ("sieges_started", "run total"),
    ("controller_changes", "run total"),
    ("men_lost", "run total, men taken off soldier pops"),
    ("risings", "run total of rebel risings"),
    ("rebel_regiments_raised", "run total"),
    ("peaces", "run total of peace treaties"),
    ("wars_declared", "run total, every country"),
    ("buildings_done", "run total of finished province buildings"),
    ("wars_changed", "run total of wars declared plus peaces signed"),
    ("lowest_gp_money", "the lowest treasury among the great powers"),
    ("armies_moved", "armies of countries now at war that stand somewhere else than at the start"),
    ("nan_countries", "countries whose money, prestige or war exhaustion is not a number"),
];

const SETUP_COMMANDS: &[&str] = &["spawn_army", "spawn_navy", "war", "end_wars", "clear_armies", "controller", "owner", "fort", "pop_money", "set_money", "budget", "move_army", "casus_belli", "play"];

fn s(b: &Block, k: &str) -> Option<String> {
    b.get(k).and_then(Value::as_str).map(str::to_string)
}

fn tag_of(b: &Block, k: &str) -> Option<String> {
    s(b, k).map(|t| t.to_uppercase())
}

fn need<T>(v: Option<T>, what: &str, line: usize) -> Result<T, String> {
    v.ok_or_else(|| format!("line {line}: {what}"))
}

fn tags(b: &Block, k: &str) -> Vec<String> {
    b.get_all(k)
        .flat_map(|v| match v {
            Value::Scalar(t) => vec![t.to_uppercase()],
            Value::Block(inner) => inner.bare_scalars().iter().map(|t| t.to_uppercase()).collect(),
        })
        .collect()
}

fn place(b: &Block, k: &str, line: usize) -> Result<Option<Place>, String> {
    match b.get(k) {
        None => Ok(None),
        Some(v) => {
            let text = v.as_str().ok_or_else(|| format!("line {line}: {k} must be a province"))?;
            Place::parse(text).map(Some).map_err(|e| format!("line {line}: {e}"))
        }
    }
}

fn is_tag(key: &str) -> bool {
    key.len() == 3 && key.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) && key.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

fn parse_step(key: &str, v: &Value, line: usize) -> Result<Step, String> {
    let block = || v.as_block().ok_or_else(|| format!("line {line}: {key} needs a {{ }} block"));
    let word = || v.as_str().map(str::to_string).ok_or_else(|| format!("line {line}: {key} needs a value"));
    let num = |b: &Block, k: &str| b.get(k).and_then(Value::as_f64);
    Ok(match key {
        "spawn_army" | "spawn_navy" => {
            let b = block()?;
            let owner = need(tag_of(b, "owner"), &format!("{key} needs owner = TAG"), line)?;
            let at = need(place(b, "at", line)?, &format!("{key} needs at = <province>"), line)?;
            let count_key = if key == "spawn_army" { "regiments" } else { "ships" };
            let n = need(num(b, count_key), &format!("{key} needs {count_key} = N"), line)?;
            if !(1.0..=500.0).contains(&n) {
                return Err(format!("line {line}: {count_key} must be 1 to 500"));
            }
            let unit = need(s(b, "unit"), &format!("{key} needs unit = <unit type>"), line)?;
            let name = s(b, "name").unwrap_or_else(|| format!("{owner} {key} at line {line}"));
            if key == "spawn_army" {
                Step::SpawnArmy { owner, at, regiments: n as u32, unit, name }
            } else {
                Step::SpawnNavy { owner, at, ships: n as u32, unit, name }
            }
        }
        "war" => {
            let b = block()?;
            let (attackers, defenders) = (tags(b, "attacker"), tags(b, "defender"));
            if attackers.is_empty() || defenders.is_empty() {
                return Err(format!("line {line}: war needs attacker = TAG and defender = TAG"));
            }
            Step::War { attackers, defenders, goal: need(s(b, "goal"), "war needs goal = <casus belli>", line)? }
        }
        "end_wars" => {
            let t = word()?;
            Step::EndWars(if t == "all" { None } else { Some(t.to_uppercase()) })
        }
        "clear_armies" => Step::ClearArmies(word()?.to_uppercase()),
        "play" => Step::Play(word()?.to_uppercase()),
        "controller" => {
            let b = block()?;
            let (at, of) = (place(b, "province", line)?, tag_of(b, "of"));
            if at.is_some() == of.is_some() {
                return Err(format!("line {line}: controller needs province = <province> or of = TAG"));
            }
            Step::Controller { at, of, to: need(tag_of(b, "to"), "controller needs to = TAG", line)? }
        }
        "owner" => {
            let b = block()?;
            Step::Owner { at: need(place(b, "province", line)?, "owner needs province = <province>", line)?, to: need(tag_of(b, "to"), "owner needs to = TAG", line)? }
        }
        "fort" => {
            let b = block()?;
            Step::Fort { at: need(place(b, "province", line)?, "fort needs province = <province>", line)?, level: need(num(b, "level"), "fort needs level = N", line)? }
        }
        "pop_money" => {
            let b = block()?;
            let (tag, at) = (tag_of(b, "tag"), place(b, "province", line)?);
            if tag.is_none() && at.is_none() {
                return Err(format!("line {line}: pop_money needs tag = TAG or province = <province>"));
            }
            Step::PopMoney { tag, at, pop_type: s(b, "pop_type"), money: need(num(b, "money"), "pop_money needs money = X", line)? }
        }
        "set_money" => {
            let b = block()?;
            Step::SetMoney { tag: need(tag_of(b, "tag"), "set_money needs tag = TAG", line)?, money: need(num(b, "money"), "set_money needs money = X", line)? }
        }
        "budget" => {
            let b = block()?;
            let tag = need(tag_of(b, "tag"), "budget needs tag = TAG", line)?;
            let mut sliders = Vec::new();
            for (k, v, l) in b.entries().filter(|(k, _, _)| *k != "tag") {
                let slider = Slider::parse(k).ok_or_else(|| format!("line {l}: unknown slider {k} (use {})", Slider::ALL.map(|x| x.name()).join(", ")))?;
                sliders.push((slider, v.as_f64().ok_or_else(|| format!("line {l}: {k} needs a number"))?));
            }
            Step::Budget { tag, sliders }
        }
        "move_army" => {
            let b = block()?;
            Step::MoveArmy { army: need(s(b, "army"), "move_army needs army = <name of a spawned army>", line)?, to: need(place(b, "to", line)?, "move_army needs to = <province>", line)? }
        }
        "casus_belli" => {
            let b = block()?;
            Step::CasusBelli { actor: need(tag_of(b, "actor"), "casus_belli needs actor = TAG", line)?, target: need(tag_of(b, "target"), "casus_belli needs target = TAG", line)?, kind: need(s(b, "type"), "casus_belli needs type = <casus belli>", line)? }
        }
        _ if is_tag(key) => Step::CountryEffects { tag: key.to_string(), block: block()?.clone() },
        _ if key.starts_with("capital:") || key.parse::<u16>().is_ok() => Step::ProvinceEffects { at: Place::parse(key)?, block: block()?.clone() },
        _ => return Err(format!("line {line}: '{key}' is no setup command ({}), country tag or province", SETUP_COMMANDS.join(", "))),
    })
}

fn parse_check(b: &Block, line: usize) -> Result<Check, String> {
    let what = need(s(b, "what"), "check needs what = <quantity>", line)?;
    if !QUANTITIES.iter().any(|(q, _)| *q == what) {
        return Err(format!("line {line}: unknown quantity '{what}' (see docs/SCENARIOS.md)"));
    }
    let args = Args { tag: tag_of(b, "tag"), province: place(b, "province", line)?, pop_type: s(b, "pop_type"), good: s(b, "good"), a: tag_of(b, "a"), b: tag_of(b, "b") };
    let num = |k: &str| -> Result<Option<f64>, String> {
        match b.get(k) {
            None => Ok(None),
            Some(v) => v.as_f64().map(Some).ok_or_else(|| format!("line {line}: {k} needs a number")),
        }
    };
    let (min, max, cmin, cmax) = (num("min")?, num("max")?, num("change_min")?, num("change_max")?);
    let want = match (s(b, "is"), s(b, "not")) {
        (Some(v), None) if min.is_none() && max.is_none() && cmin.is_none() && cmax.is_none() => Want::Is(v),
        (None, Some(v)) if min.is_none() && max.is_none() && cmin.is_none() && cmax.is_none() => Want::Not(v),
        (None, None) if (min.is_some() || max.is_some()) && cmin.is_none() && cmax.is_none() => Want::Range { min, max },
        (None, None) if (cmin.is_some() || cmax.is_some()) && min.is_none() && max.is_none() => Want::Change { min: cmin, max: cmax },
        _ => return Err(format!("line {line}: a check takes one of: is, not, min/max, change_min/change_max")),
    };
    let when = match (b.get("day").and_then(Value::as_f64), s(b, "date")) {
        (Some(d), None) if d >= 0.0 => When::Day(d as u32),
        (None, Some(d)) => When::Date(Date::parse(&d).ok_or_else(|| format!("line {line}: date '{d}' is not Y.M.D"))?),
        (None, None) => When::End,
        _ => return Err(format!("line {line}: a check takes day = N or date = Y.M.D, not both")),
    };
    let label = s(b, "name").unwrap_or_else(|| what.clone());
    Ok(Check { label, what, args, want, when, known_bug: s(b, "known_bug") })
}

/// Reads a scenario file's text.
pub fn parse(src: &str) -> Result<Scenario, String> {
    let doc = wiktoria_data::script::parse(src).map_err(|e| e.to_string())?;
    let mut sc = Scenario { days: DEFAULT_DAYS, ..Scenario::default() };
    for item in &doc.root.items {
        let Some(key) = item.key.as_deref() else { return Err(format!("line {}: a value without a key", item.line)) };
        let text = || item.value.as_str().map(str::to_string).ok_or_else(|| format!("line {}: {key} needs a value", item.line));
        match key {
            "name" => sc.name = text()?,
            "description" => sc.description = text()?,
            "what_to_look_for" => sc.what_to_look_for = text()?,
            "base_save" => sc.base_save = Some(text()?),
            "seat" => sc.seat = Some(text()?.to_uppercase()),
            "observe" => sc.observe = item.value.as_bool().ok_or_else(|| format!("line {}: observe is yes or no", item.line))?,
            "known_bug" => sc.known_bugs.push(text()?),
            "days" => sc.days = item.value.as_f64().filter(|d| *d >= 1.0).ok_or_else(|| format!("line {}: days needs a number of 1 or more", item.line))? as u32,
            "speed" => sc.speed = Some(item.value.as_f64().filter(|d| (1.0..=5.0).contains(d)).ok_or_else(|| format!("line {}: speed is 1 to 5", item.line))? as u8),
            "setup" => {
                let b = item.value.as_block().ok_or_else(|| format!("line {}: setup needs a {{ }} block", item.line))?;
                for i in &b.items {
                    let k = i.key.as_deref().ok_or_else(|| format!("line {}: a value without a key in setup", i.line))?;
                    sc.setup.push(parse_step(k, &i.value, i.line)?);
                }
            }
            "check" => sc.checks.push(parse_check(item.value.as_block().ok_or_else(|| format!("line {}: check needs a {{ }} block", item.line))?, item.line)?),
            other => return Err(format!("line {}: unknown entry '{other}'", item.line)),
        }
    }
    if sc.name.is_empty() {
        return Err("the scenario needs a name".into());
    }
    if sc.observe && (sc.seat.is_some() || sc.setup.iter().any(|x| matches!(x, Step::Play(_)))) {
        return Err("observe = yes plays no country: leave out seat and play".into());
    }
    Ok(sc)
}

// ------------------------------------------------------------------------------------ setup

/// What the setup works on.
pub struct Ctx<'a> {
    pub state: &'a mut State,
    pub core: &'a mut Core,
    pub cal: &'a Calendar,
    pub effects: &'a EffectRules,
    /// For the budget sliders' party limits; without it a slider is only kept inside 0..1.
    pub politics: Option<&'a PoliticsRules>,
}

fn known_country(state: &State, tag: &str) -> Result<(), String> {
    if state.countries.contains_key(tag) { Ok(()) } else { Err(format!("no country {tag} in the save")) }
}

fn set_controller(ctx: &mut Ctx, id: u16, to: &str) {
    let date = ctx.state.date;
    if let Some(p) = ctx.state.provinces.get_mut(&id) {
        p.controller = Some(to.to_string());
    }
    let m = ctx.core.mil.provinces.entry(id).or_default();
    m.controller = Some(to.to_string());
    m.last_controller_change = Some(date);
}

fn refresh_wars(state: &mut State, core: &Core) {
    state.wars = core.mil.wars.iter().filter(|w| w.active).map(|w| (w.attackers.iter().cloned().collect(), w.defenders.iter().cloned().collect())).collect();
}

/// Takes armies out of the world, and every combat they were in.
fn drop_armies(core: &mut Core, ids: &BTreeSet<u64>) {
    core.mil.armies.retain(|a| !ids.contains(&a.id));
    for n in &mut core.mil.navies {
        n.carried.retain(|x| !ids.contains(x));
    }
    core.mil.combats.retain(|c| c.kind == CombatKind::Naval || !c.attacker.units.iter().chain(&c.defender.units).any(|u| ids.contains(u)));
    core.ai_targets.retain(|id, _| !ids.contains(id));
    core.ai_resting.retain(|id| !ids.contains(id));
    core.ai_moved.retain(|id| !ids.contains(id));
}

fn apply_step(ctx: &mut Ctx, step: &Step, spawned: &mut BTreeMap<String, u64>, out: &mut Vec<String>) -> Result<(), String> {
    let date = ctx.state.date;
    match step {
        Step::CountryEffects { tag, block } => {
            known_country(ctx.state, tag)?;
            let mut rep = Report::default();
            crate::script::run_effects(ctx.state, ctx.cal, ctx.effects, block, &Scope::Country(tag.clone()), &mut rep);
            for k in rep.unsupported.keys() {
                out.push(format!("warning: effect not understood, skipped: {k}"));
            }
            out.push(format!("{tag}: {} effects applied", rep.applied.values().sum::<usize>()));
        }
        Step::ProvinceEffects { at, block } => {
            let id = at.resolve(ctx.state)?;
            let mut rep = Report::default();
            crate::script::run_effects(ctx.state, ctx.cal, ctx.effects, block, &Scope::Province(id), &mut rep);
            for k in rep.unsupported.keys() {
                out.push(format!("warning: effect not understood, skipped: {k}"));
            }
            out.push(format!("province {id}: {} effects applied", rep.applied.values().sum::<usize>()));
        }
        Step::SpawnArmy { owner, at, regiments, unit, name } => {
            known_country(ctx.state, owner)?;
            let id = at.resolve(ctx.state)?;
            let day = &ctx.core.day;
            let def = match day.units.get(unit) {
                Some(d) if d.land => d.clone(),
                _ => return Err(format!("'{unit}' is no land unit type (have: {})", day.units.values().filter(|u| u.land).map(|u| u.name.as_str()).collect::<Vec<_>>().join(", "))),
            };
            if day.map.provinces.get(&id).is_some_and(|p| p.sea) {
                return Err(format!("province {id} is sea: an army is placed on land"));
            }
            let army_id = ctx.core.new_id();
            let regs: Vec<Regiment> = (0..*regiments)
                .map(|i| Regiment { id: ctx.core.new_id(), name: format!("{name} {}", i + 1), kind: unit.clone(), pop: None, pop_type: None, organisation: def.organisation, strength: if def.max_strength > 0.0 { def.max_strength } else { 3.0 }, experience: 0.0, name_count: i + 1 })
                .collect();
            ctx.core.mil.armies.push(Army { id: army_id, name: name.clone(), owner: owner.clone(), location: id, base: Some(id), supplies: 1.0, regiments: regs, ..Army::default() });
            ctx.core.mil.countries.entry(owner.clone()).or_default();
            spawned.insert(name.clone(), army_id);
            out.push(format!("army '{name}' of {owner}: {regiments} x {unit} in province {id}"));
        }
        Step::SpawnNavy { owner, at, ships, unit, name } => {
            known_country(ctx.state, owner)?;
            let id = match at {
                // A sea zone need not be in the save's provinces.
                Place::Id(id) if ctx.core.day.map.provinces.get(id).is_some_and(|p| p.sea) => *id,
                _ => at.resolve(ctx.state)?,
            };
            let day = &ctx.core.day;
            let def = match day.units.get(unit) {
                Some(d) if !d.land => d.clone(),
                _ => return Err(format!("'{unit}' is no ship type (have: {})", day.units.values().filter(|u| !u.land).map(|u| u.name.as_str()).collect::<Vec<_>>().join(", "))),
            };
            let navy_id = ctx.core.new_id();
            let list: Vec<Ship> = (0..*ships).map(|i| Ship { id: ctx.core.new_id(), name: format!("{name} {}", i + 1), kind: unit.clone(), organisation: def.organisation, strength: if def.max_strength > 0.0 { def.max_strength } else { 100.0 }, experience: 0.0, name_count: i + 1 }).collect();
            ctx.core.mil.navies.push(Navy { id: navy_id, name: name.clone(), owner: owner.clone(), location: id, supplies: 1.0, ships: list, ..Navy::default() });
            ctx.core.mil.countries.entry(owner.clone()).or_default();
            out.push(format!("fleet '{name}' of {owner}: {ships} x {unit} in province {id}"));
        }
        Step::War { attackers, defenders, goal } => {
            for t in attackers.iter().chain(defenders) {
                known_country(ctx.state, t)?;
                ctx.core.mil.countries.entry(t.clone()).or_default();
            }
            let (c, t) = (&attackers[0], &defenders[0]);
            if !ctx.core.day.cb_types.is_empty() && !ctx.core.day.cb_types.contains_key(goal) {
                out.push(format!("warning: no casus belli '{goal}' in the install (have: {}); the war's goal will not tick", ctx.core.day.cb_types.keys().cloned().collect::<Vec<_>>().join(", ")));
            }
            let demand_state = ctx.core.day.cb_types.get(goal).is_some_and(|cb| cb.has("demand_state"));
            let g = WarGoal { casus_belli: goal.clone(), actor: c.clone(), receiver: t.clone(), state_province: if demand_state { Core::goal_state_province(ctx.state, t) } else { None }, date: Some(date), ..WarGoal::default() };
            let base = format!("{c}-{t} scenario war");
            let mut name = base.clone();
            for n in 2.. {
                if !ctx.core.mil.wars.iter().any(|w| w.name == name) {
                    break;
                }
                name = format!("{base} ({n})");
            }
            let mut history: Vec<WarEvent> = attackers.iter().map(|a| WarEvent { date, kind: WarEventKind::AddAttacker(a.clone()) }).collect();
            history.extend(defenders.iter().map(|d| WarEvent { date, kind: WarEventKind::AddDefender(d.clone()) }));
            history.push(WarEvent { date, kind: WarEventKind::WarGoal(g.clone()) });
            ctx.core.mil.wars.insert(0, War { name: name.clone(), active: true, attackers: attackers.clone(), defenders: defenders.clone(), original_attacker: Some(c.clone()), original_defender: Some(t.clone()), original_goal: Some(g.clone()), goals: vec![g], action: Some(date), history });
            if let Some((_, dip)) = ctx.core.dip.as_mut() {
                dip.relation_mut(c, t).last_war = Some(date);
                dip.relation_mut(t, c).last_war = Some(date);
            }
            refresh_wars(ctx.state, ctx.core);
            out.push(format!("war '{name}': {} against {}, goal {goal}", attackers.join("+"), defenders.join("+")));
        }
        Step::EndWars(tag) => {
            let ends = |w: &War| w.active && tag.as_ref().is_none_or(|t| w.takes_part(t));
            let ended: Vec<War> = ctx.core.mil.wars.iter().filter(|w| ends(w)).cloned().collect();
            ctx.core.mil.wars.retain(|w| !ends(w));
            let mut back = 0;
            for w in &ended {
                let sides: BTreeSet<&String> = w.attackers.iter().chain(&w.defenders).collect();
                let ids: Vec<u16> = ctx.state.provinces.iter().filter(|(_, p)| p.owner != p.controller && p.owner.as_ref().is_some_and(|o| sides.contains(o)) && p.controller.as_ref().is_some_and(|c| sides.contains(c))).map(|(id, _)| *id).collect();
                for id in ids {
                    let owner = ctx.state.provinces[&id].owner.clone().unwrap_or_default();
                    set_controller(ctx, id, &owner);
                    back += 1;
                }
            }
            // Battles and sieges between countries now at peace end too.
            let at_war: BTreeSet<String> = ctx.core.mil.wars.iter().filter(|w| w.active).flat_map(|w| w.attackers.iter().chain(&w.defenders).cloned()).collect();
            let rebel = ctx.core.day.rebel_tag.clone();
            let owner_of: BTreeMap<u64, String> = ctx.core.mil.armies.iter().map(|a| (a.id, a.owner.clone())).chain(ctx.core.mil.navies.iter().map(|n| (n.id, n.owner.clone()))).collect();
            ctx.core.mil.combats.retain(|c| c.attacker.units.iter().chain(&c.defender.units).filter_map(|u| owner_of.get(u)).all(|o| *o == rebel || at_war.contains(o)));
            refresh_wars(ctx.state, ctx.core);
            out.push(format!("ended {} wars{}, {back} provinces back to their owners", ended.len(), tag.as_ref().map_or(String::new(), |t| format!(" of {t}"))));
        }
        Step::ClearArmies(tag) => {
            let ids: BTreeSet<u64> = ctx.core.mil.armies_of(tag).map(|a| a.id).collect();
            drop_armies(ctx.core, &ids);
            out.push(format!("removed {} armies of {tag}", ids.len()));
        }
        Step::Controller { at, of, to } => {
            known_country(ctx.state, to)?;
            let ids: Vec<u16> = match (at, of) {
                (Some(p), _) => vec![p.resolve(ctx.state)?],
                (None, Some(t)) => ctx.state.provinces.iter().filter(|(_, p)| p.owner.as_deref() == Some(t.as_str())).map(|(id, _)| *id).collect(),
                (None, None) => Vec::new(),
            };
            for id in &ids {
                set_controller(ctx, *id, to);
            }
            out.push(format!("{} provinces now controlled by {to}", ids.len()));
        }
        Step::Owner { at, to } => {
            known_country(ctx.state, to)?;
            let id = at.resolve(ctx.state)?;
            if let Some(p) = ctx.state.provinces.get_mut(&id) {
                p.owner = Some(to.clone());
            }
            ctx.core.mil.provinces.entry(id).or_default().owner = Some(to.clone());
            set_controller(ctx, id, to);
            out.push(format!("province {id} now owned by {to}"));
        }
        Step::Fort { at, level } => {
            let id = at.resolve(ctx.state)?;
            let max = ctx.state.building_kinds.get("fort").map(|(_, m)| *m).filter(|m| *m > 0.0).unwrap_or(f64::INFINITY);
            let level = level.clamp(0.0, max);
            ctx.core.mil.provinces.entry(id).or_default().fort = level;
            let name = ctx.state.building_kinds.get("fort").map_or("fort".to_string(), |(n, _)| n.clone());
            if let Some(p) = ctx.state.provinces.get_mut(&id) {
                p.buildings.insert(name, level);
            }
            out.push(format!("province {id}: fort level {level}"));
        }
        Step::PopMoney { tag, at, pop_type, money } => {
            let only = at.as_ref().map(|p| p.resolve(ctx.state)).transpose()?;
            let scale = ctx.effects.pop_money_scale;
            let mut n = 0;
            for (id, p) in ctx.state.provinces.iter_mut() {
                if only.is_some_and(|o| o != *id) || tag.as_ref().is_some_and(|t| p.owner.as_ref() != Some(t)) {
                    continue;
                }
                for q in p.pops.iter_mut().filter(|q| pop_type.as_ref().is_none_or(|t| *t == q.pop_type)) {
                    q.money = money.max(0.0) * scale;
                    n += 1;
                }
            }
            out.push(format!("{n} pops now hold {money} each"));
        }
        Step::SetMoney { tag, money } => {
            ctx.state.countries.get_mut(tag).ok_or_else(|| format!("no country {tag} in the save"))?.money = *money;
            out.push(format!("{tag}: treasury {money}"));
        }
        Step::Budget { tag, sliders } => {
            known_country(ctx.state, tag)?;
            for (slider, value) in sliders {
                let set = match ctx.politics {
                    Some(p) => crate::player::set_slider(ctx.state, p, tag, *slider, *value)?,
                    None => {
                        let (lo, hi) = if *slider == Slider::Tariff { (-1.0, 1.0) } else { (0.0, 1.0) };
                        let v = value.clamp(lo, hi);
                        ctx.state.countries.get_mut(tag).expect("checked").spending.insert(slider.key().to_string(), v);
                        v
                    }
                };
                out.push(format!("{tag}: {} slider {set}", slider.name()));
            }
        }
        Step::MoveArmy { army, to } => {
            let id = *spawned.get(army).ok_or_else(|| format!("move_army: no army '{army}' placed by this setup"))?;
            let to = to.resolve(ctx.state)?;
            let owner = ctx.core.mil.armies.iter().find(|a| a.id == id).map(|a| a.owner.clone()).ok_or("move_army: the army is gone")?;
            let rel = ctx.core.day.relations(&ctx.core.mil, ctx.state);
            let provinces = ctx.core.mil.provinces.clone();
            let none = crate::military::siege::ProvinceMilitary::default();
            let may_enter = |a: u16, b: u16| crate::military::movement::may_enter_land(&rel, &owner, false, provinces.get(&a).unwrap_or(&none), provinces.get(&b).unwrap_or(&none));
            let (mil, day) = (&mut ctx.core.mil, &ctx.core.day);
            let (n, sea) = crate::player::move_army(mil, &day.map, &day.movement, &|f| day.fleet_capacity(f), &may_enter, &owner, id, to)?;
            out.push(format!("army '{army}' ordered to province {to}: {n} provinces{}", if sea { ", by sea" } else { "" }));
        }
        Step::CasusBelli { actor, target, kind } => {
            known_country(ctx.state, actor)?;
            known_country(ctx.state, target)?;
            if !ctx.core.day.cb_types.is_empty() && !ctx.core.day.cb_types.contains_key(kind) {
                return Err(format!("no casus belli '{kind}' in the install (have: {})", ctx.core.day.cb_types.keys().cloned().collect::<Vec<_>>().join(", ")));
            }
            let held = war::cb_gain(&ctx.core.day.war, kind, date);
            out.push(format!("{actor} holds a {kind} casus belli on {target} until {}", held.end));
            ctx.core.held_cbs.entry((actor.clone(), target.clone())).or_default().push(held);
        }
        Step::Play(tag) => {
            crate::player::set_player(ctx.state, ctx.core, tag, true)?;
            out.push(format!("{tag} is played"));
        }
    }
    Ok(())
}

/// Applies the setup (then plays the seat). Returns a line for each step, warnings first marked.
pub fn apply_setup(sc: &Scenario, ctx: &mut Ctx) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut spawned = BTreeMap::new();
    // Owners and controllers before the setup: effects change only the script state, so what
    // they move is copied to the military's provinces afterwards.
    let before: BTreeMap<u16, (Option<String>, Option<String>)> = ctx.state.provinces.iter().map(|(id, p)| (*id, (p.owner.clone(), p.controller.clone()))).collect();
    for (i, step) in sc.setup.iter().enumerate() {
        apply_step(ctx, step, &mut spawned, &mut out).map_err(|e| format!("setup step {}: {e}", i + 1))?;
    }
    for (id, p) in ctx.state.provinces.iter() {
        if before.get(id).is_some_and(|b| b.0 != p.owner || b.1 != p.controller) {
            let m = ctx.core.mil.provinces.entry(*id).or_default();
            m.owner = p.owner.clone();
            m.controller = p.controller.clone();
        }
    }
    if let Some(seat) = &sc.seat {
        crate::player::set_player(ctx.state, ctx.core, seat, true).map_err(|e| format!("seat: {e}"))?;
        out.push(format!("seat {seat} is played"));
    }
    Ok(out)
}

// ------------------------------------------------------------------------------------ checks

/// Totals of the day logs over the run.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tally {
    pub battles_started: usize,
    pub battles_finished: usize,
    pub sieges_started: usize,
    pub controller_changes: usize,
    pub men_lost: f64,
    pub risings: usize,
    pub rebel_regiments_raised: usize,
    pub peaces: usize,
    pub wars_declared: usize,
    pub buildings_done: usize,
}

impl Tally {
    pub fn add(&mut self, log: &CoreLog) {
        self.battles_started += log.battles_started;
        self.battles_finished += log.battles_finished;
        self.sieges_started += log.sieges_started;
        self.controller_changes += log.controller_changes;
        self.men_lost += log.men_lost;
        self.risings += log.risings;
        self.rebel_regiments_raised += log.rebel_regiments;
        self.peaces += log.peaces.len();
        self.wars_declared += log.wars_declared.len();
        self.buildings_done += log.buildings_done;
    }
}

/// What the checks read besides the state and the military: the economy's numbers of the last
/// market day (empty where the caller has none).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Probe {
    /// Stored pop money per money unit (FLD_POP_MONEY_EFFECT_SCALE); 0 counts as 1.
    pub pop_money_scale: f64,
    pub supply: BTreeMap<String, f64>,
    pub demand: BTreeMap<String, f64>,
    /// (province, owner, good, units a day).
    pub rgo: Vec<(u16, String, String, f64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Measured {
    Text(String),
    Num(f64),
}

impl std::fmt::Display for Measured {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Measured::Text(t) => write!(f, "{t}"),
            Measured::Num(x) => write!(f, "{}", num_text(*x)),
        }
    }
}

fn num_text(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 { format!("{x:.0}") } else if x.abs() >= 100.0 { format!("{x:.1}") } else { format!("{x:.3}") }
}

/// Everything a measure reads.
pub struct Seen<'a> {
    pub state: &'a State,
    pub core: &'a Core,
    pub probe: &'a Probe,
    pub tally: &'a Tally,
    /// The day the run began and the wars the setup started.
    pub start: Date,
    pub setup_wars: &'a BTreeSet<String>,
    /// Where each army stood at the start.
    pub start_locations: &'a BTreeMap<u64, u16>,
}

fn yes(b: bool) -> Measured {
    Measured::Text(if b { "yes" } else { "no" }.into())
}

/// The value of a check's quantity now.
pub fn measure(what: &str, a: &Args, seen: &Seen) -> Result<Measured, String> {
    let (state, mil) = (seen.state, &seen.core.mil);
    let tag = || a.tag.clone().ok_or_else(|| format!("{what} needs tag = TAG"));
    let province = || a.province.as_ref().map(|p| p.resolve(state)).transpose();
    let need_province = || province()?.ok_or_else(|| format!("{what} needs province = <province>"));
    let good = || a.good.clone().ok_or_else(|| format!("{what} needs good = <good>"));
    let country = |t: &str| state.countries.get(t).ok_or_else(|| format!("no country {t}"));
    // Pops of the tag's provinces or of the province, of the pop type.
    let pops = || -> Result<Vec<&crate::script::PopState>, String> {
        let only = province()?;
        if only.is_none() && a.tag.is_none() {
            return Err(format!("{what} needs tag = TAG or province = <province>"));
        }
        Ok(state
            .provinces
            .iter()
            .filter(|(id, p)| only.is_none_or(|o| o == **id) && a.tag.as_ref().is_none_or(|t| p.owner.as_ref() == Some(t)))
            .flat_map(|(_, p)| &p.pops)
            .filter(|q| a.pop_type.as_ref().is_none_or(|t| *t == q.pop_type))
            .collect())
    };
    let average = |f: fn(&crate::script::PopState) -> f64| -> Result<Measured, String> {
        let list = pops()?;
        let size: f64 = list.iter().map(|q| q.size).sum();
        Ok(Measured::Num(if size > 0.0 { list.iter().map(|q| q.size * f(q)).sum::<f64>() / size } else { 0.0 }))
    };
    let standing = |t: String, only: Option<u16>| -> Vec<&Army> { mil.armies.iter().filter(|x| x.owner == t && only.is_none_or(|o| x.location == o && x.carried_by.is_none())).collect() };
    let men_per = if seen.core.day.men_per_strength > 0.0 { seen.core.day.men_per_strength } else { 1000.0 };
    let n = |x: usize| Ok(Measured::Num(x as f64));
    let t = seen.tally;
    match what {
        "controller" | "owner" => {
            let p = state.provinces.get(&need_province()?).expect("resolved");
            Ok(Measured::Text(if what == "owner" { p.owner.clone() } else { p.controller.clone() }.unwrap_or_else(|| "-".into())))
        }
        "war" => {
            let (x, y) = (a.a.clone().ok_or("war needs a = TAG")?, a.b.clone().ok_or("war needs b = TAG")?);
            Ok(yes(mil.wars.iter().any(|w| w.active && ((w.attackers.contains(&x) && w.defenders.contains(&y)) || (w.attackers.contains(&y) && w.defenders.contains(&x))))))
        }
        "at_war" => Ok(yes(mil.active_wars_of(&tag()?).next().is_some())),
        "wars" => n(match &a.tag {
            Some(tg) => mil.active_wars_of(tg).count(),
            None => mil.wars.iter().filter(|w| w.active).count(),
        }),
        "wars_started" => {
            let tg = tag()?;
            n(mil.wars.iter().filter(|w| w.original_attacker.as_ref() == Some(&tg) && !seen.setup_wars.contains(&w.name) && w.started().is_some_and(|d| d >= seen.start)).count())
        }
        "regiments" => n(standing(tag()?, province()?).iter().map(|x| x.regiments.len()).sum()),
        "men" => Ok(Measured::Num(standing(tag()?, province()?).iter().flat_map(|x| &x.regiments).map(|r| r.strength * men_per).sum())),
        "armies" => n(standing(tag()?, province()?).len()),
        "ships" => n(mil.ship_count(&tag()?)),
        "sieges" | "battles" => {
            let only = province()?;
            n(mil.combats.iter().filter(|c| (c.kind == CombatKind::Siege) == (what == "sieges") && c.kind != CombatKind::Naval && only.is_none_or(|o| c.location == o)).count())
        }
        "rebels" => {
            let only = province()?;
            let rebel = &seen.core.day.rebel_tag;
            n(mil.armies_of(rebel).filter(|x| only.is_none_or(|o| x.location == o) && a.tag.as_ref().is_none_or(|tg| state.provinces.get(&x.location).and_then(|p| p.owner.as_ref()) == Some(tg))).map(|x| x.regiments.len()).sum())
        }
        "occupied" => {
            let tg = tag()?;
            n(state.provinces.values().filter(|p| p.owner.as_ref() == Some(&tg) && p.controller != p.owner).count())
        }
        "provinces" | "colonies" => {
            let tg = tag()?;
            n(state.provinces.values().filter(|p| p.owner.as_ref() == Some(&tg) && (what == "provinces" || p.colonial > 0)).count())
        }
        "money" => Ok(Measured::Num(country(&tag()?)?.money)),
        "prestige" => Ok(Measured::Num(country(&tag()?)?.prestige)),
        "war_exhaustion" => Ok(Measured::Num(country(&tag()?)?.war_exhaustion)),
        "badboy" => Ok(Measured::Num(country(&tag()?)?.badboy)),
        "factories" => {
            let tg = tag()?;
            n(state.factories.iter().filter(|f| f.owner == tg && f.level >= 1.0).count())
        }
        "factory_levels" => {
            let tg = tag()?;
            Ok(Measured::Num(state.factories.iter().filter(|f| f.owner == tg).map(|f| f.level).sum()))
        }
        "population" => Ok(Measured::Num(pops()?.iter().map(|q| q.size).sum())),
        "pop_money" => {
            let scale = if seen.probe.pop_money_scale > 0.0 { seen.probe.pop_money_scale } else { 1.0 };
            Ok(Measured::Num(pops()?.iter().map(|q| q.money).sum::<f64>() / scale))
        }
        "militancy" => average(|q| q.militancy),
        "consciousness" => average(|q| q.consciousness),
        "price" => {
            let g = good()?;
            state.prices.get(&g).map(|x| Measured::Num(*x)).ok_or_else(|| format!("no price for good {g}"))
        }
        "supply" => Ok(Measured::Num(seen.probe.supply.get(&good()?).copied().unwrap_or(0.0))),
        "demand" => Ok(Measured::Num(seen.probe.demand.get(&good()?).copied().unwrap_or(0.0))),
        "rgo_output" => {
            let only = province()?;
            Ok(Measured::Num(seen.probe.rgo.iter().filter(|(id, owner, g, _)| only.is_none_or(|o| o == *id) && a.tag.as_ref().is_none_or(|tg| tg == owner) && a.good.as_ref().is_none_or(|x| x == g)).map(|r| r.3).sum()))
        }
        "battles_started" => n(t.battles_started),
        "battles_finished" => n(t.battles_finished),
        "sieges_started" => n(t.sieges_started),
        "controller_changes" => n(t.controller_changes),
        "men_lost" => Ok(Measured::Num(t.men_lost)),
        "risings" => n(t.risings),
        "rebel_regiments_raised" => n(t.rebel_regiments_raised),
        "peaces" => n(t.peaces),
        "wars_declared" => n(t.wars_declared),
        "buildings_done" => n(t.buildings_done),
        "wars_changed" => n(t.wars_declared + t.peaces),
        "lowest_gp_money" => state.great_powers.iter().filter_map(|g| state.countries.get(g)).map(|c| c.money).min_by(f64::total_cmp).map(Measured::Num).ok_or_else(|| "no great powers".to_string()),
        "armies_moved" => {
            let at_war: BTreeSet<&String> = mil.wars.iter().filter(|w| w.active).flat_map(|w| w.attackers.iter().chain(&w.defenders)).collect();
            n(mil.armies.iter().filter(|x| at_war.contains(&x.owner) && seen.start_locations.get(&x.id).is_some_and(|l| *l != x.location)).count())
        }
        "nan_countries" => n(state.countries.values().filter(|c| !c.money.is_finite() || !c.prestige.is_finite() || !c.war_exhaustion.is_finite()).count()),
        other => Err(format!("unknown quantity {other}")),
    }
}

/// A check's result.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub label: String,
    pub pass: bool,
    pub actual: String,
    pub want: String,
    pub date: Date,
    pub known_bug: Option<String>,
}

impl Verdict {
    /// `PASS`, `FAIL`, or `KNOWN` (a fail the check expects from a known open bug).
    pub fn word(&self) -> &'static str {
        match (self.pass, self.known_bug.is_some()) {
            (true, _) => "PASS",
            (false, false) => "FAIL",
            (false, true) => "KNOWN",
        }
    }

    pub fn line(&self) -> String {
        format!("{:<5} {} [{}]: {} (want {}){}", self.word(), self.label, self.date, self.actual, self.want, self.known_bug.as_ref().filter(|_| !self.pass).map_or(String::new(), |b| format!("; known bug: {b}")))
    }
}

fn want_text(w: &Want) -> String {
    let range = |lo: &Option<f64>, hi: &Option<f64>| match (lo, hi) {
        (Some(a), Some(b)) => format!("{} to {}", num_text(*a), num_text(*b)),
        (Some(a), None) => format!("at least {}", num_text(*a)),
        (None, Some(b)) => format!("at most {}", num_text(*b)),
        (None, None) => "anything".into(),
    };
    match w {
        Want::Is(v) => v.clone(),
        Want::Not(v) => format!("not {v}"),
        Want::Range { min, max } => range(min, max),
        Want::Change { min, max } => format!("a change of {}", range(min, max)),
    }
}

fn judge(check: &Check, now: Result<Measured, String>, base: Option<&Measured>, state: &State) -> (bool, String) {
    let now = match now {
        Ok(m) => m,
        Err(e) => return (false, format!("cannot measure: {e}")),
    };
    let inside = |x: f64, lo: &Option<f64>, hi: &Option<f64>| lo.is_none_or(|l| x >= l) && hi.is_none_or(|h| x <= h);
    // `is` and `not` against a province value take `capital:TAG` too.
    let same = |v: &str| match (&now, v.parse::<f64>()) {
        (Measured::Num(x), Ok(y)) => (x - y).abs() < 1e-9,
        (Measured::Num(x), Err(_)) => Place::parse(v).ok().and_then(|p| p.resolve(state).ok()).is_some_and(|id| (*x - id as f64).abs() < 1e-9),
        (Measured::Text(t), _) => t.eq_ignore_ascii_case(v),
    };
    match (&check.want, &now) {
        (Want::Is(v), _) => (same(v), now.to_string()),
        (Want::Not(v), _) => (!same(v), now.to_string()),
        (Want::Range { min, max }, Measured::Num(x)) => (inside(*x, min, max), now.to_string()),
        (Want::Change { min, max }, Measured::Num(x)) => match base {
            Some(Measured::Num(b)) => (inside(x - b, min, max), format!("{} (from {}, change {})", num_text(*x), num_text(*b), num_text(x - b))),
            _ => (false, format!("{now} (no starting value)")),
        },
        (_, Measured::Text(t)) => (false, format!("{t} is not a number")),
    }
}

/// A run of a scenario's checks over the days: call [`Run::day`] after each day and
/// [`Run::finish`] at the end.
pub struct Run {
    pub start: Date,
    pub days: u32,
    pub tally: Tally,
    pub verdicts: Vec<Verdict>,
    pub setup_wars: BTreeSet<String>,
    pub start_locations: BTreeMap<u64, u16>,
    base: Vec<Option<Measured>>,
    done: Vec<bool>,
}

impl Run {
    /// Right after the setup: the starting values of the change checks. `setup_wars` are the
    /// wars running now, which `wars_started` does not count.
    pub fn begin(sc: &Scenario, state: &State, core: &Core, probe: &Probe) -> Run {
        let setup_wars: BTreeSet<String> = core.mil.wars.iter().filter(|w| w.active).map(|w| w.name.clone()).collect();
        let tally = Tally::default();
        let start_locations: BTreeMap<u64, u16> = core.mil.armies.iter().map(|a| (a.id, a.location)).collect();
        let seen = Seen { state, core, probe, tally: &tally, start: state.date, setup_wars: &setup_wars, start_locations: &start_locations };
        let base = sc.checks.iter().map(|c| matches!(c.want, Want::Change { .. }).then(|| measure(&c.what, &c.args, &seen).ok()).flatten()).collect();
        Run { start: state.date, days: 0, tally, verdicts: Vec::new(), setup_wars, start_locations, base, done: vec![false; sc.checks.len()] }
    }

    fn evaluate(&mut self, sc: &Scenario, i: usize, state: &State, core: &Core, probe: &Probe) {
        let c = &sc.checks[i];
        let seen = Seen { state, core, probe, tally: &self.tally, start: self.start, setup_wars: &self.setup_wars, start_locations: &self.start_locations };
        let (pass, actual) = judge(c, measure(&c.what, &c.args, &seen), self.base[i].as_ref(), state);
        self.verdicts.push(Verdict { label: c.label.clone(), pass, actual, want: want_text(&c.want), date: state.date, known_bug: c.known_bug.clone() });
        self.done[i] = true;
    }

    /// Whether a dated check falls due on the day about to be recorded with [`Run::day`] (the
    /// caller then fills a probe for it).
    pub fn due(&self, sc: &Scenario, today: Date) -> bool {
        sc.checks.iter().zip(&self.done).any(|(c, d)| !d && Self::is_due(c, self.days + 1, today))
    }

    fn is_due(c: &Check, days: u32, today: Date) -> bool {
        match c.when {
            When::End => false,
            When::Day(n) => days >= n,
            When::Date(d) => today >= d,
        }
    }

    /// After one day: adds its log and evaluates the checks due.
    pub fn day(&mut self, sc: &Scenario, log: &CoreLog, state: &State, core: &Core, probe: &Probe) {
        self.days += 1;
        self.tally.add(log);
        for i in 0..sc.checks.len() {
            if !self.done[i] && Self::is_due(&sc.checks[i], self.days, state.date) {
                self.evaluate(sc, i, state, core, probe);
            }
        }
    }

    /// Evaluates every check not evaluated yet (those for the end, and dated ones the run did not reach).
    pub fn finish(&mut self, sc: &Scenario, state: &State, core: &Core, probe: &Probe) {
        for i in 0..sc.checks.len() {
            if !self.done[i] {
                self.evaluate(sc, i, state, core, probe);
            }
        }
    }

    pub fn failed(&self, strict: bool) -> usize {
        self.verdicts.iter().filter(|v| !v.pass && (strict || v.known_bug.is_none())).count()
    }
}

// ------------------------------------------------------------------------------------ the whole game

/// The setup on a combined game.
pub fn prepare(game: &mut Game, sc: &Scenario) -> Result<Vec<String>, String> {
    let mut ctx = Ctx { state: &mut game.state, core: &mut game.core, cal: &game.econ.cal, effects: &game.econ.effect_rules, politics: Some(&game.econ.politics_rules) };
    apply_setup(sc, &mut ctx)
}

/// The probe of a combined game: the economy's numbers of `last` (the last market day) and,
/// when a check asks for it, the farm and mine output.
pub fn game_probe(game: &Game, last: Option<&DayLog>, sc: &Scenario) -> Probe {
    let rgo = if sc.checks.iter().any(|c| c.what == "rgo_output") { game.econ.farms.province_output(&game.world, &game.state) } else { Vec::new() };
    Probe { pop_money_scale: game.econ.effect_rules.pop_money_scale, supply: last.map(|l| l.money.supply.clone()).unwrap_or_default(), demand: last.map(|l| l.money.demand.clone()).unwrap_or_default(), rgo }
}

/// Runs a prepared game `days` days with the scenario's checks. `each_day` sees the game after
/// every day (for progress lines).
pub fn run(game: &mut Game, sc: &Scenario, days: u32, mut each_day: impl FnMut(&Game, &DayLog)) -> Run {
    let mut r = Run::begin(sc, &game.state, &game.core, &game_probe(game, None, sc));
    let mut last = None;
    for _ in 0..days {
        let log = game.day();
        if r.due(sc, game.state.date) {
            r.day(sc, &log.core, &game.state, &game.core, &game_probe(game, Some(&log), sc));
        } else {
            r.day(sc, &log.core, &game.state, &game.core, &Probe::default());
        }
        each_day(game, &log);
        last = Some(log);
    }
    let probe = game_probe(game, last.as_ref(), sc);
    r.finish(sc, &game.state, &game.core, &probe);
    r
}
