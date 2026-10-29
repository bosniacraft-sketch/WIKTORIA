//! Script effects on a small synthetic state (no game files).

use std::collections::BTreeMap;

use wiktoria_data::date::Date;
use wiktoria_data::script;
use wiktoria_simulation::script::{activations, conditions_hold, run_effects, unknown_conditions, CountryState, EffectRules, FactoryState, PopState, ProvinceState, Report, Scope, State, TimedModifier};
use wiktoria_simulation::Calendar;

fn state() -> State {
    let pop = |c: f64, m: f64| PopState { pop_type: "farmers".into(), size: 1000.0, consciousness: c, militancy: m, ..Default::default() };
    let prov = |owner: Option<&str>, pops: Vec<PopState>| ProvinceState { owner: owner.map(str::to_string), controller: owner.map(str::to_string), cores: owner.iter().map(|o| o.to_string()).collect(), pops, ..Default::default() };
    let provinces = BTreeMap::from([
        (1, prov(Some("AAA"), vec![pop(2.0, 0.5)])),
        (2, prov(Some("BBB"), vec![pop(3.0, 1.0), pop(9.5, 9.0)])),
        (3, prov(Some("BBB"), vec![])),
        (4, prov(Some("CCC"), vec![pop(0.0, 0.0)])),
        (5, prov(None, vec![pop(1.0, 1.0)])),
    ]);
    let country = |prestige: f64, money: f64| CountryState { prestige, money, capital: Some(2), primary_culture: Some("alpha".into()), accepted_cultures: vec!["beta".into()], civilized: prestige >= 50.0, ..Default::default() };
    // DDD exists as a tag but owns nothing.
    let countries = BTreeMap::from([("AAA".to_string(), country(70.0, 500.0)), ("BBB".into(), country(100.0, 20000.0)), ("CCC".into(), country(30.0, 800.0)), ("DDD".into(), country(5.0, 5.0))]);
    State { provinces, countries, slave_types: ["slaves".to_string()].into(), ..State::new(Date::new(1836, 1, 1)) }
}

fn run(text: &str, scope: Scope) -> (State, Report) {
    run_on(state(), text, scope)
}

fn run_on(mut s: State, text: &str, scope: Scope) -> (State, Report) {
    let rules = EffectRules {
        prestige_min: 0.0,
        treasury_min: 0.0,
        consciousness_max: 10.0,
        militancy_max: 10.0,
        literacy_max: 1.0,
        pop_size_max: 2_000_000.0,
        plurality_max: 100.0,
        war_exhaustion_max: 100.0,
        badboy_min: 0.0,
        relation_limit: 200.0,
        pop_money_scale: 1000.0,
        seed: 7,
    };
    let cal = Calendar::new(43_800_000, vec![31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]).unwrap();
    let doc = script::parse(text).unwrap();
    let mut report = Report::default();
    s.this = Some(scope.clone());
    run_effects(&mut s, &cal, &rules, &doc.root, &scope, &mut report);
    (s, report)
}

#[test]
fn any_country_skips_the_country_in_scope_and_landless_tags() {
    let text = "name = \"label\" any_country = { any_owned = { add_province_modifier = { name = plague duration = 3650 } } any_pop = { consciousness = 10 militancy = 0.5 } prestige = -50 treasury = -10000 }";
    let (s, report) = run(text, Scope::Country("AAA".into()));
    let before = state();
    assert_eq!(s.countries["AAA"], before.countries["AAA"], "the firing country is left out");
    assert_eq!(s.countries["DDD"], before.countries["DDD"], "a tag that owns nothing is not reached");
    assert_eq!((s.countries["BBB"].prestige, s.countries["BBB"].money), (50.0, 10000.0));
    assert_eq!((s.countries["CCC"].prestige, s.countries["CCC"].money), (0.0, 0.0), "both stop at their minimum");
    let plague = vec![TimedModifier { name: "plague".into(), expires: Some(Date::new(1846, 1, 1)) }];
    assert_eq!((&s.provinces[&2].modifiers, &s.provinces[&3].modifiers, &s.provinces[&4].modifiers), (&plague, &plague, &plague));
    assert!(s.provinces[&1].modifiers.is_empty() && s.provinces[&5].modifiers.is_empty(), "own and unowned provinces untouched");
    let cm = |p: u16, i: usize| (s.provinces[&p].pops[i].consciousness, s.provinces[&p].pops[i].militancy);
    assert_eq!((cm(2, 0), cm(2, 1), cm(4, 0)), ((10.0, 1.5), (10.0, 9.5), (10.0, 0.5)), "added, capped at 10");
    assert_eq!((cm(1, 0), cm(5, 0)), ((2.0, 0.5), (1.0, 1.0)));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    assert_eq!(report.applied["add_province_modifier"], 3);
}

#[test]
fn unknown_entries_and_limits_are_reported_not_guessed() {
    let text = "BBB = { prestige = 5 } 4 = { owner = { treasury = 100 } any_pop = { militancy = -3 } } \
                any_owned = { limit = { units_in_province = 1 } add_province_modifier = { name = x duration = 10 } } \
                prestige_factor = 5 consciousness = 1 add_country_modifier = { name = boom duration = -1 }";
    let (s, report) = run(text, Scope::Country("AAA".into()));
    assert_eq!(s.countries["BBB"].prestige, 105.0);
    assert_eq!(s.countries["CCC"].money, 900.0);
    assert_eq!(s.provinces[&4].pops[0].militancy, 0.0, "not below zero");
    assert!(s.provinces[&1].modifiers.is_empty(), "a scope whose limit uses an unknown condition is skipped as a whole");
    assert_eq!(s.countries["AAA"].modifiers, vec![TimedModifier { name: "boom".into(), expires: None }], "no duration = permanent");
    let skipped: Vec<&str> = report.unsupported.keys().map(String::as_str).collect();
    assert_eq!(skipped, ["any_owned with limit on units_in_province (in country scope)", "consciousness (in country scope)"]);
}

/// Owner's event 10100600: every pop of the capital province is multiplied, up to the limit.
#[test]
fn capital_scope_reduce_pop_and_the_pop_size_limit() {
    let text = "capital_scope = { any_pop = { reduce_pop = 50 literacy = 0.7 } }";
    let (once, report) = run(text, Scope::Country("BBB".into()));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    assert_eq!(once.provinces[&2].pops.iter().map(|p| p.size).collect::<Vec<_>>(), [50_000.0, 50_000.0], "capital province only, every pop");
    assert_eq!(once.provinces[&2].pops[0].literacy, 0.7);
    assert_eq!(once.provinces[&4].pops[0].size, 1000.0, "other provinces untouched");
    let (many, _) = run(&text.repeat(3), Scope::Country("BBB".into()));
    assert_eq!(many.provinces[&2].pops[0].size, 2_000_000.0, "50 x 50 x 50 x 1000 stops at the limit");
    assert_eq!(many.provinces[&2].pops[0].literacy, 1.0, "literacy stops at 100%");
    let (half, _) = run("2 = { any_pop = { reduce_pop = 0.5 } }", Scope::Country("AAA".into()));
    assert_eq!(half.provinces[&2].pops[0].size, 500.0);
}

#[test]
fn flags_and_removing_modifiers() {
    let text = [
        "set_global_flag = great_fire set_country_flag = warned BBB = { set_country_flag = other }",
        "add_country_modifier = { name = boom duration = 30 } add_country_modifier = { name = bust duration = 30 } remove_country_modifier = boom",
        "1 = { add_province_modifier = { name = plague duration = 10 } remove_province_modifier = plague } clr_country_flag = missing",
    ]
    .join(" ");
    let (s, report) = run(&text, Scope::Country("AAA".into()));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    assert!(s.global_flags.contains("great_fire"));
    assert!(s.countries["AAA"].flags.contains("warned") && s.countries["BBB"].flags.contains("other") && !s.countries["BBB"].flags.contains("warned"));
    assert_eq!(s.countries["AAA"].modifiers.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["bust"]);
    assert!(s.provinces[&1].modifiers.is_empty());
    let (s, _) = run("set_country_flag = warned clr_country_flag = warned set_global_flag = g clr_global_flag = g", Scope::Country("AAA".into()));
    assert!(s.countries["AAA"].flags.is_empty() && s.global_flags.is_empty());
}

/// Limits keep only the targets that meet every condition; numbers mean "at least".
#[test]
fn limits_filter_the_targets_of_a_scope() {
    use wiktoria_simulation::script::conditions_hold;
    // Only pops with militancy of at least 1 are touched: both pops of province 2, not the others.
    let (s, report) = run("any_country = { any_pop = { limit = { militancy = 1 } consciousness = -10 } }", Scope::Country("AAA".into()));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    let con = |p: u16, i: usize| s.provinces[&p].pops[i].consciousness;
    assert_eq!((con(2, 0), con(2, 1), con(4, 0), con(1, 0)), (0.0, 0.0, 0.0, 2.0));
    // Country conditions: BBB (prestige 100) is civilized, CCC (30) is not; a flag set earlier counts.
    let text = "any_country = { limit = { civilized = yes prestige = 60 } set_country_flag = rich } \
                any_country = { limit = { OR = { has_country_flag = rich tag = CCC } NOT = { tag = BBB } } prestige = 1 }";
    let (s, report) = run(&text.replace('\\', ""), Scope::Country("AAA".into()));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    assert!(s.countries["BBB"].flags.contains("rich") && !s.countries["CCC"].flags.contains("rich"));
    assert_eq!((s.countries["BBB"].prestige, s.countries["CCC"].prestige), (100.0, 31.0));
    // Province and pop conditions, checked directly.
    let st = state();
    let hold = |text: &str, scope: Scope| conditions_hold(&st, &script::parse(text).unwrap().root, &scope);
    assert_eq!(hold("is_core = BBB owned_by = BBB is_colonial = no province_id = 2", Scope::Province(2)), Ok(true));
    assert_eq!(hold("is_core = AAA", Scope::Province(2)), Ok(false));
    assert_eq!(hold("is_primary_culture = no", Scope::Pop(2, 0)), Ok(true), "the test pops have no culture set");
    assert_eq!(hold("type = farmers is_slave = no owner = { tag = BBB } location = { province_id = 2 }", Scope::Pop(2, 0)), Ok(true));
    assert_eq!(hold("year = 1836 owns = 4 CCC = { exists = yes }", Scope::Country("CCC".into())), Ok(true));
    assert_eq!(hold("year = 1900", Scope::Country("CCC".into())), Ok(false));
    assert_eq!(hold("exists = DDD", Scope::Country("AAA".into())), Ok(false), "a tag that owns nothing does not exist");
    assert_eq!(hold("brigades_compare = 1", Scope::Country("AAA".into())), Err("brigades_compare".to_string()), "unknown conditions are named, not guessed");
    assert_eq!(hold("is_core = THIS", Scope::Province(2)), Err("is_core".to_string()));
}

/// A world with regions, pop types and politics: AAA holds provinces 1 (region north) and 6
/// (region south, colonial); BBB holds 2 and 3 (north) ; CCC holds 4 (south).
fn wide_state() -> State {
    let mut s = state();
    s.regions = vec!["north".into(), "south".into()];
    s.strata = [("farmers", "poor"), ("clerks", "middle"), ("owners", "rich"), ("slaves", "poor")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    s.ideologies = vec!["keepers".into(), "changers".into()];
    s.issues = vec![("tariffs".into(), "trade".into()), ("open_trade".into(), "trade".into()), ("bound".into(), "bondage".into()), ("free".into(), "bondage".into())];
    s.great_powers = vec!["BBB".into()];
    let six = ProvinceState { owner: Some("AAA".into()), controller: Some("AAA".into()), colonial: 2, pops: vec![PopState { pop_type: "clerks".into(), size: 100.0, ..Default::default() }], ..Default::default() };
    s.provinces.insert(6, six);
    for (id, region) in [(1, 0), (2, 0), (3, 0), (4, 1), (5, 1), (6, 1)] {
        s.provinces.get_mut(&id).unwrap().region = Some(region);
    }
    let p1 = s.provinces.get_mut(&1).unwrap();
    p1.pops.push(PopState { pop_type: "owners".into(), size: 10.0, ..Default::default() });
    p1.pops[0].ideology = [(1, 75.0), (2, 25.0)].into();
    p1.pops[0].issues = [(1, 40.0), (2, 60.0)].into();
    s.neighbours = [(1, [2].into()), (2, [1, 3].into()), (3, [2, 4].into()), (4, [3].into())].into();
    s
}

#[test]
fn states_and_pop_groups() {
    // any_state reaches both of AAA's states; the limit keeps the colonial one.
    let (s, report) = run_on(wide_state(), "any_state = { limit = { is_colonial = yes } any_pop = { militancy = 2 } } any_state = { any_owned = { set_province_flag = seen } }", Scope::Country("AAA".into()));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    assert_eq!((s.provinces[&6].pops[0].militancy, s.provinces[&1].pops[0].militancy), (2.0, 0.5));
    assert!(s.provinces[&1].flags.contains("seen") && s.provinces[&6].flags.contains("seen") && !s.provinces[&2].flags.contains("seen"));
    // state_scope from a province: the provinces of the same region with the same owner.
    let (s, _) = run_on(wide_state(), "2 = { state_scope = { any_pop = { consciousness = 1 } } }", Scope::Country("AAA".into()));
    assert_eq!(s.provinces[&2].pops[0].consciousness, 4.0);
    assert_eq!(s.provinces[&1].pops[0].consciousness, 2.0, "same region, other owner");
    // A pop type and a strata reach the matching pops of the scope; a region name its provinces.
    let (s, report) = run_on(wide_state(), "farmers = { militancy = 1 } rich_strata = { consciousness = 3 } clerks = { literacy = 0.5 } south = { life_rating = 5 }", Scope::Country("AAA".into()));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    assert_eq!((s.provinces[&1].pops[0].militancy, s.provinces[&1].pops[1].militancy), (1.5, 0.0));
    assert_eq!((s.provinces[&1].pops[1].consciousness, s.provinces[&6].pops[0].literacy), (3.0, 0.5));
    assert_eq!(s.provinces[&2].pops[0].militancy, 1.0, "another country's farmers are untouched");
    assert_eq!((s.provinces[&4].life_rating, s.provinces[&6].life_rating, s.provinces[&1].life_rating), (5.0, 5.0, 0.0));
    // random_state picks exactly one state, the same one every time for the same seed.
    let text = "random_state = { any_owned = { set_province_flag = hit } }";
    let (a, _) = run_on(wide_state(), text, Scope::Country("AAA".into()));
    let (b, _) = run_on(wide_state(), text, Scope::Country("AAA".into()));
    assert_eq!(a, b);
    assert_eq!(a.provinces.values().filter(|p| p.flags.contains("hit")).count(), 1);
}

#[test]
fn country_values_relations_and_neighbours() {
    let text = [
        "plurality = 150 badboy = -3 war_exhaustion = 4 money = 100 capital = 1 government = republic civilized = yes",
        "relation = { who = BBB value = 150 } relation = { who = BBB value = 150 } relation = { who = THIS value = 5 }",
        "any_neighbor_country = { set_country_flag = near } any_greater_power = { prestige = 1 }",
        "social_reform = free add_accepted_culture = gamma 4 = { add_core = THIS secede_province = THIS }",
        "upper_house = { ideology = changers value = 1 } country_event = { id = 77 days = 10 }",
    ]
    .join(" ");
    let (s, report) = run_on(wide_state(), &text, Scope::Country("AAA".into()));
    assert_eq!(report.unsupported.keys().collect::<Vec<_>>(), ["relation (in country scope)"], "a relation with itself (THIS) is not applied");
    let a = &s.countries["AAA"];
    assert_eq!((a.plurality, a.badboy, a.war_exhaustion, a.money, a.capital), (100.0, 0.0, 4.0, 600.0, Some(1)));
    assert_eq!((a.government.as_deref(), a.civilized, a.reforms["bondage"].as_str()), (Some("republic"), true, "free"));
    assert_eq!((a.relations["BBB"], s.countries["BBB"].relations["AAA"]), (200.0, 200.0), "both sides, within the limit");
    assert!(s.countries["BBB"].flags.contains("near") && !s.countries["CCC"].flags.contains("near"));
    assert_eq!(s.countries["BBB"].prestige, 101.0);
    assert!(a.accepted_cultures.contains(&"gamma".to_string()));
    assert_eq!((s.provinces[&4].owner.as_deref(), s.provinces[&4].cores.clone()), (Some("AAA"), vec!["CCC".to_string(), "AAA".to_string()]));
    assert_eq!(a.upper_house["changers"], 1.0, "an empty house filled by one ideology");
    assert_eq!((s.pending_events[0].id, s.pending_events[0].fires), (77, Date::new(1836, 1, 11)));
}

#[test]
fn pop_shares_and_scaled_effects() {
    let text = "1 = { any_pop = { limit = { type = farmers } scaled_militancy = { ideology = keepers factor = 4 } scaled_consciousness = { issue = open_trade factor = -1 } ideology = { value = changers factor = 0.25 } dominant_issue = { value = tariffs factor = -1 } } }";
    let (s, report) = run_on(wide_state(), text, Scope::Country("AAA".into()));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    let p = &s.provinces[&1].pops[0];
    assert_eq!((p.militancy, p.consciousness), (0.5 + 4.0 * 0.75, 2.0 - 0.6));
    // changers 25 + 25 of 125 -> 40%; keepers 75 of 125 -> 60%.
    assert!((p.ideology[&2] - 40.0).abs() < 1e-9 && (p.ideology[&1] - 60.0).abs() < 1e-9, "{:?}", p.ideology);
    assert_eq!((p.issues[&1], p.issues[&2]), (0.0, 100.0), "a share cannot go below zero");
    // An ideology the definitions do not name is reported, not guessed.
    let (_, report) = run_on(wide_state(), "any_pop = { ideology = { value = nobody factor = 1 } }", Scope::Country("AAA".into()));
    assert!(report.unsupported.contains_key("ideology (in pop scope)"));
}

fn holds(s: &State, text: &str, scope: Scope) -> Result<bool, String> {
    conditions_hold(s, &script::parse(text).unwrap().root, &scope)
}

/// Conditions of triggers and decisions: war, spheres, technology, parties, "any" scopes.
#[test]
fn trigger_conditions() {
    let mut s = wide_state();
    s.wars = vec![(["AAA".to_string()].into(), ["CCC".to_string()].into())];
    s.vassals = vec![("BBB".into(), "CCC".into())];
    s.technologies = ["steam".to_string(), "rail".to_string()].into();
    s.inventions = vec!["pump".into(), "valve".into()];
    s.party_ideologies = vec!["keepers".into(), "changers".into()];
    s.countries.get_mut("BBB").unwrap().sphere = ["AAA".to_string()].into();
    let a = s.countries.get_mut("AAA").unwrap();
    a.technologies = ["steam".to_string()].into();
    a.inventions = [2].into();
    a.ruling_party = Some(2);
    a.upper_house = [("keepers".to_string(), 0.7), ("changers".to_string(), 0.3)].into();
    let me = || Scope::Country("AAA".into());
    for (text, want) in [
        ("war = yes war_with = CCC NOT = { war_with = BBB }", true),
        ("steam = 1 rail = 0 invention = valve NOT = { invention = pump }", true),
        ("ruling_party_ideology = changers upper_house = { ideology = keepers value = 0.6 }", true),
        ("part_of_sphere = yes in_sphere = BBB sphere_owner = { tag = BBB } is_vassal = no", true),
        ("CCC = { vassal_of = BBB is_independant = no } BBB = { is_sphere_leader_of = AAA war = no }", true),
        ("colonial_nation = yes any_owned_province = { is_colonial = yes province_id = 6 }", true),
        ("any_neighbor_country = { tag = BBB } NOT = { any_neighbor_country = { tag = CCC } }", true),
        ("any_greater_power = { tag = BBB } 1 = { any_pop = { type = owners } } any_state = { is_colonial = yes }", true),
        ("all_core = { owned_by = AAA } militancy = 0.4 has_pop_type = clerks", true),
        ("1 = { any_neighbor_province = { owned_by = BBB } controlled_by = owner trade_goods = coal }", false),
        ("ai = no", false),
    ] {
        assert_eq!(holds(&s, text, me()), Ok(want), "{text}");
    }
    // Unknown conditions are named, all of them, even after one that fails.
    let block = script::parse("war = no units_in_province = 1 OR = { brigades_compare = 1 tag = AAA }").unwrap();
    assert_eq!(unknown_conditions(&s, &block.root, &me()).into_iter().collect::<Vec<_>>(), ["brigades_compare", "units_in_province"]);
    assert_eq!(holds(&s, "units_in_province = 1", me()), Err("units_in_province".to_string()));
}

/// Wiki readings: support in percent of the people in scope, and a province's largest culture.
#[test]
fn support_shares_and_majority_culture() {
    let mut s = wide_state();
    // Province 1: 1000 farmers (alpha, 75% keepers, 60% open_trade) and 10 owners (gamma).
    let p = s.provinces.get_mut(&1).unwrap();
    p.pops[0].culture = "alpha".into();
    p.pops[1].culture = "gamma".into();
    s.culture_groups = [("alpha".to_string(), "first".to_string())].into();
    s.union_groups = ["first".to_string()].into();
    let me = || Scope::Country("AAA".into());
    // AAA: 1000 of 1110 people hold shares -> keepers 67.6%, open_trade 54.1%.
    assert_eq!(holds(&s, "keepers = 67 open_trade = 54 NOT = { keepers = 68 } NOT = { changers = 23 }", me()), Ok(true));
    assert_eq!(holds(&s, "keepers = 75", Scope::Pop(1, 0)), Ok(true));
    assert_eq!(holds(&s, "is_primary_culture = yes is_accepted_culture = no culture = alpha has_national_minority = yes total_pops = 1010", Scope::Province(1)), Ok(true));
    assert_eq!(holds(&s, "is_primary_culture = yes", Scope::Pop(1, 1)), Ok(false), "the minority pop itself");
    assert_eq!(holds(&s, "culture_has_union_tag = yes total_pops = 1110 NOT = { total_pops = 1111 }", me()), Ok(true));
}

/// a new product turns the province's farm or mine workers into the type that
/// works it, the same day; `life_rating` adds.
#[test]
fn a_new_product_changes_the_workers() {
    let mut s = state();
    s.rgo_workers = [("grain", "farmers"), ("coal", "labourers")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let pops = &mut s.provinces.get_mut(&2).unwrap().pops;
    pops.push(PopState { pop_type: "clerks".into(), size: 50.0, ..Default::default() });
    s.provinces.get_mut(&2).unwrap().life_rating = 35.0;
    let (s, _) = run_on(s, "2 = { trade_goods = coal life_rating = 10 }", Scope::Country("BBB".into()));
    let p = &s.provinces[&2];
    assert_eq!(p.trade_goods.as_deref(), Some("coal"));
    assert_eq!(p.life_rating, 45.0, "the effect adds to the life rating");
    // The two farmer pops (same culture and religion here) are now one labourer pop; clerks stay.
    let people = |kind: &str| p.pops.iter().filter(|q| q.pop_type == kind).map(|q| q.size).sum::<f64>();
    assert_eq!((people("farmers"), people("labourers"), people("clerks")), (0.0, 2000.0, 50.0));
    // Back to a farm product: labourers become farmers again.
    let (s, _) = run_on(s, "2 = { trade_goods = grain }", Scope::Country("BBB".into()));
    let p = &s.provinces[&2];
    let people = |kind: &str| p.pops.iter().filter(|q| q.pop_type == kind).map(|q| q.size).sum::<f64>();
    assert_eq!((people("farmers"), people("labourers")), (2000.0, 0.0));
}

#[test]
fn an_effect_can_turn_a_whole_pop_into_another_type() {
    let mut s = state();
    s.strata = [("farmers", "poor"), ("soldiers", "poor")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let (s, _) = run_on(s, "pop_type = soldiers", Scope::Pop(1, 0));
    assert_eq!((s.provinces[&1].pops[0].pop_type.as_str(), s.provinces[&1].pops[0].size), ("soldiers", 1000.0));
    // A name that is no pop type changes nothing.
    let (s, _) = run_on(s, "pop_type = wizards", Scope::Pop(1, 0));
    assert_eq!(s.provinces[&1].pops[0].pop_type, "soldiers");
}

#[test]
fn building_effects_add_levels_up_to_the_cap() {
    let mut s = state();
    s.building_kinds = [("infrastructure".to_string(), ("railroad".to_string(), 6.0))].into();
    let (s, _) = run_on(s, "2 = { infrastructure = 2 }", Scope::Country("BBB".into()));
    assert_eq!(s.provinces[&2].buildings["railroad"], 2.0);
    let (s, _) = run_on(s, "2 = { infrastructure = 6 }", Scope::Country("BBB".into()));
    assert_eq!(s.provinces[&2].buildings["railroad"], 6.0, "never above the highest level");
}

#[test]
fn years_of_research_give_points_at_the_present_rate() {
    let mut s = state();
    s.countries.get_mut("AAA").unwrap().research_rate = 2.0;
    let (s, _) = run_on(s, "years_of_research = 0.5", Scope::Country("AAA".into()));
    assert_eq!(s.countries["AAA"].research_points, 365.0);
}

#[test]
fn pops_can_build_factories_with_cash_in_a_civilised_country() {
    let mut s = state();
    // 6.25 in cash is enough; the ruling party plays no part. AAA and BBB are
    // civilised, CCC is not.
    s.provinces.get_mut(&1).unwrap().pops[0].money = 6.25;
    s.provinces.get_mut(&2).unwrap().pops[0].money = 6.2;
    s.provinces.get_mut(&4).unwrap().pops[0].money = 1_000_000.0;
    assert_eq!(holds(&s, "can_build_factory = yes", Scope::Pop(1, 0)), Ok(true));
    assert_eq!(holds(&s, "can_build_factory = yes", Scope::Pop(2, 0)), Ok(false));
    assert_eq!(holds(&s, "can_build_factory = yes", Scope::Pop(4, 0)), Ok(false));
    assert_eq!(holds(&s, "can_build_factory = no", Scope::Pop(4, 0)), Ok(true));
}

// ---------------------------------------------------------------- the original's script engine, as described

/// `wide_state` with reforms, cultures with a union country, a ranking and factories.
fn engine_state() -> State {
    let mut s = wide_state();
    s.issues = vec![("none".into(), "vote".into()), ("some".into(), "vote".into()), ("all".into(), "vote".into()), ("tariffs".into(), "trade".into()), ("open_trade".into(), "trade".into())];
    s.issue_categories = vec!["political_reforms".into(), "political_reforms".into(), "political_reforms".into(), "party_issues".into(), "party_issues".into()];
    s.culture_groups = [("alpha", "greek"), ("beta", "greek"), ("gamma", "hebrew")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    s.union_groups = ["greek".to_string()].into();
    s.union_tags = [("greek".to_string(), "UUU".to_string())].into();
    s.countries.insert("UUU".into(), CountryState::default());
    s.party_ideologies = vec!["keepers".into(), "changers".into()];
    (s.badboy_limit, s.colonial_rank) = (25.0, 2.0);
    s.rank_order = vec!["BBB".into(), "AAA".into(), "CCC".into()];
    s.this = Some(Scope::Country("AAA".into()));
    s.factory_types = [("mill".to_string(), ("cloth".to_string(), 1000.0))].into();
    s.factory_worker_types = ["clerks".to_string()].into();
    (s.economy_numbers.industrial_score_per_level, s.economy_numbers.industrial_score_min_staff) = (4.0, 0.05);
    s.rgo_workers = [("grain".to_string(), "farmers".to_string())].into();
    // AAA's southern state (province 6, 100 clerks) has a mill with 40 of them at work.
    s.factories = vec![FactoryState { owner: "AAA".into(), region: Some(1), kind: "mill".into(), level: 1.0, staff: Some(40.0), ..Default::default() }];
    let a = s.countries.get_mut("AAA").unwrap();
    a.reforms = [("vote".to_string(), "none".to_string()), ("trade".to_string(), "tariffs".to_string())].into();
    (a.ruling_party, a.badboy, a.capital) = (Some(1), 10.0, Some(1));
    for (id, continent, mass) in [(1, "west", 0), (2, "west", 0), (3, "west", 0), (4, "east", 0), (5, "east", 1), (6, "east", 1)] {
        s.provinces.get_mut(&id).unwrap().continent = Some(continent.into());
        s.land_mass.insert(id, mass);
    }
    s
}

#[test]
fn next_reform_and_agreement_with_the_ruling_party() {
    let s = engine_state();
    let me = || Scope::Country("AAA".into());
    assert_eq!(holds(&s, "is_next_reform = some", me()), Ok(true), "the option after the one in force");
    assert_eq!(holds(&s, "is_next_reform = all", me()), Ok(false), "two steps away");
    assert_eq!(holds(&s, "is_next_reform = none", me()), Ok(false), "the one in force itself");
    assert_eq!(holds(&s, "is_next_reform = open_trade", me()), Ok(false), "political and social reforms only");
    assert_eq!(holds(&s, "is_next_reform = some", Scope::Pop(1, 0)), Ok(true), "a pop stands for its country");
    assert_eq!(holds(&s, "is_next_reform = some", Scope::Country("BBB".into())), Ok(false), "BBB has no option in force");
    // AAA is ruled by the keepers; the first pop of province 1 supports them with 75 of 100.
    assert_eq!(holds(&s, "agree_with_ruling_party = 0.75", Scope::Pop(1, 0)), Ok(true));
    assert_eq!(holds(&s, "agree_with_ruling_party = 0.76", Scope::Pop(1, 0)), Ok(false));
    assert_eq!(holds(&s, "agree_with_ruling_party = 0", me()), Ok(false), "no pop: false");
    assert_eq!(holds(&s, "agree_with_ruling_party = 0", Scope::Pop(2, 0)), Ok(false), "no ruling party: false");
}

#[test]
fn not_any_pop_regions_and_cores_follow_the_original() {
    let s = engine_state();
    let me = || Scope::Country("AAA".into());
    assert_eq!(holds(&s, "NOT = { tag = CCC tag = BBB }", me()), Ok(true), "none of them");
    assert_eq!(holds(&s, "NOT = { tag = CCC tag = AAA }", me()), Ok(false), "one of them holds");
    assert_eq!(holds(&s, "any_pop = { type = farmers }", me()), Ok(false), "any_pop asked of a country is false");
    assert_eq!(holds(&s, "any_pop = { type = farmers }", Scope::State(0, "AAA".into())), Ok(true), "of a state it works");
    assert_eq!(holds(&s, "north = { owned_by = BBB }", me()), Ok(true), "a region: any of its provinces");
    assert_eq!(holds(&s, "north = { owned_by = CCC }", me()), Ok(false));
    assert_eq!(holds(&s, "all_core = { always_unknown = 1 }", Scope::Country("DDD".into())), Ok(false), "a country without cores fails all_core");
    assert_eq!(holds(&s, "all_core = { tag = AAA }", Scope::Province(5)), Ok(true), "a province without cores passes it");
    assert_eq!(holds(&s, "any_core = { tag = BBB } all_core = { tag = BBB }", Scope::Province(2)), Ok(true), "in a province: the countries with a core");
    assert_eq!(holds(&s, "tag = AAA", Scope::Province(1)), Ok(false), "a province has no tag");
}

#[test]
fn amounts_ranks_and_scores() {
    let s = engine_state();
    let me = || Scope::Country("AAA".into());
    // Infamy 10 of a limit of 25.
    assert_eq!(holds(&s, "badboy = 0.4 NOT = { badboy = 0.41 }", me()), Ok(true), "a share of BADBOY_LIMIT");
    assert_eq!(holds(&s, "rank = 2 NOT = { rank = 1 }", me()), Ok(true), "rank n or better");
    assert_eq!(holds(&s, "rank = 200", Scope::Province(1)), Ok(false), "not a country: false");
    assert_eq!(holds(&s, "is_secondary_power = yes", me()), Ok(true), "civilised, no great power, rank 2 of 2 allowed");
    assert_eq!(holds(&s, "is_secondary_power = no", Scope::Country("BBB".into())), Ok(true), "a great power is not");
    assert_eq!(holds(&s, "is_secondary_power = no", Scope::Country("CCC".into())), Ok(true), "uncivilised");
    // One level, 1000 places (10 hundreds), 100 clerks (1 hundred): 4 x 1 x 0.1.
    assert!((s.industrial_score("AAA") - 0.4).abs() < 1e-9);
    assert_eq!(holds(&s, "industrial_score = 0.4 NOT = { industrial_score = 0.5 } BBB = { NOT = { industrial_score = THIS } }", me()), Ok(true));
    assert_eq!(holds(&s, "industrial_score = 0", Scope::Province(6)), Ok(false), "no country: false");
    assert_eq!(holds(&s, "num_of_cities = 2 NOT = { num_of_cities = 3 } num_of_cities = THIS BBB = { num_of_cities = THIS }", me()), Ok(true), "BBB has as many provinces as AAA");
    assert_eq!(holds(&s, "total_pops = 1010 NOT = { total_pops = 1011 }", Scope::Pop(1, 0)), Ok(true), "a pop: the people of its state");
    assert_eq!(holds(&s, "total_pops = 1110", me()), Ok(true));
    let mut r = engine_state();
    r.provinces.get_mut(&6).unwrap().controller = Some("REB".into());
    r.provinces.get_mut(&2).unwrap().cores.push("AAA".into());
    assert_eq!(holds(&r, "num_of_revolts = 1 NOT = { num_of_revolts = 2 } revolt_percentage = 0.5 6 = { controlled_by_rebels = yes } 1 = { controlled_by_rebels = no }", me()), Ok(true));
    assert_eq!(holds(&r, "lost_national = 0.5 NOT = { lost_national = 0.51 }", me()), Ok(true), "one of its two cores is BBB's");
    assert_eq!(holds(&r, "revolt_percentage = 0", Scope::Country("DDD".into())), Ok(false), "no provinces: false");
}

#[test]
fn work_buildings_and_products() {
    let mut s = engine_state();
    s.provinces.get_mut(&1).unwrap().trade_goods = Some("grain".into());
    s.provinces.get_mut(&1).unwrap().buildings.insert("fort".into(), 1.0);
    let me = || Scope::Country("AAA".into());
    assert_eq!(holds(&s, "produces = grain", me()), Ok(false), "never of a country");
    assert_eq!(holds(&s, "1 = { produces = grain NOT = { produces = cloth } } 6 = { NOT = { produces = cloth } }", me()), Ok(true), "a province: its farm or mine");
    assert_eq!(holds(&s, "produces = cloth", Scope::State(1, "AAA".into())), Ok(true), "a state: its factories too");
    assert_eq!(holds(&s, "produces = grain", Scope::State(0, "AAA".into())), Ok(true));
    assert_eq!(holds(&s, "has_building = mill", me()), Ok(false), "never of a country");
    assert_eq!(holds(&s, "6 = { has_building = mill has_building = factory NOT = { has_building = fort } } 1 = { has_building = fort NOT = { has_building = mill } }", me()), Ok(true));
    let mut shut = s.clone();
    shut.factories[0].no_input_days = 11;
    assert_eq!(holds(&shut, "has_building = mill", Scope::State(1, "AAA".into())), Ok(false), "11 days without inputs");
    // 40 of the 100 clerks of province 6 have work; the 1000 farmers and 10 owners of province 1 all do.
    assert_eq!(holds(&s, "unemployment = 0.6 NOT = { unemployment = 0.61 }", Scope::Province(6)), Ok(true));
    assert_eq!(holds(&s, "unemployment = 0.054 NOT = { unemployment = 0.055 }", me()), Ok(true), "60 of 1110 people");
    assert_eq!(holds(&s, "unemployment = 0.6", Scope::Pop(6, 0)), Ok(true));
    assert_eq!(holds(&s, "unemployment = 0.01", Scope::Pop(1, 0)), Ok(false));
    assert_eq!(holds(&s, "unemployment_by_type = { type = clerks value = 0.6 } NOT = { unemployment_by_type = { type = farmers value = 0.01 } }", me()), Ok(true));
    assert_eq!(holds(&s, "unemployment_by_type = { type = clerks value = 0.01 }", Scope::Province(1)), Ok(false), "nobody of the type: 0");
    // A farm that employs 250 of the 1000 farmers.
    s.provinces.get_mut(&1).unwrap().rgo_staff = Some(250.0);
    assert_eq!(holds(&s, "unemployment_by_type = { type = farmers value = 0.75 }", Scope::Pop(1, 1)), Ok(true), "a pop: its state");
    assert_eq!(holds(&s, "administration_spending = 0.5", me()), Err("administration_spending".to_string()), "no slider known");
    s.countries.get_mut("AAA").unwrap().spending.insert("crime_fighting".into(), 0.5);
    assert_eq!(holds(&s, "administration_spending = 0.5 NOT = { administration_spending = 0.6 }", me()), Ok(true), "the crime fighting slider");
}

#[test]
fn people_cultures_and_places() {
    let mut s = engine_state();
    for (id, i, culture, religion) in [(1, 0, "alpha", "sun"), (1, 1, "gamma", "moon"), (6, 0, "beta", "sun"), (2, 0, "alpha", "sun"), (2, 1, "alpha", "sun")] {
        let q = &mut s.provinces.get_mut(&id).unwrap().pops[i];
        (q.culture, q.religion) = (culture.into(), religion.into());
    }
    let me = || Scope::Country("AAA".into());
    assert_eq!(holds(&s, "has_national_minority = yes 6 = { has_national_minority = no } 1 = { has_national_minority = yes }", me()), Ok(true), "more than one culture");
    assert_eq!(holds(&s, "minorities = yes 6 = { minorities = no } 1 = { minorities = yes }", me()), Ok(true), "beta is accepted, gamma is not");
    assert_eq!(holds(&s, "pop_majority_culture = alpha pop_majority_religion = sun pop_majority_ideology = keepers pop_majority_issue = some", me()), Ok(true));
    assert_eq!(holds(&s, "pop_majority_culture = beta", Scope::State(1, "AAA".into())), Ok(true));
    assert_eq!(holds(&s, "pop_majority_culture = alpha", Scope::Pop(1, 0)), Ok(false), "a pop alone: false");
    assert_eq!(holds(&s, "pop_majority_ideology = keepers", Scope::Pop(1, 0)), Ok(true), "but it has a largest ideology");
    // AAA's capital is province 1 (west, land 0); 6 is east on other land, 4 is east on the same land.
    assert_eq!(holds(&s, "6 = { is_overseas = yes } 1 = { is_overseas = no }", me()), Ok(true));
    assert_eq!(holds(&s, "is_overseas = no", me()), Ok(false), "only a province can be overseas");
    let mut t = s.clone();
    t.provinces.get_mut(&4).unwrap().owner = Some("AAA".into());
    assert_eq!(holds(&t, "4 = { is_overseas = no }", me()), Ok(true), "another continent, but reachable by land");
    assert_eq!(holds(&s, "5 = { empty = yes } 4 = { empty = no NOT = { has_empty_adjacent_province = yes } }", me()), Ok(true));
    t.neighbours.insert(4, [5].into());
    assert_eq!(holds(&t, "4 = { has_empty_adjacent_province = no }", me()), Ok(true), "the written yes or no is not looked at");
    // Unions: alpha and beta are greek, whose union country is UUU.
    assert_eq!(holds(&s, "this_culture_union = UUU this_culture_union = this_union is_cultural_union = no cultural_union = { tag = UUU exists = no }", me()), Ok(true));
    s.countries.get_mut("UUU").unwrap().primary_culture = Some("beta".into());
    assert_eq!(holds(&s, "is_cultural_union = yes is_cultural_union = AAA", Scope::Country("UUU".into())), Ok(true));
    s.countries.get_mut("CCC").unwrap().primary_culture = Some("gamma".into());
    assert_eq!(holds(&s, "cultural_union = { always_unknown = 1 }", Scope::Country("CCC".into())), Ok(false), "no union country: false");
}

#[test]
fn a_world_without_crises_truces_or_lost_wars() {
    let mut s = engine_state();
    let me = || Scope::Country("AAA".into());
    assert_eq!(holds(&s, "involved_in_crisis = yes", me()), Ok(false));
    assert_eq!(holds(&s, "involved_in_crisis = no", me()), Ok(false), "false with `no` too while no crisis exists");
    assert_eq!(holds(&s, "crisis_exist = no has_recently_lost_war = no NOT = { truce_with = BBB } NOT = { constructing_cb_progress = 0 }", me()), Ok(true));
    assert_eq!(holds(&s, "has_recently_lost_war = no", Scope::Province(1)), Ok(false), "no country: false either way");
    s.substates = vec![("BBB".into(), "AAA".into())];
    s.countries.get_mut("AAA").unwrap().influence.insert("CCC".into(), 30.0);
    assert_eq!(holds(&s, "substate_of = BBB vassal_of = BBB overlord = { tag = BBB } NOT = { substate_of = CCC }", me()), Ok(true));
    assert_eq!(holds(&s, "diplomatic_influence = { who = CCC value = 30 } NOT = { diplomatic_influence = { who = CCC value = 31 } } NOT = { diplomatic_influence = { who = BBB value = 1 } }", me()), Ok(true));
}

#[test]
fn change_tag_moves_the_country_and_keeps_the_cores() {
    let mut s = engine_state();
    s.countries.get_mut("BBB").unwrap().sphere = ["AAA".to_string()].into();
    s.vassals = vec![("AAA".into(), "CCC".into())];
    // `culture`: the union country of AAA's group. The prestige written after it reaches the new tag.
    let (s, report) = run_on(s, "prestige = 1 change_tag = culture prestige = 2 set_country_flag = united", Scope::Country("AAA".into()));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    assert_eq!((s.provinces[&1].owner.as_deref(), s.provinces[&6].controller.as_deref()), (Some("UUU"), Some("UUU")));
    assert_eq!(s.provinces[&1].cores, ["AAA"], "cores are not switched");
    assert_eq!(s.countries["UUU"].prestige, 73.0);
    assert!(s.countries["UUU"].flags.contains("united") && s.countries["AAA"].flags.is_empty() && s.countries["AAA"].prestige == 0.0, "the old tag is left with an empty record");
    assert!(s.countries["BBB"].sphere.contains("UUU") && s.vassals == [("UUU".to_string(), "CCC".to_string())] && s.factories[0].owner == "UUU" && s.rank_order[1] == "UUU");
    assert_eq!(s.this, Some(Scope::Country("UUU".into())));
    // A tag that owns land cannot be taken; a group without a union country changes nothing.
    let (s, report) = run_on(s, "change_tag = BBB", Scope::Country("UUU".into()));
    assert!(s.provinces[&1].owner.as_deref() == Some("UUU") && report.unsupported.len() == 1);
    let mut t = engine_state();
    t.countries.get_mut("CCC").unwrap().primary_culture = Some("gamma".into());
    let (t, _) = run_on(t, "change_tag = culture", Scope::Country("CCC".into()));
    assert_eq!(t.provinces[&4].owner.as_deref(), Some("CCC"));
    // The other word only moves the player.
    let mut h = engine_state();
    h.countries.get_mut("AAA").unwrap().human = true;
    let (h, report) = run_on(h, "change_tag_no_core_switch = UUU BBB = { change_tag_no_core_switch = DDD }", Scope::Country("AAA".into()));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    assert!(h.countries["UUU"].human && !h.countries["AAA"].human && !h.countries["DDD"].human);
    assert_eq!(h.provinces[&1].owner.as_deref(), Some("AAA"), "the country itself is not converted");
}

#[test]
fn land_changes_hands_with_its_factories() {
    // inherit: AAA takes all of CCC; annex_to the other way round.
    let (s, report) = run_on(engine_state(), "inherit = CCC", Scope::Country("AAA".into()));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    assert_eq!((s.provinces[&4].owner.as_deref(), s.provinces[&4].controller.as_deref()), (Some("AAA"), Some("AAA")));
    let (s, _) = run_on(engine_state(), "annex_to = BBB", Scope::Country("AAA".into()));
    assert!(s.provinces[&1].owner.as_deref() == Some("BBB") && s.provinces[&6].owner.as_deref() == Some("BBB"));
    assert_eq!(s.factories[0].owner, "BBB", "the factories of a state go with its last province");
    assert!(s.provinces[&6].has_factory);
    // secede_province: only from THIS's own land, or to THIS.
    let (s, report) = run_on(engine_state(), "6 = { secede_province = DDD } 2 = { secede_province = CCC } 3 = { secede_province = THIS }", Scope::Country("AAA".into()));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    assert_eq!(s.provinces[&6].owner.as_deref(), Some("DDD"), "AAA gives its own province; DDD comes into being");
    assert_eq!((s.countries["DDD"].capital, s.factories[0].owner.as_str()), (Some(6), "DDD"));
    assert_eq!(s.provinces[&2].owner.as_deref(), Some("BBB"), "a third country's province stays");
    assert_eq!(s.provinces[&3].owner.as_deref(), Some("AAA"), "anybody's province can go to THIS");
    // release: the tag takes the provinces it has cores on from the country in scope.
    let mut r = engine_state();
    r.provinces.get_mut(&6).unwrap().cores = vec!["DDD".into()];
    r.provinces.get_mut(&4).unwrap().cores.push("DDD".into());
    let (s, report) = run_on(r.clone(), "release_vassal = DDD", Scope::Country("AAA".into()));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    assert_eq!((s.provinces[&6].owner.as_deref(), s.provinces[&4].owner.as_deref(), s.provinces[&1].owner.as_deref()), (Some("DDD"), Some("CCC"), Some("AAA")));
    assert_eq!(s.vassals, [("AAA".to_string(), "DDD".to_string())]);
    let (s, _) = run_on(r, "release = DDD", Scope::Country("AAA".into()));
    assert!(s.provinces[&6].owner.as_deref() == Some("DDD") && s.vassals.is_empty(), "release makes no vassal");
    // A subject that names itself is freed.
    let mut v = engine_state();
    v.substates = vec![("BBB".into(), "AAA".into())];
    let (s, _) = run_on(v, "release_vassal = THIS", Scope::Country("AAA".into()));
    assert!(s.substates.is_empty());
}

#[test]
fn cores_cultures_capital_and_reforms() {
    let text = "add_core = 4 remove_core = 1 2 = { add_core = THIS add_core = CCC remove_core = BBB } all_core = { limit = { owned_by = THIS } add_core = DDD } \
                add_accepted_culture = union add_accepted_culture = delta capital = 6 political_reform = all";
    let (s, report) = run_on(engine_state(), text, Scope::Country("AAA".into()));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    assert_eq!(s.provinces[&4].cores, ["CCC", "AAA"], "a province number in a country scope: its own core there");
    assert!(s.provinces[&1].cores.is_empty());
    assert_eq!(s.provinces[&2].cores, ["AAA", "CCC"]);
    assert!(s.provinces[&6].cores.is_empty() && s.provinces[&3].cores == ["BBB"], "all_core reaches the country's cores; the limit kept none of them here");
    assert_eq!(s.countries["AAA"].accepted_cultures, ["beta", "delta"], "union: the other cultures of the group (beta was accepted already)");
    assert_eq!(s.countries["AAA"].capital, Some(6));
    assert_eq!(s.countries["AAA"].reforms["vote"], "all", "the option is set, whatever was in force");
    let mut u = engine_state();
    u.countries.get_mut("AAA").unwrap().accepted_cultures.clear();
    let (u, _) = run_on(u, "add_accepted_culture = union", Scope::Country("AAA".into()));
    assert_eq!(u.countries["AAA"].accepted_cultures, ["beta"]);
}

#[test]
fn pops_assimilate_move_and_multiply() {
    let mut s = engine_state();
    s.provinces.get_mut(&1).unwrap().pops[1].culture = "gamma".into();
    // assimilate reaches the whole province although only one pop passes the limit.
    let (s, report) = run_on(s, "1 = { any_pop = { limit = { type = owners } assimilate = yes move_pop = 6 } } 6 = { any_pop = { limit = { type = clerks } reduce_pop = 3 } }", Scope::Country("AAA".into()));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    assert!(s.provinces[&1].pops.iter().all(|q| q.culture == "alpha"), "every pop of the province takes the owner's culture");
    assert_eq!((s.provinces[&1].pops[1].size, s.provinces[&1].pops.len()), (0.0, 2), "the moved pop leaves an empty place");
    let there = &s.provinces[&6].pops;
    assert_eq!((there.len(), there[1].pop_type.as_str(), there[1].size, there[1].culture.as_str()), (2, "owners", 10.0, "alpha"));
    assert_eq!(there[0].size, 300.0, "reduce_pop above 1 multiplies");
    let (s, _) = run_on(s, "6 = { any_pop = { move_pop = 6 } }", Scope::Country("AAA".into()));
    assert_eq!(s.provinces[&6].pops.len(), 2, "a pop that is there already stays");
}

#[test]
fn money_modifiers_elections_and_unlocks() {
    let mut s = engine_state();
    s.countries.get_mut("AAA").unwrap().tax_income = 40.0;
    s.countries.get_mut("AAA").unwrap().government = Some("republic".into());
    s.countries.get_mut("AAA").unwrap().last_election = Some(Date::new(1835, 6, 1));
    s.countries.get_mut("BBB").unwrap().last_election = Some(Date::new(1835, 6, 1));
    s.election_months = [("republic".to_string(), 48)].into();
    let text = "add_tax_relative_income = 5 election = yes BBB = { election = yes } 6 = { add_country_modifier = { name = boom duration = 10 } treasury = 7 } 2 = { any_pop = { limit = { militancy = 9 } add_country_modifier = { name = riot duration = -1 } } }";
    let (s, report) = run_on(s, text, Scope::Country("AAA".into()));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    assert_eq!(s.countries["AAA"].money, 500.0 + 5.0 * 40.0 + 7.0, "5 days of tax income; treasury in a province is the owner's");
    assert_eq!(s.countries["AAA"].modifiers, vec![TimedModifier { name: "boom".into(), expires: Some(Date::new(1836, 1, 11)) }], "in a province scope: the owner's modifier");
    assert_eq!(s.countries["BBB"].modifiers, vec![TimedModifier { name: "riot".into(), expires: None }]);
    assert_eq!(report.modifiers_added["boom"], 1);
    assert_eq!(s.countries["AAA"].last_election, Some(Date::new(1832, 1, 1)), "as if the last election ended one full term ago: the campaign starts now");
    assert_eq!(s.countries["BBB"].last_election, Some(Date::new(1835, 6, 1)), "a government without elections holds none");
    // One slot for the unit of a technology or invention: the last line wins.
    let block = script::parse("activate_building = mill activate_building = forge activate_unit = lancer activate_unit = tank").unwrap();
    assert_eq!(activations(&block.root), (vec!["mill".to_string(), "forge".to_string()], Some("tank".to_string())));
}

#[test]
fn any_owned_asks_its_limit_again_and_chances_are_whole_numbers() {
    // The first province to be reached sets a flag that makes the others fail the limit.
    let text = "any_owned = { limit = { owner = { NOT = { has_country_flag = done } } } set_province_flag = hit owner = { set_country_flag = done } }";
    let (s, _) = run(text, Scope::Country("BBB".into()));
    assert!(s.provinces[&2].flags.contains("hit") && !s.provinces[&3].flags.contains("hit"));
    // Half a percent is no chance at all; a hundred is certain.
    let (s, report) = run(&"random = { chance = 0.9 prestige = 1 } random = { chance = 100 treasury = 1 } ".repeat(200), Scope::Country("AAA".into()));
    assert_eq!((s.countries["AAA"].prestige, s.countries["AAA"].money, report.applied["random"]), (70.0, 700.0, 400));
    // Weights 1 and 3: the first entry wins two cases of four, not one.
    let (s, _) = run(&"random_list = { 1 = { prestige = 1 } 3 = { treasury = 1 } } ".repeat(2000), Scope::Country("AAA".into()));
    let first = s.countries["AAA"].prestige - 70.0;
    assert!((900.0..1100.0).contains(&first), "{first}");
}

#[test]
fn base_game_hooks_always_names_tech_school_and_prestige_factor() {
    let mut s = engine_state();
    let me = || Scope::Country("AAA".into());
    assert_eq!(holds(&s, "always = yes", me()), Ok(true));
    assert_eq!(holds(&s, "always = no", me()), Ok(false));
    assert_eq!(holds(&s, "has_recent_imigration = 30", Scope::Province(1)), Ok(false));
    assert_eq!(holds(&s, "constructing_cb_type = conquest", me()), Ok(false));
    // Ranking BBB, AAA, CCC with COLONIAL_RANK 2: AAA is ranked second, CCC third.
    s.countries.get_mut("AAA").unwrap().civilized = true;
    s.countries.get_mut("CCC").unwrap().civilized = true;
    assert_eq!(holds(&s, "is_secondary_power = yes", me()), Ok(true));
    assert_eq!(holds(&s, "is_secondary_power = yes", Scope::Country("CCC".into())), Ok(false));
    let (s, _) = run_on(s, "change_province_name = \"Newtown\"", Scope::Province(1));
    assert_eq!(s.provinces[&1].name_override.as_deref(), Some("Newtown"));
    let (s, report) = run_on(s, "prestige_factor = -0.5 tech_school = army_school", me());
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    assert_eq!(s.countries["AAA"].prestige, 35.0);
    assert_eq!(s.countries["AAA"].tech_school.as_deref(), Some("army_school"));
}
