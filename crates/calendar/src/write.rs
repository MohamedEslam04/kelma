//! Changing events over Graph: create, update, move, RSVP, cancel, delete and forward
//! (C1.3, C2.1, C2.2, C2.5). The bodies are built by pure functions so tests cover them
//! without a network.

use reqwest::Method;
use serde_json::{Value, json};
use teams_proto::{Error, Result};

use crate::event::{Event, parse_event};
use crate::graph::{date_time, email_address, enc, graph, wall_utc};
use crate::new_event::{NewEvent, Person};

/// The answer to an invitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rsvp {
    Accept,
    Tentative,
    Decline,
}

impl Rsvp {
    /// The Graph action segment: `accept`, `tentativelyAccept` or `decline`.
    pub fn action(self) -> &'static str {
        match self {
            Rsvp::Accept => "accept",
            Rsvp::Tentative => "tentativelyAccept",
            Rsvp::Decline => "decline",
        }
    }
}

/// The body of an RSVP. `proposed_new_time` is UTC milliseconds `(start, end)` and is only
/// allowed on tentative and decline (Graph refuses it on accept).
pub fn respond_body(
    rsvp: Rsvp,
    comment: &str,
    send_response: bool,
    proposed_new_time: Option<(i64, i64)>,
) -> Result<Value> {
    let mut b = json!({"comment": comment, "sendResponse": send_response});
    if let Some((start, end)) = proposed_new_time {
        if rsvp == Rsvp::Accept {
            return Err(Error::Protocol(
                "a new time can only be proposed with tentative or decline".into(),
            ));
        }
        if end <= start {
            return Err(Error::Protocol(
                "the proposed time ends before it starts".into(),
            ));
        }
        b["proposedNewTime"] = json!({
            "start": date_time(&wall_utc(start), "UTC"),
            "end": date_time(&wall_utc(end), "UTC"),
        });
    }
    Ok(b)
}

pub fn cancel_body(comment: &str) -> Value {
    json!({"comment": comment})
}

pub fn forward_body(recipients: &[Person], comment: &str) -> Value {
    json!({
        "comment": comment,
        "toRecipients": recipients
            .iter()
            .map(|p| json!({"emailAddress": email_address(&p.name, &p.email)}))
            .collect::<Vec<_>>(),
    })
}

/// A drag-to-move or drag-to-resize: only the times change.
pub fn move_body(start: &str, end: &str, tz: &str) -> Value {
    json!({"start": date_time(start, tz), "end": date_time(end, tz)})
}

/// The id to cancel or delete. A series is the master's id, one occurrence is its own id.
pub fn target_id(event: &Event, whole_series: bool) -> &str {
    match (&event.series_master_id, whole_series) {
        (Some(master), true) => master,
        _ => &event.id,
    }
}

fn path(id: &str, suffix: &str) -> String {
    format!("/me/events/{}{suffix}", enc(id))
}

/// Creates the event. A `transactionId` is added when the caller set none, so a retry of this
/// call (not of a new one) never makes a duplicate.
pub async fn create_event(client: &teams_proto::Client, new: &NewEvent) -> Result<Event> {
    new.validate()?;
    let mut new = new.clone();
    if new.transaction_id.is_none() {
        new.transaction_id = Some(teams_proto::client::new_epid());
    }
    let v = graph(
        client,
        "create event",
        Method::POST,
        "/me/events",
        Some(&new.create_body()),
    )
    .await?;
    parse_event(&v).ok_or_else(|| Error::Protocol("create event: no id in the response".into()))
}

/// Replaces the event's fields. Attendees are replaced as a list, so pass everyone.
pub async fn update_event(client: &teams_proto::Client, id: &str, new: &NewEvent) -> Result<Event> {
    new.validate()?;
    let v = graph(
        client,
        "update event",
        Method::PATCH,
        &path(id, ""),
        Some(&new.update_body()),
    )
    .await?;
    parse_event(&v).ok_or_else(|| Error::Protocol("update event: no id in the response".into()))
}

/// Moves or resizes an event, leaving everything else alone.
pub async fn move_event(
    client: &teams_proto::Client,
    id: &str,
    start: &str,
    end: &str,
    tz: &str,
) -> Result<Event> {
    let v = graph(
        client,
        "move event",
        Method::PATCH,
        &path(id, ""),
        Some(&move_body(start, end, tz)),
    )
    .await?;
    parse_event(&v).ok_or_else(|| Error::Protocol("move event: no id in the response".into()))
}

/// Answers an invitation. With `send_response` the organizer gets the reply.
pub async fn respond(
    client: &teams_proto::Client,
    id: &str,
    rsvp: Rsvp,
    comment: &str,
    send_response: bool,
    proposed_new_time: Option<(i64, i64)>,
) -> Result<()> {
    let body = respond_body(rsvp, comment, send_response, proposed_new_time)?;
    graph(
        client,
        "respond to event",
        Method::POST,
        &path(id, &format!("/{}", rsvp.action())),
        Some(&body),
    )
    .await?;
    Ok(())
}

/// Cancels a meeting you organize and tells the attendees. Pass the series master's id for the
/// whole series (see [`target_id`]).
pub async fn cancel_event(client: &teams_proto::Client, id: &str, comment: &str) -> Result<()> {
    graph(
        client,
        "cancel event",
        Method::POST,
        &path(id, "/cancel"),
        Some(&cancel_body(comment)),
    )
    .await?;
    Ok(())
}

/// Deletes an event without telling anyone (an appointment, or an invite you decline to keep).
pub async fn delete_event(client: &teams_proto::Client, id: &str) -> Result<()> {
    graph(client, "delete event", Method::DELETE, &path(id, ""), None).await?;
    Ok(())
}

pub async fn forward_event(
    client: &teams_proto::Client,
    id: &str,
    recipients: &[Person],
    comment: &str,
) -> Result<()> {
    if recipients.is_empty() {
        return Err(Error::Protocol("forward: no recipients".into()));
    }
    graph(
        client,
        "forward event",
        Method::POST,
        &path(id, "/forward"),
        Some(&forward_body(recipients, comment)),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rsvp_actions() {
        assert_eq!(Rsvp::Accept.action(), "accept");
        assert_eq!(Rsvp::Tentative.action(), "tentativelyAccept");
        assert_eq!(Rsvp::Decline.action(), "decline");
    }

    #[test]
    fn respond_body_with_and_without_a_proposed_time() {
        assert_eq!(
            respond_body(Rsvp::Accept, "see you", true, None).unwrap(),
            json!({"comment": "see you", "sendResponse": true})
        );
        let b = respond_body(
            Rsvp::Decline,
            "",
            false,
            Some((1_794_996_000_000, 1_794_999_600_000)),
        )
        .unwrap();
        assert_eq!(b["sendResponse"], false);
        assert_eq!(
            b["proposedNewTime"],
            json!({"start": {"dateTime": "2026-11-18T10:00:00", "timeZone": "UTC"},
                   "end": {"dateTime": "2026-11-18T11:00:00", "timeZone": "UTC"}})
        );
        assert!(respond_body(Rsvp::Tentative, "", true, Some((2, 1))).is_err());
    }

    #[test]
    fn accept_cannot_propose_a_new_time() {
        assert!(respond_body(Rsvp::Accept, "", true, Some((1, 2))).is_err());
    }

    #[test]
    fn cancel_forward_and_move_bodies() {
        assert_eq!(cancel_body("moved"), json!({"comment": "moved"}));
        let b = forward_body(&[Person::new("Sara", "sara@contoso.example")], "fyi");
        assert_eq!(
            b,
            json!({"comment": "fyi", "toRecipients": [
                {"emailAddress": {"address": "sara@contoso.example", "name": "Sara"}}]})
        );
        assert_eq!(
            move_body("2026-12-18T11:00:00", "2026-12-18T11:30:00", "UTC"),
            json!({"start": {"dateTime": "2026-12-18T11:00:00", "timeZone": "UTC"},
                   "end": {"dateTime": "2026-12-18T11:30:00", "timeZone": "UTC"}})
        );
    }

    #[test]
    fn series_or_occurrence_target() {
        let ev = crate::parse_events(&json!({"value": [
            {"id": "occ", "seriesMasterId": "master", "type": "occurrence"}]}))
        .remove(0);
        assert_eq!(target_id(&ev, true), "master");
        assert_eq!(target_id(&ev, false), "occ");
        let single = crate::parse_events(&json!({"value": [{"id": "one"}]})).remove(0);
        assert_eq!(target_id(&single, true), "one");
    }

    #[test]
    fn ids_are_encoded_in_the_path() {
        assert_eq!(path("AA=/+", "/cancel"), "/me/events/AA%3D%2F%2B/cancel");
    }
}
