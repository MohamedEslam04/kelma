//! The date and time format the user picked in Settings (plan X1.2), as a shared setting the
//! time helpers read.
//!
//! The Settings page calls [`set_from`] at start and whenever the choice changes; code that
//! writes a time or a date asks [`time_pattern`] or [`date_pattern`] for the `strftime` pattern
//! instead of hard-coding one. `Auto` keeps what kelma showed before there was a choice (24-hour
//! clock, month before day), whatever the interface language.

use crate::general::{DateFormat, GeneralPrefs, TimeFormat};
use std::sync::RwLock;

static CHOICE: RwLock<(DateFormat, TimeFormat)> = RwLock::new((DateFormat::Auto, TimeFormat::Auto));

/// Sets the formats the helpers use.
pub fn set(date: DateFormat, time: TimeFormat) {
    *CHOICE.write().unwrap_or_else(|e| e.into_inner()) = (date, time);
}

/// Sets the formats from the saved general preferences.
pub fn set_from(prefs: &GeneralPrefs) {
    set(prefs.date_format, prefs.time_format);
}

/// The formats in use now.
pub fn current() -> (DateFormat, TimeFormat) {
    *CHOICE.read().unwrap_or_else(|e| e.into_inner())
}

/// The `strftime` pattern for a clock time (hours and minutes) in `format`.
pub fn time_pattern_for(format: TimeFormat) -> &'static str {
    match format {
        TimeFormat::H12 => "%-I:%M %p",
        TimeFormat::Auto | TimeFormat::H24 => "%H:%M",
    }
}

/// The pattern for a clock time in the chosen format.
pub fn time_pattern() -> &'static str {
    time_pattern_for(current().1)
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

    #[test]
    fn auto_keeps_the_24_hour_clock_and_month_first_dates() {
        assert_eq!(time_pattern_for(TimeFormat::Auto), "%H:%M");
        assert_eq!(time_pattern_for(TimeFormat::H24), "%H:%M");
        assert_eq!(date_pattern_for(DateFormat::Auto, false), "%-m/%-d");
        assert_eq!(date_pattern_for(DateFormat::Auto, true), "%-m/%-d/%y");
    }

    #[test]
    fn each_choice_has_its_own_pattern() {
        assert_eq!(time_pattern_for(TimeFormat::H12), "%-I:%M %p");
        assert_eq!(date_pattern_for(DateFormat::Dmy, true), "%-d/%-m/%y");
        assert_eq!(date_pattern_for(DateFormat::Mdy, false), "%-m/%-d");
        assert_eq!(date_pattern_for(DateFormat::Ymd, true), "%y-%m-%d");
        assert_eq!(date_pattern_for(DateFormat::Ymd, false), "%m-%d");
    }

    #[test]
    fn the_shared_setting_follows_the_preferences() {
        let prefs = GeneralPrefs {
            date_format: DateFormat::Dmy,
            time_format: TimeFormat::H12,
            ..GeneralPrefs::default()
        };
        set_from(&prefs);
        assert_eq!(current(), (DateFormat::Dmy, TimeFormat::H12));
        assert_eq!(time_pattern(), "%-I:%M %p");
        assert_eq!(date_pattern(false), "%-d/%-m");
        // Put it back: other tests in this process read the shared setting.
        set_from(&GeneralPrefs::default());
        assert_eq!(time_pattern(), "%H:%M");
    }
}
