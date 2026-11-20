//! Teams meeting helpers: the meeting chat thread from a join URL (C1.3), Meet now and join
//! by ID (C3.2), and the event types of the New menu (C2.6).

use reqwest::Method;
use serde_json::{Value, json};
use teams_proto::{Client, Error, Result};

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(v) = u8::from_str_radix(hex, 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The meeting chat thread (`19:meeting_…@thread.v2`) in a Teams join URL such as
/// `https://teams.microsoft.com/l/meetup-join/19%3ameeting_AbC%40thread.v2/0?context=…`.
/// `None` for links that carry no thread, like `/meet/<code>`.
pub fn meeting_chat_thread_id(join_url: &str) -> Option<String> {
    let no_query = join_url.split(['?', '#']).next()?;
    let after = no_query.split("/meetup-join/").nth(1)?;
    let segment = percent_decode(after.split('/').next()?);
    (segment.starts_with("19:") && segment.contains("@thread.")).then_some(segment)
}

/// The page where the organizer sets a meeting's options (lobby, who presents, recording),
/// built from the join URL: its thread and the `Tid` (tenant) and `Oid` (organizer) of its
/// `context` parameter. The shape is the one `docs/research/calendar-meetings.md` records; the
/// page itself is the organizer's only (Teams refuses it for others). `None` when the join link
/// carries no thread or no context (a `/meet/<code>` link).
pub fn meeting_options_url(join_url: &str) -> Option<String> {
    let thread = meeting_chat_thread_id(join_url)?;
    let query = join_url.split_once('?')?.1.split('#').next()?;
    let context = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("context="))?;
    let v: Value = serde_json::from_str(&percent_decode(context)).ok()?;
    let id = |key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
    };
    let (tenant, organizer) = (id("Tid")?, id("Oid")?);
    Some(format!(
        "https://teams.microsoft.com/meetingOptions/?organizerId={organizer}&tenantId={tenant}&threadId={}&messageId=0&language=en-US",
        crate::graph::enc(&thread)
    ))
}

/// `https://teams.microsoft.com/meet/<id>?p=<passcode>`, the page that joins by meeting ID.
/// Spaces in the ID ("123 456 789 012") are dropped.
pub fn join_by_id_url(meeting_id: &str, passcode: &str) -> Option<String> {
    let id: String = meeting_id.chars().filter(|c| !c.is_whitespace()).collect();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let mut url = format!("https://teams.microsoft.com/meet/{id}");
    let pass = passcode.trim();
    if !pass.is_empty() {
        url.push_str("?p=");
        url.push_str(&crate::graph::enc(pass));
    }
    Some(url)
}

/// An instant meeting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetNow {
    pub thread_id: String,
    pub join_url: String,
    pub short_url: Option<String>,
}

pub fn meet_now_body(subject: &str) -> Value {
    json!({
        "meetingType": "MeetNow",
        "isStreamEnabled": false,
        "subject": subject,
        "unhideChatThread": true,
    })
}

/// Reads `value{groupContext.threadId, links.join, links.shortJoinUrl}`.
pub fn parse_meet_now(v: &Value) -> Option<MeetNow> {
    let value = v.get("value")?;
    let s = |p: &Value, k: &str| p.get(k)?.as_str().map(String::from);
    Some(MeetNow {
        thread_id: s(value.get("groupContext")?, "threadId")?,
        join_url: s(value.get("links")?, "join")?,
        short_url: s(value.get("links")?, "shortJoinUrl"),
    })
}

/// Creates an instant meeting through the Teams middle tier and returns its chat thread and
/// links. This creates a real meeting: it is not called from tests.
pub async fn meet_now(client: &Client, subject: &str) -> Result<MeetNow> {
    let url = format!(
        "{}/beta/me/calendarEvents/privateMeeting/schedulingService/create",
        client.middle_tier().await
    );
    let v = client
        .json(
            "meet now",
            Method::POST,
            &url,
            Some(&meet_now_body(subject)),
        )
        .await?;
    parse_meet_now(&v).ok_or_else(|| Error::Protocol("meet now: unexpected response shape".into()))
}

/// What the New menu offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    Event,
    ChannelMeeting,
    Webinar,
    TownHall,
    VirtualAppointment,
    LiveEvent,
}

impl EventType {
    pub const ALL: [EventType; 6] = [
        EventType::Event,
        EventType::ChannelMeeting,
        EventType::Webinar,
        EventType::TownHall,
        EventType::VirtualAppointment,
        EventType::LiveEvent,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EventType::Event => "Event",
            EventType::ChannelMeeting => "Channel meeting",
            EventType::Webinar => "Webinar",
            EventType::TownHall => "Town hall",
            EventType::VirtualAppointment => "Virtual appointment",
            EventType::LiveEvent => "Live event",
        }
    }

    /// Webinars, town halls, virtual appointments and live events need Graph `VirtualEvent.*`
    /// (admin consent) or the Teams web UI, so kelma sends the user to the web for them.
    pub fn opens_in_web(self) -> bool {
        !matches!(self, EventType::Event | EventType::ChannelMeeting)
    }

    /// The page that creates this type in the browser, `None` for the native types.
    pub fn web_url(self) -> Option<&'static str> {
        self.opens_in_web()
            .then_some("https://teams.microsoft.com/v2/#/calendar")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_id_from_encoded_join_urls() {
        let u = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_ZmFrZWZha2VmYWtl%40thread.v2/0?context=%7b%22Tid%22%3a%22t%22%7d";
        assert_eq!(
            meeting_chat_thread_id(u).as_deref(),
            Some("19:meeting_ZmFrZWZha2VmYWtl@thread.v2")
        );
        // Already decoded, upper-case escapes, no query.
        assert_eq!(
            meeting_chat_thread_id(
                "https://teams.microsoft.com/l/meetup-join/19:meeting_abc@thread.v2/0"
            )
            .as_deref(),
            Some("19:meeting_abc@thread.v2")
        );
        assert_eq!(
            meeting_chat_thread_id(
                "https://teams.microsoft.com/l/meetup-join/19%3Ameeting_abc%40thread.v2/0"
            )
            .as_deref(),
            Some("19:meeting_abc@thread.v2")
        );
    }

    #[test]
    fn the_options_page_is_built_from_the_join_link() {
        let u = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?context=%7b%22Tid%22%3a%22aaaa-1111%22%2c%22Oid%22%3a%22bbbb-2222%22%7d";
        assert_eq!(
            meeting_options_url(u).as_deref(),
            Some(
                "https://teams.microsoft.com/meetingOptions/?organizerId=bbbb-2222&tenantId=aaaa-1111&threadId=19%3Ameeting_abc%40thread.v2&messageId=0&language=en-US"
            )
        );
        for none in [
            // No context.
            "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0",
            // A context that names no organizer.
            "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?context=%7b%22Tid%22%3a%22aaaa%22%7d",
            // Ids that are not ids do not reach the URL.
            "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?context=%7b%22Tid%22%3a%22a%26b%22%2c%22Oid%22%3a%22c%22%7d",
            // A meeting-by-code link has no thread.
            "https://teams.microsoft.com/meet/123456789012?p=abc",
        ] {
            assert_eq!(meeting_options_url(none), None, "{none}");
        }
    }

    #[test]
    fn no_thread_in_other_links() {
        for u in [
            "https://teams.microsoft.com/meet/123456789012?p=abc",
            "https://example.com/",
            "",
            "https://teams.microsoft.com/l/meetup-join/notathread/0",
        ] {
            assert_eq!(meeting_chat_thread_id(u), None, "{u}");
        }
    }

    #[test]
    fn join_by_id() {
        assert_eq!(
            join_by_id_url("123 456 789 012", "aB3 xY").as_deref(),
            Some("https://teams.microsoft.com/meet/123456789012?p=aB3%20xY")
        );
        assert_eq!(
            join_by_id_url("4567890123", "").as_deref(),
            Some("https://teams.microsoft.com/meet/4567890123")
        );
        assert_eq!(join_by_id_url("", "x"), None);
        assert_eq!(join_by_id_url("12/34", "x"), None, "no path tricks");
    }

    #[test]
    fn meet_now_request_and_response() {
        assert_eq!(
            meet_now_body("Quick sync"),
            json!({"meetingType": "MeetNow", "isStreamEnabled": false,
                   "subject": "Quick sync", "unhideChatThread": true})
        );
        let m = parse_meet_now(&json!({"value": {
            "groupContext": {"threadId": "19:meeting_x@thread.v2"},
            "links": {"join": "https://teams.microsoft.com/l/meetup-join/19%3ameeting_x%40thread.v2/0",
                      "shortJoinUrl": "https://teams.microsoft.com/meet/123"}}}))
        .unwrap();
        assert_eq!(m.thread_id, "19:meeting_x@thread.v2");
        assert_eq!(
            m.short_url.as_deref(),
            Some("https://teams.microsoft.com/meet/123")
        );
        assert_eq!(
            meeting_chat_thread_id(&m.join_url).as_deref(),
            Some("19:meeting_x@thread.v2")
        );
        assert_eq!(parse_meet_now(&json!({"value": {}})), None);
    }

    #[test]
    fn event_types() {
        assert_eq!(EventType::ALL.len(), 6);
        let native: Vec<_> = EventType::ALL
            .iter()
            .filter(|t| !t.opens_in_web())
            .collect();
        assert_eq!(native, [&EventType::Event, &EventType::ChannelMeeting]);
        assert!(EventType::Event.web_url().is_none());
        assert!(
            EventType::Webinar
                .web_url()
                .unwrap()
                .starts_with("https://teams.microsoft.com/")
        );
        assert_eq!(EventType::TownHall.label(), "Town hall");
    }
}
