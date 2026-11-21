//! Live captions, recording and transcription: all three run through Microsoft's recorder bot.
//!
//! - **The bot.** We invite `28:bdd75849-…` into the conversation (`addParticipantAndModality`,
//!   or `addParticipant` for a call without a chat thread) with `botData` that carries a token of
//!   ours for the recorder service (`initiatorUserToken`). A refusal comes back as an
//!   `addParticipantFailure` callback.
//! - **Its command URL.** Once in, the bot's roster entry names `endpointMetadata.commandUrl`
//!   and, under `processingModes`, what it runs (`closedCaptions`, `recording`).
//! - **Commands.** `POST <commandUrl>` with the recorder token as bearer and our skypetoken
//!   (`X-Skypetoken`, and `participantSkypeToken` in the body): start/stop captions
//!   (`processingModes: [closedCaptions]`), recording (`[recording, realTimeTranscript]`) or
//!   transcription (`[realTimeTranscript]`).
//! - **State.** Everyone sees recording and transcription in the conversation's
//!   `meetingStates.cloudRecording` / `meetingStates.transcription` (`value.State: Active`, with
//!   a rising `sequenceNumber`).
//! - **Caption text** arrives on the media's data channel, not here (see [`crate::captions`]).
//!
//! Facts: rusty-teams `crates/calling/src/captions.rs`, `recording.rs`, `roster.rs`
//! `caption_bot` and `signaling.rs` `add_caption_bot`/`bot_command` (MIT); eisbaw/ost
//! `src/calling/recording.rs` (MIT); mautrix-teams `teams-webclient.yaml`
//! `callAddBotParticipant` and the `aadRecorder` scheme (AGPL, facts only). None of it is
//! verified live by IronTeams yet; see `docs/protocol-calls.md`.

use super::controls::control_url;
use super::{CallSession, NgcClient};
use crate::error::{Error, Result};
use crate::types::Links;
use reqwest::Method;
use serde_json::{Value, json};
use teams_proto::broker::Audience;

/// Microsoft's recorder bot (captions, recording and transcription).
pub const RECORDER_BOT_MRI: &str = "28:bdd75849-e0a6-4cce-8fc1-d7c0d4da43e5";
/// The recorder service's resource: its tokens go to the bot's command URL and into the
/// invitation.
pub const RECORDER_RESOURCE: &str = "4580fd1d-e5a3-4f56-9ad1-aab0e3bf8f76";
const BOT_PREFIX: &str = "28:";
const ACTIVE: &str = "Active";
const RECORDING_SUFFIX: &str = "Meeting Recording";

/// What we ask the recorder bot: `true` starts, `false` stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecorderCommand {
    Captions(bool),
    Recording(bool),
    Transcription(bool),
}

impl RecorderCommand {
    pub fn starts(self) -> bool {
        match self {
            RecorderCommand::Captions(on)
            | RecorderCommand::Recording(on)
            | RecorderCommand::Transcription(on) => on,
        }
    }
}

/// The feature flags the web client sends to the recorder (invitation and recording start).
fn recorder_features() -> Value {
    json!({
        "enablePPTSharing": true,
        "intermediateLiveCaptions": false,
        "actionItemsEnabled": false,
        "enableEmailAndMeetingLanguageModel": true,
        "ceoSummit": false,
        "useUnmixedAudio": true,
        "enableTranscriptMeetingChaptering": false,
    })
}

/// Where the bot is invited: the conversation's link, else under the controller. A call with no
/// chat thread (`threadless`) uses `addParticipant`.
pub fn add_bot_url(links: &Links, controller: &str, threadless: bool) -> String {
    let name = if threadless {
        "addParticipant"
    } else {
        "addParticipantAndModality"
    };
    control_url(links, controller, name)
}

/// The bot's invitation. `initiator_token` is our token for [`RECORDER_RESOURCE`].
pub fn add_bot_body(
    s: &CallSession,
    thread_id: &str,
    threadless: bool,
    initiator_token: &str,
) -> Value {
    let group_chat = if threadless {
        Value::Null
    } else {
        json!({"threadId": thread_id, "messageId": null})
    };
    json!({
        "disableUnmute": false,
        "participants": {
            "from": s.me.wire(),
            "to": [{"id": RECORDER_BOT_MRI, "participantId": uuid::Uuid::new_v4().to_string()}],
        },
        "participantInvitationData": {"botData": {
            "meetingTitle": "",
            "clientInfo": "Teams-R4",
            "callId": s.chain_id,
            "threadId": thread_id,
            "recorderFeatures": recorder_features(),
            "mode": "RecordingAndTranscription",
            "iCalUid": null,
            "consumerType": "Teams",
            "spokenLanguage": "",
            "initiatorUserToken": initiator_token,
            "exchangeId": null,
            "meetingOrganizer": s.me.display_name,
        }},
        "replacementDetails": null,
        "groupContext": null,
        "groupChat": group_chat,
        "links": {
            "addParticipantSuccess": s.callbacks.conversation("addParticipantSuccess"),
            "addParticipantFailure": s.callbacks.conversation("addParticipantFailure"),
        },
    })
}

/// What a command needs besides who we are. `file_name` and `correlation_id` matter only to a
/// recording start.
#[derive(Debug, Clone, Copy)]
pub struct CommandContext<'a> {
    /// RFC 3339 UTC with milliseconds (`2026-11-19T08:44:12.705Z`).
    pub timestamp: &'a str,
    pub meeting_title: &'a str,
    /// See [`recording_file_name`].
    pub file_name: &'a str,
    pub correlation_id: &'a str,
}

/// The body of a command to the bot's command URL. The skypetoken is added when it is sent.
pub fn command_body(cmd: RecorderCommand, s: &CallSession, ctx: &CommandContext<'_>) -> Value {
    let mut body = json!({
        "timestamp": ctx.timestamp,
        "participantMri": s.me.mri,
        "participantLegId": s.me.participant_id,
        "action": if cmd.starts() { "start" } else { "stop" },
    });
    let (mode, modes): (Option<&str>, &[&str]) = match cmd {
        RecorderCommand::Captions(_) => (Some("transcription"), &["closedCaptions"]),
        RecorderCommand::Recording(true) => (None, &["recording", "realTimeTranscript"]),
        RecorderCommand::Recording(false) => (Some("recording"), &["recording"]),
        RecorderCommand::Transcription(_) => (Some("transcription"), &["realTimeTranscript"]),
    };
    if let Some(m) = mode {
        body["mode"] = json!(m);
    }
    body["processingModes"] = json!(modes);
    if let RecorderCommand::Recording(true) | RecorderCommand::Transcription(true) = cmd {
        let mut p = json!({
            "recordingFeatures": recorder_features(),
            "recordingStorageSettings": [{
                "StorageType": "OnedriveForBusiness",
                "StorageLocation": "Recordings",
                "FileName": ctx.file_name,
            }],
            "correlationId": ctx.correlation_id,
            "meetingTitle": ctx.meeting_title,
            "exchangeId": null,
            "meetingOrganizer": s.me.display_name,
            "spokenLanguage": "",
            "type": "start",
        });
        if cmd == RecorderCommand::Recording(true) {
            p["recordingMode"] = json!("Normal");
        }
        body["actionParameters"] = p;
    }
    body
}

/// A fresh `correlationId` for a recording start.
pub fn new_correlation_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The recording's file name: the title (unsafe characters as `_`, `Meeting` when empty), the
/// UTC time as `YYYYMMDD_HHMMSS`, and `Meeting Recording`.
pub fn recording_file_name(meeting_title: &str, stamp: &str) -> String {
    let title = match meeting_title.trim() {
        "" => "Meeting",
        t => t,
    };
    let safe: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}-{stamp}-{RECORDING_SUFFIX}")
}

/// `PUT updateEndpointMetadata`: whether we want captions (rusty-teams sends it with every
/// captions start and stop).
pub fn endpoint_metadata_body(s: &CallSession, captions: bool) -> Value {
    json!({
        "participants": {"from": s.me.wire()},
        "endpointMetadata": {
            "holographicCapabilities": 3,
            "transcriptionPrefs": {"closedCaptions": captions},
        },
    })
}

/// The recorder bot as the roster shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecorderBot {
    /// Where its commands go; absent until it has joined.
    pub command_url: Option<String>,
    /// `processingModes.closedCaptions.state` is `Active`.
    pub captions: bool,
    /// `processingModes.recording.state` is `Active`.
    pub recording: bool,
}

/// What a roster delta or snapshot says about the recorder bot: `None` when nothing,
/// `Some(None)` when it left, else the bot. A bot is a `28:` participant whose endpoint metadata
/// has a `commandUrl` or `processingModes`.
pub fn recorder_bot(roster: &Value) -> Option<Option<RecorderBot>> {
    let (_, p) = roster["participants"]
        .as_object()?
        .iter()
        .filter(|(mri, _)| mri.starts_with(BOT_PREFIX))
        .find(|(mri, p)| *mri == RECORDER_BOT_MRI || bot_metadata(p).next().is_some())?;
    if p["state"].as_str() != Some("active") {
        return Some(None);
    }
    let meta: Vec<&Value> = bot_metadata(p).collect();
    let mode = |name: &str| {
        meta.iter()
            .any(|m| m["processingModes"][name]["state"].as_str() == Some(ACTIVE))
    };
    Some(Some(RecorderBot {
        command_url: meta
            .iter()
            .find_map(|m| m["commandUrl"].as_str())
            .map(str::to_owned),
        captions: mode("closedCaptions"),
        recording: mode("recording"),
    }))
}

fn bot_metadata(p: &Value) -> impl Iterator<Item = &Value> {
    p["endpoints"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(_, e)| &e["endpointMetadata"])
        .filter(|m| m.get("commandUrl").is_some() || m.get("processingModes").is_some())
}

/// The first value under `key` anywhere in `v`.
fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => m
            .get(key)
            .or_else(|| m.values().find_map(|c| find_key(c, key))),
        Value::Array(a) => a.iter().find_map(|c| find_key(c, key)),
        _ => None,
    }
}

/// The meeting chat's thread, from a conversation update (`activeModalities.groupChat`).
pub fn chat_thread_id(body: &Value) -> Option<String> {
    find_key(body, "activeModalities")?
        .pointer("/groupChat/threadId")?
        .as_str()
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// What one update changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StateChange {
    pub recording: Option<bool>,
    pub transcription: Option<bool>,
}

impl StateChange {
    pub fn is_empty(&self) -> bool {
        self.recording.is_none() && self.transcription.is_none()
    }
}

/// The meeting's recording and transcription, from `meetingStates` in conversation updates.
#[derive(Debug, Clone, Copy, Default)]
pub struct RecordingState {
    recording: bool,
    transcription: bool,
    recording_seq: u64,
    transcription_seq: u64,
}

impl RecordingState {
    pub fn recording(&self) -> bool {
        self.recording
    }

    pub fn transcription(&self) -> bool {
        self.transcription
    }

    /// Applies an update; an older `sequenceNumber` than the last one is ignored.
    pub fn apply(&mut self, body: &Value) -> StateChange {
        let Some(states) = find_key(body, "meetingStates") else {
            return StateChange::default();
        };
        StateChange {
            recording: newer(
                &states["cloudRecording"],
                &mut self.recording_seq,
                &mut self.recording,
            ),
            transcription: newer(
                &states["transcription"],
                &mut self.transcription_seq,
                &mut self.transcription,
            ),
        }
    }
}

/// The state's new value when it is not older than `seq` and differs from `now`.
fn newer(state: &Value, seq: &mut u64, now: &mut bool) -> Option<bool> {
    let on = state["value"]["State"].as_str()? == ACTIVE;
    let n = state["sequenceNumber"].as_u64().unwrap_or_default();
    if n < *seq {
        return None;
    }
    *seq = n;
    (on != *now).then(|| {
        *now = on;
        on
    })
}

/// Why the service refused: the first `{code, subCode, phrase}` in a failure callback, else an
/// error's `message`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: i64,
    pub sub_code: i64,
    pub phrase: String,
}

impl Refusal {
    /// The phrase and the codes, for a banner.
    pub fn text(&self) -> String {
        match (self.phrase.is_empty(), self.code) {
            (false, 0) => self.phrase.clone(),
            (false, _) => format!("{} ({}/{})", self.phrase, self.code, self.sub_code),
            (true, _) => format!("code {}/{}", self.code, self.sub_code),
        }
    }
}

pub fn refusal(body: &Value) -> Option<Refusal> {
    if let Some(r) = code_holder(body) {
        return Some(r);
    }
    let message = find_key(body, "message")?.as_str()?;
    Some(Refusal {
        code: 0,
        sub_code: 0,
        phrase: message.to_owned(),
    })
}

fn code_holder(v: &Value) -> Option<Refusal> {
    match v {
        Value::Object(m) => {
            if let Some(code) = m.get("code").and_then(Value::as_i64) {
                let phrase = ["phrase", "message", "reason"]
                    .iter()
                    .find_map(|k| m.get(*k).and_then(Value::as_str))
                    .unwrap_or_default();
                return Some(Refusal {
                    code,
                    sub_code: m.get("subCode").and_then(Value::as_i64).unwrap_or_default(),
                    phrase: phrase.to_owned(),
                });
            }
            m.values().find_map(code_holder)
        }
        Value::Array(a) => a.iter().find_map(code_holder),
        _ => None,
    }
}

/// What to do next for the commands asked of the bot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BotStep {
    Wait,
    /// Invite the bot (once until it fails).
    AddBot,
    /// Send this command to this command URL.
    Send(String, RecorderCommand),
}

/// The commands waiting for the bot, in the order asked: the bot is invited once, and each
/// command goes out once the roster names its command URL.
#[derive(Debug, Clone, Default)]
pub struct RecorderFlow {
    pending: Vec<RecorderCommand>,
    bot_requested: bool,
}

impl RecorderFlow {
    pub fn request(&mut self, cmd: RecorderCommand) {
        self.pending.push(cmd);
    }

    pub fn next(&mut self, bot: Option<&RecorderBot>) -> BotStep {
        if let Some(url) = bot.and_then(|b| b.command_url.clone())
            && !self.pending.is_empty()
        {
            return BotStep::Send(url, self.pending.remove(0));
        }
        if bot.is_none() {
            // A stop with no bot in the meeting has nothing to stop.
            self.pending.retain(|c| c.starts());
        }
        if bot.is_some() || self.pending.is_empty() || self.bot_requested {
            return BotStep::Wait;
        }
        self.bot_requested = true;
        BotStep::AddBot
    }

    /// The bot left the meeting: the next start invites it again.
    pub fn bot_left(&mut self) {
        self.bot_requested = false;
    }

    /// The bot's invitation was refused: what waited for it, which is dropped.
    pub fn bot_failed(&mut self) -> Vec<RecorderCommand> {
        self.bot_requested = false;
        std::mem::take(&mut self.pending)
    }
}

impl NgcClient {
    /// A token for the recorder service (minted with the Teams client id, as the web client
    /// does). UNVERIFIED live for IronTeams's sign-in.
    pub async fn recorder_token(&self) -> Result<String> {
        Ok(self
            .client
            .token_for(&Audience::App(RECORDER_RESOURCE.to_owned()))
            .await?)
    }

    /// Invites the recorder bot (see [`add_bot_body`]). The bot shows up in the roster, or an
    /// `addParticipantFailure` callback says why not.
    pub async fn add_recorder_bot(
        &self,
        s: &CallSession,
        links: &Links,
        controller: &str,
        thread_id: &str,
        threadless: bool,
    ) -> Result<()> {
        let token = self.recorder_token().await?;
        let url = add_bot_url(links, controller, threadless);
        let body = add_bot_body(s, thread_id, threadless, &token);
        self.post("add_recorder_bot", s, &url, &body)
            .await
            .map(drop)
    }

    /// Our caption preference (`PUT updateEndpointMetadata`).
    pub async fn set_caption_preference(
        &self,
        s: &CallSession,
        links: &Links,
        controller: &str,
        captions: bool,
    ) -> Result<()> {
        let url = control_url(links, controller, "updateEndpointMetadata");
        let body = endpoint_metadata_body(s, captions);
        self.send(
            "update_endpoint_metadata",
            s,
            Method::PUT,
            &url,
            Some(&body),
        )
        .await
        .map(drop)
    }

    /// Sends a command (see [`command_body`]) to the bot's command URL, with the recorder token
    /// and our skypetoken: the only request of a call that does not carry the IC3 bearer.
    pub async fn recorder_command(
        &self,
        s: &CallSession,
        command_url: &str,
        mut body: Value,
    ) -> Result<()> {
        let target = super::endpoints::routed_url(command_url)?;
        let token = self.recorder_token().await?;
        let skype = self
            .client
            .tokens()
            .await
            .skype
            .ok_or_else(|| Error::Protocol("no skypetoken for the recorder".into()))?;
        body["participantSkypeToken"] = json!(skype);
        let resp = self
            .client
            .http()
            .post(&target)
            .header("Authorization", format!("Bearer {token}"))
            .header("X-Skypetoken", &skype)
            .header("Content-Type", "application/json")
            .header("User-Agent", teams_proto::client_info::USER_AGENT)
            .header("x-microsoft-skype-chain-id", &s.chain_id)
            .header("requestid", uuid::Uuid::new_v4().to_string())
            .body(serde_json::to_vec(&body)?)
            .send()
            .await
            .map_err(teams_proto::Error::from)?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(teams_proto::Error::from)?;
        // The body holds our skypetoken: the observer sees no request body.
        self.observe(
            "recorder_command",
            &Method::POST,
            &target,
            None,
            status,
            &text,
        );
        if !(200..300).contains(&status) {
            return Err(super::status_error("recorder_command", status, &text));
        }
        Ok(())
    }
}
