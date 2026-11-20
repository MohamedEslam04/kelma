# Overnight call work (2026-11-19 → 11-20)

Morning summary of the call features built overnight. Each feature is built on its own branch from
`feat/calls-callee-offer`, then merged back into it and on into `feat/call-settings` (draft PR #13).

## Already live-tested on 2026-11-19

- Answering an incoming 1:1 call: audio both ways (attach with `requireMediaContent`).
- Mute reaches Teams: the other side sees the mic-off icon. The conversation controller comes from
  the attach answer.
- Leaving the call view keeps the call in a floating card. The card can be dragged, and it has
  Speaker/Gallery layouts.
- Teams web, observed live: opening Transfer puts the call on hold, and Resume is a media
  renegotiation (`mediaAnswers/.../acknowledge`).

## Integrated (feat/calls-callee-offer 0aa7bee)

All branches below are merged into one tree. On that tree:
- `cargo test -p calling`: 430 passed, 0 failed.
- `cargo test -p kelma --bin kelma -- call reminders calls_page`: 144 passed, 0 failed.
- `cargo clippy -p calling -p kelma --all-targets -D warnings`: clean.

kelma-18 merged it into feat/call-settings (PR #13) as f96fc9b. The test build of f96fc9b passed. CI
on f96fc9b failed only on rustfmt (the module order in `backend/mod.rs`); kelma-18 is fixing that.

Known gaps:
- After swapping the shown and held calls, the last video frame of the other call can stay on
  screen until a new frame arrives.
- A meeting offer with both the data channel and the gallery slots has never been tried live.

## Branch status

| Feature | Branch | State |
|---|---|---|
| Add people, Hold/Resume, Transfer, Chat in 1:1 calls | feat/call-controls | DONE: 86a102a (calling + kelma call tests, clippy clean); merging into callee-offer (9 conflicting files, union) |
| Screen share: sending | feat/screen-share | DONE: c084fda (calling and kelma tests and clippy pass); behind KELMA_SCREEN_SHARE=1; not tried live |
| Viewing a remote share, and remote video in meetings | feat/remote-video | CODE DONE: 43d6e00, cargo check passes, tests written but not run; being integrated; keyframe on RTCP PLI not possible in webrtc-rs 0.21 (timer instead) |
| Profile photos in tiles, View modes (Gallery/Speaker/Focus) | feat/call-views | DONE: b360ba8, merged; 70 call tests pass |
| Live captions, start recording and transcription | feat/call-captions | Recording and transcription controls DONE (merge being tested). Captions: the toggle and the bot request work, but the caption text needs the media data channel (`x-data` SDP section, `main-channel`), which is not built yet. |
| Camera (send, self-view, 1:1 receive, accept with video) | feat/camera | CODE DONE: 112f03f, cargo check passes, tests written but not run; behind KELMA_CAMERA=1; meeting video receive not built (needs the controlVideoStreaming subscription) |
| Caption text over the media data channel (meetings) | feat/captions-channel | CODE DONE: 6a56342 (includes screen-share + call-captions); cargo check passes, 32 SDP and caption tests pass; to integrate |
| Call waiting: a second call during a call, "Hold and accept", switching between calls, "On hold" status | feat/call-waiting | CODE DONE: ef7dbab (contains call-controls + captions + views); cargo check passes; 11 UI tests written, not run |
| Calls page (history, speed dial, Call), voicemail, meeting-started banner, reactions request | feat/calls-page (kelma-ba) | DONE: 7346d2d (90 call/reminder tests and clippy pass); to merge after call-controls; voicemail and reactions UNVERIFIED |

## To test live in the morning

- **Call waiting:** A calls, Accept. Then B calls: the toast says "You're in a call", beeps, and
  offers Hold and accept / End and accept / Decline. With Hold and accept, A gets the "On hold"
  bar; Resume swaps the calls; leaving the shown call brings the held one back. A third call while
  one is held is not shown.

**Binary:** `/data/personal_projects/kelma-testbins/f96fc9b/kelma`. It has everything above,
built from feat/call-settings f96fc9b.

Run it with every opt-in feature and the call dump:

```
KELMA_CAMERA=1 KELMA_SCREEN_SHARE=1 KELMA_CALL_DUMP=/data/personal_projects/kelma-testbins/dump \
  /data/personal_projects/kelma-testbins/f96fc9b/kelma
```

Keep the phone or browser away from the laptop, or wear headphones, to avoid feedback. A good
order: 1:1 audio, then Hold, Chat, Transfer, add someone, call waiting, camera, share. After that,
a Meet now for the View menu, captions, recording and the remote share.

- **Photos:** in a call, the tiles, the People panel and the floating call show Teams profile
  photos instead of initials. Without a photo, initials stay. The first photo can take about 1 s.
  The large Speaker tile uses the 64 px chat avatar, so it may look soft.
- **View menu:** View › Gallery or Speaker. In Speaker, the person who spoke last is large and the
  others sit in a strip below; the large tile should switch when someone else talks. Focus on
  content is disabled ("Nobody is sharing").

- **Add someone:** People › "Invite someone or dial a number", then pick a person. Expect "Calling <name>…"; check that they ring and the call becomes a group call.
- **Hold:** Hold shows a banner and turns the button into Resume. The other side should see they're on hold.
- **Transfer:** Transfer holds the call. Pick a person, then Transfer now. Expect "You transferred the call to …".
- **Chat during a 1:1 call:** Chat opens the 1:1 chat and shrinks the call.

## UNVERIFIED / blocked

- **Remote video** (start kelma with `KELMA_CAMERA=1 KELMA_SCREEN_SHARE=1`): someone else's share
  takes the stage with "<name> is presenting", and meeting cameras show in their tiles through 9
  receive slots and `applyChannelParameters`. All of it is UNVERIFIED. If the meeting join is
  refused because of the extra video lines, start kelma with `KELMA_CAMERA=0` to send the old offer.
  After lobby admission the gallery isn't requested again.

- **Screen share, sending** (start kelma with `KELMA_SCREEN_SHARE=1`): Share → KDE's "Share screen"
  dialog → pick a screen → the other side should see it. UNVERIFIED: whether the media server
  accepts our join offer with a sharing line, the 1:1 re-offer answer, and H.264 profile matching
  with browsers. To check capture on its own:
  `cargo run -p calling --example screencast -- 5 /tmp/share.h264 screen`, then `ffprobe`.

- **Camera** (opt-in: start kelma with `KELMA_CAMERA=1`): it uses the Camera portal, falling back
  to V4L2 (`KELMA_CAMERA_DEVICE=/dev/video0`), and encodes with OpenH264. UNVERIFIED: whether Teams
  accepts our video line, whether the browser decodes our stream, and remote 1:1 video. In
  meetings, receiving video is NOT built; it needs `controlVideoStreaming` and source-id mapping.
  Test a 1:1 call first: press Camera, approve KDE's camera prompt, check the self-view and
  whether the browser sees you.

- **Add person** (UNVERIFIED): `POST addParticipant`. If Teams refuses it in a 1:1 call, the
  next shape to try is rusty-teams': create a group chat, then `addParticipantAndModality`.
- **Hold / Resume** (UNVERIFIED): a `mediaRenegotiation` re-offer, all lines `inactive` for hold,
  then the `mediaAcknowledgement`. If the re-offer is refused, webrtc-rs keeps a pending local
  offer (follow-up in media).
- **Transfer now** (UNVERIFIED): `POST transfer` with `callTransfer`. Consult-then-transfer is not
  built.

- **Recording, transcription and captions** all go through the Teams recorder bot. These are
  UNVERIFIED: whether kelma's sign-in can get the recorder token, whether 1:1 calls accept the bot,
  and the shape of a policy refusal. Test in a meeting: More › Record and transcribe › Start
  recording. Expect the red dot and the consent notice, or a red banner with the service's reason.
- **Caption text** (UNVERIFIED): the `x-data` SDP section maps to an SCTP data channel
  `main-channel`, and caption frames (data id 3) become caption lines. This is on only in meetings.
  Live check: the log shows `data channel open`, then More › Show live captions and someone talks.
