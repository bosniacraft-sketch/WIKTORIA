//! When each pop gets its periodic update (ideology, issues, literacy, militancy, growth):
//! on the day where (day of month - pop id) mod (days in this month) == FLD_POP_UPDATE_OFFSET,
//! so every pop is updated exactly once a month (the month-length part measured
//! on daily saves across 1836.1.31 - 1836.2.3). FLD_POP_UPDATE_PERIOD is the
//! period when the month length is not given.

use wiktoria_data::engine_rules::EngineRules;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopSchedule {
    pub period: u64,
    pub offset: u64,
}

impl PopSchedule {
    pub fn from_engine_rules(m: &EngineRules) -> Result<PopSchedule, String> {
        let period = crate::int_field(m, "FLD_POP_UPDATE_PERIOD")?;
        let offset = crate::int_field(m, "FLD_POP_UPDATE_OFFSET")?;
        if period <= 0 || offset < 0 || offset >= period {
            return Err(format!("pop schedule needs period > 0 and 0 <= offset < period, got {period}/{offset}"));
        }
        Ok(PopSchedule { period: period as u64, offset: offset as u64 })
    }

    pub fn is_due(&self, pop_id: u64, day_of_month: u8) -> bool {
        self.is_due_in(pop_id, day_of_month, self.period)
    }

    /// Whether the pop is due on this day of a month that is `month_days` long.
    pub fn is_due_in(&self, pop_id: u64, day_of_month: u8, month_days: u64) -> bool {
        month_days > 0 && (u64::from(day_of_month) + month_days - pop_id % month_days) % month_days == self.offset % month_days
    }
}
