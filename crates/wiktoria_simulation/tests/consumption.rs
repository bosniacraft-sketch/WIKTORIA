//! Unit tests for POP needs calculations and consumption simulation.

use std::collections::BTreeMap;

use wiktoria_data::defs::PopType;
use wiktoria_simulation::fixed::Fixed;
use wiktoria_simulation::consumption::*;

#[test]
fn pop_needs_calculation_applies_consciousness_multiplier() {
    let pop_type = PopType {
        name: "farmers".into(),
        life_needs: BTreeMap::from([
            ("grain".into(), 2.5),
            ("cattle".into(), 0.75),
        ]),
        everyday_needs: BTreeMap::from([
            ("tea".into(), 4.0),
            ("coal".into(), 1.0),
        ]),
        luxury_needs: BTreeMap::from([
            ("regular_clothes".into(), 3.0),
            ("paper".into(), 1.0),
        ]),
        ..Default::default()
    };

    // Pop size = 10,000 (scale 1.0), con = 5.0 (everyday/luxury needs scaled by 1.5x)
    let needs = calculate_pop_needs(&pop_type, 10000.0, 5.0);

    // Life needs: do not scale with consciousness
    assert_eq!(needs.life["grain"], Fixed::from_f64(2.5));
    assert_eq!(needs.life["cattle"], Fixed::from_f64(0.75));

    // Everyday needs: scale with consciousness (1 + 5 * 0.1 = 1.5)
    // 4 * 1.5 = 6.0 (Fixed(196608)), but float calculation first has a tiny rounding offset
    assert_eq!(needs.everyday["tea"], Fixed(196612));
    assert_eq!(needs.everyday["coal"], Fixed(49153));

    // Luxury needs: scale with consciousness (1 + 5 * 0.1 = 1.5)
    assert_eq!(needs.luxury["regular_clothes"], Fixed(147459));
    assert_eq!(needs.luxury["paper"], Fixed(49153));
}

#[test]
fn pop_buys_needs_in_correct_order_and_throttles_with_limited_cash() {
    let pop_type = PopType {
        name: "farmers".into(),
        life_needs: BTreeMap::from([
            ("grain".into(), 2.5),
            ("cattle".into(), 0.75),
        ]),
        everyday_needs: BTreeMap::from([
            ("tea".into(), 4.0),
            ("coal".into(), 1.0),
        ]),
        luxury_needs: BTreeMap::from([
            ("regular_clothes".into(), 3.0),
            ("paper".into(), 1.0),
        ]),
        ..Default::default()
    };

    // Needs are:
    // Life: grain=2.5, cattle=0.75
    // Everyday (con 5.0 -> 1.5x): tea=6.0, coal=1.5
    // Luxury (con 5.0 -> 1.5x): regular_clothes=4.5, paper=1.5
    let needs = calculate_pop_needs(&pop_type, 10000.0, 5.0);

    // Starting money = 15.0
    let money = Fixed::from_f64(15.0);

    // Price index:
    // grain = 2.0 (cost: 5.0)
    // cattle = 4.0 (cost: 3.0)
    // tea = 1.0 (cost: 6.0)
    // coal = 2.0 (cost: 3.0)
    // regular_clothes = 10.0
    // paper = 5.0
    let prices: BTreeMap<String, Fixed> = BTreeMap::from([
        ("grain".into(), Fixed::from_f64(2.0)),
        ("cattle".into(), Fixed::from_f64(4.0)),
        ("tea".into(), Fixed::from_f64(1.0)),
        ("coal".into(), Fixed::from_f64(2.0)),
        ("regular_clothes".into(), Fixed::from_f64(10.0)),
        ("paper".into(), Fixed::from_f64(5.0)),
    ]);

    let get_price = |g: &str| prices.get(g).copied();

    let (remaining_money, fulfillments, bought) = simulate_buy_needs(&needs, money, get_price);

    // 1. Life needs cost 8.0. Money left: 7.0. Fulfillments should be 100%
    assert_eq!(fulfillments[0], Fixed::ONE);
    assert_eq!(bought["grain"], Fixed::from_f64(2.5));
    assert_eq!(bought["cattle"], Fixed::from_f64(0.75));

    // 2. Everyday needs: tea and coal are processed alphabetically (coal first, then tea).
    // coal cost 3.0. They buy 100% of coal (remaining money 4.0).
    // tea cost 6.0. They buy a fraction (4.0 / 6.0) of tea = 131066 units.
    // Remaining money = 0.
    // Fulfillment = (1.5 + 4.0) / (1.5 + 6.0) = 5.5 / 7.5 = 73.33%
    assert_eq!(remaining_money, Fixed::ZERO);
    assert_eq!(bought["coal"], Fixed(49153));
    assert_eq!(bought["tea"], Fixed(131066));
    assert_eq!(fulfillments[1], Fixed(24028)); // 5.5 / 7.5 * 32768 = 24028

    // 3. Luxury needs should get 0% fulfillment
    assert_eq!(fulfillments[2], Fixed::ZERO);
    assert!(!bought.contains_key("regular_clothes"));
    assert!(!bought.contains_key("paper"));
}
