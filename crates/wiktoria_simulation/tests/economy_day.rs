//! The rules of the daily economy step and of factory building on made-up numbers.

use std::collections::BTreeMap;

use wiktoria_simulation::economy::*;
use wiktoria_simulation::industry::*;
use wiktoria_simulation::script::ProvinceState;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn hiring() -> Hiring {
    Hiring { hire_lowest: 0.001, fire_lowest: 0.001, rgo_hire_hi: 0.2, rgo_hire_lo: 0.02, rgo_fire: 0.4, speed_factory: STAFF_SPEED_FACTORY, fire_divisor: 4.0, step_floor: 50.0, rgo_fire_above: 1.05, rgo_hire_hi_below: 0.9, rgo_hire_lo_below: 1.0, rgo_fast_hire_factor: 25.0 }
}

// The rules file's values (FLD_*), copied so the tests read on their own.
const STAFF_SPEED_FACTORY: f64 = 0.15;
const POP_NEEDS_MEASURED_FACTOR: f64 = 1.25;
const POP_DISCOUNT_BASE: f64 = 0.7975;
const POP_CASH_CAP: f64 = 1500.0;
const POP_KEEP_PAY_SHARE: f64 = 0.257;

fn n() -> EconomyNumbers {
    EconomyNumbers {
        factory_budget_growth: 1.05,
        factory_budget_cash_share: 0.1,
        factory_pay_reserve_days: 7.0,
        factory_overflow_share: 0.0,
        factory_no_input_days: 11,
        industrial_score_per_level: 4.0,
        industrial_score_min_staff: 0.05,
        pop_needs_measured_factor: POP_NEEDS_MEASURED_FACTOR,
        pop_discount_base: POP_DISCOUNT_BASE,
        pop_savings_cost_days: 2.0,
        pop_savings_luxury_met: 0.99,
        pop_cash_cap: POP_CASH_CAP,
        pop_keep_pay_share: POP_KEEP_PAY_SHARE,
        pop_hold_pay_share: 0.036,
        pop_luxury_with_everyday: [0.21, 0.15, 0.18],
        invest_score_capital: 1000.0,
        invest_score_core: 200.0,
        invest_score_factory: 25.0,
        invest_score_not_colony: 50.0,
        loan_min_interest: 0.01,
        loan_interest_days: 30.0,
        ..Default::default()
    }
}

fn iai() -> IndustryAi {
    IndustryAi { reopen_cost_share: 0.1, build_period: 18.0, subsidy_off_loss_days: 30, subsidy_review_loss_days: 7, input_short: -100_000.0, output_glut: -10_000.0, top_up_new: 2.0, top_up_old: 1.5 }
}

fn budget() -> AiBudget {
    AiBudget { repay_share: 0.05, repay_max: 50.0, repay_min: 1.0, repay_treasury: 100.0, surplus_share: 0.1, reserve_days: 365.0, spend_down_share: 0.05, rich_days: 730.0, period_days: 18.0, spending_floor: 0.2, spending_cap: 0.75, education_first_share: 1.0 / 3.0, tariff_cap_peace: 0.5, tariff_base_min: 0.01 }
}

fn map(list: &[(&str, f64)]) -> BTreeMap<String, f64> {
    list.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn order(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn tariff_rate_adds_modifier_and_administration_up_to_one() {
    assert!(close(tariff_rate(0.5, 0.2, 0.05, 0.5), 0.375));
    // The efficiency stops at 1.
    assert!(close(tariff_rate(0.5, 0.2, 0.3, 0.9), 0.5));
    // A negative slider gives a negative rate: the buyers pay less and the state pays.
    assert!(close(tariff_rate(-0.2, 0.2, 0.0, 0.3), -0.1));
}

#[test]
fn world_market_serves_in_rank_order() {
    let (made, wanted) = (map(&[("CCC", 10.0)]), map(&[("AAA", 8.0), ("BBB", 8.0)]));
    let none = BTreeMap::new();
    let x = clear_good(&order(&["AAA", "BBB"]), &made, &wanted, &none);
    assert!(close(x["AAA"].fill.unwrap(), 1.0) && close(x["BBB"].fill.unwrap(), 0.25));
    assert!(close(x["AAA"].imported.unwrap(), 1.0) && close(x["CCC"].sold.unwrap(), 1.0));
    // The other way round the other country goes short; one the ranking does not list comes last.
    let x = clear_good(&order(&["BBB", "AAA"]), &made, &wanted, &none);
    assert!(close(x["BBB"].fill.unwrap(), 1.0) && close(x["AAA"].fill.unwrap(), 0.25));
    let x = clear_good(&order(&["BBB"]), &made, &wanted, &none);
    assert!(close(x["AAA"].fill.unwrap(), 0.25));
}

#[test]
fn uncivilised_take_imports_whole_or_none() {
    // BBB wants 8 with 2 left: a civilised buyer takes the 2, an uncivilised one nothing, and the
    // 2 stay for the next buyer (TST_MARKET_WHOLE).
    let (made, wanted) = (map(&[("CCC", 10.0)]), map(&[("AAA", 8.0), ("BBB", 8.0), ("DDD", 2.0)]));
    let none = BTreeMap::new();
    let x = clear_good_whole(&order(&["AAA", "BBB", "DDD"]), &made, &wanted, &none, &std::collections::BTreeSet::new());
    assert!(close(x["BBB"].fill.unwrap(), 0.25) && close(x["DDD"].fill.unwrap(), 0.0));
    let whole = ["BBB".to_string()].into_iter().collect();
    let x = clear_good_whole(&order(&["AAA", "BBB", "DDD"]), &made, &wanted, &none, &whole);
    assert!(close(x["BBB"].fill.unwrap(), 0.0) && close(x["DDD"].fill.unwrap(), 1.0));
}

#[test]
fn home_goods_come_first_and_are_not_imports() {
    let x = clear_good(&order(&["AAA", "BBB"]), &map(&[("AAA", 6.0), ("BBB", 6.0)]), &map(&[("AAA", 9.0), ("BBB", 2.0)]), &BTreeMap::new());
    // AAA buys its own 6 and 3 of the 4 BBB has left; BBB sells 2 at home and 3 of 4 abroad.
    assert!(close(x["AAA"].fill.unwrap(), 1.0) && close(x["AAA"].imported.unwrap(), 3.0 / 9.0));
    assert!(close(x["BBB"].imported.unwrap(), 0.0) && close(x["AAA"].sold.unwrap(), 1.0));
    assert!(close(x["BBB"].sold.unwrap(), 2.0 / 6.0 + 4.0 / 6.0 * 0.75));
}

#[test]
fn sphere_member_buys_from_a_copy_of_the_leaders_pool() {
    let made = map(&[("LLL", 10.0), ("MMM", 4.0)]);
    let wanted = map(&[("LLL", 5.0), ("MMM", 12.0), ("XXX", 20.0)]);
    let leader: BTreeMap<String, (String, f64)> = [("MMM".to_string(), ("LLL".to_string(), 0.75))].into();
    let x = clear_good(&order(&["LLL", "MMM", "XXX"]), &made, &wanted, &leader);
    // The leader's pool is 10 + 3; the member's supply a copy of it plus the 1 it kept: duty free.
    assert!(close(x["MMM"].fill.unwrap(), 1.0) && close(x["MMM"].imported.unwrap(), 0.0));
    assert!(close(x["MMM"].sold.unwrap(), 12.0 / 14.0 + 2.0 / 14.0));
    // Only the leader exports: the 8 it has left, all bought by the outsider.
    assert!(close(x["XXX"].fill.unwrap(), 8.0 / 20.0) && close(x["LLL"].sold.unwrap(), 1.0));
    // Without the sphere the member imports what the leader has left over.
    let x = clear_good(&order(&["LLL", "MMM"]), &made, &map(&[("LLL", 5.0), ("MMM", 12.0)]), &BTreeMap::new());
    assert!(close(x["MMM"].fill.unwrap(), 9.0 / 12.0) && close(x["MMM"].imported.unwrap(), 5.0 / 9.0));
}

#[test]
fn sphere_member_draws_on_what_the_leader_really_has_left() {
    let leader: BTreeMap<String, (String, f64)> = [("MMM".to_string(), ("LLL".to_string(), 1.0))].into();
    let x = clear_good(&order(&["LLL", "MMM", "XXX"]), &map(&[("LLL", 10.0)]), &map(&[("LLL", 2.0), ("MMM", 14.0), ("XXX", 9.0)]), &leader);
    // 10 from the copy, 4 more from the 8 really left; 4 remain for the world.
    assert!(close(x["MMM"].fill.unwrap(), 1.0) && close(x["XXX"].fill.unwrap(), 4.0 / 9.0));
}

#[test]
fn hiring_and_firing_steps() {
    let h = hiring();
    // Hiring: 15% of the gap, at least 50, never more than the gap.
    assert!(close(staffing_step(1000.0, 10000.0, STAFF_SPEED_FACTORY, &h), 2350.0));
    assert!(close(staffing_step(9950.0, 10000.0, STAFF_SPEED_FACTORY, &h), 10000.0));
    assert!(close(staffing_step(9000.0, 9100.0, STAFF_SPEED_FACTORY, &h), 9050.0));
    // Firing: a quarter of that speed, at least 50.
    assert!(close(staffing_step(10000.0, 0.0, STAFF_SPEED_FACTORY, &h), 9625.0));
    assert!(close(staffing_step(1000.0, 0.0, STAFF_SPEED_FACTORY, &h), 950.0));
    assert!(close(staffing_step(30.0, 0.0, STAFF_SPEED_FACTORY, &h), 0.0));
}

#[test]
fn farms_and_mines_follow_supply_over_demand() {
    let h = hiring();
    // Supply 20% over demand: fire at 0.4 x 0.2 of the people, a quarter of it a day.
    assert!(close(rgo_staff_step(10000.0, 10000.0, 120.0, Some(100.0), false, &h), 9800.0));
    // Well short: the high factor; a little short: the low one (here the floor of 50).
    assert!(close(rgo_staff_step(5000.0, 10000.0, 80.0, Some(100.0), false, &h), 5200.0));
    assert!(close(rgo_staff_step(5000.0, 10000.0, 95.0, Some(100.0), false, &h), 5050.0));
    // Between 1 and 1.05 nothing moves; without a demand figure everybody is hired.
    assert!(close(rgo_staff_step(5000.0, 10000.0, 102.0, Some(100.0), false, &h), 5000.0));
    assert!(close(rgo_staff_step(5000.0, 10000.0, 102.0, None, false, &h), 10000.0));
    // In the fast days: the factor x 25 of the gap.
    assert!(close(rgo_staff_step(5000.0, 10000.0, 95.0, Some(100.0), true, &h), 7500.0));
}

#[test]
fn farms_hire_below_the_engine_rules_low_threshold() {
    // The low hiring band ends at FLD_RGO_HIRE_LO_BELOW, not at a number in the code.
    let h = Hiring { rgo_hire_lo_below: 0.96, ..hiring() };
    assert!(close(rgo_staff_step(5000.0, 10000.0, 97.0, Some(100.0), false, &h), 5000.0));
    assert!(close(rgo_staff_step(5000.0, 10000.0, 95.0, Some(100.0), false, &h), 5050.0));
}

#[test]
fn artisans_make_what_they_bought_when_the_throttle_moves() {
    // A pop of 20,000 at throttle 1 (workforce 10,000, value 2) buys a full day of inputs.
    let bought = artisan_units(1.0, 20000.0, 10000.0, 1.0, 2.0);
    assert!(close(bought, 4.0));
    // It could not pay for something today, so its throttle falls to 0.85 before tomorrow:
    // tomorrow it still makes the 4 units it paid for (before the fix: 3.4).
    let stock = artisan_stock_after_throttle(1.0, 1.0, 0.85);
    assert!(close(artisan_units(0.85, 20000.0, 10000.0, stock, 2.0), bought));
    // Fully supplied, the throttle rises from 0.5 to 0.575: no free output either.
    let half = artisan_units(0.5, 20000.0, 10000.0, 1.0, 2.0);
    let stock = artisan_stock_after_throttle(1.0, 0.5, 0.575);
    assert!(close(artisan_units(0.575, 20000.0, 10000.0, stock, 2.0), half));
    // Unchanged throttle, unchanged stock; no workforce, nothing made.
    assert!(close(artisan_stock_after_throttle(0.4, 0.7, 0.7), 0.4));
    assert!(close(artisan_units(1.0, 20000.0, 0.0, 1.0, 2.0), 0.0));
}

#[test]
fn artisans_are_taxed_on_their_margin() {
    // Sold 100, paid 70 for inputs, efficiency 0.4, slider 0.5: base 12, tax 6.
    let (base, tax) = pop_tax(100.0, 70.0, 0.4, 0.5);
    assert!(close(base, 12.0) && close(tax, 6.0));
    // A pop that buys no inputs is taxed on all it earned; a loss is not taxed.
    assert!(close(pop_tax(100.0, 0.0, 0.4, 0.5).1, 20.0));
    assert!(close(pop_tax(50.0, 70.0, 0.4, 0.5).1, 0.0));
}

#[test]
fn nobody_works_where_the_owner_lost_control() {
    let mut p = ProvinceState { owner: Some("AAA".into()), controller: Some("AAA".into()), ..Default::default() };
    assert!(owner_controls(&p));
    p.controller = Some("BBB".into());
    assert!(!owner_controls(&p));
}

#[test]
fn factory_input_budget_grows_slowly() {
    assert!(close(factory_input_budget(&n(), 1000.0, 100.0), 105.0));
    assert!(close(factory_input_budget(&n(), 1000.0, 50.0), 100.0));
    assert!(close(factory_input_budget(&n(), 40.0, 100.0), 40.0));
}

#[test]
fn factory_shuts_after_eleven_days_without_inputs() {
    assert_eq!(no_input_day(&n(), 1.0, 0, false, false), (1.0, 1));
    assert_eq!(no_input_day(&n(), 1.0, 10, false, false), (1.0, 11));
    assert_eq!(no_input_day(&n(), 1.0, 11, false, false), (1.0, 11));
    // A larger one loses a level and starts counting again.
    assert_eq!(no_input_day(&n(), 3.0, 10, false, false), (2.0, 0));
    // Buying resets the count; a subsidised factory never counts.
    assert_eq!(no_input_day(&n(), 1.0, 10, true, false), (1.0, 0));
    assert_eq!(no_input_day(&n(), 1.0, 10, false, true), (1.0, 0));
}

#[test]
fn factory_pay_split() {
    // Profit 40, cash 500 of at most 1000: pay 0.25 x 40 x 0.5 = 5, half each.
    let x = factory_payout(&n(), 500.0, 100.0, 60.0, 50.0, 2.0, 1.0, 0.25, 1000.0, true);
    assert!(close(x.workers, 2.5) && close(x.owners, 2.5) && close(x.cash, 495.0));
    // The divisor grows with the level.
    let x = factory_payout(&n(), 500.0, 100.0, 60.0, 50.0, 2.0, 2.0, 0.25, 1000.0, true);
    assert!(close(x.workers, 2.0) && close(x.owners, 0.5));
    // Cash not above 7 days of inputs: no profit pay, only the minimum wage.
    let x = factory_payout(&n(), 300.0, 100.0, 60.0, 50.0, 2.0, 1.0, 0.25, 1000.0, true);
    assert!(close(x.workers, 2.0) && close(x.owners, 0.0) && close(x.cash, 298.0));
    // No owners in the state: everything to the workers, and no minimum.
    let x = factory_payout(&n(), 500.0, 100.0, 60.0, 50.0, 2.0, 1.0, 0.25, 1000.0, false);
    assert!(close(x.workers, 5.0) && close(x.owners, 0.0));
    let x = factory_payout(&n(), 300.0, 100.0, 60.0, 50.0, 2.0, 1.0, 0.25, 1000.0, false);
    assert!(close(x.workers, 0.0));
    // Cash above the limit is cut back to it and not paid out (the original's capped factories pay 0.25 x profit only).
    let x = factory_payout(&n(), 1200.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.25, 1000.0, true);
    assert!(close(x.owners, 0.0) && close(x.cash, 1000.0));
}

#[test]
fn ranking_puts_civilised_first_then_score() {
    assert!(close(industrial_score(&n(), &[(5.0, 50000.0, 25000.0)]), 10.0));
    assert!(close(industrial_score(&n(), &[(2.0, 20000.0, 100.0), (1.0, 0.0, 500.0)]), 0.4));
    // People and places count in whole hundreds: 1,332 people on 10,000 places are 13 of 100.
    assert!(close(industrial_state_score(&n(), 1.0, 10000.0, 1332.0), 0.52));
    assert!(close(industrial_state_score(&n(), 5.0, 50000.0, 35410.0), 14.16));
    assert!(close(industrial_state_score(&n(), 1.0, 0.0, 500.0), 0.0));
    let ranked = rank_order(vec![("AAA".into(), false, 500.0), ("BBB".into(), true, 10.0), ("CCC".into(), true, 30.0)]);
    assert_eq!(ranked, order(&["CCC", "BBB", "AAA"]));
}

#[test]
fn capitalists_choose_a_project() {
    let all = Policy { pop_build_factory: true, pop_expand_factory: true, pop_open_factory: true, ..Default::default() };
    assert_eq!(project_weights(&all, true, true, 2, 3), (1.0, 2.0, 3.0));
    // Room but idle factories: nothing new and nothing enlarged.
    assert_eq!(project_weights(&all, true, false, 2, 3), (0.0, 2.0, 0.0));
    assert_eq!(project_weights(&all, false, false, 2, 3), (0.0, 2.0, 3.0));
    assert_eq!(project_weights(&Policy::default(), true, true, 2, 3), (0.0, 0.0, 0.0));
}

#[test]
fn capitalists_score_a_factory_type() {
    assert!(close(capitalist_type_weight(1, 0, 1, 0), 0.475));
    assert!(close(capitalist_type_weight(0, 1, 0, 1), 0.0001));
    // Margin capped at 100, inputs half available, output wanted twice over.
    assert!(close(capitalist_type_score(150.0, &[(10.0, 100.0, 95.0)], (50.0, 200.0), 1.0), 100.0));
    // An input the world is short of rules the type out.
    assert!(close(capitalist_type_score(150.0, &[(10.0, 100.0, 95.0), (1.0, 5.0, 6.0)], (50.0, 200.0), 1.0), 0.0));
}

#[test]
fn government_scores_type_and_state() {
    assert!(close(government_type_score(&iai(), &[(2.0, 100.0, 90.0), (1.0, 50.0, 60.0)], (3.0, 10.0, 30.0)), -49930.0));
    assert!(close(government_type_score(&iai(), &[], (3.0, 30.0, 10.0)), -10060.0));
    assert!(close(government_state_value(20000.0, 100000.0, 1000.0, 10000.0, false, false), 3.0));
    assert!(close(government_state_value(20000.0, 100000.0, 1000.0, 10000.0, true, false), 2.0));
    assert!(close(government_state_value(20000.0, 100000.0, 1000.0, 10000.0, false, true), 5.0));
}

#[test]
fn ai_subsidies() {
    // A budget in the black: every running factory is subsidised.
    assert_eq!(subsidy_change(&iai(), 10.0, 0, false, 0.0, 10000.0), Some(true));
    assert_eq!(subsidy_change(&iai(), 0.0, 0, false, 0.0, 10000.0), None);
    // Off after 30 loss days; from 7 when the workers would find places next door.
    assert_eq!(subsidy_change(&iai(), 10.0, 30, true, 0.0, 10000.0), Some(false));
    assert_eq!(subsidy_change(&iai(), 10.0, 30, false, 0.0, 10000.0), None);
    assert_eq!(subsidy_change(&iai(), 10.0, 10, true, 20000.0, 10000.0), Some(false));
    assert_eq!(subsidy_change(&iai(), 10.0, 10, true, 5000.0, 10000.0), None);
}

#[test]
fn needs_grow_with_consciousness_and_inventions() {
    // 200,000 people per line; at consciousness 5 half as much again, at 10 twice.
    assert!(close(pop_scale(100000.0, 0.0, 200000.0, 20.0), 0.5));
    assert!(close(pop_scale(100000.0, 5.0, 200000.0, 20.0), 0.75));
    assert!(close(pop_scale(100000.0, 10.0, 200000.0, 20.0), 1.0));
    // Life needs: base demand + modifier, x the strata's modifier, x the measured factor.
    assert!(close(tier_factor(&n(), 0, 0.8, 0.0, 0.0, 10.0, 0.005), 0.8 * POP_NEEDS_MEASURED_FACTOR));
    assert!(close(tier_factor(&n(), 0, 0.8, 0.1, -0.2, 10.0, 0.005), 0.9 * 0.8 * POP_NEEDS_MEASURED_FACTOR));
    // Everyday and luxury needs grow with inventions.
    assert!(close(tier_factor(&n(), 1, 0.8, 0.0, 0.0, 10.0, 0.005), 0.8 * 1.05 * POP_NEEDS_MEASURED_FACTOR));
    assert!(close(tier_factor(&n(), 2, 0.8, 0.0, 0.0, 10.0, 0.005), 0.8 * 1.05 * POP_NEEDS_MEASURED_FACTOR));
}

#[test]
fn pops_order_a_share_of_the_whole_tier_and_spend_all_their_cash() {
    // Enough cash: the whole tier at its price x discount.
    let x = buy_tier(100.0, 50.0, 50.0, 1.0, 0.8, true);
    assert!(close(x.share, 1.0) && close(x.paid, 40.0) && close(x.met, 1.0));
    // Short: all its cash orders 20 / (50 x 0.8) of the tier.
    let x = buy_tier(20.0, 50.0, 50.0, 1.0, 0.8, true);
    assert!(close(x.share, 0.5) && close(x.paid, 20.0) && close(x.met, 0.5));
    // Half the goods missing, civilised: met and paid as ordered all the same.
    let x = buy_tier(20.0, 50.0, 25.0, 0.5, 0.8, true);
    assert!(close(x.share, 0.5) && close(x.paid, 20.0) && close(x.met, 0.5));
    // Uncivilised: only what can be had is met and paid for; it keeps the rest.
    let x = buy_tier(20.0, 50.0, 25.0, 0.5, 0.8, false);
    assert!(close(x.share, 0.5) && close(x.paid, 10.0) && close(x.met, 0.25));
    // Nothing to buy: nothing bought, nothing paid.
    assert_eq!(buy_tier(10.0, 0.0, 0.0, 0.0, 0.8, true), TierBought::default());
}

#[test]
fn short_of_everyday_needs_a_pop_buys_luxury_with_them() {
    let k = n().pop_luxury_with_everyday[0];
    // Everyday 40, luxury 60 (prices x discount), 20 of cash: everyday gets 20 x 40 / (40 + 60k),
    // so that luxury met (the rest / 60) is k x everyday met (its part / 40).
    let e = everyday_cash(20.0, 40.0, 60.0, k);
    assert!(close(e, 20.0 * 40.0 / (40.0 + 60.0 * k)));
    assert!(close((20.0 - e) / 60.0, k * e / 40.0));
    // Enough for the everyday tier, or nothing to buy for luxury: everyday first, as before.
    assert!(close(everyday_cash(50.0, 40.0, 60.0, k), 50.0));
    assert!(close(everyday_cash(20.0, 40.0, 0.0, k), 20.0));
}

#[test]
fn cash_left_above_a_quarter_of_the_pay_buys_more_luxury() {
    let keep = n().pop_keep_pay_share * 40.0;
    // 30 left after the shopping, a day's pay of 40: it keeps 10.28, the rest orders luxury
    // goods at the discounted price of a whole tier of 50, all of which can be had.
    let (share, paid) = luxury_overbuy(30.0, keep, 50.0, 50.0, 0.8);
    assert!(close(paid, 30.0 - keep) && close(share, (30.0 - keep) / 40.0));
    // Only 60% of the tier can be had: the same order, it pays for 60% of it.
    let (share2, paid2) = luxury_overbuy(30.0, keep, 30.0, 50.0, 0.8);
    assert!(close(share2, share) && close(paid2, 0.6 * (30.0 - keep)));
    // Less left than it keeps, or no luxury goods to be had: nothing.
    assert_eq!(luxury_overbuy(8.0, keep, 50.0, 50.0, 0.8), (0.0, 0.0));
    assert_eq!(luxury_overbuy(30.0, keep, 0.0, 50.0, 0.8), (0.0, 0.0));
    // No pay today: all that is left is offered.
    assert!(close(luxury_overbuy(30.0, 0.0, 50.0, 50.0, 1.0).1, 30.0));
}

#[test]
fn discount_moves_with_the_worlds_money() {
    // No yesterday: full price.
    assert!(close(pop_discount(&n(), 1000.0, None, 0.0), 1.0));
    // Money stood still: the base; it grew by 1%: one point more; it shrank: less.
    assert!(close(pop_discount(&n(), 1000.0, Some(1000.0), 0.0), POP_DISCOUNT_BASE));
    assert!(close(pop_discount(&n(), 1000.0, Some(990.0), 0.0), POP_DISCOUNT_BASE + 0.01));
    assert!(close(pop_discount(&n(), 1000.0, Some(1010.0), 0.0), POP_DISCOUNT_BASE - 0.01));
    // Gold money counts on both sides; above 1 or not positive means 1.
    assert!(close(pop_discount(&n(), 990.0, Some(990.0), 10.0), POP_DISCOUNT_BASE + 0.01));
    assert!(close(pop_discount(&n(), 1000.0, Some(500.0), 0.0), 1.0));
    assert!(close(pop_discount(&n(), 100.0, Some(1000.0), 0.0), 1.0));
}

#[test]
fn pops_deposit_and_withdraw() {
    // Two days of needs in hand and luxury needs met: POP_SAVINGS of the cash goes to the bank.
    assert!(close(savings_step(&n(), 100.0, 5.0, 10.0, 1.0, 0.018, 50.0), 1.8));
    // Luxury needs not met: the cash stays with the pop.
    assert!(close(savings_step(&n(), 100.0, 5.0, 10.0, 0.5, 0.018, 50.0), 0.0));
    // Less than two days of needs: the gap comes out of the savings, as far as they and the
    // bank's unlent money go.
    assert!(close(savings_step(&n(), 5.0, 30.0, 10.0, 0.0, 0.018, 50.0), -15.0));
    assert!(close(savings_step(&n(), 5.0, 8.0, 10.0, 0.0, 0.018, 50.0), -8.0));
    assert!(close(savings_step(&n(), 5.0, 30.0, 10.0, 0.0, 0.018, 4.0), -4.0));
    // Without savings a pop with its luxury needs met deposits even when short of cash.
    assert!(close(savings_step(&n(), 5.0, 0.0, 10.0, 1.0, 0.018, 0.0), 0.09));
    // Cash above the cap is deposited whatever else.
    assert!(close(savings_step(&n(), POP_CASH_CAP + 100.0, 0.0, 10.0, 0.0, 0.018, 0.0), 100.0));
}

#[test]
fn capitalists_money_goes_to_the_best_placed_state() {
    // The capital with two more core provinces and three factories, against a colony.
    assert!(close(investment_state_score(&n(), &[(true, true), (false, true), (false, true)], 3, false), 1000.0 + 600.0 + 75.0 + 50.0));
    assert!(close(investment_state_score(&n(), &[(false, false)], 0, true), 0.0));
    assert!(investment_state_score(&n(), &[(false, true)], 8, false) > investment_state_score(&n(), &[(false, true)], 0, false));
}

#[test]
fn stockpile_wants_one_day_by_the_sliders() {
    // Army 10 at slider 0.3, navy 4 at 0.5, overseas 1 without a slider, building work 20 at 0.75.
    assert!(close(stockpile_target([0.3, 0.5, 0.75], 10.0, 4.0, 1.0, 20.0), 3.0 + 2.0 + 1.0 + 15.0));
    assert!(close(stockpile_target([0.0, 0.0, 0.0], 10.0, 4.0, 0.0, 20.0), 0.0));
}

#[test]
fn building_sites_take_goods_and_hand_money_back() {
    let mut lacks = map(&[("steel", 10.0), ("cement", 5.0)]);
    let mut open = map(&[("steel", 4.0), ("cement", 8.0), ("wine", 3.0)]);
    let price = |good: &str| if good == "steel" { 5.0 } else { 2.0 };
    // 4 steel and 5 cement arrive: 30 goes back to the treasury; 6 steel are still lacking.
    assert!(close(construction_takes(&mut lacks, &mut open, 100.0, price), 30.0));
    assert_eq!(lacks, map(&[("steel", 6.0)]));
    assert!(close(open["steel"], 0.0) && close(open["cement"], 3.0) && close(open["wine"], 3.0));
    // No more than the money the site still holds.
    let mut open = map(&[("steel", 6.0)]);
    assert!(close(construction_takes(&mut lacks, &mut open, 12.0, price), 12.0));
    assert!(lacks.is_empty());
}

#[test]
fn loans_offers_and_interest() {
    // A foreign bank: its unlent money, at most 3 x the borrower's tax base less what it owes.
    assert!(close(foreign_bank_offer(500.0, 3.0, 0.0, 100.0, 50.0), 250.0));
    assert!(close(foreign_bank_offer(100.0, 3.0, 0.0, 100.0, 50.0), 100.0));
    assert!(close(foreign_bank_offer(500.0, 3.0, -1.0, 100.0, 0.0), 0.0));
    assert!(close(foreign_bank_offer(500.0, 3.0, 0.0, 100.0, 400.0), 0.0));
    // Interest: the rate is for 30 days; the modifier is not charged by the own bank.
    assert!(close(loan_interest(&n(), 1500.0, 0.02, 0.0, false), 1.0));
    assert!(close(loan_interest(&n(), 1500.0, 0.02, 0.05, false), 1.05));
    assert!(close(loan_interest(&n(), 1500.0, 0.02, 0.05, true), 1.0));
}

#[test]
fn computer_player_sets_its_budget() {
    let all = [0.0, 1.0, 1.0];
    // Tax base 100 (60, 30, 10), both salaries 50 at full, 40 of other spending, spending 80 and a
    // poor treasury: it wants +8.
    let day = BudgetDay { tax_base: [60.0, 30.0, 10.0], tariff_base: 40.0, education: 50.0, administration: 50.0, other: -40.0, spending: 80.0, treasury: 1000.0 };
    // At the floor: 100 - 40 - 20 = +40, so 32 to spend: a third to education, the rest to
    // administration (below its cap).
    let x = ai_budget_solve(&budget(), &day, all, false);
    assert_eq!((x.tariff, x.taxes), (0.0, [1.0; 3]));
    assert!(close(x.education, 0.2 + 32.0 / 3.0 / 50.0) && close(x.administration, 0.2 + 64.0 / 3.0 / 50.0));
    // More money than both caps take: the rest is a tax cut on all strata alike.
    let rich = BudgetDay { other: 0.0, ..day };
    let x = ai_budget_solve(&budget(), &rich, all, false);
    assert!(close(x.education, 0.75) && close(x.administration, 0.75));
    assert!(close(x.taxes[0], 1.0 - (100.0 - 75.0 - 8.0) / 100.0) && x.taxes[0] == x.taxes[2]);
    // Short of the target: both salaries at the floor and the tariff makes up the gap, up to 0.5
    // at peace and the party's top at war.
    let poor = BudgetDay { other: -80.0, ..day };
    let x = ai_budget_solve(&budget(), &poor, all, false);
    assert_eq!((x.education, x.administration, x.taxes), (0.2, 0.2, [1.0; 3]));
    assert!(close(x.tariff, 8.0 / 40.0));
    let broke = BudgetDay { other: -120.0, ..day };
    assert!(close(ai_budget_solve(&budget(), &broke, all, false).tariff, 0.5));
    assert!(close(ai_budget_solve(&budget(), &broke, all, true).tariff, 1.0));
    // The party's limits hold: taxes at most its top, cuts no lower than its bottom.
    assert_eq!(ai_budget_solve(&budget(), &poor, [0.1, 0.5, 1.0], false).taxes, [0.5; 3]);
    let flush = BudgetDay { other: 500.0, ..day };
    assert!(close(ai_budget_solve(&budget(), &flush, [0.1, 0.5, 1.0], false).taxes[1], 0.1));
}

/// The government pays into the project that is furthest along, a new factory
/// before a reopening, and never more than a quarter of its treasury.
#[test]
fn a_government_tops_up_its_capitalists_best_project() {
    use wiktoria_simulation::industry::project_top_up;
    // Paid, price, new factory.
    let projects = [(100.0, 1000.0, true), (900.0, 1000.0, false), (500.0, 1000.0, true), (1000.0, 1000.0, true)];
    // 0.5 + 2 beats 0.9 + 1.5; the full one is not looked at. 500 are missing, the treasury gives them.
    assert_eq!(project_top_up(&iai(), &projects, 10_000.0, 0.25), Some((2, 500.0)));
    // A poor treasury gives a quarter of what it has.
    assert_eq!(project_top_up(&iai(), &projects, 400.0, 0.25), Some((2, 100.0)));
    assert_eq!(project_top_up(&iai(), &projects, 0.0, 0.25), None);
    assert_eq!(project_top_up(&iai(), &[(1000.0, 1000.0, true)], 10_000.0, 0.25), None);
}
