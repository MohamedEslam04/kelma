//! Outgoing actions: send/edit/delete, reactions, attachments and media playback.

use super::Engine;
use super::mentions;
use serde_json::{Value, json};
use std::sync::Arc;
use store::OutboxKind;
use teams_proto::time::now_ms;

impl Engine {
    pub(super) async fn send(
        self: &Arc<Self>,
        text: String,
        subject: String,
    ) -> anyhow::Result<()> {
        let text = text.trim().to_string();
        let editing_now = self.view.lock().unwrap().editing.is_some();
        // Chips go first, each as its own outbox item (an image or a file card), then the text.
        if !editing_now && let Err(err) = self.queue_attachments() {
            // Whatever could not be queued is still a chip; the text goes back to the composer.
            if !text.is_empty() {
                self.ui.set_draft(text.clone());
                self.view.lock().unwrap().draft_text = text;
                self.schedule_draft_save();
            }
            self.flush_outbox(false).await;
            return Err(err);
        }
        if text.is_empty() {
            // Nothing but attachments (or nothing at all).
            self.flush_outbox(false).await;
            return Ok(());
        }
        // The @Name tokens the user picked become mention spans; the entries are used up.
        let prepared = {
            let mut p = self.people.lock().unwrap();
            let prepared = mentions::prepare(&text, &p.entries);
            p.entries.clear();
            prepared
        };
        let (target, reply, editing) = {
            let mut v = self.view.lock().unwrap();
            (
                self.current_target_locked(&v),
                v.reply_to.take(),
                v.editing.take(),
            )
        };
        self.ui.reply_bar(String::new(), String::new(), false);
        let Some(target) = target else {
            return Ok(());
        };
        if let Some(id) = editing {
            // The composer shows the draft that was there before the edit again.
            let draft = self.view.lock().unwrap().draft_text.clone();
            self.ui.set_draft(draft);
            let html = mentions::finish(&render::text_to_teams_html(&prepared.text), &prepared);
            if prepared.mentions.is_empty() {
                self.client
                    .edit_message(&target, &id, &html, &self.my_name)
                    .await?;
            } else {
                // An edit replaces the message properties, so the mentions are sent again.
                self.client
                    .edit_message_with_mentions(
                        &target,
                        &id,
                        &html,
                        &self.my_name,
                        &prepared.mentions,
                    )
                    .await?;
            }
            let st = self.store.lock().unwrap();
            if let Some(mut r) = st.message(&id)? {
                r.content_html = html;
                r.edited_ms = Some(now_ms());
                st.upsert_message(&r)?;
            }
            drop(st);
            self.mark(|d| d.messages = true);
            return Ok(());
        }
        let html = match &reply {
            Some(r) => {
                let preview: String =
                    render::plain_text(&render::parse_teams_html(&r.content_html))
                        .chars()
                        .take(200)
                        .collect();
                render::reply_html(
                    &r.id,
                    &r.sender_mri,
                    &r.sender_name,
                    r.compose_ms,
                    &preview,
                    &prepared.text,
                )
            }
            None => render::text_to_teams_html(&prepared.text),
        };
        let html = mentions::finish(&html, &prepared);
        // The draft is sent: forget it. It comes back if the message cannot even be queued.
        let current = self
            .view
            .lock()
            .unwrap()
            .current
            .clone()
            .unwrap_or_default();
        self.clear_draft(&current);
        let mut item = self.new_outbox_item(
            target,
            OutboxKind::Text,
            html,
            (!subject.is_empty()).then(|| subject.clone()),
            reply.map(|r| {
                // The quote block is in the html; this property links it to the original.
                teams_proto::chatsvc::quoted_messages_property(&r.id, &r.sender_mri, r.compose_ms)
            }),
            None,
        );
        item.mentions = mentions::property(&prepared);
        self.ui.clear_subject();
        if let Err(err) = self.enqueue(item) {
            self.ui.set_draft(text.clone());
            self.view.lock().unwrap().draft_text = text;
            self.schedule_draft_save();
            self.flush_outbox(false).await;
            return Err(err);
        }
        self.flush_outbox(false).await;
        Ok(())
    }

    pub(super) async fn react(self: &Arc<Self>, msg_id: &str, key: &str) -> anyhow::Result<()> {
        let Some(target) = self.current_target() else {
            return Ok(());
        };
        let row = self.store.lock().unwrap().message(msg_id)?;
        let Some(mut row) = row else { return Ok(()) };
        let mut props: Value =
            serde_json::from_str(&row.reactions_json).unwrap_or_else(|_| json!({}));
        let mut reactions: Vec<Value> = props
            .get("r")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mine = reactions.iter().any(|r| {
            r["key"] == key
                && r["users"]
                    .as_array()
                    .is_some_and(|u| u.iter().any(|x| x == self.me.as_str()))
        });
        let conv_for_msg = if row.parent_id.is_some() {
            target.clone()
        } else {
            row.conversation_id.clone()
        };
        if mine {
            self.client
                .remove_reaction(&conv_for_msg, msg_id, key)
                .await?;
            for r in reactions.iter_mut().filter(|r| r["key"] == key) {
                if let Some(u) = r["users"].as_array_mut() {
                    u.retain(|x| x != self.me.as_str());
                }
            }
            reactions.retain(|r| r["users"].as_array().is_some_and(|u| !u.is_empty()));
        } else {
            self.client.add_reaction(&conv_for_msg, msg_id, key).await?;
            match reactions.iter_mut().find(|r| r["key"] == key) {
                Some(r) => {
                    if let Some(u) = r["users"].as_array_mut() {
                        u.push(json!(self.me));
                    }
                }
                None => reactions.push(json!({"key": key, "users": [self.me]})),
            }
        }
        props["r"] = json!(reactions);
        row.reactions_json = props.to_string();
        self.store.lock().unwrap().upsert_message(&row)?;
        self.mark(|d| d.messages = true);
        Ok(())
    }

    /// Downloads a voice message / video with credentials and opens it in the system player.
    pub(super) async fn play_media(&self, url: &str) -> anyhow::Result<()> {
        let dir = session::Paths::new()?.cache_dir.join("media");
        std::fs::create_dir_all(&dir)?;
        let name: String = url
            .split("/objects/")
            .nth(1)
            .unwrap_or("media")
            .split('/')
            .next()
            .unwrap_or("media")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        let path = dir.join(format!("{name}.m4a"));
        if !path.exists() {
            self.ui.banner("Downloading…".into());
            let data = self.client.download_media(url, 100 * 1024 * 1024).await?;
            std::fs::write(&path, data)?;
            self.ui.banner(String::new());
        }
        open::that_detached(&path)?;
        Ok(())
    }

    /// Copies a chip's bytes into the cache (so they outlive the composer, and a restart) and
    /// queues them in the outbox as an image or file item. Sending is left to `flush_outbox`.
    pub(super) fn queue_attachment(
        &self,
        target: String,
        kind: OutboxKind,
        name: &str,
        data: &[u8],
    ) -> anyhow::Result<()> {
        let item = self.new_outbox_item(target, kind, String::new(), None, None, None);
        let dir = session::Paths::new()?
            .cache_dir
            .join("outbox")
            .join(&item.client_message_id);
        // Keep only the file name: it is shown in the row and used as the upload name.
        let name = std::path::Path::new(name)
            .file_name()
            .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
        let path = dir.join(name);
        let item = store::OutboxItem {
            payload_path: Some(path.to_string_lossy().into_owned()),
            ..item
        };
        let queued = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(&path, data))
            .map_err(anyhow::Error::from)
            .and_then(|()| self.enqueue(item));
        if queued.is_err() {
            let _ = std::fs::remove_dir_all(dir);
        }
        queued
    }

    /// Picks an image and adds it as a chip; it goes through the outbox when the message is sent.
    pub(super) async fn attach_image(self: &Arc<Self>) -> anyhow::Result<()> {
        let Some(file) = rfd::AsyncFileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
            .pick_file()
            .await
        else {
            return Ok(());
        };
        self.add_attachment(file.file_name(), file.read().await)
            .await
    }

    /// Picks a file and adds it as a chip; it is uploaded to OneDrive from the outbox when the
    /// message is sent.
    pub(super) async fn attach_file(self: &Arc<Self>) -> anyhow::Result<()> {
        let Some(file) = rfd::AsyncFileDialog::new().pick_file().await else {
            return Ok(());
        };
        self.add_attachment(file.file_name(), file.read().await)
            .await
    }
}
