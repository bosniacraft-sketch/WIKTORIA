//! Events and decisions over time on a small synthetic state (no game files).

use std::collections::BTreeMap;

use wiktoria_data::date::Date;
use wiktoria_data::script;
use wiktoria_simulation::events::{parse_decisions, parse_events, EventLog, EventRules, EventRunner};
use wiktoria_simulation::script::{CountryState, EffectRules, ProvinceState, State};
use wiktoria_simulation::Calendar;

fn state() -> State {
    let mut s = State::new(Date::new(1836, 1, 1));
    for (id, owner) in [(1, "AAA"), (2, "AAA"), (3, "BBB")] {
        s.provinces.insert(id, ProvinceState { owner: Some(owner.into()), controller: Some(owner.into()), ..Default::default() });
    }
    s.countries = BTreeMap::from([("AAA".to_string(), CountryState { prestige: 10.0, ..Default::default() }), ("BBB".to_string(), CountryState::default())]);
    s.seed = 3;
    s
}

/// The rules file's event numbers as a test stand-in (FLD_EVENT_*).
fn rules_of_the_engine_rules() -> EventRules {
    EventRules { check_days: 1, month_days: 30.0, year_days: 365.0, scan_every_days: 2, province_chance_factor: 2.0, min_chance: 1e-6 }
}

fn run(events: &str, decisions: &str, days: usize, check_days: i64) -> (State, EventLog) {
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
        seed: 3,
    };
    let event_rules = EventRules { check_days, ..rules_of_the_engine_rules() };
    let cal = Calendar::new(43_800_000, vec![31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]).unwrap();
    let mut runner = EventRunner::new(parse_events(&script::parse(events).unwrap().root), parse_decisions(&script::parse(decisions).unwrap().root));
    let (mut s, mut log) = (state(), EventLog::default());
    for _ in 0..days {
        runner.day(&mut s, &cal, &rules, &event_rules, &mut log);
    }
    (s, log)
}

#[test]
fn chains_expiry_and_one_time_events() {
    let events = "
        country_event = { id = 1 fire_only_once = yes trigger = { tag = AAA } mean_time_to_happen = { days = 0 }
            immediate = { set_country_flag = began }
            option = { add_country_modifier = { name = boom duration = 20 } BBB = { country_event = { id = 2 days = 5 } } } }
        country_event = { id = 2 is_triggered_only = yes option = { FROM = { prestige = 5 } set_country_flag = told } }
        country_event = { id = 3 trigger = { brigades_compare = 1 } mean_time_to_happen = { days = 1 } option = { prestige = 100 } }
        province_event = { id = 4 trigger = { owned_by = BBB } mean_time_to_happen = { days = 0 } option = { set_province_flag = hit } }";
    let (s, log) = run(events, "", 10, 1);
    assert_eq!(log.fired[&1], 1, "fire_only_once");
    assert_eq!(s.date, Date::new(1836, 1, 11));
    assert!(s.countries["AAA"].flags.contains("began") && s.countries["BBB"].flags.contains("told"), "the asked-for event fired for BBB");
    assert_eq!(s.countries["AAA"].prestige, 15.0, "FROM is the country that asked");
    assert_eq!(log.fired[&4], 5, "a waiting time of zero fires at every scan: provinces are scanned every second day");
    assert!(s.provinces[&3].flags.contains("hit") && !s.provinces[&1].flags.contains("hit"));
    assert_eq!(log.unknown, BTreeMap::from([("brigades_compare".to_string(), 1)]), "an event with an unknown condition never fires and is counted once");
    assert_eq!(s.countries["AAA"].modifiers.len(), 1);
    let (s, log) = run(events, "", 25, 1);
    assert!(s.countries["AAA"].modifiers.is_empty() && log.modifiers_expired == 1, "gone on its expiry date");
}

#[test]
fn mean_time_sets_the_firing_rate() {
    // 100 days waiting time, halved for AAA; tested every 10 days for 10,000 days.
    let events = "country_event = { id = 9 mean_time_to_happen = { days = 100 modifier = { factor = 0.5 tag = AAA } }
        option = { ai_chance = { factor = 0 } set_country_flag = never } option = { prestige = 1 } }";
    let (s, log) = run(events, "", 10_000, 10);
    // A country event is scanned every second day with chance 1 / T: 5,000 scans, so about
    // 50 times for BBB (T = 100) and 100 times for AAA (T = 50).
    let (a, b) = (s.countries["AAA"].prestige - 10.0, s.countries["BBB"].prestige);
    assert_eq!(log.fired[&9] as f64, a + b);
    assert!((75.0..=125.0).contains(&a) && (32.0..=68.0).contains(&b), "AAA {a}, BBB {b}");
    assert!(!s.countries["AAA"].flags.contains("never"), "an option with weight 0 is never picked");
    // The same seed gives the same history.
    assert_eq!(run(events, "", 10_000, 10).0, s);
}

#[test]
fn decisions_are_taken_when_allowed() {
    let decisions = "political_decisions = {
        rise = { potential = { tag = AAA NOT = { has_country_flag = risen } } allow = { prestige = 10 } effect = { set_country_flag = risen prestige = 1 } }
        wait = { potential = { tag = BBB } allow = { prestige = 10 } effect = { set_country_flag = waited } }
        never = { potential = { tag = AAA } effect = { set_country_flag = no } ai_will_do = { factor = 1 modifier = { factor = 0 tag = AAA } } } }";
    let (s, log) = run("", decisions, 5, 1);
    assert_eq!(log.decided, BTreeMap::from([("rise".to_string(), 1)]), "once: its own flag hides it afterwards");
    assert_eq!(s.countries["AAA"].prestige, 11.0);
    assert!(s.countries["BBB"].flags.is_empty() && !s.countries["AAA"].flags.contains("no"));
}

/// The original's chance per scan, and its one slot for the waiting time.
#[test]
fn the_chance_per_scan_and_the_last_written_time() {
    let r = rules_of_the_engine_rules();
    let scan_chance = |t: f64, p: bool| wiktoria_simulation::events::scan_chance(&r, t, p);
    assert_eq!(scan_chance(360.0, false), 1.0 / 360.0);
    assert_eq!(scan_chance(360.9, true), 2.0 / 360.0, "whole days; twice for a province event");
    assert_eq!((scan_chance(1.0, false), scan_chance(0.0, true), scan_chance(1.5, true)), (1.0, 1.0, 1.0), "certain at a day or less, and never above certain");
    assert_eq!((scan_chance(-5.0, false), scan_chance(1e9, false)), (1e-6, 1e-6), "one in a million at least");
    // A province event waits about its written time: 2 provinces of AAA, T = 100, 10,000 days
    // tested daily (5,000 scans at 2 / 100) = about 200 firings.
    let events = "province_event = { id = 5 trigger = { owned_by = AAA } mean_time_to_happen = { days = 100 } option = { owner = { prestige = 1 } } }";
    let (s, _) = run(events, "", 10_000, 1);
    let n = s.countries["AAA"].prestige - 10.0;
    assert!((160.0..=240.0).contains(&n), "{n}");
    // `factor` written after `months` replaces it: "2 days", not 24 months. No block: one day.
    let events = "country_event = { id = 6 trigger = { tag = AAA } mean_time_to_happen = { months = 12 factor = 2 } option = { prestige = 1 } }
        country_event = { id = 7 trigger = { tag = BBB NOT = { has_country_flag = seen } } option = { set_country_flag = seen } }";
    let (s, log) = run(events, "", 100, 1);
    let n = s.countries["AAA"].prestige - 10.0;
    assert!((15.0..=35.0).contains(&n), "{n} of 50 scans at one in two");
    assert_eq!(log.fired[&7], 1, "an event without a waiting time fires at the first scan");
}

/// The compiled conditions give the string evaluator's answers, unknown ones included, in
/// every kind of scope, with and without the figures kept per scan.
#[test]
fn compiled_conditions_answer_as_the_string_evaluator() {
    use wiktoria_simulation::cond::{Conditions, Facts};
    use wiktoria_simulation::script::{conditions_hold, PopState, Scope, TimedModifier};
    let mut s = state();
    s.strata = BTreeMap::from([("farmers".to_string(), "poor".to_string()), ("clerks".to_string(), "middle".to_string()), ("lords".to_string(), "rich".to_string())]);
    s.issues = vec![("open_vote".to_string(), "voting".to_string()), ("closed_vote".to_string(), "voting".to_string()), ("pensions".to_string(), "old_age".to_string()), ("free_trade".to_string(), "trade".to_string())];
    s.issue_categories = ["political_reforms", "political_reforms", "social_reforms", "party_issues"].map(String::from).to_vec();
    s.ideologies = vec!["liberal".to_string(), "year".to_string(), "badboy".to_string(), "farmers".to_string(), "steam".to_string()];
    s.technologies.insert("steam".to_string());
    s.regions = vec!["west".to_string()];
    s.global_flags.insert("canal".to_string());
    let pop = |kind: &str, culture: &str, size: f64, x: f64| PopState { pop_type: kind.into(), culture: culture.into(), size, militancy: 10.0 * x, life_needs: x, everyday_needs: x / 2.0, luxury_needs: x / 3.0, issues: BTreeMap::from([(1, 100.0 * x), (3, 70.0 * x), (4, 5.0)]), ideology: BTreeMap::from([(1, 90.0 * x), (2, 50.0), (4, 30.0), (9, 1.0)]), ..Default::default() };
    s.provinces.get_mut(&1).unwrap().pops = vec![pop("farmers", "red", 300.0, 0.3), pop("clerks", "blue", 200.0, 0.9), pop("lords", "blue", 100.0, 0.1)];
    s.provinces.get_mut(&2).unwrap().pops = vec![pop("farmers", "blue", 700.0, 0.6), pop("unlisted", "red", 700.0, 0.2)];
    s.provinces.get_mut(&3).unwrap().pops = vec![pop("clerks", "green", 50.0, 0.5)];
    s.provinces.insert(4, ProvinceState { pops: vec![pop("farmers", "red", 10.0, 0.4)], ..Default::default() });
    for (id, p) in s.provinces.iter_mut() {
        (p.region, p.trade_goods) = (Some(0), Some(if *id == 1 { "grain" } else { "coal" }.to_string()));
    }
    s.provinces.get_mut(&2).unwrap().modifiers.push(TimedModifier { name: "flood".into(), expires: None });
    let a = s.countries.get_mut("AAA").unwrap();
    (a.primary_culture, a.accepted_cultures) = (Some("red".into()), vec!["blue".into()]);
    a.flags.insert("seen".into());
    a.modifiers.push(TimedModifier { name: "boom".into(), expires: None });
    s.countries.get_mut("BBB").unwrap().primary_culture = Some("green".into());
    s.this = Some(Scope::Country("BBB".into()));

    let mut written = String::new();
    for x in ["0", "0.2", "0.35", "0.45", "0.5", "0.61", "2", "3.1", "4.4", "5", "9"] {
        for key in ["political_reform_want", "social_reform_want", "poor_strata_militancy", "middle_strata_life_needs", "rich_strata_everyday_needs", "poor_strata_luxury_needs", "province_id", "liberal", "pensions", "free_trade", "open_vote", "closed_vote", "badboy", "farmers", "steam", "nothing"] {
            written += &format!("{key} = {x}\n");
        }
    }
    for x in [20, 27, 28, 36, 37, 38, 45, 46, 54] {
        written += &format!("liberal = {x}\nopen_vote = {x}\npensions = {x}\n");
    }
    for year in [1835, 1836, 1837] {
        written += &format!("year = {year}\n");
    }
    for key in ["is_primary_culture", "is_accepted_culture"] {
        written += &format!("{key} = yes\n{key} = no\n{key} = maybe\n");
    }
    for (key, values) in [("tag", ["AAA", "BBB", "THIS"]), ("has_global_flag", ["canal", "dam", "THIS"]), ("has_country_flag", ["seen", "unseen", "FROM"]), ("has_country_modifier", ["boom", "flood", "x"]), ("has_province_modifier", ["boom", "flood", "x"]), ("produces", ["grain", "coal", "tin"])] {
        for v in values {
            written += &format!("{key} = {v}\n");
        }
    }
    written += "AAA = { tag = AAA has_country_flag = seen }\nBBB = { has_country_flag = seen }\nZZZ = { tag = ZZZ }\nAAA = { province_id = 1 }\nBBB = { factor = 2 exists = yes }\n";
    written += "NOT = { tag = AAA year = 1836 }\nOR = { province_id = 3 produces = grain }\nowner = { social_reform_want = 0.2 tag = AAA }\nAAA = { NOT = { is_primary_culture = yes } }\n";
    let scopes = [Scope::Country("AAA".into()), Scope::Country("BBB".into()), Scope::Country("ZZZ".into()), Scope::Province(1), Scope::Province(2), Scope::Province(3), Scope::Province(4), Scope::Pop(1, 0), Scope::Pop(2, 1), Scope::Pop(9, 0), Scope::State(0, "AAA".into()), Scope::State(0, "ZZZ".into())];
    let (facts, mut yes, mut no, mut unknown) = (Facts::new(), 0, 0, 0);
    for line in written.lines() {
        let block = script::parse(line).unwrap().root;
        let compiled = Conditions::new(&block);
        for scope in &scopes {
            let expected = conditions_hold(&s, &block, scope);
            assert_eq!(compiled.hold(&s, Some(&facts), scope), expected, "{line} in {scope:?}, with facts");
            assert_eq!(compiled.hold(&s, None, scope), expected, "{line} in {scope:?}, without facts");
            match expected {
                Ok(true) => yes += 1,
                Ok(false) => no += 1,
                Err(_) => unknown += 1,
            }
        }
    }
    assert!(yes > 100 && no > 100 && unknown > 100, "the made-up state shows all three answers: {yes} {no} {unknown}");
}

/// What `run` uses, for the tests that drive the runner day by day.
fn parts() -> (EffectRules, EventRules, Calendar) {
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
        seed: 3,
    };
    (rules, rules_of_the_engine_rules(), Calendar::new(43_800_000, vec![31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]).unwrap())
}

/// A played country chooses its events' options; unanswered, the computer's pick is taken.
#[test]
fn a_played_country_chooses_in_its_events() {
    let events = "
        country_event = { id = 20 fire_only_once = yes trigger = { tag = AAA } mean_time_to_happen = { days = 0 }
            immediate = { set_country_flag = asked }
            option = { name = OPT_A prestige = 1 ai_chance = { factor = 0 } } option = { name = OPT_B prestige = 50 } }
        country_event = { id = 21 fire_only_once = yes trigger = { tag = AAA } mean_time_to_happen = { days = 0 } option = { set_country_flag = only } }
        province_event = { id = 22 fire_only_once = yes trigger = { owned_by = AAA } mean_time_to_happen = { days = 0 }
            option = { owner = { set_country_flag = p1 } } option = { owner = { set_country_flag = p2 } } }";
    let (rules, event_rules, cal) = parts();
    let mut runner = EventRunner::new(parse_events(&script::parse(events).unwrap().root), Vec::new());
    let (mut s, mut log) = (state(), EventLog::default());
    s.countries.get_mut("AAA").unwrap().human = true;
    for _ in 0..3 {
        runner.day(&mut s, &cal, &rules, &event_rules, &mut log);
    }
    let aaa = |s: &State| s.countries["AAA"].clone();
    assert!(aaa(&s).flags.contains("asked") && aaa(&s).flags.contains("only"), "immediate and a one-option event run at once");
    assert_eq!(aaa(&s).prestige, 10.0, "no option of 20 yet");
    let held: Vec<(u64, String)> = runner.held.iter().map(|h| (h.id, h.tag.clone())).collect();
    assert_eq!(held, [(20, "AAA".to_string()), (22, "AAA".to_string())], "a province event asks the province's owner");
    assert_eq!(runner.held[0].ai_pick, 1, "the computer's pick is rolled at once");
    assert_eq!(wiktoria_simulation::events::option_names(&runner.events[0]), ["OPT_A", "OPT_B"]);
    // The player picks the first option, which the computer never would.
    let key = runner.held[0].key;
    assert!(runner.choose(&mut s, &cal, &rules, key, Some(2), &mut log).is_err(), "no third option");
    runner.choose(&mut s, &cal, &rules, key, Some(0), &mut log).unwrap();
    assert_eq!(aaa(&s).prestige, 11.0);
    assert!(runner.choose(&mut s, &cal, &rules, key, Some(0), &mut log).is_err(), "answered once");
    // The province event is left: after its time the computer's pick is taken.
    runner.answer_days = 0;
    let key = runner.held[0].key;
    runner.held[0].expires = s.date;
    runner.day(&mut s, &cal, &rules, &event_rules, &mut log);
    assert!(runner.held.iter().all(|h| h.key != key));
    assert!(aaa(&s).flags.contains("p1") || aaa(&s).flags.contains("p2"));
    // BBB, the computer's, never waits.
    assert!(runner.held.iter().all(|h| h.tag == "AAA"));
}

#[test]
fn a_played_country_takes_its_own_decisions() {
    let decisions = "political_decisions = {
        rise = { potential = { NOT = { has_country_flag = risen } } allow = { prestige = 10 } effect = { set_country_flag = risen prestige = 1 } }
        later = { potential = { always = yes } allow = { prestige = 99 } effect = { set_country_flag = later } } }";
    let (rules, event_rules, cal) = parts();
    let mut runner = EventRunner::new(Vec::new(), parse_decisions(&script::parse(decisions).unwrap().root));
    let (mut s, mut log) = (state(), EventLog::default());
    s.countries.get_mut("AAA").unwrap().human = true;
    s.countries.get_mut("BBB").unwrap().prestige = 10.0;
    runner.day(&mut s, &cal, &rules, &event_rules, &mut log);
    assert!(s.countries["BBB"].flags.contains("risen"), "the computer decides for BBB");
    assert!(!s.countries["AAA"].flags.contains("risen"), "but not for AAA");
    let open: Vec<(String, bool)> = runner.open_decisions(&mut s, "AAA").into_iter().map(|(d, ok)| (d.name.clone(), ok)).collect();
    assert_eq!(open, [("rise".to_string(), true), ("later".to_string(), false)]);
    assert!(runner.take_decision(&mut s, &cal, &rules, "AAA", "later", &mut log).is_err(), "not allowed yet");
    assert!(runner.take_decision(&mut s, &cal, &rules, "AAA", "nope", &mut log).is_err());
    runner.take_decision(&mut s, &cal, &rules, "AAA", "rise", &mut log).unwrap();
    assert!(s.countries["AAA"].flags.contains("risen") && s.countries["AAA"].prestige == 11.0);
    assert!(runner.take_decision(&mut s, &cal, &rules, "AAA", "rise", &mut log).is_err(), "its potential is gone");
}

/// A held event does not fire again for the same target while it waits for the player.
#[test]
fn a_held_event_waits_and_does_not_fire_again() {
    let events = "
        country_event = { id = 30 trigger = { tag = AAA } mean_time_to_happen = { days = 0 }
            immediate = { prestige = 1 }
            option = { name = OPT_A } option = { name = OPT_B } }";
    let (rules, event_rules, cal) = parts();
    let mut runner = EventRunner::new(parse_events(&script::parse(events).unwrap().root), Vec::new());
    let (mut s, mut log) = (state(), EventLog::default());
    s.countries.get_mut("AAA").unwrap().human = true;
    for _ in 0..(3 * event_rules.check_days.max(1)) {
        runner.day(&mut s, &cal, &rules, &event_rules, &mut log);
    }
    assert_eq!(runner.held.iter().filter(|h| h.id == 30).count(), 1, "one question waits");
    assert_eq!(s.countries["AAA"].prestige, 11.0, "immediate ran once");
    // Answered, it may fire again.
    let key = runner.held[0].key;
    runner.choose(&mut s, &cal, &rules, key, Some(0), &mut log).unwrap();
    for _ in 0..(3 * event_rules.check_days.max(1)) {
        runner.day(&mut s, &cal, &rules, &event_rules, &mut log);
    }
    assert_eq!((s.countries["AAA"].prestige, runner.held.len()), (12.0, 1), "fired again once answered");
}
