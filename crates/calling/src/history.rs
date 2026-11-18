//! Call history (plans/calls.md K1.1): the user's call log, read from the `48:calllogs` system
//! thread of the chat service.
//!
//! Each call is one message whose `properties["call-log"]` holds the entry, as a JSON string (the
//! live shape) or as an object (tolerated). Shape, from the live service and rusty-teams
//! `chatsvc/src/calllog.rs` (MIT, adapted): `callId`, `startTime`, `connectTime` (null when the
//! call was never answered), `endTime`, `callDirection` (`incoming`/`outgoing`), `callState`
//! (`accepted`/`missed`/`declined`), `callType` (`twoParty`/`multiParty`), `originator`/`target`
//! MRIs with `originatorParticipant`/`targetParticipant` (`id`, `displayName`, `type`:
//! `default`/`voicemail`), `participants` (MRIs, multi-party only), `threadId`. A voicemail left
//! for the user is a sparser entry (no state, type or target) with `voicemailCallId` and
//! `voicemailInternetMessageId`; the user's own call into someone's voicemail box is an ordinary
//! outgoing entry whose `targetParticipant.type` is `voicemail`. An outgoing group call has a
//! null `target` and `targetParticipant`.
//!
//! The log is written from the user's point of view: the user is the target of an incoming call
//! and the originator of an outgoing one. Only the originator carries a display name, so for an
//! outgoing call [`CallRecord::other_name`] is `None` and the caller resolves the MRI.

use crate::{Error, Result};
use serde_json::Value;
use std::collections::HashSet;
use std::future::Future;
use teams_proto::Client;
use teams_proto::time::parse_iso_ms;

const CALL_LOG_THREAD: &str = "48:calllogs";
const LIST_QUERY: &str = "view=msnp24Equivalent&pageSize=200";
/// Upper bound on pages followed, so a misbehaving `backwardLink` cannot loop forever.
const MAX_PAGES: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallDirection {
    Incoming,
    Outgoing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallOutcome {
    /// The call connected.
    Accepted,
    /// Never answered (also an outgoing call the other side did not pick up).
    Missed,
    Declined,
    /// Went to, or came from, a voicemail box.
    Voicemail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRecord {
    pub id: String,
    /// When the call started ringing, Unix milliseconds.
    pub start_ms: i64,
    pub end_ms: Option<i64>,
    /// Talk time (connect to end) in seconds; `None` when the call never connected.
    pub duration_secs: Option<i64>,
    pub direction: CallDirection,
    pub outcome: CallOutcome,
    /// The other party's MRI (the caller of an incoming call, the callee of an outgoing one,
    /// also when the callee's voicemail took it); `None` for an outgoing group call, whose log
    /// names no target.
    pub other_mri: Option<String>,
    /// Display name of the other party. Only incoming calls carry one.
    pub other_name: Option<String>,
    /// A multi-party call (meeting or group call) rather than a one-to-one call.
    pub is_group: bool,
    /// The chat thread of a group call or meeting, when the log names one.
    pub thread_id: Option<String>,
    /// MRIs of everyone but the user, for group calls; empty for one-to-one calls.
    pub participants: Vec<String>,
    /// Internet message id of the voicemail mail item, for a voicemail left for the user.
    pub voicemail_message_id: Option<String>,
}

impl CallRecord {
    /// An incoming call the user did not answer (Teams' "Missed" filter, shown in red).
    pub fn is_missed(&self) -> bool {
        self.direction == CallDirection::Incoming && self.outcome == CallOutcome::Missed
    }

    pub fn is_voicemail(&self) -> bool {
        self.outcome == CallOutcome::Voicemail
    }
}

/// One page of the call log: its records (newest first) and the link to the next older page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub records: Vec<CallRecord>,
    pub older: Option<String>,
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)?.as_str().filter(|s| !s.is_empty())
}

fn time(v: &Value, key: &str) -> Option<i64> {
    parse_iso_ms(v.get(key)?.as_str()?)
}

/// The `call-log` entry of a message, whether it is stored as a JSON string or an object.
fn log_of(message: &Value) -> Option<Value> {
    match message.pointer("/properties/call-log")? {
        Value::String(s) => serde_json::from_str(s).ok(),
        o @ Value::Object(_) => Some(o.clone()),
        _ => None,
    }
}

fn parse_record(message: &Value) -> Option<CallRecord> {
    let log = log_of(message)?;
    let direction = match text(&log, "callDirection")? {
        "incoming" => CallDirection::Incoming,
        "outgoing" => CallDirection::Outgoing,
        _ => return None,
    };
    let start_ms = time(&log, "startTime")?;
    let connect_ms = time(&log, "connectTime");
    let end_ms = time(&log, "endTime");
    let duration_secs = connect_ms.zip(end_ms).map(|(c, e)| ((e - c) / 1000).max(0));

    let null = Value::Null;
    let originator = log.get("originatorParticipant").unwrap_or(&null);
    let target = log.get("targetParticipant").unwrap_or(&null);
    let voicemail_message_id = text(&log, "voicemailInternetMessageId").map(str::to_owned);
    let voicemail = voicemail_message_id.is_some()
        || text(&log, "voicemailCallId").is_some()
        || [originator, target]
            .iter()
            .any(|p| text(p, "type") == Some("voicemail"));
    let outcome = if voicemail {
        CallOutcome::Voicemail
    } else {
        match text(&log, "callState") {
            Some("accepted") => CallOutcome::Accepted,
            Some("declined") => CallOutcome::Declined,
            Some("missed") => CallOutcome::Missed,
            _ if connect_ms.is_some() => CallOutcome::Accepted,
            _ => CallOutcome::Missed,
        }
    };

    // The user is the target of an incoming call and the originator of an outgoing one.
    let (other, other_key, own_key) = match direction {
        CallDirection::Incoming => (originator, "originator", "target"),
        CallDirection::Outgoing => (target, "target", "originator"),
    };
    let other_mri = text(other, "id")
        .or_else(|| text(&log, other_key))
        .map(str::to_owned);
    let other_name = text(other, "displayName").map(str::to_owned);
    let own = text(&log, own_key);
    let participants = log
        .get("participants")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .filter(|p| Some(*p) != own)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    Some(CallRecord {
        id: text(&log, "callId")
            .or_else(|| text(message, "id"))?
            .to_owned(),
        start_ms,
        end_ms,
        duration_secs,
        direction,
        outcome,
        other_mri,
        other_name,
        is_group: text(&log, "callType") == Some("multiParty"),
        thread_id: text(&log, "threadId").map(str::to_owned),
        participants,
        voicemail_message_id,
    })
}

/// Parses one page of the `48:calllogs` messages. Messages that are not call logs, or whose log
/// lacks a direction or start time, are skipped; duplicates within the page are dropped.
pub fn parse_page(body: &Value) -> Page {
    let mut seen = HashSet::new();
    let mut records: Vec<CallRecord> = body
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(parse_record)
        .filter(|r| seen.insert(r.id.clone()))
        .collect();
    records.sort_by_key(|r| std::cmp::Reverse(r.start_ms));
    Page {
        records,
        older: body
            .pointer("/_metadata/backwardLink")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
    }
}

/// First-page URL of the call log for a chat-service base URL.
pub fn call_log_url(chat_service: &str) -> String {
    let thread: String = url::form_urlencoded::byte_serialize(CALL_LOG_THREAD.as_bytes()).collect();
    format!("{chat_service}/v1/users/ME/conversations/{thread}/messages?{LIST_QUERY}")
}

/// Collects up to `limit` records newest first, fetching pages through `fetch` (given a URL,
/// returns that page's JSON) and following `_metadata.backwardLink`. Stops at the limit, when
/// there is no older page, or when a page adds nothing new. A page that fails after the first
/// ends the collection with what was gathered; a failing first page is an error.
pub async fn collect_history<F, Fut>(
    first_url: String,
    limit: usize,
    mut fetch: F,
) -> Result<Vec<CallRecord>>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<Value>>,
{
    let mut records: Vec<CallRecord> = Vec::new();
    let mut seen = HashSet::new();
    let mut url = first_url;
    for page_no in 0..MAX_PAGES {
        let body = match fetch(url).await {
            Ok(body) => body,
            Err(e) if page_no == 0 => return Err(e),
            Err(e) => {
                tracing::warn!("call history: older page failed, keeping what we have: {e}");
                break;
            }
        };
        let page = parse_page(&body);
        let before = records.len();
        records.extend(
            page.records
                .into_iter()
                .filter(|r| seen.insert(r.id.clone())),
        );
        if records.len() >= limit || records.len() == before {
            break;
        }
        match page.older {
            Some(next) => url = next,
            None => break,
        }
    }
    records.sort_by_key(|r| std::cmp::Reverse(r.start_ms));
    records.truncate(limit);
    Ok(records)
}

/// The user's call history, newest first, at most `limit` entries.
pub async fn call_history(client: &Client, limit: usize) -> Result<Vec<CallRecord>> {
    let first = call_log_url(&client.chat_service().await);
    collect_history(first, limit, |url| async move {
        client
            .json("call history", reqwest::Method::GET, &url, None)
            .await
            .map_err(Error::from)
    })
    .await
}
