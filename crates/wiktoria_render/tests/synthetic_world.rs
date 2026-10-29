//! End-to-end: a tiny synthetic install on disk -> World::load -> render/pick.
//! Every file here is original test data.

use std::path::PathBuf;

use wiktoria_data::bmp::{self, Image};
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::world::World;
use wiktoria_render::{pick, render, rgb, Camera, MapMode, Palette, RenderOptions, SEA, UNOWNED};

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
  {"field_id": "FLD_PROD_SCALAR_KEYS", "value": "farm_rgo_size,mine_rgo_size,farm_rgo_eff,mine_rgo_eff,rgo_input,rgo_output,rgo_throughput"},
  {"field_id": "FLD_PROD_GOOD_KEYS", "value": "rgo_size,rgo_goods_input,rgo_goods_output,rgo_goods_throughput"}]}}"#;

const RED: [u8; 3] = [200, 0, 0];
const GRN: [u8; 3] = [0, 200, 0];
const BLU: [u8; 3] = [0, 0, 200];
const GRY: [u8; 3] = [90, 90, 90];

/// World layout (row 0 = north):
///   1 1 2 4
///   3 3 3 3      1 = AAA, 2 = BBB (dynamic tag), 3 = sea, 4 = unowned land
fn install() -> PathBuf {
    // Tests run in parallel: every call gets its own folder.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("wiktoria-synth-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let w = |rel: &str, data: &[u8]| {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    };
    w("common/defines.lua", b"defines = {\nstart_date = '1836.1.1',\nend_date = '1935.12.31',\n}\n");
    w("common/countries.txt", b"AAA = \"countries/A.txt\"\ndynamic_tags = yes\nBBB = \"countries/B.txt\"\n");
    w("common/countries/A.txt", b"color = { 10 20 30 }\n");
    w("common/countries/B.txt", b"color = { 40 50 60 }\n");
    w("map/default.map", b"max_provinces = 5\nsea_starts = { 3 }\ndefinitions = \"definition.csv\"\nprovinces = \"provinces.bmp\"\nregion = \"region.txt\"\n");
    w("map/region.txt", b"R_WEST = { 1 }\nR_EAST = { 2 4 }\n");
    w("common/cultures.txt", b"grp = { leader = x\n alpha = { color = { 200 100 0 } }\n beta = { color = { 0 100 200 } } }\n");
    w("common/religion.txt", b"faiths = { one = { icon = 1 color = { 1 1 0 } } two = { icon = 2 color = { 0 1 1 } } }\n");
    w("poptypes/farmers.txt", b"sprite = 1\ncolor = { 1 2 3 }\nstrata = poor\n");
    w("poptypes/clerks.txt", b"sprite = 2\ncolor = { 4 5 6 }\nstrata = middle\n");
    w("poptypes/labourers.txt", b"sprite = 3\ncolor = { 7 8 9 }\nstrata = poor\n");
    w("history/pops/1836.1.1/test.txt",
      b"1 = { farmers = { culture = alpha religion = one size = 900 } clerks = { culture = beta religion = two size = 100 } }\n\
        2 = { farmers = { culture = beta religion = two size = 300 } }\n");
    w("history/pops/1900.1.1/test.txt", b"1 = { farmers = { culture = beta religion = two size = 1 } }\n");
    w("history/countries/AAA - Alpha.txt", b"capital = 1\nprimary_culture = alpha\nreligion = one\nvoting = none\nt1 = 1\ni1 = yes\n");
    w("common/static_modifiers.txt", b"base_values = { supply_limit = 5 }\n");
    w("common/ideologies.txt", b"old_group = { keepers = { } }\nnew_group = { changers = { } }\n");
    w("common/event_modifiers.txt", b"plague = { population_growth = -0.02 icon = 3 }\n");
    w("common/nationalvalues.txt", b"nv_test = { mobilisation_size = 0.04 }\n");
    w("common/crime.txt", b"bad_crime = { trigger = { } local_rgo_output = -0.2 }\n");
    w("common/issues.txt", b"political_reforms = { voting = { none = { } all = { } } }\n");
    w("common/technology.txt", b"folders = { test_tech = { area1 } }\n");
    w("technologies/test_tech.txt", b"t1 = { area = area1 year = 1836 cost = 10 }\n");
    w("inventions/test_inventions.txt", b"i1 = { limit = { t1 = 1 } }\n");
    w("common/buildings.txt", b"mill = { type = factory max_level = 99 }\n");
    w("common/production_types.txt", b"");
    w("common/bookmarks.txt", b"bookmark = { name = \"START\" date = 1836.1.1 cameraX = 2 cameraY = 1 }\n");
    w("map/definition.csv", b"province;red;green;blue;x;x\n1;200;0;0;Westfield;x\n2;0;200;0;Eastfield;x\n3;0;0;200;Sea;x\n4;90;90;90;Wilds;x\n");
    let north = [RED, RED, GRN, GRY];
    let south = [BLU; 4];
    // Stored like the game's file: the first stored (bottom) row is north.
    let mut px = Vec::new();
    for row in [south, north] {
        for c in row {
            px.extend_from_slice(&c);
        }
    }
    w("map/provinces.bmp", &bmp::encode(&Image { width: 4, height: 2, rgb: px }));
    w("history/provinces/test/1 - Westfield.txt", b"owner = AAA\ncontroller = AAA\nadd_core = AAA\n1900.1.1 = { owner = BBB }\n");
    w("history/provinces/test/2 - Eastfield.txt",
      b"owner = BBB\ncontroller = AAA\nadd_core = BBB\ntrade_goods = clay\nfort = 1\nstate_building = { level = 2 building = mill upgrade = yes }\n");
    w("localisation/a.csv", b"PROV1;West Field;x\nAAA;Alphaland;x\n");
    dir
}

#[test]
fn loads_renders_and_picks_a_synthetic_world() {
    let dir = install();
    let engine_rules = EngineRules::from_json(ENGINE_RULES).unwrap();
    let world = World::load(&dir, &engine_rules).unwrap();
    assert!(world.warnings.is_empty(), "{:#?}", world.warnings);

    let p1 = world.province(1).unwrap();
    assert_eq!(p1.name, "West Field", "localised name");
    assert_eq!(world.country(p1.owner.unwrap()).name, "Alphaland");
    let p2 = world.province(2).unwrap();
    assert_eq!(p2.name, "Eastfield", "falls back to definition.csv");
    let bbb = world.country(p2.owner.unwrap());
    assert!(bbb.country.dynamic && bbb.name == "BBB");
    assert_eq!(world.country(p2.controller.unwrap()).country.tag, "AAA");
    assert!(world.province(3).unwrap().is_sea);
    assert!(world.province(4).unwrap().owner.is_none());

    // Phase 2 data
    assert_eq!(world.pop_folder, "1836.1.1");
    assert_eq!(p1.population(), 1000.0);
    assert_eq!(world.defs.cultures.cultures[p1.dominant(|x| x.culture).unwrap() as usize].name, "alpha");
    assert_eq!(world.regions[p2.region.unwrap() as usize].name, "R_EAST");
    assert_eq!((p2.fort, p2.state_buildings[0].level), (1, 2));
    let aaa = &world.country(p1.owner.unwrap()).history;
    assert_eq!((aaa.capital, aaa.reforms["voting"].as_str()), (Some(1), "none"));
    assert!(aaa.technologies.contains("t1") && aaa.inventions.contains("i1"));
    assert_eq!(world.start_bookmark().unwrap().camera, (2.0, 1.0));
    let culture = Palette::build(&world, MapMode::Culture);
    assert_eq!((culture.color[1], culture.color[2]), (rgb([200, 100, 0]), rgb([0, 100, 200])));
    assert_eq!(culture.color[3], SEA);
    assert_eq!(culture.color[4], wiktoria_render::NO_DATA, "no pops");
    let religion = Palette::build(&world, MapMode::Religion);
    assert_eq!(religion.color[1], rgb([255, 255, 0]));
    let states = Palette::build(&world, MapMode::States);
    assert_eq!(states.color[2], states.color[4], "same state, same colour");
    assert_ne!(states.color[1], states.color[2]);
    let population = Palette::build(&world, MapMode::Population);
    assert_ne!(population.color[1], population.color[2], "different populations");
    assert!(wiktoria_render::describe(&world, 1).ends_with("pop 1,000"));

    // North row must be on top after loading.
    assert_eq!((world.map.id_at(0, 0), world.map.id_at(3, 0), world.map.id_at(0, 1)), (1, 4, 3));

    let pal = Palette::build(&world, MapMode::Political);
    let cam = Camera { x: 0.0, y: 0.0, zoom: 1.0 };
    let mut buf = vec![0; 8];
    render(&world.map, &pal, &cam, &RenderOptions { borders: false, highlight: None }, &mut buf, 4, 2);
    assert_eq!(buf, vec![rgb([10, 20, 30]), rgb([10, 20, 30]), rgb([40, 50, 60]), UNOWNED, SEA, SEA, SEA, SEA]);

    // Horizontal wrap: one map pixel to the left shows province 4 at the screen's left edge.
    let wrapped = Camera { x: -1.0, ..cam };
    assert_eq!(pick(&world.map, &wrapped, 0.5, 0.5), Some(4));
    assert_eq!(pick(&world.map, &cam, 2.5, 0.5), Some(2));
    assert_eq!(pick(&world.map, &cam, 0.5, 5.0), None, "below the map");

    // Borders: the AAA/BBB edge is a country border, the sea is a different owner too.
    let mut bordered = vec![0; 8];
    render(&world.map, &pal, &cam, &RenderOptions { borders: true, highlight: Some(2) }, &mut bordered, 4, 2);
    assert_eq!(bordered[1], wiktoria_render::COUNTRY_BORDER);
    assert_ne!(bordered[2], buf[2], "highlight changes the colour");

    let prov = Palette::build(&world, MapMode::Provinces);
    assert_eq!(prov.color[1], rgb(RED));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn camera_zoom_keeps_point_under_cursor() {
    let map = wiktoria_data::map::ProvinceMap { width: 100, height: 50, ids: vec![0; 5000], pixel_count: vec![0], centroid: vec![(0.0, 0.0)] };
    let mut cam = Camera { x: 10.0, y: 5.0, zoom: 2.0 };
    let before = cam.screen_to_map(&map, 40.0, 20.0).unwrap();
    cam.zoom_at(&map, 40.0, 20.0, 1.5, 60);
    assert_eq!(cam.screen_to_map(&map, 40.0, 20.0).unwrap(), before);
    cam.pan(&map, 1000.0, 0.0, 60);
    assert!(cam.x >= 0.0 && cam.x < 100.0, "x wraps");
    let fit = Camera::fit(&map, 200, 300);
    assert_eq!(fit.zoom, 2.0);
    assert_eq!(fit.y, (50.0 - 150.0) / 2.0, "short map is centred vertically");
}

/// A synthetic day-1 save (original text) shaped like the game's: province blocks with
/// pops whose culture/religion is written as `<culture>=<religion>`.
const SAVE: &str = "date=\"1836.1.1\"\r\nplayer=\"AAA\"\r\nstart_date=\"1836.1.1\"\r\n\
1=\r\n{\r\n\tname=\"West Field\"\r\n\towner=\"AAA\"\r\n\tcontroller=\"AAA\"\r\n\tcore=\"AAA\"\r\n\
\tfarmers=\r\n\t{\r\n\t\tid=1\r\n\t\tsize=900\r\n\t\talpha=one\r\n\t\tmoney=12.50000\r\n\t\tissues=\r\n\t\t{\r\n1=2.00000\r\n\t\t}\r\n\t}\r\n\
\tclerks=\r\n\t{\r\n\t\tid=2\r\n\t\tsize=110\r\n\t\tbeta=two\r\n\t}\r\n}\r\n\
2=\r\n{\r\n\towner=\"AAA\"\r\n\tcontroller=\"AAA\"\r\n\tcore=\"BBB\"\r\n\
\tlabourers=\r\n\t{\r\n\t\tid=3\r\n\t\tsize=300\r\n\t\tbeta=two\r\n\t}\r\n}\r\n\
3=\r\n{\r\n}\r\n4=\r\n{\r\n}\r\nAAA=\r\n{\r\n\tmoney=1.00000\r\n}\r\n";

#[test]
fn save_reader_and_comparison() {
    let dir = install();
    let world = World::load(&dir, &EngineRules::from_json(ENGINE_RULES).unwrap()).unwrap();
    let pop_types = world.defs.pop_types.iter().map(|t| t.name.clone()).collect();
    let cultures = world.defs.cultures.cultures.iter().map(|c| c.name.clone()).collect();
    let save = wiktoria_data::save::parse_save(SAVE.as_bytes(), &pop_types, &cultures).unwrap();
    assert!(save.warnings.is_empty(), "{:?}", save.warnings);
    assert_eq!((save.player.as_deref(), save.countries.clone()), (Some("AAA"), vec!["AAA".to_string()]));
    let p1 = &save.provinces[&1];
    assert_eq!((p1.name.as_str(), p1.pops.len(), p1.pops[0].money), ("West Field", 2, 12.5));
    assert_eq!((p1.pops[1].culture.as_str(), p1.pops[1].religion.as_str()), ("beta", "two"));

    let r = wiktoria_data::compare::compare(&world, &save);
    assert_eq!(r.provinces_compared, 4);
    assert_eq!(r.owner, vec![(2, "BBB".to_string(), "AAA".to_string())]);
    assert!(r.controller.is_empty() && r.cores.is_empty());
    assert_eq!(r.type_swaps["farmers->labourers"].0, 1);
    assert_eq!(r.type_swaps["farmers->labourers"].1["clay"], 1);
    assert_eq!(r.size_changes["clerks"], vec![(1, 100.0, 110.0)]);
    assert_eq!(r.size_equal["farmers"], 1);
    assert!(r.only_ours.is_empty() && r.only_save.is_empty());
    assert_eq!((r.pops_ours, r.pops_save), ((3, 1300.0), (3, 1310.0)));
    std::fs::remove_dir_all(dir).ok();
}
