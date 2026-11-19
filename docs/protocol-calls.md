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

What kelma does now (`calling::incoming`, used by the GUI accept and by
`kelma-cli call listen --accept-audio`):

1. **Fetch the offer** (default). `attach` with `requireMediaContent: true`. The caller's offer is
   expected in the attach answer's `callInvitation.mediaContent` (with `mediaLegId`), and the
   answerer path follows: ringing, then `acceptance` with our SDP answer. Evidence: rusty-teams
   (MIT, `engine.rs` `ring_task`) does exactly this for a ring without an offer, and the mautrix-teams
   OpenAPI says the web client sends `false` "because the offer came in the ring". Not yet seen live.
2. **Offer ourselves** (fallback, UNVERIFIED). When the attach answer has no offer either, kelma
   accepts with its own offer (built as `call dial` builds it, verified live for outgoing calls)
   and a `mediaAnswer` callback link. The caller's answer is taken from the acknowledgement
   (`callAcceptanceAcknowledgement.mediaContent`) or from a `call/mediaAnswer/` callback within
   20 s, applied, and the call is adopted.
3. `kelma-cli call listen --accept-audio --offer-in-attach` instead sends our offer inside the
   attach (`attach.mediaContent`, plus a `mediaAnswer` callback link), then accepts with it.
   UNVERIFIED: no reference sends media with an attach.

`--ring-only` now prints whether the attach answer carried an offer. `KELMA_CALL_DUMP=<dir>` makes
the GUI write every raw calling push (and each SDP of a call being answered) into `<dir>`, like
`call listen --dump`. Both hold real ids: keep them out of the repo.

## Open

- **TURN over TCP/TLS.** webrtc-rs 0.21.1 only relays over UDP. A network that blocks UDP 3478 needs a TLS shim to `turns:<gateway>:443`.
- **1:1 calls.** An outgoing 1:1 call to a Teams web guest worked both ways (2026-11-19, `call dial` and the GUI). Accepting a ring without an offer is built (above) but not verified; declining is not verified.
- **AEC delay.** The stream delay starts at a fixed 40 ms (AEC3 estimates the rest). It is not yet taken from cpal's measured latency.
- **Disconnected.** One short `Disconnected` → `Connected` blip was seen before leaving.
