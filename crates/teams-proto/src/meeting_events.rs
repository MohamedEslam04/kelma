//! The meeting events a chat shows as cards: a call that started or ended (`Event/Call`), a
//! meeting recording (`RichText/Media_CallRecording`) and a transcript
//! (`RichText/Media_CallTranscript`).
//!
//! An `Event/Call` is XML without a root element. A started call names no one; an ended one
//! lists who took part, each with the seconds the call lasted:
//!
//! ```text
//! <ended/><partlist alt="" count="2"><part identity="8:orgid:…"><name>8:orgid:…</name>
//!   <displayName>…</displayName><duration>3950</duration></part>…</partlist>
//! <meetingDetails><meetingDetails>…<isRecapEnabled>true</isRecapEnabled></meetingDetails>
//! </meetingDetails><callEventType>callEnded</callEventType><callId>…</callId>
//! ```
//!
//! A recording sends one message per state change (`Initial`, `ChunkFinished` for the recording
//! and for the transcript track, then `Success` with the links), all naming the same `callId`;
//! see [`crate::recaps::ChatRecording`] for the shape of the finished one.

use crate::recaps::{
    RECORDING_MESSAGE, call_id_of, first_attr, parse_transcript_message, recording_length_ms,
    start_tags, tag_attr, unescape_xml,
};

/// The message type of a call that started or ended.
pub const CALL_EVENT: &str = "Event/Call";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeetingEvent {
    Call(CallEvent),
    Recording(RecordingEvent),
    /// A transcript was started for this call (`""` when the message names none).
    Transcript {
        call_id: String,
    },
}

/// Someone who took part in a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallPart {
    pub mri: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallEvent {
    pub call_id: String,
    pub ended: bool,
    /// How long the call lasted, in seconds (the longest time anyone was in it).
    pub duration_secs: Option<i64>,
    /// The people with a name (bots and nameless guests are left out).
    pub people: Vec<CallPart>,
    /// How many took part, counting the ones without a name.
    pub count: usize,
    /// The meeting has a recap (`isRecapEnabled`).
    pub recap: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingState {
    /// Started, still recording or still being processed.
    InProgress,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingEvent {
    pub state: RecordingState,
    /// The recording's file name without `.mp4`, else the meeting title.
    pub title: String,
    /// Where it plays in the browser (https only).
    pub url: Option<String>,
    pub duration_ms: Option<i64>,
    pub call_id: Option<String>,
    /// The media object of a finished recording (`AMSDocumentID`).
    pub document_id: Option<String>,
    /// Who started the recording (an MRI).
    pub initiator: Option<String>,
    /// A transcript was made with it.
    pub transcript: bool,
}

impl RecordingEvent {
    /// What tells the messages of one recording apart from another one's.
    pub fn key(&self) -> String {
        self.call_id
            .clone()
            .or_else(|| self.document_id.clone())
            .unwrap_or_else(|| self.title.clone())
    }
}

/// The meeting event a message is, `None` for other messages and unreadable ones.
pub fn parse_meeting_event(message_type: &str, content: &str) -> Option<MeetingEvent> {
    if message_type.starts_with(CALL_EVENT) {
        parse_call(content).map(MeetingEvent::Call)
    } else if message_type == RECORDING_MESSAGE {
        parse_recording(content).map(MeetingEvent::Recording)
    } else {
        parse_transcript_message(message_type, content)
            .map(|call_id| MeetingEvent::Transcript { call_id })
    }
}

/// The text of the first `<name>…</name>`, entities decoded.
fn element_text(content: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let start = content.find(&open)? + open.len();
    let end = content[start..].find(&format!("</{name}>"))?;
    Some(unescape_xml(content[start..start + end].trim()))
}

fn parse_call(content: &str) -> Option<CallEvent> {
    let kind = element_text(content, "callEventType").unwrap_or_default();
    let ended = content.contains("<ended/>") || kind.eq_ignore_ascii_case("callEnded");
    let started = kind.eq_ignore_ascii_case("callStarted");
    if !ended && !started {
        return None;
    }
    let call_id = element_text(content, "callId").unwrap_or_default();
    let mut people = Vec::new();
    let mut count = 0;
    let mut longest: Option<i64> = None;
    for part in content.split("<part ").skip(1) {
        let part = part.split("</part>").next().unwrap_or(part);
        count += 1;
        if let Some(secs) = element_text(part, "duration").and_then(|d| d.parse::<i64>().ok()) {
            longest = Some(longest.map_or(secs, |l| l.max(secs)));
        }
        let mri = tag_attr(&format!(" {part}"), "identity")
            .or_else(|| element_text(part, "name"))
            .unwrap_or_default();
        let name = element_text(part, "displayName").unwrap_or_default();
        if !name.is_empty() && !mri.starts_with("28:") {
            people.push(CallPart { mri, name });
        }
    }
    let count = first_attr(content, "partlist", "count")
        .and_then(|c| c.parse().ok())
        .unwrap_or(count);
    Some(CallEvent {
        call_id,
        ended,
        duration_secs: longest.filter(|_| ended),
        people,
        count,
        recap: element_text(content, "isRecapEnabled")
            .is_some_and(|v| v.eq_ignore_ascii_case("true")),
    })
}

fn parse_recording(content: &str) -> Option<RecordingEvent> {
    let status = first_attr(content, "RecordingStatus", "status")?;
    let state = if status.eq_ignore_ascii_case("success") {
        RecordingState::Ready
    } else if status.to_ascii_lowercase().contains("fail")
        || status.to_ascii_lowercase().contains("error")
    {
        RecordingState::Failed
    } else {
        RecordingState::InProgress
    };
    let https = |u: String| u.starts_with("https://").then_some(u);
    let items = start_tags(content, "item");
    let item_uri = |kind: &str| {
        items
            .iter()
            .find(|t| tag_attr(t, "type").as_deref() == Some(kind))
            .and_then(|t| tag_attr(t, "uri"))
            .and_then(https)
    };
    let url = item_uri("onedriveForBusinessVideo")
        .or_else(|| first_attr(content, "a", "href").and_then(https));
    let file = first_attr(content, "OriginalName", "v").unwrap_or_default();
    let file = file.strip_suffix(".mp4").unwrap_or(&file).trim().to_owned();
    let title = if file.is_empty() {
        element_text(content, "Title").unwrap_or_default()
    } else {
        file
    };
    let id_of = |kind: &str| {
        start_tags(content, "Id")
            .into_iter()
            .find(|t| tag_attr(t, "type").as_deref() == Some(kind))
            .and_then(|t| tag_attr(t, "value"))
            .filter(|v| !v.is_empty())
    };
    Some(RecordingEvent {
        state,
        title,
        url,
        duration_ms: first_attr(content, "RecordingContent", "duration")
            .as_deref()
            .and_then(recording_length_ms)
            .filter(|ms| *ms > 0),
        call_id: call_id_of(content),
        document_id: id_of("AMSDocumentID"),
        initiator: first_attr(content, "RecordingInitiatorId", "value").filter(|v| !v.is_empty()),
        transcript: first_attr(content, "RecordingContent", "contentTypes")
            .is_some_and(|t| t.to_ascii_lowercase().contains("transcript")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENDED: &str = r#"<ended/><partlist alt="" count="4"><part identity="8:orgid:aaaa-1"><name>8:orgid:aaaa-1</name><displayName>Ada Lovelace</displayName><duration>3792</duration></part><part identity="28:bot-1"><name>28:bot-1</name><displayName></displayName><duration>3792</duration></part><part identity="8:orgid:bbbb-2"><name>8:orgid:bbbb-2</name><displayName>Grace Hopper</displayName><duration>3700</duration></part><part identity="8:teamsvisitor:cc3"><name>8:teamsvisitor:cc3</name><displayName>Guest &amp; Co</displayName><duration>12</duration></part></partlist><meetingDetails><meetingDetails><organizerUpn>ada@example.invalid</organizerUpn><meetingType>Scheduled</meetingType><isRecapEnabled>true</isRecapEnabled></meetingDetails></meetingDetails><callEventType>callEnded</callEventType>
<conversationStartTime>11/10/2026 10:27:55</conversationStartTime>
<callId>call-1</callId>
<wasGcpBotInvited>False</wasGcpBotInvited>"#;

    const STARTED: &str = r#"<partlist alt =""></partlist><meetingDetails><meetingDetails><meetingType>Scheduled</meetingType></meetingDetails></meetingDetails><callEventType>callStarted</callEventType>
<callId>call-1</callId>
"#;

    fn recording(status: &str, href: &str, duration: &str, types: &str) -> String {
        format!(
            r#"<URIObject format_version="1.1" type="Video.2/CallRecording.1" url_thumbnail="" uri="" version="1.0"><RecordingStatus status="{status}" code="200" shortChunk="False"><amsErrorResult /></RecordingStatus><Title>Weekly sync</Title><a href="{href}">Play</a><OriginalName v="Weekly sync-20261110_1230-Meeting Recording.mp4" /><RecordingInitiatorId value="8:orgid:aaaa-1" isAutoRecording="False" /><Identifiers><Id type="callId" value="call-1" /><Id type="chunkIndex" value="0" /><Id type="AMSDocumentID" value="0-doc-1" /></Identifiers><RecordingContent contentTypes="{types}" timestamp="2026-11-10T10:35:11Z" duration="{duration}" canVideoExpire="True"><item type="video" uri="" /><item type="onedriveForBusinessVideo" uri="{href}" driveId="b!d" driveItemId="01A" /></RecordingContent></URIObject>"#
        )
    }

    #[test]
    fn an_ended_call_lists_the_named_people_and_lasts_the_longest_stay() {
        let Some(MeetingEvent::Call(c)) = parse_meeting_event("Event/Call", ENDED) else {
            panic!("a call");
        };
        assert!(c.ended);
        assert_eq!(c.call_id, "call-1");
        assert_eq!(c.duration_secs, Some(3792));
        assert_eq!(c.count, 4);
        assert!(c.recap);
        let names: Vec<_> = c.people.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Ada Lovelace", "Grace Hopper", "Guest & Co"]);
        assert_eq!(c.people[0].mri, "8:orgid:aaaa-1");
    }

    #[test]
    fn a_started_call_has_no_people_and_no_length() {
        let Some(MeetingEvent::Call(c)) = parse_meeting_event("Event/Call", STARTED) else {
            panic!("a call");
        };
        assert!(!c.ended);
        assert_eq!(c.call_id, "call-1");
        assert_eq!(c.duration_secs, None);
        assert!(c.people.is_empty());
        assert_eq!(c.count, 0);
    }

    #[test]
    fn a_finished_recording_has_its_link_title_and_length() {
        let xml = recording(
            "Success",
            "https://contoso-my.sharepoint.com/:v:/g/rec",
            "1:03:12.5",
            "Recording+Transcript",
        );
        let Some(MeetingEvent::Recording(r)) =
            parse_meeting_event("RichText/Media_CallRecording", &xml)
        else {
            panic!("a recording");
        };
        assert_eq!(r.state, RecordingState::Ready);
        assert_eq!(r.title, "Weekly sync-20261110_1230-Meeting Recording");
        assert_eq!(
            r.url.as_deref(),
            Some("https://contoso-my.sharepoint.com/:v:/g/rec")
        );
        assert_eq!(r.duration_ms, Some(3_792_500));
        assert_eq!(r.call_id.as_deref(), Some("call-1"));
        assert_eq!(r.document_id.as_deref(), Some("0-doc-1"));
        assert_eq!(r.initiator.as_deref(), Some("8:orgid:aaaa-1"));
        assert!(r.transcript);
        assert_eq!(r.key(), "call-1");
    }

    #[test]
    fn a_recording_in_progress_or_failed_has_no_link() {
        for (status, state) in [
            ("Initial", RecordingState::InProgress),
            ("ChunkFinished", RecordingState::InProgress),
            ("Failure", RecordingState::Failed),
        ] {
            let xml = recording(status, "", "0:00:00", "Recording");
            let Some(MeetingEvent::Recording(r)) =
                parse_meeting_event("RichText/Media_CallRecording", &xml)
            else {
                panic!("a recording");
            };
            assert_eq!(r.state, state, "{status}");
            assert_eq!(r.url, None);
            assert_eq!(r.duration_ms, None);
            assert!(!r.transcript);
        }
    }

    #[test]
    fn a_transcript_names_its_call() {
        let content = r#"{\"scopeId\":\"call-1\",\"callId\":\"call-1\",\"isExportedToOdsp\":true}"#;
        assert_eq!(
            parse_meeting_event("RichText/Media_CallTranscript", content),
            Some(MeetingEvent::Transcript {
                call_id: "call-1".into()
            })
        );
    }

    #[test]
    fn other_and_unreadable_messages_are_none() {
        assert_eq!(parse_meeting_event("RichText/Html", "<p>hi</p>"), None);
        assert_eq!(parse_meeting_event("Event/Call", "garbage"), None);
        assert_eq!(
            parse_meeting_event("RichText/Media_CallRecording", "<p>no status</p>"),
            None
        );
    }
}
