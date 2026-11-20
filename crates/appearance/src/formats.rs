//! The date and time format and the time zone the user picked in Settings (plan X1.2), as a
//! shared setting the time helpers read.
//!
//! The Settings page calls [`set_from`] at start and whenever the choice changes; code that
//! writes a date asks [`date_pattern`] for the `strftime` pattern, and a clock time asks [`clock`]
//! instead of hard-coding one: 12-hour ("4:14 PM") by default, 24-hour ("16:14") when Settings says so.
//! `Auto` (an older config) reads as 12-hour; its dates keep the month before the day.
//!
//! The time zone is an IANA name; none (or a name that is not one) follows the system. [`local`]
//! turns epoch milliseconds into the wall-clock time in the chosen zone, so every label reads the
//! same zone.

use crate::general::{DateFormat, GeneralPrefs, TimeFormat};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Timelike, Utc};
pub use chrono_tz::Tz;
use std::sync::RwLock;

static CHOICE: RwLock<(DateFormat, TimeFormat)> = RwLock::new((DateFormat::Auto, TimeFormat::H12));
static ZONE: RwLock<Option<Tz>> = RwLock::new(None);

/// Sets the formats the helpers use.
pub fn set(date: DateFormat, time: TimeFormat) {
    *CHOICE.write().unwrap_or_else(|e| e.into_inner()) = (date, time);
}

/// Sets the time zone from an IANA name; none, or a name that is not one, follows the system.
pub fn set_time_zone(name: Option<&str>) {
    let zone = name.and_then(|n| n.trim().parse::<Tz>().ok());
    *ZONE.write().unwrap_or_else(|e| e.into_inner()) = zone;
}

/// Sets the formats and the time zone from the saved general preferences.
pub fn set_from(prefs: &GeneralPrefs) {
    set(prefs.date_format, prefs.time_format);
    set_time_zone(prefs.time_zone.as_deref());
}

/// The chosen time zone, none when following the system.
pub fn time_zone() -> Option<Tz> {
    *ZONE.read().unwrap_or_else(|e| e.into_inner())
}

/// The wall-clock time at `ms` (Unix milliseconds) in `zone`, or in the system zone for none.
pub fn local_in(zone: Option<Tz>, ms: i64) -> Option<NaiveDateTime> {
    match zone {
        Some(tz) => tz
            .timestamp_millis_opt(ms)
            .single()
            .map(|t| t.naive_local()),
        None => Local
            .timestamp_millis_opt(ms)
            .single()
            .map(|t| t.naive_local()),
    }
}

/// The wall-clock time at `ms` in the chosen time zone.
pub fn local(ms: i64) -> Option<NaiveDateTime> {
    local_in(time_zone(), ms)
}

/// The wall-clock time now in the chosen time zone.
pub fn local_now() -> Option<NaiveDateTime> {
    local(Utc::now().timestamp_millis())
}

/// Local time minus UTC at `ms` in `zone`, in milliseconds.
pub fn offset_ms_in(zone: Option<Tz>, ms: i64) -> i64 {
    match (local_in(zone, ms), DateTime::from_timestamp_millis(ms)) {
        (Some(l), Some(u)) => (l - u.naive_utc()).num_milliseconds(),
        _ => 0,
    }
}

/// Local time minus UTC at `ms` in the chosen time zone, in milliseconds.
pub fn offset_ms(ms: i64) -> i64 {
    offset_ms_in(time_zone(), ms)
}

/// The formats in use now.
pub fn current() -> (DateFormat, TimeFormat) {
    *CHOICE.read().unwrap_or_else(|e| e.into_inner())
}

/// Whether clock times read on the 24-hour clock (`Auto` and 12-hour read on the 12-hour one).
pub fn is_h24() -> bool {
    current().1 == TimeFormat::H24
}

/// "AM" or "PM": ص and م when `arabic`.
pub fn meridiem_for(pm: bool, arabic: bool) -> &'static str {
    match (arabic, pm) {
        (true, false) => "ص",
        (true, true) => "م",
        (false, false) => "AM",
        (false, true) => "PM",
    }
}

/// "AM" or "PM", in the interface language: ص and م in Arabic.
pub fn meridiem(pm: bool) -> &'static str {
    meridiem_for(pm, crate::language::current().rtl)
}

/// A clock time for `minutes` after midnight: "4:14 PM", or "16:14" when `h24`.
pub fn clock_for(minutes: i32, h24: bool, arabic: bool) -> String {
    let m = minutes.rem_euclid(24 * 60);
    let (h, min) = (m / 60, m % 60);
    if h24 {
        return format!("{h:02}:{min:02}");
    }
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    format!("{h12}:{min:02} {}", meridiem_for(h >= 12, arabic))
}

/// A clock time for `minutes` after midnight in the interface language: "4:14 PM", or "16:14"
/// when `h24`.
pub fn clock_in(minutes: i32, h24: bool) -> String {
    clock_for(minutes, h24, crate::language::current().rtl)
}

/// [`clock_in`] in the chosen format.
pub fn clock(minutes: i32) -> String {
    clock_in(minutes, is_h24())
}

/// A whole hour as the time axis writes it: "4 PM", or "16:00" when `h24`; other minutes as
/// [`clock_in`].
pub fn hour_for(minutes: i32, h24: bool, arabic: bool) -> String {
    let m = minutes.rem_euclid(24 * 60);
    if h24 || m % 60 != 0 {
        return clock_for(m, h24, arabic);
    }
    let h = m / 60;
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    format!("{h12} {}", meridiem_for(h >= 12, arabic))
}

/// [`hour_for`] in the interface language.
pub fn hour_in(minutes: i32, h24: bool) -> String {
    hour_for(minutes, h24, crate::language::current().rtl)
}

/// [`hour_in`] in the chosen format.
pub fn hour(minutes: i32) -> String {
    hour_in(minutes, is_h24())
}

/// The clock time of a date and time, in the chosen format.
pub fn time_of(t: &impl Timelike) -> String {
    clock((t.hour() * 60 + t.minute()) as i32)
}

/// The `strftime` pattern for a short numeric date in `format`, with the year or without it.
pub fn date_pattern_for(format: DateFormat, with_year: bool) -> &'static str {
    match (format, with_year) {
        (DateFormat::Auto | DateFormat::Mdy, false) => "%-m/%-d",
        (DateFormat::Auto | DateFormat::Mdy, true) => "%-m/%-d/%y",
        (DateFormat::Dmy, false) => "%-d/%-m",
        (DateFormat::Dmy, true) => "%-d/%-m/%y",
        (DateFormat::Ymd, false) => "%m-%d",
        (DateFormat::Ymd, true) => "%y-%m-%d",
    }
}

/// The pattern for a short numeric date in the chosen format.
pub fn date_pattern(with_year: bool) -> &'static str {
    date_pattern_for(current().0, with_year)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests that write the shared choice or zone run one at a time: one resets what another
    /// just set, and the test harness runs tests on several threads.
    static SHARED: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn shared() -> std::sync::MutexGuard<'static, ()> {
        SHARED.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn auto_keeps_month_first_dates() {
        assert_eq!(date_pattern_for(DateFormat::Auto, false), "%-m/%-d");
        assert_eq!(date_pattern_for(DateFormat::Auto, true), "%-m/%-d/%y");
    }

    #[test]
    fn the_clock_reads_in_both_modes() {
        let both = |m: i32| (clock_for(m, false, false), clock_for(m, true, false));
        let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(both(5), pair("12:05 AM", "00:05"));
        assert_eq!(both(12 * 60), pair("12:00 PM", "12:00"));
        assert_eq!(both(13 * 60 + 30), pair("1:30 PM", "13:30"));
        assert_eq!(both(16 * 60 + 14), pair("4:14 PM", "16:14"));
        assert_eq!(both(23 * 60 + 59), pair("11:59 PM", "23:59"));
        assert_eq!(both(0), pair("12:00 AM", "00:00"));
        assert_eq!(
            clock_for(24 * 60 + 5, false, false),
            "12:05 AM",
            "wraps past midnight"
        );
    }

    #[test]
    fn arabic_writes_the_meridiem_in_arabic() {
        assert_eq!(clock_for(16 * 60 + 14, false, true), "4:14 م");
        assert_eq!(clock_for(5, false, true), "12:05 ص");
        assert_eq!(clock_for(16 * 60 + 14, true, true), "16:14");
    }

    #[test]
    fn the_time_axis_writes_whole_hours_short() {
        assert_eq!(hour_for(16 * 60, false, false), "4 PM");
        assert_eq!(hour_for(0, false, false), "12 AM");
        assert_eq!(hour_for(12 * 60, false, false), "12 PM");
        assert_eq!(hour_for(16 * 60, true, false), "16:00");
        assert_eq!(hour_for(16 * 60 + 30, false, false), "4:30 PM");
    }

    #[test]
    fn a_date_and_time_reads_as_its_clock() {
        let _one_at_a_time = shared();
        let t = Utc.with_ymd_and_hms(2026, 11, 19, 16, 14, 59).unwrap();
        set(DateFormat::Auto, TimeFormat::H12);
        assert_eq!(time_of(&t), clock_in(16 * 60 + 14, false));
        set(DateFormat::Auto, TimeFormat::H24);
        assert_eq!(time_of(&t), "16:14");
        set(DateFormat::Auto, TimeFormat::Auto);
        assert_eq!(
            time_of(&t),
            clock_in(16 * 60 + 14, false),
            "Auto is 12-hour"
        );
        assert!(!is_h24());
        set_from(&GeneralPrefs::default());
    }

    #[test]
    fn each_choice_has_its_own_pattern() {
        assert_eq!(date_pattern_for(DateFormat::Dmy, true), "%-d/%-m/%y");
        assert_eq!(date_pattern_for(DateFormat::Mdy, false), "%-m/%-d");
        assert_eq!(date_pattern_for(DateFormat::Ymd, true), "%y-%m-%d");
        assert_eq!(date_pattern_for(DateFormat::Ymd, false), "%m-%d");
    }

    #[test]
    fn the_shared_setting_follows_the_preferences() {
        let _one_at_a_time = shared();
        let prefs = GeneralPrefs {
            date_format: DateFormat::Dmy,
            time_format: TimeFormat::H12,
            ..GeneralPrefs::default()
        };
        set_from(&prefs);
        assert_eq!(current(), (DateFormat::Dmy, TimeFormat::H12));
        assert!(!is_h24());
        assert_eq!(date_pattern(false), "%-d/%-m");
        set_from(&GeneralPrefs {
            time_format: TimeFormat::H24,
            ..GeneralPrefs::default()
        });
        assert!(is_h24());
        // Put it back: other tests in this process read the shared setting.
        set_from(&GeneralPrefs::default());
        assert!(!is_h24(), "12-hour is the default");
    }

    fn instant(h: u32) -> i64 {
        Utc.with_ymd_and_hms(2026, 11, 19, h, 0, 0)
            .unwrap()
            .timestamp_millis()
    }

    fn hhmm(zone: Option<Tz>, ms: i64) -> String {
        local_in(zone, ms).unwrap().format("%H:%M").to_string()
    }

    #[test]
    fn the_same_instant_reads_differently_in_each_zone() {
        let at = instant(8);
        assert_eq!(hhmm(Some(Tz::UTC), at), "08:00");
        assert_eq!(hhmm(Some(Tz::Africa__Cairo), at), "10:00");
        assert_eq!(hhmm(Some(Tz::Asia__Kolkata), at), "13:30");
    }

    #[test]
    fn the_offset_follows_daylight_saving() {
        let hour = 3_600_000;
        assert_eq!(offset_ms_in(Some(Tz::UTC), instant(8)), 0);
        assert_eq!(offset_ms_in(Some(Tz::Africa__Cairo), instant(8)), 2 * hour);
        let summer = Utc
            .with_ymd_and_hms(2026, 7, 1, 8, 0, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(offset_ms_in(Some(Tz::Africa__Cairo), summer), 3 * hour);
    }

    #[test]
    fn the_calendar_day_follows_the_zone() {
        // 23:00 UTC on the 19th is already the 20th in Cairo.
        let at = instant(23);
        assert_eq!(
            local_in(Some(Tz::UTC), at).unwrap().date().to_string(),
            "2026-11-19"
        );
        assert_eq!(
            local_in(Some(Tz::Africa__Cairo), at)
                .unwrap()
                .date()
                .to_string(),
            "2026-11-20"
        );
    }

    #[test]
    fn the_chosen_zone_comes_from_the_preferences() {
        let _one_at_a_time = shared();
        let prefs = GeneralPrefs {
            time_zone: Some("Africa/Cairo".into()),
            ..GeneralPrefs::default()
        };
        set_from(&prefs);
        assert_eq!(time_zone(), Some(Tz::Africa__Cairo));
        assert_eq!(hhmm(time_zone(), instant(8)), "10:00");
        assert_eq!(
            local(instant(8)),
            local_in(Some(Tz::Africa__Cairo), instant(8))
        );
        assert_eq!(offset_ms(instant(8)), 2 * 3_600_000);
        // An unknown name, or none, follows the system again.
        set_time_zone(Some("Mars/Olympus"));
        assert_eq!(time_zone(), None);
        set_time_zone(Some(" UTC "));
        assert_eq!(time_zone(), Some(Tz::UTC));
        set_from(&GeneralPrefs::default());
        assert_eq!(time_zone(), None);
    }

    #[test]
    fn following_the_system_matches_the_local_clock() {
        let at = instant(8);
        let want = Local
            .timestamp_millis_opt(at)
            .single()
            .unwrap()
            .naive_local();
        assert_eq!(local_in(None, at), Some(want));
    }
}
