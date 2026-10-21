//! STUB — the real module arrives from the calendar branch. Kept only so the
//! console compiles while that branch is merged; it is replaced wholesale.

use chrono::{Datelike, NaiveDateTime, Timelike};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Month {
    pub year: i32,
    pub month: u32,
}

impl Month {
    pub fn new(year: i32, month: u32) -> Month {
        Month { year, month }
    }
    /// The month a datetime is in.
    pub fn of(dt: &NaiveDateTime) -> Month {
        Month { year: dt.year(), month: dt.month() }
    }
    pub fn now() -> Month {
        Month { year: 2026, month: 10 }
    }
    pub fn grid(&self) -> Grid {
        Grid { weeks: Vec::new() }
    }
    pub fn next(&self) -> Month {
        *self
    }
    pub fn prev(&self) -> Month {
        *self
    }
    pub fn label(&self) -> String {
        format!("{} {}", self.month, self.year)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    pub weeks: Vec<Vec<Option<u32>>>,
}

pub fn parse(_s: &str) -> Option<NaiveDateTime> {
    None
}

pub fn format(_dt: &NaiveDateTime) -> String {
    String::new()
}

pub fn now() -> NaiveDateTime {
    NaiveDateTime::UNIX_EPOCH
}

pub fn window_ok(_start: Option<&str>, _end: Option<&str>) -> bool {
    true
}

pub fn at_day(_dt: &NaiveDateTime, _day: u32) -> Option<NaiveDateTime> {
    None
}

pub fn hhmm_of(_s: &str) -> Option<String> {
    None
}

/// The day of the month a datetime is on.
pub fn day_of(dt: &NaiveDateTime) -> u32 {
    dt.day()
}

/// The same datetime at another hour.
pub fn at_hour(dt: &NaiveDateTime, hour: u32) -> NaiveDateTime {
    dt.with_hour(hour).unwrap_or(*dt)
}
