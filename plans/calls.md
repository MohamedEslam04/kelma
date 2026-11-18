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
- [ ] **K1.5 Incoming call notification.** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work). Impact 5, effort 3.
  - **What.** A toast and an in-app incoming-call window with the caller's photo and name, and Accept with video, Accept with audio, Decline. The ringtone plays. It honors Do not disturb.
  - **How.**
    - The invite arrives over the `NextGenCalling` trouter registration as a `callInvitation` with `links` (documented in `docs/research/open-source.md` §3.15).
    - Decline: `POST` to the invitation's rejection link.
    - Accept: the native stack (K3.2), or the browser or Chromium host until it lands.
  - **Done when.** A colleague's test call rings in kelma; Decline works; Accept opens the host connected to the call.

## Phase K0: discovery (before K1.5 and K2)

- [ ] **K0.1 HAR capture** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work). of the Teams web client for:
  - an outgoing 1:1 audio call (to the user's own second device or a willing colleague)
  - accepting and declining an incoming call
  - hold, transfer, adding a person, DTMF, and the settings changes

  Sanitize it and document it in `docs/protocol-calls.md`.
- [ ] **K0.2 ic3 audience.** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work). Check what the `ic3.teams.office.com` token unlocks (likely the call controller and roster). Record which parts are REST signaling and which are media.

## Phase K2: calls with media (Chromium host)

- [ ] **K2.1 Outgoing calls.** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work). Impact 5, effort 4.
  - **What.** The audio and video buttons in the chat header and the Call button in Calls start a call in the Chromium host (deep link `https://teams.microsoft.com/l/call/0/0?users=<upn>&withVideo=true`). Group calls work from group chats.
  - **Done when.** A 1:1 call with a colleague connects both ways.
- [ ] **K2.2 In-call controls** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work). (verified in the host): mute, camera, share screen (Wayland portal), hold and resume, transfer (now, or consult first), add people, keypad, record and transcribe (policy), live captions, devices, video effects, chat during the call (the kelma chat window stays usable), convert to a meeting.
- [ ] **K2.3 Device settings, natively:** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work).
  - speaker, microphone and camera selection through PipeWire
  - "Make a test call" (`8:echo123`)
  - camera preview, mirror, and low-light brightness

  They mirror Settings › Devices and are passed to the host.

## Phase K3: native calls, ACS and safety

- [ ] **K3.1 ACS calling** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work). (see meetings M4): a kelma-branded call UI on ACS for consenting tenants. No park, merge or forwarding support.
- [ ] **K3.2 Native calls.** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work). Use `crates/calling` from meetings M5 for 1:1 and group calls.
  - **Outgoing:** `epconv` with `participants.to`.
  - **Incoming:** the trouter `callInvitation`, then `POST links.acceptance` with the accepted modalities. Decline goes to `rejection`; hang up to `end`.
  - **During the call:** hold, transfer and add participants through the links.
  - Impact 5, effort 4.
- [ ] **K3.3 Emergency numbers.** DEFERRED: needs test tenant (calls are not allowed on the work account; native calling is owned by the parallel calling work). kelma is not a compliant emergency endpoint, so block 911/112/122 and similar, and tell the user to use a phone.

## Order

K1.1, K1.5 and K0 first (calls that ring), then K2.1 (calls through the browser or Chromium host), then K3.2 together with meetings M5 (native calls), then the rest. K3.1 only on request.
