//! World market prices.
//!
//! Once a day, for every good, from the supply pool and real demand of the previous day:
//! * goods with the fixed-price flag (FLD_PRICE_FIXED_GOOD_FLAG) cost exactly their base cost;
//! * otherwise, when both pools are positive, target = base x f(real_demand / supply), where f is
//!   the stepped square root FLD_PRICE_SQRT_TABLE;
//! * the price moves one FLD_PRICE_STEP down when the target is below price - step, one step up
//!   when it is above price + step, and stays otherwise;
//! * the result is kept within [FLD_PRICE_MIN_FACTOR, FLD_PRICE_MAX_FACTOR] x base cost.

use wiktoria_data::engine_rules::EngineRules;

use crate::fixed::Fixed;

#[derive(Debug, Clone, PartialEq)]
pub struct PriceRules {
    pub step: Fixed,
    pub min_factor: Fixed,
    pub max_factor: Fixed,
    /// (ratio threshold, factor) in increasing threshold order; at or above the last threshold
    /// the factor is the square root of the ratio.
    pub table: Vec<(Fixed, Fixed)>,
    /// goods.txt flag that fixes a good's price at its base cost.
    pub fixed_good_flag: String,
    pub setup_updates: u32,
}

impl PriceRules {
    pub fn from_engine_rules(m: &EngineRules) -> Result<PriceRules, String> {
        Fixed::check_matrix(m)?;
        // The game holds these constants as single-precision floats.
        let f = |id: &str| crate::num_field(m, id).map(|x| Fixed::from_f32(x as f32));
        let mut table = Vec::new();
        for pair in m.require_str("FLD_PRICE_SQRT_TABLE")?.split(',') {
            let (t, v) = pair.split_once(':').ok_or(format!("FLD_PRICE_SQRT_TABLE: '{pair}' is not threshold:factor"))?;
            let num = |s: &str| s.trim().parse::<f32>().map_err(|e| format!("FLD_PRICE_SQRT_TABLE: '{s}': {e}"));
            table.push((Fixed::from_f32(num(t)?), Fixed::from_f32(num(v)?)));
        }
        if table.is_empty() || table.windows(2).any(|w| w[0].0 >= w[1].0) {
            return Err("FLD_PRICE_SQRT_TABLE must list increasing thresholds".into());
        }
        let setup = crate::int_field(m, "FLD_PRICE_SETUP_UPDATES")?;
        Ok(PriceRules {
            step: f("FLD_PRICE_STEP")?,
            min_factor: f("FLD_PRICE_MIN_FACTOR")?,
            max_factor: f("FLD_PRICE_MAX_FACTOR")?,
            table,
            fixed_good_flag: m.require_str("FLD_PRICE_FIXED_GOOD_FLAG")?.to_string(),
            setup_updates: u32::try_from(setup).map_err(|_| "FLD_PRICE_SETUP_UPDATES must be >= 0".to_string())?,
        })
    }

    /// f(ratio): the stepped square root.
    pub fn factor(&self, ratio: Fixed) -> Fixed {
        for &(threshold, value) in &self.table {
            if ratio < threshold {
                return value;
            }
        }
        ratio.sqrt()
    }
}

/// One good on the world market.
#[derive(Debug, Clone, PartialEq)]
pub struct GoodMarket {
    pub base: Fixed,
    pub price: Fixed,
    /// Price always equals the base cost.
    pub fixed_price: bool,
    /// Previous day's supply pool and real demand.
    pub supply: Fixed,
    pub real_demand: Fixed,
    /// The step taken in the last update.
    pub price_change: Fixed,
}

impl GoodMarket {
    pub fn new(base: Fixed, fixed_price: bool) -> GoodMarket {
        GoodMarket { base, price: base, fixed_price, supply: Fixed::ZERO, real_demand: Fixed::ZERO, price_change: Fixed::ZERO }
    }

    /// The daily price update.
    pub fn update_price(&mut self, rules: &PriceRules) {
        if self.fixed_price {
            self.price = self.base;
            return;
        }
        if self.real_demand <= Fixed::ZERO || self.supply <= Fixed::ZERO {
            return;
        }
        let Some(ratio) = self.real_demand.checked_div(self.supply) else { return };
        let target = self.base * rules.factor(ratio);
        let delta = if target < self.price - rules.step {
            -rules.step
        } else if target > self.price + rules.step {
            rules.step
        } else {
            Fixed::ZERO
        };
        let lo = self.base * rules.min_factor;
        let hi = self.base * rules.max_factor;
        let new = self.price + delta;
        self.price = if new < lo { lo } else if new > hi { hi } else { new };
        self.price_change = delta;
    }
}
