//! Meeting Recaps (plans/meeting-recaps.md R1): your recent meetings that have a recording or a
//! transcript, and for the selected one: play the recording (it opens in the browser, kelma has
//! no player), read the transcript, open the meeting notes, and Share (the OneDrive share
//! dialog). The AI recap (notes, tasks, chapters) needs Teams Premium or Copilot and is shown as
//! "Not available". The sources are described in `teams_proto::recaps`; the meeting content
//! service and the transcript path are UNVERIFIED against a real tenant.

use super::view::{RecapDetail, RecapsData, Row, day, moment};
use crate::backend::Engine;
use onedrive::DriveItem;
use std::sync::Arc;
use teams_proto::recaps::{RecapMeeting, ResourceKind};

#[derive(Default)]
pub struct Recaps {
    loaded: bool,
    loading: bool,
    meetings: Vec<RecapMeeting>,
    query: String,
    /// Where the list came from, or why it is empty.
    note: String,
    selected: Option<String>,
    transcript: String,
    transcript_note: String,
    notes_link: Option<String>,
    notes_note: String,
    detail_loading: bool,
}

/// `1 h 5 min`-style length of a meeting, empty when the end is unknown.
fn length(m: &RecapMeeting) -> String {
    match (m.start_ms, m.end_ms) {
        (Some(s), Some(e)) if e > s => super::view::duration(s, e),
        _ => String::new(),
    }
}

impl Recaps {
    fn current(&self) -> Option<&RecapMeeting> {
        self.meetings
            .iter()
            .find(|m| self.selected.as_deref() == Some(m.key.as_str()))
    }

    fn shown(&self) -> impl Iterator<Item = &RecapMeeting> {
        let q = self.query.to_lowercase();
        self.meetings.iter().filter(move |m| {
            q.is_empty()
                || m.subject.to_lowercase().contains(&q)
                || m.organizer.to_lowercase().contains(&q)
        })
    }

    pub fn data(&self) -> RecapsData {
        let meetings: Vec<Row> = self
            .shown()
            .map(|m| {
                let mut what = Vec::new();
                if m.first(ResourceKind::Recording).is_some() {
                    what.push("Recording");
                }
                if m.first(ResourceKind::Transcript).is_some() {
                    what.push("Transcript");
                }
                Row {
                    id: m.key.clone(),
                    title: if m.subject.is_empty() {
                        "(No subject)".into()
                    } else {
                        m.subject.clone()
                    },
                    subtitle: m.start_ms.map(moment).unwrap_or_default(),
                    meta: what.join(" · "),
                    selected: self.selected.as_deref() == Some(m.key.as_str()),
                    ..Default::default()
                }
            })
            .collect();
        let note = if !self.note.is_empty() {
            self.note.clone()
        } else if self.loaded && !self.loading && self.meetings.is_empty() {
            "No meetings with a recording or transcript in the last 30 days.".into()
        } else if self.loaded && !self.loading && meetings.is_empty() {
            "No meeting matches your search.".into()
        } else {
            String::new()
        };
        RecapsData {
            meetings,
            query: self.query.clone(),
            note,
            loading: self.loading,
            detail: self.detail(),
        }
    }

    fn detail(&self) -> RecapDetail {
        let Some(m) = self.current() else {
            return RecapDetail::default();
        };
        let has_ai = m.first(ResourceKind::AiSummary).is_some();
        let when = match (m.start_ms, length(m)) {
            (Some(s), l) if !l.is_empty() => format!("{}, {l}", day(s)),
            (Some(s), _) => day(s),
            _ => String::new(),
        };
        let transcript_note = if self.detail_loading {
            "Loading the transcript…".to_owned()
        } else if !self.transcript.is_empty() {
            String::new()
        } else if !self.transcript_note.is_empty() {
            self.transcript_note.clone()
        } else if m.first(ResourceKind::Transcript).is_none() {
            "This meeting has no transcript.".into()
        } else {
            String::new()
        };
        RecapDetail {
            open: true,
            title: if m.subject.is_empty() {
                "(No subject)".into()
            } else {
                m.subject.clone()
            },
            when,
            organizer: m.organizer.clone(),
            has_recording: m
                .first(ResourceKind::Recording)
                .is_some_and(|r| r.web_url.is_some()),
            has_transcript: m.first(ResourceKind::Transcript).is_some(),
            transcript: self.transcript.clone(),
            transcript_note,
            has_notes: self.notes_link.is_some(),
            notes_note: if self.detail_loading {
                "Looking for the meeting notes…".into()
            } else if self.notes_link.is_some() {
                String::new()
            } else if !self.notes_note.is_empty() {
                self.notes_note.clone()
            } else {
                "No meeting notes were found for this meeting.".into()
            },
            ai: if has_ai {
                "Available in Teams".into()
            } else {
                "Not available".into()
            },
            has_ai_link: m
                .first(ResourceKind::AiSummary)
                .is_some_and(|r| r.web_url.is_some()),
            can_share: m
                .first(ResourceKind::Recording)
                .is_some_and(|r| r.drive_id.is_some() && r.item_id.is_some()),
        }
    }
}

impl Engine {
    fn recaps_push(self: &Arc<Self>) {
        let data = self.apps.lock().unwrap().recaps.data();
        self.ui.apps_recaps(data);
    }

    fn recaps_edit(self: &Arc<Self>, f: impl FnOnce(&mut Recaps)) {
        f(&mut self.apps.lock().unwrap().recaps);
        self.recaps_push();
    }

    pub(super) async fn recaps_enter(self: &Arc<Self>) -> anyhow::Result<()> {
        self.ui.open_view("recaps".into());
        let busy = {
            let r = &self.apps.lock().unwrap().recaps;
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
        let found = self.src_recaps().await;
        let first = {
            let mut a = self.apps.lock().unwrap();
            let r = &mut a.recaps;
            r.loading = false;
            match found {
                Ok((m, source)) => {
                    r.loaded = true;
                    r.meetings = m;
                    r.note = source.to_owned();
                    // Keep the open meeting if it is still listed, else open the newest.
                    r.selected = r
                        .selected
                        .clone()
                        .filter(|k| r.meetings.iter().any(|m| m.key == *k))
                        .or_else(|| r.meetings.first().map(|m| m.key.clone()));
                }
                // A failed load is tried again the next time the view opens.
                Err(e) => r.note = format!("Loading your meetings failed: {e}"),
            }
            r.selected.clone()
        };
        self.recaps_push();
        if let Some(key) = first {
            self.recaps_open(&key).await?;
        }
        Ok(())
    }

    pub(super) fn recaps_query(self: &Arc<Self>, q: &str) -> anyhow::Result<()> {
        self.recaps_edit(|r| r.query = q.to_owned());
        Ok(())
    }

    /// Selects a meeting and loads its transcript and notes link.
    pub(super) async fn recaps_open(self: &Arc<Self>, key: &str) -> anyhow::Result<()> {
        let meeting = {
            let mut a = self.apps.lock().unwrap();
            let r = &mut a.recaps;
            r.selected = Some(key.to_owned());
            r.transcript.clear();
            r.transcript_note.clear();
            r.notes_link = None;
            r.notes_note.clear();
            r.detail_loading = true;
            r.current().cloned()
        };
        self.recaps_push();
        let Some(m) = meeting else { return Ok(()) };
        let transcript = m.first(ResourceKind::Transcript);
        let recording = m.first(ResourceKind::Recording);
        let (text, notes) = tokio::join!(
            async {
                // The meeting lists a transcript when it has one; a recording alone has none.
                if transcript.is_none() {
                    return Ok(String::new());
                }
                self.src_transcript(transcript, recording).await
            },
            self.src_meeting_notes(&m.thread_id)
        );
        self.recaps_edit(|r| {
            // Another meeting may have been picked while these loaded.
            if r.selected.as_deref() != Some(key) {
                return;
            }
            r.detail_loading = false;
            match text {
                Ok(t) => r.transcript = t,
                Err(e) => r.transcript_note = format!("The transcript could not be read: {e}"),
            }
            match notes {
                Ok(link) => r.notes_link = link,
                Err(e) => r.notes_note = format!("The meeting notes could not be looked up: {e}"),
            }
        });
        Ok(())
    }

    /// Opens a link of the selected meeting in the browser.
    fn recaps_open_link(self: &Arc<Self>, pick: impl FnOnce(&RecapMeeting) -> Option<String>) {
        let link = {
            let a = self.apps.lock().unwrap();
            a.recaps.current().and_then(pick)
        };
        match link {
            Some(l) => self.apps_open_link(&l),
            None => self.ui.banner("There is nothing to open for this.".into()),
        }
    }

    /// Plays the recording: kelma has no player, so it opens in the browser (SharePoint or
    /// Stream).
    pub(super) fn recaps_play(self: &Arc<Self>) -> anyhow::Result<()> {
        self.recaps_open_link(|m| m.first(ResourceKind::Recording)?.web_url.clone());
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

    pub(super) fn recaps_ai(self: &Arc<Self>) -> anyhow::Result<()> {
        self.recaps_open_link(|m| m.first(ResourceKind::AiSummary)?.web_url.clone());
        Ok(())
    }

    /// Share: the OneDrive share dialog for the recording.
    pub(super) async fn recaps_share(self: &Arc<Self>) -> anyhow::Result<()> {
        let item = {
            let a = self.apps.lock().unwrap();
            a.recaps.current().and_then(|m| {
                let r = m.first(ResourceKind::Recording)?;
                Some(DriveItem {
                    id: r.item_id.clone()?,
                    drive_id: r.drive_id.clone()?,
                    name: if r.name.is_empty() {
                        format!("{} (recording)", m.subject)
                    } else {
                        r.name.clone()
                    },
                    mime: "video/mp4".into(),
                    web_url: r.web_url.clone().unwrap_or_default(),
                    ..Default::default()
                })
            })
        };
        match item {
            Some(item) => self.drive_share_open(item).await,
            None => {
                self.ui.banner(
                    "This recording is not in a drive kelma can share from. Open it in the \
                     browser to share it there."
                        .into(),
                );
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teams_proto::recaps::parse_meetings;

    fn state() -> Recaps {
        let now = teams_proto::time::now_ms();
        let meetings = parse_meetings(&super::super::fixtures::recap_meetings(now));
        Recaps {
            loaded: true,
            selected: Some(meetings[0].key.clone()),
            meetings,
            ..Default::default()
        }
    }

    #[test]
    fn the_list_names_what_each_meeting_has_and_filters_by_search() {
        let mut r = state();
        let d = r.data();
        let got: Vec<(&str, &str)> = d
            .meetings
            .iter()
            .map(|m| (m.title.as_str(), m.meta.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("Weekly sync", "Recording · Transcript"),
                ("Design review", "Recording"),
                ("Retro", "Transcript"),
            ]
        );
        assert!(d.meetings[0].selected && !d.meetings[1].selected);
        r.query = "DESIGN".into();
        assert_eq!(r.data().meetings.len(), 1);
        r.query = "nothing like this".into();
        assert_eq!(r.data().note, "No meeting matches your search.");
    }

    #[test]
    fn the_detail_shows_what_can_be_done_and_labels_the_ai_recap() {
        let mut r = state();
        let d = r.data().detail;
        assert!(d.open && d.has_recording && d.has_transcript && d.can_share);
        assert_eq!(d.ai, "Available in Teams");
        assert!(!d.has_ai_link, "the fixture's summary has no link");
        assert_eq!(d.organizer, "ada@example.invalid");
        // A recording without a transcript, and no AI summary.
        r.selected = Some(r.meetings[1].key.clone());
        let d = r.data().detail;
        assert_eq!(d.ai, "Not available");
        assert!(!d.has_transcript);
        assert_eq!(d.transcript_note, "This meeting has no transcript.");
        assert_eq!(
            d.notes_note, "No meeting notes were found for this meeting.",
            "until the notes are looked up"
        );
        r.selected = None;
        assert!(!r.data().detail.open);
    }
}
