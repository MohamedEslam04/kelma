//! A meeting's content-sharing SESSION: the one screen a meeting shows, and who presents it.
//!
//! Facts only (no code copied; teams-lite has no licence file): teams-lite `NATIVE-CALLING.md`
//! § 10.4 and its measured notes of 2026-08-06 / 2026-09-04. Everything here is UNVERIFIED
//! against kelma's tenant.
//!
//! - Sending a screen in a meeting is its own `contentSharing` object, asked for BEFORE the
//!   sharing line is switched on: `POST <addModality>` (a conversation link) with
//!   `{participants {from, to: []}, contentSharing {identifier, subject: null, sessionState:
//!   null, sequenceNumber: 1, links {sessionUpdate, sessionEnd}}, links {addModalitySuccess,
//!   addModalityFailure}}` and a `ContentSharingCorrelationId` header.
//! - The POST answers `{}`; the grant arrives on a callback (`addModalitySuccess`, or the
//!   session's own `contentSharingUpdate`), carrying the session's six links:
//!   `contentSharingController`, `takeControl`, `updateSessionState`, `sync`,
//!   `notificationLinks`, `leave` (some spellings carry a `contentSharing` prefix).
//! - Giving the session back is `POST <session leave>` with `{participants {from},
//!   contentSharing {links {sessionUpdate, sessionEnd}}}`. The session's `leave` must never be
//!   merged into the call's links: stopping a share would hang the call up.
//! - A participant who presents may carry a `contentSharing` object in the roster (the client's
//!   shape; teams-lite's tenant never showed one, so the stream direction stays the main test).

use super::CallSession;
use crate::types::Links;
use serde_json::{Map, Value, json};

/// The session's own callbacks (`/conversation/contentSharingUpdate/`, `.../contentSharingEnd/`).
pub const SESSION_UPDATE: &str = "contentSharingUpdate";
pub const SESSION_END: &str = "contentSharingEnd";
/// The header every request of one session carries.
pub const CORRELATION_HEADER: &str = "ContentSharingCorrelationId";
/// Where the conversation answers an `addModality` ask.
pub const ADD_MODALITY_SUCCESS: &str = "addModalitySuccess";
pub const ADD_MODALITY_FAILURE: &str = "addModalityFailure";
const PREFIX: &str = "contentSharing";

/// A fresh id for a session's correlation id or its `identifier`.
pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn session_callbacks(s: &CallSession) -> Value {
    json!({
        "sessionUpdate": s.callbacks.conversation(SESSION_UPDATE),
        "sessionEnd": s.callbacks.conversation(SESSION_END),
    })
}

/// `POST <addModality>`: make this endpoint the presenter of the meeting's content-sharing
/// session. `subject` and `sessionState` are null and `sequenceNumber` is 1, as the web client
/// sends on a start.
pub fn start_body(s: &CallSession, identifier: &str) -> Value {
    json!({
        "participants": {"from": s.me.wire(), "to": []},
        "contentSharing": {
            "identifier": identifier,
            "subject": null,
            "sessionState": null,
            "sequenceNumber": 1,
            "links": session_callbacks(s),
        },
        "links": s.callbacks.conversation_links(&[ADD_MODALITY_SUCCESS, ADD_MODALITY_FAILURE]),
    })
}

/// `POST <session leave>`: give the session back.
pub fn leave_body(s: &CallSession) -> Value {
    json!({
        "participants": {"from": s.me.wire()},
        "contentSharing": {"links": session_callbacks(s)},
    })
}

/// The content-sharing session a meeting granted us, with its own links (never the call's).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SharingSession {
    /// The id we minted; every request of the session carries it ([`CORRELATION_HEADER`]).
    pub correlation_id: String,
    /// The service's own id, once it names one (`contentSharingSessionId`).
    pub session_id: Option<String>,
    /// The session's links, by their short name (`leave`, `takeControl`, ...).
    pub links: Links,
}

impl SharingSession {
    /// Reads a grant out of the `addModality` answer or a session callback. `None` when the body
    /// names no way to give the session back: a share that cannot be stopped is not one we hold.
    ///
    /// The links are the DEEPEST `links` object of the body: an answer may also carry the call's
    /// own `links` (with the call's `leave`) above the session's.
    pub fn read(correlation_id: &str, body: &Value) -> Option<Self> {
        let links = deepest_links(body, 0)?.1;
        let links = short_names(links);
        links.get("leave")?;
        Some(SharingSession {
            correlation_id: correlation_id.to_owned(),
            session_id: session_id_in(body),
            links,
        })
    }

    pub fn leave(&self) -> Option<&str> {
        self.links.get("leave")
    }

    pub fn take_control(&self) -> Option<&str> {
        self.links.get("takeControl")
    }

    pub fn update_session_state(&self) -> Option<&str> {
        self.links.get("updateSessionState")
    }
}

/// The `links` object deepest in `v`, with its depth (the first one at that depth).
fn deepest_links(v: &Value, depth: usize) -> Option<(usize, &Map<String, Value>)> {
    type Found<'a> = Option<(usize, &'a Map<String, Value>)>;
    fn keep<'a>(best: &mut Found<'a>, c: Found<'a>) {
        if let Some(c) = c
            && best.is_none_or(|b| c.0 > b.0)
        {
            *best = Some(c);
        }
    }
    let mut best: Found<'_> = None;
    match v {
        Value::Object(m) => {
            if let Some(l) = m.get("links").and_then(Value::as_object)
                && l.values().any(Value::is_string)
            {
                keep(&mut best, Some((depth, l)));
            }
            for (k, child) in m {
                if k != "links" {
                    keep(&mut best, deepest_links(child, depth + 1));
                }
            }
        }
        Value::Array(a) => {
            for child in a {
                keep(&mut best, deepest_links(child, depth + 1));
            }
        }
        _ => {}
    }
    best
}

/// String links by name, with `contentSharingLeave` also known as `leave` (and so on). The
/// session's controller keeps its full name.
fn short_names(m: &Map<String, Value>) -> Links {
    let mut out = Links::default();
    for (k, v) in m {
        let Some(url) = v.as_str() else { continue };
        out.0.insert(k.clone(), url.to_owned());
        if let Some(rest) = k.strip_prefix(PREFIX)
            && let Some(first) = rest.chars().next()
            && k != "contentSharingController"
        {
            let short = format!(
                "{}{}",
                first.to_ascii_lowercase(),
                &rest[first.len_utf8()..]
            );
            out.0.entry(short).or_insert_with(|| url.to_owned());
        }
    }
    out
}

/// `contentSharingSessionId` at any depth, else `contentSharing.sessionId`.
fn session_id_in(v: &Value) -> Option<String> {
    fn find<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
        match v {
            Value::Object(m) => m
                .get(key)
                .and_then(Value::as_str)
                .or_else(|| m.values().find_map(|c| find(c, key))),
            Value::Array(a) => a.iter().find_map(|c| find(c, key)),
            _ => None,
        }
    }
    find(v, "contentSharingSessionId")
        .or_else(|| {
            v.pointer("/contentSharing/sessionId")
                .and_then(Value::as_str)
        })
        .map(str::to_owned)
}

/// Whether a callback (by its event name) is about the content-sharing session.
pub fn is_session_event(event: &str) -> bool {
    matches!(
        event,
        SESSION_UPDATE | SESSION_END | ADD_MODALITY_SUCCESS | ADD_MODALITY_FAILURE
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> CallSession {
        CallSession::new("https://trouter.example.invalid/x/", "8:orgid:me", "Me")
    }

    #[test]
    fn the_start_asks_to_present_with_the_sessions_own_callbacks() {
        let s = session();
        let b = start_body(&s, "share-1");
        assert_eq!(b["contentSharing"]["identifier"], "share-1");
        assert_eq!(b["contentSharing"]["subject"], Value::Null);
        assert_eq!(b["contentSharing"]["sessionState"], Value::Null);
        assert_eq!(b["contentSharing"]["sequenceNumber"], 1);
        assert_eq!(b["participants"]["to"], json!([]));
        assert_eq!(b["participants"]["from"]["id"], "8:orgid:me");
        let update = b["contentSharing"]["links"]["sessionUpdate"]
            .as_str()
            .unwrap();
        assert!(update.ends_with("/conversation/contentSharingUpdate/"));
        let end = b["contentSharing"]["links"]["sessionEnd"].as_str().unwrap();
        assert!(end.ends_with("/conversation/contentSharingEnd/"));
        assert!(
            b["links"]["addModalitySuccess"]
                .as_str()
                .unwrap()
                .ends_with("/conversation/addModalitySuccess/")
        );
        let leave = leave_body(&s);
        assert_eq!(leave["contentSharing"]["links"]["sessionEnd"], end);
        assert!(leave["participants"].get("to").is_none());
    }

    #[test]
    fn the_sessions_leave_is_never_the_calls() {
        let answer = json!({
            "contentSharing": {
                "sessionId": "cs-1",
                "links": {"leave": "https://x/cs-leave", "takeControl": "https://x/tc"}
            },
            "links": {"leave": "https://x/hang-up"}
        });
        let s = SharingSession::read("corr", &answer).unwrap();
        assert_eq!(s.leave(), Some("https://x/cs-leave"));
        assert_eq!(s.take_control(), Some("https://x/tc"));
        assert_eq!(s.session_id.as_deref(), Some("cs-1"));
        assert_eq!(s.correlation_id, "corr");
    }

    #[test]
    fn a_grant_frame_is_read_at_any_depth_and_prefixed_names_are_shortened() {
        let frame = json!({
            "addModalitySuccess": {
                "contentSharingSessionId": "cs-42",
                "links": {
                    "contentSharingLeave": "https://x/l",
                    "contentSharingTakeControl": "https://x/t",
                    "contentSharingUpdateSessionState": "https://x/u",
                    "contentSharingController": "https://x/c",
                    "sync": "https://x/s"
                }
            }
        });
        let s = SharingSession::read("c", &frame).unwrap();
        assert_eq!(s.leave(), Some("https://x/l"));
        assert_eq!(s.take_control(), Some("https://x/t"));
        assert_eq!(s.update_session_state(), Some("https://x/u"));
        assert_eq!(s.links.get("contentSharingController"), Some("https://x/c"));
        assert_eq!(s.links.get("controller"), None);
        assert_eq!(s.session_id.as_deref(), Some("cs-42"));
        // Flat, as `contentSharingEnd` is said to arrive.
        let flat = json!({"links": {"leave": "https://x/flat"}});
        assert_eq!(
            SharingSession::read("c", &flat).unwrap().leave(),
            Some("https://x/flat")
        );
        // No way out: not a session we hold.
        assert!(SharingSession::read("c", &json!({"links": {}})).is_none());
        assert!(SharingSession::read("c", &json!({})).is_none());
    }

    #[test]
    fn session_events_are_told_apart() {
        assert!(is_session_event("contentSharingUpdate"));
        assert!(is_session_event("contentSharingEnd"));
        assert!(is_session_event("addModalitySuccess"));
        assert!(!is_session_event("conversationUpdate"));
    }
}
