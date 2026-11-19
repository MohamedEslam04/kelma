# Native calling: what was verified live

`crates/calling` joins Teams meetings natively (NGC signalling over `epconv`, WebRTC media through
webrtc-rs, Opus). This page records what was **measured against the live service**. Everything
else in the crate comes from the references in `docs/research/open-source.md` §3.15 and is still
unverified.

Test rig: kelma, signed in with a work account, joins a "Meet now" meeting hosted by a personal
(Teams free) account. The tenant blocks 1:1 calls to personal accounts, but it allows joining
their meetings. Command: `kelma-cli call join --secs 60 --out <wav> --dump <dir>`.

## Verified (2026-11-18)

1. **Discovery.**
   - authz `regionGtms` names the conversation service, e.g. `api-noam.flightproxy.teams.microsoft.com`. kelma reads it with its own authz POST.
   - `trap/tokens` returns one TURN credential that is valid for 604800 s.
2. **Calling trouter.**
   - A trouter connection of kelma's own registers `NextGenCalling` with `DesktopNgc_2.5:SkypeNgc` on `<surl>NGCallManagerWin` and is accepted.
   - Call callbacks arrive on that connection.
3. **Join target.**
   - The CSA chat entry of the meeting (`chatType: meeting`, `productContext: TFL`) carries `activeMeetup` while the call runs, plus `meetingInformation`.
   - A personal account's meeting has the all-zero tenant id and an upper-case cid as `organizerId`.
   - The join by meeting code (`meetingData{meetingCode, passcode}`, two POSTs) works.
   - The meeting's conversation lives on the consumer service: `api.flightproxy.skype.com/api/v2/cp/…conv.skype.com`.
4. **Lobby.**
   - A work account joining a personal account's meeting lands in the lobby.
   - The service still answers (`call/acceptance` with a media answer), and media reaches Connected. That connection carries no meeting audio.
5. **Admission restarts the media session.**
   - After the organizer clicks Admit, the service sends a roster update and then a `mediaRenegotiation`. This is a full new offer:
     - the audio mid changes (0 → 1) and every section has new ICE credentials
     - one audio section, ten `main-video` gallery sections, one `applicationsharing-video` section and an `x-data` section, each with its own `ice-ufrag`
     - profile `RTP/AVP`
     - Opus at payload type **102**; 111 is SIREN here
   - Answering it on the connected peer connection leaves kelma mute: webrtc-rs adds a new `recvonly` transceiver. kelma answers it on a **fresh** `MediaSession`, as audio only, with port-0 stubs for every other section, and closes the old one.
6. **Two-way audio.**
   - A 440 Hz tone sent by kelma was heard in the meeting.
   - The organizer's voice was received and decoded: 1046 packets, about 21 s, confirmed by ear.
   - webrtc-rs's DTLS-SRTP works with the Teams media server.
   - Leaving through the `leave` link ends the call cleanly.
7. **Real devices.**
   - Same run shape with `--mic --speaker`: the laptop's built-in mic through cpal/ALSA (PipeWire), output to Bluetooth earbuds (A2DP).
   - The organizer heard kelma's mic, and kelma played the organizer's voice: 4510 packets sent over 90 s, 1466 received, no underruns or errors.
   - With the phone (organizer) near the laptop, its speaker leaked into the laptop mic and came back distorted. kelma has no echo cancellation yet.
8. **TURN.**
   - Every trap relay gateway answers the first, unauthenticated Allocate with `300 Try Alternate`. The answer carries `ALTERNATE-SERVER`, `REALM=rtcmedia` and a `NONCE`.
   - The alternate address answers `401` and then allocates with the `trap/tokens` credentials: one hop.
   - webrtc-rs does not follow the 300: it signs a retry to the same gateway and fails. So kelma probes first (`relay::resolve_ice_servers`) and gives webrtc-rs the alternate's `turn:<ip>:3478?transport=udp`.
   - Gateways that resolve in DNS: `gateway-{eu,us,in,world}.az.relay.teams.cloud.microsoft`. kelma maps the authz region to one of them.
   - A relay-only run (`--relay-only`: only `typ relay` candidates) reached Connected in the lobby and in the meeting.
9. **Echo cancellation.**
   - `calling::audio` runs AEC3, noise suppression and AGC2 through `sonora` (a pure-Rust port of WebRTC's audio processing) on the mic path. The far-end reference is tapped from the playback device callback.
   - Live check: the far end, on earbuds in another room, heard no echo of themselves while kelma played them on the laptop speaker into the laptop mic.
   - With `--no-aec` the echo came back.
   - Two endpoints in the same room still howl, as they do with the official clients: kelma cancels only its own playback.
10. **Remote tracks.**
   - Teams' SDP names no SSRCs, only `a=x-ssrc-range`, so a remote track can arrive with an unknown kind.
   - Every track must be drained. Otherwise webrtc-rs's queue fills and the packets are dropped.

## Live 2026-11-19: invitation without offer; offerer path unverified until next live test

A 1:1 call from a Teams web guest to the work account reached kelma: the toast showed, and
Accept failed with "the call carries no media offer". The ring's shape (no values recorded):

- push url `<surl>NGCallManagerWin`
- `callNotification`
  - `from` / `to`: `displayName`, `displayNameSource`, `endpointId`, `id`, `participantId`,
    `tenantId`, `languageId`, `hidden`, `propertyBag`; `from.region`
  - `links`: `attach`, `progress`, `reject` (on the call controller's `forked` leg),
    `udpTransport` (`udp://…`) and `mediaAnswer: "cc://ma"`, a placeholder, not a URL
  - `mediaContent: null`, `udpKey: null`, `replaces: null`, `clientEndpointCapabilities`
  - no `callModalities` (kelma assumes audio)
- `conversationInvitation`: `conversationController`, `isMultiParty`, `isBroadcast`
- `debugContent`: `callId`, `participantId`, `callStartTime`, `ProcessingCallControllerInstance`,
  `ecsEtag`, `potentialCallNotificationSent`
- `groupContext: null`; no `groupChat`

Second live test (same day, GUI Accept, `KELMA_CALL_DUMP`), shape only:

- No SDP was exchanged: kelma dumped none, so its answer stopped at or before the attach.
- Callbacks reached kelma's attached session: `call/progress` with `status: forwarded` and
  `forwardingDestinationType: voicemail`; then `call/end` from a `voicemail` endpoint with
  `callEnd {code: 1000, subCode: 7106, phrase: SdpParsingFailure, resultCategories:
  [UnexpectedServerError]}`; then `conversation/conversationEnd` (`subCode` 5002, "all
  participants left") carrying the same 7106 as `callControllerTransactionEnd`.
- Reading: the ring went to voicemail unanswered, and the voicemail leg failed on SDP. No SDP of
  kelma's reached the service, so 7106 says nothing about kelma's SDP. The bodies kelma sent
  were not recorded; they are now.

What kelma does now (`calling::incoming`, used by the GUI accept and by
`kelma-cli call listen --accept-audio`; the mode is `KELMA_CALL_NO_OFFER` in the GUI and
`--no-offer` in the CLI):

1. **`fetch`** (default). `attach` with `requireMediaContent: true`, bounded to 15 s. An offer in the
   attach answer's `callInvitation.mediaContent` (with `mediaLegId`) is answered: ringing, then
   `acceptance` with our SDP answer. Evidence: rusty-teams (MIT, `engine.rs` `ring_task`) does
   exactly this, and the mautrix-teams OpenAPI says the web client sends `false` "because the offer
   came in the ring". Without an offer it continues as `new-offer`.
2. **`new-offer`** (UNVERIFIED). Attach as the web client does (`requireMediaContent: false`), then
   POST our offer as `{"mediaNegotiation": {callModalities, sender, links {mediaAnswer, rejection},
   mediaContent {…, newOffer: true}}}` (rusty-teams' shape for a client's own offer) to the attach
   answer's `newOffer` link, accept with `mediaContent: null`, and apply the caller's answer
   (inline, or on `call/mediaAnswer/` within 20 s).
3. **`accept-offer`** (UNVERIFIED, likely wrong). Our offer as the acceptance's `mediaContent`. That
   slot carries an answer everywhere else, and an answer with offer-only lines draws
   `SdpParsingFailure` (teams-lite).
4. **`attach-offer`** (UNVERIFIED). Our offer inside the attach, then accept with `mediaContent: null`.

Every offer of ours is built and labelled as `call dial`'s, which the service took live:
`application/sdp-ngc-1.0`, plain SDP text, `requiredFeatures: nonByPass`, our `mediaLegId`.

`--ring-only` prints whether the attach answer carried an offer. `KELMA_CALL_DUMP=<dir>` makes the
GUI write every raw calling push, every request it sends to the conversation service with its
answer (`ngc-<op>.json`), each step of an answer (`step.txt`), every SDP envelope as received
before decoding (`*-blob.txt`) and every SDP (`.sdp`). `call listen --dump` writes the same. All
hold real ids and addresses: keep them out of the repo.

## Live captions, recording and transcription (research, 2026-11-19)

Nothing in this section was run live by kelma. Everything is from the references below, read at
these commits: rusty-teams `21da0e2` (MIT), eisbaw/ost `0892144` (MIT), mautrix-teams `e1d3018`
(AGPL, facts only, no code copied). Code: `calling::signaling::recorder` (bodies, roster, state,
flow) and `calling::captions` (caption framing and text).

### All three go through the recorder bot

Teams web's More › "Record and transcribe" › Start recording / Start transcription, and More ›
"Language and speech" › Show live captions, all drive one service bot.

1. **Invite the bot.**
   - Request: `POST` on the conversation's `addParticipantAndModality` link. A call without a chat
     thread uses `addParticipant`. The credential is the usual IC3 bearer.
   - Body:
     - `participants.to[0].id` = `28:bdd75849-e0a6-4cce-8fc1-d7c0d4da43e5`, with a fresh
       `participantId`;
     - `participantInvitationData.botData`: `clientInfo: Teams-R4`, `callId` (the chain id),
       `threadId`, `recorderFeatures {…, useUnmixedAudio: true}`,
       `mode: RecordingAndTranscription`, `consumerType: Teams`, `meetingOrganizer`, and
       `initiatorUserToken`;
     - `groupChat {threadId}`;
     - `links.addParticipantSuccess` / `addParticipantFailure` (our callback URLs).
   - `initiatorUserToken` is a token for resource `4580fd1d-e5a3-4f56-9ad1-aab0e3bf8f76`, the
     recorder service.
   - Sources: rusty-teams `captions.rs` `add_bot_body`, `signaling.rs` `add_caption_bot`; ost
     `recording.rs` `add_recorder_bot` ("payload matches real Teams web client capture");
     mautrix-teams `teams-webclient.yaml` `callAddBotParticipant` (it notes this flow "is not in
     the captures").
2. **The command URL.**
   - The bot's roster entry (a `28:` participant) has, under an endpoint's `endpointMetadata`:
     - `commandUrl`, e.g. `https://api.flightproxy.teams.microsoft.com/api/v2/ep/…/v2/oncommand/<id>`;
     - `processingModes.closedCaptions.state` and `processingModes.recording.state`
       (`Active` while running).
   - Source: rusty-teams `roster.rs` `caption_bot`. ost instead builds
     `<recorder host>/v2/oncommand/<conversation id>` from the `addParticipantSuccess` push.
3. **Commands.**
   - Request: `POST <commandUrl>` with `Authorization: Bearer <recorder token>` and
     `X-Skypetoken: <skypetoken>`.
   - Body: `{timestamp, participantMri, participantLegId, action: start|stop, mode?,
     processingModes, participantSkypeToken, actionParameters?}`.
   - Variants (rusty-teams `captions.rs` `command_body`, `recording.rs` `start_body`/`stop_body`,
     "match the capture"):

     | What | `mode` | `processingModes` | `actionParameters` |
     |---|---|---|---|
     | Captions on / off | `transcription` | `[closedCaptions]` | none |
     | Start recording | none | `[recording, realTimeTranscript]` | `type: start`, `recordingMode: Normal`, `recordingStorageSettings [{StorageType: OnedriveForBusiness, StorageLocation: Recordings, FileName}]`, `meetingTitle`, `correlationId`, `recordingFeatures` |
     | Start transcription | `transcription` | `[realTimeTranscript]` | the same, without `recordingMode` |
     | Stop recording | `recording` | `[recording]` | none |
     | Stop transcription | `transcription` | `[realTimeTranscript]` | none |

   - Recording file name: `<title>-<YYYYMMDD_HHMMSS>-Meeting Recording`.
   - ost sends `participantSkypeToken: ""` and `spokenLanguage: en-us`. rusty-teams sends the real
     token and `""`. kelma follows rusty-teams.
   - rusty-teams retries a stop that the recorder answers while it "still settles".
4. **Caption preference.** rusty-teams also sends `PUT updateEndpointMetadata`
   `{participants.from, endpointMetadata {holographicCapabilities: 3, transcriptionPrefs
   {closedCaptions: true|false}}}` with each captions start and stop.

### State and consent

- Recording and transcription show in conversation updates for everyone:
  - `meetingStates.cloudRecording` and `meetingStates.transcription`, each
    `{value: {State: Active|…}, sequenceNumber}`;
  - an update with a lower `sequenceNumber` is stale.
  - Source: rusty-teams `recording.rs` `RecordingStatus::apply`.
- When the meeting requires explicit consent, `meetingDetails.recordingConsentDetails
  .consentActivelyRequired` is true. Each attendee then publishes `consentToRecording` or
  `denyConsentToRecording` (`POST publishState`, user level, `content: "{}"`).
  - Source: rusty-teams `recording.rs` `consent_body`.
  - kelma does not publish consent yet. It shows Teams' notice ("Recording has started. By
    attending this meeting, you consent to being included.") when the state turns on.

### Caption text

- **Not on trouter.** The text arrives on the media connection's SCTP data channel
  `main-channel`, which is the SDP's `x-data` section (`a=x-data-protocol:sctp`).
- **Framing.** Each frame has a header of 11 or more bytes:
  - byte 0 (low nibble) and byte 1: the header length;
  - byte 2: the data id (`& 0x3f`), with `0x80` marking the first fragment;
  - bytes 3–4: the sequence;
  - byte 5: the fragments left;
  - bytes 6–9: the source id;
  - byte 10: the recipient count, then the recipients.
- **Payload.** Captions use data id 3. The payload is JSON `recognitionResults[]` or
  `textTracks[]`, each entry `{id|utteranceId, text|rawText, isFinal, userId, displayName}`. A
  later partial or final result with the same id replaces the earlier one.
- Sources: rusty-teams `data_channel.rs`, `captions.rs` `parse_caption_message`, `call/mod.rs`
  (`DATA_CHANNEL_LABEL = "main-channel"`, `CAPTIONS_DATA_ID`). rusty-teams' ROADMAP says this
  works live with speech from guests.
- **Blocked in kelma.** kelma's media session does not open that channel: webrtc-rs refuses the
  `x-data` section, and it is answered as a port-0 stub (`media/sdp.rs` `KEPT_KINDS`). Until the
  media session maps `x-data` to a WebRTC data channel and hands its frames to
  `calling::captions::Depacketizer`, the caption strip turns on but stays empty.

### UNVERIFIED

- Whether kelma's refresh token can mint a token for the recorder resource
  (`Audience::App("4580fd1d-…")`, scope `<resource>/.default`). rusty-teams gets one in the web
  page with the Teams client, ost with its own OAuth.
- The shape of `addParticipantFailure`, and of a command's error answer when policy refuses.
  kelma reads the first `{code, subCode, phrase}`, else an error `message`, and shows it as the
  banner text.
- Whether a 1:1 call (no meeting) accepts the bot. rusty-teams adds it threadless with
  `addParticipant`. Captions may need a meeting.
- Whether the web client sends the `updateEndpointMetadata` preference. Only rusty-teams does.

## Open

- **TURN over TCP/TLS.** webrtc-rs 0.21.1 only relays over UDP. A network that blocks UDP 3478 needs a TLS shim to `turns:<gateway>:443`.
- **1:1 calls.** An outgoing 1:1 call to a Teams web guest worked both ways (2026-11-19, `call dial` and the GUI). Accepting a ring without an offer is built (above) but not verified; declining is not verified.
- **AEC delay.** The stream delay starts at a fixed 40 ms (AEC3 estimates the rest). It is not yet taken from cpal's measured latency.
- **Disconnected.** One short `Disconnected` → `Connected` blip was seen before leaving.

## Screen sharing: what the references say (research 2026-11-19, nothing verified live yet)

Sources, facts only: rusty-teams (MIT, `crates/calling/src/{video_layout,hold,signaling,sdp}.rs`,
`call/mod.rs`, test fixtures), mautrix-teams (AGPL, `docs/openapi/teams-webclient.yaml` from
captures of the web client, `pkg/msteams/{video,calls}.go`, `pkg/teamsmedia/{direct,sdp}.go`). No
code was copied from either.

1. **The sharing line.** A screen share is an ordinary `m=video` section with
   `a=label:applicationsharing-video`; the camera and the gallery are `a=label:main-video`
   (rusty-teams `sdp.rs` `LineRole`, mautrix-teams `teamsmedia/sdp.go`). There is no separate
   VBSS transport or protocol on the wire the web client uses; "applicationsharing" names the
   line. The session level of a media server's SDP announces it with
   `a=x-mediabw:applicationsharing-video send=8100;recv=8100` (rusty-teams fixture
   `teams_answer.sdp`; kelma's own live re-offer, fixture `teams_meeting_full_reoffer.sdp`).
2. **Codec.** H.264, `packetization-mode=1`. The media server's answer to a client offer names
   `H264/90000` with `profile-level-id=42C01E;packetization-mode=1;max-mbps=108000;max-fs=3600;
   max-br=2000;max-fps=3000`, `rtx` (apt) and AV1 as an alternative, `b=AS:2400`,
   `a=x-source-streamid:212` and its own `a=x-ssrc-range` (rusty-teams `teams_answer.sdp`). The
   media server's own re-offer lists `X-H264UC`, `H264` (107, `profile-level-id=42C02A`), `H265v2`,
   `x-ulpfecuc`, `VP8` and `rtx` on the sharing line (kelma live re-offer). The web client offers
   the sharing line with H.264 `profile-level-id=64001f` and screen limits
   `max-fs=8160;max-mbps=135000;max-fps=1500` and `a=x-signaling-fb:* x-message app send:src
   recv:src,vc` (rusty-teams `teams_offer.sdp`; `video_layout.rs` `SCREEN_REQUEST`). A
   constrained-baseline stream (42e01f) is the safe choice to send.
3. **Meetings: no renegotiation.** The web client offers the sharing line up front (sendrecv)
   when it joins, and switches sharing on or off by POSTing `UpdateMediaDescriptions` to the
   call leg's `updateMediaDescriptions` link:
   `{"UpdateMediaDescriptions": {"mediaDescriptions": {"descriptions": [{mid, direction, label?}…],
   "negotiationTag": "<participantId>;ss_1", "requestId": n}}}` — the sharing line
   `direction: sendonly, label: applicationsharing-video`, every other send-capable line
   `recvonly`; `requestId` increments per change and reappears as the roster streams'
   `mdRequestId`; answered 200 with `{code: 0, phrase: "Success."}`. Before a renegotiation the
   sharing mid is 11, after the media server's re-offer it is 3 (mautrix-teams openapi
   `/updateMediaDescriptions`, from captures; mautrix-teams `video.go` `SetVideo`). rusty-teams
   sends the same body with `direction: sendrecv` (`video_layout.rs` `MediaDescription`).
   The link arrives in the acceptance's or `call/mediaAcknowledgement/` callback's `links`; the
   client also publishes its own `updateMediaDescriptions` and `controlVideoStreaming` callback
   links in its acceptance (mautrix-teams `calls.go` `acceptanceLinks`; kelma already does).
4. **Meetings: who shares.** The enterprise web client also POSTs `updateParticipantProperties`
   on the conversation with `propertyBag.screenSharerAttributes {sequenceNumber, value {source:
   1}}` (202; the consumer client did not; mautrix-teams openapi). Roster endpoints list
   `call.mediaStreams[]` with `type: applicationsharing-video`, `sourceId` and a `send…`
   direction while someone shares (mautrix-teams `calls.go`; rusty-teams `roster.rs`); the
   meeting join's `callModalities` include `ScreenViewer` in mautrix-teams (`videoMedia`).
5. **Sender control.** The media controller tells the sender what to send through the
   `call/controlVideoStreaming/` callback: `controlInfo[{sourceId, fmtParams}]`, where
   `fmtParams` carries `max-fs`, `max-fps`, `max-br` and `KeyFrame=1` for a keyframe request.
   mautrix-teams routes the one naming its own sharing `sourceId` to the share encoder and waits
   up to 2 s for it before encoding (`video.go` `handleControl`, `connector/rtcvideo.go`).
6. **1:1 (direct) calls: renegotiation.** A person-to-person call has no media server to flip
   lines, so the client sends a new offer: POST to the call's `mediaRenegotiation` link
   `{"mediaNegotiation": {"callModalities": ["Audio", ("Video",) "ScreenSharer"], "sender",
   "links": {mediaAnswer, rejection}, "mediaContent": {blob, contentType, "requiredFeatures":
   "nonByPass", "negotiationTag": "<participantId>;ss_1", mediaLegId, …}}}`; the answer comes on
   `call/mediaAnswer/` (mautrix-teams `calls.go` `Renegotiate`; rusty-teams `hold.rs`
   `offer_modalities`, `signaling.rs` `renegotiation_body`). The web client's 1:1 offer has
   audio (mid 0), camera `main-video` (mid 1) and `applicationsharing-video` (mid 2); the
   sharing line is `sendonly` while sharing, `recvonly` while the other side shares, else
   `inactive` (mautrix-teams `direct.go`). A rejected sharing line is offered again under a new
   mid with the camera line's payload type.
7. **Bundling a media server's re-offer.** The media servers' re-offer gives audio, video and
   sharing different ICE credentials (and the sharing line its own port) yet lists every mid in
   one `a=group:BUNDLE`. mautrix-teams answers it with the accepted video lines bundled onto the
   audio transport (`teamsmedia/sdp.go` `answerSDP`). rusty-teams rewrites the directions of the
   answers it gets so its own sending is not stopped (`hold.rs` `keep_media_running`).

What kelma does with this (`calling::video`, `calling::media`, `calling::signaling::video`,
`kelma/src/backend/call/share.rs`):

- **Opt-in**: the sharing line is offered only with `KELMA_SCREEN_SHARE=1`, so the audio-only
  offer verified above stays the default until a live run confirms the media servers take it.
- A `MediaSession` built with `MediaOptions::video = [VideoRole::ApplicationSharing]` registers
  H.264 (`packetization-mode=1`, constrained baseline as `42c01f` at payload type 107 and
  `42e01f` at 108: webrtc-rs pairs H.264 by the first two bytes of `profile-level-id`, and the
  media servers write `42C0..` where browsers write `42e0..`) and offers one sendrecv video line
  labelled `applicationsharing-video` (the label comes from the track's stream id). It answers a
  media server's re-offer with audio plus the first sharing line, bundled on the audio transport
  (the line's own ICE credentials and candidates are replaced by the audio line's before
  webrtc-rs reads them). A `sendonly` sharing line is read as `sendrecv`, so our sender stays
  able to send, and answered `recvonly` on the wire (the direction the offer allows).
  `VideoRole::MainVideo` is the seam for the camera: same encoder, same `VideoSender`, the
  first `main-video` line.
- Meeting: start/stop is `UpdateMediaDescriptions` (`sendonly` + label, tag `;ss_1`), on the
  `updateMediaDescriptions` link, else `{callLeg}/updateMediaDescriptions`.
  1:1: start/stop is a `mediaNegotiation` re-offer with `ScreenSharer`, the sharing line
  `sendonly` (stop: `inactive`).
- A `controlVideoStreaming` callback with `KeyFrame=1` forces a keyframe; otherwise one goes out
  every 2 s.
- Capture: the ScreenCast portal (`ashpd`, monitors and windows, cursor embedded), the PipeWire
  node read by a `gst-launch-1.0 pipewiresrc` child (the portal's fd is its stdin) as raw I420
  at up to 1280x720 (the media server's answer allows `max-fs=3600`) and 15 frames a second,
  encoded in-process by OpenH264 at 1.5 Mbit/s (screen-content mode). Runtime needs GStreamer's
  PipeWire plugin (`pipewire-gstreamer` on Fedora). The capture spike:
  `cargo run -p calling --example screencast -- 5 /tmp/share.h264`, then `ffprobe` the file.

UNVERIFIED (each needs a live run with the official client watching): that the media server
accepts kelma's join offer with a sharing line but no `main-video` line; that a browser peer of a
1:1 call binds our `42c01f` track when it answers `42e01f`; that a sharing line
answered `recvonly` can then be flipped to sending by `UpdateMediaDescriptions`; that the media
server forwards a share without `updateParticipantProperties`; whether `controlVideoStreaming`
must be waited for before the first keyframe; the 1:1 re-offer's answer path.
