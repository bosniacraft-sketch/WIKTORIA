//! Phase 2 parsers on original, synthetic fixtures.

use std::collections::{HashMap, HashSet};

use wiktoria_data::country_history::{tag_from_filename, Classifier, CountryHistory};
use wiktoria_data::date::Date;
use wiktoria_data::defs;
use wiktoria_data::map::parse_regions;
use wiktoria_data::pops::{parse_pop_file, pick_pop_folder};
use wiktoria_data::script::parse;

#[test]
fn cultures_skip_group_settings_and_index_cultures() {
    let c = defs::parse_cultures(
        "north = {\n leader = european\n unit = EuropeanGC\n union = NOR\n is_overseas = no\n\
         alpha = { color = { 10 20 30 } first_names = { Ann \"Bo B\" } radicalism = 5 }\n beta = { color = { 1 2 3 } }\n}\n\
         south = { gamma = { color = { 4 5 6 } } }",
    )
    .unwrap();
    assert_eq!(c.cultures.len(), 3);
    assert_eq!(c.groups[0].union.as_deref(), Some("NOR"));
    assert_eq!((c.cultures[0].color, c.cultures[0].radicalism), ([10, 20, 30], 5.0));
    assert_eq!(c.index.get("gamma"), Some(2));
    assert_eq!(c.cultures[2].group, 1);
    assert!(defs::parse_cultures("g = { a = { } }\nh = { a = { } }").is_err(), "duplicate culture");
}

#[test]
fn religions_use_unit_colours() {
    let r = defs::parse_religions("faiths = { one = { icon = 3 color = { 1 0 0.5 } } }").unwrap();
    assert_eq!((r.religions[0].color, r.religions[0].icon, r.religions[0].group.as_str()), ([255, 0, 128], 3, "faiths"));
}

#[test]
fn pop_types_issues_techs_buildings_bookmarks() {
    let pt = defs::parse_pop_type("diggers", "sprite = 4\ncolor = { 1 2 3 }\nstrata = middle\nlife_needs = { clay = 1 }").unwrap();
    assert_eq!((pt.sprite, pt.strata.as_str(), pt.color), (4, "middle", [1, 2, 3]));

    let issues = defs::parse_issues(
        "party_issues = { trade = { open = { } closed = { } } }\n\
         political_reforms = { voting = { next_step_only = yes none = { } some = { } all = { } } }",
    )
    .unwrap();
    assert_eq!(issues.len(), 2);
    assert_eq!(issues[1].options, vec!["none", "some", "all"]);
    assert!(issues[1].next_step_only && issues[1].category == "political_reforms");

    assert_eq!(defs::parse_tech_folders("folders = { a_tech = { x y } b_tech = { z } }").unwrap(), vec!["a_tech", "b_tech"]);
    let techs = defs::parse_technologies("t1 = { area = x year = 1836 cost = 100 }\nt2 = { }", "t.txt").unwrap();
    assert_eq!((techs[0].year, techs[0].cost, techs[1].area.as_str()), (1836, 100.0, ""));
    assert_eq!(defs::parse_inventions("i1 = { limit = { t1 = 1 } }\nloose = 1", "i.txt").unwrap(), vec!["i1"]);

    let b = defs::parse_buildings("mill = { type = factory max_level = 99 }\nwall = { type = fort }").unwrap();
    assert_eq!((b[0].kind.as_str(), b[0].max_level, b[1].max_level), ("factory", 99, 0));

    let bm = defs::parse_bookmarks("bookmark = { name = \"A\" date = 1836.1.1 cameraX = 10 cameraY = 20 }\nbookmark = { date = 1900.1.1 }").unwrap();
    assert_eq!((bm[0].date, bm[0].camera, bm[1].name.as_str()), (Date::new(1836, 1, 1), (10.0, 20.0), ""));
}

#[test]
fn pops_parse_and_folder_choice() {
    let doc = parse(
        "7 = { diggers = { culture = alpha religion = one size = 100 } diggers = { culture = alpha religion = one size = 50 militancy = 2 rebel_type = angry } }\n\
         8 = { diggers = { culture = alpha size = 1 } }\nnope = { }",
    )
    .unwrap();
    let mut out = HashMap::new();
    let mut warnings = Vec::new();
    parse_pop_file(&doc, "t", &mut out, &mut warnings);
    assert_eq!(out[&7].len(), 2, "identical pops stay separate");
    assert_eq!((out[&7][1].militancy, out[&7][1].rebel_type.as_deref()), (2.0, Some("angry")));
    assert!(out[&8].is_empty());
    assert_eq!(warnings.len(), 2, "{warnings:?}");

    let folders = vec!["1836.1.1".to_string(), "1861.4.14".to_string(), "notes".to_string()];
    assert_eq!(pick_pop_folder(&folders, Date::new(1836, 1, 1)), Some("1836.1.1"));
    assert_eq!(pick_pop_folder(&folders, Date::new(1861, 7, 1)), Some("1861.4.14"));
    assert_eq!(pick_pop_folder(&folders, Date::new(1800, 1, 1)), None);
}

#[test]
fn regions_report_overlaps() {
    let (r, w) = parse_regions("R_1 = { 1 2 }\nR_2 = { 3 2 }\nR_3 = { x }").unwrap();
    assert_eq!(r[1], ("R_2".to_string(), vec![3, 2]));
    assert!(w.iter().any(|w| w.contains("province 2 is in both R_1 and R_2")));
    assert!(w.iter().any(|w| w.contains("bad province id 'x'")));
}

#[test]
fn country_history_classifies_keys_and_applies_dates() {
    let cls = Classifier {
        issue_groups: HashMap::from([("voting".to_string(), vec!["none".to_string(), "all".to_string()])]),
        technologies: HashSet::from(["t1".to_string(), "t2".to_string()]),
        inventions: HashSet::from(["i1".to_string()]),
    };
    let doc = parse(
        "capital = 7\nprimary_culture = alpha\nculture = beta\nreligion = one\ngovernment = monarchy\nliteracy = 0.5\n\
         civilized = yes\nvoting = none\nt1 = 1\ni1 = yes\nupper_house = { left = 40 right = 60 }\n\
         foreign_investment = { BBB = 100 }\ngovt_flag = { government = republic flag = other }\n\
         set_country_flag = some_flag\ncolonial_points = 5\ndecision = do_it\n\
         1850.1.1 = { voting = all t2 = 1 t1 = 0 }\n1840.1.1 = { voting = bogus }\nmystery = 1",
    )
    .unwrap();
    let at = |d: Date| {
        let mut h = CountryHistory::default();
        let mut w = Vec::new();
        h.apply(&doc, d, &cls, &mut w, "t");
        (h, w)
    };
    let (h, w) = at(Date::new(1836, 1, 1));
    assert_eq!((h.capital, h.primary_culture.as_deref(), h.cultures.clone()), (Some(7), Some("alpha"), vec!["beta".to_string()]));
    assert_eq!(h.reforms["voting"], "none");
    assert!(h.technologies.contains("t1") && h.inventions.contains("i1"));
    assert_eq!(h.upper_house["right"], 60.0);
    assert_eq!((h.foreign_investment["BBB"], h.colonial_points), (100.0, Some(5.0)));
    assert_eq!(h.govt_flags, vec![("republic".to_string(), "other".to_string())]);
    assert!(h.country_flags.contains("some_flag") && h.decisions == ["do_it"]);
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("mystery"));

    let (h, w) = at(Date::new(1860, 1, 1));
    assert_eq!(h.reforms["voting"], "all");
    assert!(h.technologies.contains("t2") && !h.technologies.contains("t1"));
    assert!(w.iter().any(|w| w.contains("voting = Some(\"bogus\")")));

    assert_eq!(tag_from_filename("ENG - Some Name.txt").as_deref(), Some("ENG"));
    assert_eq!(tag_from_filename("readme.txt"), None);
}
