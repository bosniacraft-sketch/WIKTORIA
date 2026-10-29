//! Events and decisions happening over time (hypothesis).
//!
//! The firing rule follows the description of the original's routine and was checked on its
//! saves with the gold events (province event 1100, `months = 12`, no modifiers: its 21 owned
//! provinces waited 398 days on average, 14 fired in the first year; the rule below gives 360
//! days and 13.4, the port's earlier rule 1 - 2^(-days / T) gave 520 days and 10.5):
//! * The original scans countries and provinces on alternate days. An event without
//!   `is_triggered_only` whose trigger holds for a country (country event) or an owned province
//!   (province event) fires at a scan with chance 1 / T for a country event and 2 / T for a
//!   province event, T in whole days; certain when T is one day or less. So a country event
//!   waits about twice the written time and a province event about the written time.
//! * T comes from `mean_time_to_happen`: ONE number that `days`, `months` (x FLD_MTTH_MONTH_DAYS),
//!   `years` (x FLD_MTTH_YEAR_DAYS), `factor` and `base` all write, the last one written wins;
//!   times the factor of every modifier whose conditions hold. No block at all: one day.
//! * The port tests only every FLD_EVENT_CHECK_DAYS days (for speed) and then rolls once for
//!   all the scans the original would have made since the last test: chance 1 - (1 - p)^scans.
//!   An event that is certain so fires at the next test, not the next day.
//! * `fire_only_once` events never fire again once fired (for anybody in the world).
//! * The option is chosen with chance proportional to `ai_chance` (its factor times the factors
//!   of its modifiers that hold; 1 when the option has none).
//! * **A country a person plays** (player.rs, D20) chooses itself. An event with two or more
//!   options fired for it (a country event of that country, a province, state or pop event of a
//!   province it owns) runs its `immediate` blocks at once and is then held in
//!   [`EventRunner::held`] until the player picks an option ([`EventRunner::choose`]). The
//!   computer's pick is rolled when the event fires, as for any country, so the dice of the rest of
//!   the world do not depend on when the player answers. PORT CHOICE: a held event that is not
//!   answered within [`EventRunner::answer_days`] days, or whose country goes back to the computer,
//!   takes that pick (the original's own time-out is not observed). Its option runs on the state of
//!   the day it is answered. An event with one option has nothing to choose and runs at once.
//!   Decisions: the computer takes none for a played country; [`EventRunner::open_decisions`] lists
//!   those it may take and [`EventRunner::take_decision`] takes one.
//! * An event an effect asked for (`country_event = { id days }`) fires on its day whatever its
//!   trigger says, with FROM = the scope that asked.
//! * A decision is taken by a country as soon as `potential` and `allow` hold and its
//!   `ai_will_do` weight is above zero.
//! * A timed modifier is removed on its expiry date.
//!
//! A trigger, modifier or decision with a condition the engine does not know never fires and is
//! counted in `EventLog::unknown`. All dice are the port's own (FLD_SCRIPT_SEED).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use wiktoria_data::date::Date;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::script::{Block, Value};

use crate::calendar::Calendar;
use crate::cond::{self, Conditions, Facts, Weight};
use crate::script::{conditions_hold, run_effects, EffectRules, Report, Scope, State};

#[derive(Debug, Clone)]
pub struct EventDef {
    pub id: u64,
    pub title: String,
    pub desc: String,
    /// A province event (else a country event).
    pub province: bool,
    pub trigger: Option<Block>,
    pub mean_time: Option<Block>,
    pub immediate: Vec<Block>,
    pub options: Vec<Block>,
    pub triggered_only: bool,
    pub fire_only_once: bool,
}

#[derive(Debug, Clone)]
pub struct DecisionDef {
    pub name: String,
    pub potential: Option<Block>,
    pub allow: Option<Block>,
    pub effect: Option<Block>,
    pub ai_will_do: Option<Block>,
}

/// The events of one events file.
pub fn parse_events(root: &Block) -> Vec<EventDef> {
    let mut out = Vec::new();
    for (key, value, _) in root.entries() {
        let (Some(e), true) = (value.as_block(), key == "country_event" || key == "province_event") else { continue };
        let Some(id) = e.get("id").and_then(Value::as_f64) else { continue };
        let block = |k: &str| e.get(k).and_then(Value::as_block).cloned();
        let yes = |k: &str| e.get(k).and_then(Value::as_str) == Some("yes");
        out.push(EventDef {
            id: id as u64,
            title: e.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
            desc: e.get("desc").and_then(Value::as_str).unwrap_or("").to_string(),
            province: key == "province_event",
            trigger: block("trigger"),
            mean_time: block("mean_time_to_happen"),
            immediate: e.get_all("immediate").filter_map(Value::as_block).cloned().collect(),
            options: e.get_all("option").filter_map(Value::as_block).cloned().collect(),
            triggered_only: yes("is_triggered_only"),
            fire_only_once: yes("fire_only_once"),
        });
    }
    out
}

/// The decisions of one decisions file (`political_decisions = { name = { ... } }`).
pub fn parse_decisions(root: &Block) -> Vec<DecisionDef> {
    let mut out = Vec::new();
    for (_, top, _) in root.entries() {
        let Some(top) = top.as_block() else { continue };
        for (name, d, _) in top.entries() {
            let Some(d) = d.as_block() else { continue };
            let block = |k: &str| d.get(k).and_then(Value::as_block).cloned();
            out.push(DecisionDef { name: name.to_string(), potential: block("potential"), allow: block("allow"), effect: block("effect"), ai_will_do: block("ai_will_do") });
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventRules {
    pub check_days: i64,
    pub month_days: f64,
    pub year_days: f64,
    /// FLD_EVENT_SCAN_EVERY_DAYS: countries are scanned for events on one day, provinces on the next.
    pub scan_every_days: i64,
    /// FLD_EVENT_PROVINCE_CHANCE_FACTOR: a province event's chance per scan is this many times 1 / mean time.
    pub province_chance_factor: f64,
    /// FLD_EVENT_MIN_CHANCE: the smallest chance per scan.
    pub min_chance: f64,
}

impl EventRules {
    pub fn from_engine_rules(m: &EngineRules) -> Result<EventRules, String> {
        Ok(EventRules {
            check_days: crate::int_field(m, "FLD_EVENT_CHECK_DAYS")?.max(1),
            month_days: crate::num_field(m, "FLD_MTTH_MONTH_DAYS")?,
            year_days: crate::num_field(m, "FLD_MTTH_YEAR_DAYS")?,
            scan_every_days: crate::int_field(m, "FLD_EVENT_SCAN_EVERY_DAYS")?.max(1),
            province_chance_factor: crate::num_field(m, "FLD_EVENT_PROVINCE_CHANCE_FACTOR")?,
            min_chance: crate::num_field(m, "FLD_EVENT_MIN_CHANCE")?,
        })
    }
}

/// The chance that an event with a waiting time of `mean_days` fires at one scan: 1 / whole
/// days for a country event, twice that for a province event, never below one in a million;
/// certain at one day or less. A negative time (a negative factor) leaves the smallest chance.
pub fn scan_chance(rules: &EventRules, mean_days: f64, province: bool) -> f64 {
    if mean_days < 0.0 {
        return rules.min_chance;
    }
    if mean_days <= 1.0 {
        return 1.0;
    }
    (if province { rules.province_chance_factor } else { 1.0 } / mean_days.trunc()).clamp(rules.min_chance, 1.0)
}

/// What happened during a run.
#[derive(Debug, Clone, Default)]
pub struct EventLog {
    /// Times each event fired, by id.
    pub fired: BTreeMap<u64, usize>,
    /// Times each decision was taken, by name.
    pub decided: BTreeMap<String, usize>,
    /// Conditions the engine did not know, with the number of events or decisions they blocked.
    pub unknown: BTreeMap<String, usize>,
    pub effects: Report,
    pub modifiers_expired: usize,
}

/// `factor` of a weighted block times the factors of its `modifier`s whose conditions hold.
/// Err names an unknown condition.
fn weight(state: &State, block: &Block, scope: &Scope) -> Result<f64, String> {
    let mut w = block.get("factor").and_then(Value::as_f64).unwrap_or(1.0);
    for m in block.get_all("modifier").filter_map(Value::as_block) {
        if conditions_hold(state, m, scope)? {
            w *= m.get("factor").and_then(Value::as_f64).unwrap_or(1.0);
        }
    }
    Ok(w)
}

/// An event fired for a country a person plays, waiting for the player's choice.
#[derive(Debug, Clone, PartialEq)]
pub struct HeldEvent {
    /// Our own number for it, unique in the run (what the player's order names).
    pub key: u64,
    /// The event's id.
    pub id: u64,
    /// The played country that chooses.
    pub tag: String,
    pub target: Scope,
    pub from: Option<Scope>,
    /// The option the computer would take (rolled when the event fired).
    pub ai_pick: usize,
    pub fired: Date,
    /// On this day (or later) the computer's pick is taken.
    pub expires: Date,
}

/// The played country an event for this scope asks, if any: the country itself, or the owner of
/// the province (state, pop) when a person plays it.
pub fn player_of(state: &State, scope: &Scope) -> Option<String> {
    let tag = match scope {
        Scope::Country(t) | Scope::State(_, t) => Some(t.clone()),
        Scope::Province(p) | Scope::Pop(p, _) => state.provinces.get(p).and_then(|p| p.owner.clone()),
    }?;
    state.countries.get(&tag).filter(|c| c.human).map(|_| tag)
}

/// The option names (localisation keys) of an event, in order; an option without one is `""`.
pub fn option_names(e: &EventDef) -> Vec<String> {
    e.options.iter().map(|o| o.get("name").and_then(Value::as_str).unwrap_or("").to_string()).collect()
}

/// PORT CHOICE (no rules file row yet; wanted: FLD_PLAYER_ANSWER_DAYS): days a held event or a
/// question to a player waits before the computer answers for it.
pub const ANSWER_DAYS: i64 = 30;

pub struct EventRunner {
    pub events: Vec<EventDef>,
    pub decisions: Vec<DecisionDef>,
    /// Compiled `trigger` and `mean_time_to_happen` modifiers of each event, by index.
    compiled_events: Vec<(Option<Conditions>, Vec<Conditions>)>,
    /// Compiled potential, allow and ai_will_do of each decision, by index.
    compiled_decisions: Vec<CompiledDecision>,
    /// Per event and per decision: its conditions cannot read `this`, so its tests of different
    /// targets do not depend on each other and may be made ahead on threads.
    free_events: Vec<bool>,
    free_decisions: Vec<bool>,
    by_id: BTreeMap<u64, usize>,
    fired_once: BTreeSet<u64>,
    /// Events and decisions already counted in `EventLog::unknown`.
    reported: BTreeSet<String>,
    days: i64,
    /// Events waiting for a player's choice, oldest first.
    pub held: Vec<HeldEvent>,
    next_key: u64,
    /// Days a held event waits for its player (default [`ANSWER_DAYS`]).
    pub answer_days: i64,
}

/// Whether a script block names `this` anywhere, as a key or a value (any case).
fn mentions_this(b: Option<&Block>) -> bool {
    fn value(v: &Value) -> bool {
        match v {
            Value::Block(b) => block(b),
            other => other.as_str().is_some_and(|s| s.eq_ignore_ascii_case("this")),
        }
    }
    fn block(b: &Block) -> bool {
        b.entries().any(|(k, v, _)| k.eq_ignore_ascii_case("this") || value(v))
    }
    b.is_some_and(block)
}

struct CompiledDecision {
    potential: Option<Conditions>,
    allow: Option<Conditions>,
    ai_will_do: Option<Weight>,
}

/// Whether the computer player takes a decision in a country: potential, allow, ai_will_do > 0.
fn decide(state: &State, facts: &Facts, d: &CompiledDecision, scope: &Scope) -> Result<bool, String> {
    let check = |b: &Option<Conditions>| b.as_ref().map_or(Ok(true), |b| b.hold(state, Some(facts), scope));
    if !check(&d.potential)? || !check(&d.allow)? {
        return Ok(false);
    }
    Ok(d.ai_will_do.as_ref().map_or(Ok(1.0), |w| cond::product(state, Some(facts), w, scope))? > 0.0)
}

fn threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 16)
}

/// `f` of every item, on a few scoped threads, in the items' order. For many small items of
/// about the same cost: each thread gets one run of them.
fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let chunk = items.len().div_ceil(threads()).max(32);
    if items.len() <= chunk {
        return items.iter().map(f).collect();
    }
    let f = &f;
    std::thread::scope(|s| {
        let handles: Vec<_> = items.chunks(chunk).map(|part| s.spawn(move || part.iter().map(f).collect::<Vec<R>>())).collect();
        handles.into_iter().flat_map(|h| h.join().expect("event worker panicked")).collect()
    })
}

/// `f` of every item, on a few scoped threads, in the items' order. For a few items of very
/// different cost: each thread takes the next item nobody took yet.
fn par_each<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = threads().min(items.len());
    if threads <= 1 {
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let (f, next) = (&f, &next);
    let mut found: Vec<(usize, R)> = std::thread::scope(|s| {
        let work = move || {
            let mut out = Vec::new();
            while let Some((i, item)) = Some(next.fetch_add(1, Ordering::Relaxed)).and_then(|i| Some((i, items.get(i)?))) {
                out.push((i, f(item)));
            }
            out
        };
        let handles: Vec<_> = (0..threads).map(|_| s.spawn(work)).collect();
        handles.into_iter().flat_map(|h| h.join().expect("event worker panicked")).collect()
    });
    found.sort_by_key(|(i, _)| *i);
    found.into_iter().map(|(_, r)| r).collect()
}

/// Events or decisions tested ahead in one go while nothing fires.
const AHEAD: usize = 96;
/// Provinces and countries one thread takes at a time of an event or decision tested ahead: a
/// test that is slow (one that adds up the pops of every country) is so shared by the threads.
const AHEAD_PROVINCES: usize = 256;
const AHEAD_COUNTRIES: usize = 16;
/// Targets tested ahead in one go for the rest of an event or decision after it fired.
const AHEAD_TARGETS: usize = 512;

/// The targets of a scan: owned provinces in id order, existing countries in tag order.
struct Targets {
    provinces: Vec<Scope>,
    countries: Vec<Scope>,
}

impl Targets {
    fn new(state: &State) -> Targets {
        Targets { provinces: state.provinces.iter().filter(|(_, p)| p.owner.is_some()).map(|(id, _)| Scope::Province(*id)).collect(), countries: state.existing().into_iter().map(Scope::Country).collect() }
    }
}

impl EventRunner {
    pub fn new(events: Vec<EventDef>, decisions: Vec<DecisionDef>) -> EventRunner {
        let by_id = events.iter().enumerate().map(|(i, e)| (e.id, i)).collect();
        let compiled_events = events.iter().map(|e| (e.trigger.as_ref().map(Conditions::new), e.mean_time.as_ref().map_or(Vec::new(), |m| m.get_all("modifier").filter_map(Value::as_block).map(Conditions::new).collect()))).collect();
        let compiled_decisions = decisions.iter().map(|d| CompiledDecision { potential: d.potential.as_ref().map(Conditions::new), allow: d.allow.as_ref().map(Conditions::new), ai_will_do: d.ai_will_do.as_ref().map(Weight::new) }).collect();
        let free_events = events.iter().map(|e| !mentions_this(e.trigger.as_ref()) && !mentions_this(e.mean_time.as_ref())).collect();
        let free_decisions = decisions.iter().map(|d| !mentions_this(d.potential.as_ref()) && !mentions_this(d.allow.as_ref()) && !mentions_this(d.ai_will_do.as_ref())).collect();
        EventRunner { events, decisions, compiled_events, compiled_decisions, free_events, free_decisions, by_id, fired_once: BTreeSet::new(), reported: BTreeSet::new(), days: 0, held: Vec::new(), next_key: 0, answer_days: ANSWER_DAYS }
    }

    /// The waiting time of an event for a target, in days: the last written of `days`,
    /// `months`, `years`, `factor` and `base` (they share one slot; 1 when none is written),
    /// times the factor of every modifier that holds.
    pub fn mean_days(&self, state: &State, rules: &EventRules, block: &Block, scope: &Scope) -> Result<f64, String> {
        let mut value = 1.0;
        for item in &block.items {
            match (item.key.as_deref(), item.value.as_f64()) {
                (Some("days" | "factor" | "base"), Some(x)) => value = x,
                (Some("months"), Some(x)) => value = x * rules.month_days,
                (Some("years"), Some(x)) => value = x * rules.year_days,
                _ => {}
            }
        }
        for m in block.get_all("modifier").filter_map(Value::as_block) {
            if conditions_hold(state, m, scope)? {
                value *= m.get("factor").and_then(Value::as_f64).unwrap_or(1.0);
            }
        }
        Ok(value)
    }

    /// The event `id` for `target` waits for its player's answer.
    pub fn is_held(&self, id: u64, target: &Scope) -> bool {
        self.held.iter().any(|h| h.id == id && h.target == *target)
    }

    /// Fires an event for a target: its `immediate` blocks, then one option. An event already
    /// held for the same target does nothing.
    #[allow(clippy::too_many_arguments)]
    pub fn fire(&mut self, state: &mut State, cal: &Calendar, rules: &EffectRules, id: u64, scope: &Scope, from: Option<Scope>, log: &mut EventLog) {
        let Some(&index) = self.by_id.get(&id) else {
            *log.unknown.entry(format!("event {id} (not defined)")).or_default() += 1;
            return;
        };
        // Already waiting for its player: it does not fire again (nor run `immediate` again).
        if self.is_held(id, scope) {
            return;
        }
        let event = self.events[index].clone();
        (state.this, state.from) = (Some(scope.clone()), from);
        for block in &event.immediate {
            run_effects(state, cal, rules, block, scope, &mut log.effects);
        }
        // Option weights; an option whose weight cannot be told counts as 1.
        let weights: Vec<f64> = event.options.iter().map(|o| o.get("ai_chance").and_then(Value::as_block).map_or(1.0, |c| weight(state, c, scope).unwrap_or(1.0).max(0.0))).collect();
        let total: f64 = weights.iter().sum();
        if !event.options.is_empty() {
            let mut at = if total > 0.0 { state.roll_fraction() * total } else { 0.0 };
            let mut pick = event.options.len() - 1;
            for (i, w) in weights.iter().enumerate() {
                if at < *w {
                    pick = i;
                    break;
                }
                at -= w;
            }
            match player_of(state, scope).filter(|_| event.options.len() > 1) {
                Some(tag) => {
                    self.next_key += 1;
                    let expires = cal.to_game(state.date).map_or(state.date, |g| cal.to_date(crate::calendar::GameDate(g.0 + 24 * self.answer_days.max(0))));
                    self.held.push(HeldEvent { key: self.next_key, id, tag, target: scope.clone(), from: state.from.clone(), ai_pick: pick, fired: state.date, expires });
                }
                None => run_effects(state, cal, rules, &event.options[pick], scope, &mut log.effects),
            }
        }
        *log.fired.entry(id).or_default() += 1;
        if event.fire_only_once {
            self.fired_once.insert(id);
        }
    }

    /// The player's answer to a held event: option `option` (from 0), or the computer's pick
    /// with `None`. Its effects run now, with THIS and FROM as when the event fired.
    #[allow(clippy::too_many_arguments)]
    pub fn choose(&mut self, state: &mut State, cal: &Calendar, rules: &EffectRules, key: u64, option: Option<usize>, log: &mut EventLog) -> Result<String, String> {
        let at = self.held.iter().position(|h| h.key == key).ok_or(format!("no event {key} is waiting"))?;
        let h = &self.held[at];
        let event = self.by_id.get(&h.id).map(|&i| &self.events[i]).ok_or(format!("event {} is not defined", h.id))?;
        let pick = option.unwrap_or(h.ai_pick);
        let Some(block) = event.options.get(pick).cloned() else { return Err(format!("event {} has {} options", h.id, event.options.len())) };
        let h = self.held.remove(at);
        (state.this, state.from) = (Some(h.target.clone()), h.from.clone());
        run_effects(state, cal, rules, &block, &h.target, &mut log.effects);
        Ok(format!("event {}: option {}", h.id, pick + 1))
    }

    /// Held events whose time is up, or whose country no longer has a player, take the
    /// computer's pick.
    pub fn answer_due(&mut self, state: &mut State, cal: &Calendar, rules: &EffectRules, log: &mut EventLog) {
        let today = state.date;
        let due: Vec<u64> = self.held.iter().filter(|h| h.expires <= today || !state.countries.get(&h.tag).is_some_and(|c| c.human)).map(|h| h.key).collect();
        for key in due {
            let _ = self.choose(state, cal, rules, key, None, log);
        }
    }

    /// The decisions a country's `potential` shows it, with whether `allow` holds now. A
    /// decision whose conditions the engine does not know is left out.
    /// THIS is set to the country while they are tested.
    pub fn open_decisions(&self, state: &mut State, tag: &str) -> Vec<(&DecisionDef, bool)> {
        let (facts, scope) = (Facts::new(), Scope::Country(tag.to_string()));
        (state.this, state.from) = (Some(scope.clone()), None);
        let st: &State = state;
        let hold = |b: &Option<Conditions>| b.as_ref().map_or(Ok(true), |b| b.hold(st, Some(&facts), &scope));
        let mut out = Vec::new();
        for (d, c) in self.decisions.iter().zip(&self.compiled_decisions) {
            if let (Ok(true), Ok(allowed)) = (hold(&c.potential), hold(&c.allow)) {
                out.push((d, allowed));
            }
        }
        out
    }

    /// A played country takes a decision: `potential` and `allow` must hold.
    pub fn take_decision(&mut self, state: &mut State, cal: &Calendar, rules: &EffectRules, tag: &str, name: &str, log: &mut EventLog) -> Result<String, String> {
        let index = self.decisions.iter().position(|d| d.name == name).ok_or(format!("no decision {name}"))?;
        let scope = Scope::Country(tag.to_string());
        (state.this, state.from) = (Some(scope.clone()), None);
        let facts = Facts::new();
        let c = &self.compiled_decisions[index];
        let hold = |b: &Option<Conditions>| b.as_ref().map_or(Ok(true), |b| b.hold(state, Some(&facts), &scope));
        if !hold(&c.potential).map_err(|u| format!("{name}: the engine does not know {u}"))? {
            return Err(format!("{name} is not open to {tag}"));
        }
        if !hold(&c.allow).map_err(|u| format!("{name}: the engine does not know {u}"))? {
            return Err(format!("{name}: its conditions do not hold"));
        }
        if let Some(effect) = self.decisions[index].effect.clone() {
            run_effects(state, cal, rules, &effect, &scope, &mut log.effects);
        }
        *log.decided.entry(name.to_string()).or_default() += 1;
        Ok(format!("decision {name} taken"))
    }

    fn note_unknown(&mut self, what: String, condition: String, log: &mut EventLog) {
        if self.reported.insert(what) {
            *log.unknown.entry(condition).or_default() += 1;
        }
    }

    /// A random event's test on one target: `None` when its trigger fails, else its mean days
    /// (as `mean_days`, with the compiled modifiers).
    fn test(&self, state: &State, facts: &Facts, event_rules: &EventRules, index: usize, target: &Scope) -> Result<Option<f64>, String> {
        let (event, (trigger, modifiers)) = (&self.events[index], &self.compiled_events[index]);
        if !trigger.as_ref().map_or(Ok(true), |t| t.hold(state, Some(facts), target))? {
            return Ok(None);
        }
        let Some(block) = &event.mean_time else { return Ok(Some(1.0)) };
        let mut value = 1.0;
        for item in &block.items {
            match (item.key.as_deref(), item.value.as_f64()) {
                (Some("days" | "factor" | "base"), Some(x)) => value = x,
                (Some("months"), Some(x)) => value = x * event_rules.month_days,
                (Some("years"), Some(x)) => value = x * event_rules.year_days,
                _ => {}
            }
        }
        for m in modifiers {
            if m.hold(state, Some(facts), target)? {
                value *= m.factor.unwrap_or(1.0);
            }
        }
        Ok(Some(value))
    }

    /// One day: the date moves on, expired modifiers go, due events fire, and on a check day
    /// every random event and decision is tested.
    pub fn day(&mut self, state: &mut State, cal: &Calendar, rules: &EffectRules, event_rules: &EventRules, log: &mut EventLog) {
        if let Some(g) = cal.to_game(state.date) {
            state.date = cal.to_date(cal.next_day(g));
        }
        self.days += 1;
        let today = state.date;
        let mut expired = 0;
        let mut expire = |list: &mut Vec<crate::script::TimedModifier>| {
            let before = list.len();
            list.retain(|m| m.expires.is_none_or(|d| d > today));
            expired += before - list.len();
        };
        state.provinces.values_mut().for_each(|p| expire(&mut p.modifiers));
        state.countries.values_mut().for_each(|c| expire(&mut c.modifiers));
        log.modifiers_expired += expired;

        // Held events whose time is up take the computer's pick.
        self.answer_due(state, cal, rules, log);
        // Events asked for by effects.
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut state.pending_events).into_iter().partition(|e| e.fires <= today);
        state.pending_events = later;
        for e in due {
            self.fire(state, cal, rules, e.id, &e.target, Some(e.from), log);
        }
        if self.days % event_rules.check_days != 0 {
            return;
        }

        // Random events. The scans the original made since the last test: countries on odd
        // days, provinces on even days.
        let (now, last) = (self.days, self.days - event_rules.check_days);
        let every = event_rules.scan_every_days;
        let country_scans = (now + 1).div_euclid(every) - (last + 1).div_euclid(every);
        let province_scans = now.div_euclid(every) - last.div_euclid(every);
        // Figures read from the state; made again after anything fires.
        let mut facts = Facts::new();
        // The targets, listed again before the next event or decision after anything fires.
        let mut targets = Targets::new(state);
        let mut stale = false;
        let due = |me: &EventRunner, index: usize| {
            let event = &me.events[index];
            !(event.triggered_only || (if event.province { province_scans } else { country_scans }) == 0 || me.fired_once.contains(&event.id))
        };
        // Tests of the coming events made ahead, by event index. They hold until something fires.
        let mut ahead: BTreeMap<usize, Vec<Result<Option<f64>, String>>> = BTreeMap::new();
        for index in 0..self.events.len() {
            if !due(self, index) {
                continue;
            }
            if stale {
                (targets, stale) = (Targets::new(state), false);
            }
            let event = &self.events[index];
            let (id, province, free) = (event.id, event.province, self.free_events[index]);
            let scans = if province { province_scans } else { country_scans };
            let list = if province { &targets.provinces } else { &targets.countries };
            // The tests of every target at once, on threads, when they cannot read `this` (the
            // one field the loop below sets per target): for this event and the next ones.
            if free && !ahead.contains_key(&index) {
                let (st, me, fx, tg): (&State, &EventRunner, &Facts, &Targets) = (state, self, &facts, &targets);
                let batch: Vec<usize> = (index..self.events.len()).filter(|i| self.free_events[*i] && due(self, *i)).take(AHEAD).collect();
                let work: Vec<(usize, &[Scope])> = batch.iter().flat_map(|i| if me.events[*i].province { tg.provinces.chunks(AHEAD_PROVINCES) } else { tg.countries.chunks(AHEAD_COUNTRIES) }.map(move |part| (*i, part))).collect();
                let found = par_each(&work, |(i, part)| part.iter().map(|target| me.test(st, fx, event_rules, *i, target)).collect::<Vec<_>>());
                ahead = batch.into_iter().map(|i| (i, Vec::new())).collect();
                for ((i, _), part) in work.iter().zip(found) {
                    ahead.entry(*i).or_default().extend(part);
                }
            }
            // `early[k]` is the test of target `early_from + k`.
            let (mut early, mut early_from) = (ahead.remove(&index).unwrap_or_default(), 0);
            for (n, target) in list.iter().enumerate() {
                if !self.held.is_empty() && self.is_held(id, target) {
                    continue;
                }
                state.this = Some(target.clone());
                if free && n >= early_from + early.len() {
                    // After a fire: the next targets on threads again.
                    let (st, me, fx): (&State, &EventRunner, &Facts) = (state, self, &facts);
                    early = par_map(&list[n..list.len().min(n + AHEAD_TARGETS)], |target| me.test(st, fx, event_rules, index, target));
                    early_from = n;
                }
                let mean = match free {
                    true => std::mem::replace(&mut early[n - early_from], Ok(None)),
                    false => self.test(state, &facts, event_rules, index, target),
                };
                match mean {
                    Ok(None) => {}
                    Ok(Some(mean)) => {
                        let chance = 1.0 - (1.0 - scan_chance(event_rules, mean, province)).powi(scans as i32);
                        if state.roll_fraction() < chance {
                            self.fire(state, cal, rules, id, target, None, log);
                            facts = Facts::new();
                            stale = true;
                            ahead.clear();
                            early.clear();
                            early_from = n + 1;
                            if self.fired_once.contains(&id) {
                                break;
                            }
                        }
                    }
                    Err(unknown) => {
                        self.note_unknown(format!("event {id}"), unknown, log);
                        break;
                    }
                }
            }
        }

        // Decisions. As for events: every country at once when `this` cannot matter, until one
        // decides.
        let mut ahead: BTreeMap<usize, Vec<Result<bool, String>>> = BTreeMap::new();
        for index in 0..self.decisions.len() {
            if stale {
                (targets, stale) = (Targets::new(state), false);
            }
            let (list, free) = (&targets.countries, self.free_decisions[index]);
            if free && !ahead.contains_key(&index) {
                let (st, me, fx): (&State, &EventRunner, &Facts) = (state, self, &facts);
                let batch: Vec<usize> = (index..self.decisions.len()).filter(|i| self.free_decisions[*i]).take(AHEAD).collect();
                let work: Vec<(usize, &[Scope])> = batch.iter().flat_map(|i| list.chunks(AHEAD_COUNTRIES).map(move |part| (*i, part))).collect();
                let found = par_each(&work, |(i, part)| part.iter().map(|scope| decide(st, fx, &me.compiled_decisions[*i], scope)).collect::<Vec<_>>());
                ahead = batch.into_iter().map(|i| (i, Vec::new())).collect();
                for ((i, _), part) in work.iter().zip(found) {
                    ahead.entry(*i).or_default().extend(part);
                }
            }
            let (mut early, mut early_from) = (ahead.remove(&index).unwrap_or_default(), 0);
            for (n, scope) in list.iter().enumerate() {
                state.this = Some(scope.clone());
                if free && n >= early_from + early.len() {
                    let (st, cd, fx): (&State, &CompiledDecision, &Facts) = (state, &self.compiled_decisions[index], &facts);
                    early = par_map(&list[n..list.len().min(n + AHEAD_TARGETS)], |scope| decide(st, fx, cd, scope));
                    early_from = n;
                }
                let take = match free {
                    true => std::mem::replace(&mut early[n - early_from], Ok(false)),
                    false => decide(state, &facts, &self.compiled_decisions[index], scope),
                };
                // The computer takes no decision for a country a person plays.
                let take = take.map(|t| t && player_of(state, scope).is_none());
                match take {
                    Ok(false) => {}
                    Ok(true) => {
                        let d = &self.decisions[index];
                        let (name, effect) = (d.name.clone(), d.effect.clone());
                        state.from = None;
                        if let Some(effect) = effect {
                            run_effects(state, cal, rules, &effect, scope, &mut log.effects);
                        }
                        facts = Facts::new();
                        stale = true;
                        ahead.clear();
                        early.clear();
                        early_from = n + 1;
                        *log.decided.entry(name).or_default() += 1;
                    }
                    Err(unknown) => {
                        let name = self.decisions[index].name.clone();
                        self.note_unknown(format!("decision {name}"), unknown, log);
                        break;
                    }
                }
            }
        }
    }
}
