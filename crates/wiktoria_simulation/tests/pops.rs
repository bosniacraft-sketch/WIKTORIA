//! Pop growth on plain numbers.

use std::collections::BTreeMap;

use wiktoria_data::date::Date;
use wiktoria_data::script;
use wiktoria_simulation::pops::{consciousness_change, militancy_change, ConsciousnessInputs, MilitancyInputs, MoodRules};
use wiktoria_simulation::pops::{issue_drift, issue_lists, issues_due_pops, IssueKind, IssueLists};
use wiktoria_simulation::pops::{country_month, discover_inventions, invention_chance, CountryRules, InventionRules};
use wiktoria_simulation::pops::{additive_weight, admin_efficiency, assimilate_due_pops, growth, literacy_change, migrate_due_pops, move_due_pops, GrowthRules, LiteracyRules, MobilityDefs, MobilityRules, TargetCache};
use wiktoria_simulation::script::{CountryState, PopState, ProvinceState, Scope, State};
use wiktoria_simulation::{Calendar, PopSchedule};

fn rules() -> GrowthRules {
    // 0.0001 is held as 3 units of 1/32768.
    GrowthRules { base_units: 3, bonus_units: 3, min_life_rating: 30.0, life_rating_cap: 40.0, starvation_limit: 0.5, starvation_slope: 4.0, slave_divisor: 10.0, global_scale: 0.1, growth_scale: 1.0 }
}

#[test]
fn growth_follows_life_rating_in_whole_units() {
    let r = rules();
    // The original: a pop of 342,350 at life rating 35 grew by 188 (18 units).
    assert_eq!(growth(&r, 342_350.0, 1.0, 35.0, false, 0), 188.0);
    // Life rating 40 and above counts as 40: 33 units.
    assert_eq!(growth(&r, 90_150.0, 1.0, 40.0, false, 0), 90.0);
    assert_eq!(growth(&r, 90_150.0, 1.0, 50.0, false, 0), 90.0);
    // At or below the minimum only the base is left.
    assert_eq!(growth(&r, 140_875.0, 0.72, 30.0, false, 0), 12.0);
    assert_eq!(growth(&r, 140_875.0, 1.0, 15.0, false, 0), 12.0);
    // A country modifier of 0.0002 is 6 more units (the original: 234,500 at life rating 32 grew by 107).
    assert_eq!(growth(&r, 234_500.0, 1.0, 32.0, false, 6), 107.0);
    // A province modifier of -0.02 (655 units) outweighs everything.
    assert_eq!(growth(&r, 100_000.0, 1.0, 35.0, false, -655), -1943.0);
    // Slaves grow ten times slower; small pops gain nobody.
    assert_eq!(growth(&r, 342_350.0, 1.0, 35.0, true, 0), 18.0);
    assert_eq!(growth(&r, 1_000.0, 1.0, 35.0, false, 0), 0.0);
    // Under half of their life needs pops grow more slowly: nothing at a quarter, and at none
    // they shrink as fast as they would have grown.
    assert_eq!(growth(&r, 342_350.0, 0.375, 35.0, false, 0), 94.0);
    assert_eq!(growth(&r, 342_350.0, 0.25, 35.0, false, 0), 0.0);
    assert_eq!(growth(&r, 342_350.0, 0.0, 35.0, false, 0), -188.0);
}

/// D12: the owner's scale keeps a share of the people gained and leaves losses alone.
#[test]
fn growth_scale_applies_to_gains_only() {
    let r = GrowthRules { growth_scale: 0.5, ..rules() };
    assert_eq!(growth(&r, 342_350.0, 1.0, 35.0, false, 0), 94.0);
    assert_eq!(growth(&r, 100_000.0, 1.0, 35.0, false, -655), -1943.0);
}

#[test]
fn starving_pops_shrink() {
    let r = rules();
    assert_eq!(growth(&r, 100_000.0, 0.5, 35.0, false, 0), 54.0, "at the limit: still full growth");
    assert_eq!(growth(&r, 100_000.0, 0.25, 35.0, false, 0), 0.0, "at half the limit: no growth");
    assert_eq!(growth(&r, 100_000.0, 0.0, 35.0, false, 0), -54.0, "at nothing: the full rate, downwards");
}

fn world() -> State {
    let mut s = State::new(Date::new(1836, 1, 1));
    // Pop ids 0 and 31 are due on the 1st of a 31-day month (offset 1 = day 1 for id 0).
    let pop = |id: u64, kind: &str, size: f64, literacy: f64| PopState { id, pop_type: kind.into(), size, literacy, culture: "alpha".into(), religion: "one".into(), life_needs: 1.0, ..Default::default() };
    s.provinces.insert(1, ProvinceState { owner: Some("AAA".into()), pops: vec![pop(0, "farmers", 100_000.0, 0.5), pop(31, "clerks", 1_000.0, 0.5), pop(5, "farmers", 50_000.0, 0.5)], ..Default::default() });
    s.countries.insert("AAA".into(), CountryState::default());
    s.strata = [("farmers", "poor"), ("soldiers", "poor"), ("clerks", "middle"), ("owners", "rich")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    s
}

#[test]
fn weights_add_up_and_skip_what_is_unknown() {
    let s = world();
    let block = script::parse("factor = 0.25 modifier = { factor = 0.5 literacy = 0.4 } modifier = { factor = 9 literacy = 0.9 } modifier = { factor = 7 num_of_revolts = 1 } group = { modifier = { factor = -0.1 strata = poor } modifier = { factor = -0.1 life_needs = 1 } modifier = { factor = 3 clerks = 0.5 } }").unwrap();
    // 0.25 + 0.5 (literate enough) - 0.1 - 0.1; the unknown condition and the two that fail add nothing.
    assert!((additive_weight(&s, &block.root, &Scope::Pop(1, 0)) - 0.55).abs() < 1e-12);
}

/// Inside a group the modifiers count in order up to the first one that fails.
#[test]
fn a_group_stops_at_the_first_modifier_that_fails() {
    let s = world();
    let block = script::parse("factor = 1 group = { modifier = { factor = 0.5 literacy = 0.4 } modifier = { factor = 2 literacy = 0.9 } modifier = { factor = 4 literacy = 0.1 } } modifier = { factor = 8 literacy = 0.1 }").unwrap();
    // 1 + 0.5; the second of the group fails, so its third is not looked at; the loose one counts.
    assert_eq!(additive_weight(&s, &block.root, &Scope::Pop(1, 0)), 9.5);
    // As a chance: the block's own factor times the modifiers.
    assert_eq!(wiktoria_simulation::pops::chance_value(&s, &script::parse("factor = 0.25 modifier = { factor = 2 literacy = 0.4 }").unwrap().root, &Scope::Pop(1, 0)), 0.5);
}

fn calendar() -> Calendar {
    Calendar::new(43_800_000, vec![31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]).unwrap()
}

#[test]
fn due_pops_promote_and_demote_by_the_script_weights() {
    let mut s = world();
    let chances = script::parse("promotion_chance = { factor = 0.5 modifier = { factor = 1 literacy = 0.4 } } demotion_chance = { factor = 1 modifier = { factor = 0.5 strata = poor } }").unwrap();
    let mut defs = MobilityDefs::default().with_chances(&chances.root);
    // Farmers: up to owners (clerks have no weight) or sideways to soldiers. The state's national
    // focus encourages owners, so everybody who is promoted becomes one; demotion cannot go up,
    // so it goes to the soldiers.
    defs.focus.insert("promote_owners".into(), ("owners".into(), 0.2));
    s.provinces.get_mut(&1).unwrap().region = Some(0);
    s.countries.get_mut("AAA").unwrap().national_focus.insert(0, "promote_owners".into());
    defs.add_pop_type("farmers", &script::parse("promote_to = { clerks = { factor = 0 } owners = { factor = 3 } soldiers = { factor = 2 } }").unwrap().root);
    defs.add_pop_type("clerks", &script::parse("promote_to = { farmers = { factor = 1 } }").unwrap().root);
    // Bureaucrats are 2,000 of 153,000 people, more than the 1% that gives full administrative
    // efficiency (their id 7 is not due today).
    s.provinces.get_mut(&1).unwrap().pops.push(PopState { id: 7, pop_type: "bureaucrats".into(), size: 2_000.0, culture: "alpha".into(), religion: "one".into(), ..Default::default() });
    s.provinces.get_mut(&1).unwrap().pops[0].money = 1_000.0;
    // Angry farmers: the pops they found start calm all the same.
    (s.provinces.get_mut(&1).unwrap().pops[0].militancy, s.provinces.get_mut(&1).unwrap().pops[0].consciousness) = (8.0, 5.0);
    let rules = mobility();
    let cal = calendar();
    let flows = move_due_pops(&mut s, &defs, &rules, &PopSchedule { period: 31, offset: 1 }, &cal);
    // 100,000 farmers. Promotion: block factor 0.5 x modifiers 1, plus the focus 0.2 = 0.7;
    // x 0.002 = 140 people. Demotion: 1 x 0.5 plus the focus = 0.7 as well: 140 people. Each
    // lot goes to one target type.
    let want: BTreeMap<(String, String), f64> = [(("farmers", "owners"), 140.0), (("farmers", "soldiers"), 140.0)].into_iter().map(|((a, b), n)| ((a.to_string(), b.to_string()), n)).collect();
    assert_eq!(flows, want, "the farmers with id 5 are not due");
    let pops = &s.provinces[&1].pops;
    assert_eq!((pops[0].size, pops[1].size, pops[2].size), (99_720.0, 1_000.0, 50_000.0));
    let new: Vec<(&str, f64)> = pops[4..].iter().map(|p| (p.pop_type.as_str(), p.size)).collect();
    assert_eq!(new, [("owners", 140.0), ("soldiers", 140.0)], "new pops keep culture and religion");
    assert!(pops[4..].iter().all(|p| p.culture == "alpha" && p.id > 31));
    assert!(pops[4..].iter().all(|p| p.militancy == 0.0 && p.consciousness == 0.0), "a new pop starts at the rules file's mood (0)");
    assert_eq!((pops[0].militancy, pops[0].consciousness), (8.0, 5.0));
    assert_eq!(pops.iter().map(|p| p.size).sum::<f64>(), 153_000.0, "nobody is lost");
    // The movers take their share of the cash along.
    assert!((pops[4].money - 1.4).abs() < 1e-9 && (pops.iter().map(|p| p.money).sum::<f64>() - 1_000.0).abs() < 1e-9);
    // The 1,000 clerks: 0.002 x 0.5 x 1,000 = 1 person waits; one alone does not move.
    assert_eq!((pops[1].promotion_due, pops[1].demotion_due), (1.0, 0.0));
    // A month later two are waiting, but clerks have no target above them: the lot is dropped.
    s.date = Date::new(1836, 3, 1);
    let flows = move_due_pops(&mut s, &defs, &rules, &PopSchedule { period: 31, offset: 1 }, &cal);
    assert_eq!(flows[&("farmers".to_string(), "owners".to_string())], 139.0);
    let pops = &s.provinces[&1].pops;
    assert_eq!((pops[1].size, pops[1].promotion_due), (1_000.0, 0.0));
    assert_eq!(pops[4].size, 279.0, "the next lot joins the pop founded last month");
    assert_eq!(pops[4].militancy, 0.0, "people who join a pop take its mood, they do not bring theirs");
}

#[test]
fn focus_small_lots_and_jobs_shape_promotion() {
    let mut s = world();
    let chances = script::parse("promotion_chance = { factor = 0.5 modifier = { factor = 1 literacy = 0.4 } } demotion_chance = { factor = 1 modifier = { factor = 0.5 strata = poor } }").unwrap();
    let mut defs = MobilityDefs::default().with_chances(&chances.root);
    defs.focus.insert("promote_soldiers".into(), ("soldiers".into(), 0.2));
    s.strata.insert("craftsmen".into(), "poor".into());
    let p = s.provinces.get_mut(&1).unwrap();
    p.region = Some(0);
    // Due today: 100,000 soldiers (the focused type) and 300 farmers. No bureaucrats: no
    // administrative efficiency.
    (p.pops[0].pop_type, p.pops[1].pop_type, p.pops[1].size) = ("soldiers".into(), "farmers".into(), 300.0);
    (p.pops[1].culture, p.pops[2].culture) = ("beta".into(), "beta".into());
    s.countries.get_mut("AAA").unwrap().national_focus.insert(0, "promote_soldiers".into());
    defs.add_pop_type("soldiers", &script::parse("promote_to = { farmers = { factor = 1 } craftsmen = { factor = 5 } }").unwrap().root);
    defs.add_pop_type("farmers", &script::parse("promote_to = { soldiers = { factor = 1 } }").unwrap().root);
    let flows = move_due_pops(&mut s, &defs, &mobility(), &PopSchedule { period: 31, offset: 1 }, &calendar());
    // The soldiers do not promote (the focus is on them) and the focus does not pull them down
    // either: demotion 0.5 x 0.002 = 100 people. Craftsmen have no work in a state without a
    // factory, so all go to the farmers: with no farmers of their own culture in the province
    // they join the first pop of farmers there...
    assert_eq!(flows, BTreeMap::from([(("soldiers".to_string(), "farmers".to_string()), 100.0)]));
    let pops = &s.provinces[&1].pops;
    assert_eq!((pops[0].size, pops[0].promotion_due, pops[0].demotion_due), (99_900.0, 0.0, 0.0));
    // ...the 300 who are due as well. Of those, less than one person would promote or demote:
    // one waits for each, and one alone does not move.
    assert_eq!((pops[1].size, pops[1].promotion_due, pops[1].demotion_due), (400.0, 1.0, 1.0));
    assert_eq!(pops.len(), 3);
}

/// the original's steps of 1/32768 for a few clergy shares and spending levels.
#[test]
fn literacy_follows_clergy_and_spending() {
    let r = LiteracyRules { scale: 0.01, speed: 0.1, base_clergy: 0.005, max_clergy: 0.04, min: 0.01, max: 1.0 };
    let steps = |clergy: f64, spending: f64| (literacy_change(&r, clergy, spending, 0.0) * 32768.0).round();
    // Russia: 20% spending, 2% clergy -> 2 steps (0.00006); Austria: 50%, 1% -> 2 steps.
    assert_eq!((steps(0.02, 0.2), steps(0.01, 0.5)), (2.0, 2.0));
    // Twice the spending, twice the change. The share above the base counts up to 4%: the most
    // is 0.04 / 0.035 of the speed, reached at 4.5% clergy.
    assert_eq!((steps(0.04, 0.5), steps(0.04, 1.0), steps(0.045, 1.0), steps(0.08, 1.0)), (16.0, 32.0, 37.0, 37.0));
    // At the base share nothing changes; below it literacy falls.
    assert_eq!(steps(0.005, 1.0), 0.0);
    // Below the base share literacy falls, by the same amount whatever is spent.
    assert!(literacy_change(&r, 0.0, 1.0, 0.0) < 0.0);
    assert_eq!(literacy_change(&r, 0.0, 1.0, 0.0), literacy_change(&r, 0.0, 0.2, 0.5));
    // Efficiency modifiers scale the change.
    assert_eq!((literacy_change(&r, 0.04, 1.0, 0.5) * 32768.0).round(), 49.0);
}

fn mobility() -> MobilityRules {
    MobilityRules { scale: 0.002, promotion_tune: 1.0, demotion_tune: 1.0, assimilation_scale: 0.004, immigration_scale: 0.004, admin_base: 0.2, max_bureaucracy: 0.01, whole_pop: 100.0, targets: 3, assimilation_state: 1.0, assimilation_colony: 0.5, migration_tune: 1.0, emigration_tune: 1.0, new_pop_mood: 0.0 }
}

#[test]
fn assimilation_respects_cores_and_accepted_cultures() {
    let mut s = world();
    s.countries.insert("AAA".into(), CountryState { primary_culture: Some("alpha".into()), accepted_cultures: vec!["beta".into()], civilized: true, ..Default::default() });
    s.countries.insert("GGG".into(), CountryState { primary_culture: Some("gamma".into()), ..Default::default() });
    let pop = |id: u64, culture: &str| PopState { id, pop_type: "farmers".into(), size: 100_000.0, culture: culture.into(), religion: "one".into(), ..Default::default() };
    // All due on the 1st: primary, accepted, a foreign culture, and one protected by a core.
    let p = s.provinces.get_mut(&1).unwrap();
    p.pops = vec![pop(0, "alpha"), pop(31, "beta"), pop(62, "delta"), pop(93, "gamma")];
    p.cores = vec!["AAA".into(), "GGG".into()];
    let chances = script::parse("assimilation_chance = { factor = 1 modifier = { factor = -20 has_culture_core = yes } }").unwrap();
    let defs = MobilityDefs::default().with_chances(&chances.root);
    let cal = Calendar::new(43_800_000, vec![31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]).unwrap();
    let moved = assimilate_due_pops(&mut s, &defs, &mobility(), &PopSchedule { period: 31, offset: 1 }, &cal);
    // Only the delta pop: 100,000 x 0.004 x 1 = 400 join the alpha pop.
    assert_eq!(moved, 400.0);
    let sizes: Vec<f64> = s.provinces[&1].pops.iter().map(|q| q.size).collect();
    assert_eq!(sizes, [100_400.0, 100_000.0, 99_600.0, 100_000.0], "gamma keeps its people: GGG has a core here and gamma is its primary culture");
}

#[test]
fn administrative_efficiency_follows_the_bureaucrats() {
    let mut s = world();
    let r = mobility();
    assert_eq!(admin_efficiency(&s, &r)["AAA"], 0.0, "no bureaucrats: none");
    // 755 bureaucrats among 151,755 people = 0.4975% of the 1% that gives the full benefit.
    s.provinces.get_mut(&1).unwrap().pops.push(PopState { id: 7, pop_type: "bureaucrats".into(), size: 755.0, ..Default::default() });
    assert!((admin_efficiency(&s, &r)["AAA"] - 755.0 / 151_755.0 / 0.01).abs() < 1e-12);
    s.provinces.get_mut(&1).unwrap().pops[3].size = 50_000.0;
    assert_eq!(admin_efficiency(&s, &r)["AAA"], 1.0, "never above 1");
}

#[test]
fn migrants_join_the_country_pool_and_are_spread() {
    let mut s = world();
    s.countries.insert("AAA".into(), CountryState { civilized: true, capital: Some(1), ..Default::default() });
    s.countries.insert("BBB".into(), CountryState { civilized: true, capital: Some(3), ..Default::default() });
    // An uncivilised country that would score as well as BBB: out of the draw.
    s.countries.insert("CCC".into(), CountryState { capital: Some(5), ..Default::default() });
    // AAA also owns provinces 2 and 4; BBB owns 3 and CCC owns 5 on another continent.
    let soldiers = |id: u64| PopState { id, pop_type: "soldiers".into(), size: 50.0, culture: "alpha".into(), religion: "one".into(), ..Default::default() };
    s.provinces.get_mut(&1).unwrap().continent = Some("old".into());
    s.provinces.get_mut(&1).unwrap().pops.push(soldiers(62));
    s.provinces.insert(2, ProvinceState { owner: Some("AAA".into()), life_rating: 40.0, ..Default::default() });
    s.provinces.insert(3, ProvinceState { owner: Some("BBB".into()), continent: Some("new".into()), ..Default::default() });
    s.provinces.insert(4, ProvinceState { owner: Some("AAA".into()), pops: vec![soldiers(93)], ..Default::default() });
    s.provinces.insert(5, ProvinceState { owner: Some("CCC".into()), continent: Some("new".into()), ..Default::default() });
    // The 1,000 clerks are not due today.
    s.provinces.get_mut(&1).unwrap().pops[1].id = 6;
    let chances = script::parse("migration_chance = { factor = 0.5 } emigration_chance = { factor = 0.25 }").unwrap();
    let mut defs = MobilityDefs::default().with_chances(&chances.root);
    for kind in ["farmers", "soldiers"] {
        defs.add_pop_type(kind, &script::parse("migration_target = { factor = 1 modifier = { factor = 3 life_rating = 40 } } country_migration_target = { factor = 1 modifier = { factor = 0 NOT = { continent = new } } }").unwrap().root);
    }
    let people = |s: &State, id: u16, kind: &str| s.provinces[&id].pops.iter().filter(|q| q.pop_type == kind).map(|q| q.size).sum::<f64>();
    let (within, abroad) = migrate_due_pops(&mut s, &defs, &mobility(), &PopSchedule { period: 31, offset: 1 }, &calendar(), &mut TargetCache::default(), &wiktoria_data::modifiers::ModifierLibrary::default());
    // The 100,000 farmers due: 100,000 x 0.004 x 0.5 = 200 into AAA's pool, x 0.25 = 100 abroad,
    // all to BBB. The 50 soldiers of province 1 (ids 62 and 93 are due) leave whole; the 50 who
    // are alone in province 4 stay.
    assert_eq!(within, 250.0);
    assert_eq!(abroad, BTreeMap::from([(("AAA".to_string(), "BBB".to_string()), 100.0)]));
    assert_eq!((people(&s, 3, "farmers"), people(&s, 5, "farmers")), (100.0, 0.0));
    assert_eq!(s.provinces[&3].pops[0].culture, "alpha", "migrants keep their culture");
    // AAA's pool of 200 farmers over the weights 3, 1, 1: 120 to province 2, then 16 of the 80
    // left to province 1, 12 of the 64 left to province 4, and the last 52 to one of the three.
    let got = [people(&s, 2, "farmers"), people(&s, 1, "farmers") - 149_700.0, people(&s, 4, "farmers")];
    assert!([[172.0, 16.0, 12.0], [120.0, 68.0, 12.0], [120.0, 16.0, 64.0]].contains(&got), "{got:?}");
    // The pool of 50 soldiers: 30 to province 2; the 20 left give nobody a share above 10 and go
    // to one province together.
    assert_eq!(s.provinces[&1].pops.iter().find(|q| q.id == 62).unwrap().size, 0.0);
    assert!(people(&s, 2, "soldiers") >= 30.0 && people(&s, 4, "soldiers") >= 50.0);
    assert_eq!(s.provinces.values().flat_map(|p| &p.pops).map(|q| q.size).sum::<f64>(), 151_100.0, "nobody is lost");
}

/// A province that pushes its people out counts once for moving within the country and twice
/// for going abroad.
#[test]
fn a_push_counts_twice_for_emigrants() {
    let mut s = world();
    s.countries.insert("AAA".into(), CountryState { civilized: true, capital: Some(1), ..Default::default() });
    s.countries.insert("BBB".into(), CountryState { civilized: true, capital: Some(3), ..Default::default() });
    s.provinces.get_mut(&1).unwrap().continent = Some("old".into());
    s.provinces.get_mut(&1).unwrap().modifiers.push(wiktoria_simulation::script::TimedModifier { name: "famine".into(), expires: None });
    s.provinces.get_mut(&1).unwrap().pops.truncate(1);
    s.provinces.insert(2, ProvinceState { owner: Some("AAA".into()), ..Default::default() });
    s.provinces.insert(3, ProvinceState { owner: Some("BBB".into()), continent: Some("new".into()), ..Default::default() });
    let mut lib = wiktoria_data::modifiers::ModifierLibrary::default();
    lib.event_modifiers.insert("famine".into(), [("immigrant_push".to_string(), 0.5)].into());
    let chances = script::parse("migration_chance = { factor = 0.5 } emigration_chance = { factor = 0.25 }").unwrap();
    let mut defs = MobilityDefs::default().with_chances(&chances.root);
    defs.add_pop_type("farmers", &script::parse("migration_target = { factor = 1 } country_migration_target = { factor = 1 }").unwrap().root);
    let (within, abroad) = migrate_due_pops(&mut s, &defs, &mobility(), &PopSchedule { period: 31, offset: 1 }, &calendar(), &mut TargetCache::default(), &lib);
    // 200 x 1.5 = 300 within; 100 x 1.5 x 1.5 = 225 abroad.
    assert_eq!((within, abroad[&("AAA".to_string(), "BBB".to_string())]), (300.0, 225.0));
}

#[test]
fn mood_follows_needs_clergy_and_culture() {
    let r = MoodRules { con_luxury: 0.1, con_literacy: 0.1, con_poor_clergy: -2.5, con_midrich_clergy: -1.25, con_colonial: 0.5, mil_no_life: 0.2, mil_lack_everyday: 0.1, mil_has_everyday: -0.1, mil_has_luxury: -0.2, mil_non_accepted: 0.05, mil_war_exhaustion: 0.005, mil_ideology: -0.1, mil_ruling_party: -0.1, mil_require_reform: 0.2, max: 10.0, reform_options: Vec::new() };
    let pop = |life: f64, everyday: f64, luxury: f64| PopState { life_needs: life, everyday_needs: everyday, luxury_needs: luxury, literacy: 0.1, ..Default::default() };
    let learned = |luxury: f64| PopState { luxury_needs: luxury, literacy: 0.5, ..Default::default() };
    let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
    let con = |poor: bool, clergy_share: f64, plurality: f64, colonial: bool| ConsciousnessInputs { poor, clergy_share, plurality, colonial, ..Default::default() };
    // A Russian labourer of 1836: 6% luxury and 10% literate are both too little to count (each
    // term must pass 0.01); clergy 1.2% x -2.5 = -0.03.
    assert!(close(consciousness_change(&r, &pop(1.0, 0.25, 0.06), &con(true, 0.012, 0.0, false)), -0.03));
    // Too few clergy to count: nothing lowers it.
    assert!(close(consciousness_change(&r, &pop(1.0, 0.25, 0.06), &con(true, 0.003, 0.0, false)), 0.0));
    // Middle and rich pops feel the clergy half as much. Literacy 50%: 0.05 x plurality 50% =
    // 0.025; luxury 0.1; clergy -0.025.
    assert!(close(consciousness_change(&r, &learned(1.0), &con(false, 0.02, 50.0, false)), 0.1));
    // A colonial province halves the three terms but not the modifiers.
    assert!(close(consciousness_change(&r, &learned(1.0), &ConsciousnessInputs { modifiers: 0.03, ..con(false, 0.02, 50.0, true) }), 0.05 + 0.03));
    // A free press (`literacy_con_impact` +0.25) raises the literacy term only.
    assert!(close(consciousness_change(&r, &learned(1.0), &ConsciousnessInputs { literacy_impact: 0.25, ..con(false, 0.02, 50.0, false) }), 0.1 + 0.00625));
    // Militancy: starving and not accepted rises; fed and content falls.
    let x = |accepted: bool, war_exhaustion: f64| MilitancyInputs { accepted, war_exhaustion, ..Default::default() };
    assert!(close(militancy_change(&r, &pop(0.0, 0.0, 0.0), &x(false, 0.0)), 0.1 + 0.05 + 0.05));
    // Met life needs calm nobody; met everyday and luxury needs do.
    assert!(close(militancy_change(&r, &pop(1.0, 1.0, 1.0), &x(true, 0.0)), -0.05 - 0.1));
    // Needs close to one half move nothing: each term stays within 0.01.
    assert!(close(militancy_change(&r, &pop(0.46, 0.55, 0.54), &x(true, 0.0)), 0.0));
    assert!(close(militancy_change(&r, &pop(0.46, 0.45, 0.0), &x(true, 0.0)), 0.0));
    // War exhaustion stirs only pops that hold party options with a `war_exhaustion_effect`.
    assert!(close(militancy_change(&r, &pop(1.0, 0.5, 0.0), &x(true, 20.0)), 0.0));
    assert!(close(militancy_change(&r, &pop(1.0, 0.5, 0.0), &MilitancyInputs { war_support: 0.6, ..x(true, 20.0) }), 0.06));
    // The original's tooltip for a well-fed pop: -0.05 everyday, -0.02 conservative, -0.02 ruling
    // party, +0.17 reform desire, -0.01 from a country modifier = +0.07.
    let shown = MilitancyInputs { accepted: true, conservative: 0.2, ruling: 0.2, reform_want: 0.85, modifiers: -0.01, ..Default::default() };
    assert!(close(militancy_change(&r, &pop(1.0, 1.0, 0.0), &shown), 0.07));
    // In a colonial province the wish for reforms does not count.
    assert!(close(militancy_change(&r, &pop(1.0, 1.0, 0.0), &MilitancyInputs { colonial: true, ..shown.clone() }), -0.1));
    // A Russian artisan of 1836 with nothing, culture not accepted, 70% conservative under a
    // conservative party: the original adds 0.0612.
    let russian = MilitancyInputs { accepted: false, conservative: 0.7, ruling: 0.7, ..Default::default() };
    assert!((militancy_change(&r, &pop(0.0, 0.0, 0.0), &russian) - 0.0612).abs() < 0.002);
}

/// The monthly update with the modifiers of country and province switched on.
#[test]
fn mood_update_adds_the_modifiers_of_country_and_province() {
    let r = MoodRules { con_luxury: 0.1, con_literacy: 0.1, con_poor_clergy: -2.5, con_midrich_clergy: -1.25, con_colonial: 0.5, mil_no_life: 0.2, mil_lack_everyday: 0.1, mil_has_everyday: -0.1, mil_has_luxury: -0.2, mil_non_accepted: 0.05, mil_war_exhaustion: 0.005, mil_ideology: -0.1, mil_ruling_party: -0.1, mil_require_reform: 0.2, max: 10.0, reform_options: vec![false, true] };
    let mut s = world();
    s.countries.insert("AAA".into(), CountryState { primary_culture: Some("alpha".into()), ..Default::default() });
    s.countries.get_mut("AAA").unwrap().modifiers.push(wiktoria_simulation::script::TimedModifier { name: "unrest".into(), expires: None });
    s.provinces.get_mut(&1).unwrap().modifiers.push(wiktoria_simulation::script::TimedModifier { name: "pamphlets".into(), expires: None });
    let mut lib = wiktoria_data::modifiers::ModifierLibrary::default();
    lib.event_modifiers.insert("unrest".into(), [("global_pop_consciousness_modifier".to_string(), 0.05), ("core_pop_consciousness_modifier".to_string(), 0.02), ("non_accepted_pop_consciousness_modifier".to_string(), 0.5), ("global_pop_militancy_modifier".to_string(), 0.03)].into());
    lib.event_modifiers.insert("pamphlets".into(), [("pop_consciousness_modifier".to_string(), 0.1)].into());
    // The farmers due (id 0): needs at one half, literacy 50% with plurality 0, no clergy, an
    // accepted culture; a quarter of their issue support sits on a reform option.
    let pop = &mut s.provinces.get_mut(&1).unwrap().pops[0];
    (pop.life_needs, pop.everyday_needs, pop.issues) = (0.5, 0.5, [(1, 75.0), (2, 25.0)].into());
    wiktoria_simulation::pops::mood_due_pops(&mut s, &r, &PopSchedule { period: 31, offset: 1 }, &calendar(), &lib);
    let pops = &s.provinces[&1].pops;
    assert!((pops[0].consciousness - 0.17).abs() < 1e-12, "province 0.1 + global 0.05 + not colonial 0.02");
    assert!((pops[0].militancy - (0.03 + 0.25 * 0.2)).abs() < 1e-12);
    assert_eq!((pops[2].consciousness, pops[2].militancy), (0.0, 0.0), "id 5 is not due");
}

#[test]
fn plurality_and_infamy_drift_monthly() {
    let mut s = world();
    s.countries.insert("AAA".into(), CountryState { plurality: 25.0, badboy: 0.25, ..Default::default() });
    s.countries.insert("BBB".into(), CountryState { plurality: 99.99, badboy: 5.0, ..Default::default() });
    s.provinces.insert(2, ProvinceState { owner: Some("BBB".into()), pops: vec![PopState { size: 10.0, consciousness: 8.0, ..Default::default() }], ..Default::default() });
    // AAA: 100,000 at consciousness 3 and 51,000 at 0 -> average 1.9868.
    s.provinces.get_mut(&1).unwrap().pops[0].consciousness = 3.0;
    s.wars = vec![(["BBB".to_string()].into(), ["ZZZ".to_string()].into())];
    let rules = CountryRules { plurality_per_consciousness: 0.0218, plurality_max: 100.0, badboy_peace: -0.1, badboy_war: -0.033, badboy_min: 0.0, prestige_great_power: 0.05, prestige_civilized: 0.01, tech_prestige: [("romanticism".to_string(), 0.05)].into(), ..Default::default() };
    s.great_powers = vec!["AAA".into()];
    s.countries.get_mut("AAA").unwrap().technologies.insert("romanticism".into());
    s.countries.get_mut("BBB").unwrap().civilized = true;
    let lib = wiktoria_data::modifiers::ModifierLibrary::default();
    for _ in 0..3 {
        country_month(&mut s, &rules, &lib, 30.0);
    }
    let a = &s.countries["AAA"];
    assert!((a.plurality - (25.0 + 3.0 * 0.0218 * 300_000.0 / 151_000.0)).abs() < 1e-9);
    assert_eq!(a.badboy, 0.0, "0.25 - 0.3 stops at zero");
    let b = &s.countries["BBB"];
    assert_eq!(b.plurality, 100.0, "never above the limit");
    assert!((b.badboy - (5.0 - 3.0 * 0.033)).abs() < 1e-9, "slower at war");
    // Prestige: a great power with one prestige technology gains 0.0525 a month (the original
    // shows 0.052), another civilized country 0.01.
    assert!((s.countries["AAA"].prestige - 3.0 * 0.0525).abs() < 1e-9);
    assert!((s.countries["BBB"].prestige - 3.0 * 0.01).abs() < 1e-9);
}

#[test]
fn research_points_match_the_originals_tooltips() {
    let r = CountryRules {
        research_pops: vec![("clergymen".into(), 3.0, 0.02), ("clerks".into(), 2.0, 0.04)],
        research_great_power: 1.5,
        research_civilized: 1.0,
        research_literacy: 4.0,
        research_per_plurality: 2.0,
        ..Default::default()
    };
    let shares = |clergy: f64, clerks: f64| -> std::collections::BTreeMap<String, f64> { [("clergymen".to_string(), clergy), ("clerks".to_string(), clerks)].into() };
    // Austria, 1 January 1836: clergy 1.06%, great power, literacy 16.1%, plurality 25.05, one
    // technology with +50%: the original shows 7.47 a day.
    assert!((r.research_per_day(&shares(0.0106, 0.0), 1.5, 0.161, 25.05, 0.5) - 7.47).abs() < 0.02);
    // The Chinese Empire, 1836: clergy 1.00%, not civilized, literacy 3.5%, plurality 1: 1.68.
    assert!((r.research_per_day(&shares(0.01, 0.0), 0.0, 0.035, 1.0, 0.0) - 1.68).abs() < 0.01);
    // Austria-Hungary, 1900 (a secondary power, +1.25): clergy over the optimum, clerks 0.11%,
    // literacy 40.3%, plurality bonus 147.3%, technologies +350%: 35.33.
    assert!((r.research_per_day(&shares(0.0246, 0.0011), 1.25, 0.403, 73.65, 3.5) - 35.33).abs() < 0.05);
}

#[test]
fn inventions_are_rolled_monthly_and_pay_plurality_once() {
    let block = |src: &str| wiktoria_data::script::parse(src).unwrap().root;
    let mut s = world();
    s.countries.insert("AAA".into(), CountryState { plurality: 20.0, civilized: true, national_value: Some("nv_order".into()), ..Default::default() });
    s.technologies = ["positivism".to_string(), "romanticism".to_string()].into();
    s.inventions = vec!["sure".into(), "never".into(), "locked".into()];
    s.countries.get_mut("AAA").unwrap().technologies.insert("positivism".into());
    // The original's tooltip: base 2, +3 for Liberty (not met), -1 for Order (met) = 1%.
    let shown = block("base = 2 modifier = { factor = 3 nationalvalue = nv_liberty } modifier = { factor = -1 nationalvalue = nv_order }");
    assert_eq!(invention_chance(&s, &shown, &Scope::Country("AAA".into())), 1.0);
    let rules = InventionRules {
        inventions: vec![
            (Some(block("positivism = 1")), Some(block("base = 100")), 0.05),
            (Some(block("positivism = 1")), Some(block("base = 0")), 0.10),
            (Some(block("romanticism = 1")), Some(block("base = 100")), 0.10),
        ],
        plurality_scale: 100.0,
        plurality_max: 100.0,
    };
    assert_eq!(discover_inventions(&mut s, &rules), vec![("AAA".to_string(), 1)]);
    // Found once, paid once: 0.05 in the file is 5 points of plurality.
    assert!(discover_inventions(&mut s, &rules).is_empty());
    let a = &s.countries["AAA"];
    assert!(a.inventions.contains(&1) && !a.inventions.contains(&2) && !a.inventions.contains(&3));
    assert_eq!(a.plurality, 25.0);
}

#[test]
fn research_points_are_spent_on_technologies() {
    use wiktoria_simulation::research::{research_day, ResearchRules, TechInfo};
    let tech = |name: &str, year: i32, cost: f64, previous: Option<&str>| TechInfo { name: name.into(), folder: "army_tech".into(), year, cost, ai_chance: None, previous: previous.map(str::to_string) };
    let rules = ResearchRules { year_span: 140.0, max_daily: 100.0, max_saved: 25000.0, techs: vec![tech("first", 1836, 3600.0, None), tech("second", 1836, 7200.0, Some("first")), tech("later", 1900, 100.0, None)], min_cost: 1.0, store_days: 365.0, weights: wiktoria_simulation::ai::ResearchWeights { overdue_per_year: 0.005, min_top_cost: 3600.0 } };
    // The original's saves: a technology of 1836 costs 0.9 of its price in 1850 and 0.7579 in
    // 1870; a school with +15% for the folder divides by 1.15 (7200 -> 6260 in 1836).
    assert!((rules.cost(&rules.techs[0], 1850, 0.0) - 3240.0).abs() < 1e-9);
    assert!((rules.cost(&rules.techs[1], 1870, 0.0) / 7200.0 - 0.7571).abs() < 1e-3);
    assert!((rules.cost(&rules.techs[1], 1836, 0.15) - 6260.87).abs() < 0.01);
    let mut s = world();
    s.countries.insert("AAA".into(), CountryState { civilized: true, research_rate: 40.0, ..Default::default() });
    let lib = wiktoria_data::modifiers::ModifierLibrary::default();
    // 3600 points at 40 a day: done on day 90. Only "first" may be picked: "second" needs it,
    // "later" is not due until 1900.
    let mut done = Vec::new();
    for _ in 0..90 {
        done.extend(research_day(&mut s, &rules, &lib));
    }
    assert_eq!(done, vec![("AAA".to_string(), "first".to_string())]);
    assert!(s.countries["AAA"].technologies.contains("first"));
    research_day(&mut s, &rules, &lib);
    assert_eq!(s.countries["AAA"].research.as_ref().map(|r| r.0.as_str()), Some("second"));
    // An uncivilized country only saves its points up.
    s.countries.get_mut("AAA").unwrap().civilized = false;
    research_day(&mut s, &rules, &lib);
    assert_eq!(s.countries["AAA"].research_points, 40.0);
}

#[test]
fn ideology_drifts_in_steps_towards_the_weights() {
    // A Russian pop of the original, February 1836: reactionary 11.43094, conservative 69.52023,
    // liberal 19.04880 became 11.46945, 69.45081, 19.07971.
    let rules = wiktoria_simulation::pops::IdeologyRules { step: 0.05, big_step: 0.25, big_ratio: 2.0 };
    let shares: BTreeMap<u32, f64> = [(2, 11.43094), (3, 69.52023), (6, 19.04880)].into();
    let out = wiktoria_simulation::pops::ideology_drift(&rules, &shares, &[(2, 0.8785), (3, 2.9474), (6, 1.4641)]);
    for (n, want) in [(2, 11.46945), (3, 69.45081), (6, 19.07971)] {
        assert!((out[&n] - want).abs() < 2e-4, "{n}: {} vs {want}", out[&n]);
    }
    // A weight share above twice the pop's share takes the big step; an ideology not open stays.
    let out = wiktoria_simulation::pops::ideology_drift(&rules, &[(2, 10.0), (3, 90.0)].into(), &[(2, 1.0), (3, 1.0)]);
    assert!((out[&2] - 10.25 / 1.0025).abs() < 1e-9);
}

#[test]
fn ideology_far_above_its_weight_loses_the_big_step() {
    let rules = wiktoria_simulation::pops::IdeologyRules { step: 0.05, big_step: 0.25, big_ratio: 2.0 };
    // Weights 9 : 1 against shares 10 : 90. The first is below half its target: +0.25. The
    // second holds more than twice its target of 10: -0.25.
    let out = wiktoria_simulation::pops::ideology_drift(&rules, &[(2, 10.0), (3, 90.0)].into(), &[(2, 9.0), (3, 1.0)]);
    assert!((out[&2] - 10.25).abs() < 1e-9 && (out[&3] - 89.75).abs() < 1e-9);
    // Exactly twice the target is not "more than twice": nothing is lost.
    let out = wiktoria_simulation::pops::ideology_drift(&rules, &[(2, 80.0), (3, 20.0)].into(), &[(2, 4.0), (3, 6.0)]);
    assert!((out[&2] - 80.0 / 1.0025).abs() < 1e-9 && (out[&3] - 20.25 / 1.0025).abs() < 1e-9);
    // An ideology that is not open (4) is not stepped, whatever its share; it is only rescaled
    // with the rest. Of the open ones the second is below its target of 50: +0.05.
    let out = wiktoria_simulation::pops::ideology_drift(&rules, &[(2, 50.0), (3, 30.0), (4, 20.0)].into(), &[(2, 1.0), (3, 1.0)]);
    assert!((out[&4] - 20.0 / 1.0005).abs() < 1e-9 && (out[&3] - 30.05 / 1.0005).abs() < 1e-9 && (out[&2] - 50.0 / 1.0005).abs() < 1e-9);
}

fn issue_world() -> State {
    let mut s = world();
    let issues = [("protect", "trade", "party_issues"), ("free", "trade", "party_issues"), ("jingo", "war", "party_issues"), ("pacifist", "war", "party_issues"), ("none", "vote", "political_reforms"), ("some", "vote", "political_reforms"), ("all", "vote", "political_reforms"), ("no_school", "school", "social_reforms"), ("low_school", "school", "social_reforms"), ("good_school", "school", "social_reforms"), ("no_arms", "arms", "military_reforms"), ("some_arms", "arms", "military_reforms")];
    s.issues = issues.iter().map(|x| (x.0.to_string(), x.1.to_string())).collect();
    s.issue_categories = issues.iter().map(|x| x.2.to_string()).collect();
    s.countries.get_mut("AAA").unwrap().reforms = [("trade", "protect"), ("vote", "none"), ("school", "low_school"), ("arms", "no_arms")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    s
}

#[test]
fn a_country_lists_party_options_and_the_next_step_of_each_reform() {
    let mut s = issue_world();
    s.countries.get_mut("AAA").unwrap().civilized = true;
    let lists = issue_lists(&s, &s.countries["AAA"]);
    // The first party option is left out (the original never steps it). Of the vote group the
    // step after "none", of the school group the one after "low_school".
    assert_eq!(lists.candidates, [(2, IssueKind::Party), (3, IssueKind::Party), (4, IssueKind::Party), (6, IssueKind::Political), (10, IssueKind::Social)]);
    // Every other option of those groups is wiped, but for the first of them (5).
    assert_eq!(lists.excluded, [7, 8, 9]);
    // An uncivilised country: the next step of its own kind of reforms is listed (and always
    // wiped); political and social options are left alone.
    s.countries.get_mut("AAA").unwrap().civilized = false;
    let lists = issue_lists(&s, &s.countries["AAA"]);
    assert_eq!(lists.candidates, [(2, IssueKind::Party), (3, IssueKind::Party), (4, IssueKind::Party), (12, IssueKind::Other)]);
    assert!(lists.excluded.is_empty(), "its only excluded option is the first of the list");
}

#[test]
fn issues_step_towards_the_weights_and_reforms_by_their_desire() {
    let lists = IssueLists { candidates: vec![(2, IssueKind::Party), (3, IssueKind::Party), (4, IssueKind::Party), (6, IssueKind::Political), (10, IssueKind::Social)], excluded: vec![7, 8], speed: 2.0, political: 1.1, social: 1.2, civilized: true };
    let shares: BTreeMap<u32, f64> = [(1, 40.0), (2, 20.0), (3, 10.0), (4, 10.0), (5, 5.0), (7, 5.0), (8, 10.0)].into();
    // Targets of the candidates: 20, 60, 0, 20, 0.
    let out = issue_drift(&lists, &shares, &[1.0, 3.0, 0.0, 1.0, 0.0]);
    // 1 is never stepped; 2 is at its target: up the small step 0.05 x 2; 3 is below a fifth of
    // its target: a whole point; 4 is above its target: down the small step; 5 is the excluded
    // option that is never wiped; 7 and 8 are wiped; 6 (political) starts with a whole point x
    // 1.1; 10 (social) has neither weight nor support: up the small step x 1.2.
    let want = [(1, 40.0), (2, 20.1), (3, 11.0), (4, 9.9), (5, 5.0), (6, 1.1), (10, 0.12)];
    let sum: f64 = want.iter().map(|w| w.1).sum();
    assert_eq!(out.keys().copied().collect::<Vec<_>>(), want.iter().map(|w| w.0).collect::<Vec<_>>());
    for (n, x) in want {
        assert!((out[&n] - x * 100.0 / sum).abs() < 1e-9, "{n}: {} vs {}", out[&n], x * 100.0 / sum);
    }
    // Below half the target, not below a fifth: the big step 0.25 x 2. The option with nothing
    // that gained the small step last month loses it again.
    let out = issue_drift(&lists, &[(2, 10.0), (3, 89.88), (10, 0.12)].into(), &[1.0, 3.0, 0.0, 0.0, 0.0]);
    let sum = 10.5 + 89.78 + 0.1 + 0.11;
    assert!((out[&2] - 10.5 * 100.0 / sum).abs() < 1e-9 && (out[&3] - 89.78 * 100.0 / sum).abs() < 1e-9 && out.get(&10).copied().unwrap_or(0.0) < 1e-9);
    // In an uncivilised country only party options keep support.
    let wild = IssueLists { candidates: vec![(2, IssueKind::Party), (12, IssueKind::Other)], civilized: false, speed: 1.0, political: 1.0, social: 1.0, ..Default::default() };
    let out = issue_drift(&wild, &[(2, 50.0), (12, 50.0)].into(), &[1.0, 1.0]);
    assert!(out.len() == 1 && (out[&2] - 100.0).abs() < 1e-9);
}

#[test]
fn due_pops_move_their_issues_at_the_countrys_speed() {
    let mut s = issue_world();
    let mut defs = MobilityDefs::default();
    defs.add_pop_type("farmers", &script::parse("issues = { free = { factor = 1 } jingo = { factor = 1 modifier = { factor = 3 literacy = 0.4 } } }").unwrap().root);
    for q in s.provinces.get_mut(&1).unwrap().pops.iter_mut() {
        q.issues = [(1, 50.0), (2, 50.0)].into();
    }
    // AAA lets people meet: `issue_change_speed = 1` doubles the steps.
    s.countries.get_mut("AAA").unwrap().reforms.insert("meetings".into(), "yes_meeting".into());
    let mut lib = wiktoria_data::modifiers::ModifierLibrary::default();
    lib.issue_options.insert("yes_meeting".into(), [("issue_change_speed".to_string(), 1.0)].into());
    assert_eq!(issues_due_pops(&mut s, &defs, &PopSchedule { period: 31, offset: 1 }, &calendar(), &lib), 2);
    let pops = &s.provinces[&1].pops;
    // The farmers due: targets 25 for "free" and 75 for "jingo". "free" holds twice its target:
    // down 0.1; "jingo" has nothing: a whole point; "pacifist" has neither: up 0.1. The
    // uncivilised country's reform option stays at nothing.
    let sum = 50.0 + 49.9 + 1.0 + 0.1;
    assert!((pops[0].issues[&2] - 49.9 * 100.0 / sum).abs() < 1e-9 && (pops[0].issues[&3] - 100.0 / sum).abs() < 1e-9 && (pops[0].issues[&4] - 10.0 / sum).abs() < 1e-9);
    assert!(!pops[0].issues.contains_key(&12));
    assert_eq!(pops[2].issues, BTreeMap::from([(1, 50.0), (2, 50.0)]), "id 5 is not due");
}

/// The quick form of "due today" is `PopSchedule::is_due_in`, for small and huge pop ids.
#[test]
fn due_today_is_the_schedule() {
    for (offset, month_days) in [(1u64, 31u64), (1, 28), (5, 30), (0, 1), (3, 0)] {
        let schedule = PopSchedule { period: 31, offset };
        for day in 1..=31u8 {
            let today = wiktoria_simulation::pops::DueToday::new(&schedule, day, month_days);
            for id in (0..400u64).chain([u64::from(u32::MAX) - 1, u64::from(u32::MAX), u64::from(u32::MAX) + 1, u64::MAX - 7, u64::MAX]) {
                assert_eq!(today.is_due(id), schedule.is_due_in(id, day, month_days), "id {id} day {day} of {month_days}, offset {offset}");
            }
        }
    }
}

/// Several modifier sums at once are `country_modifier` key by key, to the last bit.
#[test]
fn country_modifiers_at_once_equal_one_by_one() {
    let values = |pairs: &[(&str, f64)]| pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect::<wiktoria_data::modifiers::ModifierValues>();
    let mut lib = wiktoria_data::modifiers::ModifierLibrary::default();
    lib.national_values.insert("order".into(), values(&[("a", 0.1), ("b", 0.3)]));
    lib.schools.insert("school".into(), values(&[("b", 0.7)]));
    lib.inventions = vec![("one".into(), values(&[("a", 0.2)])), ("two".into(), values(&[("c", 1.1)])), ("three".into(), values(&[("a", 0.05), ("c", 0.3)]))];
    lib.issue_options.insert("yes_meeting".into(), values(&[("a", 1.0)]));
    lib.issue_options.insert("free_press".into(), values(&[("b", 0.15), ("c", 0.01)]));
    lib.event_modifiers.insert("riot".into(), values(&[("a", 0.33), ("c", -0.2)]));
    let mut c = CountryState { national_value: Some("order".into()), tech_school: Some("school".into()), ..Default::default() };
    c.inventions = [1, 3, 9].into_iter().collect();
    c.reforms = [("meetings", "yes_meeting"), ("press", "free_press"), ("other", "unlisted")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    c.modifiers = vec![wiktoria_simulation::script::TimedModifier { name: "riot".into(), expires: None }, wiktoria_simulation::script::TimedModifier { name: "gone".into(), expires: None }];
    let keys = ["a", "b", "c", "d"];
    let at_once = wiktoria_simulation::pops::CountryModifiers::new(&lib, keys).of(&c);
    assert_eq!(at_once, keys.map(|k| wiktoria_simulation::pops::country_modifier(&c, &lib, k)));
    assert!(at_once[0] > 1.6 && at_once[3] == 0.0);
}

/// The compiled weights of the pop rules give what the string evaluator gives: the same sums
/// and products, and for single blocks the same answer or the same unknown key, in every scope.
#[test]
fn deep_trees_answer_like_the_string_evaluator() {
    use wiktoria_simulation::cond::{Asked, DeepConditions, DeepProduct, DeepSum, Facts, Memo};
    let mut s = world();
    s.regions = vec!["region".into()];
    s.slave_types.insert("slaves".into());
    s.factory_worker_types = ["clerks".to_string()].into();
    s.factory_types.insert("mill".into(), ("cloth".into(), 400.0));
    s.factories.push(wiktoria_simulation::script::FactoryState { owner: "AAA".into(), region: Some(0), kind: "mill".into(), level: 1.0, ..Default::default() });
    s.rgo_workers.insert("grain".into(), "farmers".into());
    s.ideologies = vec!["liberal".into(), "conservative".into()];
    s.issues = vec![("free".into(), "trade".into()), ("closed".into(), "trade".into()), ("none".into(), "vote".into()), ("some".into(), "vote".into())];
    s.issue_categories = vec!["party_issues".into(), "party_issues".into(), "political_reforms".into(), "political_reforms".into()];
    s.technologies.insert("steam".into());
    s.party_ideologies = vec!["liberal".into()];
    s.wars.push((["AAA".to_string()].into(), ["BBB".to_string()].into()));
    {
        let c = s.countries.get_mut("AAA").unwrap();
        (c.civilized, c.primary_culture, c.religion, c.ruling_party, c.capital, c.plurality) = (true, Some("alpha".into()), Some("one".into()), Some(1), Some(1), 12.0);
        c.reforms = [("trade", "free"), ("vote", "none")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        c.technologies.insert("steam".into());
        c.spending.insert("education_spending".into(), 0.6);
    }
    {
        let p = s.provinces.get_mut(&1).unwrap();
        (p.region, p.trade_goods, p.rgo_staff, p.life_rating, p.has_factory, p.continent, p.controller) = (Some(0), Some("grain".into()), Some(90_000.0), 35.0, true, Some("europe".into()), Some("AAA".into()));
        p.cores = vec!["AAA".into()];
        p.pops[0].ideology = [(1, 70.0), (2, 30.0)].into();
        p.pops[0].issues = [(1, 40.0), (4, 60.0)].into();
        (p.pops[0].militancy, p.pops[0].everyday_needs, p.pops[0].money) = (3.0, 0.4, 10.0);
        p.pops.push(PopState { id: 7, pop_type: "slaves".into(), size: 20_000.0, culture: "beta".into(), religion: "two".into(), life_needs: 0.3, ..Default::default() });
    }
    s.provinces.insert(2, ProvinceState { owner: Some("AAA".into()), region: Some(0), colonial: 2, pops: vec![PopState { id: 9, pop_type: "clerks".into(), size: 3_000.0, culture: "alpha".into(), religion: "one".into(), ..Default::default() }], ..Default::default() });
    s.region_members.insert(0, vec![1, 2]);
    let blocks = [
        "literacy = 0.4 strata = poor", "strata = middle", "culture = alpha", "culture = this", "religion = two", "religion = THIS", "education_spending = 0.5", "agree_with_ruling_party = 0.6", "revanchism = 0", "revanchism = 0.1",
        "is_primary_culture = yes", "is_accepted_culture = no", "is_next_reform = some", "is_next_reform = closed", "is_ideology_enabled = liberal", "is_ideology_enabled = other", "war_exhaustion = 0", "plurality = 10", "is_state_religion = yes",
        "political_reform_want = 0.5", "social_reform_want = 0.01", "ruling_party_ideology = liberal", "type = farmers", "has_pop_type = clerks", "continent = europe", "tag = AAA", "year = 1836", "month = 2", "province_id = 1",
        "life_rating = 30", "is_coastal = no", "is_colonial = yes", "is_colonial = no", "has_factories = yes", "is_slave = yes", "has_province_modifier = x", "has_country_modifier = x", "has_country_flag = x", "has_global_flag = x",
        "government = x", "total_pops = 1000", "produces = grain", "is_capital = yes", "unemployment = 0.05", "unemployment = 0.5", "poor_strata_life_needs = 0.8", "poor_strata_militancy = 1", "middle_strata_everyday_needs = 0",
        "militancy = 2", "life_needs = 0.5", "everyday_needs = 0.5", "civilized = yes", "war = yes", "war = no", "nationalvalue = x", "is_greater_power = no", "controlled_by = owner", "has_culture_core = yes", "can_build_factory = yes",
        "steam = 1", "farmers = 0.5", "clerks = 0.5", "liberal = 50", "some = 50", "trade = free", "vote = some", "terrain = hills", "is_state_capital = yes", "any_neighbor_country = { civilized = yes }",
        "unemployment_by_type = { type = farmers value = 0.05 }", "unemployment_by_type = { type = clerks value = 0.5 }", "any_owned_province = { life_rating = 30 NOT = { unemployment_by_type = { type = farmers value = 0.5 } } }",
        "state_scope = { has_factories = yes NOT = { unemployment = 0.9 } poor_strata_life_needs = 0.5 farmers = 0.5 }", "location = { is_colonial = no terrain = hills }", "country = { war = yes trade = free steam = 1 }",
        "owner = { unemployment = 0.01 }", "OR = { literacy = 0.9 AND = { militancy = 1 NOT = { strata = rich } } }", "NOT = { terrain = hills }", "factor = 2 literacy = 0.1",
    ];
    let scopes = [Scope::Pop(1, 0), Scope::Pop(1, 1), Scope::Pop(1, 3), Scope::Pop(2, 0), Scope::Province(1), Scope::Province(2), Scope::State(0, "AAA".into()), Scope::Country("AAA".into()), Scope::Country("BBB".into())];
    let (facts, memo) = (Facts::new(), Memo::new());
    for text in blocks {
        let block = script::parse(text).unwrap().root;
        let compiled = DeepConditions::new(&block);
        for scope in &scopes {
            let at = Asked::new(&s, scope);
            let expected = wiktoria_simulation::script::conditions_hold(&s, &block, scope);
            assert_eq!(compiled.hold(&s, Some(&facts), &memo, &at), expected, "{text} for {scope:?}");
            assert_eq!(compiled.hold(&s, None, &Memo::new(), &Asked::new(&s, scope)), expected, "{text} for {scope:?}, nothing kept");
        }
    }
    let weight = script::parse("factor = 0.5 modifier = { factor = 2 literacy = 0.4 } modifier = { factor = 3 terrain = hills } group = { factor = 1 modifier = { factor = 0.25 militancy = 1 } modifier = { factor = 0.25 steam = 5 } modifier = { factor = 9 literacy = 0 } } modifier = { factor = 0.1 state_scope = { unemployment = 0 } }").unwrap().root;
    for scope in &scopes {
        let at = Asked::new(&s, scope);
        assert_eq!(DeepSum::new(&weight).weight(&s, Some(&facts), &memo, &at), additive_weight(&s, &weight, scope), "sum for {scope:?}");
        assert_eq!(DeepProduct::new(&weight).weight(&s, Some(&facts), &memo, &at), wiktoria_simulation::pops::product_weight(&s, &weight, scope), "product for {scope:?}");
    }
}
