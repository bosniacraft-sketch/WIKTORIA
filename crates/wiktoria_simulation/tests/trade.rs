//! Unit tests for Global Trade and Market Allocation.

use std::collections::BTreeMap;
use wiktoria_simulation::fixed::Fixed;
use wiktoria_simulation::trade::*;

#[test]
fn global_trade_emulates_domestic_satisfaction_and_rank_ordered_world_market() {
    let prices: BTreeMap<String, Fixed> = BTreeMap::from([
        ("coal".into(), Fixed::from_f64(2.0)),
    ]);

    // 1. UK (Rank 1):
    // Produces 10 Coal (rgo_uk). Needs 5 Coal (pop_uk). Money starts at 100.
    let uk = CountryTrade {
        tag_index: 1,
        rank: 1,
        buy_orders: vec![BuyOrder {
            buyer_id: "pop_uk".into(),
            good: "coal".into(),
            amount_needed: Fixed::from_f64(5.0),
        }],
        sell_orders: vec![SellOrder {
            producer_id: "rgo_uk".into(),
            good: "coal".into(),
            amount_produced: Fixed::from_f64(10.0),
        }],
        money: Fixed::from_f64(100.0),
    };

    // 2. Russia (Rank 2):
    // Produces 0 Coal. Needs 3 Coal (pop_rus). Money starts at 50.
    let rus = CountryTrade {
        tag_index: 2,
        rank: 2,
        buy_orders: vec![BuyOrder {
            buyer_id: "pop_rus".into(),
            good: "coal".into(),
            amount_needed: Fixed::from_f64(3.0),
        }],
        sell_orders: vec![],
        money: Fixed::from_f64(50.0),
    };

    // 3. Austria (Rank 3):
    // Produces 0 Coal. Needs 4 Coal (pop_aus). Money starts at 40.
    let aus = CountryTrade {
        tag_index: 3,
        rank: 3,
        buy_orders: vec![BuyOrder {
            buyer_id: "pop_aus".into(),
            good: "coal".into(),
            amount_needed: Fixed::from_f64(4.0),
        }],
        sell_orders: vec![],
        money: Fixed::from_f64(40.0),
    };

    let mut countries = vec![rus.clone(), uk.clone(), aus.clone()];

    let res = allocate_market(&mut countries, &prices);

    // After allocation, let's find each country in the mutated slice:
    let c_uk = countries.iter().find(|c| c.tag_index == 1).unwrap();
    let c_rus = countries.iter().find(|c| c.tag_index == 2).unwrap();
    let c_aus = countries.iter().find(|c| c.tag_index == 3).unwrap();

    // --- CHECK DOMESTIC SATISFACTION (UK) ---
    // UK bought 5 Coal domestically. Cost = 5 * 2.0 = 10.0.
    // UK also bought 0 Coal from World Market (all satisfied domestically).
    // Money left: 100.0 - 10.0 = 90.0
    assert_eq!(res.buyer_bought["pop_uk"], Fixed::from_f64(5.0));

    // --- CHECK WORLD OPEN MARKET RANK ORDER ---
    // Total coal pooled to World Market from UK: 10.0 - 5.0 (domestic sold) = 5.0 units.
    // Rank 2 Russia gets first access: Needs 3.0, buys 3.0. Fulfill = 100%. Cost = 6.0.
    // Russia money left: 50.0 - 6.0 = 44.0
    assert_eq!(res.buyer_bought["pop_rus"], Fixed::from_f64(3.0));
    assert_eq!(c_rus.money, Fixed::from_f64(44.0));

    // Rank 3 Austria gets second access: Needs 4.0, but only 2.0 left on World Market!
    // Austria buys 2.0. Fulfill = 50%. Cost = 4.0.
    // Austria money left: 40.0 - 4.0 = 36.0
    assert_eq!(res.buyer_bought["pop_aus"], Fixed::from_f64(2.0));
    assert_eq!(c_aus.money, Fixed::from_f64(36.0));

    // --- CHECK REVENUE DISTRIBUTION ---
    // UK's RGO sold: 5.0 (domestic) + 3.0 (Russia) + 2.0 (Austria) = 10.0 units (100% sold!).
    // Total Revenue earned: 10.0 * 2.0 = 20.0
    assert_eq!(res.producer_sold["rgo_uk"], Fixed::from_f64(10.0));
    assert_eq!(res.producer_revenue["rgo_uk"], Fixed::from_f64(20.0));

    // Check UK money:
    // UK paid 10.0 for its own domestic buy.
    // UK money left should be 90.0
    assert_eq!(c_uk.money, Fixed::from_f64(90.0));
}
