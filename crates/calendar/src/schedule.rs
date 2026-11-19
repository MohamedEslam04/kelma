//! The scheduling assistant and rooms (C2.3, C2.4): free/busy of people and rooms,
//! suggested meeting times, room lists.

use reqwest::Method;
use serde_json::{Value, json};
use teams_proto::time::parse_iso_ms;
use teams_proto::{Client, Error, Result};

use crate::event::ShowAs;
use crate::graph::{GRAPH, date_time, email_address, enc, graph, wall_utc};
use crate::new_event::Person;

/// One slot of an `availabilityView` string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Free,
    Tentative,
    Busy,
    Oof,
    WorkingElsewhere,
    Unknown,
}

impl Availability {
    fn from_digit(c: char) -> Availability {
        match c {
            '0' => Availability::Free,
            '1' => Availability::Tentative,
            '2' => Availability::Busy,
            '3' => Availability::Oof,
            '4' => Availability::WorkingElsewhere,
            _ => Availability::Unknown,
        }
    }

    fn from_name(s: &str) -> Availability {
        match s {
            "free" => Availability::Free,
            "tentative" => Availability::Tentative,
            "busy" => Availability::Busy,
            "oof" => Availability::Oof,
            "workingElsewhere" => Availability::WorkingElsewhere,
            _ => Availability::Unknown,
        }
    }
}

/// A busy block on someone's calendar. The subject and place are only there when the
/// calendar's permissions allow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleItem {
    pub status: ShowAs,
    pub start_ms: i64,
    pub end_ms: i64,
    pub subject: Option<String>,
    pub location: Option<String>,
    pub is_private: bool,
}

/// One person's or room's free/busy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    pub email: String,
    /// One entry per interval of the request.
    pub availability: Vec<Availability>,
    pub items: Vec<ScheduleItem>,
    /// Why Graph could not read this schedule, for example no permission.
    pub error: Option<String>,
    /// The person's work hours, when Graph returns them.
    pub working_hours: Option<crate::settings::WorkingHours>,
}

pub fn get_schedule_body(
    emails: &[String],
    start_ms: i64,
    end_ms: i64,
    interval_minutes: u32,
) -> Value {
    json!({
        "schedules": emails,
        "startTime": date_time(&wall_utc(start_ms), "UTC"),
        "endTime": date_time(&wall_utc(end_ms), "UTC"),
        "availabilityViewInterval": interval_minutes,
    })
}

fn time_ms(v: &Value, key: &str) -> Option<i64> {
    parse_iso_ms(v.get(key)?.get("dateTime")?.as_str()?)
}

fn text(v: &Value, key: &str) -> Option<String> {
    Some(v.get(key)?.as_str()?.to_string()).filter(|s| !s.is_empty())
}

pub fn parse_schedules(v: &Value) -> Vec<Schedule> {
    v.get("value")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|r| Schedule {
                    email: text(r, "scheduleId").unwrap_or_default(),
                    availability: r
                        .get("availabilityView")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .chars()
                        .map(Availability::from_digit)
                        .collect(),
                    items: r
                        .get("scheduleItems")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|i| {
                                    let start_ms = time_ms(i, "start")?;
                                    Some(ScheduleItem {
                                        status: match Availability::from_name(
                                            i.get("status").and_then(Value::as_str).unwrap_or(""),
                                        ) {
                                            Availability::Free => ShowAs::Free,
                                            Availability::Tentative => ShowAs::Tentative,
                                            Availability::Busy => ShowAs::Busy,
                                            Availability::Oof => ShowAs::Oof,
                                            Availability::WorkingElsewhere => {
                                                ShowAs::WorkingElsewhere
                                            }
                                            Availability::Unknown => ShowAs::Unknown,
                                        },
                                        start_ms,
                                        end_ms: time_ms(i, "end").unwrap_or(start_ms),
                                        subject: text(i, "subject"),
                                        location: text(i, "location"),
                                        is_private: i
                                            .get("isPrivate")
                                            .and_then(Value::as_bool)
                                            .unwrap_or(false),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    error: r
                        .get("error")
                        .and_then(|e| text(e, "message").or_else(|| text(e, "responseCode"))),
                    working_hours: r
                        .get("workingHours")
                        .map(crate::settings::parse_working_hours),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Free/busy of up to 20 people or rooms between two instants, one availability slot per
/// `interval_minutes`.
pub async fn get_schedule(
    client: &Client,
    emails: &[String],
    start_ms: i64,
    end_ms: i64,
    interval_minutes: u32,
) -> Result<Vec<Schedule>> {
    if emails.is_empty() {
        return Ok(vec![]);
    }
    let body = get_schedule_body(emails, start_ms, end_ms, interval_minutes);
    let v = graph(
        client,
        "get schedule",
        Method::POST,
        "/me/calendar/getSchedule",
        Some(&body),
    )
    .await?;
    Ok(parse_schedules(&v))
}

/// What the Scheduler asks `findMeetingTimes` for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindTimes {
    pub required: Vec<Person>,
    pub optional: Vec<Person>,
    pub duration_minutes: u32,
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub max_candidates: u32,
    /// Share of attendees that must be free for a time to be suggested, 0 to 100.
    pub min_attendee_percentage: u32,
    /// Only working hours (`work`), or any time (`unrestricted`).
    pub work_hours_only: bool,
}

fn iso_duration(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("PT{m}M"),
        (h, 0) => format!("PT{h}H"),
        (h, m) => format!("PT{h}H{m}M"),
    }
}

pub fn find_times_body(q: &FindTimes) -> Value {
    let attendee = |p: &Person, kind: &str| json!({"type": kind, "emailAddress": email_address(&p.name, &p.email)});
    let mut attendees: Vec<Value> = q.required.iter().map(|p| attendee(p, "required")).collect();
    attendees.extend(q.optional.iter().map(|p| attendee(p, "optional")));
    json!({
        "attendees": attendees,
        "timeConstraint": {
            "activityDomain": if q.work_hours_only { "work" } else { "unrestricted" },
            "timeSlots": [{
                "start": date_time(&wall_utc(q.window_start_ms), "UTC"),
                "end": date_time(&wall_utc(q.window_end_ms), "UTC"),
            }],
        },
        "meetingDuration": iso_duration(q.duration_minutes),
        "maxCandidates": q.max_candidates,
        "minimumAttendeePercentage": q.min_attendee_percentage,
        "returnSuggestionReasons": true,
        "isOrganizerOptional": false,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub start_ms: i64,
    pub end_ms: i64,
    /// 0 to 100.
    pub confidence: f64,
    pub reason: String,
    pub organizer: Availability,
    /// `(email, availability)` for each attendee.
    pub attendees: Vec<(String, Availability)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct FoundTimes {
    pub suggestions: Vec<Suggestion>,
    /// Why there are none, for example `attendeesUnavailable`.
    pub empty_reason: Option<String>,
}

pub fn parse_found_times(v: &Value) -> FoundTimes {
    let suggestions = v
        .get("meetingTimeSuggestions")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|s| {
                    let slot = s.get("meetingTimeSlot")?;
                    let name = |v: &Value, k: &str| {
                        Availability::from_name(v.get(k).and_then(Value::as_str).unwrap_or(""))
                    };
                    Some(Suggestion {
                        start_ms: time_ms(slot, "start")?,
                        end_ms: time_ms(slot, "end")?,
                        confidence: s.get("confidence").and_then(Value::as_f64).unwrap_or(0.0),
                        reason: text(s, "suggestionReason").unwrap_or_default(),
                        organizer: name(s, "organizerAvailability"),
                        attendees: s
                            .get("attendeeAvailability")
                            .and_then(Value::as_array)
                            .map(|a| {
                                a.iter()
                                    .map(|x| {
                                        let email = x
                                            .get("attendee")
                                            .and_then(|p| p.get("emailAddress"))
                                            .and_then(|e| text(e, "address"))
                                            .unwrap_or_default();
                                        (email, name(x, "availability"))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    FoundTimes {
        suggestions,
        empty_reason: text(v, "emptySuggestionsReason"),
    }
}

/// Suggested meeting times that suit everyone, best first.
pub async fn find_meeting_times(client: &Client, q: &FindTimes) -> Result<FoundTimes> {
    let v = graph(
        client,
        "find meeting times",
        Method::POST,
        "/me/findMeetingTimes",
        Some(&find_times_body(q)),
    )
    .await?;
    Ok(parse_found_times(&v))
}

/// A list of rooms, usually a building.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomList {
    pub id: String,
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub email: String,
    pub capacity: Option<u32>,
    pub building: Option<String>,
    pub floor_label: Option<String>,
    pub floor_number: Option<i32>,
    pub wheelchair_accessible: bool,
    pub tags: Vec<String>,
    pub has_video: bool,
}

pub fn parse_room_lists(v: &Value) -> Vec<RoomList> {
    v.get("value")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    Some(RoomList {
                        id: text(r, "id")?,
                        name: text(r, "displayName").unwrap_or_default(),
                        email: text(r, "emailAddress")?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_rooms(v: &Value) -> Vec<Room> {
    v.get("value")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    Some(Room {
                        id: text(r, "id")?,
                        name: text(r, "displayName").unwrap_or_default(),
                        email: text(r, "emailAddress")?,
                        capacity: r.get("capacity").and_then(Value::as_u64).map(|c| c as u32),
                        building: text(r, "building"),
                        floor_label: text(r, "floorLabel"),
                        floor_number: r
                            .get("floorNumber")
                            .and_then(Value::as_i64)
                            .map(|f| f as i32),
                        wheelchair_accessible: r
                            .get("isWheelChairAccessible")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        tags: r
                            .get("tags")
                            .and_then(Value::as_array)
                            .map(|t| {
                                t.iter()
                                    .filter_map(|s| s.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        has_video: text(r, "videoDeviceName").is_some()
                            || text(r, "displayDeviceName").is_some(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The answer of Graph beta `findRoomLists`: `{"value": [{"name", "address"}]}`. The address
/// doubles as the id.
pub fn parse_found_room_lists(v: &Value) -> Vec<RoomList> {
    found(v)
        .into_iter()
        .map(|(name, email)| RoomList {
            id: email.clone(),
            name,
            email,
        })
        .collect()
}

/// The answer of Graph beta `findRooms`: names and addresses only, no capacity or building.
pub fn parse_found_rooms(v: &Value) -> Vec<Room> {
    found(v)
        .into_iter()
        .map(|(name, email)| Room {
            id: email.clone(),
            name,
            email,
            capacity: None,
            building: None,
            floor_label: None,
            floor_number: None,
            wheelchair_accessible: false,
            tags: vec![],
            has_video: false,
        })
        .collect()
}

fn found(v: &Value) -> Vec<(String, String)> {
    v.get("value")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|r| Some((text(r, "name").unwrap_or_default(), text(r, "address")?)))
                .collect()
        })
        .unwrap_or_default()
}

const GRAPH_BETA: &str = "https://graph.microsoft.com/beta";

/// Graph beta `findRooms`, of one room list or (empty `list_email`) of the tenant.
pub fn find_rooms_url(list_email: &str) -> String {
    if list_email.is_empty() {
        format!("{GRAPH_BETA}/me/findRooms")
    } else {
        format!("{GRAPH_BETA}/me/findRooms(RoomList='{}')", enc(list_email))
    }
}

/// Whether a schedule has nothing but free time between two instants; `None` when Graph could
/// not read it.
pub fn free_between(s: &Schedule, start_ms: i64, end_ms: i64) -> Option<bool> {
    if s.error.is_some() {
        return None;
    }
    Some(
        !s.items
            .iter()
            .any(|i| i.status != ShowAs::Free && i.start_ms < end_ms && i.end_ms > start_ms),
    )
}

/// `getSchedule` takes at most this many addresses per call.
const SCHEDULES_PER_CALL: usize = 20;

/// Free/busy of any number of rooms (or people), `getSchedule` 20 addresses at a time.
pub async fn room_schedules(
    client: &Client,
    rooms: &[Room],
    start_ms: i64,
    end_ms: i64,
    interval_minutes: u32,
) -> Result<Vec<Schedule>> {
    let emails: Vec<String> = rooms.iter().map(|r| r.email.clone()).collect();
    let mut out = Vec::with_capacity(emails.len());
    for chunk in emails.chunks(SCHEDULES_PER_CALL) {
        out.extend(get_schedule(client, chunk, start_ms, end_ms, interval_minutes).await?);
    }
    Ok(out)
}

/// A safety stop for `nextLink` loops.
const MAX_PAGES: usize = 50;

async fn pages(client: &Client, context: &'static str, first: String) -> Result<Vec<Value>> {
    let mut url = first;
    let mut out = Vec::new();
    for _ in 0..MAX_PAGES {
        let page = crate::graph::send(
            client,
            &teams_proto::broker::Audience::Graph,
            context,
            Method::GET,
            &url,
            None,
        )
        .await?;
        let next = text(&page, "@odata.nextLink");
        out.push(page);
        match next {
            Some(n) => url = n,
            None => return Ok(out),
        }
    }
    Err(Error::Protocol(format!(
        "{context}: more than {MAX_PAGES} pages"
    )))
}

/// The room lists (buildings) of the tenant, from Graph places; when that is refused or empty,
/// from Graph beta `findRoomLists` (Outlook's room finder, no extra scope).
pub async fn room_lists(client: &Client) -> Result<Vec<RoomList>> {
    let pages = pages(
        client,
        "room lists",
        format!("{GRAPH}/places/microsoft.graph.roomList"),
    )
    .await;
    match pages {
        Ok(p) if p.iter().any(|p| !parse_room_lists(p).is_empty()) => {
            Ok(p.iter().flat_map(parse_room_lists).collect())
        }
        // No `Place.Read.All` or no places directory: Outlook's own room finder.
        other => {
            let found = graph_beta(client, "find room lists", "/me/findRoomLists").await;
            match (other, found) {
                (_, Ok(v)) => Ok(parse_found_room_lists(&v)),
                (Ok(_), Err(_)) => Ok(vec![]),
                (Err(e), Err(_)) => Err(e),
            }
        }
    }
}

async fn graph_beta(client: &Client, context: &'static str, url: &str) -> Result<Value> {
    let url = if url.starts_with("https://") {
        url.to_string()
    } else {
        format!("{GRAPH_BETA}{url}")
    };
    crate::graph::send(
        client,
        &teams_proto::broker::Audience::Graph,
        context,
        Method::GET,
        &url,
        None,
    )
    .await
}

/// The rooms of one list, by the list's email address. An empty `list_email` returns every
/// room of the tenant. Falls back to Graph beta `findRooms` like [`room_lists`]; those rooms
/// carry only a name and an address.
pub async fn rooms(client: &Client, list_email: &str) -> Result<Vec<Room>> {
    let url = if list_email.is_empty() {
        format!("{GRAPH}/places/microsoft.graph.room")
    } else {
        format!(
            "{GRAPH}/places/{}/microsoft.graph.roomList/rooms",
            enc(list_email)
        )
    };
    match pages(client, "rooms", url).await {
        Ok(p) if p.iter().any(|p| !parse_rooms(p).is_empty()) => {
            Ok(p.iter().flat_map(parse_rooms).collect())
        }
        other => match (
            other,
            graph_beta(client, "find rooms", &find_rooms_url(list_email)).await,
        ) {
            (_, Ok(v)) => Ok(parse_found_rooms(&v)),
            (Ok(_), Err(_)) => Ok(vec![]),
            (Err(e), Err(_)) => Err(e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_request_body() {
        let b = get_schedule_body(
            &[
                "sara@contoso.example".into(),
                "room4b@contoso.example".into(),
            ],
            1_794_996_000_000,
            1_795_024_800_000,
            30,
        );
        assert_eq!(
            b,
            json!({"schedules": ["sara@contoso.example", "room4b@contoso.example"],
                   "startTime": {"dateTime": "2026-11-18T10:00:00", "timeZone": "UTC"},
                   "endTime": {"dateTime": "2026-11-18T18:00:00", "timeZone": "UTC"},
                   "availabilityViewInterval": 30})
        );
    }

    #[test]
    fn parses_free_busy() {
        let s = parse_schedules(&json!({"value": [
            {"scheduleId": "sara@contoso.example", "availabilityView": "0012340x",
             "workingHours": {"daysOfWeek": ["monday", "tuesday"], "startTime": "08:00:00.0000000",
                "endTime": "16:00:00.0000000", "timeZone": {"name": "Egypt Standard Time"}},
             "scheduleItems": [
                {"status": "busy", "isPrivate": false,
                 "start": {"dateTime": "2026-11-18T10:00:00.0000000", "timeZone": "UTC"},
                 "end": {"dateTime": "2026-11-18T11:00:00.0000000", "timeZone": "UTC"},
                 "subject": "Review", "location": "Room 4B"},
                {"status": "oof", "isPrivate": true,
                 "start": {"dateTime": "2026-11-18T12:00:00.0000000", "timeZone": "UTC"},
                 "end": {"dateTime": "2026-11-18T13:00:00.0000000", "timeZone": "UTC"}}]},
            {"scheduleId": "nobody@contoso.example",
             "error": {"message": "No permission", "responseCode": "ErrorAccessDenied"}}
        ]}));
        assert_eq!(s.len(), 2);
        use Availability::*;
        assert_eq!(
            s[0].availability,
            [
                Free,
                Free,
                Tentative,
                Busy,
                Oof,
                WorkingElsewhere,
                Free,
                Unknown
            ]
        );
        assert_eq!(s[0].items.len(), 2);
        assert_eq!(s[0].items[0].status, ShowAs::Busy);
        assert_eq!(s[0].items[0].subject.as_deref(), Some("Review"));
        assert_eq!(s[0].items[0].end_ms - s[0].items[0].start_ms, 3_600_000);
        assert!(s[0].items[1].is_private && s[0].items[1].subject.is_none());
        let wh = s[0].working_hours.as_ref().unwrap();
        assert_eq!(wh.days.len(), 2);
        assert_eq!((wh.start_min, wh.end_min), (480, 960));
        assert_eq!(wh.time_zone, "Egypt Standard Time");
        assert!(s[1].working_hours.is_none());
        assert_eq!(s[1].error.as_deref(), Some("No permission"));
        assert!(s[1].availability.is_empty());
    }

    #[test]
    fn find_times_body_shape() {
        let q = FindTimes {
            required: vec![Person::new("Sara", "sara@contoso.example")],
            optional: vec![Person::new("", "omar@contoso.example")],
            duration_minutes: 90,
            window_start_ms: 1_794_996_000_000,
            window_end_ms: 1_795_082_400_000,
            max_candidates: 5,
            min_attendee_percentage: 100,
            work_hours_only: true,
        };
        let b = find_times_body(&q);
        assert_eq!(b["meetingDuration"], "PT1H30M");
        assert_eq!(b["timeConstraint"]["activityDomain"], "work");
        assert_eq!(
            b["timeConstraint"]["timeSlots"][0]["end"]["dateTime"],
            "2026-11-19T10:00:00"
        );
        assert_eq!(b["attendees"][1]["type"], "optional");
        assert_eq!(b["maxCandidates"], 5);
        assert_eq!(iso_duration(30), "PT30M");
        assert_eq!(iso_duration(120), "PT2H");
    }

    #[test]
    fn parses_suggestions_and_the_empty_reason() {
        let f = parse_found_times(&json!({
        "emptySuggestionsReason": "",
        "meetingTimeSuggestions": [{
            "confidence": 100.0, "organizerAvailability": "free",
            "suggestionReason": "Suggested because it is one of the nearest times",
            "meetingTimeSlot": {
                "start": {"dateTime": "2026-11-19T10:00:00.0000000", "timeZone": "UTC"},
                "end": {"dateTime": "2026-11-19T10:30:00.0000000", "timeZone": "UTC"}},
            "attendeeAvailability": [
                {"attendee": {"emailAddress": {"address": "sara@contoso.example"}}, "availability": "free"},
                {"attendee": {"emailAddress": {"address": "omar@contoso.example"}}, "availability": "tentative"}]
        }]}));
        assert_eq!(f.empty_reason, None);
        assert_eq!(f.suggestions.len(), 1);
        let s = &f.suggestions[0];
        assert_eq!(s.end_ms - s.start_ms, 1_800_000);
        assert_eq!(s.organizer, Availability::Free);
        assert_eq!(
            s.attendees[1],
            ("omar@contoso.example".to_string(), Availability::Tentative)
        );
        let none = parse_found_times(&json!({"meetingTimeSuggestions": [],
            "emptySuggestionsReason": "attendeesUnavailable"}));
        assert!(none.suggestions.is_empty());
        assert_eq!(none.empty_reason.as_deref(), Some("attendeesUnavailable"));
    }

    #[test]
    fn parses_find_rooms_answers() {
        let l = parse_found_room_lists(&json!({"value": [
            {"name": "Building 4", "address": "b4@contoso.example"},
            {"name": "no address"}]}));
        assert_eq!(
            l,
            [RoomList {
                id: "b4@contoso.example".into(),
                name: "Building 4".into(),
                email: "b4@contoso.example".into()
            }]
        );
        let r = parse_found_rooms(&json!({"value": [
            {"name": "Room 4B", "address": "room4b@contoso.example"}]}));
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, "room4b@contoso.example");
        assert_eq!(r[0].name, "Room 4B");
        assert_eq!(r[0].capacity, None);
        assert_eq!(
            find_rooms_url("b4@contoso.example"),
            "https://graph.microsoft.com/beta/me/findRooms(RoomList='b4%40contoso.example')"
        );
        assert_eq!(
            find_rooms_url(""),
            "https://graph.microsoft.com/beta/me/findRooms"
        );
    }

    #[test]
    fn free_between_reads_the_items() {
        let s = &parse_schedules(&json!({"value": [
            {"scheduleId": "room4b@contoso.example", "availabilityView": "0200",
             "scheduleItems": [
                {"status": "busy",
                 "start": {"dateTime": "2026-11-18T10:00:00", "timeZone": "UTC"},
                 "end": {"dateTime": "2026-11-18T11:00:00", "timeZone": "UTC"}},
                {"status": "free",
                 "start": {"dateTime": "2026-11-18T12:00:00", "timeZone": "UTC"},
                 "end": {"dateTime": "2026-11-18T13:00:00", "timeZone": "UTC"}}]},
            {"scheduleId": "x@contoso.example", "error": {"message": "nope"}}]}));
        let t = |h: &str| parse_iso_ms(&format!("2026-11-18T{h}:00Z")).unwrap();
        assert_eq!(free_between(&s[0], t("09:00"), t("10:00")), Some(true));
        assert_eq!(free_between(&s[0], t("10:30"), t("11:30")), Some(false));
        assert_eq!(free_between(&s[0], t("11:00"), t("13:00")), Some(true));
        assert_eq!(free_between(&s[1], t("09:00"), t("10:00")), None);
    }

    #[test]
    fn parses_room_lists_and_rooms() {
        let l = parse_room_lists(&json!({"value": [
            {"id": "l1", "displayName": "Building 4", "emailAddress": "b4@contoso.example"},
            {"id": "l2", "displayName": "no email"}]}));
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].email, "b4@contoso.example");
        let r = parse_rooms(&json!({"value": [
            {"id": "r1", "displayName": "Room 4B", "emailAddress": "room4b@contoso.example",
             "capacity": 8, "building": "4", "floorLabel": "Second", "floorNumber": 2,
             "isWheelChairAccessible": true, "tags": ["whiteboard"], "videoDeviceName": "Cam"},
            {"id": "r2", "emailAddress": "r2@contoso.example"}]}));
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].capacity, Some(8));
        assert_eq!(r[0].floor_number, Some(2));
        assert!(r[0].wheelchair_accessible && r[0].has_video);
        assert_eq!(r[0].tags, ["whiteboard"]);
        assert_eq!(r[1].capacity, None);
        assert!(!r[1].has_video);
    }
}
