//! Live (Trouter) events: incoming messages, typing, desktop notifications.

use super::Engine;
use super::unread::follow_new_message;
use super::util::*;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use store::Store;
use teams_proto::model::{Event, Message};
use teams_proto::trouter::TrouterEvent;

/// Whether an incoming message pops a desktop notification: new chat content from someone else
/// that the user is not already looking at.
fn should_notify(
    existed: bool,
    mine: bool,
    chat_content: bool,
    is_current: bool,
    focused: bool,
) -> bool {
    !existed && !mine && chat_content && !(is_current && focused)
}

impl Engine {
    pub(super) async fn live_loop(self: Arc<Self>) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(teams_proto::trouter::run(self.client.clone(), tx));
        let mut was_disconnected = false;
        while let Some(ev) = rx.recv().await {
            match ev {
                TrouterEvent::Connected => {
                    self.connected.store(true, Ordering::Relaxed);
                    self.ui.connected(true);
                    // Anything sent while offline goes out now, in order.
                    let e = self.clone();
                    tokio::spawn(async move { e.flush_outbox(true).await });
                    if was_disconnected {
                        // Catch up on anything missed while offline.
                        let e = self.clone();
                        tokio::spawn(async move {
                            let _ = e.sync().await;
                        });
                    }
                    was_disconnected = false;
                }
                TrouterEvent::Resync => {
                    let e = self.clone();
                    tokio::spawn(async move {
                        e.flush_outbox(true).await;
                        let _ = e.sync().await;
                    });
                }
                TrouterEvent::Disconnected(_) => {
                    was_disconnected = true;
                    self.connected.store(false, Ordering::Relaxed);
                    self.ui.connected(false);
                }
                TrouterEvent::Event(ev) => self.on_event(*ev),
            }
        }
    }

    fn on_event(self: &Arc<Self>, ev: Event) {
        match ev {
            Event::NewMessage(m) | Event::MessageUpdate(m) => self.on_message(m),
            Event::Typing {
                conversation_id,
                from,
            } => {
                if from != self.me {
                    let name = self
                        .store
                        .lock()
                        .unwrap()
                        .user(&from)
                        .ok()
                        .flatten()
                        .map(|u| u.display_name)
                        .unwrap_or_else(|| "Someone".into());
                    self.view.lock().unwrap().typing.insert(
                        conversation_id,
                        (name, Instant::now() + Duration::from_secs(6)),
                    );
                }
            }
            Event::ConversationUpdate { conversation_id }
            | Event::ThreadUpdate { conversation_id } => {
                let known = self
                    .store
                    .lock()
                    .unwrap()
                    .conversation(&conversation_id)
                    .ok()
                    .flatten()
                    .is_some();
                if !known {
                    let e = self.clone();
                    tokio::spawn(async move {
                        let _ = e.sync().await;
                    });
                }
            }
            Event::Presence { .. } => {}
        }
    }

    fn on_message(self: &Arc<Self>, m: Message) {
        if m.conversation_id == "48:notifications" {
            // The Activity feed: reload it instead of storing the notification as a chat message.
            self.on_activity_event();
            return;
        }
        if m.message_type.starts_with("Control/") {
            return;
        }
        let mine = m.sender_mri == self.me;
        let mut outbox_echo = None;
        let (is_current, focused, follow) = {
            let mut v = self.view.lock().unwrap();
            // Our own echo replaces the optimistic copy.
            if let Some(cmid) = &m.client_message_id {
                v.pending.retain(|(c, _)| c != cmid);
                if mine {
                    outbox_echo = Some(cmid.clone());
                }
            }
            v.typing.remove(&m.conversation_id);
            let follow = follow_new_message(v.at_bottom, mine);
            if follow {
                v.at_bottom = true;
            }
            (
                v.current.as_deref() == Some(m.conversation_id.as_str()),
                self.focused.load(Ordering::Relaxed),
                follow,
            )
        };
        if let Some(cmid) = outbox_echo {
            let _ = self.store.lock().unwrap().remove(&cmid);
        }
        let (known, counted) = {
            let st = self.store.lock().unwrap();
            let existed = st.message(&m.id).ok().flatten().is_some();
            let _ = st.upsert_message(&row_from_message(&m, None));
            let is_new_top = !existed && m.parent_id.is_none() && m.is_chat_content();
            if is_new_top || (!existed && m.parent_id.is_some()) {
                let unread = !mine && !(is_current && focused);
                let _ = st.touch_conversation(
                    &m.conversation_id,
                    m.compose_ms,
                    &preview_of(&m, &self.me),
                    unread,
                );
            }
            let counted =
                is_current && !follow && !existed && m.parent_id.is_none() && m.is_chat_content();
            if should_notify(existed, mine, m.is_chat_content(), is_current, focused) {
                self.notify(&st, &m);
            }
            (
                st.conversation(&m.conversation_id).ok().flatten().is_some(),
                counted,
            )
        };
        if is_current && focused && !mine {
            let e = self.clone();
            let msg = m.clone();
            tokio::spawn(async move {
                let _ = e.client.mark_read(&msg.conversation_id, &msg).await;
            });
        }
        if !known {
            let e = self.clone();
            tokio::spawn(async move {
                let _ = e.sync().await;
            });
        }
        if counted {
            // Scrolled up: leave the list alone and count the message on the jump button.
            let n = {
                let mut v = self.view.lock().unwrap();
                v.new_count += 1;
                v.new_count
            };
            self.ui.new_count(n);
        }
        self.mark(|d| {
            d.chats = true;
            if is_current {
                d.messages = true;
                d.scroll |= follow;
            }
        });
    }

    fn notify(&self, st: &Store, m: &Message) {
        // Muted chats and channels set to "Off" stay silent here, like in Teams, and so does
        // everything while the tray's Mute is on.
        if self.lists_silenced(&m.conversation_id) || crate::tray_wiring::notifications_muted() {
            return;
        }
        // The notification settings from Teams: type switches, pause and previews.
        let Some(preview) = super::settings::banner_for(m, &self.me) else {
            return;
        };
        let title = st
            .conversation(&m.conversation_id)
            .ok()
            .flatten()
            .map(|c| c.title)
            .unwrap_or_default();
        let summary = if title.is_empty() || title == m.sender_name {
            m.sender_name.clone()
        } else {
            format!("{} — {}", m.sender_name, title)
        };
        let body: String = render::plain_text(&render::parse_teams_html(&m.content))
            .chars()
            .take(200)
            .collect();
        let body = super::settings::banner_body(preview, body);
        tracing::info!(conversation = %m.conversation_id, "desktop notification");
        tokio::task::spawn_blocking(move || {
            let _ = notify_rust::Notification::new()
                .appname("kelma")
                .summary(&summary)
                .body(&body)
                .icon("mail-message-new")
                .show();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::should_notify;

    #[test]
    fn notifies_only_for_new_messages_from_others_out_of_view() {
        // New message from someone else in another chat, or with the window unfocused.
        assert!(should_notify(false, false, true, false, true));
        assert!(should_notify(false, false, true, true, false));
        // Already looking at it.
        assert!(!should_notify(false, false, true, true, true));
        // Own messages, repeats (edits, resync) and system lines never notify.
        assert!(!should_notify(false, true, true, false, false));
        assert!(!should_notify(true, false, true, false, false));
        assert!(!should_notify(false, false, false, false, false));
    }
}
