//! Country modifier aggregation on a synthetic library (no game files).

use std::collections::BTreeMap;

use wiktoria_data::defs::IssueGroup;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::modifiers::{ModifierLibrary, ModifierValues};
use wiktoria_simulation::modifiers::{country_totals, CountryModifierInputs, CountryModifierRules, Rank};

fn engine_rules() -> EngineRules {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/engine_rules.json");
    EngineRules::from_json(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn mv(pairs: &[(&str, f64)]) -> ModifierValues {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn group(cat: &str, name: &str, opts: &[&str]) -> IssueGroup {
    IssueGroup { category: cat.into(), name: name.into(), options: opts.iter().map(|s| s.to_string()).collect(), rules: vec![BTreeMap::new(); opts.len()], next_step_only: false }
}

fn library() -> (ModifierLibrary, Vec<IssueGroup>) {
    let mut lib = ModifierLibrary::default();
    for (k, v) in [
        ("base_values", mv(&[("supply_limit", 5.0)])),
        ("peace", mv(&[("war_exhaustion", -1.0)])),
        ("war", mv(&[("max_war_exhaustion", 100.0)])),
        ("great_power", mv(&[("prestige", 0.05)])),
        ("second_power", mv(&[("prestige", 0.025)])),
        ("civ_nation", mv(&[("prestige", 0.01)])),
        ("unciv_nation", mv(&[("cb_generation_speed_modifier", 0.8)])),
        ("plurality", mv(&[("research_points_modifier", 0.02)])),
        ("average_literacy", mv(&[("research_points", 4.0), ("self_unciv_economic_modifier", -0.5)])),
    ] {
        lib.static_modifiers.insert(k.into(), v);
    }
    lib.national_values.insert("nv_order".into(), mv(&[("mobilisation_size", 0.04)]));
    lib.schools.insert("naval".into(), mv(&[("navy_tech_research_bonus", 0.15)]));
    for (k, v) in [
        ("none_voting", mv(&[("rich_vote", 0.0)])),
        ("landed_voting", mv(&[("rich_vote", 1.0)])),
        ("no_land_reform", mv(&[("farm_rgo_size", 0.25)])),
        ("land_reform", mv(&[("farm_rgo_size", 0.5)])),
        ("laissez_faire", mv(&[("factory_owner_cost", 0.3)])),
    ] {
        lib.issue_options.insert(k.into(), v);
    }
    lib.inventions = vec![("paternalism".into(), mv(&[("org_regain", 0.05)])), ("other".into(), ModifierValues::new())];
    let issues = vec![
        group("party_issues", "economic_policy", &["laissez_faire"]),
        group("political_reforms", "vote_franschise", &["none_voting", "landed_voting"]),
        group("economic_reforms", "land_reform", &["no_land_reform", "land_reform"]),
    ];
    (lib, issues)
}

#[test]
fn civilized_great_power() {
    let (lib, issues) = library();
    let rules = CountryModifierRules::from_engine_rules(&engine_rules()).unwrap();
    let input = CountryModifierInputs {
        rank: Some(Rank::GreatPower),
        plurality: 30.029,
        literacy: 0.432,
        national_value: Some("nv_order".into()),
        school: Some("naval".into()),
        reforms: [("vote_franschise".to_string(), "landed_voting".to_string())].into(),
        party_policies: vec!["laissez_faire".into()],
        inventions: vec!["paternalism".into()],
        ..Default::default()
    };
    let t = country_totals(&lib, &issues, &rules, &input);
    let get = |k: &str| t.get(k).copied().unwrap_or(0);
    assert_eq!((get("supply_limit"), get("war_exhaustion")), (5000, -1000), "base + peace");
    assert_eq!(get("prestige"), 50, "only the great power block");
    assert_eq!(get("research_points_modifier"), 600, "20 x 30.029 truncated");
    assert_eq!((get("research_points"), get("self_unciv_economic_modifier")), (1728, -216));
    assert_eq!((get("mobilisation_size"), get("navy_tech_research_bonus")), (40, 150));
    assert_eq!(get("rich_vote"), 1000, "civilized: political reform");
    assert_eq!(get("farm_rgo_size"), 0, "civilized: no unciv reforms");
    assert_eq!((get("factory_owner_cost"), get("org_regain")), (300, 50), "party policy and invention");
    assert!(!t.contains_key("cb_generation_speed_modifier"));
}

#[test]
fn uncivilized_uses_unciv_reforms_with_defaults() {
    let (lib, issues) = library();
    let rules = CountryModifierRules::from_engine_rules(&engine_rules()).unwrap();
    let mut input = CountryModifierInputs { rank: Some(Rank::Uncivilized), at_war: true, ..Default::default() };
    let t = country_totals(&lib, &issues, &rules, &input);
    assert_eq!(t.get("farm_rgo_size"), Some(&250), "first option when none is chosen");
    assert_eq!(t.get("rich_vote"), None, "no political reforms");
    assert_eq!(t.get("cb_generation_speed_modifier"), Some(&800));
    assert_eq!((t.get("max_war_exhaustion"), t.get("war_exhaustion")), (Some(&100_000), None), "war block instead of peace");
    input.reforms.insert("land_reform".into(), "land_reform".into());
    assert_eq!(country_totals(&lib, &issues, &rules, &input).get("farm_rgo_size"), Some(&500));
}

/// Timed modifiers: each one named on a country or province adds its block, and
/// the same name twice counts twice. An unknown name adds nothing.
#[test]
fn timed_modifiers_add_their_blocks() {
    use wiktoria_simulation::modifiers::{province_totals, ProvinceModifierInputs, ProvinceModifierRules, ProvinceSources};
    let (mut lib, issues) = library();
    lib.event_modifiers.insert("plague".into(), mv(&[("population_growth", -0.02), ("pop_militancy_modifier", 0.01)]));
    lib.event_modifiers.insert("boom".into(), mv(&[("population_growth", 0.005)]));
    let m = engine_rules();

    let rules = CountryModifierRules::from_engine_rules(&m).unwrap();
    let plain = country_totals(&lib, &issues, &rules, &CountryModifierInputs::default());
    let input = CountryModifierInputs { event_modifiers: vec!["plague".into(), "boom".into(), "unknown".into()], ..Default::default() };
    let with = country_totals(&lib, &issues, &rules, &input);
    let delta = |k: &str| with.get(k).copied().unwrap_or(0) - plain.get(k).copied().unwrap_or(0);
    assert_eq!((delta("population_growth"), delta("pop_militancy_modifier"), delta("supply_limit")), (-15, 10, 0));

    let prules = ProvinceModifierRules::from_engine_rules(&m).unwrap();
    let (terrain, groups) = (Default::default(), Default::default());
    let src = ProvinceSources { statics: &lib.static_modifiers, terrain: &terrain, climates: &groups, continents: &groups, crimes: &[], buildings: &[], events: &lib.event_modifiers };
    let none = province_totals(&src, &prules, &ProvinceModifierInputs::default());
    let input = ProvinceModifierInputs { event_modifiers: vec!["plague".into(), "plague".into()], ..Default::default() };
    let got = province_totals(&src, &prules, &input);
    assert_eq!(got.get("population_growth").copied().unwrap_or(0) - none.get("population_growth").copied().unwrap_or(0), -40);
    assert_eq!(got.get("pop_militancy_modifier").copied(), Some(20));
}
