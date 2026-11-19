//! Time zones (C2.3, C3.3): Graph names zones the Windows way (`Egypt Standard Time`), or sends a
//! `customTimeZone` with a bias and transition rules. This maps Windows names to IANA, turns
//! wall-clock times into instants, and gives a zone's UTC offset at an instant (for a second time
//! zone in the grid and for other people's work hours in the Scheduler).

use chrono::{NaiveDateTime, Offset, TimeZone};
use chrono_tz::Tz;
use serde_json::Value;

use crate::range::{Date, Weekday};

const MINUTE_MS: i64 = 60_000;

/// Windows zone name to its IANA zone: the "001" (golden) rows of CLDR's `windowsZones.xml`.
const WINDOWS_ZONES: &[(&str, &str)] = &[
    ("Dateline Standard Time", "Etc/GMT+12"),
    ("UTC-11", "Etc/GMT+11"),
    ("Aleutian Standard Time", "America/Adak"),
    ("Hawaiian Standard Time", "Pacific/Honolulu"),
    ("Marquesas Standard Time", "Pacific/Marquesas"),
    ("Alaskan Standard Time", "America/Anchorage"),
    ("UTC-09", "Etc/GMT+9"),
    ("Pacific Standard Time (Mexico)", "America/Tijuana"),
    ("UTC-08", "Etc/GMT+8"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("US Mountain Standard Time", "America/Phoenix"),
    ("Mountain Standard Time (Mexico)", "America/Mazatlan"),
    ("Mountain Standard Time", "America/Denver"),
    // Exchange-only names that CLDR does not list; Graph's supportedTimeZones offers them.
    ("British Columbia Standard Time", "America/Vancouver"),
    ("Alberta Standard Time", "America/Edmonton"),
    ("Manitoba Standard Time", "America/Winnipeg"),
    ("Yukon Standard Time", "America/Whitehorse"),
    ("Central America Standard Time", "America/Guatemala"),
    ("Central Standard Time", "America/Chicago"),
    ("Easter Island Standard Time", "Pacific/Easter"),
    ("Central Standard Time (Mexico)", "America/Mexico_City"),
    ("Canada Central Standard Time", "America/Regina"),
    ("SA Pacific Standard Time", "America/Bogota"),
    ("Eastern Standard Time (Mexico)", "America/Cancun"),
    ("Eastern Standard Time", "America/New_York"),
    ("Haiti Standard Time", "America/Port-au-Prince"),
    ("Cuba Standard Time", "America/Havana"),
    ("US Eastern Standard Time", "America/Indianapolis"),
    ("Turks And Caicos Standard Time", "America/Grand_Turk"),
    ("Paraguay Standard Time", "America/Asuncion"),
    ("Atlantic Standard Time", "America/Halifax"),
    ("Venezuela Standard Time", "America/Caracas"),
    ("Central Brazilian Standard Time", "America/Cuiaba"),
    ("SA Western Standard Time", "America/La_Paz"),
    ("Pacific SA Standard Time", "America/Santiago"),
    ("Newfoundland Standard Time", "America/St_Johns"),
    ("Tocantins Standard Time", "America/Araguaina"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
    ("SA Eastern Standard Time", "America/Cayenne"),
    ("Argentina Standard Time", "America/Buenos_Aires"),
    ("Greenland Standard Time", "America/Godthab"),
    ("Montevideo Standard Time", "America/Montevideo"),
    ("Magallanes Standard Time", "America/Punta_Arenas"),
    ("Saint Pierre Standard Time", "America/Miquelon"),
    ("Bahia Standard Time", "America/Bahia"),
    ("UTC-02", "Etc/GMT+2"),
    ("Mid-Atlantic Standard Time", "Etc/GMT+2"),
    ("Azores Standard Time", "Atlantic/Azores"),
    ("Cape Verde Standard Time", "Atlantic/Cape_Verde"),
    ("UTC", "Etc/UTC"),
    ("GMT Standard Time", "Europe/London"),
    ("Greenwich Standard Time", "Atlantic/Reykjavik"),
    ("Sao Tome Standard Time", "Africa/Sao_Tome"),
    ("Morocco Standard Time", "Africa/Casablanca"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Romance Standard Time", "Europe/Paris"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("W. Central Africa Standard Time", "Africa/Lagos"),
    ("Jordan Standard Time", "Asia/Amman"),
    ("GTB Standard Time", "Europe/Bucharest"),
    ("Middle East Standard Time", "Asia/Beirut"),
    ("Egypt Standard Time", "Africa/Cairo"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("Syria Standard Time", "Asia/Damascus"),
    ("West Bank Standard Time", "Asia/Hebron"),
    ("South Africa Standard Time", "Africa/Johannesburg"),
    ("FLE Standard Time", "Europe/Kiev"),
    ("Israel Standard Time", "Asia/Jerusalem"),
    ("South Sudan Standard Time", "Africa/Juba"),
    ("Kaliningrad Standard Time", "Europe/Kaliningrad"),
    ("Sudan Standard Time", "Africa/Khartoum"),
    ("Libya Standard Time", "Africa/Tripoli"),
    ("Namibia Standard Time", "Africa/Windhoek"),
    ("Arabic Standard Time", "Asia/Baghdad"),
    ("Turkey Standard Time", "Europe/Istanbul"),
    ("Arab Standard Time", "Asia/Riyadh"),
    ("Belarus Standard Time", "Europe/Minsk"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("E. Africa Standard Time", "Africa/Nairobi"),
    ("Volgograd Standard Time", "Europe/Volgograd"),
    ("Iran Standard Time", "Asia/Tehran"),
    ("Arabian Standard Time", "Asia/Dubai"),
    ("Astrakhan Standard Time", "Europe/Astrakhan"),
    ("Azerbaijan Standard Time", "Asia/Baku"),
    ("Russia Time Zone 3", "Europe/Samara"),
    ("Mauritius Standard Time", "Indian/Mauritius"),
    ("Saratov Standard Time", "Europe/Saratov"),
    ("Georgian Standard Time", "Asia/Tbilisi"),
    ("Caucasus Standard Time", "Asia/Yerevan"),
    ("Afghanistan Standard Time", "Asia/Kabul"),
    ("West Asia Standard Time", "Asia/Tashkent"),
    ("Qyzylorda Standard Time", "Asia/Qyzylorda"),
    ("Ekaterinburg Standard Time", "Asia/Yekaterinburg"),
    ("Pakistan Standard Time", "Asia/Karachi"),
    ("India Standard Time", "Asia/Calcutta"),
    ("Sri Lanka Standard Time", "Asia/Colombo"),
    ("Nepal Standard Time", "Asia/Katmandu"),
    ("Central Asia Standard Time", "Asia/Bishkek"),
    ("Bangladesh Standard Time", "Asia/Dhaka"),
    ("Omsk Standard Time", "Asia/Omsk"),
    ("Myanmar Standard Time", "Asia/Rangoon"),
    ("SE Asia Standard Time", "Asia/Bangkok"),
    ("Altai Standard Time", "Asia/Barnaul"),
    ("W. Mongolia Standard Time", "Asia/Hovd"),
    ("North Asia Standard Time", "Asia/Krasnoyarsk"),
    ("N. Central Asia Standard Time", "Asia/Novosibirsk"),
    ("Tomsk Standard Time", "Asia/Tomsk"),
    ("China Standard Time", "Asia/Shanghai"),
    ("North Asia East Standard Time", "Asia/Irkutsk"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("W. Australia Standard Time", "Australia/Perth"),
    ("Taipei Standard Time", "Asia/Taipei"),
    ("Ulaanbaatar Standard Time", "Asia/Ulaanbaatar"),
    ("Aus Central W. Standard Time", "Australia/Eucla"),
    ("Transbaikal Standard Time", "Asia/Chita"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("North Korea Standard Time", "Asia/Pyongyang"),
    ("Korea Standard Time", "Asia/Seoul"),
    ("Yakutsk Standard Time", "Asia/Yakutsk"),
    ("Cen. Australia Standard Time", "Australia/Adelaide"),
    ("AUS Central Standard Time", "Australia/Darwin"),
    ("E. Australia Standard Time", "Australia/Brisbane"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("West Pacific Standard Time", "Pacific/Port_Moresby"),
    ("Tasmania Standard Time", "Australia/Hobart"),
    ("Vladivostok Standard Time", "Asia/Vladivostok"),
    ("Lord Howe Standard Time", "Australia/Lord_Howe"),
    ("Bougainville Standard Time", "Pacific/Bougainville"),
    ("Russia Time Zone 10", "Asia/Srednekolymsk"),
    ("Magadan Standard Time", "Asia/Magadan"),
    ("Norfolk Standard Time", "Pacific/Norfolk"),
    ("Sakhalin Standard Time", "Asia/Sakhalin"),
    ("Central Pacific Standard Time", "Pacific/Guadalcanal"),
    ("Russia Time Zone 11", "Asia/Kamchatka"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
    ("UTC+12", "Etc/GMT-12"),
    ("Fiji Standard Time", "Pacific/Fiji"),
    ("Kamchatka Standard Time", "Asia/Kamchatka"),
    ("Chatham Islands Standard Time", "Pacific/Chatham"),
    ("UTC+13", "Etc/GMT-13"),
    ("Tonga Standard Time", "Pacific/Tongatapu"),
    ("Samoa Standard Time", "Pacific/Apia"),
    ("Line Islands Standard Time", "Pacific/Kiritimati"),
];

/// The IANA zone for a Windows zone name (`Egypt Standard Time`) or an IANA name
/// (`Africa/Cairo`). Names are matched ignoring case. `None` for names neither list knows,
/// such as `Customized Time Zone`.
pub fn iana_name(zone: &str) -> Option<&'static str> {
    let zone = zone.trim();
    if let Some((_, iana)) = WINDOWS_ZONES
        .iter()
        .find(|(w, _)| w.eq_ignore_ascii_case(zone))
    {
        return Some(iana);
    }
    if let Ok(tz) = zone.parse::<Tz>() {
        return Some(tz.name());
    }
    chrono_tz::TZ_VARIANTS
        .iter()
        .find(|tz| tz.name().eq_ignore_ascii_case(zone))
        .map(|tz| tz.name())
}

/// The Windows zone name for a Windows or IANA zone name (`Africa/Cairo` gives
/// `Egypt Standard Time`), from the same golden rows. `None` when there is none.
pub fn windows_name(zone: &str) -> Option<&'static str> {
    let zone = zone.trim();
    if let Some((w, _)) = WINDOWS_ZONES
        .iter()
        .find(|(w, _)| w.eq_ignore_ascii_case(zone))
    {
        return Some(w);
    }
    let iana = iana_name(zone)?;
    WINDOWS_ZONES
        .iter()
        .find(|(_, i)| *i == iana)
        .map(|(w, _)| *w)
}

fn tz(zone: &str) -> Option<Tz> {
    iana_name(zone).and_then(|n| n.parse().ok())
}

/// A `transition` of a `customTimeZone`: the `day_occurrence`-th `weekday` of `month` (5 means
/// the last), at `time_min` minutes after local midnight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    pub month: u32,
    pub day_occurrence: u32,
    pub weekday: Weekday,
    pub time_min: u32,
}

/// Graph's `customTimeZone`, the Windows `TIME_ZONE_INFORMATION`: local time is UTC minus
/// `bias_min` (minus `daylight_bias_min` more between the daylight and standard transitions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CustomZone {
    pub bias_min: i32,
    pub daylight_bias_min: i32,
    pub standard: Option<Transition>,
    pub daylight: Option<Transition>,
}

fn weekday_named(s: &str) -> Option<Weekday> {
    Some(match s.to_ascii_lowercase().as_str() {
        "sunday" => Weekday::Sunday,
        "monday" => Weekday::Monday,
        "tuesday" => Weekday::Tuesday,
        "wednesday" => Weekday::Wednesday,
        "thursday" => Weekday::Thursday,
        "friday" => Weekday::Friday,
        "saturday" => Weekday::Saturday,
        _ => return None,
    })
}

/// A key looked up ignoring case (Graph `bias`, Outlook REST `Bias`).
fn ci<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.as_object()?
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

fn transition(v: &Value) -> Option<Transition> {
    let month = ci(v, "month")?.as_u64()? as u32;
    if !(1..=12).contains(&month) {
        return None;
    }
    let time = ci(v, "time").and_then(Value::as_str).unwrap_or("00:00");
    let mut parts = time.split(':');
    let h: u32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next().unwrap_or("0").parse().ok()?;
    Some(Transition {
        month,
        day_occurrence: ci(v, "dayOccurrence")?.as_u64()?.clamp(1, 5) as u32,
        weekday: weekday_named(ci(v, "dayOfWeek")?.as_str()?)?,
        time_min: h * 60 + m,
    })
}

/// Reads a `customTimeZone` (it has a `bias`). `None` for a named zone.
pub fn parse_custom_zone(v: &Value) -> Option<CustomZone> {
    let bias_min = ci(v, "bias")?.as_i64()? as i32;
    let daylight = ci(v, "daylightOffset");
    Some(CustomZone {
        bias_min,
        daylight_bias_min: daylight
            .and_then(|d| ci(d, "daylightBias"))
            .and_then(Value::as_i64)
            .unwrap_or(0) as i32,
        standard: ci(v, "standardOffset").and_then(transition),
        daylight: daylight.and_then(transition),
    })
}

fn weekday_index(w: Weekday) -> i64 {
    w as i64
}

/// The local midnight (as if it were UTC) of the transition day in `year`, plus its time.
fn transition_local_ms(t: &Transition, year: i32) -> i64 {
    let first = Date::new(year, t.month, 1);
    let lead = (weekday_index(t.weekday) - weekday_index(first.weekday())).rem_euclid(7);
    let mut day = first.add_days(lead + 7 * (i64::from(t.day_occurrence) - 1));
    while day.month != t.month {
        day = day.add_days(-7);
    }
    day.midnight_ms() + i64::from(t.time_min) * MINUTE_MS
}

impl CustomZone {
    /// Minutes east of UTC at the UTC instant `ms`.
    pub fn offset_min(&self, ms: i64) -> i32 {
        let standard = -self.bias_min;
        let (Some(dst_on), Some(dst_off)) = (self.daylight, self.standard) else {
            return standard;
        };
        if self.daylight_bias_min == 0 {
            return standard;
        }
        let daylight = standard - self.daylight_bias_min;
        // Transition times are local: the switch to daylight in standard time, the switch
        // back in daylight time.
        let year = Date::from_ms(ms + i64::from(standard) * MINUTE_MS).year;
        let on = transition_local_ms(&dst_on, year) - i64::from(standard) * MINUTE_MS;
        let off = transition_local_ms(&dst_off, year) - i64::from(daylight) * MINUTE_MS;
        let in_dst = if on < off {
            ms >= on && ms < off
        } else {
            // Southern hemisphere: daylight time spans the new year.
            ms >= on || ms < off
        };
        if in_dst { daylight } else { standard }
    }
}

/// Minutes east of UTC of `zone` (Windows or IANA name) at the UTC instant `ms`, with daylight
/// saving applied. `None` for an unknown name.
pub fn utc_offset_min(zone: &str, ms: i64) -> Option<i32> {
    let tz = tz(zone)?;
    let at = chrono::DateTime::from_timestamp_millis(ms)?;
    Some(
        tz.offset_from_utc_datetime(&at.naive_utc())
            .fix()
            .local_minus_utc()
            / 60,
    )
}

/// Wall-clock time to Unix ms using a fixed rule for the offset at that wall time: `offset_at`
/// gives the offset at a UTC instant; we look it up at the guess and correct once, which lands
/// on the later offset in a gap and the earlier one in an overlap.
fn wall_to_utc(wall_ms: i64, offset_at: impl Fn(i64) -> i32) -> i64 {
    let first = wall_ms - i64::from(offset_at(wall_ms)) * MINUTE_MS;
    wall_ms - i64::from(offset_at(first)) * MINUTE_MS
}

fn wall_ms(wall: &str) -> Option<i64> {
    let t = wall.trim().trim_end_matches('Z');
    let t = t.split('.').next()?;
    let naive = NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M"))
        .ok()?;
    Some(naive.and_utc().timestamp_millis())
}

/// Unix ms of a Graph `dateTimeTimeZone`: `wall` like `2026-11-18T09:00:00.0000000` read in
/// `zone` (Windows or IANA name, `UTC` too). `None` when either does not parse.
pub fn local_to_utc_ms(wall: &str, zone: &str) -> Option<i64> {
    let w = wall_ms(wall)?;
    let tz = tz(zone)?;
    let naive = chrono::DateTime::from_timestamp_millis(w)?.naive_utc();
    match tz.from_local_datetime(&naive) {
        chrono::LocalResult::Single(t) => Some(t.timestamp_millis()),
        chrono::LocalResult::Ambiguous(a, _) => Some(a.timestamp_millis()),
        // A wall time skipped by a daylight switch: read it with the offset before the gap.
        chrono::LocalResult::None => Some(wall_to_utc(w, |ms| {
            utc_offset_min(zone, ms - 3 * 3_600_000).unwrap_or(0)
        })),
    }
}

/// Reads a Graph `{"dateTime", "timeZone"}` value (keys in any case).
pub fn date_time_zone_ms(v: &Value) -> Option<i64> {
    let wall = ci(v, "dateTime")?.as_str()?;
    let zone = ci(v, "timeZone").and_then(Value::as_str).unwrap_or("UTC");
    local_to_utc_ms(wall, if zone.is_empty() { "UTC" } else { zone })
}

/// A zone as the work-hours code needs it: a named zone, a custom one, or UTC when neither is
/// known.
pub(crate) enum Zone {
    Named(Tz),
    Custom(CustomZone),
    Utc,
}

impl Zone {
    pub(crate) fn new(name: &str, custom: Option<&CustomZone>) -> Zone {
        match (custom, tz(name)) {
            (Some(c), _) => Zone::Custom(*c),
            (None, Some(t)) => Zone::Named(t),
            (None, None) => Zone::Utc,
        }
    }

    pub(crate) fn offset_min(&self, ms: i64) -> i32 {
        match self {
            Zone::Named(t) => chrono::DateTime::from_timestamp_millis(ms)
                .map(|at| {
                    t.offset_from_utc_datetime(&at.naive_utc())
                        .fix()
                        .local_minus_utc()
                        / 60
                })
                .unwrap_or(0),
            Zone::Custom(c) => c.offset_min(ms),
            Zone::Utc => 0,
        }
    }

    /// Unix ms of a wall-clock time given as ms since the epoch read as local.
    pub(crate) fn wall_to_utc(&self, wall_ms: i64) -> i64 {
        wall_to_utc(wall_ms, |ms| self.offset_min(ms))
    }

    /// The local date at the UTC instant `ms`.
    pub(crate) fn local_date(&self, ms: i64) -> Date {
        Date::from_ms(ms + i64::from(self.offset_min(ms)) * MINUTE_MS)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn windows_names() {
        use super::windows_name;
        assert_eq!(windows_name("Africa/Cairo"), Some("Egypt Standard Time"));
        assert_eq!(
            windows_name("egypt standard time"),
            Some("Egypt Standard Time")
        );
        assert_eq!(
            windows_name("America/Los_Angeles"),
            Some("Pacific Standard Time")
        );
        assert_eq!(windows_name("Not/AZone"), None);
    }

    use super::*;
    use serde_json::json;

    fn ms(s: &str) -> i64 {
        teams_proto::time::parse_iso_ms(s).unwrap()
    }

    #[test]
    fn maps_windows_names_to_iana() {
        assert_eq!(iana_name("Egypt Standard Time"), Some("Africa/Cairo"));
        assert_eq!(
            iana_name("pacific standard time"),
            Some("America/Los_Angeles")
        );
        assert_eq!(iana_name("UTC"), Some("Etc/UTC"));
        assert_eq!(iana_name("Europe/Berlin"), Some("Europe/Berlin"));
        assert_eq!(iana_name("europe/berlin"), Some("Europe/Berlin"));
        assert_eq!(iana_name("Alberta Standard Time"), Some("America/Edmonton"));
        assert_eq!(iana_name("Customized Time Zone"), None);
        assert_eq!(iana_name(""), None);
        // Every IANA name in the table is one chrono-tz knows.
        for (w, i) in WINDOWS_ZONES {
            assert!(i.parse::<Tz>().is_ok(), "{w} -> {i}");
        }
    }

    #[test]
    fn offsets_follow_daylight_saving() {
        // New York: EDT (-4) in July, EST (-5) in December.
        let jul = ms("2026-07-01T12:00:00Z");
        let dec = ms("2026-12-01T12:00:00Z");
        assert_eq!(utc_offset_min("Eastern Standard Time", jul), Some(-240));
        assert_eq!(utc_offset_min("America/New_York", dec), Some(-300));
        assert_eq!(utc_offset_min("India Standard Time", dec), Some(330));
        assert_eq!(utc_offset_min("Nepal Standard Time", dec), Some(345));
        assert_eq!(utc_offset_min("UTC", jul), Some(0));
        assert_eq!(utc_offset_min("Nowhere Standard Time", jul), None);
    }

    #[test]
    fn wall_clock_to_instant() {
        assert_eq!(
            local_to_utc_ms("2026-11-18T09:00:00.0000000", "Pacific Standard Time"),
            Some(ms("2026-11-18T17:00:00Z"))
        );
        assert_eq!(
            local_to_utc_ms("2026-07-01T09:00:00", "W. Europe Standard Time"),
            Some(ms("2026-07-01T07:00:00Z"))
        );
        assert_eq!(
            local_to_utc_ms("2026-11-18T09:00:00", "UTC"),
            Some(ms("2026-11-18T09:00:00Z"))
        );
        // 02:30 does not exist in New York on 2026-03-08; read it with EST.
        assert_eq!(
            local_to_utc_ms("2026-03-08T02:30:00", "America/New_York"),
            Some(ms("2026-03-08T07:30:00Z"))
        );
        assert_eq!(local_to_utc_ms("not a time", "UTC"), None);
        assert_eq!(
            date_time_zone_ms(
                &json!({"dateTime": "2026-11-18T09:00:00", "timeZone": "Tokyo Standard Time"})
            ),
            Some(ms("2026-11-18T00:00:00Z"))
        );
        assert_eq!(
            date_time_zone_ms(&json!({"DateTime": "2026-11-18T09:00:00", "TimeZone": "UTC"})),
            Some(ms("2026-11-18T09:00:00Z"))
        );
    }

    fn pacific_custom() -> CustomZone {
        parse_custom_zone(&json!({
            "@odata.type": "#microsoft.graph.customTimeZone",
            "bias": 480, "name": "Customized Time Zone",
            "standardOffset": {"time": "02:00:00.0000000", "dayOccurrence": 1,
                "dayOfWeek": "sunday", "month": 11, "year": 0},
            "daylightOffset": {"daylightBias": -60, "time": "02:00:00.0000000",
                "dayOccurrence": 2, "dayOfWeek": "sunday", "month": 3, "year": 0}}))
        .unwrap()
    }

    #[test]
    fn custom_zone_rules() {
        let c = pacific_custom();
        assert_eq!(c.bias_min, 480);
        assert_eq!(c.standard.unwrap().month, 11);
        assert_eq!(c.daylight.unwrap().day_occurrence, 2);
        assert_eq!(c.offset_min(ms("2026-07-01T12:00:00Z")), -420);
        assert_eq!(c.offset_min(ms("2026-12-01T12:00:00Z")), -480);
        // 2026-03-08 02:00 PST is 10:00 UTC; 2026-11-01 02:00 PDT is 09:00 UTC.
        assert_eq!(c.offset_min(ms("2026-03-08T09:59:00Z")), -480);
        assert_eq!(c.offset_min(ms("2026-03-08T10:00:00Z")), -420);
        assert_eq!(c.offset_min(ms("2026-11-01T08:59:00Z")), -420);
        assert_eq!(c.offset_min(ms("2026-11-01T09:00:00Z")), -480);
        // A named zone has no bias.
        assert!(parse_custom_zone(&json!({"name": "UTC"})).is_none());
        // No transitions: a fixed offset.
        let fixed = parse_custom_zone(&json!({"bias": -120})).unwrap();
        assert_eq!(fixed.offset_min(ms("2026-07-01T00:00:00Z")), 120);
    }

    #[test]
    fn southern_custom_zone() {
        // Sydney: +10, +11 from the first Sunday of October to the first Sunday of April.
        let c = parse_custom_zone(&json!({"bias": -600,
            "standardOffset": {"time": "03:00:00", "dayOccurrence": 1, "dayOfWeek": "sunday", "month": 4},
            "daylightOffset": {"daylightBias": -60, "time": "02:00:00", "dayOccurrence": 1,
                "dayOfWeek": "sunday", "month": 10}}))
        .unwrap();
        assert_eq!(c.offset_min(ms("2026-01-15T00:00:00Z")), 660);
        assert_eq!(c.offset_min(ms("2026-07-15T00:00:00Z")), 600);
    }

    #[test]
    fn last_weekday_of_month() {
        // The last Sunday of March 2026 is the 29th.
        let t = Transition {
            month: 3,
            day_occurrence: 5,
            weekday: Weekday::Sunday,
            time_min: 60,
        };
        assert_eq!(transition_local_ms(&t, 2026), ms("2026-03-29T01:00:00Z"));
    }
}
