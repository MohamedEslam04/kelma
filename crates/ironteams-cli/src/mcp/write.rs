//! The write tools. `draft_*` tools change nothing in Teams: they freeze what would happen and
//! return a preview. `send_draft` does one draft, once.

use super::Server;
use super::drafts::{Action, DraftError, LIFETIME_MS};
use super::guard::check_chat;
use super::read::chat_kind_name;
use super::text::{clip, iso_minute, plain};
use calendar::Rsvp;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use teams_proto::chatsvc::quoted_messages_property;
use teams_proto::model::ConversationKind;
use teams_proto::presence::Availability;
use teams_proto::time::now_ms;

/// The longest message a draft may hold, in characters.
const MAX_MESSAGE: usize = 20_000;
const MAX_NOTE: usize = 280;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e}")
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DraftMessage {
    /// The chat id, from list_chats or catch_up.
    pub chat_id: String,
    /// The message, as plain text (new lines are kept).
    pub text: String,
    /// To reply with a quote: the id of the message you answer (one of the chat's last 50).
    #[serde(default)]
    pub reply_to_message_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DraftReaction {
    pub chat_id: String,
    /// The message to react to (one of the chat's last 50).
    pub message_id: String,
    /// `like`, `heart`, `laugh`, `surprised`, `sad` or `angry`.
    pub reaction: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DraftMarkRead {
    pub chat_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DraftPresence {
    /// `Available`, `Busy`, `DoNotDisturb`, `BeRightBack`, `Away`, `Offline`, or `Reset` to go
    /// back to the automatic status.
    pub availability: String,
    /// For how many minutes (default 60, at most 1440). Not used with `Reset`.
    #[serde(default)]
    pub minutes: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DraftStatusNote {
    /// The status message (at most 280 characters); empty clears it.
    pub text: String,
    /// Clear it after this many hours (default 8, at most 168).
    #[serde(default)]
    pub hours: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DraftMeetingResponse {
    /// The event id, from calendar_agenda.
    pub event_id: String,
    /// `accept`, `tentative` or `decline`.
    pub response: String,
    /// A note for the organizer.
    #[serde(default)]
    pub comment: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DraftId {
    /// The id a draft_* tool returned.
    pub draft_id: String,
}

/// The reaction keys a draft accepts.
const REACTIONS: [&str; 6] = ["like", "heart", "laugh", "surprised", "sad", "angry"];

fn parse_availability(s: &str) -> Result<Option<Availability>, String> {
    match s.trim() {
        "Reset" | "reset" | "Automatic" | "automatic" => Ok(None),
        other => Availability::from_wire(other)
            .filter(|a| a.as_str() == other)
            .map(Some)
            .ok_or_else(|| {
                format!(
                    "unknown availability {other:?}: use Available, Busy, DoNotDisturb, BeRightBack, Away, Offline or Reset"
                )
            }),
    }
}

fn parse_rsvp(s: &str) -> Result<Rsvp, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "accept" => Ok(Rsvp::Accept),
        "tentative" => Ok(Rsvp::Tentative),
        "decline" => Ok(Rsvp::Decline),
        other => Err(format!(
            "unknown response {other:?}: use accept, tentative or decline"
        )),
    }
}

/// What a draft tool returns.
fn draft_reply(id: &str, preview: &str) -> String {
    format!(
        "Draft {id}. Nothing has been sent.\n\n{preview}\n\nShow this draft to the user. Only if they approve it, call send_draft with draft_id \"{id}\". It expires in {} minutes.",
        LIFETIME_MS / 60_000
    )
}

impl Server {
    /// Keeps a draft and returns what the tool answers.
    fn keep(&self, action: Action, preview: String) -> String {
        let id = self.drafts().add(action, preview.clone(), now_ms());
        draft_reply(&id, &preview)
    }

    /// `the group chat "Topic" (5 members)`, as well as the service tells.
    async fn describe_chat(&self, chat_id: &str) -> String {
        let kind = chat_kind_name(chat_id);
        if check_chat(chat_id) == Ok(ConversationKind::Notes) {
            return kind.to_string();
        }
        match self.client.thread_members(chat_id).await {
            Ok((members, topic)) => {
                let topic = topic
                    .filter(|t| !t.trim().is_empty())
                    .map(|t| format!(" \"{t}\""))
                    .unwrap_or_default();
                format!("the {kind}{topic} {chat_id} ({} members)", members.len())
            }
            Err(_) => format!("the {kind} {chat_id}"),
        }
    }

    /// One of the chat's last 50 messages.
    async fn recent_message(
        &self,
        chat_id: &str,
        message_id: &str,
    ) -> Result<teams_proto::model::Message, String> {
        let page = self.client.messages(chat_id, None, 50).await.map_err(err)?;
        page.messages
            .into_iter()
            .find(|m| m.id == message_id)
            .ok_or_else(|| "that message is not among the chat's last 50 messages".to_string())
    }

    /// Does what a draft froze.
    async fn execute(&self, a: &Action) -> Result<(), String> {
        let c = &self.client;
        match a {
            Action::Message {
                chat_id,
                html,
                quote: None,
                client_message_id,
            } => c
                .send_message_as(chat_id, html, &self.display_name, None, client_message_id)
                .await
                .map_err(err),
            Action::Message {
                chat_id,
                html,
                quote: Some(qtd),
                client_message_id,
            } => c
                .send_quote_reply_as(chat_id, html, &self.display_name, qtd, client_message_id)
                .await
                .map_err(err),
            Action::Reaction {
                chat_id,
                message_id,
                key,
            } => c.add_reaction(chat_id, message_id, key).await.map_err(err),
            Action::MarkRead { chat_id, message } => {
                c.mark_read(chat_id, message).await.map_err(err)
            }
            Action::Presence {
                availability: Some(av),
                until_iso,
            } => c.set_presence(*av, until_iso.as_deref()).await.map_err(err),
            Action::Presence {
                availability: None, ..
            } => c.reset_presence().await.map_err(err),
            Action::StatusNote { text, until_iso } => c
                .set_status_note(text, until_iso.as_deref(), false)
                .await
                .map_err(err),
            Action::MeetingResponse {
                event_id,
                rsvp,
                comment,
            } => calendar::respond(c, event_id, *rsvp, comment, true, None)
                .await
                .map_err(err),
        }
    }
}

#[tool_router(router = write_router, vis = "pub(super)")]
impl Server {
    #[tool(
        description = "Draft a message (or a quoted reply) in a chat. Nothing is sent: returns a draft id and a preview for the user to approve; then send_draft sends it.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn draft_message(
        &self,
        Parameters(p): Parameters<DraftMessage>,
    ) -> Result<String, String> {
        check_chat(&p.chat_id)?;
        let text = p.text.trim_end();
        if text.trim().is_empty() {
            return Err("empty message".into());
        }
        if text.chars().count() > MAX_MESSAGE {
            return Err(format!(
                "the message is longer than {MAX_MESSAGE} characters"
            ));
        }
        let (html, quote, answering) = match &p.reply_to_message_id {
            None => (render::text_to_teams_html(text), None, String::new()),
            Some(id) => {
                let orig = self.recent_message(&p.chat_id, id).await?;
                let quoted = plain(&orig.content);
                let html = render::reply_html(
                    &orig.id,
                    &orig.sender_mri,
                    &orig.sender_name,
                    orig.compose_ms,
                    &quoted,
                    text,
                );
                let qtd = quoted_messages_property(&orig.id, &orig.sender_mri, orig.compose_ms);
                let answering = format!(
                    "\nIn reply to {} ({}): {}",
                    orig.sender_name,
                    iso_minute(orig.compose_ms),
                    clip(&quoted.replace('\n', " "), 200)
                );
                (html, Some(qtd), answering)
            }
        };
        let to = self.describe_chat(&p.chat_id).await;
        let preview = format!("Send to {to}:{answering}\n---\n{text}\n---");
        let action = Action::Message {
            chat_id: p.chat_id,
            html,
            quote,
            client_message_id: now_ms().to_string(),
        };
        Ok(self.keep(action, preview))
    }

    #[tool(
        description = "Draft a reaction to a chat message. Nothing is sent until send_draft.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn draft_reaction(
        &self,
        Parameters(p): Parameters<DraftReaction>,
    ) -> Result<String, String> {
        check_chat(&p.chat_id)?;
        if !REACTIONS.contains(&p.reaction.as_str()) {
            return Err(format!(
                "unknown reaction: use one of {}",
                REACTIONS.join(", ")
            ));
        }
        let m = self.recent_message(&p.chat_id, &p.message_id).await?;
        let to = self.describe_chat(&p.chat_id).await;
        let preview = format!(
            "React {} in {to} to {}'s message ({}): {}",
            p.reaction,
            m.sender_name,
            iso_minute(m.compose_ms),
            clip(&plain(&m.content).replace('\n', " "), 200)
        );
        let action = Action::Reaction {
            chat_id: p.chat_id,
            message_id: p.message_id,
            key: p.reaction,
        };
        Ok(self.keep(action, preview))
    }

    #[tool(
        description = "Draft marking a chat as read (up to its newest message). Nothing changes until send_draft.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn draft_mark_read(
        &self,
        Parameters(p): Parameters<DraftMarkRead>,
    ) -> Result<String, String> {
        check_chat(&p.chat_id)?;
        let page = self
            .client
            .messages(&p.chat_id, None, 5)
            .await
            .map_err(err)?;
        let newest = page
            .messages
            .into_iter()
            .max_by_key(|m| m.compose_ms)
            .ok_or("the chat has no messages")?;
        let to = self.describe_chat(&p.chat_id).await;
        let preview = format!(
            "Mark {to} as read, up to the message of {}.",
            iso_minute(newest.compose_ms)
        );
        let action = Action::MarkRead {
            chat_id: p.chat_id,
            message: Box::new(newest),
        };
        Ok(self.keep(action, preview))
    }

    #[tool(
        description = "Draft setting your Teams status (Available, Busy, DoNotDisturb, BeRightBack, Away, Offline) for some minutes, or Reset to the automatic status. Nothing changes until send_draft.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn draft_presence(
        &self,
        Parameters(p): Parameters<DraftPresence>,
    ) -> Result<String, String> {
        let availability = parse_availability(&p.availability)?;
        let (until_iso, preview) = match availability {
            None => (None, "Reset your status to automatic.".to_string()),
            Some(a) => {
                let minutes = p.minutes.unwrap_or(60).clamp(5, 1440);
                let until = now_ms() + i64::from(minutes) * 60_000;
                (
                    Some(calendar::format_iso_ms(until)),
                    format!(
                        "Set your status to {} until {} (UTC).",
                        a.as_str(),
                        iso_minute(until)
                    ),
                )
            }
        };
        Ok(self.keep(
            Action::Presence {
                availability,
                until_iso,
            },
            preview,
        ))
    }

    #[tool(
        description = "Draft your Teams status message (empty text clears it). Nothing changes until send_draft.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn draft_status_note(
        &self,
        Parameters(p): Parameters<DraftStatusNote>,
    ) -> Result<String, String> {
        let text = p.text.trim().to_string();
        if text.chars().count() > MAX_NOTE {
            return Err(format!(
                "a status message has at most {MAX_NOTE} characters"
            ));
        }
        let (until_iso, preview) = if text.is_empty() {
            (None, "Clear your status message.".to_string())
        } else {
            let until = now_ms() + i64::from(p.hours.unwrap_or(8).clamp(1, 168)) * 3_600_000;
            (
                Some(calendar::format_iso_ms(until)),
                format!(
                    "Set your status message until {} (UTC):\n---\n{text}\n---",
                    iso_minute(until)
                ),
            )
        };
        Ok(self.keep(Action::StatusNote { text, until_iso }, preview))
    }

    #[tool(
        description = "Draft an answer (accept, tentative or decline) to a meeting invitation; the organizer gets it. Nothing is sent until send_draft.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn draft_meeting_response(
        &self,
        Parameters(p): Parameters<DraftMeetingResponse>,
    ) -> Result<String, String> {
        let rsvp = parse_rsvp(&p.response)?;
        let d = calendar::event_detail(&self.client, &p.event_id)
            .await
            .map_err(err)?;
        if d.is_organizer {
            return Err("you organize this meeting; there is nothing to answer".into());
        }
        let comment = p.comment.unwrap_or_default().trim().to_string();
        let e = &d.event;
        let mut preview = format!(
            "Answer {:?} to \"{}\" ({} - {} UTC) from {}; the organizer gets your answer.",
            rsvp,
            e.subject,
            iso_minute(e.start_ms),
            iso_minute(e.end_ms),
            e.organizer_name
        );
        if !comment.is_empty() {
            preview.push_str(&format!("\nNote:\n---\n{comment}\n---"));
        }
        let action = Action::MeetingResponse {
            event_id: p.event_id,
            rsvp,
            comment,
        };
        Ok(self.keep(action, preview))
    }

    #[tool(
        description = "Send (do) a draft the user has approved in this conversation. Never call it without that approval.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn send_draft(&self, Parameters(p): Parameters<DraftId>) -> Result<String, String> {
        let now = now_ms();
        let taken = self.drafts().take(&p.draft_id, now);
        let draft = match taken {
            Ok(d) => d,
            Err(e) => {
                let outcome = match e {
                    DraftError::Unknown => "unknown_draft",
                    DraftError::Expired => "expired",
                };
                self.audit.record(now, "-", None, &p.draft_id, outcome);
                return Err(e.to_string());
            }
        };
        let (kind, target) = (
            draft.action.kind(),
            draft.action.target().map(str::to_string),
        );
        if let Some(chat) = draft.action.chat_id()
            && let Err(e) = check_chat(chat)
        {
            self.audit
                .record(now, kind, target.as_deref(), &p.draft_id, "refused");
            return Err(e);
        }
        let result = self.execute(&draft.action).await;
        self.audit.record(
            now_ms(),
            kind,
            target.as_deref(),
            &p.draft_id,
            if result.is_ok() { "sent" } else { "failed" },
        );
        match result {
            Ok(()) => Ok(format!("Done.\n{}", draft.preview)),
            Err(e) => Err(format!(
                "Teams refused it: {e}. The draft is used up; check the chat before you draft it again."
            )),
        }
    }

    #[tool(
        description = "Throw a draft away.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn discard_draft(&self, Parameters(p): Parameters<DraftId>) -> String {
        if self.drafts().discard(&p.draft_id) {
            "Discarded.".into()
        } else {
            "No such draft.".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_names_are_exact() {
        assert_eq!(parse_availability("Busy"), Ok(Some(Availability::Busy)));
        assert_eq!(
            parse_availability("DoNotDisturb"),
            Ok(Some(Availability::DoNotDisturb))
        );
        assert_eq!(parse_availability("Reset"), Ok(None));
        assert!(
            parse_availability("BusyIdle").is_err(),
            "a reported value, not a choice"
        );
        assert!(parse_availability("busy").is_err());
        assert!(parse_availability("").is_err());
    }

    #[test]
    fn meeting_answers_are_three() {
        assert_eq!(parse_rsvp("accept"), Ok(Rsvp::Accept));
        assert_eq!(parse_rsvp(" Tentative "), Ok(Rsvp::Tentative));
        assert_eq!(parse_rsvp("DECLINE"), Ok(Rsvp::Decline));
        assert!(parse_rsvp("maybe").is_err());
    }

    #[test]
    fn a_draft_reply_says_nothing_was_sent_and_how_to_send() {
        let r = draft_reply("draft-1-1", "Send to your own notes:\n---\nhi\n---");
        assert!(r.starts_with("Draft draft-1-1. Nothing has been sent."));
        assert!(r.contains("send_draft with draft_id \"draft-1-1\""));
        assert!(r.contains("10 minutes"));
    }
}
