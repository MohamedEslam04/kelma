# kelma plan: Calls

Part of the Teams parity plans (see `plans/README.md`). Format as in `plans/chat.md`.

**Sources.**
- UI survey of the Calls app and of Settings › Calls in Teams web (2026-11-18).
- `docs/research/calls.md` (Microsoft Learn call flows, ACS docs, community findings; no public reverse engineering of Teams calling exists).
- The token probe. Graph `Mail.ReadWrite` is available, and so is an `ic3.teams.office.com` token (`Teams.AccessAsUser.All`), the backend audience the Teams client uses for calling.

**What the user's account sees today.**
- No dial pad: there is no Teams Phone license, so calls are Teams-to-Teams only.
- History with filters All / Missed / Incoming / Outgoing / Voicemail.
- Speed dial, View contacts, forwarding (Don't forward / Forward to voicemail), and a device menu.

**Media.** Media follows the same decision as meetings (`plans/meetings.md`, ADR 0002).
- 1:1 and group calls use the same `epconv` protocol, which open-source clients document and implement:
  - mautrix-teams (verified incoming and outgoing 1:1 calls)
  - rusty-teams and ost (MIT)
- The plan is a native call stack, with browser and Chromium hand-off until it lands.

## Phase K1: native call data

- [ ] **K1.1 Calls view and history.** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work). Impact 4, effort 3.
  - **What.**
    - A Calls rail item.
    - The left column has "Type a name" plus Call, and Speed dial.
    - The centre is History, with filters All, Missed, Incoming, Outgoing, Voicemail and Filter.
    - Each row shows the avatar, name, direction icon (incoming, outgoing, missed in red), date and duration.
    - Each row's menu: Call back, Chat, Remove from view, Add to speed dial, Add contact, Report call.
  - **How.**
    - Call logs are the system thread `48:calllogs` (parsed in rusty `calllog.rs`): `GET <chatService>/v1/users/ME/conversations/48:calllogs/messages`, or the CSA path `/api/csa/{region}/api/v1/chats/48%3Acalllogs/messages`. The JSON is in `properties['call-log']` (callDirection, callState, startTime, endTime, participants, callType).
    - New entries arrive over trouter like any chat message.
    - Check how missed calls are represented against the real history.
  - **Done when.** The last 30 entries match Teams, missed calls included.
- [ ] **K1.2 Speed dial and contacts.** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work). Impact 3, effort 2.
  - **What.**
    - Speed dial: add, remove and reorder people, shown with presence.
    - Contacts: contact groups and Add contact.
    - "View contacts" opens a contacts list.
  - **How.**
    - Contacts: Outlook contacts through Graph `/me/contacts` (`Contacts.ReadWrite.Shared` is granted) and the Teams buddy list (`/v1/users/ME/contacts`, known from purple-teams).
    - Speed dial is stored where Teams keeps it (discover); otherwise keep it locally.
- [ ] **K1.3 Voicemail.** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work). Impact 3, effort 3.
  - **What.**
    - The Voicemail filter lists messages with caller, time, duration and transcript.
    - Play at 1×, 1.5× or 2×, Call back, Delete, Mark as read.
  - **How.**
    - Voicemails are Exchange mail with message class `IPM.Note.Microsoft.Voicemail.UM*`.
    - List them with Graph `/me/messages` filtered on `singleValueExtendedProperties` `String 0x001A` (`Mail.ReadWrite` is granted).
    - The audio is the mp3 attachment and the transcript is the body.
    - Play the audio through GStreamer or the system player.
  - **Done when.** A test voicemail left by the user from their own phone (or an existing one) plays.
- [ ] **K1.4 Call settings.** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work). Impact 3, effort 3. Seen: Settings › Calls.
  - **What.**
    - Call handling and forwarding: forward all calls to voicemail; when you can't answer, redirect to voicemail, a person or a number; ring time (default 20 s); also ring (simultaneous ring) a contact, call group or delegates; call groups; delegates.
    - Ringtone with preview.
    - Voicemail: record a greeting, greeting language, what happens on redirect, text-to-speech greeting.
    - Out-of-office voicemail: all the time or when Outlook auto-replies are on, with a custom greeting.
    - The quick forwarding menu in the Calls view.
  - **How.** Discover the Teams middle-tier user calling settings endpoint. Its fields mirror `Get-CsUserCallingSettings`: ForwardingType, ForwardingTargetType, UnansweredTargetType, UnansweredDelay, Delegates, CallGroup. Make the UI read-only where the license does not allow a change.
- [ ] **K1.5 Incoming call notification.** BLOCKED: answering an incoming 1:1 call needs the callee-side media negotiation in crates/calling, which this wave must not change (fix on feat/calls-callee-offer; the ring itself already works live). Impact 5, effort 3.
  - **What.** A toast and an in-app incoming-call window with the caller's photo and name, and Accept with video, Accept with audio, Decline. The ringtone plays. It honors Do not disturb.
  - **How.**
    - The invite arrives over the `NextGenCalling` trouter registration as a `callInvitation` with `links` (documented in `docs/research/open-source.md` §3.15).
    - Decline: `POST` to the invitation's rejection link.
    - Accept: the native stack (K3.2), or the browser or Chromium host until it lands.
  - **Done when.** A colleague's test call rings in kelma; Decline works; Accept opens the host connected to the call.
  - **Evidence (2026-11-19, feat/calls-direct).** built; live check pending with tester. The incoming-call toast (`ui/call_incoming.slint`, `backend/call_direct.rs`): caller name and avatar, Accept with video (answers with audio, with a note), Accept with audio, Decline; a generated ringtone through cpal; a desktop notification when the window is not focused; closes when the caller cancels. Answering attaches, rings, accepts with the SDP answer and hands the call to the call manager. CLI: `kelma-cli call listen --ring-only|--decline|--accept-audio`. Tests use a fake backend; no live call was placed or answered. kelma adds no missed-call entry of its own: the service writes it to `48:calllogs`, which `calling::history` only reads.

## Phase K0: discovery (before K1.5 and K2)

  - **Evidence (live, 2026-11-19).** A call from the guest account rang kelma: the toast and the desktop notification showed the caller. Accepting failed: the invitation carries no SDP (`mediaContent: null`, `links.mediaAnswer: "cc://ma"`), so the callee must send the offer with `attach` and take the answer from the mediaAnswer callback. Built on feat/calls-callee-offer; live check next.
- [ ] **K0.1 HAR capture** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work). of the Teams web client for:
  - an outgoing 1:1 audio call (to the user's own second device or a willing colleague)
  - accepting and declining an incoming call
  - hold, transfer, adding a person, DTMF, and the settings changes

  Sanitize it and document it in `docs/protocol-calls.md`.
- [ ] **K0.2 ic3 audience.** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work). Check what the `ic3.teams.office.com` token unlocks (likely the call controller and roster). Record which parts are REST signaling and which are media.

## Phase K2: calls with media (Chromium host)

- [x] **K2.1 Outgoing calls.** Impact 5, effort 4. Native, not through the Chromium host; video and group calls stay with K2.2 and M5.2.
  - **What.** The audio and video buttons in the chat header and the Call button in Calls start a call in the Chromium host (deep link `https://teams.microsoft.com/l/call/0/0?users=<upn>&withVideo=true`). Group calls work from group chats.
  - **Done when.** A 1:1 call with a colleague connects both ways.
  - **Evidence (2026-11-19, feat/calls-direct).** built; live check pending with tester. Natively, not through the host: Call (audio) and a disabled Video button in a 1:1 chat's header, Call on the profile card, and the ringing screen ("Calling…", "Ringing…", Cancel; Declined / No answer / Call ended). On answer the call goes to the call manager. CLI: `kelma-cli call dial <mri|email>`. Tests use a fake backend; no live call was placed.
  - **Evidence (live, 2026-11-19).** The user's own accounts: the personal account as a guest in the work tenant (kelma-sandbox), the work account in kelma. `kelma-cli call dial <MRI>` and the GUI's Audio call in the 1:1 chat header both rang the phone, connected, and carried audio both ways (CLI: 3510 packets sent, 1579 received; roster 2). The GUI call had echo on the far side: the GUI audio path lacked AEC, fixed on feat/calls-callee-offer.
- [ ] **K2.2 In-call controls** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work). (verified in the host): mute, camera, share screen (Wayland portal), hold and resume, transfer (now, or consult first), add people, keypad, record and transcribe (policy), live captions, devices, video effects, chat during the call (the kelma chat window stays usable), convert to a meeting.
- [ ] **K2.3 Device settings, natively:** DEFERRED: not built (the test call and the camera rows wait for video in native calls; speaker and microphone selection, noise suppression and echo cancellation are done).
  - speaker, microphone and camera selection through PipeWire
  - "Make a test call" (`8:echo123`)
  - camera preview, mirror, and low-light brightness

  They mirror Settings › Devices and are passed to the host.
  - **Status.** Speaker and microphone come from Settings › Devices (X1.4, with live evidence in
    `plans/settings-and-platform.md`). Settings › Devices › Call audio adds Noise suppression and Echo
    cancellation switches, saved with the device prefs (on by default). The call microphone path reads them
    each time the devices (re)start: `AudioProcessor::with_options(ProcessingOptions)`, and `KELMA_NO_AEC`
    still turns echo cancellation off. Tests: `devices` prefs defaults and old files, `calling`
    `options_switch_echo_cancellation_and_noise_suppression`, `kelma` `the_device_switches_shape_the_mic_processing`
    and the UI test `call_audio_switches_start_on_and_save_with_the_device_prefs`. Not compared live by ear yet.
  - **"In a call" presence.** While a call or meeting is connected, kelma publishes its device presence as
    Busy / `InACall` (`InAConferenceCall` in a meeting), and Available when it ends
    (`backend/call_presence.rs`, from the call manager). Evidence (2026-11-19, live, the user's own work account):
    `kelma-cli presence endpoint Busy InACall` read back Busy / InACall, `presence endpoint Available Available`
    read back Available, and `presence remove-endpoint` removed the test device (back to Away). Away is refused
    for a device that reports activity by transport (HTTP 400), so leaving a call publishes Available.

## Phase K3: native calls, ACS and safety

- [ ] **K3.1 ACS calling** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work). (see meetings M4): a kelma-branded call UI on ACS for consenting tenants. No park, merge or forwarding support.
- [ ] **K3.2 Native calls.** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work). Use `crates/calling` from meetings M5 for 1:1 and group calls.
  - **Outgoing:** `epconv` with `participants.to`.
  - **Incoming:** the trouter `callInvitation`, then `POST links.acceptance` with the accepted modalities. Decline goes to `rejection`; hang up to `end`.
  - **During the call:** hold, transfer and add participants through the links.
  - Impact 5, effort 4.
  - **Evidence (2026-11-19, feat/calls-direct).** built; live check pending with tester. 1:1 incoming (attach, ringing, accept, decline) and outgoing (`place_call`, progress, acceptance, cancel) are wired to `crates/calling` in the GUI and in `kelma-cli call listen` / `call dial`; group calls, hold, transfer and adding people are not.
- [ ] **K3.3 Emergency numbers.** DEFERRED: not built (the GUI dial path has no phone-number dialling yet, so its guard waits for it; the check and the CLI guard are done). kelma is not a compliant emergency endpoint, so block 911/112/122 and similar, and tell the user to use a phone.
  - **Status.** `calling::emergency::is_emergency_number` matches emergency short codes (911, 112, 999, 000, 110,
    122, 123, 180 and others) on the whole dialled number, ignoring spaces, dashes and brackets; a longer number or
    one with `+`/`00` never matches. `kelma-cli call dial` refuses them with `calling::emergency::REFUSAL`
    ("kelma can't place emergency calls. Use a phone …"). Unit tests in `crates/calling/src/emergency.rs`.

## Order

K1.1, K1.5 and K0 first (calls that ring), then K2.1 (calls through the browser or Chromium host), then K3.2 together with meetings M5 (native calls), then the rest. K3.1 only on request.
