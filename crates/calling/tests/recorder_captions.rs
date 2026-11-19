//! Live captions and recording/transcription: the recorder bot's invitation, its commands, the
//! roster entry that names its command URL, the meeting's recording state, refusals, and the
//! caption text framing (offline, synthetic fixtures).

use calling::Links;
use calling::captions::{CaptionLine, Captions, Depacketizer, parse_caption_message};
use calling::signaling::recorder::{
    BotStep, CommandContext, RECORDER_BOT_MRI, RecorderCommand, RecorderFlow, RecordingState,
    StateChange, add_bot_body, add_bot_url, chat_thread_id, command_body, endpoint_metadata_body,
    recorder_bot, recording_file_name, refusal,
};
use calling::signaling::{CallSession, CallbackLinks, LocalEndpoint};
use serde_json::{Value, json};

const CONTROLLER: &str = "https://api.flightproxy.skype.com/api/v2/cp/conv-example.conv.skype.com/conv/AAAAAAAAAAAAAAAAAAAAAA?i=1";
const ME: &str = "8:orgid:00000000-0000-0000-0000-0000000000b2";
const THREAD: &str = "19:meeting_example@thread.v2";
const COMMAND_URL: &str =
    "https://api.flightproxy.teams.microsoft.com/api/v2/ep/recorder.example.invalid/v2/oncommand/1";

fn session() -> CallSession {
    CallSession {
        me: LocalEndpoint {
            mri: ME.into(),
            display_name: "Self Example".into(),
            endpoint_id: "sess-1".into(),
            participant_id: "part-1".into(),
            language_id: "en-US".into(),
        },
        callbacks: CallbackLinks::new("https://trouter.example.invalid/v4/f/AbCd/", "sess-1"),
        chain_id: "chain-1".into(),
        media_leg_id: "LEG1".into(),
    }
}

fn ctx() -> CommandContext<'static> {
    CommandContext {
        timestamp: "2026-11-19T08:44:12.705Z",
        meeting_title: "Standup",
        file_name: "Standup-20261119_084412-Meeting Recording",
        correlation_id: "corr-1",
    }
}

// --- the bot invitation ---

#[test]
fn the_bot_invitation_names_the_recorder_the_meeting_and_our_token() {
    let body = add_bot_body(&session(), THREAD, false, "recorder-token");
    assert_eq!(body["participants"]["from"]["id"], ME);
    assert_eq!(body["participants"]["to"][0]["id"], RECORDER_BOT_MRI);
    assert!(
        body["participants"]["to"][0]["participantId"]
            .as_str()
            .is_some_and(|p| !p.is_empty())
    );
    let bot = &body["participantInvitationData"]["botData"];
    assert_eq!(bot["mode"], "RecordingAndTranscription");
    assert_eq!(bot["clientInfo"], "Teams-R4");
    assert_eq!(bot["consumerType"], "Teams");
    assert_eq!(bot["callId"], "chain-1");
    assert_eq!(bot["threadId"], THREAD);
    assert_eq!(bot["initiatorUserToken"], "recorder-token");
    assert_eq!(bot["meetingOrganizer"], "Self Example");
    assert_eq!(bot["recorderFeatures"]["useUnmixedAudio"], true);
    assert_eq!(body["groupChat"]["threadId"], THREAD);
    for link in ["addParticipantSuccess", "addParticipantFailure"] {
        let url = body["links"][link].as_str().unwrap();
        assert!(url.contains(&format!("/conversation/{link}/")), "{url}");
    }
}

#[test]
fn a_call_without_a_chat_thread_adds_the_bot_without_a_group_chat() {
    let body = add_bot_body(&session(), "19:a_b@unq.gbl.spaces", true, "t");
    assert_eq!(body["groupChat"], Value::Null);
    assert_eq!(
        body["participantInvitationData"]["botData"]["threadId"],
        "19:a_b@unq.gbl.spaces"
    );
}

#[test]
fn the_bot_is_added_on_the_conversation_link_or_under_the_controller() {
    let mut links = Links::default();
    assert_eq!(
        add_bot_url(&links, CONTROLLER, false),
        "https://api.flightproxy.skype.com/api/v2/cp/conv-example.conv.skype.com/conv/AAAAAAAAAAAAAAAAAAAAAA/addParticipantAndModality?i=1"
    );
    assert!(add_bot_url(&links, CONTROLLER, true).contains("/addParticipant?"));
    links
        .0
        .insert("addParticipantAndModality".into(), "https://l/add".into());
    assert_eq!(add_bot_url(&links, CONTROLLER, false), "https://l/add");
}

// --- the bot's commands ---

#[test]
fn captions_start_and_stop_differ_only_in_the_action() {
    let on = command_body(RecorderCommand::Captions(true), &session(), &ctx());
    let off = command_body(RecorderCommand::Captions(false), &session(), &ctx());
    assert_eq!(on["action"], "start");
    assert_eq!(off["action"], "stop");
    for b in [&on, &off] {
        assert_eq!(b["mode"], "transcription");
        assert_eq!(b["processingModes"], json!(["closedCaptions"]));
        assert_eq!(b["participantMri"], ME);
        assert_eq!(b["participantLegId"], "part-1");
        assert_eq!(b["timestamp"], "2026-11-19T08:44:12.705Z");
        assert!(b.get("participantSkypeToken").is_none(), "added on send");
    }
}

#[test]
fn start_recording_records_video_to_onedrive_and_transcribes() {
    let b = command_body(RecorderCommand::Recording(true), &session(), &ctx());
    assert_eq!(b["action"], "start");
    assert!(b.get("mode").is_none());
    assert_eq!(
        b["processingModes"],
        json!(["recording", "realTimeTranscript"])
    );
    let p = &b["actionParameters"];
    assert_eq!(p["type"], "start");
    assert_eq!(p["recordingMode"], "Normal");
    assert_eq!(p["meetingTitle"], "Standup");
    assert_eq!(p["correlationId"], "corr-1");
    assert_eq!(p["meetingOrganizer"], "Self Example");
    let storage = &p["recordingStorageSettings"][0];
    assert_eq!(storage["StorageType"], "OnedriveForBusiness");
    assert_eq!(storage["StorageLocation"], "Recordings");
    assert_eq!(
        storage["FileName"],
        "Standup-20261119_084412-Meeting Recording"
    );
}

#[test]
fn start_transcription_is_transcript_only_without_a_recording_mode() {
    let b = command_body(RecorderCommand::Transcription(true), &session(), &ctx());
    assert_eq!(b["action"], "start");
    assert_eq!(b["mode"], "transcription");
    assert_eq!(b["processingModes"], json!(["realTimeTranscript"]));
    assert!(b["actionParameters"].get("recordingMode").is_none());
    assert_eq!(b["actionParameters"]["type"], "start");
}

#[test]
fn stopping_names_only_what_stops() {
    let rec = command_body(RecorderCommand::Recording(false), &session(), &ctx());
    assert_eq!(rec["action"], "stop");
    assert_eq!(rec["mode"], "recording");
    assert_eq!(rec["processingModes"], json!(["recording"]));
    assert!(rec.get("actionParameters").is_none());
    let tr = command_body(RecorderCommand::Transcription(false), &session(), &ctx());
    assert_eq!(tr["action"], "stop");
    assert_eq!(tr["mode"], "transcription");
    assert_eq!(tr["processingModes"], json!(["realTimeTranscript"]));
}

#[test]
fn recording_file_names_are_safe_and_stamped() {
    assert_eq!(
        recording_file_name("Q3 / Plan: v2", "20261119_084412"),
        "Q3 _ Plan_ v2-20261119_084412-Meeting Recording"
    );
    assert_eq!(
        recording_file_name("  ", "20261119_084412"),
        "Meeting-20261119_084412-Meeting Recording"
    );
}

#[test]
fn the_caption_preference_is_our_endpoint_metadata() {
    let b = endpoint_metadata_body(&session(), true);
    assert_eq!(b["participants"]["from"]["id"], ME);
    assert_eq!(
        b["endpointMetadata"]["transcriptionPrefs"]["closedCaptions"],
        true
    );
    assert_eq!(
        endpoint_metadata_body(&session(), false)["endpointMetadata"]["transcriptionPrefs"]["closedCaptions"],
        false
    );
}

// --- the bot in the roster ---

fn bot_delta(state: &str, metadata: Value) -> Value {
    json!({"participants": {RECORDER_BOT_MRI: {
        "version": 3, "state": state, "details": {"displayName": "Recorder"},
        "endpoints": {"ep": {"endpointMetadata": metadata, "call": {}}}
    }}})
}

#[test]
fn the_roster_names_the_bots_command_url_and_what_it_runs() {
    let delta = bot_delta(
        "active",
        json!({"commandUrl": COMMAND_URL, "processingModes": {
            "closedCaptions": {"state": "Active"},
            "recording": {"state": "Inactive"}
        }}),
    );
    let bot = recorder_bot(&delta).unwrap().unwrap();
    assert_eq!(bot.command_url.as_deref(), Some(COMMAND_URL));
    assert!(bot.captions);
    assert!(!bot.recording);
}

#[test]
fn a_bot_that_left_and_other_people_are_told_apart() {
    assert_eq!(
        recorder_bot(&bot_delta("inactive", json!({}))),
        Some(None),
        "the bot left"
    );
    let person = json!({"participants": {ME: {"version": 1, "state": "active"}}});
    assert_eq!(recorder_bot(&person), None, "nothing about the bot");
    let without_url = bot_delta("active", json!({}));
    assert_eq!(
        recorder_bot(&without_url).unwrap().unwrap().command_url,
        None
    );
}

#[test]
fn the_meeting_chat_thread_is_read_from_a_conversation_update() {
    let body = json!({"conversation": {"activeModalities": {"groupChat": {"threadId": THREAD}}}});
    assert_eq!(chat_thread_id(&body).as_deref(), Some(THREAD));
    assert_eq!(chat_thread_id(&json!({"x": 1})), None);
}

// --- the flow: add the bot once, then send what was asked ---

fn bot(url: bool) -> calling::signaling::recorder::RecorderBot {
    calling::signaling::recorder::RecorderBot {
        command_url: url.then(|| COMMAND_URL.to_owned()),
        captions: false,
        recording: false,
    }
}

#[test]
fn the_flow_adds_the_bot_once_then_sends_each_command_in_order() {
    let mut f = RecorderFlow::default();
    assert_eq!(f.next(None), BotStep::Wait);
    f.request(RecorderCommand::Captions(true));
    f.request(RecorderCommand::Recording(true));
    assert_eq!(f.next(None), BotStep::AddBot);
    assert_eq!(f.next(None), BotStep::Wait, "the bot is on its way");
    assert_eq!(
        f.next(Some(&bot(false))),
        BotStep::Wait,
        "no command URL yet"
    );
    assert_eq!(
        f.next(Some(&bot(true))),
        BotStep::Send(COMMAND_URL.into(), RecorderCommand::Captions(true))
    );
    assert_eq!(
        f.next(Some(&bot(true))),
        BotStep::Send(COMMAND_URL.into(), RecorderCommand::Recording(true))
    );
    assert_eq!(f.next(Some(&bot(true))), BotStep::Wait);
}

#[test]
fn a_bot_already_in_the_meeting_is_reused() {
    let mut f = RecorderFlow::default();
    f.request(RecorderCommand::Transcription(true));
    assert_eq!(
        f.next(Some(&bot(true))),
        BotStep::Send(COMMAND_URL.into(), RecorderCommand::Transcription(true))
    );
}

#[test]
fn stopping_with_no_bot_has_nothing_to_stop() {
    let mut f = RecorderFlow::default();
    f.request(RecorderCommand::Captions(false));
    assert_eq!(f.next(None), BotStep::Wait);
}

#[test]
fn a_refused_bot_drops_what_waited_and_allows_another_try() {
    let mut f = RecorderFlow::default();
    f.request(RecorderCommand::Recording(true));
    assert_eq!(f.next(None), BotStep::AddBot);
    assert_eq!(f.bot_failed(), vec![RecorderCommand::Recording(true)]);
    assert_eq!(f.next(None), BotStep::Wait);
    f.request(RecorderCommand::Recording(true));
    assert_eq!(f.next(None), BotStep::AddBot);
}

// --- the meeting's recording state ---

fn states(recording: Option<(&str, u64)>, transcription: Option<(&str, u64)>) -> Value {
    let mut m = serde_json::Map::new();
    if let Some((s, n)) = recording {
        m.insert(
            "cloudRecording".into(),
            json!({"value": {"State": s}, "sequenceNumber": n}),
        );
    }
    if let Some((s, n)) = transcription {
        m.insert(
            "transcription".into(),
            json!({"value": {"State": s}, "sequenceNumber": n}),
        );
    }
    json!({"conversation": {"meetingStates": m}})
}

#[test]
fn the_recording_state_follows_the_meeting_state_and_ignores_stale_updates() {
    let mut st = RecordingState::default();
    assert_eq!(
        st.apply(&states(Some(("Active", 3)), None)),
        StateChange {
            recording: Some(true),
            transcription: None
        }
    );
    assert!(st.apply(&states(Some(("Active", 3)), None)).is_empty());
    assert!(
        st.apply(&states(Some(("Inactive", 2)), None)).is_empty(),
        "older"
    );
    assert_eq!(
        st.apply(&states(Some(("Inactive", 4)), None)).recording,
        Some(false)
    );
}

#[test]
fn transcription_has_its_own_state() {
    let mut st = RecordingState::default();
    assert_eq!(
        st.apply(&states(None, Some(("Active", 1)))),
        StateChange {
            recording: None,
            transcription: Some(true)
        }
    );
    assert!(st.apply(&json!({"participants": {}})).is_empty());
    assert!(st.apply(&Value::Null).is_empty());
}

// --- refusals ---

#[test]
fn a_refusal_reads_the_code_and_phrase_anywhere() {
    let body = json!({"participantInfos": [{"id": RECORDER_BOT_MRI}], "outcome": {
        "code": 403, "subCode": 10501, "phrase": "Recording is disabled by policy"
    }});
    let r = refusal(&body).unwrap();
    assert_eq!((r.code, r.sub_code), (403, 10501));
    assert_eq!(r.phrase, "Recording is disabled by policy");
    assert_eq!(r.text(), "Recording is disabled by policy (403/10501)");
    let bare = refusal(&json!({"code": 500})).unwrap();
    assert_eq!(bare.text(), "code 500/0");
    assert!(refusal(&json!({"participantInfos": []})).is_none());
}

#[test]
fn a_refusal_also_reads_an_error_message() {
    let body = json!({"error": {"code": "Forbidden", "message": "Not allowed"}});
    assert_eq!(refusal(&body).unwrap().text(), "Not allowed");
}

// --- caption text ---

fn frame(
    data_id: u8,
    seq: u16,
    remaining: u8,
    start: bool,
    source: u32,
    payload: &[u8],
) -> Vec<u8> {
    let mut b = vec![0x10, 11, data_id | if start { 0x80 } else { 0 }];
    b.extend_from_slice(&seq.to_be_bytes());
    b.push(remaining);
    b.extend_from_slice(&source.to_be_bytes());
    b.push(0);
    b.extend_from_slice(payload);
    b
}

#[test]
fn a_single_frame_is_one_message() {
    let mut d = Depacketizer::default();
    let m = d.push(&frame(3, 0, 0, true, 77, b"{\"a\":1}")).unwrap();
    assert_eq!((m.data_id, m.source_id), (3, 77));
    assert_eq!(m.payload, b"{\"a\":1}");
}

#[test]
fn fragments_join_in_sequence_and_a_gap_drops_the_message() {
    let mut d = Depacketizer::default();
    assert!(d.push(&frame(3, 10, 2, true, 5, b"he")).is_none());
    assert!(
        d.push(&frame(9, 0, 0, true, 6, b"x")).is_some(),
        "another id"
    );
    assert!(d.push(&frame(3, 11, 2, false, 5, b"ll")).is_none());
    assert_eq!(
        d.push(&frame(3, 12, 2, false, 5, b"o")).unwrap().payload,
        b"hello"
    );

    assert!(d.push(&frame(3, 1, 1, true, 5, b"a")).is_none());
    assert!(d.push(&frame(3, 5, 1, false, 5, b"b")).is_none(), "gap");
    assert!(
        d.push(&frame(3, 0, 0, false, 5, b"a")).is_none(),
        "no start"
    );
    assert!(d.push(&[0x10, 11, 0x83]).is_none(), "short");
    assert!(d.push(&[]).is_none());
}

#[test]
fn recipients_in_the_header_are_skipped() {
    let mut b = frame(3, 0, 0, true, 5, b"");
    b[1] = 15;
    b[10] = 1;
    b.extend_from_slice(&9u32.to_be_bytes());
    b.extend_from_slice(b"body");
    assert_eq!(Depacketizer::default().push(&b).unwrap().payload, b"body");
}

#[test]
fn caption_messages_carry_speaker_text_and_finality() {
    let message = json!({"recognitionResults": [
        {"id": "u1", "text": " Hello there ", "isFinal": false, "userId": "8:orgid:a", "displayName": "Ada Example"},
        {"id": "u2", "text": "", "userId": "8:orgid:b"},
        {"utteranceId": "u3", "rawText": "raw", "isFinal": true, "userId": "8:orgid:b"},
    ]});
    let lines = parse_caption_message(message.to_string().as_bytes());
    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines[0],
        CaptionLine {
            id: "u1".into(),
            user_id: "8:orgid:a".into(),
            speaker: "Ada Example".into(),
            text: "Hello there".into(),
            is_final: false,
        }
    );
    assert_eq!((lines[1].id.as_str(), lines[1].is_final), ("u3", true));
    let tracks = json!({"textTracks": [{"id": "t", "text": "from a track", "userId": "x"}]});
    assert_eq!(
        parse_caption_message(tracks.to_string().as_bytes()).len(),
        1
    );
    assert!(parse_caption_message(b"not json").is_empty());
    assert!(parse_caption_message(br#"{"events": []}"#).is_empty());
}

fn line(id: &str, speaker: &str, text: &str, is_final: bool) -> CaptionLine {
    CaptionLine {
        id: id.into(),
        user_id: String::new(),
        speaker: speaker.into(),
        text: text.into(),
        is_final,
    }
}

#[test]
fn the_caption_log_updates_an_utterance_in_place_and_keeps_the_latest() {
    let mut c = Captions::new(2);
    assert!(c.add(line("u1", "Ada", "Hel", false)));
    assert!(c.add(line("u1", "Ada", "Hello", true)));
    assert!(!c.add(line("u1", "Ada", "Hello", true)), "nothing new");
    assert_eq!(c.lines().len(), 1);
    assert_eq!(c.lines()[0].text, "Hello");
    c.add(line("u2", "Bo", "Hi", true));
    c.add(line("u3", "Ada", "Bye", true));
    let ids: Vec<&str> = c.lines().iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, ["u2", "u3"]);
    c.clear();
    assert!(c.lines().is_empty());
}
