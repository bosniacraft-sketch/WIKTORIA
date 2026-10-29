//! The live source: a `wiktoria_simulation::game::Game` built from a save, ticking on a background thread.
//!
//! The sim thread owns the `Game`. After a day (at most every 150 ms of wall time, and always when it
//! pauses) it builds a [`Snapshot`] of everything the panels show and publishes it through an
//! `Arc<Mutex<Option<Arc<Snapshot>>>>`. The UI thread only swaps the `Arc` in `poll()`, so it never waits
//! on a day. Speed and pause go the other way through atomics in [`Control`].
//!
//! Messages (wars declared and ended, battles, occupations, risings, factories) are found by comparing
//! the state before and after each day, so `wiktoria_simulation` needs no extra log.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wiktoria_data::date::Date;
use wiktoria_data::world::World;
use wiktoria_render::{MapMode, Palette};
use wiktoria_simulation::game::Game;
use wiktoria_simulation::military::battle::CombatKind;
use wiktoria_simulation::script::ProvinceState;

use crate::watch::*;

/// Target days per second for speeds 1..=5 (5 = as fast as the sim goes).
const SPEED_DPS: [f64; 5] = [1.0, 2.0, 4.0, 8.0, f64::INFINITY];
const MAX_MESSAGES: usize = 300;

pub struct Control {
    pub paused: AtomicBool,
    pub speed: AtomicU8,
    pub player: AtomicU16,
    pub stop: AtomicBool,
    /// Days run since the start; the sim pauses itself at `limit` (GM_UI_RUN_DAYS), 0 = no limit.
    pub days_run: AtomicU64,
    pub limit: AtomicU64,
}

#[derive(Clone, Default)]
pub struct Army {
    pub owner: u16,
    pub navy: bool,
    pub name: String,
    pub detail: String,
}

/// Everything the UI reads, as of one day.
#[derive(Default)]
pub struct Snapshot {
    pub date: Option<Date>,
    pub data: WatchData,
    pub summaries: HashMap<u16, CountrySummary>,
    pub prestige: HashMap<u16, f64>,
    pub treasury: HashMap<u16, f64>,
    pub provinces: BTreeMap<u16, ProvinceState>,
    pub owner: Vec<Option<u16>>,
    pub controller: Vec<Option<u16>>,
    /// Factory levels by state region, and the factories (kind, level, owner tag) in each.
    pub factory_levels: HashMap<u16, f64>,
    pub factories: HashMap<u16, Vec<(String, f64)>>,
    /// Country -> (war number, on the attacking side).
    pub war_side: HashMap<u16, (usize, bool)>,
    pub units: Vec<Army>,
    pub ideologies: Vec<String>,
    /// Map-visible data version.
    pub revision: u64,
    pub days_per_sec: f64,
    pub day_ms: f64,
    pub days_run: u64,
}

pub struct GameSource {
    fallback: StaticWorldSource,
    snap: Option<Arc<Snapshot>>,
    shared: Arc<Mutex<Option<Arc<Snapshot>>>>,
    status: Arc<Mutex<String>>,
    pub control: Arc<Control>,
}

/// Days to run flat out before handing control to the UI: `--start-days N` or GM_UI_RUN_DAYS=N.
fn start_days() -> u64 {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == "--start-days")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .or_else(|| std::env::var("GM_UI_RUN_DAYS").ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn tag_idx(w: &World, tag: &str) -> Option<u16> {
    w.tag_index.get(tag).copied()
}

impl GameSource {
    /// `world` is the UI's copy (map, names); the game loads its own on the sim thread.
    pub fn start(world: World, root: PathBuf, install: PathBuf, save: PathBuf) -> GameSource {
        let fallback = StaticWorldSource::new(world);
        let player0 = fallback.data().ledger.first().map_or(0, |r| r.country);
        let control = Arc::new(Control {
            paused: AtomicBool::new(true),
            speed: AtomicU8::new(1),
            player: AtomicU16::new(player0),
            stop: AtomicBool::new(false),
            days_run: AtomicU64::new(0),
            limit: AtomicU64::new(start_days()),
        });
        let shared = Arc::new(Mutex::new(None));
        let status = Arc::new(Mutex::new(format!("building game from {} ...", save.display())));
        {
            let (control, shared, status) = (control.clone(), shared.clone(), status.clone());
            std::thread::Builder::new()
                .name("sim".into())
                .spawn(move || {
                    if let Err(e) = sim_thread(&root, &install, &save, &control, &shared, &status) {
                        eprintln!("sim: {e}");
                        *status.lock().unwrap() = format!("sim failed: {e}");
                    }
                })
                .expect("spawn sim thread");
        }
        GameSource { fallback, snap: None, shared, status, control }
    }

    fn country_idx(&self, idx: u16) -> CountryFacts {
        let c = self.fallback.world().country(idx);
        let s = self.snap.as_ref();
        CountryFacts {
            idx,
            tag: c.country.tag.clone(),
            name: c.name.clone(),
            color: c.country.color,
            prestige: s.and_then(|s| s.prestige.get(&idx).copied()).or(c.history.prestige),
            treasury: s.and_then(|s| s.treasury.get(&idx).copied()),
        }
    }
}

impl Drop for GameSource {
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::Relaxed);
    }
}

fn load_game(root: &Path, install: &Path, save_path: &Path) -> Result<Game, String> {
    let engine_rules = wiktoria_data::config::load_engine_rules(root)?;
    let w = World::load(install, &engine_rules)?;
    let pop_types = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let bytes = std::fs::read(save_path).map_err(|e| format!("cannot read {}: {e}", save_path.display()))?;
    let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
    Game::new(w, &engine_rules, install, &save)
}

/// What a day is compared against to find messages.
struct Before {
    wars: BTreeMap<String, (Vec<String>, Vec<String>)>,
    controller: BTreeMap<u16, Option<String>>,
    battles: Vec<(u16, Vec<u64>, Vec<u64>)>,
    rebel_armies: BTreeSet<u64>,
}

fn before(g: &Game) -> Before {
    let mil = &g.core.mil;
    Before {
        wars: mil.wars.iter().filter(|w| w.active).map(|w| (w.name.clone(), (w.attackers.clone(), w.defenders.clone()))).collect(),
        controller: g.state.provinces.iter().map(|(id, p)| (*id, p.controller.clone())).collect(),
        battles: mil.combats.iter().filter(|c| c.kind == CombatKind::Land).map(|c| (c.location, c.attacker.units.clone(), c.defender.units.clone())).collect(),
        rebel_armies: mil.armies_of(&g.core.day.rebel_tag).map(|a| a.id).collect(),
    }
}

fn messages_of_day(g: &Game, b: &Before, log: &wiktoria_simulation::game::DayLog, player: &str) -> Vec<String> {
    let w = &g.world;
    let pname = |id: u16| w.province(id).map_or(format!("#{id}"), |p| p.name.clone());
    let cname = |t: &str| tag_idx(w, t).map_or(t.to_string(), |i| w.country(i).name.clone());
    let mut out = Vec::new();
    let mil = &g.core.mil;
    let now: BTreeMap<&str, &wiktoria_simulation::military::war::War> = mil.wars.iter().filter(|w| w.active).map(|w| (w.name.as_str(), w)).collect();
    for (name, war) in &now {
        if !b.wars.contains_key(*name) {
            out.push(format!("War declared: {name} ({} vs {})", war.attackers.iter().map(|t| cname(t)).collect::<Vec<_>>().join(", "), war.defenders.iter().map(|t| cname(t)).collect::<Vec<_>>().join(", ")));
        }
    }
    for name in b.wars.keys() {
        if !now.contains_key(name.as_str()) {
            out.push(format!("Peace: {name} has ended"));
        }
    }
    // Battles that are no longer running: the side whose armies still stand there won.
    for (loc, att, def) in &b.battles {
        if mil.combats.iter().any(|c| c.kind == CombatKind::Land && c.location == *loc) {
            continue;
        }
        let alive = |ids: &Vec<u64>| ids.iter().any(|id| mil.armies.iter().any(|a| a.id == *id && a.location == *loc));
        let owner = |ids: &Vec<u64>| ids.iter().find_map(|id| mil.armies.iter().find(|a| a.id == *id).map(|a| a.owner.clone()));
        let (wo, lo) = match (alive(att), alive(def)) {
            (true, false) => (owner(att), owner(def)),
            (false, true) => (owner(def), owner(att)),
            _ => (None, None),
        };
        match (wo, lo) {
            (Some(win), lose) => {
                let lose = lose.map_or("the enemy".to_string(), |t| cname(&t));
                let tagline = if win == player { " (won)" } else if Some(player) == owner(att).or(owner(def)).as_deref() { " (lost)" } else { "" };
                out.push(format!("Battle of {}: {} beat {}{tagline}", pname(*loc), cname(&win), lose));
            }
            _ => out.push(format!("Battle of {} ended", pname(*loc))),
        }
    }
    // Occupation.
    let mut occ = 0usize;
    for (id, p) in &g.state.provinces {
        let was = b.controller.get(id).cloned().flatten();
        if p.controller != was {
            occ += 1;
            if occ <= 6 {
                let who = p.controller.as_deref().map_or("nobody".to_string(), cname);
                if p.controller == p.owner {
                    out.push(format!("{who} took back {}", pname(*id)));
                } else {
                    out.push(format!("{who} occupied {}", pname(*id)));
                }
            }
        }
    }
    if occ > 6 {
        out.push(format!("... and {} more provinces changed hands", occ - 6));
    }
    // Rebels.
    let new_rebels: Vec<&wiktoria_simulation::military::units::Army> = mil.armies_of(&g.core.day.rebel_tag).filter(|a| !b.rebel_armies.contains(&a.id)).collect();
    for a in new_rebels.iter().take(4) {
        let owner = g.state.provinces.get(&a.location).and_then(|p| p.owner.as_deref()).map_or(String::new(), |t| format!(" ({})", cname(t)));
        out.push(format!("Rebels rose in {}{owner}: {} regiments", pname(a.location), a.regiments.len()));
    }
    if new_rebels.is_empty() && log.core.risings > 0 {
        out.push(format!("{} rebel risings ({} regiments)", log.core.risings, log.core.rebel_regiments));
    }
    // Factories.
    for (tag, kind) in log.built.started_by_pops.iter().chain(&log.built.started_by_states).take(5) {
        out.push(format!("{} began building a {}", cname(tag), kind.replace('_', " ")));
    }
    if log.built.opened > 0 {
        out.push(format!("{} factories opened", log.built.opened));
    }
    for (tag, tech) in &log.researched {
        if tag == player {
            out.push(format!("Researched {}", tech.replace('_', " ")));
        }
    }
    out
}

fn sim_thread(root: &Path, install: &Path, save: &Path, control: &Control, shared: &Mutex<Option<Arc<Snapshot>>>, status: &Mutex<String>) -> Result<(), String> {
    let t = Instant::now();
    let mut game = load_game(root, install, save)?;
    println!("sim: game built in {:.1?} at {}", t.elapsed(), game.state.date);
    *status.lock().unwrap() = String::new();
    let party_names: Vec<String> = game.world.countries.iter().flat_map(|c| c.country.parties.iter().map(|p| p.name.clone())).collect();
    let mut messages: VecDeque<MessageEntry> = VecDeque::new();
    messages.push_back(MessageEntry { date: game.state.date, text: format!("Game started from {}", save.file_name().map_or(String::new(), |f| f.to_string_lossy().into_owned())) });
    let mut revision = 1u64;
    let mut map_key: u64 = 0;
    let mut times: VecDeque<(Instant, f64)> = VecDeque::new();
    let mut last_pub = Instant::now() - Duration::from_secs(10);
    let mut dirty = true;
    let mut day_ms = 0.0;
    loop {
        if control.stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let limit = control.limit.load(Ordering::Relaxed);
        let run = control.days_run.load(Ordering::Relaxed);
        // Jump ahead: the first `limit` days run flat out whatever the UI says; then the UI's pause rules.
        let prerun = run < limit;
        let paused = !prerun && control.paused.load(Ordering::Relaxed);
        if paused {
            if dirty {
                publish(&game, control, shared, &party_names, &messages, &mut revision, &mut map_key, &times, day_ms);
                dirty = false;
            }
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        let start = Instant::now();
        let player = game.world.country(control.player.load(Ordering::Relaxed)).country.tag.clone();
        let b = before(&game);
        let log = game.day();
        for text in messages_of_day(&game, &b, &log, &player) {
            messages.push_back(MessageEntry { date: game.state.date, text });
        }
        while messages.len() > MAX_MESSAGES {
            messages.pop_front();
        }
        control.days_run.fetch_add(1, Ordering::Relaxed);
        let took = start.elapsed();
        day_ms = took.as_secs_f64() * 1000.0;
        times.push_back((Instant::now(), took.as_secs_f64()));
        while times.len() > 1 && times.front().is_some_and(|(t, _)| t.elapsed() > Duration::from_secs(10)) {
            times.pop_front();
        }
        dirty = true;
        if last_pub.elapsed() > Duration::from_millis(150) {
            publish(&game, control, shared, &party_names, &messages, &mut revision, &mut map_key, &times, day_ms);
            last_pub = Instant::now();
            dirty = false;
        }
        let speed = control.speed.load(Ordering::Relaxed).clamp(1, 5);
        let target = SPEED_DPS[speed as usize - 1];
        if target.is_finite() && !prerun {
            let want = Duration::from_secs_f64(1.0 / target);
            // Sleep in small steps so pause and speed changes act at once.
            while start.elapsed() < want && !control.paused.load(Ordering::Relaxed) && control.speed.load(Ordering::Relaxed) == speed && !control.stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10).min(want.saturating_sub(start.elapsed())));
            }
        }
    }
}

fn opinion_name(level: u8) -> &'static str {
    use wiktoria_simulation::diplomacy::relations::level::*;
    match level {
        HOSTILE => "Hostile",
        OPPOSED => "Opposed",
        CORDIAL => "Cordial",
        FRIENDLY => "Friendly",
        IN_SPHERE => "In sphere",
        _ => "Neutral",
    }
}

#[allow(clippy::too_many_arguments)]
fn publish(g: &Game, control: &Control, shared: &Mutex<Option<Arc<Snapshot>>>, party_names: &[String], messages: &VecDeque<MessageEntry>, revision: &mut u64, map_key: &mut u64, times: &VecDeque<(Instant, f64)>, day_ms: f64) {
    let snap = build_snapshot(g, control, party_names, messages, revision, map_key, times, day_ms);
    *shared.lock().unwrap() = Some(Arc::new(snap));
}

#[allow(clippy::too_many_arguments)]
fn build_snapshot(g: &Game, control: &Control, party_names: &[String], messages: &VecDeque<MessageEntry>, revision: &mut u64, map_key: &mut u64, times: &VecDeque<(Instant, f64)>, day_ms: f64) -> Snapshot {
    let (w, s, mil) = (&g.world, &g.state, &g.core.mil);
    let dip = g.core.dip.as_ref().map(|(_, d)| d);
    let player = control.player.load(Ordering::Relaxed);
    let ptag = w.country(player).country.tag.clone();
    let idx = |t: &str| tag_idx(w, t);
    let n = w.provinces.len();
    let mut owner = vec![None; n];
    let mut controller = vec![None; n];
    let mut pop_by: HashMap<String, f64> = HashMap::new();
    for (id, p) in &s.provinces {
        if (*id as usize) < n {
            owner[*id as usize] = p.owner.as_deref().and_then(idx);
            controller[*id as usize] = p.controller.as_deref().and_then(idx);
        }
        if let Some(o) = &p.owner {
            *pop_by.entry(o.clone()).or_default() += p.pops.iter().map(|q| q.size).sum::<f64>();
        }
    }
    let mut factory_levels: HashMap<u16, f64> = HashMap::new();
    let mut factories: HashMap<u16, Vec<(String, f64)>> = HashMap::new();
    let mut factory_count: HashMap<String, usize> = HashMap::new();
    for f in &s.factories {
        if let Some(r) = f.region {
            *factory_levels.entry(r).or_default() += f.level.max(0.0);
            factories.entry(r).or_default().push((f.kind.clone(), f.level));
        }
        *factory_count.entry(f.owner.clone()).or_default() += 1;
    }
    // Map key: owners, controllers and factory levels; bump the revision when any of them changed.
    let mut h: u64 = 1469598103934665603;
    let mut mix = |x: u64| h = (h ^ x).wrapping_mul(1099511628211);
    for (o, c) in owner.iter().zip(&controller) {
        mix(o.map_or(0, |v| v as u64 + 1));
        mix(c.map_or(0, |v| v as u64 + 1));
    }
    let mut fl: Vec<_> = factory_levels.iter().collect();
    fl.sort_by_key(|x| *x.0);
    for (r, l) in fl {
        mix(*r as u64);
        mix((l * 100.0) as u64);
    }
    if h != *map_key {
        *map_key = h;
        *revision += 1;
    }
    // Wars.
    let scores = g.core.day.war_scores(mil, s);
    let mut war_side = HashMap::new();
    let mut wars = Vec::new();
    for (i, war) in mil.wars.iter().filter(|w| w.active).enumerate() {
        let a: Vec<u16> = war.attackers.iter().filter_map(|t| idx(t)).collect();
        let d: Vec<u16> = war.defenders.iter().filter_map(|t| idx(t)).collect();
        for c in &a {
            war_side.entry(*c).or_insert((i, true));
        }
        for c in &d {
            war_side.entry(*c).or_insert((i, false));
        }
        let score = scores.iter().find(|(n, _)| *n == war.name).map(|x| x.1);
        wars.push(WarEntry { name: format!("{}{}", war.name, score.map_or(String::new(), |x| format!("  ({x:+.0})"))), attackers: a, defenders: d, war_score: score.map(|x| ((x + 100.0) / 200.0).clamp(0.0, 1.0) as f32) });
    }
    let enemies_of = |t: &str| -> BTreeSet<String> {
        mil.wars.iter().filter(|w| w.active).flat_map(|w| {
            if w.attackers.iter().any(|x| x == t) {
                w.defenders.clone()
            } else if w.defenders.iter().any(|x| x == t) {
                w.attackers.clone()
            } else {
                Vec::new()
            }
        }).collect()
    };
    // Ranking.
    let order: Vec<&String> = s.rank_order.iter().filter(|t| s.countries.contains_key(*t)).collect();
    let rank_of: HashMap<&str, u32> = order.iter().enumerate().map(|(i, t)| (t.as_str(), i as u32 + 1)).collect();
    let mut ledger = Vec::new();
    for t in &order {
        let Some(i) = idx(t) else { continue };
        let c = &s.countries[*t];
        ledger.push(LedgerRow { country: i, rank: rank_of[t.as_str()], military: Some(c.military_score), industrial: Some(s.industrial_score(t)), prestige: Some(c.prestige), total: Some(c.total_score) });
    }
    let gps: Vec<&String> = order.iter().copied().filter(|t| s.great_powers.contains(t)).chain(s.great_powers.iter().filter(|t| !rank_of.contains_key(t.as_str()))).collect();
    let ranking: Vec<RankEntry> = gps.iter().filter_map(|t| Some(RankEntry { country: idx(t)?, rank: rank_of.get(t.as_str()).copied().unwrap_or(0), score: s.countries.get(*t).map_or(0.0, |c| c.total_score) })).collect();
    // Summaries.
    let mut summaries = HashMap::new();
    let mut prestige = HashMap::new();
    let mut treasury = HashMap::new();
    for (t, c) in &s.countries {
        let Some(i) = idx(t) else { continue };
        prestige.insert(i, c.prestige);
        treasury.insert(i, c.money);
        let men = |tag: &str| mil.armies_of(tag).flat_map(|a| &a.regiments).map(|r| r.strength).sum::<f64>() * g.core.day.men_per_strength;
        summaries.insert(
            i,
            CountrySummary {
                rank: rank_of.get(t.as_str()).copied(),
                prestige: Some(c.prestige),
                industry_score: Some(s.industrial_score(t)),
                military_score: Some(c.military_score),
                government: c.government.clone(),
                ruling_party: c.ruling_party.and_then(|p| party_names.get((p as usize).checked_sub(1)?).cloned()),
                treasury: Some(c.money),
                daily_balance: None,
                research: c.research.as_ref().map(|(n, _)| n.replace('_', " ")),
                population: pop_by.get(t).copied(),
                pop_growth: None,
                trade: None,
                at_war: Some(s.wars.iter().any(|(a, d)| a.contains(t) || d.contains(t))),
                army_size: Some(men(t)),
                navy_size: Some(mil.ship_count(t) as f64),
                war_exhaustion: Some(c.war_exhaustion),
                sphere_leader: dip.and_then(|d| d.sphere_leader(t)).and_then(idx),
                overlord: dip.and_then(|d| d.overlord_of(t)).and_then(idx),
                allies: dip.map(|d| d.allies_of(t).filter_map(idx).collect()).unwrap_or_default(),
            },
        );
    }
    // Diplomacy rows, seen from the player.
    let enemies = enemies_of(&ptag);
    let mut neighbours: BTreeSet<String> = BTreeSet::new();
    for (id, p) in &s.provinces {
        if p.owner.as_deref() == Some(ptag.as_str()) {
            for nb in s.neighbours.get(id).into_iter().flatten() {
                if let Some(o) = s.provinces.get(nb).and_then(|q| q.owner.clone()) {
                    neighbours.insert(o);
                }
            }
        }
    }
    let gp_cols: Vec<&String> = gps.iter().copied().take(8).collect();
    let diplomacy: Vec<DiplomacyRow> = order
        .iter()
        .filter(|t| ***t != ptag)
        .filter_map(|t| {
            let i = idx(t)?;
            let region = w.country(i).history.capital.and_then(|cap| w.province(cap)).and_then(|p| p.continent.clone()).unwrap_or_else(|| "Other".into());
            Some(DiplomacyRow {
                country: i,
                region,
                relation: dip.map(|d| d.relation_value(&ptag, t).round() as i32),
                influence: gp_cols.iter().map(|gp| dip.map(|d| d.influence(gp, t)).filter(|x| *x > 0.0)).collect(),
                opinion: dip.map(|d| opinion_name(d.level(&ptag, t)).to_string()),
                neighbour: neighbours.contains(*t),
                in_sphere: dip.and_then(|d| d.sphere_leader(t)) == Some(ptag.as_str()),
                allied: dip.is_some_and(|d| d.allied(&ptag, t)),
                enemy: enemies.contains(*t),
            })
        })
        .collect();
    // Outliner and units.
    let pname = |id: u16| w.province(id).map_or(format!("#{id}"), |p| p.name.clone());
    let mut units = Vec::new();
    let mut outliner = Vec::new();
    for a in &mil.armies {
        if a.owner == ptag {
            let men = a.regiments.iter().map(|r| r.strength).sum::<f64>() * g.core.day.men_per_strength;
            let moving = a.path.last().map_or(String::new(), |d| format!(" -> {}", pname(*d)));
            let fight = if mil.combats.iter().any(|c| c.attacker.units.contains(&a.id) || c.defender.units.contains(&a.id)) { ", in combat" } else { "" };
            let e = OutlinerEntry { kind: OutlinerKind::Army, name: a.name.clone(), detail: format!("{}{moving}: {} regts, {} men{fight}", pname(a.location), a.regiments.len(), wiktoria_render::fmt_int(men)) };
            outliner.push(e);
        }
        if let Some(o) = idx(&a.owner) {
            units.push(Army { owner: o, navy: false, name: a.name.clone(), detail: String::new() });
        }
    }
    for nv in mil.navies_of(&ptag) {
        let strength: f64 = nv.ships.iter().map(|x| x.strength).sum::<f64>() / nv.ships.len().max(1) as f64;
        outliner.push(OutlinerEntry { kind: OutlinerKind::Navy, name: nv.name.clone(), detail: format!("{}: {} ships, {:.0}% strength", pname(nv.location), nv.ships.len(), strength) });
    }
    if let Some(c) = s.countries.get(&ptag) {
        let mut inf: Vec<(&String, &f64)> = c.influence.iter().filter(|(_, v)| **v > 0.0).collect();
        inf.sort_by(|a, b| b.1.total_cmp(a.1));
        for (t, v) in inf.into_iter().take(10) {
            let name = idx(t).map_or(t.clone(), |i| w.country(i).name.clone());
            outliner.push(OutlinerEntry { kind: OutlinerKind::Influence, name, detail: format!("{v:.0}") });
        }
    }
    let population_by_country = {
        let mut v = vec![0.0; w.countries.len()];
        for (t, p) in &pop_by {
            if let Some(i) = idx(t) {
                v[i as usize] = *p;
            }
        }
        v
    };
    let span: f64 = times.iter().map(|x| x.1).sum();
    let window = times.front().map_or(0.0, |(t, _)| t.elapsed().as_secs_f64());
    let days_per_sec = if times.len() >= 2 && window > 0.0 { (times.len() - 1) as f64 / window } else if span > 0.0 { times.len() as f64 / span } else { 0.0 };
    Snapshot {
        date: Some(s.date),
        data: WatchData {
            ranking,
            ranking_note: "Great powers by total score (rank_day).".into(),
            wars_note: format!("{} wars running; bar = attackers' war score.", wars.len()),
            wars,
            ledger,
            diplomacy,
            outliner,
            messages: messages.iter().cloned().collect(),
            population_by_country,
        },
        summaries,
        prestige,
        treasury,
        provinces: s.provinces.clone(),
        owner,
        controller,
        factory_levels,
        factories,
        war_side,
        units,
        ideologies: s.ideologies.clone(),
        revision: *revision,
        days_per_sec,
        day_ms,
        days_run: control.days_run.load(Ordering::Relaxed),
    }
}

fn blend(a: u32, b: u32, t: f64) -> u32 {
    let ch = |s: u32| -> [f64; 3] { [((s >> 16) & 255) as f64, ((s >> 8) & 255) as f64, (s & 255) as f64] };
    let (x, y) = (ch(a), ch(b));
    let c: Vec<u32> = (0..3).map(|i| (x[i] + (y[i] - x[i]) * t).round() as u32).collect();
    (c[0] << 16) | (c[1] << 8) | c[2]
}

impl WatchSource for GameSource {
    fn world(&self) -> &World {
        self.fallback.world()
    }
    fn date(&self) -> Date {
        self.snap.as_ref().and_then(|s| s.date).unwrap_or_else(|| self.fallback.date())
    }
    fn data(&self) -> &WatchData {
        self.snap.as_ref().map_or_else(|| self.fallback.data(), |s| &s.data)
    }
    fn revision(&self) -> u64 {
        self.snap.as_ref().map_or(0, |s| s.revision)
    }
    fn poll(&mut self) {
        if let Some(s) = self.shared.lock().unwrap().take() {
            self.snap = Some(s);
        }
    }
    fn set_control(&self, paused: bool, speed: u8, player: u16) {
        self.control.paused.store(paused, Ordering::Relaxed);
        self.control.speed.store(speed, Ordering::Relaxed);
        self.control.player.store(player, Ordering::Relaxed);
    }
    fn live(&self) -> bool {
        true
    }
    fn sim_status(&self) -> Option<String> {
        let st = self.status.lock().unwrap().clone();
        if !st.is_empty() {
            return Some(st);
        }
        let s = self.snap.as_ref()?;
        Some(format!("sim {:.2} days/s ({:.0} ms/day), day {}", s.days_per_sec, s.day_ms, s.days_run))
    }
    fn capture_ready(&self) -> bool {
        let Some(s) = self.snap.as_ref() else { return false };
        let limit = self.control.limit.load(Ordering::Relaxed);
        s.days_run >= limit
    }
    fn supports(&self, _mode: ViewMode) -> bool {
        true
    }
    fn country(&self, idx: u16) -> CountryFacts {
        self.country_idx(idx)
    }
    fn country_summary(&self, idx: u16) -> CountrySummary {
        match self.snap.as_ref().and_then(|s| s.summaries.get(&idx)) {
            Some(x) => x.clone(),
            None => self.fallback.country_summary(idx),
        }
    }
    fn province(&self, id: u16) -> Option<ProvinceFacts> {
        let Some(snap) = self.snap.as_ref() else { return self.fallback.province(id) };
        let base = self.fallback.province(id)?;
        let Some(p) = snap.provinces.get(&id) else { return Some(base) };
        let w = self.world();
        let total: f64 = p.pops.iter().map(|q| q.size).sum();
        let group = |key: &dyn Fn(&wiktoria_simulation::script::PopState) -> &str| {
            let mut acc: BTreeMap<String, f64> = BTreeMap::new();
            for q in &p.pops {
                *acc.entry(key(q).to_string()).or_default() += q.size;
            }
            let mut v: Vec<(String, f64)> = acc.into_iter().collect();
            v.sort_by(|a, b| b.1.total_cmp(&a.1));
            v
        };
        let share = |v: Vec<(String, f64)>| -> Vec<(String, f64)> { v.into_iter().map(|(n, x)| (n, if total > 0.0 { x / total } else { 0.0 })).collect() };
        let weighted = |f: &dyn Fn(&wiktoria_simulation::script::PopState) -> f64| if total > 0.0 { p.pops.iter().map(|q| f(q) * q.size).sum::<f64>() / total } else { 0.0 };
        let mut ideo: BTreeMap<u32, f64> = BTreeMap::new();
        for q in &p.pops {
            for (k, v) in &q.ideology {
                *ideo.entry(*k).or_default() += v / 100.0 * q.size;
            }
        }
        let mut ideologies: Vec<(String, f64)> = ideo.into_iter().filter_map(|(k, v)| Some((snap.ideologies.get((k as usize).checked_sub(1)?)?.clone(), if total > 0.0 { v / total } else { 0.0 }))).collect();
        ideologies.sort_by(|a, b| b.1.total_cmp(&a.1));
        let lvl = |name: &str| p.buildings.get(name).copied().unwrap_or(0.0) as u32;
        let buildings = p.region.and_then(|r| snap.factories.get(&r)).map(|v| v.iter().map(|(k, l)| (k.replace('_', " "), *l as u32)).collect()).unwrap_or_default();
        let fact = |t: &Option<String>| t.as_deref().and_then(|t| tag_idx(w, t)).map(|i| self.country_idx(i));
        Some(ProvinceFacts {
            owner: fact(&p.owner),
            controller: fact(&p.controller),
            rgo_good: p.trade_goods.clone(),
            total_pop: total,
            pop_types: group(&|q| &q.pop_type),
            cultures: share(group(&|q| &q.culture)),
            religions: share(group(&|q| &q.religion)),
            fort: lvl("fort"),
            naval_base: lvl("naval_base"),
            railroad: lvl("railroad"),
            buildings,
            life_rating: Some(p.life_rating),
            literacy: weighted(&|q| q.literacy),
            militancy: weighted(&|q| q.militancy),
            cores: p.cores.clone(),
            ideologies,
            colonial: p.colonial > 0,
            ..base
        })
    }
    fn palette(&self, mode: ViewMode) -> Palette {
        let Some(snap) = self.snap.as_ref() else { return self.fallback.palette(mode) };
        let w = self.world();
        if mode == ViewMode::Terrain {
            return self.fallback.palette(mode);
        }
        let mut pal = Palette::build(w, MapMode::Political);
        let color = |i: u16| wiktoria_render::rgb(w.country(i).country.color);
        let max_pop = snap.provinces.values().map(|p| p.pops.iter().map(|q| q.size).sum::<f64>()).fold(0.0, f64::max);
        let max_lvl = snap.factory_levels.values().copied().fold(0.0, f64::max);
        for p in w.provinces.iter().flatten() {
            let id = p.id as usize;
            if p.is_sea || id >= snap.owner.len() {
                continue;
            }
            let o = snap.owner[id];
            let c = snap.controller[id];
            pal.owner_key[id] = o.map_or(0, |v| v as u32 + 1);
            pal.color[id] = match mode {
                ViewMode::Political => o.map_or(wiktoria_render::UNOWNED, color),
                ViewMode::Population => {
                    let pop = snap.provinces.get(&p.id).map_or(0.0, |q| q.pops.iter().map(|x| x.size).sum());
                    wiktoria_render::population_color(pop, max_pop)
                }
                ViewMode::War => {
                    let side = |x: Option<u16>| x.and_then(|x| snap.war_side.get(&x)).map(|(_, att)| if *att { 0xc0392b } else { 0x2e6fb8 });
                    let base = match side(o) {
                        Some(sc) => blend(o.map_or(0x777777, color), sc, 0.65),
                        None => blend(o.map_or(wiktoria_render::UNOWNED, color), 0x8a8a8a, 0.8),
                    };
                    if o.is_some() && c != o {
                        // Occupied: the controller's colour, pushed to its side's colour and lightened.
                        let cc = c.map_or(0x000000, color);
                        let s = side(c).unwrap_or(0xe0e0e0);
                        blend(blend(cc, s, 0.4), 0xffffff, 0.25)
                    } else {
                        base
                    }
                }
                ViewMode::Industry => {
                    let lvl = snap.provinces.get(&p.id).and_then(|q| q.region).and_then(|r| snap.factory_levels.get(&r)).copied().unwrap_or(0.0);
                    if o.is_none() {
                        wiktoria_render::UNOWNED
                    } else if lvl <= 0.0 {
                        0x4a4a46
                    } else {
                        let t = (lvl / max_lvl.max(1.0)).sqrt();
                        blend(0x6b5a2a, 0xffd34d, t)
                    }
                }
                ViewMode::Terrain => unreachable!(),
            };
        }
        pal
    }
}
