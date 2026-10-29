//! The game calendar: dates are hours since an epoch (FLD_DATE_EPOCH_HOURS), one tick is one day,
//! and every year has the same months (FLD_MONTH_DAYS, no leap years).

use wiktoria_data::date::Date;
use wiktoria_data::engine_rules::EngineRules;

/// A date as the game stores it: hours since the epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GameDate(pub i64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calendar {
    epoch_hours: i64,
    month_days: Vec<u32>,
    year_days: u32,
}

impl Calendar {
    pub fn new(epoch_hours: i64, month_days: Vec<u32>) -> Result<Calendar, String> {
        if month_days.is_empty() || month_days.contains(&0) {
            return Err("calendar needs at least one month, none of length 0".into());
        }
        let year_days = month_days.iter().sum();
        Ok(Calendar { epoch_hours, month_days, year_days })
    }

    pub fn from_engine_rules(m: &EngineRules) -> Result<Calendar, String> {
        let epoch = crate::int_field(m, "FLD_DATE_EPOCH_HOURS")?;
        let months = m
            .require_str("FLD_MONTH_DAYS")?
            .split(',')
            .map(|s| s.trim().parse::<u32>().map_err(|e| format!("FLD_MONTH_DAYS: '{s}': {e}")))
            .collect::<Result<Vec<_>, _>>()?;
        Calendar::new(epoch, months)
    }

    pub fn days_per_year(&self) -> u32 {
        self.year_days
    }

    /// Hours for a calendar date; `None` for a month or day outside the calendar.
    pub fn to_game(&self, d: Date) -> Option<GameDate> {
        let m = usize::from(d.month).checked_sub(1)?;
        let len = *self.month_days.get(m)?;
        if d.day == 0 || u32::from(d.day) > len {
            return None;
        }
        let doy: u32 = self.month_days[..m].iter().sum::<u32>() + u32::from(d.day) - 1;
        Some(GameDate(self.epoch_hours + (i64::from(d.year) * i64::from(self.year_days) + i64::from(doy)) * 24))
    }

    pub fn to_date(&self, g: GameDate) -> Date {
        let days = (g.0 - self.epoch_hours).div_euclid(24);
        let year = days.div_euclid(i64::from(self.year_days));
        let mut doy = days.rem_euclid(i64::from(self.year_days)) as u32;
        let mut month = 0;
        while doy >= self.month_days[month] {
            doy -= self.month_days[month];
            month += 1;
        }
        Date::new(year as i32, month as u8 + 1, doy as u8 + 1)
    }

    /// Days in a month (1-12); 0 for a month outside the calendar.
    pub fn month_len(&self, month: u8) -> u64 {
        usize::from(month).checked_sub(1).and_then(|m| self.month_days.get(m)).map_or(0, |d| u64::from(*d))
    }

    /// The next day (one tick).
    pub fn next_day(&self, g: GameDate) -> GameDate {
        GameDate(g.0 + 24)
    }
}
