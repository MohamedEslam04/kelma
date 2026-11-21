//! Meeting Recaps, built the way Teams builds it.
//!
//! The home lists the user's online meetings of the last 30 days from kelma's own calendar
//! (canceled ones too, as `Canceled: …`), grouped by week, each with a Recap button. Nothing in
//! the list depends on a server answer beyond the calendar sync.
//!
//! A meeting's recap reads its meeting chat (`19:meeting_…@thread.v2`, from the event's join
//! link): the recordings the chat announces (`RichText/Media_CallRecording`, see
//! `teams_proto::recaps::parse_recording_message`) are the parts, the files sent there are the
//! shared files, and a transcript message or the recording's own `contentTypes` says a part has a
//! transcript. The recording usually sits in the organizer's OneDrive, which is why the user's
//! own `Recordings` folder had none. The video opens in the browser (kelma has no player); Share
//! copies its link. The transcript text is read from SharePoint when kelma can, else the tab
//! offers to open it in the browser. The meeting content service (MCPS) only adds recordings the
//! chat does not show, and its failures (a 401 today) are logged, never shown. The AI summary and
//! the Speakers timeline are Copilot features and are left out.

use super::view::{Nav, RecapDetail, RecapRow, RecapsData, Row};
use crate::backend::Engine;
use ::calendar::Event;
use std::sync::Arc;
use store::MessageRow;
use teams_proto::recaps::{
    RecapMeeting, RecapResource, ResourceKind, parse_recording_message, parse_transcript_message,
};
use teams_proto::time::now_ms;

const DAY_MS: i64 = 86_400_000;
const WINDOW_DAYS: i64 = 30;
const FOOTER: &str = "Loaded all meetings in the last 30 days.";
const NO_MEETINGS: &str = "No online meetings in the last 30 days.";
const NO_MATCH: &str = "No meeting matches your search.";
const STILL_SYNCING: &str = "Your calendar is still loading. Refresh in a moment.";
const NO_CHAT: &str =
    "This meeting has no meeting chat, so there is no recording or shared file to show.";
const NO_RECORDING: &str = "No recording was found in the meeting chat.";
const CHAT_UNREADABLE: &str =
    "The meeting chat could not be read right now, so its recordings and files are not shown.";
const TRANSCRIPT_UNREADABLE: &str =
    "The transcript could not be read here. Open it in your browser.";

/// A meeting of the home list.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Meeting {
    /// The calendar event id (the thread id for a chat's Recap tab without an event).
    pub key: String,
    pub title: String,
    pub canceled: bool,
    /// Zero when unknown (a meeting chat with no event in the calendar).
    pub start_ms: i64,
    pub end_ms: i64,
    pub organizer: String,
    /// The meeting chat, empty when the join link names none.
    pub thread_id: String,
    pub recurring: bool,
    /// `This week`, `Last week`, `2 weeks ago`, …
    pub group: String,
}

/// `Canceled: ` in front of a canceled meeting's subject, as Teams shows it (once: Outlook may
/// have put it there already).
fn shown_title(e: &Event) -> String {
    let subject = match e.subject.trim() {
        "" => "(No subject)",
        s => s,
    };
    let lower = subject.to_lowercase();
    if e.is_cancelled && !lower.starts_with("canceled:") && !lower.starts_with("cancelled:") {
        format!("Canceled: {subject}")
    } else {
        subject.to_owned()
    }
}

/// The week heading of a meeting that started at `start_ms`; `week_start_ms` is 00:00 of the
/// first day of this week.
fn week_label(start_ms: i64, week_start_ms: i64) -> String {
    if start_ms >= week_start_ms {
        return "This week".into();
    }
    match (week_start_ms - start_ms - 1) / (7 * DAY_MS) + 1 {
        1 => "Last week".into(),
        n => format!("{n} weeks ago"),
    }
}

/// The online meetings among the calendar's events that started in the last 30 days (and not
/// after now), newest first, each with its week heading.
pub(super) fn meetings_from_events(
    events: &[Event],
    now_ms: i64,
    week_start_ms: i64,
) -> Vec<Meeting> {
    let from = now_ms - WINDOW_DAYS * DAY_MS;
    let mut out: Vec<Meeting> = events
        .iter()
        .filter(|e| (e.is_online_meeting || e.join_url.is_some()) && !e.is_all_day)
        .filter(|e| e.start_ms >= from && e.start_ms <= now_ms)
        .map(|e| Meeting {
            key: e.id.clone(),
            title: shown_title(e),
            canceled: e.is_cancelled,
            start_ms: e.start_ms,
            end_ms: e.end_ms,
            organizer: if e.organizer_name.is_empty() {
                e.organizer_email.clone()
            } else {
                e.organizer_name.clone()
            },
            thread_id: e
                .join_url
                .as_deref()
                .and_then(::calendar::meeting_chat_thread_id)
                .unwrap_or_default(),
            recurring: e.is_recurring,
            group: week_label(e.start_ms, week_start_ms),
        })
        .collect();
    out.sort_by(|a, b| b.start_ms.cmp(&a.start_ms).then_with(|| a.key.cmp(&b.key)));
    out.dedup_by(|a, b| a.key == b.key);
    out
}

/// A recording of the meeting: one "Part" of the recap.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Part {
    pub name: String,
    pub web_url: Option<String>,
    pub drive_id: Option<String>,
    pub item_id: Option<String>,
    /// When the chat announced it.
    pub at_ms: i64,
    pub duration_ms: Option<i64>,
    pub transcript: bool,
}

impl Part {
    fn resource(&self) -> RecapResource {
        RecapResource {
            kind: ResourceKind::Recording,
            name: self.name.clone(),
            web_url: self.web_url.clone(),
            drive_id: self.drive_id.clone(),
            item_id: self.item_id.clone(),
            transcript_id: None,
        }
    }
}

/// The finished recordings a meeting chat announces, oldest first (Part 1 is the first).
/// `window` keeps those posted during one occurrence of a recurring meeting, whose occurrences
/// share one chat. A part has a transcript when its recording says so, or when a transcript
/// message names its call (or, naming none, the meeting has a single part).
pub(super) fn parts_from_chat(rows: &[MessageRow], window: Option<(i64, i64)>) -> Vec<Part> {
    let inside = |ms: i64| window.is_none_or(|(a, b)| ms >= a && ms <= b);
    let mut found: Vec<(Part, Option<String>)> = Vec::new();
    let mut transcripts: Vec<String> = Vec::new();
    for r in rows.iter().filter(|r| !r.deleted && inside(r.compose_ms)) {
        if let Some(rec) = parse_recording_message(&r.message_type, &r.content_html) {
            if rec.web_url.is_some() && found.iter().any(|(p, _)| p.web_url == rec.web_url) {
                continue;
            }
            found.push((
                Part {
                    name: rec.name,
                    web_url: rec.web_url,
                    drive_id: rec.drive_id,
                    item_id: rec.item_id,
                    at_ms: r.compose_ms,
                    duration_ms: rec.duration_ms,
                    transcript: rec.with_transcript,
                },
                rec.call_id,
            ));
        } else if let Some(call) = parse_transcript_message(&r.message_type, &r.content_html) {
            transcripts.push(call);
        }
    }
    found.sort_by_key(|(p, _)| p.at_ms);
    for call in transcripts {
        let named = found
            .iter()
            .position(|(_, c)| !call.is_empty() && c.as_deref() == Some(call.as_str()));
        match named {
            Some(i) => found[i].0.transcript = true,
            None if found.len() == 1 => found[0].0.transcript = true,
            None => {}
        }
    }
    found.into_iter().map(|(p, _)| p).collect()
}

/// A file sent in the meeting chat.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SharedFile {
    pub name: String,
    pub url: String,
    pub ext: String,
    pub by: String,
    pub at_ms: i64,
}

/// The files the meeting chat's messages carry (the `f` list the store keeps for each message),
/// newest first, once each, without the recordings themselves.
pub(super) fn files_from_chat(rows: &[MessageRow], parts: &[Part]) -> Vec<SharedFile> {
    let mut out: Vec<SharedFile> = Vec::new();
    for r in rows.iter().filter(|r| !r.deleted) {
        let props: serde_json::Value = serde_json::from_str(&r.reactions_json).unwrap_or_default();
        for f in props["f"].as_array().into_iter().flatten() {
            let (Some(name), Some(url)) = (f["name"].as_str(), f["url"].as_str()) else {
                continue;
            };
            if url.is_empty()
                || out.iter().any(|o| o.url == url)
                || parts.iter().any(|p| p.web_url.as_deref() == Some(url))
            {
                continue;
            }
            let ext = f["type"]
                .as_str()
                .filter(|t| !t.is_empty())
                .map(str::to_uppercase)
                .or_else(|| name.rsplit_once('.').map(|(_, e)| e.to_uppercase()))
                .unwrap_or_else(|| "FILE".into());
            out.push(SharedFile {
                name: name.to_owned(),
                url: url.to_owned(),
                ext,
                by: r.sender_name.clone(),
                at_ms: r.compose_ms,
            });
        }
    }
    out.sort_by_key(|f| std::cmp::Reverse(f.at_ms));
    out
}

/// Adds what the meeting content service lists for the meeting chat: its recordings when the
/// chat shows none (their messages may be older than the history read), and a transcript for a
/// single part.
pub(super) fn with_mcps(mut parts: Vec<Part>, thread: &str, mcps: &[RecapMeeting]) -> Vec<Part> {
    let Some(m) = mcps
        .iter()
        .find(|m| !thread.is_empty() && m.thread_id == thread)
    else {
        return parts;
    };
    if parts.is_empty() {
        parts = m
            .resources
            .iter()
            .filter(|r| r.kind == ResourceKind::Recording)
            .map(|r| Part {
                name: r.name.clone(),
                web_url: r.web_url.clone(),
                drive_id: r.drive_id.clone(),
                item_id: r.item_id.clone(),
                at_ms: m.start_ms.unwrap_or_default(),
                duration_ms: None,
                transcript: false,
            })
            .collect();
    }
    if m.first(ResourceKind::Transcript).is_some() && parts.len() == 1 {
        parts[0].transcript = true;
    }
    parts
}

/// `18 min`, `1 h 5 min`, `45 s`.
fn length(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s / 60),
        _ if s % 3600 / 60 == 0 => format!("{} h", s / 3600),
        _ => format!("{} h {} min", s / 3600, s % 3600 / 60),
    }
}

/// `Tue, Nov 18 · 10:00 – 10:30` in the time zone chosen in Settings.
fn when(m: &Meeting) -> String {
    use appearance::formats::{local, time_of};
    let (Some(s), Some(e)) = (local(m.start_ms), local(m.end_ms)) else {
        return String::new();
    };
    if m.start_ms <= 0 {
        return String::new();
    }
    format!(
        "{} · {} – {}",
        s.format("%a, %b %-d"),
        time_of(&s),
        time_of(&e)
    )
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum Tab {
    #[default]
    Notes,
    Transcript,
}

#[derive(Default)]
pub struct Recaps {
    loaded: bool,
    loading: bool,
    meetings: Vec<Meeting>,
    query: String,
    /// Why the list is empty (the calendar is still loading).
    note: String,
    /// What the meeting content service listed, when it answered.
    mcps: Vec<RecapMeeting>,
    /// The recap page is showing (else the home list).
    detail: bool,
    /// Shown as a meeting chat's Recap tab.
    in_chat: bool,
    open: Option<Meeting>,
    chat_parts: Vec<Part>,
    part: usize,
    files: Vec<SharedFile>,
    tab: Tab,
    detail_loading: bool,
    detail_note: String,
    transcript: String,
    transcript_note: String,
    transcript_loading: bool,
    notes_link: Option<String>,
    /// Bumped for every recap opened, so a late answer for another one is dropped.
    seq: u64,
}

impl Recaps {
    fn finish(&mut self, meetings: Vec<Meeting>, synced: bool) {
        self.loading = false;
        self.loaded = synced || !meetings.is_empty();
        self.note = if self.loaded {
            String::new()
        } else {
            STILL_SYNCING.into()
        };
        self.meetings = meetings;
    }

    /// The parts shown: the chat's, completed from the meeting content service.
    fn parts(&self) -> Vec<Part> {
        let thread = self.open.as_ref().map_or("", |m| m.thread_id.as_str());
        with_mcps(self.chat_parts.clone(), thread, &self.mcps)
    }

    fn current_part(&self) -> Option<Part> {
        self.parts().get(self.part).cloned()
    }

    fn shown(&self) -> impl Iterator<Item = &Meeting> {
        let q = self.query.trim().to_lowercase();
        self.meetings.iter().filter(move |m| {
            q.is_empty()
                || m.title.to_lowercase().contains(&q)
                || m.organizer.to_lowercase().contains(&q)
        })
    }

    fn rows(&self) -> Vec<RecapRow> {
        let mut out = Vec::new();
        let mut group = "";
        for m in self.shown() {
            if m.group != group {
                group = m.group.as_str();
                out.push(RecapRow {
                    id: format!("week:{group}"),
                    title: group.to_owned(),
                    header: true,
                    ..Default::default()
                });
            }
            let local = appearance::formats::local(m.start_ms);
            out.push(RecapRow {
                id: m.key.clone(),
                title: m.title.clone(),
                when: when(m),
                organizer: m.organizer.clone(),
                month: local
                    .map(|t| t.format("%b").to_string())
                    .unwrap_or_default(),
                day: local
                    .map(|t| t.format("%-d").to_string())
                    .unwrap_or_default(),
                header: false,
                canceled: m.canceled,
            });
        }
        out
    }

    pub fn data(&self) -> RecapsData {
        let meetings = self.rows();
        let done = self.loaded && !self.loading;
        let note = if !self.note.is_empty() {
            self.note.clone()
        } else if done && self.meetings.is_empty() {
            NO_MEETINGS.into()
        } else if done && meetings.is_empty() {
            NO_MATCH.into()
        } else {
            String::new()
        };
        RecapsData {
            page: if self.detail && self.open.is_some() {
                "detail"
            } else {
                "home"
            },
            footer: if done && !self.meetings.is_empty() {
                FOOTER.into()
            } else {
                String::new()
            },
            meetings,
            query: self.query.clone(),
            note,
            loading: self.loading,
            detail: self.detail_data(),
        }
    }

    fn transcript_note(&self, part: Option<&Part>) -> String {
        if self.transcript_loading {
            return "Loading the transcript…".into();
        }
        if !self.transcript.is_empty() {
            return String::new();
        }
        if !self.transcript_note.is_empty() {
            return self.transcript_note.clone();
        }
        match part {
            _ if self.detail_loading => String::new(),
            None => "There is no recording or transcript in the meeting chat.".into(),
            Some(p) if !p.transcript => "This recording has no transcript.".into(),
            Some(_) => String::new(),
        }
    }

    fn detail_data(&self) -> RecapDetail {
        let Some(m) = self.open.as_ref().filter(|_| self.detail) else {
            return RecapDetail::default();
        };
        let parts = self.parts();
        let part = parts.get(self.part);
        let files = self
            .files
            .iter()
            .map(|f| Row {
                id: f.url.clone(),
                title: f.name.clone(),
                subtitle: [f.by.clone(), super::view::day(f.at_ms)]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · "),
                meta: f.ext.clone(),
                ..Default::default()
            })
            .collect();
        RecapDetail {
            open: true,
            in_chat: self.in_chat,
            title: m.title.clone(),
            when: when(m),
            organizer: m.organizer.clone(),
            parts: if parts.len() > 1 {
                (0..parts.len())
                    .map(|i| Nav::new(&i.to_string(), &format!("Part {}", i + 1)))
                    .collect()
            } else {
                Vec::new()
            },
            part: self.part as i32,
            has_recording: part.is_some_and(|p| p.web_url.is_some()),
            part_label: match part {
                Some(p) => match p.duration_ms {
                    Some(d) => format!("Recording · {}", length(d)),
                    None => "Recording".into(),
                },
                None if self.detail_loading => "Looking for the recording…".into(),
                None => NO_RECORDING.into(),
            },
            loading: self.detail_loading,
            note: self.detail_note.clone(),
            files,
            files_note: if self.detail_loading {
                "Looking for shared files…".into()
            } else if self.files.is_empty() {
                "No files were shared.".into()
            } else {
                String::new()
            },
            tab: match self.tab {
                Tab::Notes => "notes",
                Tab::Transcript => "transcript",
            }
            .into(),
            has_notes: self.notes_link.is_some(),
            notes_note: if self.detail_loading {
                "Looking for the meeting notes…".into()
            } else if self.notes_link.is_some() {
                "The meeting notes open in your browser.".into()
            } else {
                "No notes".into()
            },
            transcript: self.transcript.clone(),
            transcript_note: self.transcript_note(part),
            has_transcript_link: part.is_some_and(|p| p.transcript && p.web_url.is_some())
                && self.transcript.is_empty()
                && !self.transcript_loading,
        }
    }
}

impl Engine {
    fn recaps_push(self: &Arc<Self>) {
        let data = self.apps.lock().unwrap().recaps.data();
        self.ui.apps_recaps(data);
    }

    fn recaps_edit<T>(self: &Arc<Self>, f: impl FnOnce(&mut Recaps) -> T) -> T {
        let out = f(&mut self.apps.lock().unwrap().recaps);
        self.recaps_push();
        out
    }

    /// The meetings of the calendar the home lists.
    fn recaps_meetings(&self) -> (Vec<Meeting>, bool) {
        let (events, synced) = self.src_recap_events();
        let week = crate::backend::cal::local_week_start();
        (meetings_from_events(&events, now_ms(), week), synced)
    }

    pub(super) async fn recaps_enter(self: &Arc<Self>) -> anyhow::Result<()> {
        self.ui.open_view("recaps".into());
        let busy = {
            let mut a = self.apps.lock().unwrap();
            let r = &mut a.recaps;
            // A chat's Recap tab was the last use: the app starts at its home.
            if r.in_chat {
                r.in_chat = false;
                r.detail = false;
            }
            r.loaded || r.loading
        };
        if busy {
            self.recaps_push();
            return Ok(());
        }
        self.recaps_refresh().await
    }

    pub(super) async fn recaps_refresh(self: &Arc<Self>) -> anyhow::Result<()> {
        self.recaps_edit(|r| {
            r.loading = true;
            r.note.clear();
        });
        let (meetings, synced) = self.recaps_meetings();
        self.recaps_edit(|r| r.finish(meetings, synced));
        if !self.apps_offline() {
            // Quietly: the list never waits for it, and a failure only goes to the log.
            let e = self.clone();
            tokio::spawn(async move {
                let mcps = e.src_mcps_recaps().await;
                if !mcps.is_empty() {
                    e.recaps_edit(|r| r.mcps = mcps);
                }
            });
        }
        Ok(())
    }

    pub(super) fn recaps_query(self: &Arc<Self>, q: &str) -> anyhow::Result<()> {
        self.recaps_edit(|r| r.query = q.to_owned());
        Ok(())
    }

    /// Opens the recap of a meeting of the list.
    pub(super) async fn recaps_open(self: &Arc<Self>, key: &str) -> anyhow::Result<()> {
        let m = {
            let a = self.apps.lock().unwrap();
            a.recaps.meetings.iter().find(|m| m.key == key).cloned()
        };
        match m {
            Some(m) => self.recaps_show(m, false).await,
            None => Ok(()),
        }
    }

    /// The Recap tab of a meeting chat: the newest meeting of the calendar with this chat, else
    /// the chat alone under its own name.
    pub(super) async fn recaps_chat(self: &Arc<Self>, thread: &str) -> anyhow::Result<()> {
        let listed = {
            let a = self.apps.lock().unwrap();
            a.recaps
                .meetings
                .iter()
                .find(|m| m.thread_id == thread)
                .cloned()
        };
        let m = listed
            .or_else(|| {
                self.recaps_meetings()
                    .0
                    .into_iter()
                    .find(|m| m.thread_id == thread)
            })
            .unwrap_or_else(|| Meeting {
                key: thread.to_owned(),
                title: self
                    .store
                    .lock()
                    .unwrap()
                    .conversation(thread)
                    .ok()
                    .flatten()
                    .map(|c| c.title)
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| "Meeting chat".into()),
                canceled: false,
                start_ms: 0,
                end_ms: 0,
                organizer: String::new(),
                thread_id: thread.to_owned(),
                recurring: false,
                group: String::new(),
            });
        self.recaps_show(m, true).await
    }

    /// Shows a meeting's recap and reads its chat and notes.
    async fn recaps_show(self: &Arc<Self>, m: Meeting, in_chat: bool) -> anyhow::Result<()> {
        let seq = self.recaps_edit(|r| {
            r.seq += 1;
            r.detail = true;
            r.in_chat = in_chat;
            r.open = Some(m.clone());
            r.chat_parts.clear();
            r.part = 0;
            r.files.clear();
            r.detail_loading = !m.thread_id.is_empty();
            r.detail_note = if m.thread_id.is_empty() {
                NO_CHAT.into()
            } else {
                String::new()
            };
            r.transcript.clear();
            r.transcript_note.clear();
            r.transcript_loading = false;
            r.notes_link = None;
            r.seq
        });
        if m.thread_id.is_empty() {
            return Ok(());
        }
        // The occurrences of a series share one chat: keep what this one recorded.
        let window = (!in_chat && m.recurring && m.start_ms > 0)
            .then(|| (m.start_ms - 3_600_000, m.end_ms + 12 * 3_600_000));
        let (chat, notes) = tokio::join!(
            self.src_meeting_chat(&m.thread_id),
            self.src_meeting_notes(&m.thread_id)
        );
        let transcript = self.recaps_edit(|r| {
            if r.seq != seq {
                return false;
            }
            r.detail_loading = false;
            match chat {
                Ok(rows) => {
                    r.chat_parts = parts_from_chat(&rows, window);
                    r.files = files_from_chat(&rows, &r.chat_parts);
                }
                // Said plainly, without the raw error (it goes to the log).
                Err(e) => {
                    tracing::info!(err = %e, "meeting chat could not be read");
                    r.detail_note = CHAT_UNREADABLE.into();
                }
            }
            // A failed lookup reads as "No notes"; the error only goes to the log.
            match notes {
                Ok(link) => r.notes_link = link,
                Err(e) => tracing::debug!(err = %e, "meeting notes could not be looked up"),
            }
            r.tab == Tab::Transcript
        });
        if transcript {
            self.recaps_load_transcript(seq).await;
        }
        Ok(())
    }

    /// Reads the selected part's transcript, when it has one.
    async fn recaps_load_transcript(self: &Arc<Self>, seq: u64) {
        let part = self.recaps_edit(|r| {
            if r.seq != seq || r.detail_loading {
                return None;
            }
            let p = r.current_part().filter(|p| p.transcript)?;
            r.transcript_loading = true;
            Some(p)
        });
        let Some(p) = part else { return };
        let read = self.src_transcript(None, Some(&p.resource())).await;
        self.recaps_edit(|r| {
            if r.seq != seq || r.current_part().as_ref() != Some(&p) {
                return;
            }
            r.transcript_loading = false;
            match read {
                Ok(t) => r.transcript = t,
                Err(e) => {
                    tracing::debug!(err = %e, "transcript could not be read");
                    r.transcript_note = TRANSCRIPT_UNREADABLE.into();
                }
            }
        });
    }

    pub(super) fn recaps_home(self: &Arc<Self>) -> anyhow::Result<()> {
        self.recaps_edit(|r| {
            r.detail = false;
            r.seq += 1;
        });
        Ok(())
    }

    pub(super) async fn recaps_part(self: &Arc<Self>, i: usize) -> anyhow::Result<()> {
        let (seq, transcript) = self.recaps_edit(|r| {
            r.part = i.min(r.parts().len().saturating_sub(1));
            r.transcript.clear();
            r.transcript_note.clear();
            r.transcript_loading = false;
            (r.seq, r.tab == Tab::Transcript)
        });
        if transcript {
            self.recaps_load_transcript(seq).await;
        }
        Ok(())
    }

    pub(super) async fn recaps_tab(self: &Arc<Self>, tab: &str) -> anyhow::Result<()> {
        let (seq, load) = self.recaps_edit(|r| {
            r.tab = if tab == "transcript" {
                Tab::Transcript
            } else {
                Tab::Notes
            };
            let load = r.tab == Tab::Transcript
                && r.transcript.is_empty()
                && r.transcript_note.is_empty()
                && !r.transcript_loading;
            (r.seq, load)
        });
        if load {
            self.recaps_load_transcript(seq).await;
        }
        Ok(())
    }

    fn recaps_part_link(self: &Arc<Self>) -> Option<String> {
        self.apps
            .lock()
            .unwrap()
            .recaps
            .current_part()
            .and_then(|p| p.web_url)
    }

    /// Watch in browser: kelma has no video player, so the recording opens in SharePoint or
    /// Stream.
    pub(super) fn recaps_watch(self: &Arc<Self>) -> anyhow::Result<()> {
        match self.recaps_part_link() {
            Some(l) => self.apps_open_link(&l),
            None => self.ui.banner("There is no recording to open.".into()),
        }
        Ok(())
    }

    pub(super) fn recaps_transcript_web(self: &Arc<Self>) -> anyhow::Result<()> {
        self.recaps_watch()
    }

    /// Share: copies the recording's link (whoever opens it needs access to the recording).
    pub(super) fn recaps_share(self: &Arc<Self>) -> anyhow::Result<()> {
        let Some(link) = self.recaps_part_link() else {
            self.ui.banner("There is no recording to share.".into());
            return Ok(());
        };
        if !self.apps_offline()
            && let Err(e) = arboard::Clipboard::new().and_then(|mut c| c.set_text(link))
        {
            self.ui.banner(format!("The link could not be copied: {e}"));
            return Ok(());
        }
        self.ui.banner("Link to the recording copied.".into());
        Ok(())
    }

    pub(super) fn recaps_notes(self: &Arc<Self>) -> anyhow::Result<()> {
        let link = self.apps.lock().unwrap().recaps.notes_link.clone();
        match link {
            Some(l) => self.apps_open_link(&l),
            None => self.ui.banner("No meeting notes were found.".into()),
        }
        Ok(())
    }

    /// Opens a file of the shared files list (only those: the address comes from the screen).
    pub(super) fn recaps_file(self: &Arc<Self>, url: &str) -> anyhow::Result<()> {
        let known = self
            .apps
            .lock()
            .unwrap()
            .recaps
            .files
            .iter()
            .any(|f| f.url == url);
        if known {
            self.apps_open_link(url);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teams_proto::recaps::parse_meetings;

    const NOW: i64 = 1_795_000_000_000;

    fn events() -> Vec<Event> {
        ::calendar::parse_events(&super::super::fixtures::recap_events(NOW))
    }

    /// The chat of a demo meeting as the store keeps it.
    fn chat(thread: &str) -> Vec<MessageRow> {
        let v = super::super::fixtures::meeting_chat(thread, NOW);
        v["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| teams_proto::model::message_from_json(m, thread))
            .map(|m| crate::backend::util::row_from_message(&m, None))
            .collect()
    }

    #[test]
    fn the_home_lists_the_online_meetings_of_the_last_30_days_newest_first() {
        // The week started three days ago.
        let week = NOW - 3 * DAY_MS;
        let m = meetings_from_events(&events(), NOW, week);
        let got: Vec<(&str, &str, &str)> = m
            .iter()
            .map(|m| (m.title.as_str(), m.group.as_str(), m.organizer.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("Weekly sync", "This week", "Ada Example"),
                ("Canceled: Budget review", "Last week", "Grace Example"),
                ("Design review", "Last week", "Sam Example"),
                ("Retro", "2 weeks ago", "Ada Example"),
            ],
            "the in-person lunch, the 40-day-old and the coming meeting are left out"
        );
        assert!(m[1].canceled && !m[0].canceled);
        assert_eq!(m[0].thread_id, "19:meeting_demo1@thread.v2");
        assert_eq!(m[0].key, "ev-sync");
    }

    #[test]
    fn week_headings_and_the_canceled_prefix() {
        let week = 100 * DAY_MS;
        assert_eq!(week_label(week, week), "This week");
        assert_eq!(week_label(week - 1, week), "Last week");
        assert_eq!(week_label(week - 7 * DAY_MS, week), "Last week");
        assert_eq!(week_label(week - 7 * DAY_MS - 1, week), "2 weeks ago");
        assert_eq!(week_label(week - 22 * DAY_MS, week), "4 weeks ago");
        // Outlook may already have put the prefix there: it is not doubled.
        let mut e = events().remove(1);
        assert!(e.is_cancelled);
        e.subject = "Canceled: Budget review".into();
        assert_eq!(shown_title(&e), "Canceled: Budget review");
        e.subject = "  ".into();
        assert_eq!(shown_title(&e), "Canceled: (No subject)");
    }

    #[test]
    fn the_home_groups_by_week_filters_by_search_and_ends_with_a_footer() {
        let mut r = Recaps::default();
        r.finish(meetings_from_events(&events(), NOW, NOW - 3 * DAY_MS), true);
        let d = r.data();
        assert_eq!(d.page, "home");
        let rows: Vec<(bool, &str)> = d
            .meetings
            .iter()
            .map(|m| (m.header, m.title.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                (true, "This week"),
                (false, "Weekly sync"),
                (true, "Last week"),
                (false, "Canceled: Budget review"),
                (false, "Design review"),
                (true, "2 weeks ago"),
                (false, "Retro"),
            ]
        );
        assert!(d.meetings[3].canceled);
        assert!(!d.meetings[1].month.is_empty() && !d.meetings[1].day.is_empty());
        assert_eq!(d.footer, FOOTER);
        assert_eq!(d.note, "");
        // Search: by title or organizer.
        r.query = "sam".into();
        let titles: Vec<String> = r.data().meetings.into_iter().map(|m| m.title).collect();
        assert_eq!(titles, ["Last week", "Design review"]);
        r.query = "budget".into();
        assert_eq!(r.data().meetings.len(), 2);
        r.query = "nothing like this".into();
        assert_eq!(r.data().note, NO_MATCH);
        // Nothing in the calendar, before and after its first sync.
        let mut r = Recaps::default();
        r.finish(vec![], false);
        assert_eq!(r.data().note, STILL_SYNCING);
        assert!(!r.loaded, "entering again reads the calendar again");
        r.finish(vec![], true);
        assert_eq!(r.data().note, NO_MEETINGS);
        assert_eq!(r.data().footer, "");
    }

    #[test]
    fn the_meeting_chat_gives_the_parts_their_transcript_and_the_shared_files() {
        let rows = chat("19:meeting_demo1@thread.v2");
        let parts = parts_from_chat(&rows, None);
        let got: Vec<(&str, Option<i64>, bool)> = parts
            .iter()
            .map(|p| (p.name.as_str(), p.duration_ms, p.transcript))
            .collect();
        assert_eq!(
            got,
            [
                ("01DEMO1.mp4", Some(1_080_000), false),
                ("01DEMO1B.mp4", Some(605_000), true),
            ],
            "the started recording is not a part"
        );
        assert_eq!(
            parts[0].web_url.as_deref(),
            Some("https://contoso-my.sharepoint.com.invalid/01DEMO1.mp4")
        );
        assert_eq!(parts[0].item_id.as_deref(), Some("01DEMO1"));
        let files = files_from_chat(&rows, &parts);
        assert_eq!(files.len(), 1);
        assert_eq!(
            (
                files[0].name.as_str(),
                files[0].ext.as_str(),
                files[0].by.as_str()
            ),
            ("Agenda.docx", "DOCX", "Ada Example")
        );
        // An occurrence of a series keeps what was posted while it ran.
        let start = NOW - 2 * DAY_MS;
        assert_eq!(
            parts_from_chat(&rows, Some((start, start + 25 * 60_000))).len(),
            1
        );
        // A transcript message alone marks a single part.
        let one = chat("19:meeting_demo2@thread.v2");
        let mut with_tr = one.clone();
        with_tr.push(MessageRow {
            id: "t".into(),
            message_type: teams_proto::recaps::TRANSCRIPT_MESSAGE.into(),
            content_html: "{}".into(),
            compose_ms: NOW,
            ..one[0].clone()
        });
        assert!(!parts_from_chat(&one, None)[0].transcript);
        assert!(parts_from_chat(&with_tr, None)[0].transcript);
        assert!(parts_from_chat(&chat("19:meeting_demo3@thread.v2"), None).is_empty());
    }

    #[test]
    fn the_meeting_content_service_only_fills_in_what_the_chat_lacks() {
        let mcps = parse_meetings(&super::super::fixtures::recap_meetings(NOW));
        // The chat has no recording: the service's recording and transcript are used.
        let parts = with_mcps(vec![], "19:meeting_demo1@thread.v2", &mcps);
        assert_eq!(parts.len(), 1);
        assert!(parts[0].transcript);
        assert_eq!(parts[0].item_id.as_deref(), Some("01DEMO1"));
        // The chat's own parts stay as they are; another thread gets nothing.
        let chat_parts = parts_from_chat(&chat("19:meeting_demo1@thread.v2"), None);
        assert_eq!(
            with_mcps(chat_parts.clone(), "19:meeting_demo1@thread.v2", &mcps),
            chat_parts
        );
        assert!(with_mcps(vec![], "19:meeting_other@thread.v2", &mcps).is_empty());
        assert!(with_mcps(vec![], "", &mcps).is_empty());
    }

    #[test]
    fn the_detail_shows_parts_files_notes_and_honest_lines() {
        let rows = chat("19:meeting_demo1@thread.v2");
        let mut r = Recaps::default();
        r.finish(meetings_from_events(&events(), NOW, NOW - 3 * DAY_MS), true);
        r.open = Some(r.meetings[0].clone());
        r.detail = true;
        r.chat_parts = parts_from_chat(&rows, None);
        r.files = files_from_chat(&rows, &r.chat_parts);
        let d = r.data();
        assert_eq!(d.page, "detail");
        let parts: Vec<String> = d.detail.parts.iter().map(|p| p.label.clone()).collect();
        assert_eq!(parts, ["Part 1", "Part 2"]);
        assert!(d.detail.has_recording);
        assert_eq!(d.detail.part_label, "Recording · 18 min");
        assert_eq!(d.detail.files[0].title, "Agenda.docx");
        assert_eq!(d.detail.files_note, "");
        assert_eq!(d.detail.notes_note, "No notes");
        assert_eq!(d.detail.tab, "notes");
        // Part 1 has no transcript; Part 2 has one and can open in the browser.
        assert_eq!(
            d.detail.transcript_note,
            "This recording has no transcript."
        );
        assert!(!d.detail.has_transcript_link);
        r.part = 1;
        let d = r.data().detail;
        assert_eq!(d.part_label, "Recording · 10 min");
        assert!(d.has_transcript_link);
        // A meeting with no recording and no files.
        r.chat_parts.clear();
        r.files.clear();
        r.part = 0;
        let d = r.data().detail;
        assert!(!d.has_recording && d.parts.is_empty());
        assert_eq!(d.part_label, NO_RECORDING);
        assert_eq!(d.files_note, "No files were shared.");
        // The chat could not be read: said plainly, in the detail.
        r.detail_note = CHAT_UNREADABLE.into();
        assert_eq!(r.data().detail.note, CHAT_UNREADABLE);
        r.detail = false;
        assert_eq!(r.data().page, "home");
        assert!(!r.data().detail.open);
    }

    #[test]
    fn lengths() {
        assert_eq!(length(45_000), "45 s");
        assert_eq!(length(1_080_000), "18 min");
        assert_eq!(length(3_600_000), "1 h");
        assert_eq!(length(3_900_000), "1 h 5 min");
    }
}
