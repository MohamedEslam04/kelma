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

## 1:1 call controls: add a person, hold, transfer (built 2026-11-19, UNVERIFIED live)

`calling::signaling::call_ops` builds these; the GUI sends them from the call window (People
panel invite box, Hold / Resume, Transfer). No HAR backs them: each shape comes from the
reference named, and none has been run against the service yet.

Where the links come from (live 1:1 dump, names only):

- the attach answer's `additionalActionResponses[].output` carries the conversation controller
  and the conversation's `links` (`addParticipant`, `addParticipantAndModality`, `admit`, ...).
  kelma keeps the controller and the two add links; it does not take the conversation's `leave`,
  so a 1:1 call still hangs up on its leg (`DELETE callLeg`).
- the acceptance acknowledgement's links carry `hold`, `transfer`, `replacement`,
  `retargetCompletion`, `startOutgoingNegotiation`, `callLeg` and `mediaRenegotiation`.

1. **Add a person** (UNVERIFIED). `POST addParticipant` (the link, else
   `<controller>/addParticipant`):
   `{disableUnmute: false, participants: {from, to: [{id, displayName, participantId: <new uuid>}]},
   participantInvitationData: {}, replacementDetails: null, debugContent: {},
   links: {addParticipantSuccess, addParticipantFailure}}` (our `conversation/` callbacks).
   - mautrix-teams `calls.go` `AddParticipant` and `teams-webclient.yaml` `/add` (AGPL, facts
     only): without a known group chat thread the web client posts to `/addParticipant` with
     `debugContent: {}` instead of `groupContext`/`groupChat`. Read from the web client's code;
     no capture shows it.
   - rusty-teams `organizer.rs` `add_participants_body` (MIT): the same fields plus
     `displayName`. For a 1:1 call rusty-teams first creates a group chat and posts to
     `addParticipantAndModality` with `groupChat: {threadId, messageId: null}`; kelma does not
     create that chat yet. If the plain add is refused, that is the next shape to try.
   - The answer comes on `conversation/addParticipantSuccess` or `addParticipantFailure`
     (the GUI shows an error on failure). The move to a conference is the service's: kelma
     answers its re-offer as it answers the lobby's (a fresh media session when ICE restarts).
     rusty-teams instead re-offers on a new peer connection itself and waits for
     `retargetCompletion`; kelma does not.
2. **Hold / Resume** (UNVERIFIED). A re-offer on the same peer connection, `POST
   mediaRenegotiation` with `{mediaNegotiation: {callModalities, sender, links {mediaAnswer,
   rejection}, mediaContent {blob, contentType, mediaLegId, requiredFeatures: nonByPass,
   newOffer: false}}}`. To hold, every media line of our offer says `a=inactive`; to resume, the
   offer is sent as the session makes it. The answer (inline or on `call/mediaAnswer/`) is applied
   with our own directions mirrored back, so the local media never stops; kelma stops sending
   and playing audio itself while on hold. A media answer that names `mediaAcknowledgement` is
   acknowledged with an empty POST.
   - rusty-teams (MIT): `hold.rs` `all_inactive` / `keep_media_running`, `call/peer_sdp.rs`
     `offer_same_peer` (`newOffer: false`) and `call/mod.rs` (`POST mediaAcknowledgement` after
     applying an answer). Its roadmap lists hold as "renegotiates every line inactive", not run
     with a second person.
   - Teams on the web (live, 2026-11-19, network log only): opening Transfer puts the call on
     hold; Resume showed only a media answer acknowledgement
     (`.../cc/v1/mediaAnswers/<id>/<n>/t/<n>/acknowledge`), no separate hold or resume URL.
   - Neither reference posts to the acknowledgement's `hold` link; kelma does not either.
   - A refused re-offer leaves webrtc-rs with a pending local offer (kelma cannot roll it back
     outside `crates/calling/src/media`); the GUI puts the hold state back and shows an error.
3. **Transfer now** (blind; UNVERIFIED). `POST transfer` (the acknowledgement's link):
   `{callTransfer: {target: {id: <MRI>}, transferor: {details: {id, endpointId, participantId,
   languageId}, authorizationToken: null}, links: {transferAcceptance, transferCompletion}
   (our `call/` callbacks), replacementDetails: null, disableForwardingAndUnanswered: false,
   transferContext: null}}`.
   - rusty-teams `transfer.rs` `transfer_body` and `completion` (MIT).
   - `call/transferCompletion` with `code` 0 or 2xx means done: kelma ends the call ("You
     transferred the call to ...") and still ends its leg. Any other code keeps the call on hold
     with an error.
   - The GUI opens Transfer with the call on hold, as Teams on the web does. Consult first (a
     second call, then `replacementDetails` naming its leg) is not built.

## Open

- **TURN over TCP/TLS.** webrtc-rs 0.21.1 only relays over UDP. A network that blocks UDP 3478 needs a TLS shim to `turns:<gateway>:443`.
- **1:1 calls.** An outgoing 1:1 call to a Teams web guest worked both ways (2026-11-19, `call dial` and the GUI). Accepting a ring without an offer is built (above) but not verified; declining is not verified.
- **AEC delay.** The stream delay starts at a fixed 40 ms (AEC3 estimates the rest). It is not yet taken from cpal's measured latency.
- **Disconnected.** One short `Disconnected` → `Connected` blip was seen before leaving.
