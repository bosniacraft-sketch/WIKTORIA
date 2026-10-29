//! All fixtures here are original, synthetic text written for these tests.
//! No Victoria 2 files are included in this repository.

use wiktoria_data::goods::load_goods;
use wiktoria_data::engine_rules::{parse_json, Json, EngineRules};
use wiktoria_data::script::{decode_windows1252, parse, parse_bytes, Value};

const SYNTHETIC_GOODS: &str = r#"
# synthetic categories and goods, not game data
test_raw = {
    clay = {
        cost = 1.5
        color = { 10 20 30 }
    }
    gold_dust = { cost = 9 color = { 250 200 0 } money = yes }
}
test_made = {
    widgets = {
        cost = 12.25
        color = { 1 2 3 }
        available_from_start = no
        overseas_penalty = yes
        tradeable = no
    }
}
"#;

#[test]
fn parses_key_values_blocks_and_bare_values() {
    let doc = parse("a = 1 b = { x = yes 4 5 } c = \"quoted text\" # comment\n d = { }").unwrap();
    assert!(doc.warnings.is_empty());
    let r = &doc.root;
    assert_eq!(r.get("a").and_then(Value::as_f64), Some(1.0));
    let b = r.get("b").and_then(Value::as_block).unwrap();
    assert_eq!(b.get("x").and_then(Value::as_bool), Some(true));
    assert_eq!(b.bare_scalars(), vec!["4", "5"]);
    assert_eq!(r.get("c").and_then(Value::as_str), Some("quoted text"));
    assert!(r.get("d").and_then(Value::as_block).unwrap().items.is_empty());
}

#[test]
fn later_assignment_wins_and_get_all_sees_repeats() {
    let doc = parse("k = 1 k = 2").unwrap();
    assert_eq!(doc.root.get("k").and_then(Value::as_f64), Some(2.0));
    assert_eq!(doc.root.get_all("k").count(), 2);
}

#[test]
fn stray_top_level_brace_is_a_warning() {
    let doc = parse("a = 1 }\nb = 2").unwrap();
    assert_eq!(doc.warnings.len(), 1);
    assert!(doc.warnings[0].starts_with("line 1"));
    assert_eq!(doc.root.get("b").and_then(Value::as_f64), Some(2.0));
}

#[test]
fn unclosed_brace_reports_opening_line() {
    let err = parse("ok = 1\nbad = {\n x = 1\n").unwrap_err();
    assert_eq!(err.line, 2);
}

#[test]
fn missing_value_is_an_error() {
    assert!(parse("a = ").is_err());
    assert!(parse("= 3").is_err());
}

#[test]
fn windows1252_decoding() {
    // "Württemberg" in CP1252, plus the euro sign from the 0x80 block.
    let bytes = b"W\xfcrttemberg \x80";
    assert_eq!(decode_windows1252(bytes), "Württemberg €");
    let doc = parse_bytes(b"name = \"K\xf6ln\"").unwrap();
    assert_eq!(doc.root.get("name").and_then(Value::as_str), Some("Köln"));
}

#[test]
fn loads_synthetic_goods_with_defaults() {
    let doc = parse(SYNTHETIC_GOODS).unwrap();
    let loaded = load_goods(&doc).unwrap();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    assert_eq!(loaded.goods.len(), 3);

    let clay = &loaded.goods[0];
    assert_eq!((clay.id.as_str(), clay.category.as_str(), clay.cost), ("clay", "test_raw", 1.5));
    assert_eq!(clay.color, [10, 20, 30]);
    assert!(clay.available_from_start && clay.tradeable && !clay.money && !clay.overseas_penalty);

    assert!(loaded.goods[1].money);

    let w = &loaded.goods[2];
    assert!(!w.available_from_start && !w.tradeable && w.overseas_penalty);
    assert_eq!(w.cost, 12.25);
}

#[test]
fn goods_errors_and_warnings() {
    assert!(load_goods(&parse("c = { g = { color = { 1 2 3 } } }").unwrap()).is_err(), "missing cost");
    assert!(load_goods(&parse("c = { g = { cost = 1 color = { 1 2 } } }").unwrap()).is_err(), "bad color");
    assert!(load_goods(&parse("a = { g = { cost = 1 } } b = { g = { cost = 2 } }").unwrap()).is_err(), "duplicate id");
    let l = load_goods(&parse("c = { g = { cost = 1 color = { 1 2 3 } mystery = 4 } }").unwrap()).unwrap();
    assert!(l.warnings.iter().any(|w| w.contains("mystery")));
}

#[test]
fn json_reader_handles_escapes_and_numbers() {
    let j = parse_json(r#"{"a":[1,-2.5e1,true,null],"s":"x\"ü\n"}"#).unwrap();
    let a = j.get("a").and_then(Json::as_array).unwrap();
    assert_eq!(a[1].as_f64(), Some(-25.0));
    assert_eq!(j.get("s").and_then(Json::as_str), Some("x\"ü\n"));
    assert!(parse_json("{\"a\":1} x").is_err());
}

#[test]
fn engine_rules_file_has_the_fields_the_engine_needs() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/engine_rules.json");
    let src = std::fs::read_to_string(path).expect("data/engine_rules.json is missing");
    let m = EngineRules::from_json(&src).unwrap();
    assert_eq!(m.require_str("FLD_TEXT_ENCODING").unwrap(), "windows-1252");
    assert_eq!(m.require_str("FLD_GOODS_FILE").unwrap(), "common/goods.txt");
}
