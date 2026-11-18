# kelma plan: Meetings (join, in-meeting, options, recording, recap, events)

Part of the Teams parity plans (see `plans/README.md`). Format as in `plans/chat.md`.

**Sources.** The Calendar UI survey (event peek, Join ▾, Meet now, New ▾ templates), the research in
`docs/research/calendar-meetings.md` (Microsoft Learn, ACS and Graph docs, the open-source
landscape), and the token probe.

## Where native media stands

**Microsoft does not document the Teams meeting and calling protocol, but open-source clients have
worked it out.** See `docs/research/open-source.md` §3.15.
- **mautrix-teams** (AGPL, Go) has verified audio both ways, video, screen share, meetings, and 1:1
  incoming and outgoing calls.
- **rusty-teams** (MIT, Rust) implements the signaling, roster, captions and recording control.
- **eisbaw/ost** (MIT, Rust) has the signaling and an SDP compressor.

The protocol:
1. A second trouter registration (`NextGenCalling`).
2. One `POST https://api-<geo>.flightproxy.teams.microsoft.com/api/v2/epconv` with an **IC3**
   bearer. kelma's token already mints the `ic3.teams.office.com` audience.
3. Everything after that POSTs to the `links` the server returns: mute, admit, roster, leave.
4. Media is standard WebRTC/ICE with an `application/sdp-ngc-1.0` SDP blob. The TURN credentials
   come from `GET teams.microsoft.com/trap/tokens`.

**Decision (ADR 0002, to write in M0).**
1. **Native data:**
   - calendar and invites
   - the meeting chat (a normal thread, `19:meeting_…@thread.v2`)
   - start and end events in the chat
   - recordings and transcripts (MCPS, files)
   - meeting options (`MT/beta/meetings/v1/options/…`, native, documented by rusty)
   - the recap
2. **Media:** the `MeetingLauncher` trait has four implementations:
   - `Browser`, the default at first: opens the join link in the browser or the Teams PWA. Full parity, no cost.
   - `Native`: kelma's own call stack (`crates/calling`) on `epconv` and WebRTC (`webrtc-rs`, or libwebrtc through bindings), following rusty-teams' MIT code. It becomes the default once M5 passes.
   - `ChromiumWindow`: Teams web in an on-demand Chromium window, for features the native stack lacks (breakout rooms, Whiteboard, PowerPoint Live, apps).
   - `Acs`: optional; only if a tenant wants a supported API.

## Phase M0: decisions and discovery

- [ ] **M0.1 ADR 0002 "meeting media host".** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). Compare the Browser, ChromiumWindow (CEF or Chromium `--app`), ACS and native options on:
  - feature parity
  - memory (kelma's budget)
  - Wayland screen share (PipeWire portal)
  - terms of service and maintenance cost

  Record the choice.
- [ ] **M0.2 HAR capture** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). of the Teams web join, meeting options page, recap tab and attendance, saved sanitized to `docs/protocol-meetings.md` with no tokens. This shows which first-party endpoints the web client uses beyond Graph.

## Phase M1: native meeting data

- [ ] **M1.1 Meeting chat.** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). Impact 5, effort 2.
  - **What.**
    - Meeting chats appear in the chat list under the "Meeting chats" filter.
    - They open with the meeting title, a Join banner while the meeting is live, and a Recap tab afterwards.
    - "Meeting started", "Meeting ended · 45m" and "Recording started" show as system lines.
  - **How.**
    - The thread id comes from the join URL.
    - The events are thread messages (`Event/Call`, `RecordingStarted`…); kelma already parses `ThreadActivity/*`.
    - Map them to rich system rows.
  - **Done when.** A real meeting chat shows its start and end lines and the Join banner.
- [ ] **M1.2 Join entry points.** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). Impact 5, effort 2.
  - **What.** Join from:
    - the calendar peek and its reminder toast
    - the chat banner and the Activity item
    - a pasted `teams.microsoft.com/l/meetup-join/…` link
    - "Join with an ID" (meeting ID and passcode)

    "Copy join link" is also available.
  - **How.** Each one goes through `MeetingLauncher`.
- [ ] **M1.3 Meet now.** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). Impact 4, effort 2.
  - **What.**
    - From the calendar: Meet now, then "Get a link to share" or "Start meeting".
    - From a chat header: "Meet now" in the chat, which invites the chat members.
  - **How.**
    - Calendar: an instant event with `isOnlineMeeting`.
    - Chat: discover (the chat-header "Meet now").
- [ ] **M1.4 Recordings and transcripts.** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). (Known: MCPS `api/mcps/<geo>/user/meetings/contents`, see `plans/meeting-recaps.md`.) Impact 4, effort 3.
  - **What.** For your meetings:
    - recordings with play (system player, or the Chromium host for Stream), share link and download
    - transcripts with speaker names and timestamps, searchable, with jump to a time
  - **How.**
    - Graph `OnlineMeetingRecording/Transcript.*` need admin consent, so they are not usable.
    - Recordings are files in the organizer's OneDrive `Recordings` folder (or the channel's SharePoint), reached with `Files.ReadWrite.All`.
    - Transcripts are `.vtt` or `.docx` beside them, or come from the recap endpoint (M0.2).
- [ ] **M1.5 Attendance report.** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). Impact 2, effort 3.
  - **What.** For meetings you organized: participants, join and leave times, duration and role, with CSV export.
  - **How.** Graph attendanceReports needs `OnlineMeetingArtifact.Read.All`, which is not granted, so use the endpoint found in M0.2.
- [ ] **M1.6 Meeting options.** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). Native via `GET/POST MT/beta/meetings/v1/options/<tenant>/<organizer>/<thread>/0?monarch=true` (rusty `meeting_options.rs`); the web page is only the fallback. Impact 3, effort 2.
  - **What.** The organizer's "Meeting options" button opens the options page with every option listed in the research:
    - lobby
    - who can present
    - attendee mic and camera
    - chat mode
    - reactions
    - Q&A
    - record automatically
    - who can record
    - watermark and E2EE (Premium)
    - Copilot settings
  - **How.** `https://teams.microsoft.com/meetingOptions/?organizerId&tenantId&threadId&messageId=0` in the Chromium host. Graph PATCH is not possible (OnlineMeetings is refused).
- [ ] **M1.7 Recap.** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). See `plans/meeting-recaps.md`.

## Phase M2: meetings in the Chromium host (full parity)

Everything Teams web has, inside a kelma-owned window. Each item is "verify it works in the host", not "build it".
- [ ] **M2.1 Host window.** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). Impact 5, effort 4.
  - **What.**
    - Dedicated profile.
    - Sign-in with the same account (the user signs in once in the host; kelma never handles the password).
    - Open the join URL and close on hang-up.
    - The window title shows the meeting.
    - kelma's chat for the meeting stays usable side by side.
  - **Done when.**
    - Joining a real meeting works.
    - Camera, mic and speaker work.
    - The process exits after leaving.
    - Memory is back to kelma's baseline afterwards.
- [ ] **M2.2 Pre-join screen:** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). name, camera and mic toggles, device pickers, test call, background effects, noise suppression, audio options (computer, phone "call me", room, no audio), join or ask to join, lobby waiting.
- [ ] **M2.3 In-meeting controls:** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). Chat, People, Raise, React, View, Notes, Apps, More, Camera, Mic, Share, Leave or End for everyone; push-to-talk (Space); keyboard shortcuts.
- [ ] **M2.4 Screen share on Wayland.** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). Screen, window and tab sharing through the xdg-desktop-portal and PipeWire, including "include computer sound", presenter modes and give or take control.
- [ ] **M2.5 Participants:** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work).
  - lists of presenters and attendees, Invited, Lobby, Raised hands
  - mute, mute all, disable mic or camera
  - spotlight, pin, roles (presenter, attendee, co-organizer)
  - remove, admit or deny, lock the meeting
  - add people, copy join info, call someone by phone
- [ ] **M2.6 Views:** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). Dynamic, Gallery, Large gallery, Speaker, Focus on content, gallery at top, swap content. Together mode was retired in June 2026 and is not built.
- [ ] **M2.7 Breakout rooms, polls, Q&A, Whiteboard, PowerPoint Live, Loop notes, apps in meetings, language interpretation.** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work).
- [ ] **M2.8 Captions, transcription and recording controls:** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). live captions with language and translation, start transcription, start and stop recording, consent prompts.
- [ ] **M2.9 Copilot in meetings and Facilitator** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). (license-gated).
- [ ] **M2.10 Companion mode, transfer to another device, dial-in information, "Call me".** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work).

## Phase M3: webinars, town halls, virtual appointments

- [ ] **M3.1 Attend:** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). join webinars and town halls through the launcher, and register from an invite link in the host.
- [ ] **M3.2 Create and manage:** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). the New ▾ templates (Webinar, Town hall, Virtual appointment, Live Event) open their web forms in the host. Graph `VirtualEvent.*` needs admin consent.

## Phase M4: optional ACS mode (only with a product reason)

- [ ] **M4.1** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). A kelma-branded call UI in a Chromium page on the ACS Calling SDK with the Teams-user identity. It needs:
  - a kelma Entra app with `Teams.ManageCalls` and `Teams.ManageChats`
  - admin consent per tenant
  - an ACS resource and a token-exchange service

  Document the feature gaps in the UI.

## Phase M5: native meeting media

- [ ] **M5.1 Calling crate.** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). Impact 5, effort 5.
  - **What.** A `crates/calling` crate covering:
    - trouter `NextGenCalling` registration
    - `epconv` create and join with `meetingInfo{tenantId, organizerId}`
    - the link-driven state machine (keepAlive, leave, mute, admit…)
    - TURN tokens from `trap/tokens`
    - ICE, DTLS/SDES-SRTP and Opus through `webrtc-rs`
  - **How.** Follow rusty-teams `crates/calling` (MIT; reuse with attribution) and the facts from mautrix-teams `calls.go` (AGPL; facts only).
  - **Done when.** kelma joins a real meeting and audio flows both ways.
- [ ] **M5.2 Video and screen share:** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). H.264/VP8 send and receive, and screen capture through the PipeWire portal. The UI shows the gallery and speaker views in Slint (frames rendered as textures).
- [ ] **M5.3 Roster and controls:** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). participants, mute and unmute, raise hand, reactions, spotlight, admit from the lobby, captions, and recording start and stop (bot commands to the roster URL).
- [ ] **M5.4 Switch the default launcher to Native** DEFERRED: needs test tenant (meetings are not allowed on the work account; native media is owned by the parallel calling work). once M5.1–M5.3 pass. ChromiumWindow stays for breakout rooms, Whiteboard, PowerPoint Live and apps.

## Order

M0, then M1.1–M1.3 (they make meetings feel native with no media work), then M2.1 with the Browser launcher as the default. ChromiumWindow after the ADR. M5 after M2.1, because it replaces the browser hand-off with kelma's own call window. M4 only on request.
