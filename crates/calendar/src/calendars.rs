//! The calendars list and other people's calendars (C1.6): my calendars and groups, creating
//! and deleting calendars, the view of one calendar, and a colleague's calendar with a
//! free/busy fallback.

use reqwest::Method;
use serde_json::{Value, json};
use teams_proto::broker::Audience;
use teams_proto::{Client, Error, Result};

use crate::api::SELECT;
use crate::event::{Event, next_link, parse_events};
use crate::graph::{GRAPH, enc, graph, send, send_raw};
use crate::range::format_iso_ms;
use crate::schedule::{Schedule, get_schedule};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarInfo {
    pub id: String,
    pub name: String,
    /// Graph's named color (`lightBlue`, `auto`, …).
    pub color: String,
    /// `#RRGGBB` when the server gives one.
    pub hex_color: Option<String>,
    pub is_default: bool,
    pub can_edit: bool,
    pub can_share: bool,
    pub owner_name: String,
    pub owner_email: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarGroup {
    pub id: String,
    pub name: String,
}

fn text(v: &Value, key: &str) -> Option<String> {
    Some(v.get(key)?.as_str()?.to_string()).filter(|s| !s.is_empty())
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub fn parse_calendars(v: &Value) -> Vec<CalendarInfo> {
    v.get("value")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|c| {
                    let owner = c.get("owner").unwrap_or(&Value::Null);
                    Some(CalendarInfo {
                        id: text(c, "id")?,
                        name: text(c, "name").unwrap_or_default(),
                        color: text(c, "color").unwrap_or_default(),
                        hex_color: text(c, "hexColor"),
                        is_default: flag(c, "isDefaultCalendar"),
                        can_edit: flag(c, "canEdit"),
                        can_share: flag(c, "canShare"),
                        owner_name: text(owner, "name").unwrap_or_default(),
                        owner_email: text(owner, "address").unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_calendar_groups(v: &Value) -> Vec<CalendarGroup> {
    v.get("value")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|g| {
                    Some(CalendarGroup {
                        id: text(g, "id")?,
                        name: text(g, "name").unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The colors Graph accepts for a calendar.
pub const CALENDAR_COLORS: [&str; 11] = [
    "auto",
    "lightBlue",
    "lightGreen",
    "lightOrange",
    "lightGray",
    "lightYellow",
    "lightTeal",
    "lightPink",
    "lightBrown",
    "lightRed",
    "maxColor",
];

/// The body of `POST /me/calendars`. An unknown color name becomes `auto`.
pub fn create_calendar_body(name: &str, color: &str) -> Value {
    let color = CALENDAR_COLORS
        .iter()
        .find(|c| c.eq_ignore_ascii_case(color))
        .copied()
        .unwrap_or("auto");
    json!({"name": name, "color": color})
}

/// My calendars: the default one, the ones I made and the ones shared with me.
pub async fn calendars(client: &Client) -> Result<Vec<CalendarInfo>> {
    let v = graph(
        client,
        "calendars",
        Method::GET,
        "/me/calendars?$top=100",
        None,
    )
    .await?;
    Ok(parse_calendars(&v))
}

pub async fn calendar_groups(client: &Client) -> Result<Vec<CalendarGroup>> {
    let v = graph(
        client,
        "calendar groups",
        Method::GET,
        "/me/calendarGroups?$top=100",
        None,
    )
    .await?;
    Ok(parse_calendar_groups(&v))
}

/// The calendars inside one group.
pub async fn calendars_in_group(client: &Client, group_id: &str) -> Result<Vec<CalendarInfo>> {
    let v = graph(
        client,
        "calendars in group",
        Method::GET,
        &format!("/me/calendarGroups/{}/calendars?$top=100", enc(group_id)),
        None,
    )
    .await?;
    Ok(parse_calendars(&v))
}

/// Creates a blank calendar.
pub async fn create_calendar(client: &Client, name: &str, color: &str) -> Result<CalendarInfo> {
    let v = graph(
        client,
        "create calendar",
        Method::POST,
        "/me/calendars",
        Some(&create_calendar_body(name, color)),
    )
    .await?;
    parse_calendars(&json!({"value": [v]}))
        .into_iter()
        .next()
        .ok_or_else(|| Error::Protocol("create calendar: no id in the response".into()))
}

/// Deletes a calendar I made. The default calendar cannot be deleted.
pub async fn delete_calendar(client: &Client, id: &str) -> Result<()> {
    graph(
        client,
        "delete calendar",
        Method::DELETE,
        &format!("/me/calendars/{}", enc(id)),
        None,
    )
    .await?;
    Ok(())
}

fn view_url(base: &str, start_ms: i64, end_ms: i64) -> String {
    format!(
        "{base}/calendarView?startDateTime={}&endDateTime={}&$top=200&$select={SELECT}",
        format_iso_ms(start_ms),
        format_iso_ms(end_ms)
    )
}

const MAX_PAGES: usize = 100;

async fn follow(client: &Client, first: Value) -> Result<Vec<Event>> {
    let mut events = parse_events(&first);
    let mut next = next_link(&first);
    for _ in 0..MAX_PAGES {
        let Some(url) = next else {
            return Ok(events);
        };
        let page = send(
            client,
            &Audience::Graph,
            "calendar view",
            Method::GET,
            &url,
            None,
        )
        .await?;
        events.extend(parse_events(&page));
        next = next_link(&page);
    }
    Err(Error::Protocol(format!(
        "calendar view: more than {MAX_PAGES} pages"
    )))
}

/// The events of one calendar between two instants, recurring series expanded.
pub async fn calendar_view(
    client: &Client,
    calendar_id: &str,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<Event>> {
    let url = view_url(
        &format!("{GRAPH}/me/calendars/{}", enc(calendar_id)),
        start_ms,
        end_ms,
    );
    let first = send(
        client,
        &Audience::Graph,
        "calendar view",
        Method::GET,
        &url,
        None,
    )
    .await?;
    follow(client, first).await
}

/// A colleague's calendar: the events when I may read them, or just free/busy when I may not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonView {
    Events(Vec<Event>),
    FreeBusyOnly(Schedule),
}

/// Graph answers 403 (`ErrorAccessDenied`) when the owner shares only free/busy, or nothing.
pub fn is_access_denied(status: u16, body: &str) -> bool {
    status == 403 || (status == 404 && body.contains("ErrorAccessDenied"))
}

/// `GET /users/{upn}/calendarView`, and `getSchedule` for the same person when that is
/// refused. The schedule's own `error` says when even free/busy is hidden.
pub async fn person_view(
    client: &Client,
    upn: &str,
    start_ms: i64,
    end_ms: i64,
    interval_minutes: u32,
) -> Result<PersonView> {
    let url = view_url(&format!("{GRAPH}/users/{}", enc(upn)), start_ms, end_ms);
    let (status, text) = send_raw(client, &Audience::Graph, Method::GET, &url, None).await?;
    if is_access_denied(status, &text) {
        let schedules = get_schedule(
            client,
            &[upn.to_string()],
            start_ms,
            end_ms,
            interval_minutes,
        )
        .await?;
        let schedule = schedules
            .into_iter()
            .next()
            .ok_or_else(|| Error::Protocol("get schedule: no schedule for this person".into()))?;
        return Ok(PersonView::FreeBusyOnly(schedule));
    }
    let first = crate::graph::parse("calendar view", status, &text)?;
    Ok(PersonView::Events(follow(client, first).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_calendars() {
        let c = parse_calendars(&json!({"value": [
            {"id": "c1", "name": "Calendar", "color": "auto", "hexColor": "",
             "isDefaultCalendar": true, "canEdit": true, "canShare": true,
             "owner": {"name": "Me", "address": "me@contoso.example"}},
            {"id": "c2", "name": "Team", "color": "lightBlue", "hexColor": "#0078d4",
             "owner": {"name": "Sara", "address": "sara@contoso.example"}},
            {"name": "no id"}]}));
        assert_eq!(c.len(), 2);
        assert!(c[0].is_default && c[0].can_edit && c[0].hex_color.is_none());
        assert_eq!(c[1].hex_color.as_deref(), Some("#0078d4"));
        assert!(!c[1].can_edit);
        assert_eq!(c[1].owner_email, "sara@contoso.example");
    }

    #[test]
    fn parses_groups() {
        let g = parse_calendar_groups(&json!({"value": [
            {"id": "g1", "name": "My Calendars", "classId": "x"}, {"name": "x"}]}));
        assert_eq!(
            g,
            [CalendarGroup {
                id: "g1".into(),
                name: "My Calendars".into()
            }]
        );
    }

    #[test]
    fn create_body_keeps_known_colors_only() {
        assert_eq!(
            create_calendar_body("ironteams-test", "lightblue"),
            json!({"name": "ironteams-test", "color": "lightBlue"})
        );
        assert_eq!(create_calendar_body("x", "purple")["color"], "auto");
    }

    #[test]
    fn view_urls_for_a_calendar_and_a_person() {
        let u = view_url(
            "https://graph.microsoft.com/v1.0/users/sara%40contoso.example",
            1_794_960_000_000,
            1_795_564_800_000,
        );
        assert!(u.starts_with(
            "https://graph.microsoft.com/v1.0/users/sara%40contoso.example/calendarView?\
             startDateTime=2026-11-18T00:00:00.000Z&endDateTime=2026-11-25T00:00:00.000Z&$top=200&$select=id,"
        ));
    }

    #[test]
    fn access_denied_means_fall_back_to_free_busy() {
        assert!(is_access_denied(403, ""));
        assert!(is_access_denied(
            404,
            "{\"error\":{\"code\":\"ErrorAccessDenied\"}}"
        ));
        assert!(!is_access_denied(
            404,
            "{\"error\":{\"code\":\"ErrorItemNotFound\"}}"
        ));
        assert!(!is_access_denied(200, ""));
        assert!(!is_access_denied(500, ""));
    }
}
