//! Outgoing actions: send/edit/delete, reactions, attachments and media playback.

use super::Engine;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use store::MessageRow;
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
        let (target, reply, editing, current) = {
            let mut v = self.view.lock().unwrap();
            (
                self.current_target_locked(&v),
                v.reply_to.take(),
                v.editing.take(),
                v.current.clone(),
            )
        };
        self.ui.reply_bar(String::new(), String::new(), false);
        let (Some(target), Some(current)) = (target, current) else {
            return Ok(());
        };
        if let Some(id) = editing {
            // The composer shows the draft that was there before the edit again.
            let draft = self.view.lock().unwrap().draft_text.clone();
            self.ui.set_draft(draft);
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
        self.clear_draft(&current);
        let pending_id = format!("pending-{}", now_ms());
        let thread_root = self.view.lock().unwrap().thread_root.clone();
        let row = MessageRow {
            id: pending_id.clone(),
            conversation_id: current.clone(),
            parent_id: thread_root,
            sender_mri: self.me.clone(),
            sender_name: self.my_name.clone(),
            compose_ms: now_ms(),
            edited_ms: None,
            deleted: false,
            message_type: "RichText/Html".into(),
            content_html: html.clone(),
            client_message_id: None,
            reactions_json: json!({"s": if subject.is_empty() { Value::Null } else { Value::String(subject.clone()) }}).to_string(),
        };
        self.view
            .lock()
            .unwrap()
            .pending
            .push((pending_id.clone(), row));
        self.mark(|d| {
            d.messages = true;
            d.scroll = true;
        });
        let subj = (!subject.is_empty()).then_some(subject.as_str());
        match self
            .client
            .send_message(&target, &html, &self.my_name, subj)
            .await
        {
            Ok(cmid) => {
                let mut v = self.view.lock().unwrap();
                if let Some(p) = v.pending.iter_mut().find(|(id, _)| id == &pending_id) {
                    p.0 = cmid;
                }
                drop(v);
                self.ui.clear_subject();
                let st = self.store.lock().unwrap();
                let _ = st.touch_conversation(&current, now_ms(), &format!("You: {text}"), false);
                drop(st);
                self.mark(|d| d.chats = true);
                // Fallback in case the live echo never arrives.
                let e = self.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(8)).await;
                    let stale = e.view.lock().unwrap().pending.iter().any(|(c, r)| {
                        r.conversation_id == current && c.chars().all(|ch| ch.is_ascii_digit())
                    });
                    if stale {
                        e.fetch_latest(&target).await;
                        e.view.lock().unwrap().pending.retain(|(c, r)| {
                            !(r.conversation_id == current
                                && c.chars().all(|ch| ch.is_ascii_digit()))
                        });
                        e.mark(|d| d.messages = true);
                    }
                });
            }
            Err(err) => {
                self.view
                    .lock()
                    .unwrap()
                    .pending
                    .retain(|(id, _)| id != &pending_id);
                self.ui.set_draft(text.clone());
                self.view.lock().unwrap().draft_text = text;
                self.schedule_draft_save();
                self.mark(|d| d.messages = true);
                return Err(err.into());
            }
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

    pub(super) async fn attach_image(self: &Arc<Self>) -> anyhow::Result<()> {
        let Some(target) = self.current_target() else {
            return Ok(());
        };
        let Some(file) = rfd::AsyncFileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
            .pick_file()
            .await
        else {
            return Ok(());
        };
        let data = file.read().await;
        let (w, h) = image::load_from_memory(&data)
            .map(|i| (i.width(), i.height()))
            .unwrap_or((400, 300));
        let (w, h) = if w > 400 {
            (400, h * 400 / w.max(1))
        } else {
            (w, h)
        };
        let conv_only = target
            .split(";messageid=")
            .next()
            .unwrap_or(&target)
            .to_string();
        let id = self
            .client
            .upload_image(&conv_only, &file.file_name(), data)
            .await?;
        let html = teams_proto::ams::image_html(&id, w, h);
        self.client
            .send_message(&target, &html, &self.my_name, None)
            .await?;
        self.ui.banner(String::new());
        Ok(())
    }

    /// Picks a file, uploads it to OneDrive, shares it with the chat and posts it.
    pub(super) async fn attach_file(self: &Arc<Self>) -> anyhow::Result<()> {
        let Some(target) = self.current_target() else {
            return Ok(());
        };
        let Some(file) = rfd::AsyncFileDialog::new().pick_file().await else {
            return Ok(());
        };
        let name = file.file_name();
        self.ui.banner(format!("Uploading {name}…"));
        let data = file.read().await;
        self.client
            .send_file(&target, &name, data, &self.my_name)
            .await?;
        self.ui.banner(String::new());
        Ok(())
    }
}
