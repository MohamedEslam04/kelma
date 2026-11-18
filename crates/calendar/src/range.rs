//! Calendar dates and the time ranges the views show. Everything is computed in UTC; a caller
//! that shows local time shifts the range by its UTC offset.

use std::fmt;

const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weekday {
    Sunday = 0,
    Monday = 1,
    Tuesday = 2,
    Wednesday = 3,
    Thursday = 4,
    Friday = 5,
    Saturday = 6,
}

/// A proleptic Gregorian date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

/// What the calendar shows around a date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Monday to Friday of the week containing the date.
    WorkWeek,
    /// The seven days of the week containing the date.
    Week,
    /// `n` days starting at the date (`Day(1)` is a single day).
    Day(u32),
    /// The weeks that cover the month, as a month grid shows them.
    Month,
}

// Howard Hinnant's civil-date algorithms.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

impl Date {
    pub fn new(year: i32, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }

    fn days(self) -> i64 {
        days_from_civil(self.year.into(), self.month.into(), self.day.into())
    }

    fn from_days(days: i64) -> Date {
        let (y, m, d) = civil_from_days(days);
        Date::new(y as i32, m as u32, d as u32)
    }

    /// The UTC date containing Unix milliseconds `ms`.
    pub fn from_ms(ms: i64) -> Date {
        Date::from_days(ms.div_euclid(DAY_MS))
    }

    /// Unix milliseconds at 00:00 UTC of this date.
    pub fn midnight_ms(self) -> i64 {
        self.days() * DAY_MS
    }

    pub fn add_days(self, n: i64) -> Date {
        Date::from_days(self.days() + n)
    }

    pub fn weekday(self) -> Weekday {
        // 1970-01-01 was a Thursday
        match (self.days() + 4).rem_euclid(7) {
            0 => Weekday::Sunday,
            1 => Weekday::Monday,
            2 => Weekday::Tuesday,
            3 => Weekday::Wednesday,
            4 => Weekday::Thursday,
            5 => Weekday::Friday,
            _ => Weekday::Saturday,
        }
    }

    /// The first of the next month.
    fn next_month_start(self) -> Date {
        if self.month == 12 {
            Date::new(self.year + 1, 1, 1)
        } else {
            Date::new(self.year, self.month + 1, 1)
        }
    }

    /// The first day of the week containing this date.
    fn week_start(self, first_day: Weekday) -> Date {
        let back = (self.weekday() as i64 - first_day as i64).rem_euclid(7);
        self.add_days(-back)
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// Unix milliseconds as `2026-11-18T09:30:00.000Z`, the form Graph takes in query strings.
pub fn format_iso_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let milli = ms.rem_euclid(1000);
    let date = Date::from_ms(ms);
    let t = secs.rem_euclid(86_400);
    format!(
        "{date}T{:02}:{:02}:{:02}.{milli:03}Z",
        t / 3600,
        (t / 60) % 60,
        t % 60
    )
}

/// The half-open range `[start_ms, end_ms)` to fetch for `view` around `date`, with weeks
/// starting on `first_day_of_week`.
pub fn week_range(date: Date, view: View, first_day_of_week: Weekday) -> (i64, i64) {
    let (start, days) = match view {
        View::Day(n) => (date, i64::from(n.max(1))),
        View::Week => (date.week_start(first_day_of_week), 7),
        View::WorkWeek => {
            let ws = date.week_start(first_day_of_week);
            let to_monday = (Weekday::Monday as i64 - first_day_of_week as i64).rem_euclid(7);
            (ws.add_days(to_monday), 5)
        }
        View::Month => {
            let first = Date::new(date.year, date.month, 1);
            let start = first.week_start(first_day_of_week);
            let last = first.next_month_start().add_days(-1);
            let end = last.week_start(first_day_of_week).add_days(7);
            return (start.midnight_ms(), end.midnight_ms());
        }
    };
    (start.midnight_ms(), start.add_days(days).midnight_ms())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOV_18: i64 = 1_794_960_000_000; // 2026-11-18T00:00:00Z, a Wednesday

    fn range(date: Date, view: View, first: Weekday) -> (Date, Date) {
        let (s, e) = week_range(date, view, first);
        (Date::from_ms(s), Date::from_ms(e))
    }

    #[test]
    fn dates_round_trip() {
        let d = Date::new(2026, 11, 18);
        assert_eq!(d.midnight_ms(), NOV_18);
        assert_eq!(Date::from_ms(NOV_18 + 86_399_999), d);
        assert_eq!(d.weekday(), Weekday::Wednesday);
        assert_eq!(d.add_days(14), Date::new(2026, 12, 2));
        assert_eq!(Date::new(2024, 2, 28).add_days(2), Date::new(2024, 3, 1));
        assert_eq!(Date::from_ms(-1), Date::new(1969, 12, 31));
        assert_eq!(d.to_string(), "2026-11-18");
    }

    #[test]
    fn formats_graph_timestamps() {
        assert_eq!(format_iso_ms(NOV_18), "2026-11-18T00:00:00.000Z");
        assert_eq!(
            format_iso_ms(NOV_18 + 9 * 3_600_000 + 30 * 60_000 + 5_007),
            "2026-11-18T09:30:05.007Z"
        );
    }

    #[test]
    fn week_with_monday_start() {
        let (s, e) = range(Date::new(2026, 11, 18), View::Week, Weekday::Monday);
        assert_eq!((s, e), (Date::new(2026, 11, 16), Date::new(2026, 11, 23)));
        assert_eq!(s.weekday(), Weekday::Monday);
    }

    #[test]
    fn week_with_sunday_start() {
        let (s, e) = range(Date::new(2026, 11, 18), View::Week, Weekday::Sunday);
        assert_eq!((s, e), (Date::new(2026, 11, 15), Date::new(2026, 11, 22)));
    }

    #[test]
    fn a_sunday_belongs_to_the_week_before_when_weeks_start_on_monday() {
        let sunday = Date::new(2026, 11, 22);
        assert_eq!(sunday.weekday(), Weekday::Sunday);
        let (s, _) = range(sunday, View::Week, Weekday::Monday);
        assert_eq!(s, Date::new(2026, 11, 16));
        let (s, _) = range(sunday, View::Week, Weekday::Sunday);
        assert_eq!(s, sunday);
    }

    #[test]
    fn work_week_is_monday_to_friday_whatever_the_first_day() {
        for first in [Weekday::Monday, Weekday::Sunday, Weekday::Saturday] {
            let (s, e) = range(Date::new(2026, 11, 18), View::WorkWeek, first);
            assert_eq!(
                (s, e),
                (Date::new(2026, 11, 16), Date::new(2026, 11, 21)),
                "{first:?}"
            );
        }
        let (s, e) = week_range(Date::new(2026, 11, 18), View::WorkWeek, Weekday::Monday);
        assert_eq!(s, NOV_18 - 2 * 86_400_000);
        assert_eq!(e - s, 5 * 86_400_000);
    }

    #[test]
    fn day_and_n_days() {
        let (s, e) = range(Date::new(2026, 11, 18), View::Day(1), Weekday::Monday);
        assert_eq!((s, e), (Date::new(2026, 11, 18), Date::new(2026, 11, 19)));
        let (s, e) = range(Date::new(2026, 11, 29), View::Day(4), Weekday::Monday);
        assert_eq!((s, e), (Date::new(2026, 11, 29), Date::new(2026, 12, 3)));
    }

    #[test]
    fn month_grid_covers_whole_weeks() {
        // November 2026 starts on a Sunday and ends on a Monday
        let nov = Date::new(2026, 11, 18);
        let (s, e) = range(nov, View::Month, Weekday::Monday);
        assert_eq!((s, e), (Date::new(2026, 10, 26), Date::new(2026, 12, 7)));
        let (s, e) = range(nov, View::Month, Weekday::Sunday);
        assert_eq!((s, e), (Date::new(2026, 11, 1), Date::new(2026, 12, 6)));
    }

    #[test]
    fn month_grid_across_a_year_end() {
        let (s, e) = range(Date::new(2026, 12, 10), View::Month, Weekday::Monday);
        assert_eq!((s, e), (Date::new(2026, 11, 30), Date::new(2027, 1, 4)));
    }
}
