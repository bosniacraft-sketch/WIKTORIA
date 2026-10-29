//! wiktoria_simulation: the port's simulation.
//!
//! Every engine constant is read by stable ID (`FLD_*`) from `data/engine_rules.json`; game content
//! (goods, provinces, pops, events...) comes from the game data folder.
//! No window or graphics code. Deterministic: no clocks, no unseeded randomness.

pub mod ai;
pub mod buildings;
pub mod calendar;
pub mod colonies;
pub mod cond;
pub mod consumption;
pub mod diplomacy;
pub mod events;
pub mod economy;
pub mod fixed;
pub mod game;
pub mod industry;
pub mod market;
pub mod market_day;
pub mod military;
pub mod modifiers;
pub mod player;
pub mod pops;
pub mod politics;
pub mod production;
pub mod prof;
pub mod rank;
pub mod reforms;
pub mod research;
pub mod scenario;
pub mod schedule;
pub mod script;
pub mod trade;
pub mod unrest;

pub use calendar::{Calendar, GameDate};
pub use consumption::{calculate_pop_needs, simulate_buy_needs, PopNeeds};
pub use fixed::Fixed;
pub use market::{GoodMarket, PriceRules};
pub use schedule::PopSchedule;
pub use trade::{allocate_market, BuyOrder, SellOrder, CountryTrade, AllocationResult};

use wiktoria_data::engine_rules::EngineRules;

pub(crate) fn int_field(m: &EngineRules, id: &str) -> Result<i64, String> {
    let v = m.field(id).and_then(|v| v.as_f64()).ok_or(format!("rules field {id} missing or not a number"))?;
    if v.fract() != 0.0 {
        return Err(format!("rules field {id} must be an integer, got {v}"));
    }
    Ok(v as i64)
}

pub fn num_field(m: &EngineRules, id: &str) -> Result<f64, String> {
    m.field(id).and_then(|v| v.as_f64()).ok_or(format!("rules field {id} missing or not a number"))
}
