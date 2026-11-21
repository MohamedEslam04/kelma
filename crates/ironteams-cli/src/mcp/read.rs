//! The read tools. None of them changes anything in Teams or in the server.

use super::Server;
use super::guard::{check_chat, is_chat, kind_name};
use super::text::{MAX_BODY, chat_line, clip, iso_minute, message_line, plain};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use teams_proto::model::conversation_kind;
use teams_proto::search::SearchFilters;
use teams_proto::time::now_ms;

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e}")
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListChats {
    /// Only chats with messages you have not read.
    #[serde(default)]
    pub unread_only: bool,
    /// How many chats, newest activity first (default 30, at most 100).
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadChat {
    /// The chat id, from list_chats or catch_up.
    pub chat_id: String,
    /// How many messages (default 30, at most 200).
    #[serde(default)]
    pub limit: Option<u32>,
    /// To page back: only messages older than this time (Unix milliseconds, for example the
    /// oldest message you already have).
    #[serde(default)]
    pub before_ms: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CatchUp {
    /// How far back to look for unread chats and mentions, in hours (default 24, at most 336).
    #[serde(default)]
    pub since_hours: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchMessages {
    /// What to look for (words, as in the Teams search box).
    pub query: String,
    /// Only in this chat.
    #[serde(default)]
    pub chat_id: Option<String>,
    /// Only messages sent by this person (their id, `8:orgid:…`, from find_people or read_chat).
    #[serde(default)]
    pub from_person: Option<String>,
    /// How many hits (default 15, at most 50).
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindPeople {
    /// A name or email address, or part of one.
    pub query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetPresence {
    /// Person ids (`8:orgid:…`), from find_people or read_chat.
    pub people: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CalendarAgenda {
    /// Start this many days from now (default 0; negative looks back).
    #[serde(default)]
    pub from_day: Option<i32>,
    /// How many days to cover (default 1, at most 14).
    #[serde(default)]
    pub days: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MeetingDetail {
    /// The event id, from calendar_agenda.
    pub event_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindFreeTime {
    /// Email addresses of the people who must attend (you are always included).
    #[serde(default)]
    pub attendees: Vec<String>,
    /// Meeting length in minutes (default 30).
    #[serde(default)]
    pub duration_minutes: Option<u32>,
    /// Look this many days ahead (default 5, at most 14).
    #[serde(default)]
    pub within_days: Option<u32>,
}

fn limit(v: Option<u32>, default: u32, max: u32) -> u32 {
    v.unwrap_or(default).clamp(1, max)
}

#[tool_router(router = read_router, vis = "pub(super)")]
impl Server {
    #[tool(
        description = "List your Teams chats (1:1, group and meeting chats and your notes; never channels), newest activity first. One line per chat: id | kind | title | read state | last activity (UTC) | last message.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn list_chats(&self, Parameters(p): Parameters<ListChats>) -> Result<String, String> {
        let n = limit(p.limit, 30, 100) as usize;
        let mut convs = self.client.conversations(0, 100).await.map_err(err)?;
        convs.retain(|c| is_chat(c.kind));
        convs.sort_by_key(|c| std::cmp::Reverse(c.last_activity_ms));
        let lines: Vec<String> = convs
            .iter()
            .filter(|c| !p.unread_only || super::text::is_unread(c, &self.self_mri))
            .take(n)
            .map(|c| chat_line(c, &self.self_mri))
            .collect();
        if lines.is_empty() {
            return Ok(if p.unread_only {
                "No unread chats.".into()
            } else {
                "No chats.".into()
            });
        }
        Ok(lines.join("\n"))
    }

    #[tool(
        description = "Read the messages of one chat, oldest first. Each line: [time UTC] sender (message id, sender id): text. Use before_ms to page back.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn read_chat(&self, Parameters(p): Parameters<ReadChat>) -> Result<String, String> {
        check_chat(&p.chat_id)?;
        let n = limit(p.limit, 30, 200);
        let page = self
            .client
            .messages(&p.chat_id, p.before_ms, n)
            .await
            .map_err(err)?;
        let mut lines: Vec<String> = page
            .messages
            .iter()
            .filter(|m| m.is_chat_content())
            .map(|m| message_line(m, &self.self_mri))
            .collect();
        if lines.is_empty() {
            lines.push("No messages.".into());
        }
        if page.more {
            let oldest = page.messages.first().map(|m| m.compose_ms).unwrap_or(0);
            lines.push(format!("(older messages exist: before_ms={oldest})"));
        }
        Ok(lines.join("\n"))
    }

    #[tool(
        description = "What you missed: unread chats with their unread messages, and the chat messages that mention you. Use it for 'what did I miss' or a morning summary.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn catch_up(&self, Parameters(p): Parameters<CatchUp>) -> Result<String, String> {
        let since = now_ms() - i64::from(p.since_hours.unwrap_or(24).clamp(1, 336)) * HOUR_MS;
        let mut convs = self.client.conversations(since, 100).await.map_err(err)?;
        convs.retain(|c| is_chat(c.kind) && super::text::is_unread(c, &self.self_mri));
        convs.sort_by_key(|c| std::cmp::Reverse(c.last_activity_ms));
        let mut out = vec![format!("Unread chats: {}", convs.len())];
        for c in convs.iter().take(10) {
            out.push(String::new());
            out.push(chat_line(c, &self.self_mri));
            let page = match self.client.messages(&c.id, None, 30).await {
                Ok(p) => p,
                Err(e) => {
                    out.push(format!("  (could not read: {e})"));
                    continue;
                }
            };
            let unread: Vec<String> = page
                .messages
                .iter()
                .filter(|m| {
                    m.is_chat_content()
                        && m.sender_mri != self.self_mri
                        && m.compose_ms > c.read_horizon_ms
                })
                .map(|m| format!("  {}", message_line(m, &self.self_mri)))
                .collect();
            out.extend(unread);
        }
        if convs.len() > 10 {
            out.push(format!(
                "\n({} more unread chats: list_chats with unread_only)",
                convs.len() - 10
            ));
        }
        let mentions = self.client.mentions(30).await.map_err(err)?;
        let mentions: Vec<String> = mentions
            .iter()
            .filter(|m| m.timestamp_ms >= since)
            .filter(|m| is_chat(conversation_kind(&m.source_thread_id, None)))
            .map(|m| {
                format!(
                    "[{}] {} in {} ({}, chat {}, message {}): {}",
                    iso_minute(m.timestamp_ms),
                    m.actor_name,
                    if m.thread_topic.is_empty() {
                        "a chat"
                    } else {
                        m.thread_topic.as_str()
                    },
                    if m.is_read { "read" } else { "unread" },
                    m.source_thread_id,
                    m.source_message_id,
                    clip(&plain(&m.preview).replace('\n', " "), 300)
                )
            })
            .collect();
        out.push(String::new());
        out.push(format!("Mentions in chats: {}", mentions.len()));
        out.extend(mentions);
        Ok(out.join("\n"))
    }

    #[tool(
        description = "Search your chat messages (channels are left out). One line per hit: [time UTC] sender (chat id, message id): matching text.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn search_messages(
        &self,
        Parameters(p): Parameters<SearchMessages>,
    ) -> Result<String, String> {
        if p.query.trim().is_empty() {
            return Err("empty query".into());
        }
        if let Some(c) = &p.chat_id {
            check_chat(c)?;
        }
        let filters = SearchFilters {
            in_thread: p.chat_id.clone(),
            from_mri: p.from_person.clone(),
            since_day: None,
        };
        let n = limit(p.limit, 15, 50);
        let page = self
            .client
            .search_messages(&self.self_mri, &p.query, &filters, 0, n)
            .await
            .map_err(err)?;
        let hits: Vec<String> = page
            .hits
            .iter()
            .filter(|h| {
                h.root_id.is_empty()
                    && h.thread_type != "space"
                    && h.thread_type != "topic"
                    && check_chat(&h.conversation_id).is_ok()
            })
            .map(|h| {
                format!(
                    "[{}] {} in {} (chat {}, message {}): {}",
                    iso_minute(h.time_ms),
                    h.sender_name,
                    if h.topic.is_empty() {
                        "a chat"
                    } else {
                        h.topic.as_str()
                    },
                    h.conversation_id,
                    h.message_id,
                    clip(&h.snippet.replace('\n', " "), 400)
                )
            })
            .collect();
        if hits.is_empty() {
            return Ok("No matching chat messages.".into());
        }
        Ok(format!(
            "{} hits in chats (of {} in all of Teams)\n{}",
            hits.len(),
            page.total,
            hits.join("\n")
        ))
    }

    #[tool(
        description = "Find people in your organization by name or email, with their presence. One line per person: name | email | job title | person id | availability, activity, status message.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn find_people(&self, Parameters(p): Parameters<FindPeople>) -> Result<String, String> {
        if p.query.trim().is_empty() {
            return Err("empty query".into());
        }
        let mut found = self
            .client
            .search_people(&self.self_mri, &p.query)
            .await
            .map_err(err)?;
        found.truncate(10);
        if found.is_empty() {
            return Ok("Nobody found.".into());
        }
        let mris: Vec<String> = found.iter().map(|f| f.mri.clone()).collect();
        let presence = self.client.get_presence(&mris).await.unwrap_or_default();
        Ok(found
            .iter()
            .map(|f| {
                let pr = presence
                    .iter()
                    .find(|p| p.mri == f.mri)
                    .map(presence_text)
                    .unwrap_or_else(|| "presence unknown".into());
                format!(
                    "{} | {} | {} | {} | {}",
                    f.display_name,
                    f.email.as_deref().unwrap_or("-"),
                    f.job_title.as_deref().unwrap_or("-"),
                    f.mri,
                    pr
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    #[tool(
        description = "Presence of people by person id: availability, activity, out of office and status message.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn get_presence(&self, Parameters(p): Parameters<GetPresence>) -> Result<String, String> {
        if p.people.is_empty() || p.people.len() > 50 {
            return Err("give 1 to 50 person ids".into());
        }
        let found = self.client.get_presence(&p.people).await.map_err(err)?;
        Ok(found
            .iter()
            .map(|p| format!("{} | {}", p.mri, presence_text(p)))
            .collect::<Vec<_>>()
            .join("\n"))
    }

    #[tool(
        description = "Your calendar: events from now (or from_day days from now) for the given number of days. One line per event: start-end (UTC) | subject | organizer | your response | location | event id.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn calendar_agenda(
        &self,
        Parameters(p): Parameters<CalendarAgenda>,
    ) -> Result<String, String> {
        let start = now_ms() + i64::from(p.from_day.unwrap_or(0).clamp(-30, 60)) * DAY_MS;
        let end = start + i64::from(p.days.unwrap_or(1).clamp(1, 14)) * DAY_MS;
        let mut events = calendar::Calendar::view(&self.client, start, end)
            .await
            .map_err(err)?;
        events.retain(|e| !e.is_cancelled);
        events.sort_by_key(|e| e.start_ms);
        if events.is_empty() {
            return Ok("No events.".into());
        }
        Ok(events
            .iter()
            .map(|e| {
                format!(
                    "{}-{} | {} | {} | {:?} | {} | {}",
                    iso_minute(e.start_ms),
                    iso_minute(e.end_ms),
                    if e.sensitivity_private {
                        "(private)"
                    } else {
                        e.subject.as_str()
                    },
                    e.organizer_name,
                    e.response,
                    if e.is_online_meeting && e.location.is_empty() {
                        "Teams meeting"
                    } else {
                        e.location.as_str()
                    },
                    e.id
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    #[tool(
        description = "The details of one calendar event: time, organizer, attendees and their responses, and the invitation text.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn meeting_detail(
        &self,
        Parameters(p): Parameters<MeetingDetail>,
    ) -> Result<String, String> {
        let d = calendar::event_detail(&self.client, &p.event_id)
            .await
            .map_err(err)?;
        let e = &d.event;
        let mut out = vec![
            format!("Subject: {}", e.subject),
            format!(
                "When: {} - {} (UTC)",
                iso_minute(e.start_ms),
                iso_minute(e.end_ms)
            ),
            format!("Organizer: {} <{}>", e.organizer_name, e.organizer_email),
            format!("Your response: {:?}", e.response),
            format!("Location: {}", e.location),
            format!("Online meeting: {}", e.is_online_meeting),
            format!(
                "Responses: {} accepted, {} tentative, {} declined, {} not yet",
                d.counts.accepted, d.counts.tentative, d.counts.declined, d.counts.not_responded
            ),
        ];
        if !d.hide_attendees {
            for a in &e.attendees {
                out.push(format!(
                    "  attendee: {} <{}> {:?}",
                    a.name, a.email, a.response
                ));
            }
        }
        let body = if d.body_is_html {
            plain(&d.body)
        } else {
            d.body.trim().to_string()
        };
        out.push(format!("Invitation text:\n{}", clip(&body, MAX_BODY)));
        Ok(out.join("\n"))
    }

    #[tool(
        description = "Suggest meeting times when you and the given attendees are free, within working hours.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn find_free_time(
        &self,
        Parameters(p): Parameters<FindFreeTime>,
    ) -> Result<String, String> {
        let start = now_ms();
        let q = calendar::FindTimes {
            required: p
                .attendees
                .iter()
                .map(|e| calendar::Person::new("", e))
                .collect(),
            optional: vec![],
            duration_minutes: p.duration_minutes.unwrap_or(30).clamp(5, 480),
            window_start_ms: start,
            window_end_ms: start + i64::from(p.within_days.unwrap_or(5).clamp(1, 14)) * DAY_MS,
            max_candidates: 10,
            min_attendee_percentage: 100,
            work_hours_only: true,
        };
        let found = calendar::find_meeting_times(&self.client, &q)
            .await
            .map_err(err)?;
        if found.suggestions.is_empty() {
            return Ok(format!(
                "No free time found ({}).",
                found.empty_reason.as_deref().unwrap_or("no reason given")
            ));
        }
        Ok(found
            .suggestions
            .iter()
            .map(|s| {
                format!(
                    "{}-{} (UTC) confidence {:.0}%",
                    iso_minute(s.start_ms),
                    iso_minute(s.end_ms),
                    s.confidence
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

fn presence_text(p: &teams_proto::presence::Presence) -> String {
    let mut s = format!("{}, {}", p.availability, p.activity);
    if p.ooo {
        s.push_str(", out of office");
    }
    if let Some(n) = p.note.as_ref().filter(|n| !n.text.is_empty()) {
        s.push_str(&format!(", status: {}", clip(&plain(&n.text), 200)));
    }
    s
}

/// The kind name of a chat id, for previews.
pub(super) fn chat_kind_name(id: &str) -> &'static str {
    kind_name(conversation_kind(id, None))
}
