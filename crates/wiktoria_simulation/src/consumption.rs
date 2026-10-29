//! Pop needs and consumption.
//!
//! SCAFFOLDING, NOT VERIFIED (D9). The consciousness scaling and buying order below are a plausible guess, not taken from the
//! original and not measured; constants are hard-coded instead of read from the
//! rules file. is a `hypothesis` row with no measurement. Do not treat a match with this
//! code as proof. It is listed as scaffolding to replace in docs/AI_STATUS.md.
//!
//! Pops have three categories of needs: Life, Everyday, and Luxury.
//! Needs are defined per 10,000 population size (common/poptypes/*.txt).
//!
//! * Life Needs: Food and shelter required to survive. Does not scale with consciousness.
//!   If life needs are less than 100% fulfilled, pops can starve (population decline).
//! * Everyday Needs: Base comfort needs. Scales with consciousness:
//!   `everyday_need = base_need * (1 + consciousness * 10%)`
//! * Luxury Needs: Advanced comfort needs. Scales with consciousness:
//!   `luxury_need = base_need * (1 + consciousness * 10%)`
//!
//! Order of buying:
//! 1. Life needs are bought first using the pop's money.
//! 2. If life needs are 100% satisfied, remaining money is spent on everyday needs.
//! 3. If everyday needs are 100% satisfied, remaining money is spent on luxury needs.
//!
//! Fulfillment is the ratio of bought goods over total needed goods for each category.

use std::collections::BTreeMap;

use wiktoria_data::defs::PopType;
use crate::fixed::Fixed;

/// Evaluates a pop's total needed units for a single good.
pub fn pop_good_need(base_need: f64, size: f64, consciousness: f64, scales_with_con: bool) -> Fixed {
    let scale = Fixed::from_f64(size / 10000.0);
    let base = Fixed::from_f64(base_need);
    let need = base * scale;
    if scales_with_con {
        let con_multiplier = Fixed::ONE + Fixed::from_f64(consciousness) * Fixed::from_f64(0.1);
        need * con_multiplier
    } else {
        need
    }
}

/// Evaluates a pop's total needed units for all goods in each needs category today.
#[derive(Debug, Clone, Default)]
pub struct PopNeeds {
    pub life: BTreeMap<String, Fixed>,
    pub everyday: BTreeMap<String, Fixed>,
    pub luxury: BTreeMap<String, Fixed>,
}

pub fn calculate_pop_needs(pop_type: &PopType, size: f64, consciousness: f64) -> PopNeeds {
    let mut needs = PopNeeds::default();
    for (good, amt) in &pop_type.life_needs {
        needs.life.insert(good.clone(), pop_good_need(*amt, size, consciousness, false));
    }
    for (good, amt) in &pop_type.everyday_needs {
        needs.everyday.insert(good.clone(), pop_good_need(*amt, size, consciousness, true));
    }
    for (good, amt) in &pop_type.luxury_needs {
        needs.luxury.insert(good.clone(), pop_good_need(*amt, size, consciousness, true));
    }
    needs
}

/// Simulates a pop buying their needs. Returns:
/// `(remaining_money, [life_fulfillment, everyday_fulfillment, luxury_fulfillment])`
/// and the list of goods actually bought with their amounts.
pub fn simulate_buy_needs(
    needs: &PopNeeds,
    mut money: Fixed,
    get_price: impl Fn(&str) -> Option<Fixed>,
) -> (Fixed, [Fixed; 3], BTreeMap<String, Fixed>) {
    let mut bought: BTreeMap<String, Fixed> = BTreeMap::new();
    let mut fulfillments = [Fixed::ONE, Fixed::ONE, Fixed::ONE];

    // Order of satisfaction: Life -> Everyday -> Luxury
    for (cat_idx, category) in [&needs.life, &needs.everyday, &needs.luxury].iter().enumerate() {
        if category.is_empty() {
            fulfillments[cat_idx] = Fixed::ONE;
            continue;
        }

        let mut total_needed = Fixed::ZERO;
        let mut total_bought = Fixed::ZERO;

        for (good, &needed_amount) in category.iter() {
            total_needed = total_needed + needed_amount;
            if needed_amount.0 == 0 {
                continue;
            }

            let Some(price) = get_price(good) else {
                // If price is missing, they cannot buy the good (0 fulfilled for this good)
                continue;
            };

            if price.0 == 0 {
                // Free good (can buy 100%)
                total_bought = total_bought + needed_amount;
                let entry = bought.entry(good.clone()).or_default();
                *entry = *entry + needed_amount;
                continue;
            }

            // How much they can afford with their current money
            let cost = needed_amount * price;
            if money >= cost {
                money = money - cost;
                total_bought = total_bought + needed_amount;
                let entry = bought.entry(good.clone()).or_default();
                *entry = *entry + needed_amount;
            } else if money.0 > 0 {
                // Buy a fraction of what is needed
                let fraction = Fixed((money.0 << 15) / cost.0).min(Fixed::ONE);
                let bought_amount = needed_amount * fraction;
                money = Fixed::ZERO;
                total_bought = total_bought + bought_amount;
                let entry = bought.entry(good.clone()).or_default();
                *entry = *entry + bought_amount;
                break; // out of money for this category
            } else {
                break; // out of money
            }
        }

        if total_needed.0 > 0 {
            fulfillments[cat_idx] = Fixed((total_bought.0 << 15) / total_needed.0).min(Fixed::ONE);
        } else {
            fulfillments[cat_idx] = Fixed::ONE;
        }

        if money.0 == 0 {
            // Out of money: all subsequent categories get 0% fulfillment
            for i in (cat_idx + 1)..3 {
                fulfillments[i] = Fixed::ZERO;
            }
            break;
        }
    }

    (money, fulfillments, bought)
}
