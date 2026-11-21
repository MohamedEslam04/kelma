//! Exporting events as an iCalendar (`.ics`) file (C3.3 Print and Share): RFC 5545 text that
//! any calendar app opens, and that a print dialog can lay out.

use crate::event::{Event, ShowAs};
use crate::range::{Date, format_iso_ms};

/// `20261218T100000Z` from Unix milliseconds.
fn utc_stamp(ms: i64) -> String {
    let s = format_iso_ms(ms);
    format!(
        "{}{}{}T{}{}{}Z",
        &s[0..4],
        &s[5..7],
        &s[8..10],
        &s[11..13],
        &s[14..16],
        &s[17..19]
    )
}

fn date_stamp(ms: i64) -> String {
    let d = Date::from_ms(ms);
    format!("{:04}{:02}{:02}", d.year, d.month, d.day)
}

/// Escapes text values: backslash, semicolon, comma and newlines.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Folds a content line to 75 octets, continuing with a space, never inside a character.
fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut len = 0;
    for c in line.chars() {
        let n = c.len_utf8();
        if len + n > 75 {
            out.push_str("\r\n ");
            len = 1;
        }
        out.push(c);
        len += n;
    }
    out
}

fn push(out: &mut String, line: String) {
    out.push_str(&fold(&line));
    out.push_str("\r\n");
}

/// The events as one `VCALENDAR`. `now_ms` stamps `DTSTAMP`, passed in so output is stable.
/// Cancelled events are left out.
pub fn to_ics(events: &[Event], now_ms: i64) -> String {
    let mut out = String::new();
    push(&mut out, "BEGIN:VCALENDAR".into());
    push(&mut out, "VERSION:2.0".into());
    push(&mut out, "PRODID:-//ironteams//calendar//EN".into());
    push(&mut out, "CALSCALE:GREGORIAN".into());
    for e in events.iter().filter(|e| !e.is_cancelled) {
        push(&mut out, "BEGIN:VEVENT".into());
        let uid = if e.ical_uid.is_empty() {
            &e.id
        } else {
            &e.ical_uid
        };
        // An occurrence of a series shares the series' UID, so its start makes it unique.
        push(&mut out, format!("UID:{}-{}", escape(uid), e.start_ms));
        push(&mut out, format!("DTSTAMP:{}", utc_stamp(now_ms)));
        if e.is_all_day {
            push(
                &mut out,
                format!("DTSTART;VALUE=DATE:{}", date_stamp(e.start_ms)),
            );
            push(
                &mut out,
                format!("DTEND;VALUE=DATE:{}", date_stamp(e.end_ms)),
            );
        } else {
            push(&mut out, format!("DTSTART:{}", utc_stamp(e.start_ms)));
            push(&mut out, format!("DTEND:{}", utc_stamp(e.end_ms)));
        }
        push(&mut out, format!("SUMMARY:{}", escape(&e.subject)));
        if !e.location.is_empty() {
            push(&mut out, format!("LOCATION:{}", escape(&e.location)));
        }
        if !e.body_preview.is_empty() {
            push(&mut out, format!("DESCRIPTION:{}", escape(&e.body_preview)));
        }
        if !e.categories.is_empty() {
            let cats: Vec<String> = e.categories.iter().map(|c| escape(c)).collect();
            push(&mut out, format!("CATEGORIES:{}", cats.join(",")));
        }
        if !e.organizer_email.is_empty() {
            push(
                &mut out,
                format!(
                    "ORGANIZER;CN={}:mailto:{}",
                    escape(&e.organizer_name).replace(':', ""),
                    e.organizer_email
                ),
            );
        }
        push(
            &mut out,
            format!(
                "STATUS:{}",
                if e.show_as == ShowAs::Tentative {
                    "TENTATIVE"
                } else {
                    "CONFIRMED"
                }
            ),
        );
        push(
            &mut out,
            format!(
                "TRANSP:{}",
                if matches!(e.show_as, ShowAs::Free | ShowAs::WorkingElsewhere) {
                    "TRANSPARENT"
                } else {
                    "OPAQUE"
                }
            ),
        );
        if e.sensitivity_private {
            push(&mut out, "CLASS:PRIVATE".into());
        }
        if let Some(url) = &e.join_url {
            push(&mut out, format!("URL:{url}"));
        }
        push(&mut out, "END:VEVENT".into());
    }
    push(&mut out, "END:VCALENDAR".into());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(items: serde_json::Value) -> Vec<Event> {
        crate::parse_events(&json!({"value": items}))
    }

    #[test]
    fn writes_a_timed_event() {
        let events = ev(json!([{
            "id": "A1", "iCalUId": "uid-1", "subject": "Plan; review, v2",
            "start": {"dateTime": "2026-12-18T10:00:00.0000000", "timeZone": "UTC"},
            "end": {"dateTime": "2026-12-18T10:30:00.0000000", "timeZone": "UTC"},
            "showAs": "tentative", "sensitivity": "private", "categories": ["Blue", "Red"],
            "location": {"displayName": "Room 4B"}, "bodyPreview": "line1\nline2",
            "organizer": {"emailAddress": {"name": "Sara Hassan", "address": "sara@contoso.example"}},
            "onlineMeeting": {"joinUrl": "https://teams.microsoft.com/l/meetup-join/x"}
        }]));
        let ics = to_ics(&events, 1_794_960_000_000);
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n"));
        assert!(ics.ends_with("END:VEVENT\r\nEND:VCALENDAR\r\n"));
        for line in [
            "UID:uid-1-1797588000000",
            "DTSTAMP:20261118T000000Z",
            "DTSTART:20261218T100000Z",
            "DTEND:20261218T103000Z",
            "SUMMARY:Plan\\; review\\, v2",
            "LOCATION:Room 4B",
            "DESCRIPTION:line1\\nline2",
            "CATEGORIES:Blue,Red",
            "ORGANIZER;CN=Sara Hassan:mailto:sara@contoso.example",
            "STATUS:TENTATIVE",
            "TRANSP:OPAQUE",
            "CLASS:PRIVATE",
            "URL:https://teams.microsoft.com/l/meetup-join/x",
        ] {
            assert!(
                ics.contains(&format!("{line}\r\n")),
                "missing {line}\n{ics}"
            );
        }
    }

    #[test]
    fn all_day_uses_dates_and_free_is_transparent() {
        let events = ev(json!([{
            "id": "B", "subject": "Holiday", "isAllDay": true, "showAs": "free",
            "start": {"dateTime": "2026-12-25T00:00:00.0000000", "timeZone": "UTC"},
            "end": {"dateTime": "2026-12-26T00:00:00.0000000", "timeZone": "UTC"}}]));
        let ics = to_ics(&events, 0);
        assert!(ics.contains("DTSTART;VALUE=DATE:20261225\r\n"));
        assert!(ics.contains("DTEND;VALUE=DATE:20261226\r\n"));
        assert!(ics.contains("TRANSP:TRANSPARENT\r\n"));
        assert!(ics.contains("UID:B-"), "falls back to the id");
    }

    #[test]
    fn cancelled_events_are_skipped_and_long_lines_fold() {
        let long = "x".repeat(200);
        let events = ev(json!([
            {"id": "C", "subject": "gone", "isCancelled": true},
            {"id": "D", "subject": long}]));
        let ics = to_ics(&events, 0);
        assert!(!ics.contains("gone"));
        assert_eq!(ics.matches("BEGIN:VEVENT").count(), 1);
        for line in ics.split("\r\n") {
            assert!(line.len() <= 75, "unfolded line of {} bytes", line.len());
        }
        let unfolded = ics.replace("\r\n ", "");
        assert!(unfolded.contains(&format!("SUMMARY:{long}")));
    }

    #[test]
    fn folding_never_splits_a_character() {
        let events = ev(json!([{"id": "E", "subject": "مرحبا ".repeat(30)}]));
        let ics = to_ics(&events, 0);
        assert!(String::from_utf8(ics.clone().into_bytes()).is_ok());
        for line in ics.split("\r\n") {
            assert!(line.len() <= 75);
        }
    }
}
