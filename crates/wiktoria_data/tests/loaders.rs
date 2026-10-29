//! Loader tests on original, synthetic fixtures. No Victoria 2 data is included.

use wiktoria_data::bmp::{self, Image};
use wiktoria_data::countries::{parse_country_file, parse_country_list};
use wiktoria_data::date::Date;
use wiktoria_data::defines::Defines;
use wiktoria_data::history::{province_id_from_filename, ProvinceHistory};
use wiktoria_data::localisation::Localisation;
use wiktoria_data::map::{parse_default_map, parse_definitions, ProvinceMap};
use wiktoria_data::script::parse;

#[test]
fn dates_parse_and_order() {
    assert_eq!(Date::parse("1836.1.1"), Some(Date::new(1836, 1, 1)));
    assert!(Date::parse("1836.13.1").is_none());
    assert!(Date::parse("owner").is_none());
    assert!(Date::new(1836, 1, 1) < Date::new(1836, 1, 2));
    assert_eq!(Date::new(1861, 1, 1).to_string(), "1861.1.1");
}

fn image(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 3]) -> Image {
    let mut rgb = Vec::new();
    for y in 0..h {
        for x in 0..w {
            rgb.extend_from_slice(&f(x, y));
        }
    }
    Image { width: w, height: h, rgb }
}

#[test]
fn bmp_round_trip_and_flip() {
    // Width 3 forces row padding (9 bytes -> 12).
    let img = image(3, 2, |x, y| [x as u8 * 10, y as u8 * 100, 7]);
    let back = bmp::decode(&bmp::encode(&img)).unwrap();
    assert_eq!(back.rgb, img.rgb);
    let mut flipped = back.clone();
    flipped.flip_vertical();
    assert_eq!(flipped.pixel(2, 0), img.pixel(2, 1));
    assert_eq!(flipped.pixel(0, 1), img.pixel(0, 0));
    assert!(bmp::decode(b"XX").is_err());
}

#[test]
fn definitions_skip_unused_rows_and_read_atoi_style() {
    let csv = "province;red;green;blue;x;x\n1;10;20;30;Alpha;x\n;5;5;5;x;\n2;40;50;60.;Beta;x\n";
    let (defs, warnings) = parse_definitions(csv).unwrap();
    assert_eq!(defs.len(), 2);
    assert_eq!(defs[1].rgb, [40, 50, 60]);
    assert_eq!(defs[1].name, "Beta");
    assert!(warnings.iter().any(|w| w.contains("read as 60")));
    assert!(warnings.iter().any(|w| w.contains("1 rows without a province id")));
    assert!(parse_definitions("1;1;1;1;A;x\n1;2;2;2;B;x\n").is_err(), "duplicate id");
    assert!(parse_definitions("1;1;x;1;A;x\n").is_err(), "bad channel");
}

#[test]
fn default_map_and_province_map() {
    let dm = parse_default_map(
        "max_provinces = 4\nsea_starts = { 3 3 }\ndefinitions = \"d.csv\"\nprovinces = \"p.bmp\"\nregion = \"r.txt\"",
    )
    .unwrap();
    assert_eq!(dm.region, "r.txt");
    assert!(parse_default_map("definitions = \"d.csv\"\nprovinces = \"p.bmp\"").is_err(), "region is required");
    assert_eq!((dm.max_provinces, dm.sea_starts.clone(), dm.provinces.as_str()), (4, vec![3, 3], "p.bmp"));
    let (defs, _) = parse_definitions("1;255;0;0;A;x\n2;0;255;0;B;x\n3;0;0;255;C;x\n").unwrap();
    // left column red (1), right column green (2), one stray pixel not in the csv
    let img = image(2, 3, |x, y| if (x, y) == (1, 2) { [9, 9, 9] } else if x == 0 { [255, 0, 0] } else { [0, 255, 0] });
    let (map, warnings) = ProvinceMap::build(&img, &defs);
    assert_eq!(map.id_at(0, 0), 1);
    assert_eq!(map.id_at(1, 1), 2);
    assert_eq!(map.id_at(1, 2), 0);
    assert_eq!(map.pixel_count[1], 3);
    assert_eq!(map.centroid[2], (1.0, 0.5));
    let centres = map.bounds_centres();
    assert_eq!((centres[1], centres[2], centres[3]), ((0.0, 1.0), (1.0, 0.5), (0.0, 0.0)), "middle of the bounding box; none without pixels");
    assert!(warnings.iter().any(|w| w.contains("1 pixels in 1 colours") && w.contains("rgb(9,9,9)")));
    assert!(warnings.iter().any(|w| w.contains("1 defined provinces have no pixels")));
}

#[test]
fn country_list_marks_dynamic_tags() {
    let (list, warnings) =
        parse_country_list("AAA = \"countries/A.txt\"\ndynamic_tags = yes\nBBB = \"countries/B.txt\"\nAAA = \"x\"").unwrap();
    assert_eq!(list.len(), 2);
    assert!(!list[0].dynamic && list[1].dynamic);
    assert!(warnings.iter().any(|w| w.contains("duplicate tag AAA")));
    let c = parse_country_file(&list[0], "color = { 1 2 3 }\ngraphical_culture = TestGC").unwrap();
    assert_eq!((c.color, c.graphical_culture.as_deref()), ([1, 2, 3], Some("TestGC")));
    assert!(parse_country_file(&list[0], "color = { 1 2 }").is_err());
}

#[test]
fn history_applies_dated_blocks_up_to_the_date() {
    let doc = parse(
        "owner = AAA\ncontroller = AAA\nadd_core = AAA\ntrade_goods = clay\nlife_rating = 30\n\
         1900.1.1 = { owner = CCC }\n1850.1.1 = { owner = BBB add_core = BBB remove_core = AAA }\nmystery = 1",
    )
    .unwrap();
    let at = |d: Date| {
        let mut h = ProvinceHistory::default();
        let mut w = Vec::new();
        h.apply(&doc, d, &mut w, "test");
        (h, w)
    };
    let (h1836, w) = at(Date::new(1836, 1, 1));
    assert_eq!((h1836.owner.as_deref(), h1836.cores.clone()), (Some("AAA"), vec!["AAA".to_string()]));
    assert_eq!((h1836.trade_goods.as_deref(), h1836.life_rating), (Some("clay"), Some(30.0)));
    assert!(w.iter().any(|w| w.contains("mystery")));
    let (h1850, _) = at(Date::new(1850, 1, 1));
    assert_eq!((h1850.owner.as_deref(), h1850.cores.clone()), (Some("BBB"), vec!["BBB".to_string()]));
    let (h1950, _) = at(Date::new(1950, 1, 1));
    assert_eq!(h1950.owner.as_deref(), Some("CCC"));
    assert_eq!(province_id_from_filename("300 - Somewhere.txt"), Some(300));
    assert_eq!(province_id_from_filename("readme.txt"), None);
}

#[test]
fn localisation_first_definition_wins() {
    let mut l = Localisation::default();
    l.add_file("#comment\nKEY1;First;Fr;De;;Es;x\n;empty key\n");
    l.add_file("KEY1;Second;x\nKEY2;Other;x\n");
    assert_eq!(l.get("KEY1"), Some("First"));
    assert_eq!(l.get("KEY2"), Some("Other"));
    assert_eq!(l.len(), 2);
}

#[test]
fn defines_lua_subset() {
    let src = "defines = {\n\nstart_date = '1836.1.1',\nend_date = '1935.12.31',\n\neconomy = {\n\
               \tSOME_RATE = 0.25, -- a comment with = signs, and commas\n\tNEGATIVE = -3,\n},\n}\n";
    let d = Defines::parse(src).unwrap();
    assert_eq!(d.start_date().unwrap(), Date::new(1836, 1, 1));
    assert_eq!(d.end_date().unwrap(), Date::new(1935, 12, 31));
    assert_eq!(d.get("economy", "SOME_RATE"), Some(0.25));
    assert_eq!(d.get("economy", "NEGATIVE"), Some(-3.0));
    assert_eq!(d.sections(), vec![("economy", 2)]);
    assert!(Defines::parse("other = { }").is_err());
}

#[test]
fn river_pixels_are_counted_on_the_border_they_lie_on() {
    use wiktoria_data::bmp::IndexedImage;
    use wiktoria_data::map::ProvinceMap;
    // Three provinces side by side, 2 columns each and 3 rows; 9 = no river.
    let ids = vec![1, 1, 2, 2, 3, 3, 1, 1, 2, 2, 3, 3, 1, 1, 2, 2, 3, 3];
    let map = ProvinceMap { width: 6, height: 3, ids, pixel_count: vec![0, 6, 6, 6], centroid: vec![(0.0, 0.0); 4] };
    // A river down the last column of province 1 (two of three rows), none between 2 and 3.
    let index = vec![9, 2, 9, 9, 9, 9, 9, 2, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9];
    let img = IndexedImage { width: 6, height: 3, index };
    let r = wiktoria_data::geography::river_borders(&map, &img, 5).unwrap();
    assert_eq!(r.get(&(1, 2)), Some(&2));
    assert_eq!(r.get(&(2, 3)), None);
    // The map wraps east to west: provinces 3 and 1 meet there, without a river.
    assert_eq!(r.len(), 1);
    // A picture of another size is refused.
    assert!(wiktoria_data::geography::river_borders(&map, &IndexedImage { width: 3, height: 3, index: vec![9; 9] }, 5).is_err());
}
