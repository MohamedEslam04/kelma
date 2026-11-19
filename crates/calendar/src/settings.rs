//! Calendar settings that live in the mailbox (work hours, time zone) and the work location
//! pin (C3.3, C3.4).
//!
//! Graph's `/me/mailboxSettings` answers for this client, so the settings come from there;
//! Outlook REST (403) and `getSchedule`'s own work hours are the fallbacks.

use reqwest::Method;
use serde_json::{Value, json};
use teams_proto::broker::Audience;
use teams_proto::time::parse_iso_ms;
use teams_proto::{Client, Error, Result};

use crate::graph::{graph, parse, send_raw};
use crate::range::Weekday;

pub const MAILBOX_SETTINGS_URL: &str = "https://outlook.office.com/api/v2.0/me/MailboxSettings";

/// Graph's mailbox settings, read with the Graph token (it answers for this client).
pub const GRAPH_MAILBOX_SETTINGS_PATH: &str = "/me/mailboxSettings";

/// The days and hours I work, in `time_zone`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkingHours {
    pub days: Vec<Weekday>,
    /// Minutes after midnight.
    pub start_min: u32,
    pub end_min: u32,
    pub time_zone: String,
    /// The zone's rules when Graph sends a `customTimeZone` instead of a name.
    pub custom_zone: Option<crate::tz::CustomZone>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MailboxSettings {
    /// The mailbox time zone, a Windows zone name.
    pub time_zone: String,
    pub working_hours: WorkingHours,
    pub date_format: String,
    pub time_format: String,
}

/// A key looked up ignoring case, because Outlook REST uses `TimeZone` and Graph `timeZone`.
fn ci<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.as_object()?
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

fn ci_str(v: &Value, key: &str) -> String {
    ci(v, key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn weekday(s: &str) -> Option<Weekday> {
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

/// `09:30:00.0000000` to minutes after midnight.
fn minutes_of_day(s: &str) -> Option<u32> {
    let ms = parse_iso_ms(&format!("1970-01-01T{s}"))?;
    Some((ms / 60_000) as u32)
}

/// Reads a `workingHours` object, from Outlook REST, mailbox settings or `getSchedule`.
pub fn parse_working_hours(hours: &Value) -> WorkingHours {
    let zone = ci(hours, "TimeZone").unwrap_or(&Value::Null);
    WorkingHours {
        days: ci(hours, "DaysOfWeek")
            .and_then(Value::as_array)
            .map(|d| d.iter().filter_map(|x| weekday(x.as_str()?)).collect())
            .unwrap_or_default(),
        start_min: minutes_of_day(&ci_str(hours, "StartTime")).unwrap_or(0),
        end_min: minutes_of_day(&ci_str(hours, "EndTime")).unwrap_or(0),
        time_zone: match zone {
            Value::String(s) => s.clone(),
            other => ci_str(other, "Name"),
        },
        custom_zone: crate::tz::parse_custom_zone(zone),
    }
}

/// The stretches of `from_ms..to_ms` (UTC) inside these work hours, read in the hours' own
/// zone (a Windows or IANA name, or the custom zone; UTC when neither is known). Each day's
/// hours are placed on that day's local date, so daylight saving moves them as it should.
pub fn work_intervals(hours: &WorkingHours, from_ms: i64, to_ms: i64) -> Vec<(i64, i64)> {
    const DAY: i64 = 86_400_000;
    let zone = crate::tz::Zone::new(&hours.time_zone, hours.custom_zone.as_ref());
    if hours.days.is_empty() || hours.end_min == hours.start_min || to_ms <= from_ms {
        return vec![];
    }
    let mut out = Vec::new();
    let mut day = zone.local_date(from_ms).add_days(-1);
    let last = zone.local_date(to_ms).add_days(1);
    while day <= last {
        if hours.days.contains(&day.weekday()) {
            let midnight = day.midnight_ms();
            let start = zone.wall_to_utc(midnight + i64::from(hours.start_min) * 60_000);
            // Hours that end at or before they start run past midnight.
            let end_wall = if hours.end_min > hours.start_min {
                midnight + i64::from(hours.end_min) * 60_000
            } else {
                midnight + DAY + i64::from(hours.end_min) * 60_000
            };
            let (s, e) = (start.max(from_ms), zone.wall_to_utc(end_wall).min(to_ms));
            if s < e {
                out.push((s, e));
            }
        }
        day = day.add_days(1);
    }
    out
}

/// One flag per Scheduler slot (`slots` slots of `interval_minutes` from `start_ms`): whether
/// the slot starts inside these work hours. This shades each attendee's row by their own hours.
pub fn work_slots(
    hours: &WorkingHours,
    start_ms: i64,
    interval_minutes: u32,
    slots: usize,
) -> Vec<bool> {
    let step = i64::from(interval_minutes) * 60_000;
    let end = start_ms + step * slots as i64;
    let spans = work_intervals(hours, start_ms, end);
    (0..slots as i64)
        .map(|i| {
            let at = start_ms + i * step;
            spans.iter().any(|&(s, e)| at >= s && at < e)
        })
        .collect()
}

pub fn parse_mailbox_settings(v: &Value) -> MailboxSettings {
    MailboxSettings {
        time_zone: ci_str(v, "TimeZone"),
        working_hours: parse_working_hours(ci(v, "WorkingHours").unwrap_or(&Value::Null)),
        date_format: ci_str(v, "DateFormat"),
        time_format: ci_str(v, "TimeFormat"),
    }
}

/// A category from the mailbox's master list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub name: String,
    /// Outlook's color preset, 0 to 24, when the category has a color.
    pub color_preset: Option<u8>,
}

/// Reads `GET /api/v2.0/me/MasterCategories`: `{"value": [{"DisplayName", "Color": "Preset3"}]}`.
pub fn parse_categories(v: &Value) -> Vec<Category> {
    ci(v, "value")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|c| {
                    let name = ci_str(c, "DisplayName");
                    (!name.is_empty()).then(|| Category {
                        name,
                        color_preset: ci_str(c, "Color")
                            .strip_prefix("Preset")
                            .and_then(|n| n.parse().ok()),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The category names and colors the category pickers and filters offer. Graph's
/// `/me/outlook/masterCategories` needs a scope the token lacks; Outlook REST serves the same
/// list.
pub async fn categories(client: &Client) -> Result<Vec<Category>> {
    let v = crate::graph::send(
        client,
        &Audience::Outlook,
        "categories",
        Method::GET,
        "https://outlook.office.com/api/v2.0/me/MasterCategories",
        None,
    )
    .await?;
    Ok(parse_categories(&v))
}

/// Work hours and time zone, and the display formats when the mailbox lets us read them.
///
/// Graph `GET /me/mailboxSettings` answers for this client (checked live on 2026-11-19), so it
/// goes first. Outlook REST's `/me/MailboxSettings` answers 403 (it asks for
/// `MailboxSettings.Read`); it stays as the second try. When both fail the work hours and time
/// zone come from the `workingHours` that Graph `getSchedule` returns for my own address, and
/// the formats stay empty.
pub async fn mailbox_settings(client: &Client) -> Result<MailboxSettings> {
    let (status, text) = send_raw(
        client,
        &Audience::Graph,
        Method::GET,
        &format!("{}{GRAPH_MAILBOX_SETTINGS_PATH}", crate::graph::GRAPH),
        None,
    )
    .await?;
    if (200..300).contains(&status) {
        return Ok(parse_mailbox_settings(&parse(
            "mailbox settings",
            status,
            &text,
        )?));
    }
    let (status, text) = send_raw(
        client,
        &Audience::Outlook,
        Method::GET,
        MAILBOX_SETTINGS_URL,
        None,
    )
    .await?;
    if status != 403 {
        return Ok(parse_mailbox_settings(&parse(
            "mailbox settings",
            status,
            &text,
        )?));
    }
    let me = graph(
        client,
        "me",
        Method::GET,
        "/me?$select=mail,userPrincipalName",
        None,
    )
    .await?;
    let email = me
        .get("mail")
        .and_then(Value::as_str)
        .or_else(|| me.get("userPrincipalName").and_then(Value::as_str))
        .ok_or_else(|| Error::Protocol("me: no mail address".into()))?
        .to_string();
    let now = teams_proto::time::now_ms();
    let schedules =
        crate::schedule::get_schedule(client, &[email], now, now + 3_600_000, 60).await?;
    let hours = schedules
        .into_iter()
        .next()
        .and_then(|s| s.working_hours)
        .ok_or_else(|| Error::Protocol("get schedule: no working hours".into()))?;
    Ok(MailboxSettings {
        time_zone: hours.time_zone.clone(),
        working_hours: hours,
        ..MailboxSettings::default()
    })
}

/// A time zone the mailbox offers, for a time zone picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeZoneInfo {
    /// The zone's id: a Windows name, or an IANA name when asked for IANA.
    pub alias: String,
    pub display_name: String,
}

/// Reads `GET /me/outlook/supportedTimeZones`: `{"value": [{"alias", "displayName"}]}`.
pub fn parse_time_zones(v: &Value) -> Vec<TimeZoneInfo> {
    ci(v, "value")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|z| {
                    let alias = ci_str(z, "alias");
                    (!alias.is_empty()).then(|| TimeZoneInfo {
                        display_name: ci_str(z, "displayName"),
                        alias,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn time_zones_path(iana: bool) -> &'static str {
    if iana {
        "/me/outlook/supportedTimeZones(TimeZoneStandard=microsoft.graph.timeZoneStandard'Iana')"
    } else {
        "/me/outlook/supportedTimeZones"
    }
}

/// The zones a second-time-zone picker can offer, in Windows names (`iana: false`, 143 live)
/// or IANA names (`iana: true`, 553 live).
pub async fn supported_time_zones(client: &Client, iana: bool) -> Result<Vec<TimeZoneInfo>> {
    let v = graph(
        client,
        "supported time zones",
        Method::GET,
        time_zones_path(iana),
        None,
    )
    .await?;
    Ok(parse_time_zones(&v))
}

/// Where I am working on a day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkLocation {
    /// Cleared: no pin.
    Unset,
    Office,
    Remote,
}

impl WorkLocation {
    fn code(self) -> u8 {
        match self {
            WorkLocation::Unset => 0,
            WorkLocation::Office => 1,
            WorkLocation::Remote => 2,
        }
    }

    /// Reads the presence service's value, a number or a name.
    pub fn from_value(v: &Value) -> WorkLocation {
        let code = v
            .as_u64()
            .or_else(|| match v.as_str()?.to_ascii_lowercase().as_str() {
                "office" | "inoffice" => Some(1),
                "remote" | "remoteoffice" | "home" => Some(2),
                _ => None,
            });
        match code {
            Some(1) => WorkLocation::Office,
            Some(2) => WorkLocation::Remote,
            _ => WorkLocation::Unset,
        }
    }
}

/// The `PUT …/me/workLocation/` body. `expires_ms` is when the pin lapses (usually the end of
/// the day); `Unset` clears it.
pub fn work_location_body(location: WorkLocation, expires_ms: i64) -> Value {
    match location {
        WorkLocation::Unset => json!({"location": 0}),
        l => json!({
            "location": l.code(),
            "expirationTime": crate::range::format_iso_ms(expires_ms),
        }),
    }
}

/// Pins my work location until `expires_ms`.
pub async fn set_work_location(
    client: &Client,
    location: WorkLocation,
    expires_ms: i64,
) -> Result<()> {
    let body = work_location_body(location, expires_ms);
    client
        .presence_json(
            "set work location",
            Method::PUT,
            "me/workLocation/",
            Some(&body),
        )
        .await?;
    Ok(())
}

/// The work location in a `getpresence` answer for one person.
pub fn parse_work_location(presence: &Value) -> WorkLocation {
    let item = presence
        .as_array()
        .and_then(|a| a.first())
        .unwrap_or(presence);
    let loc = item
        .get("presence")
        .unwrap_or(item)
        .get("workLocation")
        .unwrap_or(&Value::Null);
    WorkLocation::from_value(loc.get("location").unwrap_or(loc))
}

/// My work location, read through `getpresence` for my own MRI.
pub async fn work_location(client: &Client) -> Result<WorkLocation> {
    let me = graph(client, "me", Method::GET, "/me?$select=id", None).await?;
    let oid = me
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol("me: no id".into()))?;
    let body = json!([{"mri": format!("8:orgid:{oid}")}]);
    let v = client
        .presence_json(
            "work location",
            Method::POST,
            "presence/getpresence/",
            Some(&body),
        )
        .await?;
    Ok(parse_work_location(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_outlook_rest_pascal_case() {
        let s = parse_mailbox_settings(&json!({
            "@odata.context": "x",
            "TimeZone": "Egypt Standard Time",
            "DateFormat": "dd/MM/yyyy", "TimeFormat": "HH:mm",
            "WorkingHours": {
                "DaysOfWeek": ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday"],
                "StartTime": "09:00:00.0000000", "EndTime": "17:30:00.0000000",
                "TimeZone": {"Name": "Egypt Standard Time"}}}));
        assert_eq!(s.time_zone, "Egypt Standard Time");
        assert_eq!(s.working_hours.days.len(), 5);
        assert_eq!(s.working_hours.days[0], Weekday::Sunday);
        assert_eq!(
            (s.working_hours.start_min, s.working_hours.end_min),
            (540, 1050)
        );
        assert_eq!(s.working_hours.time_zone, "Egypt Standard Time");
        assert_eq!(s.time_format, "HH:mm");
    }

    #[test]
    fn parses_graph_camel_case_too() {
        let s = parse_mailbox_settings(&json!({
            "timeZone": "UTC",
            "workingHours": {"daysOfWeek": ["monday"], "startTime": "08:00:00.0000000",
                "endTime": "16:00:00.0000000", "timeZone": {"name": "UTC"}}}));
        assert_eq!(s.working_hours.days, [Weekday::Monday]);
        assert_eq!(
            (s.working_hours.start_min, s.working_hours.end_min),
            (480, 960)
        );
        assert_eq!(s.working_hours.time_zone, "UTC");
    }

    fn ms(s: &str) -> i64 {
        parse_iso_ms(s).unwrap()
    }

    fn hours(zone: Value) -> WorkingHours {
        parse_working_hours(&json!({"daysOfWeek": ["monday", "tuesday"],
            "startTime": "09:00:00.0000000", "endTime": "17:00:00.0000000", "timeZone": zone}))
    }

    #[test]
    fn work_intervals_in_a_named_zone() {
        // Tokyo is UTC+9: Monday 2026-11-16 09:00-17:00 local is Monday 00:00-08:00 UTC.
        let h = hours(json!({"name": "Tokyo Standard Time"}));
        let from = ms("2026-11-15T00:00:00Z");
        let to = ms("2026-11-22T00:00:00Z");
        assert_eq!(
            work_intervals(&h, from, to),
            [
                (ms("2026-11-16T00:00:00Z"), ms("2026-11-16T08:00:00Z")),
                (ms("2026-11-17T00:00:00Z"), ms("2026-11-17T08:00:00Z")),
            ]
        );
        // Clipped to the window.
        assert_eq!(
            work_intervals(&h, ms("2026-11-16T06:00:00Z"), ms("2026-11-16T12:00:00Z")),
            [(ms("2026-11-16T06:00:00Z"), ms("2026-11-16T08:00:00Z"))]
        );
    }

    #[test]
    fn work_intervals_with_a_custom_zone_and_daylight_saving() {
        let h = hours(json!({"@odata.type": "#microsoft.graph.customTimeZone",
            "bias": 480, "name": "Customized Time Zone",
            "standardOffset": {"time": "02:00:00.0000000", "dayOccurrence": 1,
                "dayOfWeek": "sunday", "month": 11, "year": 0},
            "daylightOffset": {"daylightBias": -60, "time": "02:00:00.0000000",
                "dayOccurrence": 2, "dayOfWeek": "sunday", "month": 3, "year": 0}}));
        assert!(h.custom_zone.is_some());
        assert_eq!(h.time_zone, "Customized Time Zone");
        // Monday 2026-07-06 in PDT (-7) and Monday 2026-11-16 in PST (-8).
        assert_eq!(
            work_intervals(&h, ms("2026-07-06T00:00:00Z"), ms("2026-07-07T00:00:00Z")),
            [(ms("2026-07-06T16:00:00Z"), ms("2026-07-07T00:00:00Z"))]
        );
        assert_eq!(
            work_intervals(&h, ms("2026-11-16T00:00:00Z"), ms("2026-11-18T00:00:00Z")),
            [
                (ms("2026-11-16T17:00:00Z"), ms("2026-11-17T01:00:00Z")),
                (ms("2026-11-17T17:00:00Z"), ms("2026-11-18T00:00:00Z")),
            ]
        );
    }

    #[test]
    fn work_slots_shade_each_slot() {
        // Cairo is UTC+2 in November 2026: 09:00-17:00 local is 07:00-15:00 UTC.
        let h = hours(json!({"name": "Egypt Standard Time"}));
        let start = ms("2026-11-16T06:00:00Z");
        let slots = work_slots(&h, start, 60, 10);
        assert_eq!(
            slots,
            [false, true, true, true, true, true, true, true, true, false]
        );
        // An unknown zone reads as UTC; no work days means no work slots.
        let utc = hours(json!({"name": "Somewhere Else"}));
        assert_eq!(
            work_slots(&utc, ms("2026-11-16T08:00:00Z"), 60, 2),
            [false, true]
        );
        assert_eq!(
            work_slots(&WorkingHours::default(), start, 30, 3),
            [false; 3]
        );
    }

    #[test]
    fn parses_supported_time_zones() {
        let z = parse_time_zones(&json!({"value": [
            {"alias": "Egypt Standard Time", "displayName": "(UTC+02:00) Cairo"},
            {"alias": "", "displayName": "skipped"}]}));
        assert_eq!(
            z,
            [TimeZoneInfo {
                alias: "Egypt Standard Time".into(),
                display_name: "(UTC+02:00) Cairo".into()
            }]
        );
        assert!(time_zones_path(true).ends_with("timeZoneStandard'Iana')"));
        assert_eq!(time_zones_path(false), "/me/outlook/supportedTimeZones");
    }

    #[test]
    fn parses_master_categories() {
        let c = parse_categories(&json!({"value": [
            {"DisplayName": "Blue category", "Color": "Preset7", "Id": "x"},
            {"DisplayName": "Plain", "Color": "None"},
            {"Color": "Preset1"}]}));
        assert_eq!(
            c,
            [
                Category {
                    name: "Blue category".into(),
                    color_preset: Some(7)
                },
                Category {
                    name: "Plain".into(),
                    color_preset: None
                },
            ]
        );
    }

    #[test]
    fn empty_settings_are_empty() {
        assert_eq!(
            parse_mailbox_settings(&json!({})),
            MailboxSettings::default()
        );
    }

    #[test]
    fn work_location_request_body() {
        assert_eq!(
            work_location_body(WorkLocation::Remote, 1_795_046_400_000),
            json!({"location": 2, "expirationTime": "2026-11-19T00:00:00.000Z"})
        );
        assert_eq!(
            work_location_body(WorkLocation::Office, 1_795_046_400_000)["location"],
            1
        );
        assert_eq!(
            work_location_body(WorkLocation::Unset, 0),
            json!({"location": 0})
        );
    }

    #[test]
    fn reads_work_location_from_presence() {
        let p = json!([{"mri": "8:orgid:x", "presence": {"availability": "Available",
            "workLocation": {"location": 2, "locationSource": "UserSet"}}}]);
        assert_eq!(parse_work_location(&p), WorkLocation::Remote);
        let p = json!([{"mri": "8:orgid:x", "presence": {"workLocation": {"location": "office"}}}]);
        assert_eq!(parse_work_location(&p), WorkLocation::Office);
        assert_eq!(
            parse_work_location(&json!([{"presence": {}}])),
            WorkLocation::Unset
        );
        assert_eq!(parse_work_location(&json!([])), WorkLocation::Unset);
    }
}
