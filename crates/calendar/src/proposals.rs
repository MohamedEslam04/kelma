//! New time proposals, the organizer's side (C2.5): an attendee who answers tentative or
//! decline with `proposedNewTime` leaves it on the event's attendee, and the organizer's
//! mailbox gets an `eventMessageResponse` carrying the same time.

use reqwest::Method;
use serde_json::Value;
use teams_proto::Result;
use teams_proto::time::parse_iso_ms;

use crate::event::Response;
use crate::graph::{GraphRequest, enc, graph, wall_utc};
use crate::tz::date_time_zone_ms;

/// One attendee's proposed time for a meeting I organize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub name: String,
    pub email: String,
    /// `Tentative` or `Declined`: the answer the proposal came with.
    pub response: Response,
    /// The proposed slot, Unix milliseconds.
    pub start_ms: i64,
    pub end_ms: i64,
    /// The event the proposal is about, when known.
    pub event_id: Option<String>,
    /// The response message in my mailbox, when the proposal was read from there.
    pub message_id: Option<String>,
    /// The attendee's note (the message preview); empty from the event.
    pub comment: String,
    /// When the answer was sent, when known.
    pub sent_ms: Option<i64>,
}

/// A key looked up ignoring case, so Outlook REST (`ProposedNewTime`) reads too.
fn ci<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.as_object()?
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

fn ci_str(v: &Value, key: &str) -> String {
    ci(v, key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn non_empty(s: String) -> Option<String> {
    Some(s).filter(|s| !s.is_empty())
}

/// `(start, end)` of a `timeSlot`.
fn slot(v: &Value) -> Option<(i64, i64)> {
    let start = date_time_zone_ms(ci(v, "start")?)?;
    let end = date_time_zone_ms(ci(v, "end")?)?;
    (end > start).then_some((start, end))
}

fn response_named(s: &str) -> Response {
    match s.to_ascii_lowercase().as_str() {
        "tentativelyaccepted" | "meetingtentativelyaccepted" | "meetingtenativelyaccepted" => {
            Response::Tentative
        }
        "declined" | "meetingdeclined" => Response::Declined,
        "accepted" | "meetingaccepted" => Response::Accepted,
        _ => Response::None,
    }
}

/// The proposals on one event's attendees (`GET /me/events/{id}`).
pub fn parse_event_proposals(event: &Value) -> Vec<Proposal> {
    let event_id = non_empty(ci_str(event, "id"));
    ci(event, "attendees")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|a| {
                    let (start_ms, end_ms) = slot(ci(a, "proposedNewTime")?)?;
                    let addr = ci(a, "emailAddress").unwrap_or(&Value::Null);
                    let status = ci(a, "status").unwrap_or(&Value::Null);
                    Some(Proposal {
                        name: ci_str(addr, "name"),
                        email: ci_str(addr, "address"),
                        response: response_named(&ci_str(status, "response")),
                        start_ms,
                        end_ms,
                        event_id: event_id.clone(),
                        message_id: None,
                        comment: String::new(),
                        sent_ms: parse_iso_ms(&ci_str(status, "time")),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The proposals among meeting answers in my mailbox (`GET /me/messages`, see
/// [`proposal_messages_path`]). Answers without a proposed time are skipped.
pub fn parse_proposal_messages(v: &Value) -> Vec<Proposal> {
    ci(v, "value")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|m| {
                    let (start_ms, end_ms) = slot(ci(m, "proposedNewTime")?)?;
                    let from = ci(m, "from")
                        .or_else(|| ci(m, "sender"))
                        .and_then(|f| ci(f, "emailAddress"))
                        .unwrap_or(&Value::Null);
                    let mut response = response_named(&ci_str(m, "responseType"));
                    if response == Response::None {
                        response = response_named(&ci_str(m, "meetingMessageType"));
                    }
                    Some(Proposal {
                        name: ci_str(from, "name"),
                        email: ci_str(from, "address"),
                        response,
                        start_ms,
                        end_ms,
                        event_id: ci(m, "event").and_then(|e| non_empty(ci_str(e, "id"))),
                        message_id: non_empty(ci_str(m, "id")),
                        comment: ci_str(m, "bodyPreview"),
                        sent_ms: parse_iso_ms(&ci_str(m, "sentDateTime")),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `GET /me/events/{id}` with only what the proposals need.
pub fn event_proposals_path(event_id: &str) -> String {
    format!(
        "/me/events/{}?$select=id,subject,allowNewTimeProposals,attendees",
        enc(event_id)
    )
}

/// The newest `top` tentative and decline answers in my mailbox, with the proposal fields and
/// the event's id. Graph's v1.0 enum spells the tentative value `meetingTenativelyAccepted`
/// (`meetingTentativelyAccepted` is a 400, checked live). Adding `$orderby` to this filter is
/// refused (`InefficientFilter`), so [`proposal_messages`] sorts.
pub fn proposal_messages_path(top: u32) -> String {
    let kind = "microsoft.graph.eventMessage/meetingMessageType";
    let value = |v: &str| format!("{kind}%20eq%20microsoft.graph.meetingMessageType'{v}'");
    format!(
        "/me/messages?$top={top}\
         &$filter={}%20or%20{}\
         &$select=subject,from,sentDateTime,bodyPreview,{kind},\
         microsoft.graph.eventMessageResponse/responseType,\
         microsoft.graph.eventMessageResponse/proposedNewTime\
         &$expand=microsoft.graph.eventMessage/event($select=id)",
        value("meetingTenativelyAccepted"),
        value("meetingDeclined"),
    )
}

/// The times attendees proposed for one of my events.
pub async fn event_proposals(
    client: &teams_proto::Client,
    event_id: &str,
) -> Result<Vec<Proposal>> {
    let v = graph(
        client,
        "event proposals",
        Method::GET,
        &event_proposals_path(event_id),
        None,
    )
    .await?;
    Ok(parse_event_proposals(&v))
}

/// The proposals in my mailbox's newest `top` tentative or decline answers, newest first.
pub async fn proposal_messages(client: &teams_proto::Client, top: u32) -> Result<Vec<Proposal>> {
    let v = graph(
        client,
        "proposal messages",
        Method::GET,
        &proposal_messages_path(top),
        None,
    )
    .await?;
    let mut out = parse_proposal_messages(&v);
    out.sort_by_key(|p| std::cmp::Reverse(p.sent_ms));
    Ok(out)
}

/// Accepting a proposal moves the event to the proposed slot: the same `PATCH` as
/// [`crate::move_event`] with the proposal's times in UTC. Built here so it can be shown
/// (`--dry-run`) before it is sent.
pub fn accept_proposal_request(event_id: &str, p: &Proposal) -> GraphRequest {
    GraphRequest {
        method: Method::PATCH,
        path: format!("/me/events/{}", enc(event_id)),
        body: Some(crate::write::move_body(
            &wall_utc(p.start_ms),
            &wall_utc(p.end_ms),
            "UTC",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ms(s: &str) -> i64 {
        parse_iso_ms(s).unwrap()
    }

    #[test]
    fn reads_proposals_on_event_attendees() {
        let p = parse_event_proposals(&json!({
            "id": "AAMk=1", "subject": "Lunch", "allowNewTimeProposals": true,
            "attendees": [
                {"type": "required",
                 "status": {"response": "tentativelyAccepted", "time": "2026-11-10T07:06:24.5046431Z"},
                 "proposedNewTime": {
                    "start": {"dateTime": "2026-11-16T12:00:00.0000000", "timeZone": "UTC"},
                    "end": {"dateTime": "2026-11-16T14:00:00.0000000", "timeZone": "UTC"}},
                 "emailAddress": {"name": "Sara", "address": "sara@contoso.example"}},
                {"type": "required", "status": {"response": "accepted"},
                 "emailAddress": {"name": "Omar", "address": "omar@contoso.example"}},
                {"type": "optional", "status": {"response": "declined"},
                 "proposedNewTime": {
                    "start": {"dateTime": "2026-11-16T12:00:00", "timeZone": "Pacific Standard Time"},
                    "end": {"dateTime": "2026-11-16T13:00:00", "timeZone": "Pacific Standard Time"}},
                 "emailAddress": {"name": "Lina", "address": "lina@contoso.example"}}]}));
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].email, "sara@contoso.example");
        assert_eq!(p[0].response, Response::Tentative);
        assert_eq!(
            (p[0].start_ms, p[0].end_ms),
            (ms("2026-11-16T12:00:00Z"), ms("2026-11-16T14:00:00Z"))
        );
        assert_eq!(p[0].event_id.as_deref(), Some("AAMk=1"));
        assert_eq!(p[0].sent_ms, Some(ms("2026-11-10T07:06:24.504Z")));
        assert_eq!(p[0].message_id, None);
        // A proposal in another zone becomes UTC: 12:00 PST is 20:00 UTC.
        assert_eq!(p[1].response, Response::Declined);
        assert_eq!(p[1].start_ms, ms("2026-11-16T20:00:00Z"));
        assert!(parse_event_proposals(&json!({})).is_empty());
    }

    #[test]
    fn reads_proposals_from_meeting_answers() {
        let p = parse_proposal_messages(&json!({"value": [
            {"@odata.type": "#microsoft.graph.eventMessageResponse", "id": "msg1",
             "subject": "New Time Proposed: Lunch", "bodyPreview": "Can we do the next day?",
             "sentDateTime": "2026-11-10T07:06:24Z",
             "meetingMessageType": "meetingTenativelyAccepted", "responseType": "tentativelyAccepted",
             "from": {"emailAddress": {"name": "Sara", "address": "sara@contoso.example"}},
             "event": {"id": "AAMk=1"},
             "proposedNewTime": {
                "start": {"dateTime": "2026-11-17T12:00:00", "timeZone": "UTC"},
                "end": {"dateTime": "2026-11-17T14:00:00", "timeZone": "UTC"}}},
            {"@odata.type": "#microsoft.graph.eventMessageResponse", "id": "msg2",
             "meetingMessageType": "meetingDeclined", "responseType": "declined",
             "from": {"emailAddress": {"address": "omar@contoso.example"}}},
            {"Id": "msg3", "MeetingMessageType": "MeetingDeclined",
             "From": {"EmailAddress": {"Address": "lina@contoso.example"}},
             "ProposedNewTime": {
                "Start": {"DateTime": "2026-11-18T09:00:00", "TimeZone": "UTC"},
                "End": {"DateTime": "2026-11-18T09:30:00", "TimeZone": "UTC"}}}]}));
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].response, Response::Tentative);
        assert_eq!(p[0].message_id.as_deref(), Some("msg1"));
        assert_eq!(p[0].event_id.as_deref(), Some("AAMk=1"));
        assert_eq!(p[0].comment, "Can we do the next day?");
        assert_eq!(p[0].sent_ms, Some(ms("2026-11-10T07:06:24Z")));
        assert_eq!(p[0].end_ms - p[0].start_ms, 7_200_000);
        // Outlook REST PascalCase, and the type from meetingMessageType alone.
        assert_eq!(p[1].email, "lina@contoso.example");
        assert_eq!(p[1].response, Response::Declined);
        assert_eq!(p[1].event_id, None);
    }

    #[test]
    fn a_slot_that_ends_before_it_starts_is_ignored() {
        let p = parse_event_proposals(&json!({"attendees": [{
            "proposedNewTime": {
                "start": {"dateTime": "2026-11-16T14:00:00", "timeZone": "UTC"},
                "end": {"dateTime": "2026-11-16T12:00:00", "timeZone": "UTC"}},
            "emailAddress": {"address": "sara@contoso.example"}}]}));
        assert!(p.is_empty());
    }

    #[test]
    fn request_paths() {
        assert_eq!(
            event_proposals_path("a=b"),
            "/me/events/a%3Db?$select=id,subject,allowNewTimeProposals,attendees"
        );
        let m = proposal_messages_path(25);
        assert!(m.starts_with("/me/messages?$top=25&"));
        assert!(m.contains("meetingMessageType'meetingTenativelyAccepted'"));
        assert!(m.contains("meetingMessageType'meetingDeclined'"));
        assert!(m.contains("eventMessageResponse/proposedNewTime"));
        assert!(m.contains("$expand=microsoft.graph.eventMessage/event($select=id)"));
        assert!(!m.contains(' '));
        assert!(!m.contains("$orderby"));
    }

    #[test]
    fn accepting_moves_the_event() {
        let p = Proposal {
            name: String::new(),
            email: "sara@contoso.example".into(),
            response: Response::Tentative,
            start_ms: ms("2026-11-17T12:00:00Z"),
            end_ms: ms("2026-11-17T14:00:00Z"),
            event_id: Some("e/1".into()),
            message_id: None,
            comment: String::new(),
            sent_ms: None,
        };
        let r = accept_proposal_request("e/1", &p);
        assert_eq!(r.method, Method::PATCH);
        assert_eq!(r.path, "/me/events/e%2F1");
        assert_eq!(
            r.body,
            Some(json!({
                "start": {"dateTime": "2026-11-17T12:00:00", "timeZone": "UTC"},
                "end": {"dateTime": "2026-11-17T14:00:00", "timeZone": "UTC"}}))
        );
    }
}
