//! Parsing of Graph calendarView and delta JSON, from sanitized, invented fixtures.

use calendar::{AttendeeKind, EventChange, Response, ShowAs, delta_link, next_link};
use calendar::{parse_delta_page, parse_events};
use serde_json::Value;

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

const T_2026_11_18: i64 = 1_794_960_000_000; // 2026-11-18T00:00:00Z

#[test]
fn parses_all_day_event() {
    let events = parse_events(&fixture("calendar_view_page1.json"));
    assert_eq!(events.len(), 5);
    let e = &events[0];
    assert_eq!(e.id, "AAA-allday");
    assert_eq!(e.ical_uid, "uid-allday");
    assert!(e.is_all_day);
    assert_eq!(e.start_ms, T_2026_11_18);
    assert_eq!(e.end_ms, T_2026_11_18 + 86_400_000);
    assert_eq!(e.show_as, ShowAs::Free);
    assert_eq!(e.response, Response::None);
    assert!(!e.is_recurring);
    assert_eq!(e.organizer_email, "alex@example.com");
    assert_eq!(e.join_url, None);
}

#[test]
fn parses_recurring_occurrence() {
    let events = parse_events(&fixture("calendar_view_page1.json"));
    let e = &events[1];
    assert!(e.is_recurring);
    assert_eq!(e.series_master_id.as_deref(), Some("AAA-master"));
    assert_eq!(e.start_ms, T_2026_11_18 + 9 * 3_600_000);
    assert_eq!(e.end_ms - e.start_ms, 15 * 60_000);
    assert_eq!(e.show_as, ShowAs::Busy);
    assert_eq!(e.response, Response::Accepted);
    assert_eq!(e.categories, vec!["Blue category", "Team"]);
    assert_eq!(e.location, "Room 4");
    assert_eq!(e.organizer_name, "Sam Sample");
    assert_eq!(e.body_preview, "Quick sync");
    assert_eq!(
        e.web_link,
        "https://outlook.example.com/calendar/item/AAA-occurrence"
    );
}

#[test]
fn parses_declined_private_and_tentative() {
    let events = parse_events(&fixture("calendar_view_page1.json"));
    assert_eq!(events[2].response, Response::Declined);
    assert!(!events[2].sensitivity_private);

    assert!(events[3].sensitivity_private);
    assert_eq!(events[3].show_as, ShowAs::Oof);
    assert_eq!(events[3].response, Response::Organizer);

    assert_eq!(events[4].show_as, ShowAs::Tentative);
    assert_eq!(events[4].response, Response::Tentative);
    // an exception to a series counts as recurring too
    assert!(events[4].is_recurring);
    assert_eq!(events[4].series_master_id, None);
}

#[test]
fn parses_online_meeting_with_attendees() {
    let events = parse_events(&fixture("calendar_view_page2.json"));
    assert_eq!(events.len(), 1, "an item without an id is skipped");
    let e = &events[0];
    assert!(e.is_online_meeting);
    assert_eq!(
        e.join_url.as_deref(),
        Some("https://teams.example.com/l/meetup-join/19%3ameeting_EXAMPLE%40thread.v2/0")
    );
    assert_eq!(e.response, Response::NotResponded);
    assert!(e.is_recurring, "series master");
    assert_eq!(e.attendees.len(), 3);
    assert_eq!(e.attendees[0].kind, AttendeeKind::Required);
    assert_eq!(e.attendees[0].response, Response::Accepted);
    assert_eq!(e.attendees[0].email, "sam@example.com");
    assert_eq!(e.attendees[1].kind, AttendeeKind::Optional);
    assert_eq!(e.attendees[1].response, Response::None);
    assert_eq!(e.attendees[2].kind, AttendeeKind::Resource);
    assert_eq!(e.attendees[2].response, Response::Tentative);
}

#[test]
fn unknown_values_do_not_fail() {
    let v = serde_json::json!({"value": [{
        "id": "x", "showAs": "somethingNew", "responseStatus": {"response": "weird"},
        "start": {"dateTime": "2026-11-18T09:00:00", "timeZone": "UTC"},
        "end": {"dateTime": "bad", "timeZone": "UTC"}
    }]});
    let e = &parse_events(&v)[0];
    assert_eq!(e.show_as, ShowAs::Unknown);
    assert_eq!(e.response, Response::None);
    assert_eq!(
        e.end_ms, e.start_ms,
        "an unreadable end falls back to the start"
    );
}

#[test]
fn follows_paging_links() {
    let p1 = fixture("calendar_view_page1.json");
    let p2 = fixture("calendar_view_page2.json");
    assert!(next_link(&p1).unwrap().contains("%24skip=200"));
    assert_eq!(delta_link(&p1), None);
    assert_eq!(next_link(&p2), None);
    assert!(delta_link(&p2).unwrap().contains("$deltatoken=abc"));
}

#[test]
fn parses_delta_upserts_and_removals() {
    let page = parse_delta_page(&fixture("calendar_delta.json"));
    assert_eq!(page.changes.len(), 3);
    match &page.changes[0] {
        EventChange::Upsert(e) => {
            assert_eq!(e.id, "AAA-changed");
            assert_eq!(e.start_ms, T_2026_11_18 + 2 * 86_400_000 + 8 * 3_600_000);
        }
        other => panic!("expected upsert, got {other:?}"),
    }
    assert_eq!(page.changes[1], EventChange::Removed("AAA-gone".into()));
    assert_eq!(
        page.changes[2],
        EventChange::Removed("AAA-out-of-range".into())
    );
    assert!(page.next_link.unwrap().contains("$skiptoken=xyz"));
    assert_eq!(page.delta_link, None);
}
