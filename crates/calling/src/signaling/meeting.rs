//! What to join: a meeting target read from a CSA meeting chat or from a join link.
//!
//! Two ways to address a meeting, both in the references:
//! - by thread + context (`groupChat` + `meetingInfo {tenantId, organizerId}`): the long
//!   `/l/meetup-join/<thread>/<message>?context={Tid,Oid}` link; teams-lite verified it live on a
//!   work tenant.
//! - by code (`meetingData {meetingCode, passcode, meetingUrl}`): the short `/meet/<code>?p=<pass>`
//!   link; mautrix-teams joins any meeting this way, also outside the user's tenant (its test uses
//!   a `teams.live.com` link), and teams-lite uses it for its own tenant's short links.

use super::bodies::MeetingInfo;
use serde_json::{Value, json};
use url::Url;

const MEETING_HOSTS: [&str; 3] = [
    "teams.microsoft.com",
    "teams.cloud.microsoft",
    "teams.live.com",
];

/// Everything known about a meeting to join. Only what the source really carried is set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeetingTarget {
    /// `19:meeting_...@thread.v2`.
    pub thread_id: Option<String>,
    /// The reply-chain message id; `"0"` for a meeting chat.
    pub message_id: String,
    pub tenant_id: Option<String>,
    /// As the source names it: a bare object id for a work organizer, an upper-case cid for a
    /// personal one.
    pub organizer_id: Option<String>,
    /// The running conversation (`activeMeetup.conversationUrl`), only while it is active.
    pub conversation_url: Option<String>,
    pub meeting_code: Option<String>,
    pub passcode: Option<String>,
    /// The join link itself, sent as `meetingData.meetingUrl`.
    pub join_url: Option<String>,
}

impl MeetingTarget {
    /// `meetingData` for a join by code, when a code is known.
    pub fn meeting_data(&self) -> Option<Value> {
        let code = self.meeting_code.as_ref()?;
        let mut data = json!({"meetingCode": code});
        if let Some(p) = &self.passcode {
            data["passcode"] = json!(p);
        }
        if let Some(u) = &self.join_url {
            data["meetingUrl"] = json!(u);
        }
        Some(data)
    }

    /// Thread + context for a join by `meetingInfo`, when all three are known.
    pub fn meeting_info(&self) -> Option<MeetingInfo> {
        Some(MeetingInfo {
            thread_id: self.thread_id.clone()?,
            tenant_id: self.tenant_id.clone()?,
            organizer_id: self.organizer_id.clone()?,
        })
    }
}

fn text(v: &Value, ptr: &str) -> Option<String> {
    v.pointer(ptr)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Reads a chat entry of CSA `GET /api/csa/api/v1/teams/users/me` (`chats[]`). `None` when the
/// chat has no meeting (`meetingInformation` / `activeMeetup`).
pub fn meeting_from_csa_chat(chat: &Value) -> Option<MeetingTarget> {
    let info = chat.get("meetingInformation").filter(|v| v.is_object());
    let meetup = chat.get("activeMeetup").filter(|v| v.is_object());
    if info.is_none() && meetup.is_none() {
        return None;
    }
    let info = info.cloned().unwrap_or(Value::Null);
    let meetup = meetup.cloned().unwrap_or(Value::Null);
    let link = text(&info, "/meetingJoinUrl").and_then(|u| meeting_from_join_url(&u));
    let active = meetup["status"].as_str() == Some("Active");
    Some(MeetingTarget {
        thread_id: text(chat, "/id"),
        message_id: text(&meetup, "/messageId").unwrap_or_else(|| "0".into()),
        tenant_id: text(&info, "/tenantId").or_else(|| text(chat, "/tenantId")),
        organizer_id: text(&info, "/organizerId"),
        conversation_url: active.then(|| text(&meetup, "/conversationUrl")).flatten(),
        meeting_code: text(&meetup, "/meetingData/meetingCode")
            .or_else(|| text(&info, "/meetingData/meetingCode"))
            .or_else(|| link.as_ref().and_then(|l| l.meeting_code.clone())),
        passcode: link.as_ref().and_then(|l| l.passcode.clone()),
        join_url: link.and_then(|l| l.join_url),
    })
}

/// Reads a meeting join link: the short `https://teams.live.com/meet/<code>?p=<passcode>` (also on
/// `teams.microsoft.com` / `teams.cloud.microsoft`) or the long
/// `.../l/meetup-join/<thread>/<message>?context={"Tid":..,"Oid":..}`.
pub fn meeting_from_join_url(link: &str) -> Option<MeetingTarget> {
    let url = Url::parse(link).ok()?;
    if url.scheme() != "https" || !MEETING_HOSTS.contains(&url.host_str()?) {
        return None;
    }
    let segments: Vec<String> = url
        .path_segments()?
        .map(|s| {
            url::form_urlencoded::parse(format!("x={s}").as_bytes())
                .next()
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default()
        })
        .collect();
    let query = |k: &str| {
        url.query_pairs()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.into_owned())
            .filter(|v| !v.is_empty())
    };
    match segments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["meet", code, ..] => {
            if code.is_empty() || !code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return None;
            }
            Some(MeetingTarget {
                message_id: "0".into(),
                meeting_code: Some((*code).to_owned()),
                passcode: query("p"),
                join_url: Some(link.to_owned()),
                ..Default::default()
            })
        }
        ["l", "meetup-join", thread, rest @ ..] => {
            if !thread.starts_with("19:") {
                return None;
            }
            let context: Value = query("context")
                .and_then(|c| serde_json::from_str(&c).ok())
                .unwrap_or(Value::Null);
            Some(MeetingTarget {
                thread_id: Some((*thread).to_owned()),
                message_id: rest
                    .first()
                    .filter(|m| !m.is_empty())
                    .map_or("0", |m| m)
                    .to_owned(),
                tenant_id: text(&context, "/Tid"),
                organizer_id: text(&context, "/Oid"),
                join_url: Some(link.to_owned()),
                ..Default::default()
            })
        }
        _ => None,
    }
}
