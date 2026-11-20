//! Voicemail (plans/calls.md K1.3): Teams voicemails are Exchange mail items, read through Graph.
//!
//! Facts (Microsoft documentation of Exchange Unified Messaging and Cloud Voicemail, and Graph's
//! extended-property filters): a voicemail is a message whose message class (MAPI `String
//! 0x001A`, `PidTagMessageClass`) is `IPM.Note.Microsoft.Voicemail.UM.CA` (Cloud Voicemail) or
//! `IPM.Note.Microsoft.Voicemail.UM`; its length in seconds is `Integer 0x6801`
//! (`PidTagVoiceMessageDuration`); the recording is a file attachment (`audio/mpeg`, an `.mp3`,
//! sometimes `.wma`/`.wav`); the transcript, when there is one, is the body.
//!
//! The list is one `GET /me/messages` filtered on the message class with the duration expanded;
//! Graph refuses `$orderby` next to an extended-property filter on some mailboxes, so the list
//! is sorted here. The recording is `GET /me/messages/{id}/attachments` then the audio
//! attachment's `/$value`. Marking one read is `PATCH /me/messages/{id}` with `isRead`.
//!
//! UNVERIFIED live: written without a voicemail on the test account (no live calls tonight).
//! The call log's voicemail entries ([`crate::history::CallRecord::voicemail_message_id`]) carry
//! the same `internetMessageId`, which is how a history row finds its mail.

use crate::{Error, Result};
use serde_json::{Value, json};
use teams_proto::Client;
use teams_proto::time::parse_iso_ms;

const GRAPH: &str = "https://graph.microsoft.com/v1.0";
const CLASSES: [&str; 2] = [
    "IPM.Note.Microsoft.Voicemail.UM.CA",
    "IPM.Note.Microsoft.Voicemail.UM",
];
const DURATION_PROP: &str = "Integer 0x6801";
/// A voicemail recording bigger than this is not downloaded.
pub const MAX_AUDIO_BYTES: usize = 25 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voicemail {
    /// Graph message id.
    pub id: String,
    pub internet_message_id: Option<String>,
    pub caller_name: String,
    /// The caller's address (an email address or `+number@…`), empty when the mail has none.
    pub caller_address: String,
    pub received_ms: i64,
    pub read: bool,
    pub duration_secs: Option<i64>,
    /// The body preview: the transcript when the service made one.
    pub transcript: String,
}

/// The recording of a voicemail, as listed by its message's attachments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording {
    pub attachment_id: String,
    pub name: String,
    pub content_type: String,
}

/// Percent-encodes a query value (spaces as `%20`, which Graph's OData parser expects).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The `/me/messages` URL that lists the voicemails of one message `class`, at most `top`.
/// Graph allows only `and` and `eq` inside an extended-property filter (`or` is refused with
/// `ErrorInvalidUrlQueryFilter`, seen live), so each class is its own request.
pub fn list_url(class: &str, top: usize) -> String {
    let filter = format!(
        "singleValueExtendedProperties/any(ep:ep/id eq 'String 0x001A' and ep/value eq '{class}')"
    );
    let expand = format!("singleValueExtendedProperties($filter=id eq '{DURATION_PROP}')");
    format!(
        "{GRAPH}/me/messages?$filter={}&$expand={}&$select={}&$top={top}",
        encode(&filter),
        encode(&expand),
        "id,internetMessageId,from,receivedDateTime,isRead,bodyPreview,hasAttachments",
    )
}

fn duration_of(m: &Value) -> Option<i64> {
    m.get("singleValueExtendedProperties")?
        .as_array()?
        .iter()
        .find(|p| {
            p.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id.eq_ignore_ascii_case(DURATION_PROP))
        })?
        .get("value")?
        .as_str()?
        .trim()
        .parse()
        .ok()
}

fn parse_one(m: &Value) -> Option<Voicemail> {
    let s = |p: &str| m.pointer(p).and_then(Value::as_str).unwrap_or_default();
    let id = s("/id");
    if id.is_empty() {
        return None;
    }
    let address = s("/from/emailAddress/address").to_owned();
    let name = s("/from/emailAddress/name");
    Some(Voicemail {
        id: id.to_owned(),
        internet_message_id: Some(s("/internetMessageId").to_owned()).filter(|x| !x.is_empty()),
        caller_name: if name.is_empty() {
            address.clone()
        } else {
            name.to_owned()
        },
        caller_address: address,
        received_ms: parse_iso_ms(s("/receivedDateTime")).unwrap_or(0),
        read: m.get("isRead").and_then(Value::as_bool).unwrap_or(true),
        duration_secs: duration_of(m),
        transcript: s("/bodyPreview").trim().to_owned(),
    })
}

/// Parses a `/me/messages` page into voicemails, newest first.
pub fn parse_list(body: &Value) -> Vec<Voicemail> {
    let mut out: Vec<Voicemail> = body
        .get("value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(parse_one)
        .collect();
    out.sort_by_key(|v| std::cmp::Reverse(v.received_ms));
    out
}

/// The audio attachment among a message's attachments: the first `audio/*` one, else the first
/// whose name ends in a known audio extension.
pub fn pick_recording(body: &Value) -> Option<Recording> {
    let items: Vec<Recording> = body
        .get("value")?
        .as_array()?
        .iter()
        .filter_map(|a| {
            Some(Recording {
                attachment_id: a.get("id")?.as_str()?.to_owned(),
                name: a
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                content_type: a
                    .get("contentType")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_ascii_lowercase(),
            })
        })
        .collect();
    let audio_ext = |n: &str| {
        let n = n.to_ascii_lowercase();
        [".mp3", ".wma", ".wav", ".m4a"]
            .iter()
            .any(|e| n.ends_with(e))
    };
    items
        .iter()
        .find(|r| r.content_type.starts_with("audio/"))
        .or_else(|| items.iter().find(|r| audio_ext(&r.name)))
        .cloned()
}

fn message_url(id: &str) -> String {
    format!("{GRAPH}/me/messages/{}", encode(id))
}

pub fn attachments_url(id: &str) -> String {
    format!(
        "{}/attachments?$select=id,name,contentType,size",
        message_url(id)
    )
}

pub fn recording_url(id: &str, attachment_id: &str) -> String {
    format!(
        "{}/attachments/{}/$value",
        message_url(id),
        encode(attachment_id)
    )
}

/// `PATCH /me/messages/{id}` body for Mark as read / unread.
pub fn read_body(read: bool) -> Value {
    json!({ "isRead": read })
}

/// The newest `top` voicemails, of every voicemail message class.
pub async fn voicemails(client: &Client, top: usize) -> Result<Vec<Voicemail>> {
    let mut all = Vec::new();
    for class in CLASSES {
        let body = client
            .json(
                "voicemail",
                reqwest::Method::GET,
                &list_url(class, top),
                None,
            )
            .await?;
        if let Some(v) = body.get("value").and_then(Value::as_array) {
            all.extend(v.iter().cloned());
        }
    }
    let mut list = parse_list(&json!({ "value": all }));
    let mut seen = std::collections::HashSet::new();
    list.retain(|v| seen.insert(v.id.clone()));
    list.truncate(top);
    Ok(list)
}

/// Downloads the recording of voicemail `id`: its file name and bytes.
pub async fn recording(client: &Client, id: &str) -> Result<(String, Vec<u8>)> {
    let list = client
        .json(
            "voicemail attachments",
            reqwest::Method::GET,
            &attachments_url(id),
            None,
        )
        .await?;
    let rec = pick_recording(&list)
        .ok_or_else(|| Error::Protocol("the voicemail has no recording".into()))?;
    let data = client
        .bytes(&recording_url(id, &rec.attachment_id), &[], MAX_AUDIO_BYTES)
        .await?;
    let name = if rec.name.is_empty() {
        "voicemail.mp3".to_owned()
    } else {
        rec.name
    };
    Ok((name, data))
}

/// Marks voicemail `id` read or unread.
pub async fn mark_read(client: &Client, id: &str, read: bool) -> Result<()> {
    client
        .json(
            "voicemail read",
            reqwest::Method::PATCH,
            &message_url(id),
            Some(&read_body(read)),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_url_filters_on_one_voicemail_class_and_expands_the_duration() {
        let u = list_url("IPM.Note.Microsoft.Voicemail.UM", 50);
        assert!(u.starts_with("https://graph.microsoft.com/v1.0/me/messages?$filter="));
        assert!(!u.contains(' '), "spaces are encoded: {u}");
        assert!(u.contains("%27IPM.Note.Microsoft.Voicemail.UM%27"));
        assert!(
            !u.contains("%20or%20"),
            "Graph refuses `or` in this filter: {u}"
        );
        assert!(u.contains("String%200x001A"));
        assert!(u.contains("Integer%200x6801"));
        assert!(u.ends_with("&$top=50"));
    }

    #[test]
    fn voicemails_parse_with_caller_duration_and_transcript_newest_first() {
        let body = json!({"value": [
            {
                "id": "AAMk-old",
                "internetMessageId": "<old@example.test>",
                "from": {"emailAddress": {"name": "Person A", "address": "a@example.test"}},
                "receivedDateTime": "2026-11-18T09:00:00Z",
                "isRead": true,
                "bodyPreview": "  Hi, call me back.  ",
                "singleValueExtendedProperties": [{"id": "Integer 0x6801", "value": "17"}]
            },
            {
                "id": "AAMk-new",
                "from": {"emailAddress": {"name": "", "address": "+15550100@example.test"}},
                "receivedDateTime": "2026-11-19T10:30:00Z",
                "isRead": false,
                "bodyPreview": ""
            },
            {"subject": "no id, skipped"}
        ]});
        let v = parse_list(&body);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].id, "AAMk-new");
        assert_eq!(
            v[0].caller_name, "+15550100@example.test",
            "address stands in for a name"
        );
        assert!(!v[0].read);
        assert_eq!(v[0].duration_secs, None);
        assert_eq!(v[1].caller_name, "Person A");
        assert_eq!(v[1].duration_secs, Some(17));
        assert_eq!(v[1].transcript, "Hi, call me back.");
        assert_eq!(
            v[1].internet_message_id.as_deref(),
            Some("<old@example.test>")
        );
        assert!(v[0].received_ms > v[1].received_ms);
    }

    #[test]
    fn the_recording_is_the_audio_attachment() {
        let body = json!({"value": [
            {"id": "a1", "name": "logo.png", "contentType": "image/png"},
            {"id": "a2", "name": "audio.mp3", "contentType": "audio/mpeg"}
        ]});
        assert_eq!(pick_recording(&body).unwrap().attachment_id, "a2");
        let by_name = json!({"value": [
            {"id": "b1", "name": "VoiceMessage.WMA", "contentType": "application/octet-stream"}
        ]});
        assert_eq!(pick_recording(&by_name).unwrap().attachment_id, "b1");
        assert_eq!(pick_recording(&json!({"value": []})), None);
    }

    #[test]
    fn message_ids_are_encoded_in_paths() {
        assert_eq!(
            recording_url("AA/k=", "x+y"),
            "https://graph.microsoft.com/v1.0/me/messages/AA%2Fk%3D/attachments/x%2By/$value"
        );
        assert_eq!(read_body(true), json!({"isRead": true}));
    }
}
