//! Global trade and market allocation engine.
//!
//! SCAFFOLDING, NOT VERIFIED (D9). The allocation order and revenue split below are a plausible guess, not taken from the
//! original and not measured; constants are hard-coded instead of read from the
//! rules file. is a `hypothesis` row with no measurement. Do not treat a match with this
//! code as proof. It is listed as scaffolding to replace in docs/AI_STATUS.md.
//!
//! Emulates Victoria 2's rank-ordered trade allocation system:
//! 1. Domestic Satisfaction: Each country fulfills its own demand from its own domestic production.
//! 2. Surplus Pooling: Leftover domestic supply goes to the World Market pool. Leftover demand is sent as world market orders.
//! 3. World Market Allocation: The World Market pool is rationed to remaining orders in Country Rank Order.
//! 4. Revenue Distribution: Money earned from total sales (domestic + world market) is paid back to producers.
//! 5. Buy Recording: Fulfillments and purchases are recorded for all buyers.

use std::collections::BTreeMap;
use crate::fixed::Fixed;

/// A buy order for a specific good from a buyer (pop, artisan, factory).
#[derive(Debug, Clone)]
pub struct BuyOrder {
    pub buyer_id: String, // e.g. "pop_<prov_id>_<index>", "art_300", "fac_300"
    pub good: String,
    pub amount_needed: Fixed,
}

/// A sell order (produced supply) from a producer (RGO, artisan, factory).
#[derive(Debug, Clone)]
pub struct SellOrder {
    pub producer_id: String, // e.g. "rgo_300", "art_300", "fac_300"
    pub good: String,
    pub amount_produced: Fixed,
}

/// A country's trade orders and assets.
#[derive(Debug, Clone)]
pub struct CountryTrade {
    pub tag_index: u16,
    pub rank: u32, // Lower rank = higher priority (rank 1 is best)
    pub buy_orders: Vec<BuyOrder>,
    pub sell_orders: Vec<SellOrder>,
    pub money: Fixed,
}

/// The result of allocating market goods.
#[derive(Debug, Clone, Default)]
pub struct AllocationResult {
    /// Total bought per good.
    pub bought: BTreeMap<String, Fixed>,
    /// Total cost per good.
    pub cost: BTreeMap<String, Fixed>,
    /// Total revenue paid to each producer.
    pub producer_revenue: BTreeMap<String, Fixed>,
    /// Actual amount sold by each producer.
    pub producer_sold: BTreeMap<String, Fixed>,
    /// Actual amount bought by each buyer.
    pub buyer_bought: BTreeMap<String, Fixed>,
}

/// Runs the daily trade and market allocation across all countries.
pub fn allocate_market(
    countries: &mut [CountryTrade],
    prices: &BTreeMap<String, Fixed>,
) -> AllocationResult {
    let mut res = AllocationResult::default();

    // 1. Domestic Satisfaction Pass
    for c in countries.iter_mut() {
        let mut dom_supply: BTreeMap<String, Fixed> = BTreeMap::new();
        for s in &c.sell_orders {
            let entry = dom_supply.entry(s.good.clone()).or_default();
            *entry = *entry + s.amount_produced;
        }

        for b in &c.buy_orders {
            let Some(&price) = prices.get(&b.good) else { continue };
            if b.amount_needed.0 == 0 { continue; }

            let available = dom_supply.get_mut(&b.good);
            if let Some(sup) = available {
                if sup.0 > 0 {
                    let bought_amount = if b.amount_needed <= *sup {
                        b.amount_needed
                    } else {
                        *sup
                    };

                    if c.money >= bought_amount * price {
                        c.money = c.money - bought_amount * price;
                        *sup = *sup - bought_amount;
                        let entry = res.buyer_bought.entry(b.buyer_id.clone()).or_default();
                        *entry = *entry + bought_amount;
                        
                        distribute_domestic_sale(c, &b.good, bought_amount, price, &mut res);
                    }
                }
            }
        }
    }

    // Sort countries by rank (lowest rank is rank 1, highest priority).
    countries.sort_by_key(|c| c.rank);

    // 2. Open Market Pooling
    let mut wm_pool: BTreeMap<String, Fixed> = BTreeMap::new();
    for c in countries.iter() {
        let mut dom_supply: BTreeMap<String, Fixed> = BTreeMap::new();
        for s in &c.sell_orders {
            let entry = dom_supply.entry(s.good.clone()).or_default();
            *entry = *entry + s.amount_produced;
        }
        for b in &c.buy_orders {
            let bought_amount = res.buyer_bought.get(&b.buyer_id).copied().unwrap_or(Fixed::ZERO);
            if let Some(sup) = dom_supply.get_mut(&b.good) {
                *sup = (*sup - bought_amount).max(Fixed::ZERO);
            }
        }
        for (good, sup) in dom_supply {
            let entry = wm_pool.entry(good.clone()).or_default();
            *entry = *entry + sup;
        }
    }

    let wm_pool_init = wm_pool.clone();

    // 3. World Market Rank Order Pass
    for c in countries.iter_mut() {
        for b in &c.buy_orders {
            let Some(&price) = prices.get(&b.good) else { continue };
            let already_bought = res.buyer_bought.get(&b.buyer_id).copied().unwrap_or(Fixed::ZERO);
            let remaining_need = b.amount_needed - already_bought;
            if remaining_need.0 <= 0 { continue; }

            let available = wm_pool.get_mut(&b.good);
            if let Some(sup) = available {
                if sup.0 > 0 {
                    let bought_amount = if remaining_need <= *sup {
                        remaining_need
                    } else {
                        *sup
                    };

                    let cost = bought_amount * price;
                    if c.money >= cost {
                        c.money = c.money - cost;
                        *sup = *sup - bought_amount;
                        let entry = res.buyer_bought.entry(b.buyer_id.clone()).or_default();
                        *entry = *entry + bought_amount;
                    }
                }
            }
        }
    }

    // 4. World Market Revenue Distribution
    for (good, &init_supply) in &wm_pool_init {
        let Some(&final_supply) = wm_pool.get(good) else { continue };
        let sold_amount = init_supply - final_supply;
        if sold_amount.0 <= 0 { continue; }

        let Some(&price) = prices.get(good) else { continue };

        let mut world_leftover = Vec::new();
        let mut total_leftover = Fixed::ZERO;
        for c in countries.iter() {
            for s in &c.sell_orders {
                if s.good == *good {
                    let sold_dom = res.producer_sold.get(&s.producer_id).copied().unwrap_or(Fixed::ZERO);
                    let left = s.amount_produced - sold_dom;
                    if left.0 > 0 {
                        world_leftover.push((s.producer_id.clone(), left));
                        total_leftover = total_leftover + left;
                    }
                }
            }
        }

        if total_leftover.0 == 0 { continue; }

        let revenue = sold_amount * price;
        for (p_id, left) in world_leftover {
            let share = Fixed((left.0 << 15) / total_leftover.0).min(Fixed::ONE);
            let p_revenue = revenue * share;
            let p_sold = sold_amount * share;
            
            let rev_entry = res.producer_revenue.entry(p_id.clone()).or_default();
            *rev_entry = *rev_entry + p_revenue;

            let sold_entry = res.producer_sold.entry(p_id.clone()).or_default();
            *sold_entry = *sold_entry + p_sold;
        }
    }

    res
}

fn distribute_domestic_sale(
    c: &CountryTrade,
    good: &str,
    amount: Fixed,
    price: Fixed,
    res: &mut AllocationResult,
) {
    let mut matching_producers = Vec::new();
    for s in &c.sell_orders {
        if s.good == good && s.amount_produced.0 > 0 {
            matching_producers.push(s.clone());
        }
    }

    if matching_producers.is_empty() { return; }

    let revenue = amount * price;
    let total_produced: Fixed = matching_producers.iter().map(|s| s.amount_produced).fold(Fixed::ZERO, |acc, x| acc + x);

    for p in matching_producers {
        let share = Fixed((p.amount_produced.0 << 15) / total_produced.0).min(Fixed::ONE);
        let p_revenue = revenue * share;
        let p_sold = amount * share;
        
        let rev_entry = res.producer_revenue.entry(p.producer_id.clone()).or_default();
        *rev_entry = *rev_entry + p_revenue;

        let sold_entry = res.producer_sold.entry(p.producer_id.clone()).or_default();
        *sold_entry = *sold_entry + p_sold;
    }
}

