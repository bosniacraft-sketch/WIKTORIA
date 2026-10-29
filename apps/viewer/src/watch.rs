//! The seam between the simulation and the UI.
//!
//! The UI only reads through [`WatchSource`]. Today [`StaticWorldSource`] implements it from the
//! loaded start-date `World` (no ticking). The combined day loop will implement the same trait
//! later, so nothing in `ui.rs` or `main.rs` changes when the simulation arrives.
//!
//! Everything returned is owned plain data (no lifetimes into the World), so a `Game` can implement
//! the trait by building these structs from its own state. `None` means "not simulated yet" and the UI
//! shows a placeholder.

use std::collections::BTreeMap;

use wiktoria_data::date::Date;
use wiktoria_data::world::World;
use wiktoria_render::{MapMode, Palette};

/// Map modes the UI offers. `War` and `Industry` are hooks: they need the combined day loop
/// (occupation, factories) and fall back to political colours until a source provides them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Political,
    Population,
    Terrain,
    War,
    Industry,
}

impl ViewMode {
    pub const ALL: [ViewMode; 5] =
        [ViewMode::Political, ViewMode::Population, ViewMode::Terrain, ViewMode::War, ViewMode::Industry];

    pub fn label(self) -> &'static str {
        match self {
            ViewMode::Political => "Political",
            ViewMode::Population => "Population",
            ViewMode::Terrain => "Terrain",
            ViewMode::War => "War",
            ViewMode::Industry => "Industry",
        }
    }

    /// Whether the current source can draw this mode.
    pub fn available(self) -> bool {
        matches!(self, ViewMode::Political | ViewMode::Population | ViewMode::Terrain)
    }
}

#[derive(Debug, Clone)]
pub struct CountryFacts {
    pub idx: u16,
    pub tag: String,
    pub name: String,
    pub color: [u8; 3],
    pub prestige: Option<f64>,
    /// None until a simulation provides it.
    pub treasury: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct ProvinceFacts {
    pub id: u16,
    pub name: String,
    pub is_sea: bool,
    pub owner: Option<CountryFacts>,
    pub controller: Option<CountryFacts>,
    pub state: Option<String>,
    pub terrain: Option<String>,
    pub rgo_good: Option<String>,
    pub total_pop: f64,
    /// (pop type, size), largest first.
    pub pop_types: Vec<(String, f64)>,
    /// (name, share 0..1), largest first.
    pub cultures: Vec<(String, f64)>,
    pub religions: Vec<(String, f64)>,
    pub fort: u32,
    pub naval_base: u32,
    pub railroad: u32,
    /// Factories and other state buildings (name, level).
    pub buildings: Vec<(String, u32)>,
    pub life_rating: Option<f64>,
    /// Population-weighted literacy (0..1) and militancy (0..10).
    pub literacy: f64,
    pub militancy: f64,
    /// Core tags.
    pub cores: Vec<String>,
    /// Ideology shares (name, 0..1). Empty until a simulation provides them.
    pub ideologies: Vec<(String, f64)>,
    pub supply_limit: Option<f64>,
    pub rgo_output: Option<f64>,
    pub income: Option<f64>,
    pub colonial: bool,
}

/// What the top bar and the owner block show about one country. `None` = not simulated yet.
#[derive(Debug, Clone, Default)]
pub struct CountrySummary {
    pub rank: Option<u32>,
    pub prestige: Option<f64>,
    pub industry_score: Option<f64>,
    pub military_score: Option<f64>,
    pub government: Option<String>,
    pub ruling_party: Option<String>,
    pub treasury: Option<f64>,
    pub daily_balance: Option<f64>,
    /// Technology being researched.
    pub research: Option<String>,
    pub population: Option<f64>,
    pub pop_growth: Option<f64>,
    pub trade: Option<String>,
    pub at_war: Option<bool>,
    pub army_size: Option<f64>,
    pub navy_size: Option<f64>,
    pub war_exhaustion: Option<f64>,
    pub sphere_leader: Option<u16>,
    pub overlord: Option<u16>,
    pub allies: Vec<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlinerKind {
    Army,
    Navy,
    Influence,
}

#[derive(Debug, Clone)]
pub struct OutlinerEntry {
    pub kind: OutlinerKind,
    pub name: String,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct MessageEntry {
    pub date: Date,
    pub text: String,
}

/// One row of the diplomacy country table.
#[derive(Debug, Clone)]
pub struct DiplomacyRow {
    pub country: u16,
    /// Continent name used by the region filter tabs.
    pub region: String,
    pub relation: Option<i32>,
    /// Influence per great power, in ranking order.
    pub influence: Vec<Option<f64>>,
    pub opinion: Option<String>,
    pub neighbour: bool,
    pub in_sphere: bool,
    pub allied: bool,
    pub enemy: bool,
}

/// One row of the ledger's nation ranking.
#[derive(Debug, Clone)]
pub struct LedgerRow {
    pub country: u16,
    pub rank: u32,
    pub military: Option<f64>,
    pub industrial: Option<f64>,
    pub prestige: Option<f64>,
    pub total: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct RankEntry {
    pub country: u16,
    pub rank: u32,
    pub score: f64,
}

#[derive(Debug, Clone)]
pub struct WarEntry {
    pub name: String,
    pub attackers: Vec<u16>,
    pub defenders: Vec<u16>,
    /// Attacker's war score 0..1, if known.
    pub war_score: Option<f32>,
}

/// Everything the side lists show. A real source refreshes this each day.
#[derive(Debug, Clone, Default)]
pub struct WatchData {
    /// Great powers (diplomacy cards).
    pub ranking: Vec<RankEntry>,
    /// What `RankEntry::score` means, shown as a caption.
    pub ranking_note: String,
    pub wars: Vec<WarEntry>,
    pub wars_note: String,
    /// Nation ranking, best first. Also gives each country's rank.
    pub ledger: Vec<LedgerRow>,
    pub diplomacy: Vec<DiplomacyRow>,
    pub outliner: Vec<OutlinerEntry>,
    /// Oldest first.
    pub messages: Vec<MessageEntry>,
    /// Total population per country index. Empty if unknown.
    pub population_by_country: Vec<f64>,
}

pub trait WatchSource {
    /// Static world data (map, definitions). Only read.
    fn world(&self) -> &World;
    fn date(&self) -> Date;
    fn data(&self) -> &WatchData;
    /// Bumped whenever province ownership or other map-visible data changed.
    fn revision(&self) -> u64 {
        0
    }
    /// Take the newest data from a background simulation (called once a frame; must not block).
    fn poll(&mut self) {}
    /// Pause, speed (1..=5) and the watched country, for a ticking source.
    fn set_control(&self, _paused: bool, _speed: u8, _player: u16) {}
    /// Whether the source ticks (the date moves).
    fn live(&self) -> bool {
        false
    }
    /// One line for the top bar (sim speed, loading state).
    fn sim_status(&self) -> Option<String> {
        None
    }
    /// Screenshot mode: true once the data asked for (GM_UI_RUN_DAYS) is there.
    fn capture_ready(&self) -> bool {
        true
    }
    /// Whether this source can draw a map mode.
    fn supports(&self, mode: ViewMode) -> bool {
        mode.available()
    }
    fn country(&self, idx: u16) -> CountryFacts {
        let w = self.world();
        let c = w.country(idx);
        CountryFacts {
            idx,
            tag: c.country.tag.clone(),
            name: c.name.clone(),
            color: c.country.color,
            prestige: c.history.prestige,
            treasury: None,
        }
    }
    /// Summary block for the top bar, the owner block and the country windows.
    fn country_summary(&self, idx: u16) -> CountrySummary {
        let d = self.data();
        let h = &self.world().country(idx).history;
        CountrySummary {
            rank: d.ledger.iter().find(|r| r.country == idx).map(|r| r.rank),
            prestige: h.prestige,
            government: h.government.clone(),
            ruling_party: h.ruling_party.clone(),
            population: d.population_by_country.get(idx as usize).copied().filter(|p| *p > 0.0),
            ..Default::default()
        }
    }
    /// Armies, navies and influence targets of the player's country.
    fn outliner(&self) -> Vec<OutlinerEntry> {
        self.data().outliner.clone()
    }
    /// The last `n` messages, oldest first.
    fn messages(&self, n: usize) -> Vec<MessageEntry> {
        let m = &self.data().messages;
        m[m.len().saturating_sub(n)..].to_vec()
    }
    fn diplomacy_rows(&self) -> Vec<DiplomacyRow> {
        self.data().diplomacy.clone()
    }
    fn ranking_rows(&self) -> Vec<LedgerRow> {
        self.data().ledger.clone()
    }
    fn province(&self, id: u16) -> Option<ProvinceFacts> {
        let w = self.world();
        let p = w.province(id)?;
        let total = p.population();
        let group = |key: &dyn Fn(&wiktoria_data::world::Pop) -> u16, name: &dyn Fn(u16) -> String| {
            let mut acc: BTreeMap<u16, f64> = BTreeMap::new();
            for pop in &p.pops {
                *acc.entry(key(pop)).or_default() += pop.size;
            }
            let mut v: Vec<(String, f64)> = acc.into_iter().map(|(k, s)| (name(k), s)).collect();
            v.sort_by(|a, b| b.1.total_cmp(&a.1));
            v
        };
        let pop_types = group(&|x| x.pop_type, &|i| w.defs.pop_types[i as usize].name.clone());
        let share = |v: Vec<(String, f64)>| -> Vec<(String, f64)> {
            v.into_iter().map(|(n, s)| (n, if total > 0.0 { s / total } else { 0.0 })).collect()
        };
        let weighted = |f: &dyn Fn(&wiktoria_data::world::Pop) -> f64| {
            if total > 0.0 { p.pops.iter().map(|x| f(x) * x.size).sum::<f64>() / total } else { 0.0 }
        };
        Some(ProvinceFacts {
            id,
            name: p.name.clone(),
            is_sea: p.is_sea,
            owner: p.owner.map(|o| self.country(o)),
            controller: p.controller.map(|o| self.country(o)),
            state: p.region.map(|r| w.regions[r as usize].name.clone()),
            terrain: p.terrain.clone(),
            rgo_good: p.trade_goods.clone(),
            total_pop: total,
            pop_types,
            cultures: share(group(&|x| x.culture, &|i| w.defs.cultures.cultures[i as usize].name.clone())),
            religions: share(group(&|x| x.religion, &|i| w.defs.religions.religions[i as usize].name.clone())),
            fort: p.fort,
            naval_base: p.naval_base,
            railroad: p.railroad,
            buildings: p.state_buildings.iter().map(|b| (b.building.clone(), b.level)).collect(),
            life_rating: p.life_rating,
            literacy: weighted(&|x| x.literacy),
            militancy: weighted(&|x| x.militancy),
            cores: p.cores.iter().map(|c| w.country(*c).country.tag.clone()).collect(),
            colonial: p.colonial.is_some(),
            ..Default::default()
        })
    }
    /// Province colours for a map mode. Sources with live data override this.
    fn palette(&self, mode: ViewMode) -> Palette {
        let w = self.world();
        match mode {
            ViewMode::Political | ViewMode::War | ViewMode::Industry => Palette::build(w, MapMode::Political),
            ViewMode::Population => Palette::build(w, MapMode::Population),
            ViewMode::Terrain => {
                let mut pal = Palette::build(w, MapMode::Political);
                let mut names: Vec<&str> = w.provinces.iter().flatten().filter_map(|p| p.terrain.as_deref()).collect();
                names.sort_unstable();
                names.dedup();
                for p in w.provinces.iter().flatten() {
                    if p.is_sea {
                        continue;
                    }
                    pal.color[p.id as usize] = match p.terrain.as_deref().and_then(|t| names.iter().position(|n| *n == t)) {
                        Some(i) => wiktoria_render::index_color(i as u32 + 3),
                        None => wiktoria_render::NO_DATA,
                    };
                }
                pal
            }
        }
    }
}

/// Static 1836 start from the loaded world. Does not tick.
pub struct StaticWorldSource {
    world: World,
    data: WatchData,
}

impl StaticWorldSource {
    pub fn new(world: World) -> Self {
        let n = world.countries.len();
        let mut pop = vec![0.0f64; n];
        let mut owned = vec![false; n];
        for p in world.provinces.iter().flatten() {
            if let Some(o) = p.owner {
                owned[o as usize] = true;
                pop[o as usize] += p.population();
            }
        }
        // Placeholder ranking: start-date prestige, all countries that own land.
        let mut led: Vec<(u16, f64)> = (0..n)
            .filter(|i| owned[*i])
            .map(|i| (i as u16, world.countries[i].history.prestige.unwrap_or(0.0)))
            .collect();
        led.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let ledger: Vec<LedgerRow> = led
            .iter()
            .enumerate()
            .map(|(i, (c, p))| LedgerRow { country: *c, rank: i as u32 + 1, military: None, industrial: None, prestige: Some(*p), total: None })
            .collect();
        let ranking = ledger
            .iter()
            .take(8)
            .map(|r| RankEntry { country: r.country, rank: r.rank, score: r.prestige.unwrap_or(0.0) })
            .collect();
        let diplomacy = led
            .iter()
            .map(|(c, _)| {
                let region = world.countries[*c as usize]
                    .history
                    .capital
                    .and_then(|cap| world.province(cap))
                    .and_then(|p| p.continent.clone())
                    .unwrap_or_else(|| "Other".into());
                DiplomacyRow {
                    country: *c,
                    region,
                    relation: None,
                    influence: Vec::new(),
                    opinion: None,
                    neighbour: false,
                    in_sphere: false,
                    allied: false,
                    enemy: false,
                }
            })
            .collect();
        let data = WatchData {
            ranking,
            ranking_note: "Placeholder: start prestige, not the real rank formula.".into(),
            wars: Vec::new(),
            wars_note: "No wars yet. Fed by the day loop later.".into(),
            ledger,
            diplomacy,
            outliner: Vec::new(),
            messages: vec![MessageEntry {
                date: world.start_date,
                text: "Static start loaded. Events appear here once the day loop runs.".into(),
            }],
            population_by_country: pop,
        };
        StaticWorldSource { world, data }
    }
}

impl WatchSource for StaticWorldSource {
    fn world(&self) -> &World {
        &self.world
    }
    fn date(&self) -> Date {
        self.world.start_date
    }
    fn data(&self) -> &WatchData {
        &self.data
    }
}
