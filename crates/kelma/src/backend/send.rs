//! Outgoing actions: send/edit/delete, reactions, attachments and media playback.

use super::Engine;
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
        if text.is_empty() {
            return Ok(());
        }
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
            let html = render::text_to_teams_html(&text);
            self.client
                .edit_message(&target, &id, &html, &self.my_name)
                .await?;
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
                    &text,
                )
            }
            None => render::text_to_teams_html(&text),
        };
        let item = self.new_outbox_item(
            target,
            OutboxKind::Text,
            html,
            (!subject.is_empty()).then(|| subject.clone()),
            reply.map(|r| r.id),
            None,
        );
        self.ui.clear_subject();
        if let Err(err) = self.enqueue_and_flush(item).await {
            self.ui.set_draft(text);
            return Err(err);
        }
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

    /// Copies a picked file into the cache (so it outlives the file dialog, and a restart) and
    /// queues it in the outbox.
    async fn queue_attachment(
        self: &Arc<Self>,
        kind: OutboxKind,
        name: &str,
        data: Vec<u8>,
    ) -> anyhow::Result<()> {
        let Some(target) = self.current_target() else {
            return Ok(());
        };
        let item = self.new_outbox_item(target, kind, String::new(), None, None, None);
        let dir = session::Paths::new()?
            .cache_dir
            .join("outbox")
            .join(&item.client_message_id);
        std::fs::create_dir_all(&dir)?;
        // Keep only the file name: it is shown in the row and used as the upload name.
        let name = std::path::Path::new(name)
            .file_name()
            .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
        let path = dir.join(name);
        std::fs::write(&path, data)?;
        let item = store::OutboxItem {
            payload_path: Some(path.to_string_lossy().into_owned()),
            ..item
        };
        if let Err(err) = self.enqueue_and_flush(item).await {
            let _ = std::fs::remove_dir_all(dir);
            return Err(err);
        }
        Ok(())
    }

    pub(super) async fn attach_image(self: &Arc<Self>) -> anyhow::Result<()> {
        if self.current_target().is_none() {
            return Ok(());
        }
        let Some(file) = rfd::AsyncFileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
            .pick_file()
            .await
        else {
            return Ok(());
        };
        let data = file.read().await;
        self.queue_attachment(OutboxKind::Image, &file.file_name(), data)
            .await
    }

    /// Picks a file; it is uploaded to OneDrive, shared with the chat and posted from the outbox.
    pub(super) async fn attach_file(self: &Arc<Self>) -> anyhow::Result<()> {
        if self.current_target().is_none() {
            return Ok(());
        }
        let Some(file) = rfd::AsyncFileDialog::new().pick_file().await else {
            return Ok(());
        };
        let data = file.read().await;
        self.queue_attachment(OutboxKind::File, &file.file_name(), data)
            .await
    }
}
