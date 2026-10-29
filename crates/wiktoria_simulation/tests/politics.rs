//! Elections and the upper house, on a made-up country; and, further down,
//! administrative efficiency and research (the same helper's area).

use std::collections::BTreeMap;

use wiktoria_data::date::Date;
use wiktoria_simulation::politics::{election_winner, party_votes, politics_day, upper_house, Citizenship, Government, HouseRule, PartyInfo, PoliticsRules, VotingSystem};
use wiktoria_simulation::script::{CountryState, PopState, ProvinceState, State, TimedModifier};

fn party(id: u32, ideology: &str, start: i32, end: i32) -> PartyInfo {
    let positions = if ideology == "blue" { vec!["free".to_string()] } else { vec!["duties".to_string()] };
    PartyInfo { id, ideology: ideology.into(), start: Some(Date::new(start, 1, 1)), end: Some(Date::new(end, 1, 1)), min_tax: 0.0, max_tax: if id == 2 { 0.5 } else { 1.0 }, max_military: if id == 2 { 0.75 } else { 1.0 }, min_tariff: -1.0, max_tariff: if id == 2 { 0.25 } else { 1.0 }, positions, citizenship: Citizenship::All }
}

fn rules() -> PoliticsRules {
    PoliticsRules {
        campaign_months: 6,
        governments: [
            ("kingdom".to_string(), Government { election: true, duration: 48, ideologies: ["red", "blue"].into_iter().map(str::to_string).collect() }),
            ("empire".to_string(), Government { election: false, duration: 0, ideologies: ["blue"].into_iter().map(str::to_string).collect() }),
        ]
        .into(),
        franchise: [("nobody".to_string(), [0.0, 0.0, 0.0]), ("the_rich".to_string(), [0.0, 0.0, 1.0]), ("everybody".to_string(), [1.0, 1.0, 1.0])].into(),
        franchise_reform: "vote".into(),
        house_reform: "house".into(),
        house: [("lords".to_string(), HouseRule::RichOnly), ("party".to_string(), HouseRule::RulingParty), ("states".to_string(), HouseRule::StateVote), ("people".to_string(), HouseRule::PopulationVote)].into(),
        // Party 2 (red) ends in 1850; party 4 (red) takes over. Green has a party but no government allows it.
        parties: [("AAA".to_string(), vec![party(1, "blue", 1800, 2000), party(2, "red", 1800, 1850), party(3, "green", 1800, 2000), party(4, "red", 1850, 2000)])].into(),
        voting_reform: "system".into(),
        systems: [("winner".to_string(), VotingSystem::LargestShare), ("jefferson".to_string(), VotingSystem::Jefferson), ("shares".to_string(), VotingSystem::Proportional)].into(),
        ruling_support: [("loyal_press".to_string(), 0.5)].into(),
    }
}

/// Every pop has consciousness 2, so its support for a party is 2 x its ideology share in
/// percent / 20 (nobody holds an issue the parties stand for unless a test says so).
fn world(date: Date, vote: &str, house: &str, government: &str) -> State {
    let mut s = State::new(date);
    s.ideologies = vec!["blue".into(), "red".into(), "green".into()];
    s.party_ideologies = vec!["blue".into(), "red".into(), "green".into(), "red".into()];
    s.issues = vec![("free".into(), "trade".into()), ("duties".into(), "trade".into())];
    s.strata = [("farmers", "poor"), ("owners", "rich")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let pop = |kind: &str, size: f64, blue: f64, red: f64, green: f64| PopState { pop_type: kind.into(), size, culture: "alpha".into(), consciousness: 2.0, ideology: [(1, blue), (2, red), (3, green)].into(), ..Default::default() };
    // State 0: many red farmers, a few blue owners. State 1: blue farmers. A colony full of reds.
    s.provinces.insert(1, ProvinceState { owner: Some("AAA".into()), region: Some(0), pops: vec![pop("farmers", 90_000.0, 20.0, 50.0, 30.0), pop("owners", 1_000.0, 80.0, 20.0, 0.0)], ..Default::default() });
    s.provinces.insert(2, ProvinceState { owner: Some("AAA".into()), region: Some(1), pops: vec![pop("farmers", 10_000.0, 100.0, 0.0, 0.0)], ..Default::default() });
    s.provinces.insert(3, ProvinceState { owner: Some("AAA".into()), region: Some(2), colonial: 2, pops: vec![pop("farmers", 500_000.0, 0.0, 100.0, 0.0)], ..Default::default() });
    let reforms: BTreeMap<String, String> = [("vote", vote), ("house", house)].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let spending = [("rich_tax", 0.8), ("poor_tax", 0.3), ("military_spending", 1.0), ("tariffs", 0.6)].into_iter().map(|(a, b)| (a.to_string(), b)).collect();
    s.countries.insert("AAA".into(), CountryState { government: Some(government.into()), primary_culture: Some("alpha".into()), reforms, spending, ruling_party: Some(1), last_election: Some(Date::new(1836, 1, 1)), ..Default::default() });
    s
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} is not {b}");
}

#[test]
fn the_franchise_decides_who_wins() {
    let r = rules();
    // Only the rich vote: 800 blue against 200 red.
    let s = world(Date::new(1840, 7, 1), "the_rich", "lords", "kingdom");
    assert_eq!(election_winner(&s, &r, "AAA").map(|w| w.0), Some(1));
    // Everybody votes. Green has no party the government allows, so the farmers of state 0
    // split 20 : 50 between blue and red: blue 25,714 + 800 + 10,000, red 64,286 + 200. The
    // colony does not vote.
    let s = world(Date::new(1840, 7, 1), "everybody", "lords", "kingdom");
    let (party, share) = election_winner(&s, &r, "AAA").unwrap();
    assert_eq!(party, 2);
    close(share, (90_000.0 * 5.0 / 7.0 + 200.0) / 101_000.0);
    // From 1850 the red votes go to the red party of that time.
    let s = world(Date::new(1850, 1, 1), "everybody", "lords", "kingdom");
    assert_eq!(election_winner(&s, &r, "AAA").map(|w| w.0), Some(4));
    // Nobody may vote: no election.
    let s = world(Date::new(1840, 7, 1), "nobody", "lords", "kingdom");
    assert_eq!(election_winner(&s, &r, "AAA"), None);
}

#[test]
fn issues_and_consciousness_make_a_pops_support() {
    let r = rules();
    let mut s = world(Date::new(1840, 7, 1), "everybody", "lords", "kingdom");
    s.provinces.remove(&1);
    // 10,000 farmers, all blue by ideology, but 60% of them hold the red party's trade policy.
    // Blue: 2 x 100 / 20 = 10; red: 0.6. Blue gets 10 / 10.6 of them.
    s.provinces.get_mut(&2).unwrap().pops[0].issues = [(2, 60.0)].into();
    let votes = party_votes(&s, &r, "AAA").unwrap().0;
    close(votes[&1], 10_000.0 * 10.0 / 10.6);
    close(votes[&2], 10_000.0 * 0.6 / 10.6);
    // Without consciousness only the issues count: blue has none of them.
    s.provinces.get_mut(&2).unwrap().pops[0].consciousness = 0.0;
    let votes = party_votes(&s, &r, "AAA").unwrap().0;
    assert_eq!((votes.get(&1).copied().unwrap_or(0.0), votes[&2]), (0.0, 10_000.0));
    // A pop with neither casts no vote.
    s.provinces.get_mut(&2).unwrap().pops[0].issues.clear();
    assert_eq!(election_winner(&s, &r, "AAA"), None);
}

#[test]
fn the_ruling_party_is_helped_by_its_support_modifiers() {
    let r = rules();
    let mut s = world(Date::new(1840, 7, 1), "everybody", "lords", "kingdom");
    s.provinces.remove(&2);
    s.provinces.get_mut(&1).unwrap().pops.truncate(1);
    // 90,000 farmers, blue 20 : red 50. The blues rule and the country's modifier gives them +50%:
    // 30 : 50.
    s.countries.get_mut("AAA").unwrap().modifiers.push(TimedModifier { name: "loyal_press".into(), expires: None });
    let votes = party_votes(&s, &r, "AAA").unwrap().0;
    close(votes[&1], 90_000.0 * 30.0 / 80.0);
    // The same modifier on the province adds to it: 40 : 50.
    s.provinces.get_mut(&1).unwrap().modifiers.push(TimedModifier { name: "loyal_press".into(), expires: None });
    close(party_votes(&s, &r, "AAA").unwrap().0[&1], 90_000.0 * 40.0 / 90.0);
}

#[test]
fn citizenship_cuts_the_votes_of_other_cultures() {
    let mut r = rules();
    let mut s = world(Date::new(1840, 7, 1), "everybody", "lords", "kingdom");
    // The blue farmers of state 1 are of another culture.
    s.provinces.get_mut(&2).unwrap().pops[0].culture = "beta".into();
    let blue = |s: &State, r: &PoliticsRules| party_votes(s, r, "AAA").unwrap().0[&1];
    let others = 90_000.0 * 2.0 / 7.0 + 800.0;
    close(blue(&s, &r), others + 10_000.0);
    r.parties.get_mut("AAA").unwrap()[0].citizenship = Citizenship::Accepted;
    close(blue(&s, &r), others + 5_000.0);
    r.parties.get_mut("AAA").unwrap()[0].citizenship = Citizenship::Primary;
    close(blue(&s, &r), others);
    // An accepted culture votes in full.
    s.countries.get_mut("AAA").unwrap().accepted_cultures.push("beta".into());
    close(blue(&s, &r), others + 10_000.0);
}

#[test]
fn the_voting_system_decides_what_a_province_hands_on() {
    let r = rules();
    let mut s = world(Date::new(1840, 7, 1), "everybody", "lords", "kingdom");
    // A few reds among the blue farmers of state 1: 4% of that province.
    s.provinces.get_mut(&2).unwrap().pops[0].ideology = [(1, 96.0), (2, 4.0)].into();
    let votes = |s: &State| party_votes(s, &r, "AAA").unwrap().0;
    let system = |s: &mut State, option: &str| s.countries.get_mut("AAA").unwrap().reforms.insert("system".into(), option.into());
    system(&mut s, "shares");
    close(votes(&s)[&2], 90_000.0 * 5.0 / 7.0 + 200.0 + 400.0);
    // Jefferson method: under 5% of the province the reds of state 1 are dropped.
    system(&mut s, "jefferson");
    close(votes(&s)[&2], 90_000.0 * 5.0 / 7.0 + 200.0);
    close(votes(&s)[&1], 90_000.0 * 2.0 / 7.0 + 800.0 + 9_600.0);
    // First past the post: red takes all of province 1, blue all of province 2.
    system(&mut s, "winner");
    close(votes(&s)[&1], 10_000.0);
    close(votes(&s)[&2], 91_000.0);
}

#[test]
fn a_calm_country_elects_the_largest_ideology_first() {
    let mut r = rules();
    // Two red parties at the same time.
    r.parties.get_mut("AAA").unwrap()[3].start = Some(Date::new(1800, 1, 1));
    r.parties.get_mut("AAA").unwrap()[3].positions = vec!["free".into()];
    let mut s = world(Date::new(1840, 7, 1), "everybody", "lords", "kingdom");
    s.provinces.remove(&1);
    s.provinces.remove(&3);
    // 10,000 farmers without consciousness: 40% hold `free` and 35% `duties`. Blue stands for
    // `free` (40), red party 2 for `duties` (35), red party 4 for `free` too (40): the votes
    // split 40 : 35 : 40.
    let pop = &mut s.provinces.get_mut(&2).unwrap().pops[0];
    (pop.consciousness, pop.issues) = (0.0, [(1, 40.0), (2, 35.0)].into());
    // Calm: the reds together (75 of 115) are the largest ideology and their larger party wins.
    assert_eq!(election_winner(&s, &r, "AAA").map(|w| w.0), Some(4));
    r.parties.get_mut("AAA").unwrap()[3].positions = vec![];
    // Now red party 4 gets nothing: 40 blue against 35 red. Calm or not, blue wins.
    assert_eq!(election_winner(&s, &r, "AAA").map(|w| w.0), Some(1));
    // Restless (average militancy 5 or more): the largest party wins whatever its ideology.
    r.parties.get_mut("AAA").unwrap()[3].positions = vec!["duties".into()];
    r.parties.get_mut("AAA").unwrap()[1].positions = vec!["duties".into()];
    // Blue 40, the two red parties 35 each: calm -> a red party; restless -> blue.
    assert_ne!(election_winner(&s, &r, "AAA").map(|w| w.0), Some(1));
    s.provinces.get_mut(&2).unwrap().pops[0].militancy = 6.0;
    assert_eq!(election_winner(&s, &r, "AAA").map(|w| w.0), Some(1));
}

#[test]
fn elections_end_four_and_a_half_years_after_the_last() {
    let r = rules();
    let mut s = world(Date::new(1840, 6, 1), "everybody", "lords", "kingdom");
    assert!(politics_day(&mut s, &r).elections.is_empty(), "a month early");
    s.date = Date::new(1840, 7, 2);
    assert!(politics_day(&mut s, &r).elections.is_empty(), "only on the 1st");
    s.date = Date::new(1840, 7, 1);
    assert_eq!(politics_day(&mut s, &r).elections, vec![("AAA".to_string(), Some(1), 2)]);
    assert_eq!((s.countries["AAA"].ruling_party, s.countries["AAA"].last_election), (Some(2), Some(Date::new(1840, 7, 1))));
    // The new ruling party allows taxes up to 50%, military spending up to 75% and tariffs up to 25%.
    let c = &s.countries["AAA"];
    assert_eq!((c.spending["rich_tax"], c.spending["poor_tax"], c.spending["military_spending"], c.spending["tariffs"]), (0.5, 0.3, 0.75, 0.25));
    s.date = Date::new(1840, 8, 1);
    assert!(politics_day(&mut s, &r).elections.is_empty(), "the next one is due in 1845");
    // A government without elections keeps its party.
    let mut s = world(Date::new(1840, 7, 1), "everybody", "lords", "empire");
    assert!(politics_day(&mut s, &r).elections.is_empty());
}

#[test]
fn the_tariff_stays_within_the_ruling_partys_trade_policy() {
    let mut r = rules();
    r.parties.get_mut("AAA").unwrap()[0].min_tariff = -0.25;
    let mut s = world(Date::new(1837, 2, 1), "everybody", "lords", "kingdom");
    s.countries.get_mut("AAA").unwrap().spending.insert("tariffs".into(), -0.6);
    politics_day(&mut s, &r);
    assert_eq!(s.countries["AAA"].spending["tariffs"], -0.25);
}

#[test]
fn the_upper_house_follows_its_reform() {
    let r = rules();
    let share = |s: &State, k: &str| upper_house(s, &r, "AAA").unwrap().get(k).copied().unwrap_or(0.0);
    let s = world(Date::new(1837, 1, 1), "everybody", "lords", "kingdom");
    assert!((share(&s, "blue") - 0.8).abs() < 1e-9, "the rich alone");
    let s = world(Date::new(1837, 1, 1), "everybody", "party", "kingdom");
    assert_eq!(share(&s, "blue"), 1.0, "the ruling party's ideology");
    let s = world(Date::new(1837, 1, 1), "everybody", "people", "kingdom");
    assert!((share(&s, "blue") - 28_800.0 / 101_000.0).abs() < 1e-9, "all votes");
    // States count alike: state 0 is 18,800 / 91,000 blue, state 1 all blue.
    let s = world(Date::new(1837, 1, 1), "everybody", "states", "kingdom");
    assert!((share(&s, "blue") - (18_800.0 / 91_000.0 + 1.0) / 2.0).abs() < 1e-9);
    // It is filled on 1 January.
    let mut s = world(Date::new(1837, 1, 1), "everybody", "lords", "kingdom");
    politics_day(&mut s, &r);
    assert!((s.countries["AAA"].upper_house["blue"] - 0.8).abs() < 1e-9);
}

// ---------------------------------------------------------------- administrative efficiency

#[test]
fn administrative_efficiency_needs_more_bureaucrats_with_every_administrative_reform() {
    use wiktoria_simulation::pops::{admin_efficiency_shares, AdminRules};
    let mut s = State::new(Date::new(1836, 1, 1));
    let pop = |kind: &str, size: f64| PopState { pop_type: kind.into(), size, ..Default::default() };
    s.provinces.insert(1, ProvinceState { owner: Some("AAA".into()), pops: vec![pop("farmers", 99_400.0), pop("bureaucrats", 600.0)], ..Default::default() });
    s.countries.insert("AAA".into(), CountryState::default());
    // 0.6% bureaucrats of the 1% wanted. No base is added.
    close(admin_efficiency_shares(&s, 0.01)["AAA"], 0.6);
    // Two reforms of administrative levels 4 and 1 raise the wanted share to 1.5%; a technology
    // and a reform option with +5% each make the bureaucrats count 1.1 times.
    s.admin_rules = AdminRules { increment: 0.001, option_levels: [("good_wages".to_string(), 4.0), ("some_schools".to_string(), 1.0)].into(), modifier_bonus: [("some_schools".to_string(), 0.05)].into(), tech_bonus: [("ledgers".to_string(), 0.05)].into() };
    let c = s.countries.get_mut("AAA").unwrap();
    c.reforms = [("wages", "good_wages"), ("schools", "some_schools")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    c.technologies.insert("ledgers".into());
    close(admin_efficiency_shares(&s, 0.01)["AAA"], 0.006 * 1.1 / 0.015);
    // Never above 1.
    s.provinces.get_mut(&1).unwrap().pops[1].size = 50_000.0;
    assert_eq!(admin_efficiency_shares(&s, 0.01)["AAA"], 1.0);
}

// ---------------------------------------------------------------- research

fn research_world(rate: f64) -> State {
    let mut s = State::new(Date::new(1840, 1, 1));
    s.provinces.insert(1, ProvinceState { owner: Some("AAA".into()), ..Default::default() });
    s.countries.insert("AAA".into(), CountryState { civilized: true, research_rate: rate, ..Default::default() });
    s
}

#[test]
fn research_points_go_into_a_technology_at_most_a_hundred_a_day() {
    use wiktoria_simulation::research::{research_day, ResearchRules, TechInfo};
    let tech = |name: &str, cost: f64| TechInfo { name: name.into(), folder: "army_tech".into(), year: 1840, cost, ai_chance: None, previous: None };
    let rules = ResearchRules { year_span: 140.0, max_daily: 100.0, max_saved: 25000.0, techs: vec![tech("only", 1000.0)], min_cost: 1.0, store_days: 365.0, weights: wiktoria_simulation::ai::ResearchWeights { overdue_per_year: 0.005, min_top_cost: 3600.0 } };
    let lib = wiktoria_data::modifiers::ModifierLibrary::default();
    let mut s = research_world(40.0);
    s.countries.get_mut("AAA").unwrap().research_points = 1000.0;
    // 40 of today and 1000 stored: 100 go in, 940 stay stored.
    research_day(&mut s, &rules, &lib);
    assert_eq!((s.countries["AAA"].research.clone(), s.countries["AAA"].research_points), (Some(("only".to_string(), 100.0)), 940.0));
    // Nine more days of 100; what is left over stays stored.
    let mut done = Vec::new();
    for _ in 0..9 {
        done.extend(research_day(&mut s, &rules, &lib));
    }
    assert_eq!(done, vec![("AAA".to_string(), "only".to_string())]);
    assert_eq!(s.countries["AAA"].research_points, 1000.0 + 400.0 - 1000.0);
    // A technology a century and a half overdue still costs 1 point.
    let old = TechInfo { year: 1600, ..tech("old", 5000.0) };
    assert_eq!(rules.cost(&old, 1840, 0.0), 1.0);
}

#[test]
fn a_country_with_nothing_to_research_stores_a_year_of_points() {
    use wiktoria_simulation::research::{research_day, ResearchRules};
    let rules = ResearchRules { year_span: 140.0, max_daily: 100.0, max_saved: 25000.0, techs: vec![], min_cost: 1.0, store_days: 365.0, weights: wiktoria_simulation::ai::ResearchWeights { overdue_per_year: 0.005, min_top_cost: 3600.0 } };
    let lib = wiktoria_data::modifiers::ModifierLibrary::default();
    let mut s = research_world(10.0);
    for _ in 0..400 {
        research_day(&mut s, &rules, &lib);
    }
    assert_eq!(s.countries["AAA"].research_points, 3650.0);
    // An uncivilized country stores up to MAX_RESEARCH_POINTS instead.
    let c = s.countries.get_mut("AAA").unwrap();
    (c.civilized, c.research_points) = (false, 24_995.0);
    research_day(&mut s, &rules, &lib);
    assert_eq!(s.countries["AAA"].research_points, 25_000.0);
}

#[test]
fn the_next_technology_is_picked_by_the_computer_players_weights() {
    use wiktoria_simulation::research::{research_day, ResearchRules, TechInfo};
    let never = wiktoria_data::script::parse("factor = 0").unwrap().root;
    let tech = |name: &str, ai_chance: Option<wiktoria_data::script::Block>| TechInfo { name: name.into(), folder: "army_tech".into(), year: 1836, cost: 3600.0, ai_chance, previous: None };
    let rules = ResearchRules { year_span: 140.0, max_daily: 100.0, max_saved: 25000.0, techs: vec![tech("unwanted", Some(never)), tech("wanted", None)], min_cost: 1.0, store_days: 365.0, weights: wiktoria_simulation::ai::ResearchWeights { overdue_per_year: 0.005, min_top_cost: 3600.0 } };
    let lib = wiktoria_data::modifiers::ModifierLibrary::default();
    for seed in 0..20 {
        let mut s = research_world(10.0);
        s.seed = seed;
        research_day(&mut s, &rules, &lib);
        assert_eq!(s.countries["AAA"].research.as_ref().map(|r| r.0.as_str()), Some("wanted"));
    }
}
