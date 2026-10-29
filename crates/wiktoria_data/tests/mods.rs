//! Mod layering over a base install (original synthetic fixtures, no game files).

use std::path::{Path, PathBuf};

use wiktoria_data::bmp::{self, Image};
use wiktoria_data::install_fs::InstallFs;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::world::World;

const ENGINE_RULES: &str = r#"{"schema_version": 1, "source": {"rules file": "t", "sha256": "0000000000000000000000000000000000000000000000000000000000000000"},
 "tables": {"Entities": [], "Rules": [], "Fields": [
  {"field_id": "FLD_DEFINES_FILE", "value": "common/defines.lua"},
  {"field_id": "FLD_COUNTRIES_FILE", "value": "common/countries.txt"},
  {"field_id": "FLD_DEFAULT_MAP_FILE", "value": "map/default.map"},
  {"field_id": "FLD_PROVINCE_HISTORY_DIR", "value": "history/provinces"},
  {"field_id": "FLD_LOCALISATION_DIR", "value": "localisation"},
  {"field_id": "FLD_CULTURES_FILE", "value": "common/cultures.txt"},
  {"field_id": "FLD_RELIGION_FILE", "value": "common/religion.txt"},
  {"field_id": "FLD_POPTYPES_DIR", "value": "poptypes"},
  {"field_id": "FLD_POP_HISTORY_DIR", "value": "history/pops"},
  {"field_id": "FLD_COUNTRY_HISTORY_DIR", "value": "history/countries"},
  {"field_id": "FLD_ISSUES_FILE", "value": "common/issues.txt"},
  {"field_id": "FLD_IDEOLOGIES_FILE", "value": "common/ideologies.txt"},
  {"field_id": "FLD_TECHNOLOGY_FILE", "value": "common/technology.txt"},
  {"field_id": "FLD_TECHNOLOGY_DIR", "value": "technologies"},
  {"field_id": "FLD_INVENTIONS_DIR", "value": "inventions"},
  {"field_id": "FLD_BUILDINGS_FILE", "value": "common/buildings.txt"},
  {"field_id": "FLD_BOOKMARKS_FILE", "value": "common/bookmarks.txt"},
  {"field_id": "FLD_PRODUCTION_TYPES_FILE", "value": "common/production_types.txt"},
  {"field_id": "FLD_STATIC_MODIFIERS_FILE", "value": "common/static_modifiers.txt"},
  {"field_id": "FLD_EVENT_MODIFIERS_FILE", "value": "common/event_modifiers.txt"},
  {"field_id": "FLD_NATIONAL_VALUES_FILE", "value": "common/nationalvalues.txt"},
  {"field_id": "FLD_CRIME_FILE", "value": "common/crime.txt"},
  {"field_id": "FLD_PROD_SCALAR_KEYS", "value": "farm_rgo_size"},
  {"field_id": "FLD_PROD_GOOD_KEYS", "value": "rgo_size"}]}}"#;

fn temp(tag: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("wiktoria-mods-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn w(dir: &Path, rel: &str, data: &[u8]) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, data).unwrap();
}

fn definition_csv(prefix: &str) -> String {
    format!("province;red;green;blue;x;x\n1;10;0;0;{prefix}1;x\n2;20;0;0;{prefix}2;x\n")
}

fn base(dir: &Path) {
    w(dir, "common/defines.lua", b"defines = {\nstart_date = '1836.1.1',\n}\n");
    w(dir, "common/countries.txt", b"AAA = \"countries/A.txt\"\nBBB = \"countries/B.txt\"\n");
    w(dir, "common/countries/A.txt", b"color = { 1 2 3 }\n");
    w(dir, "common/countries/B.txt", b"color = { 4 5 6 }\n");
    w(dir, "map/default.map", b"max_provinces = 3\ndefinitions = \"definition.csv\"\nprovinces = \"provinces.bmp\"\nregion = \"region.txt\"\n");
    w(dir, "map/region.txt", b"R_ONE = { 1 2 }\n");
    w(dir, "map/definition.csv", definition_csv("BASE").as_bytes());
    let colors: [[u8; 3]; 2] = [[10, 0, 0], [20, 0, 0]];
    w(dir, "map/provinces.bmp", &bmp::encode(&Image { width: 2, height: 1, rgb: colors.concat() }));
    w(dir, "common/cultures.txt", b"grp = { alpha = { color = { 1 1 1 } } }\n");
    w(dir, "common/religion.txt", b"faiths = { one = { icon = 1 color = { 1 1 0 } } }\n");
    w(dir, "poptypes/farmers.txt", b"strata = poor\n");
    w(dir, "common/production_types.txt", b"");
    w(dir, "common/static_modifiers.txt", b"");
    w(dir, "common/ideologies.txt", b"g = { keepers = { } }\n");
    w(dir, "common/event_modifiers.txt", b"");
    w(dir, "common/nationalvalues.txt", b"");
    w(dir, "common/crime.txt", b"");
    w(dir, "common/issues.txt", b"");
    w(dir, "common/technology.txt", b"folders = { }\n");
    w(dir, "inventions/none.txt", b"");
    w(dir, "localisation/none.csv", b"");
    w(dir, "common/buildings.txt", b"");
    w(dir, "common/bookmarks.txt", b"");
    w(dir, "history/countries/AAA - A.txt", b"capital = 1\nprimary_culture = alpha\nreligion = one\n");
    w(dir, "history/countries/BBB - B.txt", b"capital = 2\nprimary_culture = alpha\nreligion = one\n");
    w(dir, "history/pops/1836.1.1/p.txt", b"1 = { farmers = { culture = alpha religion = one size = 100 } }\n2 = { farmers = { culture = alpha religion = one size = 50 } }\n");
    w(dir, "history/provinces/base/1 - P.txt", b"owner = AAA\ncontroller = AAA\n");
    w(dir, "history/provinces/base/2 - P.txt", b"owner = AAA\ncontroller = AAA\n");
}

/// The mod: adds a province history file, a pop type, and points its own default.map at
/// `../mod/<name>/map/definition.csv`.
fn the_mod(mod_dir: &Path, name: &str) {
    w(mod_dir, "history/provinces/m/1 - P.txt", b"owner = BBB\ncontroller = BBB\n");
    w(mod_dir, "poptypes/clergymen.txt", b"strata = middle\n");
    w(mod_dir, "map/default.map", format!("max_provinces = 3\ndefinitions = \"../mod/{name}/map/definition.csv\"\nprovinces = \"provinces.bmp\"\nregion = \"region.txt\"\n").as_bytes());
    w(mod_dir, "map/definition.csv", definition_csv("MOD").as_bytes());
}

fn engine_rules() -> EngineRules {
    EngineRules::from_json(ENGINE_RULES).unwrap()
}

fn owner_tag(world: &World, id: u16) -> Option<String> {
    world.province(id).unwrap().owner.map(|o| world.country(o).country.tag.clone())
}

#[test]
fn base_only_is_unchanged() {
    let dir = temp("base");
    base(&dir);
    let world = World::load(&dir, &engine_rules()).unwrap();
    assert_eq!(owner_tag(&world, 1).as_deref(), Some("AAA"));
    assert_eq!(owner_tag(&world, 2).as_deref(), Some("AAA"));
    assert_eq!(world.province(1).unwrap().name, "BASE1");
    assert_eq!(world.defs.pop_types.len(), 1);
}

#[test]
fn mod_beside_the_game_folder_layers_over_base() {
    let root = temp("outside");
    let game = root.join("game");
    base(&game);
    let mods = root.join("mods");
    w(&mods, "my.mod", b"name = \"My\"\npath = \"mod/My\"\nreplace_path = \"history/provinces\"\n");
    the_mod(&mods.join("My"), "My");
    let fs = InstallFs::with_mods(&game, &[mods.join("my.mod")]).unwrap();
    let world = World::load_with(fs, &engine_rules()).unwrap();
    // The mod's map definition is read through ../mod/My/ and wins over the base file.
    assert_eq!(world.province(1).unwrap().name, "MOD1");
    // history/provinces is replaced: only the mod's province 1 file counts, province 2 has none.
    assert_eq!(owner_tag(&world, 1).as_deref(), Some("BBB"));
    assert_eq!(owner_tag(&world, 2), None);
    // poptypes is a union of base and mod.
    assert_eq!(world.defs.pop_types.len(), 2);
    // Base-only files still come from the base (pops).
    assert_eq!(world.province(1).unwrap().population(), 100.0);
}

#[test]
fn mod_under_the_game_mod_folder() {
    let game = temp("inside");
    base(&game);
    w(&game, "mod/My.mod", b"path = \"mod/My\"\n");
    the_mod(&game.join("mod/My"), "My");
    let fs = InstallFs::with_mods(&game, &[game.join("mod/My.mod")]).unwrap();
    let world = World::load_with(fs, &engine_rules()).unwrap();
    assert_eq!(world.province(2).unwrap().name, "MOD2");
    // No replace_path: the base province history is still there and the mod's file adds to it.
    assert_eq!(owner_tag(&world, 2).as_deref(), Some("AAA"));
    assert_eq!(owner_tag(&world, 1).as_deref(), Some("BBB"), "mod file applied after the base's");
}

#[test]
fn resolver_rules() {
    let root = temp("rules");
    let game = root.join("g");
    w(&game, "events/a.txt", b"");
    w(&game, "events/b.txt", b"");
    w(&game, "decisions/d.txt", b"");
    let mods = root.join("m");
    w(&mods, "x.mod", b"path = \"mod/X\"\nreplace_path = \"events\"\n");
    w(&mods, "X/events/b.txt", b"mod");
    w(&mods, "X/events/c.txt", b"");
    w(&mods, "X/decisions/e.txt", b"");
    let fs = InstallFs::with_mods(&game, &[mods.join("x.mod")]).unwrap();
    let names = |rel: &str| -> Vec<String> { fs.dir_files(rel, "txt").unwrap().iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect() };
    assert_eq!(names("events"), ["b.txt", "c.txt"], "replace_path hides base files");
    assert_eq!(names("decisions"), ["d.txt", "e.txt"], "union otherwise");
    assert_eq!(fs.file("events/b.txt"), mods.join("X").join("events/b.txt"));
    assert_eq!(fs.file("decisions/d.txt"), game.join("decisions/d.txt"));
    assert_eq!(fs.file("map/../mod/X/events/c.txt"), mods.join("X").join("events/c.txt"));
}
