//! Game setup pass on a small synthetic install (original test data, no game files).

use std::path::PathBuf;

use wiktoria_data::bmp::{self, Image};
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::setup::{self, SetupSteps};
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
  {"field_id": "FLD_PROD_SCALAR_KEYS", "value": "farm_rgo_size,mine_rgo_size,farm_rgo_eff,mine_rgo_eff,rgo_input,rgo_output,rgo_throughput"},
  {"field_id": "FLD_PROD_GOOD_KEYS", "value": "rgo_size,rgo_goods_input,rgo_goods_output,rgo_goods_throughput"},
  {"field_id": "FLD_SETUP_RGO_WORKER_TYPE", "value": true},
  {"field_id": "FLD_SETUP_SLAVE_CONVERSION", "value": true},
  {"field_id": "FLD_SETUP_STATE_CAPITAL_ONLY", "value": true},
  {"field_id": "FLD_SETUP_MERGE_IDENTICAL", "value": false}]}}"#;

/// Provinces: 1 AAA grain, 2 AAA coal (same region, listed first), 3 CCC grain, 4 unowned grain,
/// 5 BBB grain, 6 sea. AAA: civilized, slavery allowed. BBB: not civilized (no flag), slavery
/// allowed by default (first option). CCC: civilized, slavery banned.
fn install() -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("wiktoria-setup-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let w = |rel: &str, data: &[u8]| {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    };
    w("common/defines.lua", b"defines = {\nstart_date = '1836.1.1',\n}\n");
    w("common/countries.txt", b"AAA = \"countries/A.txt\"\nBBB = \"countries/B.txt\"\nCCC = \"countries/C.txt\"\n");
    for c in ["A", "B", "C"] {
        w(&format!("common/countries/{c}.txt"), b"color = { 1 2 3 }\n");
    }
    w("map/default.map", b"max_provinces = 7\nsea_starts = { 6 }\ndefinitions = \"definition.csv\"\nprovinces = \"provinces.bmp\"\nregion = \"region.txt\"\n");
    w("map/region.txt", b"R_ONE = { 2 1 }\nR_TWO = { 3 }\nR_THREE = { 4 5 }\n");
    w("common/cultures.txt", b"grp = { alpha = { color = { 1 1 1 } } beta = { color = { 2 2 2 } } }\n");
    w("common/religion.txt", b"faiths = { one = { icon = 1 color = { 1 1 0 } } two = { icon = 2 color = { 0 1 1 } } }\n");
    w("poptypes/farmers.txt", b"strata = poor\nequivalent = labourers\n");
    w("poptypes/labourers.txt", b"strata = poor\nequivalent = farmers\n");
    w("poptypes/slaves.txt", b"strata = poor\nis_slave = yes\n");
    w("poptypes/nobles.txt", b"strata = rich\nstate_capital_only = yes\n");
    w("poptypes/makers.txt", b"strata = poor\nmerge_max_size = 100\n");
    w("common/production_types.txt",
      b"field_t = { owner = { poptype = nobles } employees = { { poptype = farmers amount = 1 } { poptype = slaves amount = 1 } } type = rgo }\n\
        pit_t = { owner = { poptype = nobles } employees = { { poptype = labourers amount = 1 } { poptype = slaves amount = 1 } } type = rgo }\n\
        grain_field = { template = field_t output_goods = grain }\n\
        coal_pit = { template = pit_t output_goods = coal }\n\
        workshop = { employees = { { poptype = makers } } type = factory output_goods = tools }\n");
    w("common/static_modifiers.txt", b"base_values = { supply_limit = 5 }\n");
    w("common/ideologies.txt", b"old_group = { keepers = { } }\nnew_group = { changers = { } }\n");
    w("common/event_modifiers.txt", b"plague = { population_growth = -0.02 icon = 3 }\n");
    w("common/nationalvalues.txt", b"nv_test = { mobilisation_size = 0.04 }\n");
    w("common/crime.txt", b"bad_crime = { trigger = { } local_rgo_output = -0.2 }\n");
    w("common/issues.txt", b"social_reforms = { slavery = { yes_slavery = { rules = { slavery_allowed = yes } } no_slavery = { rules = { slavery_allowed = no } } } }\n");
    w("history/countries/AAA - A.txt", b"capital = 1\nprimary_culture = alpha\nreligion = one\ncivilized = yes\nslavery = yes_slavery\nliteracy = 0.6\n");
    w("history/countries/BBB - B.txt", b"capital = 5\nprimary_culture = alpha\nreligion = one\n");
    w("history/countries/CCC - C.txt", b"capital = 3\nprimary_culture = beta\nculture = alpha\nreligion = one\ncivilized = yes\nslavery = no_slavery\nliteracy = 1.4\nnon_state_culture_literacy = 0.25\n");
    w("common/technology.txt", b"folders = { }\n");
    w("inventions/none.txt", b"");
    w("localisation/none.csv", b"");
    w("common/buildings.txt", b"");
    w("common/bookmarks.txt", b"");
    w("history/pops/1836.1.1/test.txt",
      b"1 = { farmers = { culture = alpha religion = one size = 100 }\n\
              nobles = { culture = alpha religion = one size = 10 }\n\
              slaves = { culture = beta religion = two size = 50 }\n\
              makers = { culture = alpha religion = one size = 60 }\n\
              makers = { culture = alpha religion = one size = 30 }\n\
              makers = { culture = alpha religion = one size = 50 } }\n\
        2 = { farmers = { culture = alpha religion = one size = 200 }\n\
              labourers = { culture = alpha religion = one size = 5 }\n\
              nobles = { culture = alpha religion = one size = 20 }\n\
              nobles = { culture = beta religion = one size = 3 } }\n\
        3 = { slaves = { culture = alpha religion = one size = 40 } }\n\
        4 = { slaves = { culture = alpha religion = one size = 7 } }\n\
        5 = { slaves = { culture = alpha religion = one size = 9 } }\n");
    let colors: [[u8; 3]; 6] = [[10, 0, 0], [20, 0, 0], [30, 0, 0], [40, 0, 0], [50, 0, 0], [60, 0, 0]];
    let mut csv = String::from("province;red;green;blue;x;x\n");
    for (i, c) in colors.iter().enumerate() {
        csv.push_str(&format!("{};{};0;0;P{};x\n", i + 1, c[0], i + 1));
    }
    w("map/definition.csv", csv.as_bytes());
    w("map/provinces.bmp", &bmp::encode(&Image { width: 6, height: 1, rgb: colors.concat() }));
    for (id, owner, goods) in [(1, Some("AAA"), "grain"), (2, Some("AAA"), "coal"), (3, Some("CCC"), "grain"), (4, None, "grain"), (5, Some("BBB"), "grain")] {
        let own = owner.map(|o| format!("owner = {o}\ncontroller = {o}\n")).unwrap_or_default();
        w(&format!("history/provinces/test/{id} - P.txt"), format!("{own}trade_goods = {goods}\n").as_bytes());
    }
    dir
}

/// (type, culture, religion, size) of a province's pops, in order.
fn pops(world: &World, id: u16) -> Vec<(String, String, String, f64)> {
    world
        .province(id)
        .unwrap()
        .pops
        .iter()
        .map(|p| {
            (
                world.defs.pop_types[p.pop_type as usize].name.clone(),
                world.defs.cultures.cultures[p.culture as usize].name.clone(),
                world.defs.religions.religions[p.religion as usize].name.clone(),
                p.size,
            )
        })
        .collect()
}

fn p(t: &str, c: &str, r: &str, s: f64) -> (String, String, String, f64) {
    (t.into(), c.into(), r.into(), s)
}

#[test]
fn reads_definitions_used_by_setup() {
    let world = World::load(&install(), &EngineRules::from_json(ENGINE_RULES).unwrap()).unwrap();
    assert!(world.warnings.is_empty(), "{:#?}", world.warnings);
    let t = |n: &str| &world.defs.pop_types[world.defs.pop_type_index.get(n).unwrap() as usize];
    assert_eq!(t("farmers").equivalent.as_deref(), Some("labourers"));
    assert!(t("slaves").is_slave && t("nobles").state_capital_only);
    assert_eq!(t("makers").merge_max_size, Some(100.0));
    let names = |good: &str| -> Vec<String> {
        world.defs.rgo_employees[good].iter().map(|i| world.defs.pop_types[*i as usize].name.clone()).collect()
    };
    assert_eq!(names("grain"), ["farmers", "slaves"], "employees come from the template");
    assert_eq!(names("coal"), ["labourers", "slaves"]);
    assert!(!world.defs.rgo_employees.contains_key("tools"), "factories are not RGOs");
    let tag = |t: &str| *world.tag_index.get(t).unwrap();
    assert_eq!(setup::country_rule(&world, tag("AAA"), "slavery_allowed"), Some(true));
    assert_eq!(setup::country_rule(&world, tag("BBB"), "slavery_allowed"), Some(true), "first option by default");
    assert_eq!(setup::country_rule(&world, tag("CCC"), "slavery_allowed"), Some(false));
}

#[test]
fn applies_all_setup_steps() {
    let engine_rules = EngineRules::from_json(ENGINE_RULES).unwrap();
    let mut world = World::load(&install(), &engine_rules).unwrap();
    let steps = SetupSteps::from_engine_rules(&engine_rules).unwrap();
    assert!(!steps.merge_identical, "switches come from the rules file");
    let r = setup::apply(&mut world, SetupSteps::ALL);

    // 1: slaves kept (civilized, allowed); nobles left for the state capital; makers 60+30 merge,
    //    50 more would pass merge_max_size 100.
    assert_eq!(pops(&world, 1), [p("farmers", "alpha", "one", 100.0), p("slaves", "beta", "two", 50.0), p("makers", "alpha", "one", 90.0), p("makers", "alpha", "one", 50.0)]);
    // 2: first province of the region = state capital; farmers on a coal pit become labourers and
    //    merge with the labourers; nobles from province 1 merge per culture and religion.
    assert_eq!(pops(&world, 2), [p("labourers", "alpha", "one", 205.0), p("nobles", "alpha", "one", 30.0), p("nobles", "beta", "one", 3.0)]);
    assert_eq!(pops(&world, 3), [p("farmers", "alpha", "one", 40.0)], "slavery banned");
    assert_eq!(pops(&world, 4), [p("slaves", "alpha", "one", 7.0)], "unowned: kept");
    assert_eq!(pops(&world, 5), [p("farmers", "alpha", "one", 9.0)], "not civilized");
    assert_eq!((r.type_changes, r.slaves_converted, r.slaves_kept, r.moved_to_capital, r.merged), (1, 2, 2, 1, 3));
    assert_eq!((r.pops_before, r.pops_after), (13, 10));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn steps_can_be_switched_off() {
    let mut world = World::load(&install(), &EngineRules::from_json(ENGINE_RULES).unwrap()).unwrap();
    let before = pops(&world, 1);
    let r = setup::apply(&mut world, SetupSteps::NONE);
    assert_eq!(pops(&world, 1), before);
    assert_eq!(r.pops_before, r.pops_after);
}

/// Starting literacy: by culture and owner, never by pop type.
#[test]
fn starting_literacy_follows_culture() {
    let engine_rules = EngineRules::from_json(ENGINE_RULES).unwrap();
    let mut world = World::load(&install(), &engine_rules).unwrap();
    let rules = setup::LiteracyRules { enabled: true, non_state_default: 0.1, max: 1.0 };
    let set = setup::starting_literacy(&mut world, &rules);
    let lit = |id: u16| -> Vec<(String, f64)> {
        world.province(id).unwrap().pops.iter().map(|p| (world.defs.cultures.cultures[p.culture as usize].name.clone(), p.literacy)).collect()
    };
    // AAA: literacy 0.6, primary culture alpha, no value for other cultures -> the default.
    assert!(lit(1).iter().all(|(c, l)| *l == if c == "alpha" { 0.6 } else { 0.1 }), "{:?}", lit(1));
    assert!(lit(2).iter().any(|(c, l)| c == "beta" && *l == 0.1) && lit(2).iter().any(|(c, l)| c == "alpha" && *l == 0.6));
    // CCC: alpha is an accepted culture; its literacy 1.4 is kept within 100%.
    assert_eq!(lit(3), vec![("alpha".to_string(), 1.0)]);
    // BBB gives no literacy; province 4 has no owner and is left alone.
    assert_eq!(lit(5), vec![("alpha".to_string(), 0.0)]);
    assert_eq!(lit(4), vec![("alpha".to_string(), 0.0)]);
    assert_eq!(set, world.provinces.iter().flatten().filter(|p| p.owner.is_some()).map(|p| p.pops.len()).sum::<usize>());
    let off = setup::LiteracyRules { enabled: false, ..rules };
    assert_eq!(setup::starting_literacy(&mut world, &off), 0);
}
