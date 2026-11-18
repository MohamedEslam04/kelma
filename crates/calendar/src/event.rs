//! The event model and the parsers for Graph calendarView and delta JSON.

use serde_json::Value;
use teams_proto::time::parse_iso_ms;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShowAs {
    Free,
    Tentative,
    Busy,
    Oof,
    WorkingElsewhere,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    None,
    Organizer,
    Accepted,
    Tentative,
    Declined,
    NotResponded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttendeeKind {
    Required,
    Optional,
    Resource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attendee {
    pub name: String,
    pub email: String,
    pub response: Response,
    pub kind: AttendeeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub id: String,
    pub ical_uid: String,
    pub series_master_id: Option<String>,
    pub subject: String,
    /// Unix milliseconds. All-day events start at midnight and end at the next midnight.
    pub start_ms: i64,
    pub end_ms: i64,
    pub is_all_day: bool,
    pub show_as: ShowAs,
    /// The signed-in user's own response.
    pub response: Response,
    pub is_cancelled: bool,
    pub sensitivity_private: bool,
    pub categories: Vec<String>,
    pub organizer_name: String,
    pub organizer_email: String,
    pub location: String,
    pub is_online_meeting: bool,
    pub join_url: Option<String>,
    pub web_link: String,
    /// An occurrence or exception of a series, or the series master itself.
    pub is_recurring: bool,
    pub body_preview: String,
    pub attendees: Vec<Attendee>,
}

/// One entry of a delta page.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)] // changes are short-lived and handled by value
pub enum EventChange {
    Upsert(Event),
    Removed(String),
}

/// A page of a delta response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaPage {
    pub changes: Vec<EventChange>,
    pub next_link: Option<String>,
    pub delta_link: Option<String>,
}

fn str_at<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn opt_str(v: &Value, key: &str) -> Option<String> {
    Some(v.get(key)?.as_str()?.to_string()).filter(|s| !s.is_empty())
}

fn show_as(s: &str) -> ShowAs {
    match s {
        "free" => ShowAs::Free,
        "tentative" => ShowAs::Tentative,
        "busy" => ShowAs::Busy,
        "oof" => ShowAs::Oof,
        "workingElsewhere" => ShowAs::WorkingElsewhere,
        _ => ShowAs::Unknown,
    }
}

fn response(s: &str) -> Response {
    match s {
        "organizer" => Response::Organizer,
        "accepted" => Response::Accepted,
        "tentativelyAccepted" => Response::Tentative,
        "declined" => Response::Declined,
        "notResponded" => Response::NotResponded,
        _ => Response::None,
    }
}

/// Graph returns `{ "dateTime": "2026-11-18T09:00:00.0000000", "timeZone": "UTC" }`. Requests
/// carry `Prefer: outlook.timezone="UTC"`, so the value is UTC.
fn date_time_ms(v: &Value, key: &str) -> Option<i64> {
    parse_iso_ms(v.get(key)?.get("dateTime")?.as_str()?)
}

fn parse_attendee(a: &Value) -> Attendee {
    let addr = a.get("emailAddress").unwrap_or(&Value::Null);
    Attendee {
        name: str_at(addr, "name").to_string(),
        email: str_at(addr, "address").to_string(),
        response: response(str_at(a.get("status").unwrap_or(&Value::Null), "response")),
        kind: match str_at(a, "type") {
            "optional" => AttendeeKind::Optional,
            "resource" => AttendeeKind::Resource,
            _ => AttendeeKind::Required,
        },
    }
}

/// Parses one Graph event. `None` when it has no id.
pub(crate) fn parse_event(v: &Value) -> Option<Event> {
    let id = opt_str(v, "id")?;
    let start_ms = date_time_ms(v, "start").unwrap_or(0);
    let end_ms = date_time_ms(v, "end").unwrap_or(start_ms);
    let organizer = v
        .get("organizer")
        .and_then(|o| o.get("emailAddress"))
        .unwrap_or(&Value::Null);
    let join_url = v
        .get("onlineMeeting")
        .and_then(|m| opt_str(m, "joinUrl"))
        .or_else(|| opt_str(v, "onlineMeetingUrl"));
    Some(Event {
        id,
        ical_uid: str_at(v, "iCalUId").to_string(),
        series_master_id: opt_str(v, "seriesMasterId"),
        subject: str_at(v, "subject").to_string(),
        start_ms,
        end_ms,
        is_all_day: v.get("isAllDay").and_then(Value::as_bool).unwrap_or(false),
        show_as: show_as(str_at(v, "showAs")),
        response: response(str_at(
            v.get("responseStatus").unwrap_or(&Value::Null),
            "response",
        )),
        is_cancelled: v
            .get("isCancelled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        sensitivity_private: str_at(v, "sensitivity") == "private",
        categories: v
            .get("categories")
            .and_then(Value::as_array)
            .map(|c| {
                c.iter()
                    .filter_map(|s| s.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        organizer_name: str_at(organizer, "name").to_string(),
        organizer_email: str_at(organizer, "address").to_string(),
        location: str_at(v.get("location").unwrap_or(&Value::Null), "displayName").to_string(),
        is_online_meeting: v
            .get("isOnlineMeeting")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        join_url,
        web_link: str_at(v, "webLink").to_string(),
        is_recurring: matches!(
            str_at(v, "type"),
            "occurrence" | "exception" | "seriesMaster"
        ),
        body_preview: str_at(v, "bodyPreview").to_string(),
        attendees: v
            .get("attendees")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(parse_attendee).collect())
            .unwrap_or_default(),
    })
}

/// Parses the events of a calendarView page (`{"value": [...]}`). Items without an id are
/// skipped.
pub fn parse_events(page: &Value) -> Vec<Event> {
    page.get("value")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(parse_event).collect())
        .unwrap_or_default()
}

/// `@odata.nextLink` of a page, when there are more.
pub fn next_link(page: &Value) -> Option<String> {
    opt_str(page, "@odata.nextLink")
}

/// `@odata.deltaLink` of the last page of a delta response.
pub fn delta_link(page: &Value) -> Option<String> {
    opt_str(page, "@odata.deltaLink")
}

/// Parses a delta page: entries with `@removed` become [`EventChange::Removed`].
pub fn parse_delta_page(page: &Value) -> DeltaPage {
    let changes = page
        .get("value")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    if item.get("@removed").is_some() {
                        opt_str(item, "id").map(EventChange::Removed)
                    } else {
                        parse_event(item).map(EventChange::Upsert)
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    DeltaPage {
        changes,
        next_link: next_link(page),
        delta_link: delta_link(page),
    }
}
