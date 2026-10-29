//! Province building rules on synthetic fixtures.

use std::collections::BTreeMap;

use wiktoria_data::defines::Defines;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_simulation::buildings::*;
use wiktoria_simulation::military::siege::ProvinceMilitary;

const FILE: &str = "
fort = { type = fort max_level = 4 time = 10 province = yes default_enabled = no
  goods_cost = { cement = 10 iron = 4 } fort_level = 1 }
naval_base = { type = naval_base max_level = 3 time = 6 province = yes port = yes default_enabled = yes
  goods_cost = { lumber = 10 } naval_capacity = 1 }
railroad = { type = infrastructure max_level = 5 time = 4 province = yes default_enabled = no
  goods_cost = { steel = 5 } infrastructure = 0.5 }
mill = { type = factory max_level = 99 time = 100 default_enabled = yes goods_cost = { lumber = 1 } }
";

fn engine_rules() -> EngineRules {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/engine_rules.json");
    EngineRules::from_json(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn rules() -> BuildingRules {
    let d = Defines::parse(
        "defines = {\n ai = {\n CHANCE_BUILD_FORT = 0.5,\n CHANCE_BUILD_NAVAL_BASE = 0.75,\n CHANCE_BUILD_RAILROAD = 0.2,\n }\n military = {\n NAVAL_BASE_SUPPLY_SCORE_BASE = 4,\n NAVAL_BASE_SUPPLY_SCORE_EMPTY = 2,\n NAVAL_BASE_NON_CORE_SUPPLY_SCORE = 0.5,\n MOBILIZATION_SPEED_RAILS_MULT = 3,\n }\n}",
    )
    .unwrap();
    BuildingRules::new(FILE, &d, &engine_rules()).unwrap()
}

fn site() -> Site {
    Site { owner: Some("AAA".into()), controller: Some("AAA".into()), coastal: true, ..Default::default() }
}

#[test]
fn factories_are_not_province_buildings_and_values_come_from_the_file() {
    let r = rules();
    assert_eq!(r.defs.len(), 3);
    assert!(r.def("mill").is_none());
    let f = r.def("fort").unwrap();
    assert_eq!((f.max_level, f.days), (4, 10.0));
    assert_eq!(f.goods_cost, vec![("cement".to_string(), 10.0), ("iron".to_string(), 4.0)]);
    assert_eq!(r.n.chance_fort, 0.5);
    assert!(Defines::parse("defines = { ai = { } }").is_ok() && BuildingRules::new(FILE, &Defines::parse("defines = { ai = { } }").unwrap(), &engine_rules()).is_err());
}

#[test]
fn technology_unlocks_and_limits() {
    let r = rules();
    let (fort, nb, rail) = (r.def("fort").unwrap(), r.def("naval_base").unwrap(), r.def("railroad").unwrap());
    let none = |_: &str| false;
    let tech = |n: &str| n == "fort";
    assert!(!unlocked(fort, &none) && unlocked(fort, &tech) && unlocked(nb, &none));
    assert_eq!(can_build(fort, false, &site(), "AAA", false), Err(Refusal::NotUnlocked));
    assert_eq!(can_build(fort, true, &site(), "AAA", false), Ok(()));
    assert_eq!(can_build(fort, true, &Site { level: 4, ..site() }, "AAA", false), Err(Refusal::AtMaxLevel));
    assert_eq!(can_build(fort, true, &Site { under_construction: true, ..site() }, "AAA", false), Err(Refusal::AlreadyBuilding));
    assert_eq!(can_build(nb, true, &Site { coastal: false, ..site() }, "AAA", false), Err(Refusal::NeedsPort));
    assert_eq!(can_build(rail, true, &Site { controller: Some("BBB".into()), ..site() }, "AAA", false), Err(Refusal::Occupied));
    assert_eq!(can_build(rail, true, &site(), "BBB", false), Err(Refusal::ForeignNotAllowed));
    assert_eq!(can_build(rail, true, &site(), "BBB", true), Ok(()));
    let mut cap = rail.clone();
    cap.capital = true;
    assert_eq!(can_build(&cap, true, &site(), "AAA", false), Err(Refusal::CapitalOnly));
}

#[test]
fn price_and_goods_carry_the_efficiency_factor() {
    let r = rules();
    let fort = r.def("fort").unwrap();
    let g = fort.order_goods(0.75);
    assert!((g["cement"] - 12.5).abs() < 1e-9 && (g["iron"] - 5.0).abs() < 1e-9);
    let price = |g: &str| if g == "cement" { 2.0 } else { 1.0 };
    let p = order_price(&r.n, fort, 1.0, price, false);
    assert!((p - 24.0).abs() < 1e-9);
    assert!((order_price(&r.n, fort, 1.0, price, true) - 72.0).abs() < 1e-9);
}

#[test]
fn site_takes_goods_pays_back_and_completes_when_time_and_goods_are_in() {
    let r = rules();
    let rail = r.def("railroad").unwrap();
    let mut c = order(rail, 7, 2, 1.0, 10.0);
    assert_eq!((c.to_level, c.days_left), (3, 4.0));
    assert!((daily_ask(&c, 0.1)["steel"] - 0.5).abs() < 1e-9);
    let price = |_: &str| 1.0;
    // Only 2 steel in the open: not done after 4 days even though the timer ran out.
    let mut open = BTreeMap::from([("steel".to_string(), 2.0)]);
    let mut paid = 0.0;
    for _ in 0..4 {
        let d = construction_day(&mut c, &mut open, price);
        paid += d.to_treasury;
        assert_eq!(d.completed, None);
    }
    assert!((paid - 2.0).abs() < 1e-9);
    assert!((c.site.lacks["steel"] - 3.0).abs() < 1e-9);
    open.insert("steel".into(), 10.0);
    let d = construction_day(&mut c, &mut open, price);
    assert_eq!(d.completed, Some(3));
    assert!((d.to_treasury - 3.0).abs() < 1e-9);
    assert!((d.refund - 5.0).abs() < 1e-9);
    assert!((open["steel"] - 7.0).abs() < 1e-9);
}

#[test]
fn bankruptcy_refunds_the_site() {
    let r = rules();
    let mut c = order(r.def("fort").unwrap(), 1, 0, 1.0, 30.0);
    assert_eq!(cancel(&mut c), 30.0);
    assert_eq!(c.site.money, 0.0);
}

#[test]
fn effects_read_by_other_code() {
    let r = rules();
    let (fort, rail) = (r.def("fort").unwrap(), r.def("railroad").unwrap());
    assert_eq!(fort_level(fort, 3), 3.0);
    assert_eq!(infrastructure(rail, 4), 2.0);
    assert_eq!(supply_bonus(rail, 4), 0.0);
    // Two of three provinces at half, one at full: share 2/3 * ... = (0.4+0.4+1.0)/3.
    let f = mobilisation_rail_factor(&r.n, rail, &[2, 2, 5]);
    assert!((f - (1.0 + 3.0 * 0.6)).abs() < 1e-9);
    assert_eq!(mobilisation_rail_factor(&r.n, rail, &[]), 1.0);
    assert!(has_naval_base(1) && !has_naval_base(0));
}

#[test]
fn naval_supply_limit_per_state() {
    let r = rules();
    let st = |coastal, best_level, non_core_overseas| StateNaval { coastal, best_level, non_core_overseas };
    // level 3 base: 4 x 4 = 16; coastal empty: 2; inland: 0; overseas level 1: 4 x 0.5 = 2.
    let limit = naval_supply_limit(&r.n, &[st(true, 3, false), st(true, 0, false), st(false, 0, false), st(true, 1, true)]);
    assert_eq!(limit, 16.0 + 2.0 + 2.0);
    // Overseas score is never below 1.
    assert_eq!(naval_supply_limit(&r.n, &[st(true, 0, true)]), 1.0);
}

fn prov(owner: &str, fort: f64, nb: f64) -> ProvinceMilitary {
    ProvinceMilitary { owner: Some(owner.into()), controller: Some(owner.into()), fort, naval_base: nb, ..Default::default() }
}

#[test]
fn peace_destroys_levels_of_the_loser_only() {
    let mut p = vec![prov("LOS", 3.0, 1.0), prov("WIN", 2.0, 2.0), prov("LOS", 0.0, 2.0)];
    assert_eq!(destroy_buildings(&mut p, "LOS", Destroy::Forts), 1);
    assert_eq!((p[0].fort, p[0].naval_base, p[1].fort), (0.0, 1.0, 2.0));
    assert_eq!(destroy_buildings(&mut p, "LOS", Destroy::NavalBases), 2);
    assert_eq!((p[0].naval_base, p[2].naval_base, p[1].naval_base), (0.0, 0.0, 2.0));
}

fn cand(province: usize, capital: bool, listed: bool, nbs: &[(u8, f64)]) -> FortCandidate {
    FortCandidate { province, capital, listed, buildable: true, colonial_state: false, neighbours: nbs.iter().map(|&(border_kind, threat)| Neighbour { border_kind, threat }).collect() }
}

#[test]
fn fort_score_and_choice() {
    assert_eq!(fort_score(&cand(0, true, true, &[])), 1000.0);
    // land border: +200, threat 200 x 2; river: threat 160 x 1.5; low threat adds nothing.
    assert_eq!(fort_score(&cand(0, false, false, &[(1, 200.0), (2, 160.0), (2, 100.0)])), 200.0 + 400.0 + 240.0);
    let r = rules();
    let cs = vec![cand(1, false, false, &[(1, 0.0)]), cand(2, true, false, &[]), FortCandidate { buildable: false, ..cand(3, true, true, &[]) }];
    let plan = ai_fort(&r, 0.3, &cs, 1000.0, 100.0).unwrap();
    assert_eq!((plan.province, plan.kind.as_str()), (2, "fort"));
    assert!(ai_fort(&r, 0.6, &cs, 1000.0, 100.0).is_none(), "dice above CHANCE_BUILD_FORT");
    assert!(ai_fort(&r, 0.3, &cs, 200.0, 100.0).is_none(), "treasury must exceed twice the price");
    assert!(ai_fort(&r, 0.3, &[cand(1, false, false, &[])], 1000.0, 100.0).is_none(), "score 0");
    let colonial = FortCandidate { colonial_state: true, ..cand(4, true, true, &[]) };
    assert!(ai_fort(&r, 0.3, &[colonial], 1000.0, 100.0).is_none());
}

#[test]
fn naval_base_chance_is_a_third_for_weak_countries() {
    let r = rules();
    assert!(ai_naval_base(&r, 0.7, false, Some(5), 100.0, 50.0).is_some());
    assert!(ai_naval_base(&r, 0.7, true, Some(5), 100.0, 50.0).is_none());
    assert!(ai_naval_base(&r, 0.2, true, Some(5), 100.0, 50.0).is_some());
    assert!(ai_naval_base(&r, 0.2, false, Some(5), 40.0, 50.0).is_none());
    assert!(ai_naval_base(&r, 0.2, false, None, 100.0, 50.0).is_none());
}

#[test]
fn railway_goes_to_the_state_with_most_factories() {
    let r = rules();
    let st = |state, factories, colonial, province, price| RailState { state, factories, colonial, province, price };
    let states = vec![st(0, 2, false, Some(10), 20.0), st(1, 5, false, Some(11), 20.0), st(2, 9, true, Some(12), 20.0), st(3, 7, false, None, 20.0)];
    let plan = ai_railway(&r, 0.1, true, &states, 1000.0, false, 1.0).unwrap();
    assert_eq!((plan.province, plan.kind.as_str(), plan.price), (11, "infrastructure", 20.0));
    assert_eq!(ai_railway(&r, 0.1, true, &states, 1000.0, true, 1.0).unwrap().price, 60.0);
    // Measured: a run orders one whenever it may (FLD_BLD_AI_RAIL_CHANCE = 1).
    assert!(ai_railway(&r, 0.99, true, &states, 1000.0, false, 1.0).is_some());
    assert!(ai_railway(&r, 0.1, false, &states, 1000.0, false, 1.0).is_none());
    // Needs the price plus 25 x the goods price in the treasury; abroad it needs the x3 price.
    assert!(ai_railway(&r, 0.1, true, &states, 40.0, false, 1.0).is_none());
    assert!(ai_railway(&r, 0.1, true, &states, 50.0, true, 1.0).is_none());
}
