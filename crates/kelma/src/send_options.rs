//! The send button's options (`ui/composer_send.slint`): the day and time words of the "Schedule
//! message" picker, the chosen time as milliseconds, and the "Sending on …" label of a scheduled
//! message. Local time, like Teams.

use crate::{AppWindow, SendStore};
use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone, Timelike};
use slint::{ComponentHandle, SharedString};

/// Days the picker goes ahead (Teams' date picker allows a few weeks).
pub const MAX_DAYS: i32 = 60;

fn local(ms: i64) -> Option<DateTime<Local>> {
    Local.timestamp_millis_opt(ms).single()
}

/// "8:00 AM" for `minutes` after midnight.
pub fn time_label(minutes: i32) -> String {
    let m = minutes.clamp(0, 24 * 60 - 1);
    let (h, min) = (m / 60, m % 60);
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    format!("{h12}:{min:02} {}", if h < 12 { "AM" } else { "PM" })
}

/// "Fri, Nov 21" for the day `offset` days after `now`'s day.
pub fn day_label(offset: i32, now: DateTime<Local>) -> String {
    let d = now.date_naive() + Duration::days(i64::from(offset));
    d.format("%a, %b %-d").to_string()
}

/// Milliseconds of `minutes` after midnight on the day `offset` days after `now`'s day, or
/// `None` when that is not in the future.
pub fn resolve(offset: i32, minutes: i32, now: DateTime<Local>) -> Option<i64> {
    let day = now.date_naive() + Duration::days(i64::from(offset));
    let m = u32::try_from(minutes.clamp(0, 24 * 60 - 1)).ok()?;
    let at = day.and_time(NaiveTime::from_hms_opt(m / 60, m % 60, 0)?);
    let at = Local.from_local_datetime(&at).earliest()?;
    (at > now).then(|| at.timestamp_millis())
}

/// "Fri, Nov 21 at 8:00 AM" for a time in milliseconds.
pub fn when_label(ms: i64) -> String {
    let Some(t) = local(ms) else {
        return String::new();
    };
    let minutes = (t.hour() * 60 + t.minute()) as i32;
    let year = if t.year() == Local::now().year() {
        String::new()
    } else {
        format!(" {}", t.year())
    };
    format!(
        "{}{year} at {}",
        t.format("%a, %b %-d"),
        time_label(minutes)
    )
}

/// The text of a scheduled message's bubble.
pub fn sending_label(ms: i64) -> String {
    format!("Sending on {}", when_label(ms))
}

/// Connects the picker's words and time to the window.
pub fn wire(window: &AppWindow) {
    let s = window.global::<SendStore>();
    s.on_day_label(|offset| day_label(offset.clamp(0, MAX_DAYS), Local::now()).into());
    s.on_time_label(|minutes| time_label(minutes).into());
    s.on_resolve(|offset, minutes| {
        resolve(offset.clamp(0, MAX_DAYS), minutes, Local::now())
            .map(|ms| SharedString::from(ms.to_string()))
            .unwrap_or_default()
    });
    s.on_when_label(|ms: SharedString| ms.parse().map(when_label).unwrap_or_default().into());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn times_read_like_teams() {
        assert_eq!(time_label(0), "12:00 AM");
        assert_eq!(time_label(8 * 60), "8:00 AM");
        assert_eq!(time_label(12 * 60 + 30), "12:30 PM");
        assert_eq!(time_label(23 * 60 + 45), "11:45 PM");
    }

    #[test]
    fn the_day_counts_from_today() {
        let now = at(2026, 11, 20, 15, 0);
        assert_eq!(day_label(0, now), "Fri, Nov 20");
        assert_eq!(day_label(1, now), "Sat, Nov 21");
        assert_eq!(day_label(12, now), "Wed, Dec 2");
    }

    #[test]
    fn a_time_in_the_past_resolves_to_nothing() {
        let now = at(2026, 11, 20, 15, 0);
        assert_eq!(resolve(0, 9 * 60, now), None);
        assert_eq!(resolve(0, 15 * 60, now), None, "now is not later");
        let tomorrow_8 = resolve(1, 8 * 60, now).unwrap();
        assert_eq!(tomorrow_8, at(2026, 11, 21, 8, 0).timestamp_millis());
    }

    #[test]
    fn the_bubble_says_when() {
        let ms = at(Local::now().year(), 11, 21, 8, 0).timestamp_millis();
        assert!(sending_label(ms).starts_with("Sending on "));
        assert!(sending_label(ms).ends_with("Nov 21 at 8:00 AM"));
        let other_year = at(2001, 1, 2, 13, 5).timestamp_millis();
        assert_eq!(when_label(other_year), "Tue, Jan 2 2001 at 1:05 PM");
    }
}
