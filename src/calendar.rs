//! The calendar: a month as six weeks of days, and the local datetimes a
//! scenario's window is written in.
//!
//! WHY THE GRID IS ARITHMETIC AND NOT A WIDGET. The mini-calendar beside the
//! composer's date field is a picture of a month, and a month is arithmetic:
//! which day falls on which weekday, and how many weeks it takes to hold the
//! days. That arithmetic is here, with no toolkit in the file, so it can be
//! tested with no display at all and so the console and the tests read the
//! same month. The console draws the grid; this module knows nothing about
//! drawing.
//!
//! WHY SIX WEEKS ALWAYS. A grid sized to its own month is a control that
//! changes height as the operator steps from month to month, and a control
//! that changes height under the pointer is a control nobody trusts. Six
//! weeks hold every month there is — the longest one, begun on a Sunday,
//! reaches the sixth week — so the calendar's box is the same in October and
//! in February, and the operator's eye can stay where it was.
//!
//! WHY THE DATETIMES ARE LOCAL AND ZONE-FREE. A scenario's window is a plan
//! the operator writes down — when this exercise is to be run — and not an
//! instant the world agrees on. So it carries no zone, and it is written
//! `YYYY-MM-DDTHH:MM`: exact enough to be read, short enough to be typed,
//! and without the seconds and the offset that would have two operators
//! write the same plan two different ways. The parser is therefore strict —
//! anything but that exact shape is refused rather than guessed at, because
//! a wrong answer to a date is a silent one.

use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, Timelike};

/// The month names, in the order the calendar counts them.
///
/// Spelled out here rather than asked of `chrono`'s formatter so that the
/// label is the same on every machine: the locale of the console's host is
/// not a fact a scenario's window should depend on.
const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// How many days a month has, as the calendar counts them.
///
/// The leap rule is the ordinary one: every fourth year, except the
/// centuries, except the four-hundredths.
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// A month, as a year and a month number (1..=12).
///
/// The month number is the plain count of months, so a caller may write 13
/// meaning January of the next year and be right. [`Month::new`] normalises
/// it; the fields are public so that a month can be carried as plain data,
/// and a hand-built month is read through the same normalisation as one the
/// constructor made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Month {
    /// The year the month falls in.
    pub year: i32,
    /// The month, 1 for January through 12 for December.
    pub month: u32,
}

impl Month {
    /// A month, with the month number normalised into range.
    ///
    /// Zero is December of the year before, thirteen is January of the year
    /// after, and every other number is carried the same way. The reason is
    /// that "the next month" is arithmetic and should not have to be a
    /// special case at every call site: stepping forward from December must
    /// not be a different piece of code from stepping forward from June.
    pub fn new(year: i32, month: u32) -> Month {
        // The count of months since year zero, which is what makes the
        // normalisation one division rather than a case per month.
        let total = year as i64 * 12 + month as i64 - 1;
        let normalised_year = total.div_euclid(12);
        let normalised_month = total.rem_euclid(12) + 1;
        Month {
            year: normalised_year.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            month: normalised_month as u32,
        }
    }

    /// The month the wall clock is in, locally.
    ///
    /// Local, because the operator's scenario is written against the clock
    /// on the console's own wall, not against the world: the same console
    /// shows the same month to two operators in the same room, and the
    /// difference between two time zones is a difference between two plans
    /// rather than a correction to one.
    pub fn now() -> Month {
        let today = Local::now().date_naive();
        Month {
            year: today.year(),
            month: today.month(),
        }
    }

    /// The month's days, in weeks, Monday first.
    ///
    /// Six weeks always, and each cell carries the day of the month or
    /// nothing at all for a day that belongs to the month before or the
    /// month after. The first cell is the Monday on or before the first of
    /// the month and the last cell is the Sunday on or after its last day, so
    /// the first of the month always has a weekday's worth of room before
    /// it and the grid never needs a seventh week.
    ///
    /// A cell carries the day as a number rather than a whole date because
    /// the console draws a number in a box and needs nothing more; the month
    /// itself is what says which month the number is in.
    pub fn grid(&self) -> Grid {
        let month = Month::new(self.year, self.month);
        let mut weeks = Vec::with_capacity(6);
        // A month outside the calendar's own reach has no days to draw, but
        // it still draws a box of the same size — a grid that changed its
        // height for a year nobody can reach would be a worse lie than an
        // empty one.
        let first = match NaiveDate::from_ymd_opt(month.year, month.month, 1) {
            Some(first) => first,
            None => {
                for _ in 0..6 {
                    weeks.push(vec![None; 7]);
                }
                return Grid { weeks };
            }
        };
        // The offset of the first of the month from the Monday on or before
        // it: zero when the month starts on a Monday, six when it starts on
        // a Sunday.
        let offset = first.weekday().num_days_from_monday() as i64;
        let start = first - chrono::Duration::days(offset);
        for week in 0..6i64 {
            let mut row = Vec::with_capacity(7);
            for day in 0..7i64 {
                let cell = start + chrono::Duration::days(week * 7 + day);
                row.push(
                    if cell.year() == month.year && cell.month() == month.month {
                        Some(cell.day())
                    } else {
                        None
                    },
                );
            }
            weeks.push(row);
        }
        Grid { weeks }
    }

    /// The month after this one.
    ///
    /// December is followed by January of the next year, because that is
    /// what December is followed by.
    pub fn next(&self) -> Month {
        Month::new(self.year, self.month.saturating_add(1))
    }

    /// The month before this one.
    ///
    /// January is preceded by December of the year before, because that is
    /// what January is preceded by.
    pub fn prev(&self) -> Month {
        Month::new(self.year, self.month.saturating_sub(1))
    }

    /// The month as the console names it: "October 2026".
    ///
    /// The name and the year, and nothing else — no weekday, no "today", no
    /// punctuation the operator would have to read past. The name is the
    /// ordinary English one, whatever the host's locale says, because the
    /// month a plan is named for should not change with the machine it was
    /// typed on.
    pub fn label(&self) -> String {
        let month = Month::new(self.year, self.month);
        match MONTH_NAMES.get((month.month - 1) as usize) {
            Some(name) => format!("{name} {}", month.year),
            None => format!("{} {}", month.month, month.year),
        }
    }
}

/// A month's days, in weeks. Six weeks always, so the calendar's height does
/// not change from month to month — a mini-calendar that grows and shrinks
/// under the pointer is a calendar nobody trusts.
///
/// The weeks run Monday first, because a week that starts on Monday is the
/// week the whole console counts in, and a grid that started on Sunday would
/// have the month's last days and the next month's first days in the same
/// row as Sunday.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    /// Six weeks, each of seven days, each day the number of the day or
    /// nothing for a day outside the month.
    pub weeks: Vec<Vec<Option<u32>>>,
}

/// Parse a local datetime. `None` for anything else, including a zone.
///
/// The shape is `YYYY-MM-DDTHH:MM` and only that shape: no seconds, no
/// fractional part, no zone, and no space where the `T` belongs. The
/// refusals are the point. A date field that accepts what it does not
/// understand turns a typo into a wrong plan that nobody sees for a week,
/// so this parser says no to everything it did not write itself.
pub fn parse(s: &str) -> Option<NaiveDateTime> {
    let bytes = s.as_bytes();
    if bytes.len() != 16 {
        return None;
    }
    if bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' || bytes[13] != b':' {
        return None;
    }
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 4 | 7 | 10 | 13) {
            continue;
        }
        if !byte.is_ascii_digit() {
            return None;
        }
    }
    let year: i32 = s[0..4].parse().ok()?;
    let month: u32 = s[5..7].parse().ok()?;
    let day: u32 = s[8..10].parse().ok()?;
    let hour: u32 = s[11..13].parse().ok()?;
    let minute: u32 = s[14..16].parse().ok()?;
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    let time = date.and_hms_opt(hour, minute, 0)?;
    Some(time)
}

/// Write a local datetime: `YYYY-MM-DDTHH:MM`.
///
/// The shape is exactly the one [`parse`] accepts, so a datetime that goes
/// out of this module and comes back is the datetime that was there — which
/// is the whole reason a scenario's window survives a save and a load.
pub fn format(dt: &NaiveDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        dt.year(),
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute()
    )
}

/// The local now.
///
/// Local, because the plan an operator writes down is written against the
/// clock in the room, and a console that offered the world an instant for a
/// scenario window would be offering a different answer to two operators in
/// two rooms.
pub fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

/// Whether an end is after its start. A window with one end is legal (a
/// scenario may have a start and no end); a window whose end is not after
/// its start is not.
///
/// The halves are legal because they are real states: a scenario that starts
/// and does not end yet is an ordinary scenario, and the composer offers the
/// half rather than inventing the other end. The refusal is for the window
/// that runs backwards, and for a pair that cannot be read at all — an end
/// nobody can place is not an end after the start.
///
/// An empty string counts as no end at all, because that is what a field the
/// operator has not touched holds.
pub fn window_ok(start: Option<&str>, end: Option<&str>) -> bool {
    let start = start.filter(|value| !value.trim().is_empty());
    let end = end.filter(|value| !value.trim().is_empty());
    match (start, end) {
        (None, _) | (_, None) => true,
        (Some(start), Some(end)) => match (parse(start), parse(end)) {
            (Some(start), Some(end)) => end > start,
            _ => false,
        },
    }
}

/// The datetime at a given day of a month, keeping the time of day.
///
/// The day is a day OF THE DATETIME'S OWN MONTH, so the 1st of October
/// stays in October however late the month is, and a month that has no 31st
/// has no answer for the 31st — `None` rather than a day borrowed from the
/// month before, which would be a date the operator never asked for.
pub fn at_day(dt: &NaiveDateTime, day: u32) -> Option<NaiveDateTime> {
    NaiveDate::from_ymd_opt(dt.year(), dt.month(), day).map(|date| date.and_time(dt.time()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The month as the calendar counts it, for a test to check the grid
    /// against.
    fn days_in(year: i32, month: u32) -> u32 {
        days_in_month(year, month)
    }

    /// The days a grid holds, in order.
    fn drawn_days(grid: &Grid) -> Vec<u32> {
        grid.weeks.iter().flatten().filter_map(|day| *day).collect()
    }

    /// A month that begins on a Monday needs no padding before it, and a
    /// thirty-day month leaves the last week short: five weeks carry the
    /// days and the sixth is empty. The height is the same either way, which
    /// is the reason the grid is six weeks tall.
    #[test]
    fn a_month_that_starts_on_a_monday_is_padded_after_its_thirty_days() {
        // June 2026: the first of the month is a Monday and the month has
        // thirty days.
        let grid = Month::new(2026, 6).grid();
        assert_eq!(grid.weeks.len(), 6);
        for week in &grid.weeks {
            assert_eq!(week.len(), 7);
        }
        assert_eq!(
            grid.weeks[0],
            vec![
                Some(1),
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                Some(7)
            ]
        );
        assert_eq!(
            grid.weeks[4],
            vec![Some(29), Some(30), None, None, None, None, None]
        );
        assert_eq!(grid.weeks[5], vec![None; 7]);
        assert_eq!(drawn_days(&grid), (1..=30).collect::<Vec<u32>>());
    }

    /// A month that begins on a Sunday puts its first day in the LAST cell
    /// of the first week, because the week counts from Monday and the
    /// Sunday is where the week ends. The six cells before it belong to the
    /// month before and carry nothing: the grid draws this month's days and
    /// no others.
    #[test]
    fn a_month_that_starts_on_a_sunday_puts_the_first_under_its_weekday() {
        // March 2026: the first of the month is a Sunday and the month has
        // thirty-one days.
        let grid = Month::new(2026, 3).grid();
        assert_eq!(grid.weeks.len(), 6);
        assert_eq!(grid.weeks[0][0], None);
        assert_eq!(grid.weeks[0][5], None);
        assert_eq!(grid.weeks[0][6], Some(1));
        assert_eq!(
            grid.weeks[1],
            vec![
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                Some(7),
                Some(8)
            ]
        );
        assert_eq!(
            grid.weeks[4],
            vec![
                Some(23),
                Some(24),
                Some(25),
                Some(26),
                Some(27),
                Some(28),
                Some(29)
            ]
        );
        assert_eq!(
            grid.weeks[5],
            vec![Some(30), Some(31), None, None, None, None, None]
        );
        assert_eq!(drawn_days(&grid), (1..=31).collect::<Vec<u32>>());
    }

    /// February is the month that decides whether the calendar knows the
    /// leap rule, and it is worth both answers: twenty-nine days in a leap
    /// year and twenty-eight in a common one, and the grid says which.
    #[test]
    fn february_knows_whether_it_is_a_leap_year() {
        let leap = Month::new(2024, 2).grid();
        assert_eq!(drawn_days(&leap), (1..=29).collect::<Vec<u32>>());
        // The first of February 2024 is a Thursday, so the month's days
        // start in the fourth cell of the first week.
        assert_eq!(leap.weeks[0][0], None);
        assert_eq!(leap.weeks[0][3], Some(1));
        assert_eq!(leap.weeks[4][3], Some(29));
        assert_eq!(leap.weeks[5], vec![None; 7]);

        let common = Month::new(2026, 2).grid();
        assert_eq!(drawn_days(&common), (1..=28).collect::<Vec<u32>>());
        // The first of February 2026 is a Sunday, so the month's days start
        // in the last cell of the first week.
        assert_eq!(common.weeks[0][0], None);
        assert_eq!(common.weeks[0][6], Some(1));
        assert_eq!(common.weeks[4][5], Some(28));
        assert_eq!(common.weeks[5], vec![None; 7]);
    }

    /// The six-week rule holds for every month of two years, which is the
    /// whole promise of the grid: the calendar's box is the same in every
    /// month, and the days it draws are that month's days and no others.
    #[test]
    fn every_month_of_two_years_is_six_weeks_of_seven_days() {
        for year in [2026, 2027] {
            for month in 1..=12 {
                let grid = Month::new(year, month).grid();
                assert_eq!(grid.weeks.len(), 6, "{year}-{month} is not six weeks");
                for week in &grid.weeks {
                    assert_eq!(week.len(), 7, "{year}-{month} has a short week");
                }
                let expected: Vec<u32> = (1..=days_in(year, month)).collect();
                assert_eq!(
                    drawn_days(&grid),
                    expected,
                    "{year}-{month} drew the wrong days"
                );
            }
        }
    }

    /// The month number is normalised in both directions, so that "the next
    /// month" is one piece of arithmetic rather than a special case for
    /// December.
    #[test]
    fn a_month_outside_the_range_is_normalised_both_ways() {
        assert_eq!(
            Month::new(2026, 0),
            Month {
                year: 2025,
                month: 12
            }
        );
        assert_eq!(
            Month::new(2026, 13),
            Month {
                year: 2027,
                month: 1
            }
        );
        assert_eq!(
            Month::new(2026, 25),
            Month {
                year: 2028,
                month: 1
            }
        );
        assert_eq!(
            Month::new(2026, 12).next(),
            Month {
                year: 2027,
                month: 1
            }
        );
        assert_eq!(
            Month::new(2026, 1).prev(),
            Month {
                year: 2025,
                month: 12
            }
        );
    }

    /// The label is the month and the year, in the ordinary English, and
    /// nothing else.
    #[test]
    fn a_month_says_its_name_and_its_year() {
        assert_eq!(Month::new(2026, 10).label(), "October 2026");
        assert_eq!(Month::new(2026, 1).label(), "January 2026");
        assert_eq!(Month::new(2026, 12).label(), "December 2026");
    }

    /// A datetime survives the round trip, including a leap day — the one
    /// date a naive formatter gets wrong by handing back the 28th.
    #[test]
    fn a_leap_day_survives_the_round_trip() {
        let dt = parse("2024-02-29T13:45").expect("a leap day is a day");
        assert_eq!(format(&dt), "2024-02-29T13:45");
        assert_eq!(dt.day(), 29);
        assert_eq!(dt.month(), 2);
    }

    /// The parser refuses everything that is not the shape it writes: a
    /// zone, a space, a date with no time, and a string that is not a date
    /// at all. A field that took any of these would hand the operator a
    /// plan nobody had checked.
    #[test]
    fn the_parser_refuses_anything_but_the_written_shape() {
        assert_eq!(parse("2026-10-22T08:00Z"), None);
        assert_eq!(parse("2026-10-22 08:00"), None);
        assert_eq!(parse("2026-10-22"), None);
        assert_eq!(parse("not a date"), None);
        // And a few more of the same kind, for the same reason.
        assert_eq!(parse("2026-10-22T08:00:00"), None);
        assert_eq!(parse("2026-10-22T08:00.000"), None);
        assert_eq!(parse("2026-10-22T08"), None);
        assert_eq!(parse("2026-10-22T08:00 "), None);
        assert_eq!(parse("2026-13-22T08:00"), None);
        assert_eq!(parse("2026-10-32T08:00"), None);
        assert_eq!(parse("2026-10-22T25:00"), None);
    }

    /// A window is legal when its end is after its start, and legal when it
    /// has only one end — a scenario may start and not finish yet. It is
    /// NOT legal when it runs backwards, and not legal when the two ends
    /// cannot be read.
    #[test]
    fn a_window_is_legal_unless_its_end_is_not_after_its_start() {
        assert!(window_ok(
            Some("2026-10-22T08:00"),
            Some("2026-10-22T16:00")
        ));
        assert!(window_ok(Some("2026-10-22T08:00"), None));
        assert!(window_ok(None, Some("2026-10-22T16:00")));
        assert!(window_ok(None, None));
        // An untouched field holds an empty string, and an empty string is
        // no end at all.
        assert!(window_ok(Some(""), None));
        assert!(window_ok(Some("2026-10-22T08:00"), Some("")));
        // The end must be strictly after the start: the same minute is not
        // a window that ends.
        assert!(!window_ok(
            Some("2026-10-22T08:00"),
            Some("2026-10-22T08:00")
        ));
        assert!(!window_ok(
            Some("2026-10-22T16:00"),
            Some("2026-10-22T08:00")
        ));
        assert!(!window_ok(Some("2026-10-22T08:00"), Some("not a date")));
    }

    /// A day is taken in the datetime's OWN month: the 15th is the 15th
    /// whatever the date was, a month with thirty days has no 31st, and the
    /// day chosen never crosses out of the month the datetime is in, even
    /// when the datetime sits on the month's last day.
    #[test]
    fn a_day_is_taken_in_its_own_month() {
        // September has thirty days, so it has no 31st.
        let dt = parse("2026-09-22T08:00").expect("a datetime");
        assert_eq!(
            format(&at_day(&dt, 15).expect("the 15th")),
            "2026-09-15T08:00"
        );
        assert_eq!(at_day(&dt, 31), None);
        // And the 1st of September is still in September.
        assert_eq!(
            format(&at_day(&dt, 1).expect("the 1st")),
            "2026-09-01T08:00"
        );
        // A month with thirty-one days does have one.
        let october = parse("2026-10-22T08:00").expect("a datetime");
        assert_eq!(
            format(&at_day(&october, 31).expect("the 31st")),
            "2026-10-31T08:00"
        );
        // The month boundary is not crossed either way: a datetime on the
        // last day of January still has its 1st in January, and one on the
        // last day of December has its 1st in December.
        let last_of_january = parse("2026-01-31T23:59").expect("a datetime");
        assert_eq!(
            format(&at_day(&last_of_january, 1).expect("the 1st")),
            "2026-01-01T23:59"
        );
        let last_of_december = parse("2026-12-31T23:59").expect("a datetime");
        assert_eq!(
            format(&at_day(&last_of_december, 1).expect("the 1st")),
            "2026-12-01T23:59"
        );
    }
}
