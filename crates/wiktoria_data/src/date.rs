//! Calendar dates as written in Victoria 2 scripts: `1836.1.1`.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl Date {
    pub const fn new(year: i32, month: u8, day: u8) -> Self {
        Date { year, month, day }
    }

    /// Parses `Y.M.D`. Returns None for anything else, so callers can tell dated
    /// history blocks apart from ordinary keys.
    pub fn parse(s: &str) -> Option<Date> {
        let mut it = s.trim().split('.');
        let year = it.next()?.parse().ok()?;
        let month: u8 = it.next()?.parse().ok()?;
        let day: u8 = it.next()?.parse().ok()?;
        if it.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        Some(Date { year, month, day })
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.year, self.month, self.day)
    }
}
