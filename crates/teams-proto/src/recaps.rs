//! Meeting Recaps (plans/meeting-recaps.md R1): past meetings that have a recording or a
//! transcript, and what the user can read of them without an AI license.
//!
//! Sources, in the order kelma tries them:
//! 1. The meeting content service (MCPS), `GET <mcps>/user/meetings/contents?top=&startDateTime=
//!    &endDateTime=&includeTypes=Recording,TranscriptV2`, documented by the mautrix OpenAPI from a
//!    captured web client. Each entry pairs `threadInfo` (thread id, iCalUid, subject, organizer,
//!    start, end) with `resources` (`Recording`, `TranscriptV2`, `AISummary`, each with a
//!    `metadata` of `driveId`, `driveItemId`, `webUrl`, `transcriptId`). UNVERIFIED here: the
//!    capture strips the bearer, so the audience is a guess (the Teams host credentials), and the
//!    `<geo>` of the host is derived from the middle tier's region.
//! 2. The files in the user's OneDrive `Recordings` folder (Graph, known to work), grouped by
//!    the name Teams gives them, `<subject>-<yyyymmdd_hhmmss>UTC-Meeting Recording.mp4`.
//!
//! A transcript is read from SharePoint (`_api/v2.1/drives/<d>/items/<i>/media/transcripts`,
//! named by the plan; the `streamContent` path to the text is UNVERIFIED), or, when it is a
//! `.vtt` file of the folder, from Graph `/drives/<d>/items/<i>/content`. The AI notes, tasks and
//! chapters (`AISummary`) need Teams Premium or Copilot and are not read.

use crate::broker::Audience;
use crate::channel_tabs::{ChannelTab, parse_tabs};
use crate::client::Client;
use crate::error::{Error, Result};
use crate::graph::{graph_url, ms_of, str_of, values};
use crate::time::{iso_utc, parse_iso_ms};
use reqwest::Method;
use serde_json::Value;

/// What a recap resource is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Recording,
    Transcript,
    /// AI notes, tasks and chapters (license-gated).
    AiSummary,
}

/// A recording, transcript or AI summary of a meeting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecapResource {
    pub kind: ResourceKind,
    /// A file name when the source gave one.
    pub name: String,
    /// Opens it in the browser (SharePoint, Stream).
    pub web_url: Option<String>,
    pub drive_id: Option<String>,
    pub item_id: Option<String>,
    pub transcript_id: Option<String>,
}

/// A past meeting with something to watch or read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecapMeeting {
    /// Stable key: the thread id with the iCalUid, else the subject and start.
    pub key: String,
    pub thread_id: String,
    pub subject: String,
    pub organizer: String,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub resources: Vec<RecapResource>,
}

impl RecapMeeting {
    pub fn first(&self, kind: ResourceKind) -> Option<&RecapResource> {
        self.resources.iter().find(|r| r.kind == kind)
    }

    /// Recorded or transcribed (an AI summary alone is not listed).
    pub fn has_content(&self) -> bool {
        self.first(ResourceKind::Recording).is_some()
            || self.first(ResourceKind::Transcript).is_some()
    }
}

fn resource_kind(t: &str) -> Option<ResourceKind> {
    match t {
        "Recording" => Some(ResourceKind::Recording),
        "TranscriptV2" | "Transcript" => Some(ResourceKind::Transcript),
        "AISummary" => Some(ResourceKind::AiSummary),
        _ => None,
    }
}

fn parse_resource(r: &Value) -> Option<RecapResource> {
    let m = &r["metadata"];
    Some(RecapResource {
        kind: resource_kind(r["type"].as_str()?)?,
        name: String::new(),
        web_url: str_of(m, "webUrl").filter(|u| u.starts_with("https://")),
        drive_id: str_of(m, "driveId"),
        item_id: str_of(m, "driveItemId"),
        transcript_id: str_of(m, "transcriptId"),
    })
}

/// The meetings of an MCPS list answer, newest first. Entries without a recording or
/// transcript are left out.
pub fn parse_meetings(v: &Value) -> Vec<RecapMeeting> {
    let mut out: Vec<RecapMeeting> = values(v)
        .iter()
        .filter_map(|e| {
            let t = &e["threadInfo"];
            let thread_id = str_of(t, "threadId").unwrap_or_default();
            let subject = str_of(t, "subject").unwrap_or_default();
            let start_ms = ms_of(t, "start");
            let key = match str_of(t, "iCalUid") {
                Some(u) => format!("{thread_id}|{u}"),
                None => format!("{subject}|{}", start_ms.unwrap_or(0)),
            };
            let m = RecapMeeting {
                key,
                thread_id,
                subject,
                organizer: str_of(&t["organizer"], "emailAddress")
                    .or_else(|| str_of(&t["organizer"]["emailAddress"], "name"))
                    .unwrap_or_default(),
                start_ms,
                end_ms: ms_of(t, "end"),
                resources: e["resources"]
                    .as_array()
                    .map(|a| a.iter().filter_map(parse_resource).collect())
                    .unwrap_or_default(),
            };
            m.has_content().then_some(m)
        })
        .collect();
    out.sort_by_key(|m| std::cmp::Reverse(m.start_ms));
    out
}

/// The MCPS host for a middle tier URL: `https://teams.microsoft.com/api/mt/emea-02` gives
/// `https://teams.microsoft.com/api/mcps/eu`. The region names are a guess from the one
/// documented pair (`eu`); UNVERIFIED for the others.
pub fn mcps_base(middle_tier: &str) -> String {
    let mt = middle_tier.trim_end_matches('/');
    let (host, region) = mt.split_once("/api/mt/").unwrap_or((mt, ""));
    let part = region.split(['-', '/']).next().unwrap_or_default();
    let geo = match part {
        "emea" => "eu",
        "amer" | "noam" => "us",
        "apac" => "ap",
        other if !other.is_empty() => other,
        _ => "eu",
    };
    format!("{host}/api/mcps/{geo}")
}

/// The list request for meetings that ended between `from_ms` and `to_ms`.
pub fn contents_url(base: &str, from_ms: i64, to_ms: i64, top: u32) -> String {
    format!(
        "{base}/user/meetings/contents?top={top}&startDateTime={}&endDateTime={}\
         &includeTypes=Recording,TranscriptV2",
        iso_utc(from_ms),
        iso_utc(to_ms)
    )
}

/// A file of the OneDrive `Recordings` folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingFile {
    pub id: String,
    pub drive_id: String,
    pub name: String,
    pub web_url: String,
    pub modified_ms: i64,
}

/// `20260105_100000` at the start of `s` as UTC milliseconds.
fn stamp_ms(s: &str) -> Option<i64> {
    let b = s.get(..15)?;
    if b.as_bytes()[8] != b'_' {
        return None;
    }
    let (d, t) = (&b[..8], &b[9..]);
    parse_iso_ms(&format!(
        "{}-{}-{}T{}:{}:{}Z",
        &d[..4],
        &d[4..6],
        &d[6..],
        &t[..2],
        &t[2..4],
        &t[4..]
    ))
}

/// Groups the files of the `Recordings` folder into meetings: Teams names a recording
/// `<subject>-<yyyymmdd_hhmmss>UTC-Meeting Recording.mp4` and a transcript
/// `…-Meeting Transcript.vtt` (or `.docx`). Other files are ignored. Newest first.
pub fn meetings_from_files(files: &[RecordingFile]) -> Vec<RecapMeeting> {
    let mut out: Vec<RecapMeeting> = Vec::new();
    for f in files {
        let cut = f.name.len().saturating_sub(4);
        let kind = match f.name.get(cut..) {
            Some(e) if e.eq_ignore_ascii_case(".mp4") => ResourceKind::Recording,
            Some(e) if e.eq_ignore_ascii_case(".vtt") => ResourceKind::Transcript,
            _ => continue,
        };
        let stem = &f.name[..cut];
        // Drop `-Meeting Recording` / `-Meeting Transcript`.
        let stem = stem
            .rfind("-Meeting ")
            .map_or(stem, |i| &stem[..i])
            .to_owned();
        let (subject, start) = match stem.rsplit_once('-') {
            Some((subject, stamp)) if stamp_ms(stamp).is_some() => {
                (subject.to_owned(), stamp_ms(stamp))
            }
            _ => (stem.clone(), None),
        };
        let start_ms = start.or(Some(f.modified_ms));
        let key = format!("{subject}|{}", start.unwrap_or(f.modified_ms));
        let res = RecapResource {
            kind,
            name: f.name.clone(),
            web_url: Some(f.web_url.clone()).filter(|u| u.starts_with("https://")),
            drive_id: Some(f.drive_id.clone()).filter(|d| !d.is_empty()),
            item_id: Some(f.id.clone()),
            transcript_id: None,
        };
        match out.iter_mut().find(|m| m.key == key) {
            Some(m) => m.resources.push(res),
            None => out.push(RecapMeeting {
                key,
                thread_id: String::new(),
                subject,
                organizer: String::new(),
                start_ms,
                end_ms: None,
                resources: vec![res],
            }),
        }
    }
    out.sort_by_key(|m| std::cmp::Reverse(m.start_ms));
    out
}

/// One line of a transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    pub start_ms: i64,
    pub speaker: String,
    pub text: String,
}

fn cue_time(s: &str) -> Option<i64> {
    // `hh:mm:ss.mmm` or `mm:ss.mmm`.
    let (hms, frac) = s.trim().split_once('.')?;
    let parts: Vec<i64> = hms.split(':').filter_map(|p| p.parse().ok()).collect();
    let secs = match parts[..] {
        [h, m, s] => h * 3600 + m * 60 + s,
        [m, s] => m * 60 + s,
        _ => return None,
    };
    let ms: String = frac.chars().take(3).collect();
    Some(secs * 1000 + format!("{ms:0<3}").parse::<i64>().ok()?)
}

/// Reads WebVTT: each cue's start, its speaker (`<v Name>` voice tag) and its text.
pub fn parse_vtt(vtt: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    let mut lines = vtt.lines().peekable();
    while let Some(line) = lines.next() {
        let Some((from, _)) = line.split_once("-->") else {
            continue;
        };
        let Some(start_ms) = cue_time(from) else {
            continue;
        };
        let mut body = String::new();
        while let Some(l) = lines.peek() {
            if l.trim().is_empty() {
                break;
            }
            body.push_str(l.trim());
            body.push(' ');
            lines.next();
        }
        let (speaker, text) = match body.strip_prefix("<v ").and_then(|b| b.split_once('>')) {
            Some((name, rest)) => (name.trim().to_owned(), rest.to_owned()),
            None => (String::new(), body.clone()),
        };
        let text = strip_tags(&text);
        if !text.is_empty() {
            cues.push(Cue {
                start_ms,
                speaker,
                text,
            });
        }
    }
    cues
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The transcript as lines: `[0:01:05] Ada: text` (the speaker is left out when unknown).
pub fn transcript_text(cues: &[Cue]) -> String {
    cues.iter()
        .map(|c| {
            let s = c.start_ms / 1000;
            let at = format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60);
            if c.speaker.is_empty() {
                format!("[{at}] {}", c.text)
            } else {
                format!("[{at}] {}: {}", c.speaker, c.text)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `https://<tenant>.sharepoint.com` of a SharePoint web URL.
pub fn sharepoint_origin(web_url: &str) -> Option<String> {
    let u = url::Url::parse(web_url).ok()?;
    let host = u.host_str()?;
    (u.scheme() == "https" && host.ends_with(".sharepoint.com")).then(|| format!("https://{host}"))
}

/// `GET <origin>/_api/v2.1/drives/<d>/items/<i>/media/transcripts`, which lists a recording's
/// transcripts (named by plans/meeting-recaps.md).
pub fn transcripts_url(origin: &str, drive_id: &str, item_id: &str) -> String {
    format!("{origin}/_api/v2.1/drives/{drive_id}/items/{item_id}/media/transcripts")
}

/// The ids of the transcripts a [`transcripts_url`] answer lists.
pub fn parse_transcript_ids(v: &Value) -> Vec<String> {
    values(v).iter().filter_map(|t| str_of(t, "id")).collect()
}

/// The text of one transcript as WebVTT. UNVERIFIED path: `…/transcripts/<id>/streamContent`.
pub fn transcript_content_url(origin: &str, drive_id: &str, item_id: &str, id: &str) -> String {
    format!(
        "{}/{id}/streamContent?format=vtt",
        transcripts_url(origin, drive_id, item_id)
    )
}

/// The meeting-notes tab of a meeting chat's tabs (named "Notes"): where to open it.
pub fn notes_link(tabs: &[ChannelTab]) -> Option<String> {
    tabs.iter()
        .find(|t| t.name.eq_ignore_ascii_case("notes") && t.open_url.starts_with("https://"))
        .map(|t| t.open_url.clone())
}

impl Client {
    /// Meetings that ended in the last `days` days and have a recording or transcript (MCPS,
    /// UNVERIFIED audience). `mcps_base` comes from [`mcps_base`] on the middle tier.
    pub async fn recap_meetings(&self, days: i64) -> Result<Vec<RecapMeeting>> {
        let now = crate::time::now_ms();
        let base = mcps_base(&self.middle_tier().await);
        let url = contents_url(&base, now - days * 86_400_000, now, 50);
        let v = self
            .json("meeting contents", Method::GET, &url, None)
            .await?;
        Ok(parse_meetings(&v))
    }

    /// A transcript of a recording as WebVTT, from SharePoint (see [`transcripts_url`]).
    pub async fn sharepoint_transcript(
        &self,
        web_url: &str,
        drive_id: &str,
        item_id: &str,
    ) -> Result<String> {
        let origin = sharepoint_origin(web_url)
            .ok_or_else(|| Error::Protocol("the recording is not on SharePoint".into()))?;
        let host = origin.trim_start_matches("https://").to_owned();
        let aud = Audience::SharePoint(host);
        let list = self
            .json_as(
                "transcripts",
                &aud,
                Method::GET,
                &transcripts_url(&origin, drive_id, item_id),
                None,
            )
            .await?;
        let id = parse_transcript_ids(&list)
            .into_iter()
            .next()
            .ok_or_else(|| Error::Protocol("this recording has no transcript".into()))?;
        let url = transcript_content_url(&origin, drive_id, item_id, &id);
        let (status, text) = self.request_as(&aud, Method::GET, &url, None, &[]).await?;
        if !(200..300).contains(&status) {
            return Err(Error::status("transcript", status, &text));
        }
        Ok(text)
    }

    /// The text of a file in a drive (a `.vtt` transcript in the `Recordings` folder).
    pub async fn drive_file_text(&self, drive_id: &str, item_id: &str) -> Result<String> {
        let url = graph_url(&["drives", drive_id, "items", item_id, "content"], "");
        let (status, text) = self
            .request_as(&Audience::Graph, Method::GET, &url, None, &[])
            .await?;
        if !(200..300).contains(&status) {
            return Err(Error::status("transcript file", status, &text));
        }
        Ok(text)
    }

    /// The tabs of a meeting chat (`GET /chats/{id}/tabs?$expand=teamsApp`).
    pub async fn chat_tabs(&self, chat_id: &str) -> Result<Vec<ChannelTab>> {
        let url = graph_url(&["chats", chat_id, "tabs"], "$expand=teamsApp");
        let v = self
            .json_as("meeting tabs", &Audience::Graph, Method::GET, &url, None)
            .await?;
        Ok(parse_tabs(&v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn answer() -> Value {
        json!({"value": [
            {"threadInfo": {"threadId": "19:meeting_old@thread.v2", "iCalUid": "uid-1",
                "subject": "Weekly sync", "start": "2026-11-10T10:00:00.0000000Z",
                "end": "2026-11-10T10:30:00Z",
                "organizer": {"emailAddress": "ada@example.invalid"}},
             "resources": [
                {"type": "Recording", "metadata": {"driveId": "b!d", "driveItemId": "01A",
                    "webUrl": "https://contoso-my.sharepoint.com/x/rec.mp4"}},
                {"type": "TranscriptV2", "metadata": {"transcriptId": "tr1", "driveId": "b!d",
                    "driveItemId": "01A"}},
                {"type": "AISummary", "metadata": {}},
                {"type": "Whatever"}]},
            {"threadInfo": {"threadId": "19:meeting_new@thread.v2", "subject": "Retro",
                "start": "2026-11-12T09:00:00Z"},
             "resources": [{"type": "TranscriptV2", "metadata": {"webUrl": "http://insecure.invalid/t"}}]},
            {"threadInfo": {"subject": "Nothing recorded"}, "resources": null},
            {"threadInfo": {"subject": "Only AI"}, "resources": [{"type": "AISummary"}]}
        ]})
    }

    #[test]
    fn meetings_with_a_recording_or_transcript_come_newest_first() {
        let m = parse_meetings(&answer());
        let subjects: Vec<&str> = m.iter().map(|m| m.subject.as_str()).collect();
        assert_eq!(subjects, ["Retro", "Weekly sync"]);
        let sync = &m[1];
        assert_eq!(sync.thread_id, "19:meeting_old@thread.v2");
        assert_eq!(sync.key, "19:meeting_old@thread.v2|uid-1");
        assert_eq!(sync.organizer, "ada@example.invalid");
        assert_eq!(sync.start_ms, Some(1_794_304_800_000));
        assert_eq!(sync.resources.len(), 3, "the unknown type is dropped");
        let rec = sync.first(ResourceKind::Recording).unwrap();
        assert_eq!(rec.drive_id.as_deref(), Some("b!d"));
        assert_eq!(rec.item_id.as_deref(), Some("01A"));
        assert!(sync.first(ResourceKind::AiSummary).is_some());
        assert_eq!(
            sync.first(ResourceKind::Transcript)
                .unwrap()
                .transcript_id
                .as_deref(),
            Some("tr1")
        );
        // A link that is not https is never kept.
        assert_eq!(m[0].first(ResourceKind::Transcript).unwrap().web_url, None);
    }

    #[test]
    fn the_mcps_host_follows_the_middle_tier_region() {
        assert_eq!(
            mcps_base("https://teams.microsoft.com/api/mt/emea-02/"),
            "https://teams.microsoft.com/api/mcps/eu"
        );
        assert_eq!(
            mcps_base("https://teams.microsoft.com/api/mt/amer"),
            "https://teams.microsoft.com/api/mcps/us"
        );
        assert_eq!(
            contents_url("https://h/api/mcps/eu", 0, 86_400_000, 20),
            "https://h/api/mcps/eu/user/meetings/contents?top=20&startDateTime=1970-01-01T00:00:00Z\
             &endDateTime=1970-01-02T00:00:00Z&includeTypes=Recording,TranscriptV2"
        );
    }

    fn file(name: &str, id: &str) -> RecordingFile {
        RecordingFile {
            id: id.into(),
            drive_id: "b!me".into(),
            name: name.into(),
            web_url: format!("https://contoso-my.sharepoint.com/{id}"),
            modified_ms: 5,
        }
    }

    #[test]
    fn recordings_folder_files_group_into_meetings() {
        let m = meetings_from_files(&[
            file("Weekly sync-20260105_100000UTC-Meeting Recording.mp4", "r1"),
            file(
                "Weekly sync-20260105_100000UTC-Meeting Transcript.vtt",
                "t1",
            ),
            file("Retro-20260112_090000UTC-Meeting Recording.mp4", "r2"),
            file("Notes.docx", "n"),
            file("Standalone.mp4", "r3"),
        ]);
        let got: Vec<(&str, usize, Option<i64>)> = m
            .iter()
            .map(|m| (m.subject.as_str(), m.resources.len(), m.start_ms))
            .collect();
        assert_eq!(
            got,
            [
                ("Retro", 1, Some(1_768_208_400_000)),
                ("Weekly sync", 2, Some(1_767_607_200_000)),
                ("Standalone", 1, Some(5)),
            ]
        );
        let sync = m.iter().find(|m| m.subject == "Weekly sync").unwrap();
        assert_eq!(
            sync.first(ResourceKind::Transcript)
                .unwrap()
                .item_id
                .as_deref(),
            Some("t1")
        );
        assert!(sync.has_content());
    }

    #[test]
    fn vtt_becomes_speaker_lines() {
        let vtt = "WEBVTT\n\n00:00:01.500 --> 00:00:04.000\n<v Ada Lovelace>Hello &amp; welcome</v>\n\n\
                   00:01:05.000 --> 00:01:08.000\n<v Sam>Two <c>lines</c>\nin a cue</v>\n\n\
                   2\n00:00:09.2 --> 00:00:10.000\nNo speaker here\n";
        let cues = parse_vtt(vtt);
        assert_eq!(cues.len(), 3);
        assert_eq!(cues[0].start_ms, 1_500);
        assert_eq!(cues[0].speaker, "Ada Lovelace");
        assert_eq!(cues[0].text, "Hello & welcome");
        assert_eq!(cues[1].text, "Two lines in a cue");
        assert_eq!(cues[2].speaker, "");
        assert_eq!(cues[2].start_ms, 9_200);
        assert_eq!(
            transcript_text(&cues),
            "[0:00:01] Ada Lovelace: Hello & welcome\n[0:01:05] Sam: Two lines in a cue\n[0:00:09] No speaker here"
        );
        assert!(parse_vtt("not a transcript").is_empty());
    }

    #[test]
    fn sharepoint_urls_and_the_notes_tab() {
        assert_eq!(
            sharepoint_origin("https://contoso-my.sharepoint.com/personal/x/rec.mp4").as_deref(),
            Some("https://contoso-my.sharepoint.com")
        );
        assert_eq!(sharepoint_origin("https://evil.invalid/x"), None);
        assert_eq!(sharepoint_origin("http://a.sharepoint.com/x"), None);
        assert_eq!(
            transcript_content_url("https://a.sharepoint.com", "b!d", "01A", "tr1"),
            "https://a.sharepoint.com/_api/v2.1/drives/b!d/items/01A/media/transcripts/tr1/streamContent?format=vtt"
        );
        assert_eq!(
            parse_transcript_ids(&json!({"value": [{"id": "a"}, {"name": "x"}]})),
            ["a"]
        );
        let tabs = parse_tabs(&json!({"value": [
            {"displayName": "Whiteboard", "webUrl": "https://example.invalid/w"},
            {"displayName": "Notes", "webUrl": "https://example.invalid/n"}]}));
        assert_eq!(
            notes_link(&tabs).as_deref(),
            Some("https://example.invalid/n")
        );
        assert_eq!(notes_link(&tabs[..1]), None);
    }
}
