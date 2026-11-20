//! "Reset status after", the status message's "Clear after" and the pause durations: which
//! instant each choice means, in the user's own zone (the one chosen in Settings, or the system
//! one), written the way the presence and notification services take it.

use appearance::formats::Tz;
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};

/// The time "Until tomorrow" stops a pause, on the next day.
pub const TOMORROW_HOUR: u32 = 8;

/// The text the service takes: UTC with milliseconds.
pub fn iso(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// A wall-clock time in `zone` as an instant. Skipped times (daylight saving) move forward.
fn at<Z: TimeZone>(zone: &Z, day: NaiveDate, hour: u32, minute: u32) -> Option<DateTime<Utc>> {
    let wall = day.and_hms_opt(hour, minute, 0)?;
    zone_instant(zone, wall)
}

fn zone_instant<Z: TimeZone>(zone: &Z, wall: NaiveDateTime) -> Option<DateTime<Utc>> {
    match zone.from_local_datetime(&wall) {
        chrono::LocalResult::Single(t) => Some(t.with_timezone(&Utc)),
        chrono::LocalResult::Ambiguous(first, _) => Some(first.with_timezone(&Utc)),
        // An hour that does not exist: the same wall time an hour later does.
        chrono::LocalResult::None => zone
            .from_local_datetime(&(wall + Duration::hours(1)))
            .earliest()
            .map(|t| t.with_timezone(&Utc)),
    }
}

fn expiry_in<Z: TimeZone>(
    zone: &Z,
    choice: &str,
    custom: &str,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, String> {
    let today = now.with_timezone(zone).date_naive();
    let from_now = |d: Duration| Ok(Some(now + d));
    match choice {
        "" | "never" => Ok(None),
        "30m" => from_now(Duration::minutes(30)),
        "1h" => from_now(Duration::hours(1)),
        "2h" => from_now(Duration::hours(2)),
        "4h" => from_now(Duration::hours(4)),
        // The end of today: midnight, when tomorrow begins.
        "today" => Ok(at(zone, today + Duration::days(1), 0, 0)),
        // The end of this week: midnight at the start of next Monday.
        "week" => {
            let to_monday = 7 - i64::from(today.weekday().num_days_from_monday());
            Ok(at(zone, today + Duration::days(to_monday), 0, 0))
        }
        "tomorrow" => Ok(at(zone, today + Duration::days(1), TOMORROW_HOUR, 0)),
        "custom" => {
            let wall = NaiveDateTime::parse_from_str(custom.trim(), "%Y-%m-%d %H:%M")
                .map_err(|_| "Write the time as 2026-12-31 17:30.".to_string())?;
            match zone_instant(zone, wall) {
                Some(t) if t > now => Ok(Some(t)),
                Some(_) => Err("Pick a time in the future.".into()),
                None => Err("That time does not exist in your time zone.".into()),
            }
        }
        other => Err(format!("unknown duration {other}")),
    }
}

/// When `choice` ends, counted from `now` in `zone` (`None`: the system zone). `Ok(None)` means
/// "never". `custom` is `YYYY-MM-DD HH:MM` in that zone and only read for `custom`.
///
/// Choices: `never`, `30m`, `1h`, `2h`, `4h`, `today` (midnight), `week` (midnight before next
/// Monday), `tomorrow` (08:00 the next day), `custom`.
pub fn expiry(
    zone: Option<Tz>,
    choice: &str,
    custom: &str,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, String> {
    match zone {
        Some(z) => expiry_in(&z, choice, custom, now),
        None => expiry_in(&chrono::Local, choice, custom, now),
    }
}

/// [`expiry`] as the service's text, in the zone chosen in Settings.
pub fn expiry_iso(choice: &str, custom: &str) -> Result<Option<String>, String> {
    expiry(appearance::formats::time_zone(), choice, custom, Utc::now()).map(|t| t.map(iso))
}

/// "Until 3:45 PM" under the status in the profile menu: the clock time when the status ends
/// today, else the day as well (`Until 11/21 3:45 PM`). `time_pat` and `date_pat` are the
/// `strftime` patterns of the user's formats.
pub fn until_text(
    until: NaiveDateTime,
    now: NaiveDateTime,
    time_pat: &str,
    date_pat: &str,
) -> String {
    if until.date() == now.date() {
        format!("Until {}", until.format(time_pat))
    } else {
        format!(
            "Until {} {}",
            until.format(date_pat),
            until.format(time_pat)
        )
    }
}

/// [`until_text`] for an instant (ms), in the zone and the formats chosen in Settings.
pub fn until_label(until_ms: i64) -> String {
    use appearance::formats as f;
    match (f::local(until_ms), f::local_now()) {
        (Some(u), Some(n)) => until_text(u, n, f::time_pattern(), f::date_pattern(false)),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn until_shows_the_clock_time_today_and_the_day_after_that() {
        let now = wall("2026-11-20 14:00");
        assert_eq!(
            until_text(wall("2026-11-20 15:45"), now, "%-I:%M %p", "%-m/%-d"),
            "Until 3:45 PM"
        );
        assert_eq!(
            until_text(wall("2026-11-21 00:00"), now, "%H:%M", "%-m/%-d"),
            "Until 11/21 00:00"
        );
    }

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// Friday 2026-11-20 10:00 in Tokyo (UTC+9, no daylight saving).
    fn now() -> DateTime<Utc> {
        utc("2026-11-20T01:00:00Z")
    }

    fn ends(choice: &str) -> Option<String> {
        expiry(Some(Tz::Asia__Tokyo), choice, "", now())
            .unwrap()
            .map(iso)
    }

    #[test]
    fn never_has_no_end() {
        assert_eq!(ends("never"), None);
        assert_eq!(ends(""), None);
    }

    #[test]
    fn minutes_and_hours_count_from_now() {
        assert_eq!(ends("30m").as_deref(), Some("2026-11-20T01:30:00.000Z"));
        assert_eq!(ends("1h").as_deref(), Some("2026-11-20T02:00:00.000Z"));
        assert_eq!(ends("2h").as_deref(), Some("2026-11-20T03:00:00.000Z"));
        assert_eq!(ends("4h").as_deref(), Some("2026-11-20T05:00:00.000Z"));
    }

    #[test]
    fn today_ends_at_midnight_of_the_users_zone_not_utc() {
        // Midnight at the start of Saturday in Tokyo is 15:00 UTC on Friday.
        assert_eq!(ends("today").as_deref(), Some("2026-11-20T15:00:00.000Z"));
    }

    #[test]
    fn this_week_ends_before_next_monday() {
        // Friday: next Monday 00:00 Tokyo is Sunday 15:00 UTC.
        assert_eq!(ends("week").as_deref(), Some("2026-11-22T15:00:00.000Z"));
        // On a Monday the week still has seven days.
        let monday = utc("2026-11-16T01:00:00Z");
        let t = expiry(Some(Tz::Asia__Tokyo), "week", "", monday)
            .unwrap()
            .map(iso);
        assert_eq!(t.as_deref(), Some("2026-11-22T15:00:00.000Z"));
    }

    #[test]
    fn until_tomorrow_is_eight_in_the_morning() {
        assert_eq!(
            ends("tomorrow").as_deref(),
            Some("2026-11-20T23:00:00.000Z"),
            "08:00 Saturday in Tokyo"
        );
    }

    #[test]
    fn a_custom_time_is_read_in_the_zone_and_must_be_ahead() {
        let t = |text: &str| expiry(Some(Tz::Asia__Tokyo), "custom", text, now());
        assert_eq!(
            t("2026-12-31 17:30").unwrap().map(iso).as_deref(),
            Some("2026-12-31T08:30:00.000Z")
        );
        assert_eq!(
            t(" 2026-11-21 00:00 ").unwrap().map(iso).as_deref(),
            Some("2026-11-20T15:00:00.000Z")
        );
        assert!(t("2026-11-20 09:00").unwrap_err().contains("future"));
        assert!(t("tomorrow noon").unwrap_err().contains("2026-12-31"));
        assert!(t("").is_err());
    }

    #[test]
    fn an_unknown_choice_is_an_error() {
        assert!(expiry(Some(Tz::UTC), "forever", "", now()).is_err());
    }

    #[test]
    fn the_zone_decides_which_day_it_is() {
        // 23:30 UTC on Friday is already Saturday in Tokyo: "today" ends a day later there.
        let late = utc("2026-11-20T23:30:00Z");
        let tokyo = expiry(Some(Tz::Asia__Tokyo), "today", "", late)
            .unwrap()
            .map(iso);
        let utc_end = expiry(Some(Tz::UTC), "today", "", late).unwrap().map(iso);
        assert_eq!(tokyo.as_deref(), Some("2026-11-21T15:00:00.000Z"));
        assert_eq!(utc_end.as_deref(), Some("2026-11-21T00:00:00.000Z"));
    }
}
