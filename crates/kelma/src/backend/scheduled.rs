//! The send button's "Schedule message": the chat service keeps the message as a scheduled draft
//! and sends it at the chosen time (`teams_proto::scheduled`, from rusty-teams). UNVERIFIED live:
//! on 2026-11-19 the work account answered the drafts endpoints with 404 (no drafts thread) while
//! Teams web offered no "Schedule message"; Teams web offers it now, but the kelma calls were not
//! captured against it. A refused schedule puts the text back in the composer and says why.
//!
//! The open chat's waiting messages show at the bottom of the conversation as "Sending on <day>
//! at <time>" with Edit (back into the composer with the same time, the waiting copy cancelled)
//! and Delete.

use super::Engine;
use super::mentions;
use crate::send_options::sending_label;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use teams_proto::scheduled::ScheduledMessage;
use teams_proto::time::now_ms;

/// Commands of the send options (`SendStore.command(name, argument)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduledCmd {
    /// Schedule the composer text for this time (ms since 1970). Argument: `<ms>\t<text>`.
    Schedule { at_ms: i64, text: String },
    /// A waiting message (by client message id) back into the composer, with its time chosen.
    Edit(String),
    /// Cancel a waiting message (by client message id).
    Delete(String),
}

impl ScheduledCmd {
    pub fn parse(name: &str, arg: &str) -> Option<Self> {
        Some(match name {
            "schedule" => {
                let (at, text) = arg.split_once('\t')?;
                ScheduledCmd::Schedule {
                    at_ms: at.parse().ok()?,
                    text: text.to_string(),
                }
            }
            "edit" if !arg.is_empty() => ScheduledCmd::Edit(arg.into()),
            "delete" if !arg.is_empty() => ScheduledCmd::Delete(arg.into()),
            _ => return None,
        })
    }
}

/// One bubble of the open chat: client message id, "Sending on …", the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledData {
    pub id: String,
    pub label: String,
    pub text: String,
}

/// The waiting messages of `conv` (still in the future at `now`), soonest first.
pub(super) fn bubbles(list: &[ScheduledMessage], conv: &str, now: i64) -> Vec<ScheduledData> {
    let mut mine: Vec<&ScheduledMessage> = list
        .iter()
        .filter(|m| m.conversation_id == conv && m.send_at_ms > now)
        .collect();
    mine.sort_by_key(|m| m.send_at_ms);
    mine.into_iter()
        .map(|m| ScheduledData {
            id: m.client_message_id.clone(),
            label: sending_label(m.send_at_ms),
            text: ::render::plain_text(&::render::parse_teams_html(&m.html)),
        })
        .collect()
}

impl Engine {
    pub(super) async fn scheduled_cmd(self: &Arc<Self>, c: ScheduledCmd) -> anyhow::Result<()> {
        match c {
            ScheduledCmd::Schedule { at_ms, text } => self.schedule(at_ms, text).await,
            ScheduledCmd::Edit(id) => self.edit_scheduled(&id).await,
            ScheduledCmd::Delete(id) => self.delete_scheduled(&id).await,
        }
    }

    fn offline_now(&self) -> bool {
        self.offline.load(Ordering::Relaxed)
    }

    /// A chat was opened: its waiting messages show; the account's list is read once.
    pub(super) fn scheduled_chat_opened(self: &Arc<Self>) {
        self.ui.schedule_choice(None);
        self.push_scheduled();
        let load = {
            let mut v = self.view.lock().unwrap();
            !std::mem::replace(&mut v.scheduled_loaded, true)
        };
        if load && !self.offline_now() {
            let e = self.clone();
            tokio::spawn(async move {
                if let Err(err) = e.reload_scheduled().await {
                    // Not every account has the drafts thread (404 LocationLookupFailed).
                    tracing::debug!(%err, "scheduled messages");
                }
            });
        }
    }

    async fn reload_scheduled(self: &Arc<Self>) -> anyhow::Result<()> {
        let list = self.client.scheduled_messages().await?;
        self.view.lock().unwrap().scheduled = list;
        self.push_scheduled();
        Ok(())
    }

    fn push_scheduled(&self) {
        let (list, conv, thread) = {
            let v = self.view.lock().unwrap();
            (
                v.scheduled.clone(),
                v.current.clone(),
                v.thread_root.is_some(),
            )
        };
        let items = match conv {
            Some(c) if !thread => bubbles(&list, &c, now_ms()),
            _ => Vec::new(),
        };
        self.ui.scheduled(items);
    }

    async fn schedule(self: &Arc<Self>, at_ms: i64, text: String) -> anyhow::Result<()> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Ok(());
        }
        let (conv, thread) = {
            let v = self.view.lock().unwrap();
            (v.current.clone(), v.thread_root.is_some())
        };
        let Some(conv) = conv else { return Ok(()) };
        let refuse = |why: &str| {
            // The text goes back to the composer, so nothing typed is lost.
            self.ui.set_draft(text.clone());
            self.view.lock().unwrap().draft_text = text.clone();
            anyhow::anyhow!("{why}")
        };
        if thread {
            return Err(refuse("Replies can't be scheduled"));
        }
        if at_ms <= now_ms() {
            return Err(refuse("Pick a time in the future"));
        }
        let prepared = {
            let mut p = self.people.lock().unwrap();
            let prepared = mentions::prepare(&text, &p.entries);
            p.entries.clear();
            prepared
        };
        let html = mentions::finish(
            &::render::markdown_to_composer_html(&prepared.text),
            &prepared,
        );
        let cmid = if self.offline_now() {
            now_ms().to_string()
        } else {
            match self
                .client
                .schedule_message(&conv, &html, &self.my_name, at_ms)
                .await
            {
                Ok(cmid) => cmid,
                Err(err) => {
                    return Err(refuse(&format!("Couldn't schedule the message: {err}")));
                }
            }
        };
        self.clear_draft(&conv);
        // The answer has no id: it is filled in by the next read of the list.
        self.view.lock().unwrap().scheduled.push(ScheduledMessage {
            id: String::new(),
            conversation_id: conv,
            send_at_ms: at_ms,
            html,
            client_message_id: cmid,
        });
        self.push_scheduled();
        if !self.offline_now()
            && let Err(err) = self.reload_scheduled().await
        {
            tracing::debug!(%err, "scheduled messages after scheduling");
        }
        Ok(())
    }

    fn find_scheduled(&self, cmid: &str) -> anyhow::Result<ScheduledMessage> {
        self.view
            .lock()
            .unwrap()
            .scheduled
            .iter()
            .find(|m| m.client_message_id == cmid)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("That message was already sent"))
    }

    /// Takes the waiting message out of the list and cancels it on the account. Its server id is
    /// looked up first when it was scheduled here and not listed since.
    async fn take_scheduled(self: &Arc<Self>, cmid: &str) -> anyhow::Result<ScheduledMessage> {
        let mut m = self.find_scheduled(cmid)?;
        if !self.offline_now() {
            if m.id.is_empty() {
                self.reload_scheduled().await?;
                m = self.find_scheduled(cmid)?;
            }
            self.client.cancel_scheduled(&m.id).await?;
        }
        self.view
            .lock()
            .unwrap()
            .scheduled
            .retain(|x| x.client_message_id != cmid);
        self.push_scheduled();
        Ok(m)
    }

    async fn delete_scheduled(self: &Arc<Self>, cmid: &str) -> anyhow::Result<()> {
        self.take_scheduled(cmid).await.map(|_| ())
    }

    /// Edit: the waiting copy is cancelled and its text goes back to the composer with its time
    /// chosen again, so Send schedules it anew. The text stays as the chat's draft meanwhile.
    async fn edit_scheduled(self: &Arc<Self>, cmid: &str) -> anyhow::Result<()> {
        let m = self.take_scheduled(cmid).await?;
        let text = ::render::plain_text(&::render::parse_teams_html(&m.html));
        self.ui.set_draft(text.clone());
        self.view.lock().unwrap().draft_text = text;
        self.schedule_draft_save();
        self.ui.schedule_choice(Some(m.send_at_ms));
        self.ui.focus();
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn test_scheduled(&self) -> Vec<ScheduledMessage> {
        self.view.lock().unwrap().scheduled.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn waiting(cmid: &str, conv: &str, at: i64) -> ScheduledMessage {
        ScheduledMessage {
            id: format!("d{cmid}"),
            conversation_id: conv.into(),
            send_at_ms: at,
            html: format!("<p>text {cmid}</p>"),
            client_message_id: cmid.into(),
        }
    }

    #[test]
    fn parses_the_send_option_commands() {
        assert_eq!(
            ScheduledCmd::parse("schedule", "1800000000000\thello\tthere"),
            Some(ScheduledCmd::Schedule {
                at_ms: 1_800_000_000_000,
                text: "hello\tthere".into()
            })
        );
        assert_eq!(ScheduledCmd::parse("schedule", "soon\thi"), None);
        assert_eq!(ScheduledCmd::parse("schedule", "123"), None);
        assert_eq!(
            ScheduledCmd::parse("edit", "c1"),
            Some(ScheduledCmd::Edit("c1".into()))
        );
        assert_eq!(
            ScheduledCmd::parse("delete", "c1"),
            Some(ScheduledCmd::Delete("c1".into()))
        );
        assert_eq!(ScheduledCmd::parse("delete", ""), None);
    }

    #[test]
    fn bubbles_are_the_open_chats_future_messages_soonest_first() {
        let list = vec![
            waiting("late", "19:a", 3_000),
            waiting("other", "19:b", 2_000),
            waiting("soon", "19:a", 2_000),
            waiting("past", "19:a", 500),
        ];
        let b = bubbles(&list, "19:a", 1_000);
        let ids: Vec<_> = b.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["soon", "late"]);
        assert_eq!(b[0].text, "text soon");
        assert!(b[0].label.starts_with("Sending on "));
    }
}
