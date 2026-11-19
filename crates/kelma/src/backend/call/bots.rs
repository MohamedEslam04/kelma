//! Live captions, recording and transcription of a live call (More › Record and transcribe,
//! More › Language and speech): every one goes through Microsoft's recorder bot, which this
//! invites once and then commands (`calling::signaling::recorder`). The meeting's recording and
//! transcription state come back for everyone in conversation updates. A refusal by the service
//! is shown as the call's error banner, with the service's own words when it gives some.
//!
//! Caption text is not wired yet: it arrives on the media's data channel, which the media
//! session does not open (docs/protocol-calls.md, "Caption text").

use super::{Control, Event, EventSink};
use calling::Links;
use calling::signaling::recorder::{
    BotStep, CommandContext, RecorderBot, RecorderCommand, RecorderFlow, RecordingState,
    chat_thread_id, command_body, new_correlation_id, recorder_bot, recording_file_name, refusal,
};
use calling::signaling::{CallSession, NgcClient};
use serde_json::Value;

/// The recorder bot's side of one call.
pub(super) struct Bots {
    flow: RecorderFlow,
    bot: Option<RecorderBot>,
    state: RecordingState,
    /// The chat the bot is invited into.
    thread: Option<String>,
    /// A call that is no meeting (1:1) invites the bot with `addParticipant`.
    threadless: bool,
}

impl Bots {
    pub(super) fn new(thread: Option<String>, meeting: bool) -> Self {
        Bots {
            flow: RecorderFlow::default(),
            bot: None,
            state: RecordingState::default(),
            thread,
            threadless: !meeting,
        }
    }

    /// Queues a More command. False when `c` is none.
    pub(super) fn ask(&mut self, c: &Control) -> bool {
        let cmd = match *c {
            Control::Captions(on) => RecorderCommand::Captions(on),
            Control::Recording(on) => RecorderCommand::Recording(on),
            Control::Transcription(on) => RecorderCommand::Transcription(on),
            _ => return false,
        };
        self.flow.request(cmd);
        true
    }

    /// A roster delta or snapshot: where the bot is.
    pub(super) fn roster(&mut self, body: &Value) {
        match recorder_bot(body) {
            None => {}
            Some(None) => {
                self.bot = None;
                self.flow.bot_left();
            }
            Some(Some(b)) => self.bot = Some(b),
        }
    }

    /// A conversation update: the meeting chat, and the recording and transcription state.
    pub(super) fn conversation(&mut self, body: &Value) -> Vec<Event> {
        if self.thread.is_none() {
            self.thread = chat_thread_id(body);
        }
        let change = self.state.apply(body);
        let mut out = Vec::new();
        out.extend(change.recording.map(Event::Recording));
        out.extend(change.transcription.map(Event::Transcribing));
        out
    }

    /// The bot's invitation was refused (`addParticipantFailure`).
    pub(super) fn bot_refused(&mut self, body: &Value) -> Vec<Event> {
        let why = refusal(body)
            .map(|r| r.text())
            .unwrap_or_else(|| "the service didn't let the recorder join".into());
        self.drop_all(&why)
    }

    fn drop_all(&mut self, why: &str) -> Vec<Event> {
        self.flow
            .bot_failed()
            .into_iter()
            .flat_map(|cmd| refused(cmd, why))
            .collect()
    }

    /// Sends whatever can go now: the bot's invitation, or commands once it has a command URL.
    pub(super) async fn step(
        &mut self,
        ngc: &NgcClient,
        s: &CallSession,
        links: &Links,
        controller: &str,
        sink: &EventSink,
    ) {
        loop {
            match self.flow.next(self.bot.as_ref()) {
                BotStep::Wait => return,
                BotStep::AddBot => {
                    let added = match self.thread.clone() {
                        Some(t) => ngc
                            .add_recorder_bot(s, links, controller, &t, self.threadless)
                            .await
                            .map_err(|e| reason(&e)),
                        None => Err("this call has no chat for the recorder".to_owned()),
                    };
                    if let Err(why) = added {
                        tracing::warn!(%why, "recorder bot not invited");
                        for ev in self.drop_all(&why) {
                            sink.send(ev);
                        }
                        return;
                    }
                }
                BotStep::Send(url, cmd) => {
                    let now = chrono::Utc::now();
                    let timestamp = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                    let file_name =
                        recording_file_name("", &now.format("%Y%m%d_%H%M%S").to_string());
                    let correlation_id = new_correlation_id();
                    let ctx = CommandContext {
                        timestamp: &timestamp,
                        meeting_title: "",
                        file_name: &file_name,
                        correlation_id: &correlation_id,
                    };
                    let body = command_body(cmd, s, &ctx);
                    if let Err(e) = ngc.recorder_command(s, &url, body).await {
                        tracing::warn!(%e, ?cmd, "recorder command refused");
                        for ev in refused(cmd, &reason(&e)) {
                            sink.send(ev);
                        }
                    } else if let RecorderCommand::Captions(on) = cmd
                        && let Err(e) = ngc.set_caption_preference(s, links, controller, on).await
                    {
                        tracing::warn!(%e, "caption preference not published");
                    }
                }
            }
        }
    }
}

/// What the call window hears when `cmd` was refused for `why`.
fn refused(cmd: RecorderCommand, why: &str) -> Vec<Event> {
    let what = match cmd {
        RecorderCommand::Captions(true) => "turn on live captions",
        RecorderCommand::Captions(false) => "turn off live captions",
        RecorderCommand::Recording(true) => "start recording",
        RecorderCommand::Recording(false) => "stop recording",
        RecorderCommand::Transcription(true) => "start transcription",
        RecorderCommand::Transcription(false) => "stop transcription",
    };
    let mut out = Vec::new();
    if cmd == RecorderCommand::Captions(true) {
        out.push(Event::Captions(false));
    }
    out.push(Event::Error(format!("Couldn't {what}: {why}")));
    out
}

/// The service's reason in an error answer, else its status.
fn reason(e: &calling::Error) -> String {
    match e {
        calling::Error::Status { status, body, .. } => serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|v| refusal(&v))
            .map(|r| r.text())
            .unwrap_or_else(|| format!("the service answered HTTP {status}")),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use calling::signaling::recorder::RECORDER_BOT_MRI;
    use serde_json::json;

    #[test]
    fn a_refused_start_names_what_failed_and_turns_captions_back_off() {
        assert_eq!(
            refused(RecorderCommand::Captions(true), "no"),
            [
                Event::Captions(false),
                Event::Error("Couldn't turn on live captions: no".into())
            ]
        );
        assert_eq!(
            refused(RecorderCommand::Recording(true), "policy"),
            [Event::Error("Couldn't start recording: policy".into())]
        );
    }

    #[test]
    fn the_reason_is_the_services_own_words_when_it_gives_some() {
        let status = |body: &str| calling::Error::Status {
            op: "recorder_command",
            status: 403,
            body: body.into(),
        };
        assert_eq!(
            reason(&status(
                r#"{"code": 403, "subCode": 1, "phrase": "Disabled by policy"}"#
            )),
            "Disabled by policy (403/1)"
        );
        assert_eq!(reason(&status("")), "the service answered HTTP 403");
    }

    #[test]
    fn a_refused_invitation_reports_every_command_that_waited() {
        let mut b = Bots::new(Some("19:meeting_x@thread.v2".into()), true);
        assert!(b.ask(&Control::Recording(true)));
        assert!(b.ask(&Control::Captions(true)));
        assert!(!b.ask(&Control::Leave));
        assert_eq!(b.flow.next(None), BotStep::AddBot);
        let events = b.bot_refused(&json!({"code": 403, "phrase": "Not allowed"}));
        assert_eq!(
            events,
            [
                Event::Error("Couldn't start recording: Not allowed (403/0)".into()),
                Event::Captions(false),
                Event::Error("Couldn't turn on live captions: Not allowed (403/0)".into()),
            ]
        );
    }

    #[test]
    fn the_roster_and_conversation_updates_feed_the_bot_and_the_state() {
        let mut b = Bots::new(None, false);
        let joined = json!({"participants": {RECORDER_BOT_MRI: {"version": 1, "state": "active",
            "endpoints": {"e": {"endpointMetadata": {"commandUrl": "https://x.teams.microsoft.com/c"}}}}}});
        b.roster(&joined);
        assert!(b.bot.is_some());
        let events = b.conversation(&json!({
            "activeModalities": {"groupChat": {"threadId": "19:a@thread.v2"}},
            "meetingStates": {"cloudRecording": {"value": {"State": "Active"}, "sequenceNumber": 1}}
        }));
        assert_eq!(events, [Event::Recording(true)]);
        assert_eq!(b.thread.as_deref(), Some("19:a@thread.v2"));
        b.roster(&json!({"participants": {RECORDER_BOT_MRI: {"version": 2, "state": "inactive"}}}));
        assert!(b.bot.is_none());
    }
}
