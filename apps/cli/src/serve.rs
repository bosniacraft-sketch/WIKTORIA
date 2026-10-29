//! `wiktoria-cli serve --save <file.v2> [--port 8036]`: the combined game (wiktoria_simulation::game::Game) on a
//! background thread, and a small web server on 127.0.0.1 that answers the page in `apps/web/`.
//! The contract is `apps/web/API.md`. Standard library only: hand-written HTTP/1.1 and JSON.
//!
//! The game lies in a `Mutex`. The simulation thread takes it for one day at a time; requests take
//! it between days (a request that waits is let in before the next day starts). The map picture
//! is drawn outside the lock from a palette made inside it.
//!
//! Money: treasuries, banks, debts and the day's report are in pounds already; only the cash of
//! pops and factories is stored x FLD_POP_MONEY_EFFECT_SCALE, and nothing shown here reads those.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::{Duration, Instant};

use wiktoria_data::date::Date;
use wiktoria_data::localisation::Localisation;
use wiktoria_data::map::ProvinceMap;
use wiktoria_render::layers::{Glow, Ground, GroundSource, Layers};
use wiktoria_render::{MapMode, Palette};
use wiktoria_simulation::game::{DayLog, Game};
use wiktoria_simulation::player::{Order, Slider};
use wiktoria_simulation::script::{PopState, State};

use crate::{config, load_world, root, Args};

mod diplomacy;
mod military;
mod play;
mod population;
mod politics;
mod trade;
mod production;

/// Days per second of wall time for speeds 1..=5 (5 = as fast as it can).
const SPEED_DPS: [f64; 5] = [1.0, 2.0, 5.0, 15.0, f64::INFINITY];
const NEWS_KEPT: usize = 400;
const NEWS_SHOWN: usize = 30;
const PICTURES_KEPT: usize = 6;
/// The simulation thread makes the watched screen ready at most this often, and only while a
/// page has asked within `SCREEN_WATCH`.
const SCREEN_EVERY: Duration = Duration::from_millis(200);
const SCREEN_WATCH: Duration = Duration::from_secs(3);
const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 1024 * 1024;

// ---------------------------------------------------------------------------------------------
// Pure helpers: JSON, query strings, paths.

/// A JSON string with its quotes.
pub(crate) fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSON number: `null` for NaN and infinities, whole numbers as `12.0`, others with at most
/// four decimals.
pub(crate) fn json_num(v: f64) -> String {
    if !v.is_finite() {
        return "null".into();
    }
    let r = if v.abs() < 1e11 { (v * 1e4).round() / 1e4 } else { v.round() } + 0.0;
    if r == r.trunc() {
        return format!("{r:.1}");
    }
    let s = format!("{r:.4}");
    s.trim_end_matches('0').to_string()
}

/// A whole number without decimals (people, men, days); `null` for NaN and infinities.
pub(crate) fn json_int(v: f64) -> String {
    if v.is_finite() { format!("{:.0}", v.round() + 0.0) } else { "null".into() }
}

fn json_opt(v: Option<f64>) -> String {
    v.map_or("null".into(), json_num)
}

fn json_opt_str(v: Option<&str>) -> String {
    v.map_or("null".into(), json_str)
}

fn json_list<S: AsRef<str>>(items: impl IntoIterator<Item = S>) -> String {
    format!("[{}]", items.into_iter().map(|s| json_str(s.as_ref())).collect::<Vec<_>>().join(","))
}

/// `[["farmers",0.62],["artisans",0.18]]`.
fn json_pairs(items: &[(String, f64)]) -> String {
    format!("[{}]", items.iter().map(|(n, v)| format!("[{},{}]", json_str(n), json_num(*v))).collect::<Vec<_>>().join(","))
}

/// `%41` -> `A`, `+` -> space when `plus` (query strings only). Bad escapes are kept as written.
pub(crate) fn percent_decode(s: &str, plus: bool) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() && hex(b[i + 1]).is_some() && hex(b[i + 2]).is_some() => {
                out.push(hex(b[i + 1]).unwrap_or(0) * 16 + hex(b[i + 2]).unwrap_or(0));
                i += 3;
                continue;
            }
            b'+' if plus => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `/api/screen?province=300` -> (`/api/screen`, `province=300`).
pub(crate) fn split_target(target: &str) -> (&str, &str) {
    let target = target.split('#').next().unwrap_or("");
    target.split_once('?').unwrap_or((target, ""))
}

/// `a=1&b=x%20y` -> [(a, 1), (b, "x y")]. A key without `=` gets an empty value.
pub(crate) fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k, true), percent_decode(v, true))
        })
        .collect()
}

fn param<'a>(q: &'a [(String, String)], key: &str) -> Option<&'a str> {
    q.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// The file under the web folder a URL path names, as a relative path; `None` for anything that
/// could leave the folder: `..`, a drive letter or other colon, a backslash, an empty part
/// (`//`, also an absolute path after decoding), a part ending in a dot or a space.
pub(crate) fn safe_path(url_path: &str) -> Option<PathBuf> {
    let decoded = percent_decode(url_path, false);
    let rest = decoded.strip_prefix('/')?;
    let rest = if rest.is_empty() { "index.html" } else { rest };
    if rest.chars().any(|c| c == '\\' || c == ':' || (c as u32) < 0x20) {
        return None;
    }
    let mut out = PathBuf::new();
    for part in rest.split('/') {
        if part.is_empty() || part.starts_with("..") || part == "." || part.ends_with('.') || part.ends_with(' ') || part.starts_with(' ') {
            return None;
        }
        out.push(part);
    }
    Some(out)
}

pub(crate) fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("bmp") => "image/bmp",
        Some("md") => "text/markdown; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// `steel_factory` -> `steel factory`: what the other commands print for a key without a name.
fn plain(key: &str) -> String {
    key.replace('_', " ")
}

// ---------------------------------------------------------------------------------------------
// What the threads share.

struct News {
    date: Date,
    /// `None` = news of the world, shown to every seat; otherwise only to that country.
    tag: Option<String>,
    text: String,
}

/// Per-day sums that cost a pass over the world: made once on the first request of a day.
#[derive(Default)]
struct Daily {
    date: Option<Date>,
    admin: BTreeMap<String, f64>,
    /// Province -> (good, units a day) of its farm or mine.
    output: HashMap<u16, (String, f64)>,
    /// Owner -> good -> units a day of its farms and mines.
    made: BTreeMap<String, BTreeMap<String, f64>>,
}

/// The last day's money, per country.
#[derive(Default)]
struct Yesterday {
    /// The market's report: taxes and duties less salaries, goods and subsidies.
    report: BTreeMap<String, f64>,
    /// What the treasury really changed by over the day.
    treasury: BTreeMap<String, f64>,
}

/// Map mode, width, map version, layer bits, glowing country.
type PictureKey = (&'static str, usize, u64, u8, String);

/// What the static layers of the map picture are built from (read again at the first picture).
struct GroundInputs {
    terrain: Option<PathBuf>,
    rivers: Option<PathBuf>,
    terrain_colors: [Option<u32>; 256],
    river_max: u8,
    is_sea: Vec<bool>,
    /// The folder of the install's map colour pictures (next to terrain.bmp).
    textures: Option<PathBuf>,
}

/// The install's map colour pictures, in its map folder's `terrain` folder: land, sea, waves and the
/// paper the political map is drawn on. Which pictures to use is our presentation choice.
const LAND_COLORS: &str = "colormap.dds";
const SEA_COLORS: &str = "colormap_water.dds";
const WAVES: &str = "water.dds";
const PAPER: &str = "map_overlay_tile.dds";

/// The paper the political map is drawn on, as grey around a middle of 128 (so the page can lay it
/// over the map picture with an overlay blend: 128 leaves the map as it is, darker lines darken it).
fn paper_bmp(dir: &Option<PathBuf>) -> Option<Vec<u8>> {
    let img = texture(dir, PAPER)?;
    let grey: Vec<f64> = img.rgb.chunks_exact(3).map(|p| p.iter().map(|&c| f64::from(c)).sum::<f64>() / 3.0).collect();
    let mean = (grey.iter().sum::<f64>() / grey.len().max(1) as f64).max(1.0);
    let rgb = grey.iter().flat_map(|g| [(g / mean * 128.0).round().clamp(0.0, 255.0) as u8; 3]).collect();
    Some(wiktoria_data::bmp::encode(&wiktoria_data::bmp::Image { width: img.width, height: img.height, rgb }))
}

/// One colour picture, north at the top; None (and a note) when it cannot be read.
fn texture(dir: &Option<PathBuf>, name: &str) -> Option<wiktoria_data::bmp::Image> {
    let p = dir.as_ref()?.join(name);
    let got = std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| wiktoria_data::dds::decode(&b));
    match got {
        Ok(mut img) => {
            img.flip_vertical();
            Some(img)
        }
        Err(e) => {
            eprintln!("map picture: {} left out: {e}", p.display());
            None
        }
    }
}

impl GroundInputs {
    fn new(w: &wiktoria_data::world::World, engine_rules: &wiktoria_data::engine_rules::EngineRules) -> Self {
        let mut terrain_colors = [None; 256];
        for (i, cat) in &w.defs.terrain.index_category {
            terrain_colors[*i as usize] = w.defs.terrain.categories.get(cat).and_then(|c| c.color).map(wiktoria_render::rgb);
        }
        GroundInputs {
            terrain: w.map_pictures.terrain.clone(),
            rivers: w.map_pictures.rivers.clone(),
            terrain_colors,
            river_max: engine_rules.field("FLD_MAP_RIVER_INDEX_MAX").and_then(|v| v.as_f64()).map_or(0, |m| m.clamp(0.0, 255.0) as u8),
            is_sea: w.provinces.iter().map(|p| p.as_ref().is_some_and(|p| p.is_sea)).collect(),
            textures: w.map_pictures.terrain.as_ref().and_then(|t| t.parent()).map(|d| d.join("terrain")),
        }
    }

    /// The static layers, with the time each step took printed. A picture that cannot be read
    /// leaves its layer out (and says so).
    fn build(&self, map: &ProvinceMap) -> Ground {
        let t = Instant::now();
        let read = |p: &Option<PathBuf>| -> Option<(wiktoria_data::bmp::IndexedImage, Vec<[u8; 3]>)> {
            let p = p.as_ref()?;
            let got = std::fs::read(p).map_err(|e| e.to_string()).and_then(|b| {
                let mut img = wiktoria_data::bmp::decode_indexed(&b)?;
                img.flip_vertical();
                Ok((img, wiktoria_data::bmp::palette(&b)?))
            });
            got.map_err(|e| eprintln!("map picture: {} left out: {e}", p.display())).ok()
        };
        let (terrain, rivers) = (read(&self.terrain), read(&self.rivers));
        eprintln!("map picture: terrain and river pictures read in {:.2?}", t.elapsed());
        let (colormap, water_colormap, water_tile) = (texture(&self.textures, LAND_COLORS), texture(&self.textures, SEA_COLORS), texture(&self.textures, WAVES));
        let wide = rivers.as_ref().map_or([false; 256], |(_, pal)| wiktoria_render::layers::wide_river_codes(pal, self.river_max));
        let src = GroundSource {
            map,
            is_sea: &self.is_sea,
            terrain: terrain.as_ref().map(|x| &x.0),
            terrain_colors: &self.terrain_colors,
            rivers: rivers.as_ref().map(|x| &x.0),
            river_max: self.river_max,
            wide: &wide,
            colormap: colormap.as_ref(),
            water_colormap: water_colormap.as_ref(),
            water_tile: water_tile.as_ref(),
        };
        let (ground, times) = Ground::build(&src);
        for (name, d) in times {
            eprintln!("map picture: {name} layer built in {d:.2?}");
        }
        eprintln!("map picture: static layers ready in {:.2?}", t.elapsed());
        ground
    }
}

/// The state part of a screen answer, kept for the next request of the same seat and province.
struct Kept {
    seat: String,
    province: Option<u16>,
    /// `Shared::days` when it was made.
    days: u64,
    body: String,
}

struct Shared {
    /// The country names on the map, with the map version they were laid out for.
    labels: Mutex<Option<(u64, Response)>>,
    game: Mutex<Game>,
    /// Requests waiting for the game: the simulation does not start a day while there are any.
    waiting: AtomicUsize,
    paused: AtomicBool,
    speed: AtomicU8,
    days: AtomicU64,
    /// f64 bits.
    days_per_second: AtomicU64,
    map_version: AtomicU64,
    date: Mutex<Date>,
    seat: Mutex<String>,
    news: Mutex<VecDeque<News>>,
    yesterday: Mutex<Yesterday>,
    daily: Mutex<Daily>,
    /// The seat and province the page last asked for, and when.
    watched: Mutex<Option<(String, Option<u16>, Instant)>>,
    screen: Mutex<Option<Kept>>,
    /// A copy of the province map, so the picture is drawn without holding the game.
    map: ProvinceMap,
    pictures: Mutex<VecDeque<(PictureKey, Arc<Vec<u8>>)>>,
    ground_inputs: GroundInputs,
    /// Terrain, water, rivers and province edges: built at the first picture, kept for the game.
    ground: OnceLock<Ground>,
    /// The owner, border and glow layers print their times once.
    composed_once: AtomicBool,
    loc: Localisation,
    /// Party keys in the order a save numbers them.
    parties: Vec<String>,
    web: PathBuf,
    install: PathBuf,
    mods: Vec<String>,
    /// Rank order and (industrial, military, prestige) scores worked out at the start: the state
    /// has yesterday's, which a save does not carry, so these stand in until the first day ran.
    first_order: Vec<String>,
    first_scores: BTreeMap<String, [f64; 3]>,
    /// The test scenario being played (`--scenario`) and its checks so far.
    scenario: Mutex<Option<(wiktoria_simulation::scenario::Scenario, wiktoria_simulation::scenario::Run)>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    /// The game, for a request: announces itself first so the simulation lets it in.
    fn game(&self) -> MutexGuard<'_, Game> {
        self.waiting.fetch_add(1, Ordering::SeqCst);
        let g = lock(&self.game);
        self.waiting.fetch_sub(1, Ordering::SeqCst);
        g
    }

    /// The display name of a key, or the key with spaces.
    fn name(&self, key: &str) -> String {
        self.loc.get(key).filter(|s| !s.is_empty()).map_or_else(|| plain(key), str::to_string)
    }

    fn clock_json(&self) -> String {
        let paused = self.paused.load(Ordering::Relaxed);
        let dps = if paused { 0.0 } else { f64::from_bits(self.days_per_second.load(Ordering::Relaxed)) };
        format!(
            "{{\"date\":{},\"day\":{},\"paused\":{},\"speed\":{},\"days_per_second\":{}}}",
            json_str(&lock(&self.date).to_string()),
            self.days.load(Ordering::SeqCst),
            paused,
            self.speed.load(Ordering::Relaxed),
            json_num(dps)
        )
    }
}

fn country_name(g: &Game, tag: &str) -> String {
    g.world.tag_index.get(tag).map_or_else(|| tag.to_string(), |i| g.world.country(*i).name.clone())
}

fn province_name(g: &Game, id: u16) -> String {
    if let Some(n) = g.state.provinces.get(&id).and_then(|p| p.name_override.clone()) {
        return n;
    }
    g.world.province(id).map_or_else(|| format!("#{id}"), |p| p.name.clone())
}

fn region_name(sh: &Shared, g: &Game, region: Option<u16>) -> Option<String> {
    let r = region?;
    if let Some(n) = g.state.region_names.get(&r) {
        return Some(n.clone());
    }
    g.world.regions.get(r as usize).map(|x| sh.name(&x.name))
}

/// FNV-1a over every province's owner and controller.
fn map_key(s: &State) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut mix = |t: &Option<String>| {
        for b in t.as_deref().unwrap_or("-").bytes().chain([0xff]) {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
    };
    for p in s.provinces.values() {
        mix(&p.owner);
        mix(&p.controller);
    }
    h
}

// ---------------------------------------------------------------------------------------------
// The simulation thread.

fn news_of_day(g: &Game, log: &DayLog, great_before: &[String], sh: &Shared) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    let party = |n: u32| (n as usize).checked_sub(1).and_then(|i| sh.parties.get(i)).map_or_else(|| format!("party {n}"), |k| sh.name(k));
    for w in &log.core.wars_declared {
        out.push((None, format!("War declared: {w}")));
    }
    for w in &log.core.peaces {
        out.push((None, format!("Peace: {w}")));
    }
    for t in g.state.great_powers.iter().filter(|t| !great_before.contains(t)) {
        out.push((None, format!("{} became a great power", country_name(g, t))));
    }
    for t in great_before.iter().filter(|t| !g.state.great_powers.contains(t)) {
        out.push((None, format!("{} is no longer a great power", country_name(g, t))));
    }
    for t in &log.core.mobilised {
        out.push((Some(t.clone()), format!("{} mobilised", country_name(g, t))));
    }
    for (t, old, new) in &log.elections {
        let text = if *old == Some(*new) { format!("Election: {} stays in power", party(*new)) } else { format!("Election won by {}", party(*new)) };
        out.push((Some(t.clone()), text));
    }
    for (t, group, option) in &log.reforms {
        out.push((Some(t.clone()), format!("Reform passed: {} ({})", sh.name(option), sh.name(group))));
    }
    for (t, tech) in &log.researched {
        out.push((Some(t.clone()), format!("Researched {}", sh.name(tech))));
    }
    for (t, kind) in log.built.started_by_pops.iter().chain(&log.built.started_by_states) {
        out.push((Some(t.clone()), format!("Work began: {}", sh.name(kind))));
    }
    out
}

fn sim_thread(sh: &Shared) {
    let mut key = map_key(&lock(&sh.game).state);
    let mut done: VecDeque<Instant> = VecDeque::new();
    let mut screen_made = Instant::now();
    loop {
        if sh.paused.load(Ordering::Relaxed) {
            done.clear();
            sh.days_per_second.store(0f64.to_bits(), Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(15));
            continue;
        }
        let start = Instant::now();
        // Requests first: the lock is not fair, so a day does not start while one is waiting.
        while sh.waiting.load(Ordering::SeqCst) > 0 {
            std::thread::sleep(Duration::from_micros(200));
        }
        {
            let mut g = lock(&sh.game);
            let great: Vec<String> = g.state.great_powers.clone();
            let before: Vec<(String, f64)> = g.state.countries.iter().map(|(t, c)| (t.clone(), c.money)).collect();
            let log = g.day();
            trade::after_day(&g, &log);
            let today = g.state.date;
            *lock(&sh.date) = today;
            {
                let mut y = lock(&sh.yesterday);
                y.treasury = before.into_iter().filter_map(|(t, m)| Some((g.state.countries.get(&t)?.money - m, t))).map(|(change, t)| (t, change)).collect();
                y.report = log.money.balance.clone();
            }
            let mut fresh = news_of_day(&g, &log, &great, sh);
            fresh.extend(crate::scenario::serve_day(&mut lock(&sh.scenario), &g, &log).into_iter().map(|text| (None, text)));
            if !fresh.is_empty() {
                let mut news = lock(&sh.news);
                news.extend(fresh.into_iter().map(|(tag, text)| News { date: today, tag, text }));
                while news.len() > NEWS_KEPT {
                    news.pop_front();
                }
            }
            let now = map_key(&g.state);
            if now != key {
                key = now;
                sh.map_version.fetch_add(1, Ordering::SeqCst);
            }
            sh.days.fetch_add(1, Ordering::SeqCst);
            // While a page is asking, its screen is made ready here, so it need not wait for a day.
            let watched = lock(&sh.watched).clone().filter(|w| w.2.elapsed() < SCREEN_WATCH);
            if let Some((seat, province, _)) = watched.filter(|_| screen_made.elapsed() >= SCREEN_EVERY) {
                let body = screen_body(sh, &g, &seat, province);
                keep_screen(sh, &seat, province, &body);
                screen_made = Instant::now();
            }
        }
        let took = start.elapsed();
        done.push_back(Instant::now());
        while done.len() > 2 && done.front().is_some_and(|t| t.elapsed() > Duration::from_secs(4)) {
            done.pop_front();
        }
        let dps = match (done.front(), done.back()) {
            (Some(a), Some(b)) if done.len() >= 2 && *b > *a => (done.len() - 1) as f64 / (*b - *a).as_secs_f64(),
            _ => 1.0 / took.as_secs_f64().max(1e-6),
        };
        sh.days_per_second.store(dps.to_bits(), Ordering::Relaxed);
        let speed = sh.speed.load(Ordering::Relaxed).clamp(1, 5);
        let target = SPEED_DPS[speed as usize - 1];
        if target.is_finite() {
            let want = Duration::from_secs_f64(1.0 / target);
            // Small steps, so a pause or a new speed acts at once.
            while start.elapsed() < want && !sh.paused.load(Ordering::Relaxed) && sh.speed.load(Ordering::Relaxed) == speed {
                std::thread::sleep(Duration::from_millis(10).min(want.saturating_sub(start.elapsed())));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Answers.

fn rank_order<'a>(sh: &'a Shared, s: &'a State) -> &'a [String] {
    if s.rank_order.is_empty() { &sh.first_order } else { &s.rank_order }
}

fn rank_of(sh: &Shared, s: &State, tag: &str) -> Option<usize> {
    rank_order(sh, s).iter().position(|t| t == tag).map(|i| i + 1)
}

fn countries_json(sh: &Shared, g: &Game) -> String {
    let s = &g.state;
    let rebels = &g.core.day.rebel_tag;
    let row = |t: &String, rank: Option<usize>| {
        format!(
            "{{\"tag\":{},\"name\":{},\"rank\":{},\"great_power\":{},\"exists\":{}}}",
            json_str(t),
            json_str(&country_name(g, t)),
            rank.map_or("null".into(), |r| r.to_string()),
            s.great_powers.contains(t),
            rank.is_some()
        )
    };
    let order = rank_order(sh, s);
    let mut rows: Vec<String> = order.iter().enumerate().filter(|(_, t)| s.countries.contains_key(*t)).map(|(i, t)| row(t, Some(i + 1))).collect();
    let ranked: BTreeSet<&String> = order.iter().collect();
    rows.extend(s.countries.keys().filter(|t| !ranked.contains(t) && *t != rebels).map(|t| row(t, None)));
    format!("[{}]", rows.join(","))
}

/// The day's world-wide sums, made on the first request of a day.
fn refresh_daily<'a>(sh: &'a Shared, g: &Game) -> MutexGuard<'a, Daily> {
    let mut d = lock(&sh.daily);
    if d.date != Some(g.state.date) {
        d.date = Some(g.state.date);
        d.admin = wiktoria_simulation::pops::admin_efficiency(&g.state, &g.econ.mobility_rules);
        d.output.clear();
        d.made.clear();
        for (id, owner, good, units) in g.econ.farms.province_output(&g.world, &g.state) {
            *d.made.entry(owner).or_default().entry(good.clone()).or_default() += units;
            d.output.insert(id, (good, units));
        }
    }
    d
}

/// Shares by a key of the pops, largest first.
fn shares(pops: &[PopState], total: f64, key: impl Fn(&PopState) -> &str) -> Vec<(String, f64)> {
    let mut acc: BTreeMap<&str, f64> = BTreeMap::new();
    for q in pops {
        *acc.entry(key(q)).or_default() += q.size;
    }
    let mut v: Vec<(String, f64)> = acc.into_iter().map(|(k, x)| (k.to_string(), if total > 0.0 { x / total } else { 0.0 })).collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v
}

fn province_json(sh: &Shared, g: &Game, daily: &Daily, id: u16) -> String {
    let (w, s) = (&g.world, &g.state);
    let Some(wp) = w.province(id) else { return "null".into() };
    let name = json_str(&province_name(g, id));
    let Some(p) = s.provinces.get(&id).filter(|_| !wp.is_sea) else {
        return format!(
            "{{\"id\":{id},\"name\":{name},\"sea\":{},\"state\":null,\"owner\":null,\"controller\":null,\"terrain\":{},\"life_rating\":null,\"population\":null,\"growth\":null,\"goods\":null,\"output\":null,\"income\":null,\"workforce\":[],\"ideology\":[],\"culture\":[],\"fort\":null,\"naval_base\":null,\"railroad\":null,\"cores\":[],\"supply_limit\":null,\"admin_efficiency\":null}}",
            wp.is_sea,
            json_opt_str(wp.terrain.as_deref())
        );
    };
    let total: f64 = p.pops.iter().map(|q| q.size).sum();
    let mut ideo: BTreeMap<u32, f64> = BTreeMap::new();
    for q in &p.pops {
        for (k, v) in &q.ideology {
            *ideo.entry(*k).or_default() += v / 100.0 * q.size;
        }
    }
    let mut ideology: Vec<(String, f64)> = ideo.into_iter().filter_map(|(k, v)| Some((s.ideologies.get((k as usize).checked_sub(1)?)?.clone(), if total > 0.0 { v / total } else { 0.0 }))).collect();
    ideology.sort_by(|a, b| b.1.total_cmp(&a.1));
    let level = |n: &str| p.buildings.get(n).copied().unwrap_or(0.0).max(0.0).round() as u32;
    let made = daily.output.get(&id);
    let output = made.map(|x| x.1);
    let income = made.and_then(|(good, units)| Some(units * s.prices.get(good)?));
    format!(
        "{{\"id\":{id},\"name\":{name},\"sea\":false,\"state\":{},\"owner\":{},\"controller\":{},\"terrain\":{},\"life_rating\":{},\"population\":{},\"growth\":null,\"goods\":{},\"output\":{},\"income\":{},\"workforce\":{},\"ideology\":{},\"culture\":{},\"fort\":{},\"naval_base\":{},\"railroad\":{},\"cores\":{},\"supply_limit\":null,\"admin_efficiency\":{}}}",
        json_opt_str(region_name(sh, g, p.region).as_deref()),
        json_opt_str(p.owner.as_deref()),
        json_opt_str(p.controller.as_deref()),
        json_opt_str(wp.terrain.as_deref()),
        json_num(p.life_rating),
        json_int(total),
        json_opt_str(p.trade_goods.as_deref()),
        json_opt(output),
        json_opt(income),
        json_pairs(&shares(&p.pops, total, |q| &q.pop_type)),
        json_pairs(&ideology),
        json_pairs(&shares(&p.pops, total, |q| &q.culture)),
        level("fort"),
        level("naval_base"),
        level("railroad"),
        json_list(&p.cores),
        json_opt(p.owner.as_ref().and_then(|o| daily.admin.get(o)).copied()),
    )
}

/// `{"rate":..,"<what>":..}`.
fn rate_json(rate: Option<f64>, what: &str, amount: Option<f64>) -> String {
    format!("{{\"rate\":{},\"{what}\":{}}}", json_opt(rate), json_opt(amount))
}

/// `{"poor":[0.1,0.9],...}`: the lowest and highest value of each budget slider for this country.
fn limits_json(g: &Game, tag: &str) -> String {
    let rows: Vec<String> = Slider::ALL
        .iter()
        .map(|s| {
            let (lo, hi) = wiktoria_simulation::player::slider_limits(&g.state, &g.econ.politics_rules, tag, *s);
            format!("{}:[{},{}]", json_str(s.name()), json_num(lo), json_num(hi))
        })
        .collect();
    format!("{{{}}}", rows.join(","))
}

/// What the seat may research (`GET /api/research`).
fn research_json(sh: &Shared, g: &Game, tag: &str) -> String {
    let (s, rules) = (&g.state, &g.econ.research_rules);
    let Some(c) = s.countries.get(tag) else { return "null".into() };
    let lib = &g.world.defs.modifiers;
    let bonus = |folder: &str| c.tech_school.as_ref().and_then(|x| lib.schools.get(x)).and_then(|v| v.get(&format!("{folder}_research_bonus"))).copied().unwrap_or(0.0);
    let cost = |t: &wiktoria_simulation::research::TechInfo| rules.cost(t, s.date.year, bonus(&t.folder));
    let current = c.research.as_ref().map_or("null".into(), |(name, points)| {
        let tech = rules.techs.iter().find(|t| t.name == *name);
        format!("{{\"key\":{},\"name\":{},\"progress\":{}}}", json_str(name), json_str(&sh.name(name)), json_opt(tech.map(|t| (points / cost(t)).clamp(0.0, 1.0))))
    });
    let options: Vec<String> = wiktoria_simulation::player::open_technologies(s, rules, tag)
        .into_iter()
        .map(|t| format!("{{\"key\":{},\"name\":{},\"folder\":{},\"folder_name\":{},\"year\":{},\"cost\":{}}}", json_str(&t.name), json_str(&sh.name(&t.name)), json_str(&t.folder), json_str(&sh.name(&t.folder)), t.year, json_num(cost(t))))
        .collect();
    format!(
        "{{\"tag\":{},\"played\":{},\"civilized\":{},\"current\":{current},\"points_stored\":{},\"points_a_day\":{},\"options\":[{}]}}",
        json_str(tag),
        c.human,
        c.civilized,
        json_num(c.research_points),
        json_num(c.research_rate),
        options.join(",")
    )
}

/// The order a POST to an order endpoint gives (`/api/budget`, `/api/research`, `/api/army/move`).
pub(crate) fn order_of(path: &str, q: &[(String, String)]) -> Result<Order, String> {
    let need = |k: &str| param(q, k).map(str::trim).filter(|v| !v.is_empty()).ok_or(format!("{k} missing"));
    match path {
        "/api/budget" => {
            let name = need("slider")?;
            let slider = Slider::parse(name).ok_or(format!("slider must be one of {}", Slider::ALL.map(|s| s.name()).join(", ")))?;
            let value = need("value")?.parse::<f64>().ok().filter(|v| v.is_finite()).ok_or("value must be a number")?;
            Ok(Order::Budget { slider, value })
        }
        "/api/research" => Ok(Order::Research { tech: need("tech")?.to_string() }),
        "/api/army/move" => {
            let army = need("army")?.parse::<u64>().map_err(|_| "army must be a number")?;
            let to = need("to")?.parse::<u16>().map_err(|_| "to must be a province number")?;
            Ok(Order::MoveArmy { army, to })
        }
        _ => play::order_of(path, q).unwrap_or(Err("not an order".into())),
    }
}

/// Hands an order of the seat's player to the game's queue and has it carried out at once:
/// between two days, which is the same as at the start of the next one.
fn give_order(sh: &Shared, order: Order) -> Response {
    let seat = lock(&sh.seat).clone();
    let mut g = sh.game();
    if !wiktoria_simulation::player::is_player(&g.state, &seat) {
        return error(409, "the seat is observed: POST /api/play?tag=... first");
    }
    g.players.push(&seat, order);
    let out = g.apply_orders();
    drop(g);
    *lock(&sh.screen) = None;
    match out.into_iter().rev().find(|o| o.tag == seat) {
        Some(o) => match o.result {
            Ok(text) => {
                if play::logged(&o.order) {
                    let date = *lock(&sh.date);
                    lock(&sh.news).push_back(News { date, tag: Some(seat.clone()), text: text.clone() });
                }
                let value = match o.order {
                    Order::Budget { slider, .. } => sh.game().state.countries.get(&seat).and_then(|c| c.spending.get(slider.key()).copied()),
                    _ => None,
                };
                json(format!("{{\"ok\":true,\"text\":{},\"value\":{}}}", json_str(&text), json_opt(value)))
            }
            Err(e) => error(400, &e),
        },
        None => error(400, "the order was not carried out"),
    }
}

/// Takes control of `tag` (and gives every other played country back to the computer), or with
/// `None` observes. Returns the answer of `/api/play` and `/api/observe`.
fn set_play(sh: &Shared, tag: Option<&str>) -> Response {
    let mut g = sh.game();
    if let Some(t) = tag
        && (!g.state.countries.contains_key(t) || t == g.core.day.rebel_tag)
    {
        return error(400, "unknown country");
    }
    let mut released = Vec::new();
    for t in wiktoria_simulation::player::players(&g.state) {
        if Some(t.as_str()) != tag {
            let _ = g.set_player(&t, false);
            released.push(t);
        }
    }
    if let Some(t) = tag {
        if let Err(e) = g.set_player(t, true) {
            return error(400, &e);
        }
        *lock(&sh.seat) = t.to_string();
    }
    let date = g.state.date;
    let names: Vec<String> = released.iter().map(|t| country_name(&g, t)).collect();
    let now = wiktoria_simulation::player::players(&g.state);
    let seat = lock(&sh.seat).clone();
    let name = country_name(&g, &seat);
    drop(g);
    *lock(&sh.screen) = None;
    {
        let mut news = lock(&sh.news);
        for (t, n) in released.iter().zip(&names) {
            news.push_back(News { date, tag: Some(t.clone()), text: format!("The computer plays {n} again") });
        }
        if let Some(t) = tag {
            news.push_back(News { date, tag: Some(t.to_string()), text: format!("You play {name}") });
        }
    }
    json(format!("{{\"tag\":{},\"name\":{},\"played\":{},\"players\":{},\"released\":{}}}", json_str(&seat), json_str(&name), now.contains(&seat), json_list(&now), json_list(&released)))
}

/// The `country`, `budget` and `outliner` members for the seat.
fn seat_json(sh: &Shared, g: &Game, daily: &Daily, tag: &str) -> Option<String> {
    let (w, s, mil) = (&g.world, &g.state, &g.core.mil);
    let c = s.countries.get(tag)?;
    let lib = &w.defs.modifiers;
    // One pass over the country's provinces.
    let soldier_types = &g.econ.rank_rules.soldier_types;
    let (mut people, mut literate, mut militancy, mut consciousness, mut soldiers) = (0.0, 0.0, 0.0, 0.0, 0.0);
    let (mut occupied, mut provinces) = (0usize, 0usize);
    for p in s.provinces.values().filter(|p| p.owner.as_deref() == Some(tag)) {
        provinces += 1;
        if p.controller != p.owner {
            occupied += 1;
        }
        for q in &p.pops {
            people += q.size;
            literate += q.literacy * q.size;
            militancy += q.militancy * q.size;
            consciousness += q.consciousness * q.size;
            if soldier_types.contains(&q.pop_type) {
                soldiers += q.size;
            }
        }
    }
    let avg = |x: f64| if people > 0.0 { Some(x / people) } else { None };
    let rank = rank_of(sh, s, tag);
    let military = if s.rank_order.is_empty() { sh.first_scores.get(tag).map_or(c.military_score, |x| x[1]) } else { c.military_score };
    let y = lock(&sh.yesterday);
    let (report, treasury) = (y.report.get(tag).copied(), y.treasury.get(tag).copied());
    drop(y);
    // Research.
    let research = match &c.research {
        Some((name, points)) => {
            let progress = g.econ.research_rules.techs.iter().find(|t| t.name == *name).map(|t| {
                let bonus = c.tech_school.as_ref().and_then(|x| lib.schools.get(x)).and_then(|v| v.get(&format!("{}_research_bonus", t.folder))).copied().unwrap_or(0.0);
                (points / g.econ.research_rules.cost(t, s.date.year, bonus)).clamp(0.0, 1.0)
            });
            format!("{{\"name\":{},\"progress\":{},\"points_a_day\":{}}}", json_str(&sh.name(name)), json_opt(progress), json_num(c.research_rate))
        }
        None => format!("{{\"name\":null,\"progress\":null,\"points_a_day\":{}}}", json_num(c.research_rate)),
    };
    let party = c.ruling_party.and_then(|n| sh.parties.get((n as usize).checked_sub(1)?)).map(|k| sh.name(k));
    let government = c.government.as_deref().map(|k| sh.name(k));
    let dip = g.core.dip.as_ref().map(|(_, d)| d.points(tag));
    let leadership = mil.countries.get(tag).map_or(c.leadership, |m| m.leadership);
    // Goods: what its farms, mines and factories make (by value), and what it mostly buys abroad.
    let price = |good: &str| s.prices.get(good).copied().unwrap_or(0.0);
    let mut value: BTreeMap<&str, f64> = BTreeMap::new();
    for (good, units) in daily.made.get(tag).into_iter().flatten() {
        *value.entry(good.as_str()).or_default() += units * price(good);
    }
    let mut building = Vec::new();
    for f in s.factories.iter().filter(|f| f.owner == tag) {
        if let Some(days) = f.building_days {
            building.push(format!("{{\"kind\":{},\"state\":{},\"days\":{}}}", json_str(&f.kind), json_opt_str(region_name(sh, g, f.region).as_deref()), json_int(days.max(0.0).ceil())));
        }
        if let Some((good, _)) = s.factory_types.get(&f.kind).filter(|_| f.level >= 1.0 && !f.closed) {
            *value.entry(good.as_str()).or_default() += f.income.max(0.0);
        }
    }
    let mut produces: Vec<(&str, f64)> = value.into_iter().filter(|(_, v)| *v > 0.0).collect();
    produces.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut imports: Vec<(&String, f64)> = s.home_market.get(tag).into_iter().flatten().filter(|(_, row)| row[2] > 0.5).map(|(good, row)| (good, row[2])).collect();
    imports.sort_by(|a, b| b.1.total_cmp(&a.1));
    // Wars and alerts.
    let wars: Vec<String> = mil
        .active_wars_of(tag)
        .map(|war| {
            let against = if war.attackers.iter().any(|t| t == tag) { &war.defenders } else { &war.attackers };
            format!("{{\"name\":{},\"against\":{}}}", json_str(&war.name), json_list(against))
        })
        .collect();
    let money = s.money.countries.get(tag);
    let debt: f64 = money.map_or(0.0, |m| m.loans.iter().map(|l| l.debt).sum());
    let loan_modifier = wiktoria_simulation::pops::country_modifier(c, lib, "loan_interest");
    let interest: f64 = money.map_or(0.0, |m| m.loans.iter().map(|l| wiktoria_simulation::economy::loan_interest(&s.economy_numbers, l.debt, l.rate, loan_modifier, l.lender == tag)).sum());
    let rebels = &g.core.day.rebel_tag;
    let mut alerts: Vec<&str> = Vec::new();
    if !building.is_empty() {
        alerts.push("factory_building");
    }
    if !wars.is_empty() {
        alerts.push("at_war");
    }
    if c.civilized && c.research.is_none() {
        alerts.push("no_research");
    }
    if debt > 0.0 {
        alerts.push("in_debt");
    }
    if occupied > 0 {
        alerts.push("occupied");
    }
    if mil.armies_of(rebels).any(|a| s.provinces.get(&a.location).is_some_and(|p| p.owner.as_deref() == Some(tag))) {
        alerts.push("rebels");
    }
    if s.badboy_limit > 0.0 && c.badboy > s.badboy_limit {
        alerts.push("over_infamy");
    }
    if mil.countries.get(tag).is_some_and(|m| m.mobilized) || c.mobilized {
        alerts.push("mobilised");
    }
    let focus_max = g.econ.focus_ai.points.get(tag).copied();
    if focus_max.is_some_and(|m| c.national_focus.len() < m) {
        alerts.push("focus_free");
    }
    let mut out = String::new();
    let _ = write!(
        out,
        "\"country\":{{\"tag\":{},\"name\":{},\"played\":{},\"rank\":{},\"great_power\":{},\"exists\":{},\"prestige\":{},\"industry\":{},\"military\":{},\"money\":{},\"balance\":{},\"research\":{research},\"literacy\":{},\"population\":{},\"militancy\":{},\"consciousness\":{},\"infamy\":{},\"ruling_party\":{},\"government\":{},\"focus_used\":{},\"focus_max\":{},\"diplomatic_points\":{},\"regiments\":{},\"ships\":{},\"manpower\":{},\"leadership\":{},\"produces\":{},\"exports\":null,\"imports\":{},\"alerts\":{},\"waiting\":{{\"events\":{},\"questions\":{}}}}}",
        json_str(tag),
        json_str(&country_name(g, tag)),
        c.human,
        rank.map_or("null".into(), |r| r.to_string()),
        s.great_powers.iter().any(|t| t == tag),
        provinces > 0,
        json_num(c.prestige),
        json_num(s.industrial_score(tag)),
        json_num(military),
        json_num(c.money),
        json_opt(treasury),
        json_opt(avg(literate)),
        json_int(people),
        json_opt(avg(militancy)),
        json_opt(avg(consciousness)),
        json_num(c.badboy),
        json_opt_str(party.as_deref()),
        json_opt_str(government.as_deref()),
        c.national_focus.len(),
        focus_max.map_or("null".into(), |m| m.to_string()),
        json_opt(dip),
        mil.regiment_count(tag),
        mil.ship_count(tag),
        json_int(soldiers),
        json_num(leadership),
        json_list(produces.iter().take(6).map(|x| x.0)),
        json_list(imports.iter().take(6).map(|x| x.0)),
        json_list(alerts),
        g.econ.runner.held.iter().filter(|h| h.tag == tag).count(),
        g.core.asks.of(tag).count(),
    );
    // Budget.
    // A slider the save leaves out stands at nothing: that is how the market reads it.
    let slider = |k: &str| Some(c.spending.get(k).copied().unwrap_or(0.0));
    let sliders = money.map(|m| m.sliders);
    let _ = write!(
        out,
        ",\"budget\":{{\"tax\":{{\"poor\":{},\"middle\":{},\"rich\":{}}},\"gold\":null,\"income_total\":{},\"bank\":{},\"funds\":{},\"debt\":{},\"interest\":{},\"spending\":{{\"education\":{},\"administration\":{},\"social\":{},\"military\":{}}},\"stockpile\":{{\"land\":{},\"naval\":{},\"projects\":{},\"cost\":null}},\"admin_efficiency\":{},\"subsidies\":null,\"expenses_total\":null,\"tariff\":{},\"balance\":{},\"limits\":{}}}",
        rate_json(slider("poor_tax"), "income", None),
        rate_json(slider("middle_tax"), "income", None),
        rate_json(slider("rich_tax"), "income", None),
        json_num(c.tax_income),
        json_opt(money.map(|m| m.bank)),
        json_num(c.money),
        json_num(debt),
        json_num(interest),
        rate_json(slider("education_spending"), "cost", None),
        rate_json(slider("crime_fighting"), "cost", None),
        rate_json(slider("social_spending"), "cost", None),
        rate_json(slider("military_spending"), "cost", None),
        json_opt(sliders.map(|x| x[0])),
        json_opt(sliders.map(|x| x[1])),
        json_opt(sliders.map(|x| x[2])),
        json_opt(daily.admin.get(tag).copied()),
        rate_json(slider("tariffs"), "income", None),
        json_opt(report),
        limits_json(g, tag),
    );
    // Outliner.
    let men = g.core.day.men_per_strength;
    let armies: Vec<String> = mil
        .armies_of(tag)
        .map(|a| {
            let to = a.path.last().map_or("null".into(), |p| p.to_string());
            format!("{{\"id\":{},\"name\":{},\"men\":{},\"province\":{},\"province_id\":{},\"to\":{to}}}", a.id, json_str(&a.name), json_int(a.men(men)), json_str(&province_name(g, a.location)), a.location)
        })
        .collect();
    let navies: Vec<String> = mil.navies_of(tag).map(|n| format!("{{\"name\":{},\"ships\":{}}}", json_str(&n.name), n.ships.len())).collect();
    let _ = write!(out, ",\"outliner\":{{\"armies\":[{}],\"navies\":[{}],\"wars\":[{}],\"factories_building\":[{}]}}", armies.join(","), navies.join(","), wars.join(","), building.join(","));
    Some(out)
}

/// The state part of the screen (`country`, `budget`, `outliner`, `province`) for a seat.
fn screen_body(sh: &Shared, g: &Game, seat: &str, province: Option<u16>) -> String {
    let daily = refresh_daily(sh, g);
    let seat_part = seat_json(sh, g, &daily, seat).unwrap_or_else(|| "\"country\":null,\"budget\":null,\"outliner\":{\"armies\":[],\"navies\":[],\"wars\":[],\"factories_building\":[]}".into());
    let province = province.map_or("null".into(), |id| province_json(sh, g, &daily, id));
    format!("{seat_part},\"province\":{province}")
}

/// Keeps the body for the next request with the same seat and province.
fn keep_screen(sh: &Shared, seat: &str, province: Option<u16>, body: &str) {
    *lock(&sh.screen) = Some(Kept { seat: seat.to_string(), province, days: sh.days.load(Ordering::SeqCst), body: body.to_string() });
}

/// The whole screen. While a day is being worked out the page gets the body the simulation
/// thread kept after an earlier day (at most `SCREEN_EVERY` and one day old) instead of waiting
/// for the day to end; a first question, or one for another seat or province, waits.
fn screen_json(sh: &Shared, province: Option<u16>) -> String {
    let seat = lock(&sh.seat).clone();
    *lock(&sh.watched) = Some((seat.clone(), province, Instant::now()));
    let kept = lock(&sh.screen).as_ref().filter(|k| k.seat == seat && k.province == province).map(|k| (k.days, k.body.clone()));
    let body = match kept {
        Some((days, body)) if days == sh.days.load(Ordering::SeqCst) => body,
        Some((_, body)) => match sh.game.try_lock() {
            Err(TryLockError::WouldBlock) => body,
            free => {
                let g = match free {
                    Ok(g) => g,
                    Err(TryLockError::Poisoned(p)) => p.into_inner(),
                    Err(TryLockError::WouldBlock) => sh.game(),
                };
                let fresh = screen_body(sh, &g, &seat, province);
                keep_screen(sh, &seat, province, &fresh);
                fresh
            }
        },
        None => {
            let g = sh.game();
            let fresh = screen_body(sh, &g, &seat, province);
            keep_screen(sh, &seat, province, &fresh);
            fresh
        }
    };
    let news = lock(&sh.news);
    let log: Vec<String> = news.iter().rev().filter(|n| n.tag.as_ref().is_none_or(|t| *t == seat)).take(NEWS_SHOWN).map(|n| format!("{{\"date\":{},\"text\":{}}}", json_str(&n.date.to_string()), json_str(&n.text))).collect();
    drop(news);
    format!("{{\"clock\":{},{body},\"log\":[{}]}}", sh.clock_json(), log.join(","))
}

fn blend(a: u32, b: u32, t: f64) -> u32 {
    let ch = |s: u32| {
        let (x, y) = ((a >> s & 0xff) as f64, (b >> s & 0xff) as f64);
        ((x + (y - x) * t).round() as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

/// The colours of a map mode from the running state: owners, controllers and pops as they are
/// now. `provinces` and `states` do not change during a game.
fn live_palette(g: &Game, mode: MapMode) -> Palette {
    let (w, s) = (&g.world, &g.state);
    let mut pal = Palette::build(w, mode);
    let idx = |t: &Option<String>| t.as_deref().and_then(|t| w.tag_index.get(t)).copied();
    let color = |i: u16| wiktoria_render::rgb(w.country(i).country.color);
    let people = |id: u16| s.provinces.get(&id).map_or(0.0, |p| p.pops.iter().map(|q| q.size).sum::<f64>());
    let max_pop = if mode == MapMode::Population { s.provinces.keys().map(|id| people(*id)).fold(0.0, f64::max) } else { 0.0 };
    let largest = |pops: &[PopState], key: &dyn Fn(&PopState) -> &str| -> Option<String> {
        let mut acc: BTreeMap<&str, f64> = BTreeMap::new();
        for q in pops {
            *acc.entry(key(q)).or_default() += q.size;
        }
        acc.into_iter().max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(a.0))).map(|x| x.0.to_string())
    };
    for p in w.provinces.iter().flatten() {
        let id = p.id as usize;
        let Some(ps) = s.provinces.get(&p.id).filter(|_| !p.is_sea && id < pal.color.len()) else { continue };
        let (o, c) = (idx(&ps.owner), idx(&ps.controller));
        pal.owner_key[id] = o.map_or(0, |v| v as u32 + 1);
        match mode {
            MapMode::Political => {
                let base = o.map_or(wiktoria_render::UNOWNED, color);
                // Occupied land: half the occupier's colour (rebels and unknown tags: dark).
                pal.color[id] = if ps.owner.is_some() && ps.controller != ps.owner { blend(base, c.map_or(0x202020, color), 0.5) } else { base };
            }
            MapMode::Population => pal.color[id] = wiktoria_render::population_color(people(p.id), max_pop),
            MapMode::Culture => pal.color[id] = largest(&ps.pops, &|q| &q.culture).and_then(|n| w.defs.cultures.index.get(&n)).map_or(wiktoria_render::NO_DATA, |i| wiktoria_render::rgb(w.defs.cultures.cultures[i as usize].color)),
            MapMode::Religion => pal.color[id] = largest(&ps.pops, &|q| &q.religion).and_then(|n| w.defs.religions.index.get(&n)).map_or(wiktoria_render::NO_DATA, |i| wiktoria_render::rgb(w.defs.religions.religions[i as usize].color)),
            MapMode::Provinces | MapMode::States => {}
        }
    }
    pal
}

/// The world as a 24-bit BMP in our layered look, from the cache when owners and controllers have
/// not changed. `tag` is the country that glows (when the glow layer is on).
fn map_bmp(sh: &Shared, mode: MapMode, width: usize, layers: Layers, tag: &str) -> Arc<Vec<u8>> {
    let key_of = |version: u64| (mode.label(), width, version, layers.bits(), if layers.glow { tag.to_string() } else { String::new() });
    let (pal, glow_of, version) = {
        let g = sh.game();
        // Read under the lock, so the picture is never older than the version it is stored for.
        let version = sh.map_version.load(Ordering::SeqCst);
        let key = key_of(version);
        if let Some((_, hit)) = lock(&sh.pictures).iter().find(|(k, _)| *k == key) {
            return hit.clone();
        }
        let glow_of = g.world.tag_index.get(tag).map(|i| (u32::from(*i) + 1, wiktoria_render::rgb(g.world.country(*i).country.color)));
        (live_palette(&g, mode), glow_of, version)
    };
    let map = &sh.map;
    let ground = sh.ground.get_or_init(|| sh.ground_inputs.build(map));
    let t = Instant::now();
    let glow = glow_of.filter(|_| layers.glow).and_then(|(key, color)| Glow::build(map, &pal, key, color, wiktoria_render::layers::GLOW_RADIUS));
    let glow_time = t.elapsed();
    let height = ((width as f64 * map.height as f64 / map.width as f64).round() as usize).max(1);
    let buf = wiktoria_render::layers::compose(ground, map, &pal, layers, glow.as_ref(), width, height);
    if !sh.composed_once.swap(true, Ordering::Relaxed) {
        eprintln!("map picture: glow built in {glow_time:.2?}; owners, rivers, borders and glow composed at {width}x{height} in {:.2?}", t.elapsed() - glow_time);
    }
    let img = wiktoria_data::bmp::Image { width, height, rgb: buf.iter().flat_map(|c| [(c >> 16) as u8, (c >> 8) as u8, *c as u8]).collect() };
    let bytes = Arc::new(wiktoria_data::bmp::encode(&img));
    let mut pictures = lock(&sh.pictures);
    pictures.push_back((key_of(version), bytes.clone()));
    while pictures.len() > PICTURES_KEPT {
        pictures.pop_front();
    }
    bytes
}

/// The country names laid over the political map (`wiktoria_render::labels`), for the current owners.
/// Laid out again only when an owner changed.
fn labels_json(sh: &Shared) -> Response {
    let (version, owner_key, names, capitals) = {
        let g = sh.game();
        let version = sh.map_version.load(Ordering::SeqCst);
        if let Some((_, r)) = lock(&sh.labels).as_ref().filter(|(v, _)| *v == version) {
            return Response { status: r.status, kind: r.kind, body: r.body.clone() };
        }
        let pal = live_palette(&g, MapMode::Political);
        let n = g.world.countries.len();
        let names: Vec<(String, String)> = (0..n).map(|i| {
            let c = g.world.country(i as wiktoria_data::world::CountryIdx);
            (c.country.tag.clone(), c.name.clone())
        }).collect();
        let capitals: Vec<Option<u16>> = names.iter().map(|(t, _)| g.state.countries.get(t).and_then(|c| c.capital)).collect();
        (version, pal.owner_key, names, capitals)
    };
    let is_land: Vec<bool> = sh.ground_inputs.is_sea.iter().map(|s| !s).collect();
    let at = |key: u32| (key as usize).checked_sub(1).filter(|&i| i < names.len());
    let got = wiktoria_render::labels::labels(&sh.map, &is_land, &owner_key, &|k| at(k).map_or_else(String::new, |i| names[i].1.clone()), &|k| at(k).and_then(|i| capitals[i]));
    let r = |x: f32| (x * 10.0).round() / 10.0;
    let body = format!(
        "[{}]",
        got.iter()
            .filter_map(|l| at(l.key).map(|i| (i, l)))
            .map(|(i, l)| format!(
                "{{\"tag\":{},\"text\":{},\"size\":{},\"glyphs\":[{}]}}",
                json_str(&names[i].0),
                json_str(&l.text),
                r(l.size),
                l.glyphs.iter().map(|&(x, y, a)| format!("[{},{},{}]", r(x), r(y), (a * 1000.0).round() / 1000.0)).collect::<Vec<_>>().join(",")
            ))
            .collect::<Vec<_>>()
            .join(",")
    );
    let resp = json(body);
    *lock(&sh.labels) = Some((version, Response { status: resp.status, kind: resp.kind, body: resp.body.clone() }));
    resp
}

// ---------------------------------------------------------------------------------------------
// HTTP.

struct Response {
    status: u16,
    kind: &'static str,
    body: Arc<Vec<u8>>,
}

const JSON: &str = "application/json; charset=utf-8";

fn json(body: String) -> Response {
    Response { status: 200, kind: JSON, body: Arc::new(body.into_bytes()) }
}

fn error(status: u16, text: &str) -> Response {
    Response { status, kind: JSON, body: Arc::new(format!("{{\"error\":{}}}", json_str(text)).into_bytes()) }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        _ => "Error",
    }
}

fn route(sh: &Shared, method: &str, target: &str) -> Response {
    let (path, query) = split_target(target);
    let q = parse_query(query);
    let post = method == "POST";
    match path {
        "/api/clock" => {
            if post {
                if let Some(p) = param(&q, "paused") {
                    match p {
                        "true" | "1" => sh.paused.store(true, Ordering::Relaxed),
                        "false" | "0" => sh.paused.store(false, Ordering::Relaxed),
                        _ => return error(400, "paused must be true or false"),
                    }
                }
                if let Some(v) = param(&q, "speed") {
                    match v.parse::<u8>() {
                        Ok(n) if (1..=5).contains(&n) => sh.speed.store(n, Ordering::Relaxed),
                        _ => return error(400, "speed must be 1..5"),
                    }
                }
            }
            json(sh.clock_json())
        }
        "/api/countries" => json(countries_json(sh, &sh.game())),
        "/api/seat" => {
            if post {
                let Some(tag) = param(&q, "tag").map(str::to_uppercase) else { return error(400, "tag missing") };
                let g = sh.game();
                if !g.state.countries.contains_key(&tag) || tag == g.core.day.rebel_tag {
                    return error(400, "unknown country");
                }
                *lock(&sh.seat) = tag;
            }
            let tag = lock(&sh.seat).clone();
            let name = country_name(&sh.game(), &tag);
            json(format!("{{\"tag\":{},\"name\":{}}}", json_str(&tag), json_str(&name)))
        }
        "/api/play" => {
            if !post {
                return json(format!("{{\"players\":{}}}", json_list(wiktoria_simulation::player::players(&sh.game().state))));
            }
            let Some(tag) = param(&q, "tag").map(str::to_uppercase) else { return error(400, "tag missing") };
            set_play(sh, Some(&tag))
        }
        "/api/observe" if post => set_play(sh, None),
        "/api/observe" => error(405, "POST to observe"),
        "/api/research" if !post => {
            let seat = lock(&sh.seat).clone();
            json(research_json(sh, &sh.game(), &seat))
        }
        "/api/events" if !post => {
            let seat = lock(&sh.seat).clone();
            json(play::events_json(sh, &mut sh.game(), &seat))
        }
        "/api/population" if !post => match population::ask_of(&q) {
            Ok(a) => {
                let seat = lock(&sh.seat).clone();
                json(population::population_json(sh, &sh.game(), &seat, &a))
            }
            Err(e) => error(400, &e),
        },
        "/api/politics" if !post => {
            let seat = lock(&sh.seat).clone();
            json(politics::politics_json(sh, &sh.game(), &seat))
        }
        "/api/military" if !post => {
            let seat = lock(&sh.seat).clone();
            json(military::military_json(sh, &sh.game(), &seat))
        }
        "/api/trade" if !post => {
            let seat = lock(&sh.seat).clone();
            json(trade::trade_json(sh, &sh.game(), &seat, param(&q, "good"), param(&q, "days").and_then(|d| d.parse().ok())))
        }
        "/api/production" if !post => {
            let seat = lock(&sh.seat).clone();
            json(production::production_json(sh, &sh.game(), &seat))
        }
        "/api/diplomacy" if !post => {
            let seat = lock(&sh.seat).clone();
            json(diplomacy::diplomacy_json(sh, &sh.game(), &seat))
        }
        p if p == "/api/budget" || p == "/api/research" || p == "/api/army/move" || play::PATHS.contains(&p) => {
            if !post {
                return error(405, "orders are POSTed");
            }
            match order_of(path, &q) {
                Ok(order) => give_order(sh, order),
                Err(e) => error(400, &e),
            }
        }
        "/api/screen" => {
            let province = match param(&q, "province").map(str::trim).filter(|p| !p.is_empty() && *p != "null") {
                Some(p) => match p.parse::<u16>() {
                    Ok(id) => Some(id),
                    Err(_) => return error(400, "province must be a number or empty"),
                },
                None => None,
            };
            json(screen_json(sh, province))
        }
        "/api/map" => json(format!(
            "{{\"version\":{},\"modes\":{},\"layers\":{},\"width\":{},\"height\":{}}}",
            sh.map_version.load(Ordering::SeqCst),
            json_list(MapMode::ALL.map(|m| m.label())),
            json_list(Layers::NAMES),
            sh.map.width,
            sh.map.height
        )),
        "/api/pick" => {
            let coord = |k: &str| param(&q, k).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite());
            let (Some(x), Some(y)) = (coord("x"), coord("y")) else { return error(400, "x and y must be numbers 0..1") };
            let id = if (0.0..1.0).contains(&x) && (0.0..1.0).contains(&y) { sh.map.id_at((x * sh.map.width as f64) as usize, (y * sh.map.height as f64) as usize) } else { 0 };
            json(format!("{{\"province\":{}}}", if id == 0 { "null".into() } else { id.to_string() }))
        }
        "/api/install" => json(format!("{{\"path\":{},\"ok\":{},\"version\":null,\"mods\":{}}}", json_str(&sh.install.display().to_string()), sh.install.is_dir(), json_list(&sh.mods))),
        "/api/labels" => labels_json(sh),
        "/map/paper.bmp" => match paper_bmp(&sh.ground_inputs.textures) {
            Some(body) => Response { status: 200, kind: "image/bmp", body: Arc::new(body) },
            None => error(404, "the install has no paper picture"),
        },
        "/map.bmp" => {
            let mode = match param(&q, "mode") {
                Some(m) => match MapMode::from_label(m) {
                    Some(m) => m,
                    None => return error(400, "unknown mode"),
                },
                None => MapMode::Political,
            };
            let width = match param(&q, "width") {
                Some(v) => match v.parse::<usize>() {
                    Ok(n) => n.clamp(64, 8192),
                    Err(_) => return error(400, "width must be a number"),
                },
                None => 2048,
            };
            let layers = match param(&q, "layers") {
                Some(l) => match Layers::parse(l) {
                    Some(l) => l,
                    None => return error(400, "layers are a list of terrain, owners, rivers, borders, glow"),
                },
                None => Layers::ALL,
            };
            let tag = param(&q, "tag").map_or_else(|| lock(&sh.seat).clone(), str::to_uppercase);
            Response { status: 200, kind: "image/bmp", body: map_bmp(sh, mode, width, layers, &tag) }
        }
        p if p == "/api" || p.starts_with("/api/") => error(404, "no such question"),
        p => {
            let Some(rel) = safe_path(p) else { return error(403, "only files inside apps/web are served") };
            let file = sh.web.join(&rel);
            match std::fs::read(&file) {
                Ok(bytes) if file.is_file() => Response { status: 200, kind: content_type(&rel), body: Arc::new(bytes) },
                _ => error(404, "no such file"),
            }
        }
    }
}

fn handle(sh: &Shared, mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
    let mut head = Vec::new();
    let mut chunk = [0u8; 2048];
    let end = loop {
        if let Some(i) = head.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        match stream.read(&mut chunk) {
            Ok(n) if n > 0 && head.len() + n <= MAX_HEAD => head.extend_from_slice(&chunk[..n]),
            _ => return,
        }
    };
    let text = String::from_utf8_lossy(&head[..end]).into_owned();
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, target) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
    let length = lines.filter_map(|l| l.split_once(':')).find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.trim().parse::<usize>().ok()).unwrap_or(0);
    let resp = if length > MAX_BODY {
        error(413, "body too large")
    } else {
        // The body (nothing reads it: every question is in the query string) is taken off the line.
        let mut left = length.saturating_sub(head.len() - end - 4);
        while left > 0 {
            match stream.read(&mut chunk) {
                Ok(n) if n > 0 => left = left.saturating_sub(n),
                _ => break,
            }
        }
        match method {
            "GET" | "HEAD" | "POST" if target.starts_with('/') => route(sh, method, target),
            "GET" | "HEAD" | "POST" => error(400, "bad request"),
            _ => error(405, "only GET and POST"),
        }
    };
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        resp.status,
        reason(resp.status),
        resp.kind,
        resp.body.len()
    );
    if stream.write_all(header.as_bytes()).is_ok() && method != "HEAD" {
        let _ = stream.write_all(&resp.body);
    }
    let _ = stream.flush();
}

pub fn serve(args: &Args) -> Result<(), String> {
    let usage = "usage: serve --save <file.v2> [--port 8036] | serve --scenario <file> [--save <file.v2>] [--port 8036]";
    let mut port = 8036u16;
    let mut scenario_file = None;
    let mut it = args.positional.iter();
    while let Some(a) = it.next() {
        if a == "--port" {
            port = it.next().and_then(|n| n.parse().ok()).ok_or("--port needs a number")?;
        }
        if a == "--scenario" {
            scenario_file = Some(it.next().ok_or("--scenario needs a scenario file")?.clone());
        }
    }
    // A scenario names its own base save (docs/SCENARIOS.md); --save still overrides it.
    let scenario = scenario_file.map(|f| crate::scenario::load_for_serve(args, &f)).transpose()?;
    let save_path = match (&scenario, &args.save) {
        (Some((_, p)), _) => p.to_string_lossy().into_owned(),
        (None, Some(p)) => p.clone(),
        (None, None) => return Err(usage.into()),
    };
    let save_path = &save_path;
    let w = load_world(args)?;
    let engine_rules = config::load_engine_rules(&root())?;
    let install = config::install_dir(&root(), args.install.clone())?;
    let mut loc = Localisation::default();
    let mod_files: Vec<PathBuf> = args.mod_files.iter().map(PathBuf::from).collect();
    for f in wiktoria_data::install_fs::InstallFs::with_mods(&install, &mod_files)?.dir_files(engine_rules.require_str("FLD_LOCALISATION_DIR")?, "csv")? {
        let bytes = std::fs::read(&f).map_err(|e| format!("cannot read {}: {e}", f.display()))?;
        loc.add_file(&wiktoria_data::script::decode_windows1252(&bytes));
    }
    let pop_types = w.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures = w.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let bytes = std::fs::read(save_path).map_err(|e| format!("cannot read {save_path}: {e}"))?;
    let save = wiktoria_data::save::parse_save(&bytes, &pop_types, &cultures)?;
    let map = w.map.clone();
    let ground_inputs = GroundInputs::new(&w, &engine_rules);
    let parties: Vec<String> = w.countries.iter().flat_map(|c| c.country.parties.iter().map(|p| p.name.clone())).collect();
    let t = Instant::now();
    let mut game = Game::new(w, &engine_rules, &install, &save)?;
    // The game starts observed (D20): every country is the computer's, also the save's player.
    let was = wiktoria_simulation::player::observe_all(&mut game.state, &mut game.core);
    if !was.is_empty() {
        eprintln!("observing: {} handed to the computer", was.join(" "));
    }
    eprintln!("game built in {:.2?} at {}", t.elapsed(), game.state.date);
    let mut start_news = Vec::new();
    let live = match scenario {
        Some((sc, _)) => {
            for line in wiktoria_simulation::scenario::prepare(&mut game, &sc)? {
                eprintln!("scenario setup: {line}");
            }
            start_news.push(format!("Scenario: {}. {}", sc.name, sc.what_to_look_for));
            let probe = wiktoria_simulation::scenario::game_probe(&game, None, &sc);
            let run = wiktoria_simulation::scenario::Run::begin(&sc, &game.state, &game.core, &probe);
            Some((sc, run))
        }
        None => None,
    };
    let first_scores = wiktoria_simulation::rank::scores(&game.state, &game.econ.rank_rules, &game.world.defs.modifiers);
    let first_order = wiktoria_simulation::economy::rank_order(first_scores.iter().filter_map(|(t, x)| Some((t.clone(), game.state.countries.get(t)?.civilized, x.iter().sum::<f64>()))).collect());
    let seat = live.as_ref().and_then(|(sc, _)| sc.seat.clone()).or(game.state.rank_order.first().or(first_order.first()).or(game.state.countries.keys().next()).cloned()).unwrap_or_default();
    let start_speed = live.as_ref().and_then(|(sc, _)| sc.speed).unwrap_or(3);
    let web = root().join("apps").join("web");
    let game_date = game.state.date;
    let sh = Arc::new(Shared {
        date: Mutex::new(game.state.date),
        game: Mutex::new(game),
        waiting: AtomicUsize::new(0),
        paused: AtomicBool::new(true),
        speed: AtomicU8::new(start_speed),
        days: AtomicU64::new(0),
        days_per_second: AtomicU64::new(0f64.to_bits()),
        map_version: AtomicU64::new(1),
        seat: Mutex::new(seat),
        news: Mutex::new(VecDeque::from_iter(std::iter::once(News { date: game_date, tag: None, text: format!("Game started from {}", Path::new(save_path).file_name().map_or_else(|| save_path.clone(), |f| f.to_string_lossy().into_owned())) }).chain(start_news.into_iter().map(|text| News { date: game_date, tag: None, text })))),
        scenario: Mutex::new(live),
        yesterday: Mutex::new(Yesterday::default()),
        daily: Mutex::new(Daily::default()),
        watched: Mutex::new(None),
        screen: Mutex::new(None),
        map,
        pictures: Mutex::new(VecDeque::new()),
        ground_inputs,
        ground: OnceLock::new(),
        composed_once: AtomicBool::new(false),
        labels: Mutex::new(None),
        loc,
        parties,
        web,
        install,
        mods: args.mod_files.clone(),
        first_order,
        first_scores,
    });
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("cannot listen on 127.0.0.1:{port}: {e}"))?;
    {
        let sh = sh.clone();
        std::thread::Builder::new().name("sim".into()).spawn(move || sim_thread(&sh)).map_err(|e| format!("cannot start the simulation thread: {e}"))?;
    }
    println!("serving {} on http://127.0.0.1:{port}/ (paused; stop with Ctrl+C)", sh.web.display());
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let sh = sh.clone();
        if std::thread::Builder::new().name("http".into()).spawn(move || handle(&sh, stream)).is_err() {
            eprintln!("serve: could not start a thread for a request");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(json_str("plain"), "\"plain\"");
        assert_eq!(json_str("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(json_str("line\nnext\ttab\r"), "\"line\\nnext\\ttab\\r\"");
        assert_eq!(json_str("\u{1}"), "\"\\u0001\"");
        assert_eq!(json_str("Zażółć"), "\"Zażółć\"");
    }

    #[test]
    fn json_numbers_are_never_nan_or_infinite() {
        assert_eq!(json_num(f64::NAN), "null");
        assert_eq!(json_num(f64::INFINITY), "null");
        assert_eq!(json_num(f64::NEG_INFINITY), "null");
        assert_eq!(json_num(12.0), "12.0");
        assert_eq!(json_num(-0.5), "-0.5");
        assert_eq!(json_num(0.123456), "0.1235");
        assert_eq!(json_num(3.10004), "3.1");
        assert_eq!(json_num(2.99999), "3.0");
        assert_eq!(json_num(-0.0), "0.0");
        assert_eq!(json_num(-0.00001), "0.0");
        assert_eq!(json_int(31963257.6), "31963258");
        assert_eq!(json_int(f64::NAN), "null");
        assert_eq!(json_opt(None), "null");
        assert_eq!(json_pairs(&[("a".into(), 0.5), ("b\"".into(), f64::NAN)]), "[[\"a\",0.5],[\"b\\\"\",null]]");
        assert_eq!(json_list(["x", "y"]), "[\"x\",\"y\"]");
    }

    #[test]
    fn queries_are_split_and_decoded() {
        assert_eq!(split_target("/api/screen?province=300"), ("/api/screen", "province=300"));
        assert_eq!(split_target("/index.html"), ("/index.html", ""));
        assert_eq!(split_target("/a?b=1#frag"), ("/a", "b=1"));
        let q = parse_query("x=0.5&y=0.3&name=a%20b+c&flag&bad=%zz&tail=%4");
        assert_eq!(param(&q, "x"), Some("0.5"));
        assert_eq!(param(&q, "y"), Some("0.3"));
        assert_eq!(param(&q, "name"), Some("a b c"));
        assert_eq!(param(&q, "flag"), Some(""));
        assert_eq!(param(&q, "bad"), Some("%zz"));
        assert_eq!(param(&q, "tail"), Some("%4"));
        assert_eq!(param(&q, "none"), None);
        assert!(parse_query("").is_empty());
        assert_eq!(percent_decode("a+b%2F", false), "a+b/");
    }

    #[test]
    fn paths_stay_inside_the_folder() {
        assert_eq!(safe_path("/"), Some(PathBuf::from("index.html")));
        assert_eq!(safe_path("/style/main.css"), Some(PathBuf::from("style").join("main.css")));
        assert_eq!(safe_path("/API.md"), Some(PathBuf::from("API.md")));
        for bad in [
            "/../Cargo.toml",
            "/a/../../b",
            "/%2e%2e/secret",
            "/..%2fsecret",
            "/a/%2e%2e%5cb",
            "/..\\x",
            "/C:/Windows/win.ini",
            "/c%3a/x",
            "//etc/passwd",
            "/%2fetc/passwd",
            "/a//b",
            "/a/./b",
            "/a/.../b",
            "/trailing./x",
            "/dir/",
            "no-slash",
            "/a\0b",
        ] {
            assert_eq!(safe_path(bad), None, "{bad}");
        }
    }

    #[test]
    fn orders_are_read_from_the_query() {
        let q = |s: &str| parse_query(s);
        assert_eq!(order_of("/api/budget", &q("slider=poor&value=0.35")), Ok(Order::Budget { slider: Slider::PoorTax, value: 0.35 }));
        assert_eq!(order_of("/api/budget", &q("slider=tariff&value=-0.2")), Ok(Order::Budget { slider: Slider::Tariff, value: -0.2 }));
        assert!(order_of("/api/budget", &q("slider=gold&value=1")).unwrap_err().contains("administration"));
        assert!(order_of("/api/budget", &q("slider=poor&value=NaN")).is_err());
        assert!(order_of("/api/budget", &q("slider=poor")).is_err());
        assert_eq!(order_of("/api/research", &q("tech=post_napoleonic_thought")), Ok(Order::Research { tech: "post_napoleonic_thought".into() }));
        assert!(order_of("/api/research", &q("tech=")).is_err());
        assert_eq!(order_of("/api/army/move", &q("army=12&to=300")), Ok(Order::MoveArmy { army: 12, to: 300 }));
        assert!(order_of("/api/army/move", &q("army=x&to=300")).is_err());
        assert!(order_of("/api/army/move", &q("army=1&to=70000")).is_err());
        assert!(order_of("/api/clock", &q("")).is_err());
    }

    #[test]
    fn content_types_by_ending() {
        assert_eq!(content_type(Path::new("index.html")), "text/html; charset=utf-8");
        assert_eq!(content_type(Path::new("a/b.CSS")), "text/css; charset=utf-8");
        assert_eq!(content_type(Path::new("x.js")), "text/javascript; charset=utf-8");
        assert_eq!(content_type(Path::new("x.bmp")), "image/bmp");
        assert_eq!(content_type(Path::new("x.md")), "text/markdown; charset=utf-8");
        assert_eq!(content_type(Path::new("x")), "application/octet-stream");
    }
}
