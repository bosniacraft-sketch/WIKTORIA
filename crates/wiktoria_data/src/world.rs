//! Loads the static start-date world from the player's own install.
//! File locations come from `data/engine_rules.json` (the `FLD_*_FILE` / `FLD_*_DIR` entries).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::bmp;
use crate::countries::{self, Country};
use crate::country_history::{self, Classifier, CountryHistory};
use crate::date::Date;
use crate::defines::Defines;
use crate::defs::{self, Bookmark, BuildingType, Cultures, Index, IssueGroup, PopType, Religions, Technology};
use crate::history::{province_id_from_filename, ProvinceHistory, StateBuilding};
use crate::localisation::Localisation;
use crate::map::{self, ProvinceId, ProvinceMap};
use crate::install_fs::InstallFs;
use crate::engine_rules::EngineRules;
use crate::pops::{self, PopEntry};
use crate::script;

pub type CountryIdx = u16;

#[derive(Debug, Clone, PartialEq)]
pub struct Pop {
    pub pop_type: u16,
    pub culture: u16,
    pub religion: u16,
    pub size: f64,
    /// Share of the pop that can read, 0..1. Set by `setup::starting_literacy`.
    pub literacy: f64,
    pub militancy: f64,
    pub rebel_type: Option<String>,
    pub money: f64,
    pub life_needs: f64,
    pub everyday_needs: f64,
    pub luxury_needs: f64,
}

#[derive(Debug, Clone)]
pub struct Province {
    pub id: ProvinceId,
    pub name: String,
    pub rgb: [u8; 3],
    pub is_sea: bool,
    /// Index into `World::regions` (the province's state region).
    pub region: Option<u16>,
    pub owner: Option<CountryIdx>,
    pub controller: Option<CountryIdx>,
    pub cores: Vec<CountryIdx>,
    pub trade_goods: Option<String>,
    pub life_rating: Option<f64>,
    pub colonial: Option<f64>,
    pub fort: u32,
    pub naval_base: u32,
    pub railroad: u32,
    pub state_buildings: Vec<StateBuilding>,
    pub pops: Vec<Pop>,
    /// Terrain category (history override, else the terrain map majority).
    pub terrain: Option<String>,
    pub climate: Option<String>,
    pub continent: Option<String>,
}

impl Province {
    pub fn population(&self) -> f64 {
        self.pops.iter().map(|p| p.size).sum()
    }

    /// Index with the largest summed size under `key` (e.g. culture), if any pops.
    pub fn dominant(&self, key: impl Fn(&Pop) -> u16) -> Option<u16> {
        let mut acc: Vec<(u16, f64)> = Vec::new();
        for p in &self.pops {
            let k = key(p);
            match acc.iter_mut().find(|(c, _)| *c == k) {
                Some(e) => e.1 += p.size,
                None => acc.push((k, p.size)),
            }
        }
        // Ties go to the lower index so the result is deterministic.
        acc.into_iter().max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0))).map(|(k, _)| k)
    }
}

#[derive(Debug, Clone)]
pub struct CountryInfo {
    pub country: Country,
    pub name: String,
    pub history: CountryHistory,
}

#[derive(Debug, Clone)]
pub struct Region {
    pub name: String,
    pub provinces: Vec<ProvinceId>,
}

/// Definition tables from common/, poptypes/, technologies/.
#[derive(Debug)]
pub struct Definitions {
    pub cultures: Cultures,
    pub religions: Religions,
    pub pop_types: Vec<PopType>,
    pub pop_type_index: Index,
    pub issues: Vec<IssueGroup>,
    /// Ideology names in file order (FLD_IDEOLOGIES_FILE).
    pub ideologies: Vec<String>,
    /// Per ideology (same order): the date it becomes available, if it has one.
    pub ideology_dates: Vec<Option<crate::date::Date>>,
    pub technologies: Vec<Technology>,
    pub inventions: Vec<String>,
    pub buildings: Vec<BuildingType>,
    pub bookmarks: Vec<Bookmark>,
    /// RGO good -> pop types it employs, in file order (production_types.txt).
    pub rgo_employees: BTreeMap<String, Vec<u16>>,
    /// Modifier blocks from static modifiers, national values, reforms, schools and inventions.
    pub modifiers: crate::modifiers::ModifierLibrary,
    pub terrain: crate::geography::TerrainDefs,
    pub climates: crate::geography::ProvinceGroups,
    pub continents: crate::geography::ProvinceGroups,
    /// Crimes in file order (a province's crime is a position in this list).
    pub crimes: Vec<(String, crate::modifiers::ModifierValues)>,
    /// Resource-gathering production types (production_types.txt).
    pub rgo_types: Vec<crate::production::RgoType>,
    /// Artisan production types (production_types.txt).
    pub artisan_types: Vec<crate::production::ArtisanType>,
    /// Factory production types (production_types.txt).
    pub factory_types: Vec<crate::production::FactoryType>,
    /// Production effects of each technology, by name.
    pub tech_production: BTreeMap<String, crate::production::ProductionEffects>,
    /// Production effects of each invention's `effect` block, in the game's invention order
    /// (same order as `modifiers.inventions`).
    pub invention_production: Vec<crate::production::ProductionEffects>,
    /// Each invention's whole block (`limit`, `chance`, `effect`), in the same order.
    pub invention_blocks: Vec<script::Block>,
}

#[derive(Debug)]
pub struct World {
    pub start_date: Date,
    pub defines: Defines,
    pub defs: Definitions,
    pub map: ProvinceMap,
    /// Indexed by province id; None for ids not in definition.csv.
    pub provinces: Vec<Option<Province>>,
    pub regions: Vec<Region>,
    pub countries: Vec<CountryInfo>,
    pub tag_index: HashMap<String, CountryIdx>,
    /// The `history/pops` folder used for the start date.
    pub pop_folder: String,
    /// Neighbouring provinces by id (from the province map).
    pub adjacency: Vec<std::collections::BTreeSet<ProvinceId>>,
    /// Extra adjacencies (straits, canals, impassable borders) from adjacencies.csv.
    pub extra_adjacencies: Vec<crate::geography::ExtraAdjacency>,
    /// River pixels on each border (lower id first), from the river picture default.map names;
    /// empty when there is none (`geography::river_borders`).
    pub river_borders: std::collections::BTreeMap<(ProvinceId, ProvinceId), u32>,
    /// Where the terrain and river pictures default.map names were found (mods first), so the
    /// map picture can read them again without keeping them in memory.
    pub map_pictures: MapPictures,
    pub warnings: Vec<String>,
}

/// Paths of the per-pixel pictures of the map, when default.map names them.
#[derive(Debug, Clone, Default)]
pub struct MapPictures {
    pub terrain: Option<std::path::PathBuf>,
    pub rivers: Option<std::path::PathBuf>,
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn read_text(path: &Path) -> Result<String, String> {
    Ok(script::decode_windows1252(&read(path)?))
}

fn parse_file(path: &Path) -> Result<script::Document, String> {
    script::parse(&read_text(path)?).map_err(|e| format!("{}: {e}", path.display()))
}

fn file_name(p: &Path) -> &str {
    p.file_name().and_then(|n| n.to_str()).unwrap_or_default()
}

fn rel_parent(rel: &str) -> String { rel.replace('\\', "/").rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default() }

struct Loader<'a> {
    fs: InstallFs,
    engine_rules: &'a EngineRules,
    warnings: Vec<String>,
}

impl Loader<'_> {
    fn path(&self, field: &str) -> Result<PathBuf, String> {
        Ok(self.fs.file(self.engine_rules.require_str(field)?))
    }

    fn files(&self, field: &str, ext: &str) -> Result<Vec<PathBuf>, String> { self.fs.dir_files(self.engine_rules.require_str(field)?, ext) }

    fn definitions(&mut self) -> Result<Definitions, String> {
        let cultures = defs::parse_cultures(&read_text(&self.path("FLD_CULTURES_FILE")?)?)?;
        let religions = defs::parse_religions(&read_text(&self.path("FLD_RELIGION_FILE")?)?)?;

        let mut pop_types = Vec::new();
        let mut pop_type_index = Index::default();
        for f in self.files("FLD_POPTYPES_DIR", "txt")? {
            let name = f.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
            pop_type_index.insert(&name)?;
            pop_types.push(defs::parse_pop_type(&name, &read_text(&f)?)?);
        }

        let issues_text = read_text(&self.path("FLD_ISSUES_FILE")?)?;
        let issues = defs::parse_issues(&issues_text)?;
        let (ideologies, ideology_dates) = defs::parse_ideology_dates(&read_text(&self.path("FLD_IDEOLOGIES_FILE")?)?)?.into_iter().unzip();
        let mut modifiers = crate::modifiers::ModifierLibrary {
            issue_options: crate::modifiers::parse_issue_option_modifiers(&script::parse(&issues_text).map_err(|e| format!("issues.txt: {e}"))?.root),
            ..Default::default()
        };
        for (field, slot) in [("FLD_STATIC_MODIFIERS_FILE", &mut modifiers.static_modifiers), ("FLD_NATIONAL_VALUES_FILE", &mut modifiers.national_values)] {
            let p = self.path(field)?;
            let doc = script::parse(&read_text(&p)?).map_err(|e| format!("{}: {e}", p.display()))?;
            self.warnings.extend(doc.warnings.iter().map(|w| format!("{}: {w}", p.display())));
            *slot = crate::modifiers::parse_named_blocks(&doc.root);
        }
        {
            let p = self.path("FLD_EVENT_MODIFIERS_FILE")?;
            let doc = script::parse(&read_text(&p)?).map_err(|e| format!("{}: {e}", p.display()))?;
            self.warnings.extend(doc.warnings.iter().map(|w| format!("{}: {w}", p.display())));
            modifiers.event_modifiers = crate::modifiers::parse_event_modifiers(&doc.root);
        }

        let list = |id: &str| -> Result<Vec<String>, String> {
            Ok(self.engine_rules.require_str(id)?.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        };
        let prod_scalars = list("FLD_PROD_SCALAR_KEYS")?;
        let prod_goods = list("FLD_PROD_GOOD_KEYS")?;
        let mut tech_production = BTreeMap::new();
        let mut invention_production = Vec::new();
        let mut invention_blocks = Vec::new();

        let tech_dir = self.engine_rules.require_str("FLD_TECHNOLOGY_DIR")?.to_string();
        let mut technologies = Vec::new();
        let tech_text = read_text(&self.path("FLD_TECHNOLOGY_FILE")?)?;
        if let Ok(d) = script::parse(&tech_text) {
            modifiers.schools = crate::modifiers::parse_schools(&d.root);
        }
        for folder in defs::parse_tech_folders(&tech_text)? {
            let f = self.fs.file(&format!("{tech_dir}/{folder}.txt"));
            let text = read_text(&f)?;
            technologies.extend(defs::parse_technologies(&text, file_name(&f))?);
            if let Ok(d) = script::parse(&text) {
                for (name, e) in crate::production::technology_effects(&d.root, &prod_scalars, &prod_goods) {
                    tech_production.insert(name, e);
                }
            }
        }
        let mut seen = HashSet::new();
        for t in &technologies {
            if !seen.insert(t.name.as_str()) {
                self.warnings.push(format!("technology '{}' defined twice", t.name));
            }
        }

        let mut inventions = Vec::new();
        for f in self.files("FLD_INVENTIONS_DIR", "txt")? {
            let text = read_text(&f)?;
            inventions.extend(defs::parse_inventions(&text, file_name(&f))?);
            if let Ok(d) = script::parse(&text) {
                modifiers.inventions.extend(crate::modifiers::parse_invention_modifiers(&d.root));
                invention_blocks.extend(d.root.entries().filter_map(|(_, v, _)| v.as_block().cloned()));
                invention_production.extend(crate::production::invention_effects(&d.root, &prod_scalars, &prod_goods).into_iter().map(|(_, e)| e));
            }
        }

        let buildings = defs::parse_buildings(&read_text(&self.path("FLD_BUILDINGS_FILE")?)?)?;
        let bookmarks = defs::parse_bookmarks(&read_text(&self.path("FLD_BOOKMARKS_FILE")?)?)?;
        let production_text = read_text(&self.path("FLD_PRODUCTION_TYPES_FILE")?)?;
        let rgo_types = crate::production::parse_rgo_types(&production_text)?;
        let artisan_types = crate::production::parse_artisan_types(&production_text)?;
        let factory_types = crate::production::parse_factory_types(&production_text)?;
        let mut rgo_employees = BTreeMap::new();
        for (good, types) in defs::parse_rgo_employees(&production_text)? {
            let mut ids = Vec::new();
            for t in types {
                match pop_type_index.get(&t) {
                    Some(i) => ids.push(i),
                    None => self.warnings.push(format!("production type for {good}: unknown pop type '{t}'")),
                }
            }
            rgo_employees.insert(good, ids);
        }
        Ok(Definitions {
            cultures,
            religions,
            pop_types,
            pop_type_index,
            issues,
            ideologies,
            ideology_dates,
            technologies,
            inventions,
            buildings,
            bookmarks,
            rgo_employees,
            modifiers,
            terrain: Default::default(),
            climates: Default::default(),
            continents: Default::default(),
            rgo_types,
            artisan_types,
            factory_types,
            tech_production,
            invention_production,
            invention_blocks,
            crimes: crate::geography::ordered_blocks(&script::parse(&read_text(&self.path("FLD_CRIME_FILE")?)?).map_err(|e| format!("crime file: {e}"))?.root),
        })
    }
}

impl World {
    pub fn load(install: &Path, engine_rules: &EngineRules) -> Result<World, String> { Self::load_with(InstallFs::base_only(install), engine_rules) }

    /// Loads the base install with mod layers over it (see `InstallFs`).
    pub fn load_with(fs: InstallFs, engine_rules: &EngineRules) -> Result<World, String> {
        let mut l = Loader { fs, engine_rules, warnings: Vec::new() };

        let defines = Defines::parse(&read_text(&l.path("FLD_DEFINES_FILE")?)?)?;
        let start_date = defines.start_date()?;
        let mut defs = l.definitions()?;

        // ---- map
        let default_map_path = l.path("FLD_DEFAULT_MAP_FILE")?;
        let map_rel = rel_parent(l.engine_rules.require_str("FLD_DEFAULT_MAP_FILE")?);
        let map_file = |name: &str| l.fs.file(&format!("{map_rel}/{name}"));
        let dm = map::parse_default_map(&read_text(&default_map_path)?)?;
        let (province_defs, w) = map::parse_definitions(&read_text(&map_file(&dm.definitions))?)?;
        l.warnings.extend(w);
        let mut img = bmp::decode(&read(&map_file(&dm.provinces))?).map_err(|e| format!("{}: {e}", dm.provinces))?;
        // The game treats the first *stored* row as north, so a standard (bottom-up) decode
        // is upside down. Flip so row 0 is the top of the world map.
        img.flip_vertical();
        let (province_map, w) = ProvinceMap::build(&img, &province_defs);
        // ---- geography: terrain map, climates, continents, adjacency
        let parse_map_file = |name: &Option<String>| -> Result<Option<crate::script::Document>, String> {
            match name {
                Some(n) => script::parse(&read_text(&map_file(n))?).map(Some).map_err(|e| format!("{n}: {e}")),
                None => Ok(None),
            }
        };
        if let Some(d) = parse_map_file(&dm.terrain_definition)? {
            defs.terrain = crate::geography::parse_terrain(&d.root);
        }
        let mut map_terrain = Vec::new();
        if let Some(t) = &dm.terrain {
            let mut ti = bmp::decode_indexed(&read(&map_file(t))?).map_err(|e| format!("{t}: {e}"))?;
            ti.flip_vertical();
            map_terrain = crate::geography::province_terrain(&province_map, &ti, &defs.terrain)?;
        }
        // default.map names no climate file; the rules file gives its path (FLD_CLIMATE_FILE).
        let climate_doc = match &dm.climate {
            Some(_) => parse_map_file(&dm.climate)?,
            None => match l.engine_rules.field("FLD_CLIMATE_FILE").and_then(|v| v.as_str()) {
                Some(rel) => {
                    let p = l.fs.file(rel);
                    Some(script::parse(&read_text(&p)?).map_err(|e| format!("{}: {e}", p.display()))?)
                }
                None => None,
            },
        };
        if let Some(d) = climate_doc {
            defs.climates = crate::geography::parse_province_groups(&d.root);
        }
        if let Some(d) = parse_map_file(&dm.continent)? {
            defs.continents = crate::geography::parse_province_groups(&d.root);
        }
        let map_pictures = MapPictures { terrain: dm.terrain.as_deref().map(map_file), rivers: dm.rivers.as_deref().map(map_file) };
        let adjacency = crate::geography::adjacency(&province_map);
        let mut river_borders = std::collections::BTreeMap::new();
        if let (Some(f), Some(max)) = (&dm.rivers, l.engine_rules.field("FLD_MAP_RIVER_INDEX_MAX").and_then(|v| v.as_f64())) {
            match std::fs::read(map_file(f)) {
                Ok(b) => {
                    let mut ri = bmp::decode_indexed(&b).map_err(|e| format!("{f}: {e}"))?;
                    ri.flip_vertical();
                    river_borders = crate::geography::river_borders(&province_map, &ri, max as u8)?;
                }
                Err(e) => l.warnings.push(format!("{f}: {e}")),
            }
        }
        let extra_adjacencies = match &dm.adjacencies {
            Some(f) => match std::fs::read(map_file(f)) {
                Ok(b) => crate::geography::parse_adjacencies(&String::from_utf8_lossy(&b)),
                Err(e) => {
                    l.warnings.push(format!("{f}: {e}"));
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        drop(img);
        l.warnings.extend(w);
        let (region_list, w) = map::parse_regions(&read_text(&map_file(&dm.region))?)?;
        l.warnings.extend(w);

        // ---- localisation
        let mut loc = Localisation::default();
        for f in l.files("FLD_LOCALISATION_DIR", "csv")? {
            loc.add_file(&read_text(&f)?);
        }

        // ---- countries
        let countries_path = l.path("FLD_COUNTRIES_FILE")?;
        let common_rel = rel_parent(l.engine_rules.require_str("FLD_COUNTRIES_FILE")?);
        let (list, w) = countries::parse_country_list(&read_text(&countries_path)?)?;
        l.warnings.extend(w);
        let mut countries_out = Vec::new();
        let mut tag_index = HashMap::new();
        for entry in list {
            let path = l.fs.file(&format!("{common_rel}/{}", entry.file));
            let country = match read_text(&path).and_then(|src| countries::parse_country_file(&entry, &src)) {
                Ok(c) => c,
                Err(e) => {
                    l.warnings.push(format!("country {}: {e}", entry.tag));
                    continue;
                }
            };
            let name = loc.get(&entry.tag).unwrap_or(&entry.tag).to_string();
            tag_index.insert(entry.tag.clone(), countries_out.len() as CountryIdx);
            countries_out.push(CountryInfo { country, name, history: CountryHistory::default() });
        }

        // ---- country history
        let classifier = Classifier {
            issue_groups: defs.issues.iter().map(|g| (g.name.clone(), g.options.clone())).collect(),
            technologies: defs.technologies.iter().map(|t| t.name.clone()).collect(),
            inventions: defs.inventions.iter().cloned().collect(),
        };
        for f in l.files("FLD_COUNTRY_HISTORY_DIR", "txt")? {
            let fname = file_name(&f).to_string();
            let Some(tag) = country_history::tag_from_filename(&fname) else {
                l.warnings.push(format!("{fname}: no country tag in file name"));
                continue;
            };
            let Some(&idx) = tag_index.get(&tag) else {
                l.warnings.push(format!("{fname}: tag {tag} is not in countries.txt"));
                continue;
            };
            let doc = parse_file(&f)?;
            let label = format!("history {fname}");
            l.warnings.extend(doc.warnings.iter().map(|w| format!("{label}: {w}")));
            countries_out[idx as usize].history.apply(&doc, start_date, &classifier, &mut l.warnings, &label);
        }

        // ---- province history
        let mut history: HashMap<ProvinceId, ProvinceHistory> = HashMap::new();
        let mut files_per_id: HashMap<ProvinceId, usize> = HashMap::new();
        for f in l.files("FLD_PROVINCE_HISTORY_DIR", "txt")? {
            let fname = file_name(&f).to_string();
            let Some(id) = province_id_from_filename(&fname) else {
                l.warnings.push(format!("{}: no province id in file name", f.display()));
                continue;
            };
            let label = format!("history {fname}");
            let doc = parse_file(&f)?;
            l.warnings.extend(doc.warnings.iter().map(|w| format!("{label}: {w}")));
            history.entry(id).or_default().apply(&doc, start_date, &mut l.warnings, &label);
            *files_per_id.entry(id).or_default() += 1;
        }
        let mut dup: Vec<_> = files_per_id.iter().filter(|(_, n)| **n > 1).map(|(id, _)| *id).collect();
        dup.sort();
        if !dup.is_empty() {
            l.warnings.push(format!("province history: several files for ids {dup:?}; applied in path order"));
        }

        // ---- pops
        let pop_rel = l.engine_rules.require_str("FLD_POP_HISTORY_DIR")?.to_string();
        let pop_root = l.fs.base.join(&pop_rel);
        let folders = l.fs.subdirs(&pop_rel)?;
        let pop_folder = pops::pick_pop_folder(&folders, start_date)
            .ok_or(format!("no pop folder in {} dated on or before {start_date}", pop_root.display()))?
            .to_string();
        let mut pop_entries: HashMap<ProvinceId, Vec<PopEntry>> = HashMap::new();
        for f in l.fs.dir_files(&format!("{pop_rel}/{pop_folder}"), "txt")? {
            let doc = parse_file(&f)?;
            pops::parse_pop_file(&doc, &format!("pops {}", file_name(&f)), &mut pop_entries, &mut l.warnings);
        }

        // ---- assemble provinces
        let max_id = province_defs.iter().map(|d| d.id).max().unwrap_or(0) as usize;
        let sea: HashSet<ProvinceId> = dm.sea_starts.iter().copied().collect();
        let mut region_of: HashMap<ProvinceId, u16> = HashMap::new();
        for (i, (_, ids)) in region_list.iter().enumerate() {
            for id in ids {
                region_of.entry(*id).or_insert(i as u16);
            }
        }
        let building_names: HashSet<&str> = defs.buildings.iter().map(|b| b.name.as_str()).collect();
        let mut provinces: Vec<Option<Province>> = vec![None; max_id + 1];
        let mut warnings = std::mem::take(&mut l.warnings);
        let resolve = |tag: &str, what: &str, id: ProvinceId, warnings: &mut Vec<String>| {
            let idx = tag_index.get(tag).copied();
            if idx.is_none() {
                warnings.push(format!("province {id}: {what} {tag} is not in countries.txt"));
            }
            idx
        };
        let mut unknown_pop = (0usize, HashSet::new());
        for d in &province_defs {
            let h = history.remove(&d.id).unwrap_or_default();
            let owner = h.owner.as_deref().and_then(|t| resolve(t, "owner", d.id, &mut warnings));
            let controller = h.controller.as_deref().and_then(|t| resolve(t, "controller", d.id, &mut warnings));
            let cores = h.cores.iter().filter_map(|c| resolve(c, "core", d.id, &mut warnings)).collect();
            for b in &h.state_buildings {
                if !building_names.contains(b.building.as_str()) {
                    warnings.push(format!("province {}: unknown building '{}'", d.id, b.building));
                }
            }
            let mut pops_out = Vec::new();
            for e in pop_entries.remove(&d.id).unwrap_or_default() {
                let ids = (
                    defs.pop_type_index.get(&e.pop_type),
                    defs.cultures.index.get(&e.culture),
                    defs.religions.index.get(&e.religion),
                );
                match ids {
                    (Some(pop_type), Some(culture), Some(religion)) => pops_out.push(Pop {
                        pop_type,
                        culture,
                        religion,
                        size: e.size,
                        literacy: 0.0,
                        militancy: e.militancy,
                        rebel_type: e.rebel_type,
                        money: 10.0,
                        life_needs: 1.0,
                        everyday_needs: 1.0,
                        luxury_needs: 1.0,
                    }),
                    _ => {
                        unknown_pop.0 += 1;
                        unknown_pop.1.insert(format!("{}/{}/{}", e.pop_type, e.culture, e.religion));
                    }
                }
            }
            let name = loc.get(&format!("PROV{}", d.id)).map(str::to_string).unwrap_or_else(|| d.name.clone());
            provinces[d.id as usize] = Some(Province {
                id: d.id,
                name,
                rgb: d.rgb,
                is_sea: sea.contains(&d.id),
                region: region_of.get(&d.id).copied(),
                owner,
                controller,
                cores,
                trade_goods: h.trade_goods,
                life_rating: h.life_rating,
                colonial: h.colonial,
                fort: h.fort,
                naval_base: h.naval_base,
                railroad: h.railroad,
                state_buildings: h.state_buildings,
                pops: pops_out,
                terrain: h.terrain.clone().or_else(|| map_terrain.get(d.id as usize).cloned().flatten()),
                climate: defs.climates.member.get(&d.id).cloned(),
                continent: defs.continents.member.get(&d.id).cloned(),
            });
        }
        if unknown_pop.0 > 0 {
            let mut ex: Vec<_> = unknown_pop.1.into_iter().collect();
            ex.sort();
            ex.truncate(5);
            warnings.push(format!("{} pops with unknown type/culture/religion skipped, e.g. {ex:?}", unknown_pop.0));
        }
        let mut orphan: Vec<_> = history.keys().copied().collect();
        orphan.sort();
        if !orphan.is_empty() {
            warnings.push(format!("province history for ids not in definition.csv: {orphan:?}"));
        }
        let mut orphan_pops: Vec<_> = pop_entries.keys().copied().collect();
        orphan_pops.sort();
        if !orphan_pops.is_empty() {
            warnings.push(format!("pops for ids not in definition.csv: {orphan_pops:?}"));
        }

        // ---- country cross-checks
        for c in &countries_out {
            let h = &c.history;
            let tag = &c.country.tag;
            if let Some(cap) = h.capital
                && provinces.get(cap as usize).and_then(|p| p.as_ref()).is_none()
            {
                warnings.push(format!("{tag}: capital {cap} is not a defined province"));
            }
            for cul in h.primary_culture.iter().chain(&h.cultures) {
                if defs.cultures.index.get(cul).is_none() {
                    warnings.push(format!("{tag}: unknown culture '{cul}'"));
                }
            }
            if let Some(r) = &h.religion
                && defs.religions.index.get(r).is_none()
            {
                warnings.push(format!("{tag}: unknown religion '{r}'"));
            }
        }

        let regions = region_list.into_iter().map(|(name, provinces)| Region { name, provinces }).collect();
        Ok(World {
            start_date,
            defines,
            defs,
            map: province_map,
            provinces,
            regions,
            countries: countries_out,
            tag_index,
            pop_folder,
            adjacency,
            extra_adjacencies,
            river_borders,
            map_pictures,
            warnings,
        })
    }

    pub fn province(&self, id: ProvinceId) -> Option<&Province> {
        self.provinces.get(id as usize)?.as_ref()
    }

    pub fn country(&self, idx: CountryIdx) -> &CountryInfo {
        &self.countries[idx as usize]
    }

    /// The bookmark matching the start date, if any (used for the initial camera).
    pub fn start_bookmark(&self) -> Option<&Bookmark> {
        self.defs.bookmarks.iter().find(|b| b.date == self.start_date)
    }
}
