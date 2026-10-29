//! Country modifier totals.
//!
//! The game keeps one value per modifier key in thousandths (FLD_MOD_STORAGE_SCALE). A country's
//! totals are the sum of named blocks:
//! * FLD_MOD_BASE_BLOCKS, plus the peace or war block;
//! * one rank block (FLD_MOD_RANK_BLOCKS: great power, secondary power, civilized, uncivilized);
//! * blocks scaled by a country value (FLD_MOD_SCALED_BLOCKS), each scaled entry truncated toward
//!   zero to thousandths;
//! * the national value and the technology school;
//! * the chosen option of every reform group in FLD_MOD_CIV_REFORM_CATEGORIES (civilized) or
//!   FLD_MOD_UNCIV_REFORM_CATEGORIES (uncivilized), the group's first option when none is set;
//! * the ruling party's policies (FLD_MOD_PARTY_ISSUE_CATEGORY), for every country;
//! * the direct modifier keys of the country's active inventions.
//!
//! Technology effects and invention `effect` blocks are not part of these totals.

use std::collections::BTreeMap;

use wiktoria_data::defs::IssueGroup;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::modifiers::{ModifierLibrary, ModifierValues};

/// Modifier key -> value in thousandths.
pub type Totals = BTreeMap<String, i64>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rank {
    GreatPower,
    SecondaryPower,
    Civilized,
    Uncivilized,
}

/// The country values the scaled blocks are multiplied by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ScaleBy {
    Plurality,
    Literacy,
    WarExhaustion,
    Badboy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CountryModifierRules {
    pub scale: f64,
    pub base_blocks: Vec<String>,
    pub peace_block: String,
    pub war_block: String,
    /// Great power, secondary power, civilized, uncivilized.
    pub rank_blocks: [String; 4],
    pub scaled_blocks: Vec<(String, ScaleBy)>,
    pub civ_reform_categories: Vec<String>,
    pub unciv_reform_categories: Vec<String>,
    pub party_issue_category: String,
}

fn list(m: &EngineRules, id: &str) -> Result<Vec<String>, String> {
    Ok(m.require_str(id)?.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
}

impl CountryModifierRules {
    pub fn from_engine_rules(m: &EngineRules) -> Result<CountryModifierRules, String> {
        let ranks = list(m, "FLD_MOD_RANK_BLOCKS")?;
        let rank_blocks: [String; 4] = ranks.try_into().map_err(|_| "FLD_MOD_RANK_BLOCKS needs four block names".to_string())?;
        let mut scaled_blocks = Vec::new();
        for pair in list(m, "FLD_MOD_SCALED_BLOCKS")? {
            let (block, by) = pair.split_once(':').ok_or(format!("FLD_MOD_SCALED_BLOCKS: '{pair}' is not block:value"))?;
            let by = match by {
                "plurality" => ScaleBy::Plurality,
                "literacy" => ScaleBy::Literacy,
                "war_exhaustion" => ScaleBy::WarExhaustion,
                "badboy" => ScaleBy::Badboy,
                other => return Err(format!("FLD_MOD_SCALED_BLOCKS: unknown value '{other}'")),
            };
            scaled_blocks.push((block.to_string(), by));
        }
        Ok(CountryModifierRules {
            scale: crate::num_field(m, "FLD_MOD_STORAGE_SCALE")?,
            base_blocks: list(m, "FLD_MOD_BASE_BLOCKS")?,
            peace_block: m.require_str("FLD_MOD_PEACE_BLOCK")?.to_string(),
            war_block: m.require_str("FLD_MOD_WAR_BLOCK")?.to_string(),
            rank_blocks,
            scaled_blocks,
            civ_reform_categories: list(m, "FLD_MOD_CIV_REFORM_CATEGORIES")?,
            unciv_reform_categories: list(m, "FLD_MOD_UNCIV_REFORM_CATEGORIES")?,
            party_issue_category: m.require_str("FLD_MOD_PARTY_ISSUE_CATEGORY")?.to_string(),
        })
    }
}

/// What a country's totals depend on.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CountryModifierInputs {
    pub rank: Option<Rank>,
    pub at_war: bool,
    /// Value of each scaled block's country value (literacy as a 0..1 share, already truncated
    /// to thousandths; plurality in percent).
    pub plurality: f64,
    pub literacy: f64,
    pub war_exhaustion: f64,
    pub badboy: f64,
    pub national_value: Option<String>,
    pub school: Option<String>,
    /// Reform group -> chosen option (groups not listed use their first option).
    pub reforms: BTreeMap<String, String>,
    /// The ruling party's policy options.
    pub party_policies: Vec<String>,
    /// Active inventions by name.
    pub inventions: Vec<String>,
    /// Timed modifiers currently on the country, by name (put there by events and decisions).
    pub event_modifiers: Vec<String>,
}

pub fn country_totals(lib: &ModifierLibrary, issues: &[IssueGroup], rules: &CountryModifierRules, input: &CountryModifierInputs) -> Totals {
    let mut t = Totals::new();
    let scale = rules.scale;
    let mut add = |block: Option<&ModifierValues>, factor: Option<f64>| {
        let Some(block) = block else { return };
        for (k, v) in block {
            let x = match factor {
                None => (v * scale).round() as i64,
                Some(f) => ((v * scale).round() * f).trunc() as i64,
            };
            *t.entry(k.clone()).or_default() += x;
        }
    };
    for b in &rules.base_blocks {
        add(lib.static_modifiers.get(b), None);
    }
    add(lib.static_modifiers.get(if input.at_war { &rules.war_block } else { &rules.peace_block }), None);
    let rank = input.rank.unwrap_or(Rank::Uncivilized);
    let civilized = rank != Rank::Uncivilized;
    let rank_block = match rank {
        Rank::GreatPower => &rules.rank_blocks[0],
        Rank::SecondaryPower => &rules.rank_blocks[1],
        Rank::Civilized => &rules.rank_blocks[2],
        Rank::Uncivilized => &rules.rank_blocks[3],
    };
    add(lib.static_modifiers.get(rank_block), None);
    for (block, by) in &rules.scaled_blocks {
        let f = match by {
            ScaleBy::Plurality => input.plurality,
            ScaleBy::Literacy => input.literacy,
            ScaleBy::WarExhaustion => input.war_exhaustion,
            ScaleBy::Badboy => input.badboy,
        };
        if f != 0.0 {
            add(lib.static_modifiers.get(block), Some(f));
        }
    }
    if let Some(nv) = &input.national_value {
        add(lib.national_values.get(nv), None);
    }
    if let Some(s) = &input.school {
        add(lib.schools.get(s), None);
    }
    let reform_cats = if civilized { &rules.civ_reform_categories } else { &rules.unciv_reform_categories };
    for g in issues.iter().filter(|g| reform_cats.contains(&g.category)) {
        let chosen = input.reforms.get(&g.name).filter(|o| g.options.contains(o)).or(g.options.first());
        if let Some(o) = chosen {
            add(lib.issue_options.get(o), None);
        }
    }
    for p in &input.party_policies {
        add(lib.issue_options.get(p), None);
    }
    for inv in &input.inventions {
        add(lib.inventions.iter().find(|(n, _)| n == inv).map(|(_, v)| v), None);
    }
    for name in &input.event_modifiers {
        add(lib.event_modifiers.get(name), None);
    }
    t.retain(|_, v| *v != 0);
    t
}

// ---------------------------------------------------------------- provinces

#[derive(Debug, Clone, PartialEq)]
pub struct ProvinceModifierRules {
    pub scale: f64,
    pub land_block: String,
    pub sea_block: String,
    pub coastal_sea_block: String,
    /// Static block scaled by the summed building value of this name.
    pub infrastructure: (String, String),
    pub siege_block: String,
    pub terrain_ignored: Vec<String>,
    pub building_keys: Vec<String>,
}

impl ProvinceModifierRules {
    pub fn from_engine_rules(m: &EngineRules) -> Result<ProvinceModifierRules, String> {
        let infra = m.require_str("FLD_PMOD_INFRASTRUCTURE_BLOCK")?;
        let (block, value) = infra.split_once(':').ok_or("FLD_PMOD_INFRASTRUCTURE_BLOCK must be block:building_value")?;
        Ok(ProvinceModifierRules {
            scale: crate::num_field(m, "FLD_MOD_STORAGE_SCALE")?,
            land_block: m.require_str("FLD_PMOD_LAND_BLOCK")?.to_string(),
            sea_block: m.require_str("FLD_PMOD_SEA_BLOCK")?.to_string(),
            coastal_sea_block: m.require_str("FLD_PMOD_COASTAL_SEA_BLOCK")?.to_string(),
            infrastructure: (block.trim().to_string(), value.trim().to_string()),
            siege_block: m.require_str("FLD_PMOD_SIEGE_BLOCK")?.to_string(),
            terrain_ignored: list(m, "FLD_PMOD_TERRAIN_IGNORED_KEYS")?,
            building_keys: list(m, "FLD_PMOD_BUILDING_KEYS")?,
        })
    }
}

/// What a province's totals depend on.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProvinceModifierInputs {
    pub is_sea: bool,
    /// Sea province with a land neighbour.
    pub coastal_sea: bool,
    pub terrain: Option<String>,
    pub climate: Option<String>,
    pub continent: Option<String>,
    /// Position in the crime list.
    pub crime: Option<usize>,
    /// Building name -> level.
    pub buildings: BTreeMap<String, f64>,
    pub besieged: bool,
    /// Timed modifiers currently on the province, by name (put there by events and decisions).
    pub event_modifiers: Vec<String>,
}

/// Definitions the province totals are built from.
pub struct ProvinceSources<'a> {
    pub statics: &'a BTreeMap<String, ModifierValues>,
    pub terrain: &'a wiktoria_data::geography::TerrainDefs,
    pub climates: &'a wiktoria_data::geography::ProvinceGroups,
    pub continents: &'a wiktoria_data::geography::ProvinceGroups,
    pub crimes: &'a [(String, ModifierValues)],
    pub buildings: &'a [wiktoria_data::defs::BuildingType],
    pub events: &'a BTreeMap<String, ModifierValues>,
}

pub fn province_totals(src: &ProvinceSources, rules: &ProvinceModifierRules, input: &ProvinceModifierInputs) -> Totals {
    let mut t = Totals::new();
    let scale = rules.scale;
    let mut add = |block: Option<&ModifierValues>, factor: Option<f64>, skip: &[String]| {
        let Some(block) = block else { return };
        for (k, v) in block {
            if skip.contains(k) {
                continue;
            }
            let x = match factor {
                None => (v * scale).round() as i64,
                Some(f) => ((v * scale).round() * f).trunc() as i64,
            };
            *t.entry(k.clone()).or_default() += x;
        }
    };
    if input.is_sea {
        add(src.statics.get(&rules.sea_block), None, &[]);
        if input.coastal_sea {
            add(src.statics.get(&rules.coastal_sea_block), None, &[]);
        }
    } else {
        add(src.statics.get(&rules.land_block), None, &[]);
    }
    if let Some(cat) = input.terrain.as_ref().and_then(|c| src.terrain.categories.get(c)) {
        add(Some(&cat.values), None, &rules.terrain_ignored);
    }
    if let Some(c) = &input.climate {
        add(src.climates.modifiers.get(c), None, &[]);
    }
    if let Some(c) = &input.continent {
        add(src.continents.modifiers.get(c), None, &[]);
    }
    if let Some(i) = input.crime {
        add(src.crimes.get(i).map(|(_, v)| v), None, &[]);
    }
    let mut infra = 0.0;
    let mut per_level = ModifierValues::new();
    for (name, level) in &input.buildings {
        let Some(b) = src.buildings.iter().find(|b| &b.name == name) else { continue };
        for k in &rules.building_keys {
            if let Some(v) = b.values.get(k) {
                *per_level.entry(k.clone()).or_default() += v * level;
            }
        }
        infra += b.values.get(&rules.infrastructure.1).copied().unwrap_or(0.0) * level;
    }
    add(Some(&per_level), None, &[]);
    if infra != 0.0 {
        add(src.statics.get(&rules.infrastructure.0), Some(infra), &[]);
    }
    if input.besieged {
        add(src.statics.get(&rules.siege_block), None, &[]);
    }
    for name in &input.event_modifiers {
        add(src.events.get(name), None, &[]);
    }
    t.retain(|_, v| *v != 0);
    t
}
