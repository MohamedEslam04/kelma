# kelma plan: Meeting Recaps

Part of the Teams parity plans (see `plans/README.md`). Format as in `plans/chat.md`.

**Sources.**
- UI survey of the Meeting Recaps app (All / Followed, search for meetings, list or grid view, pin to the app bar).
- `docs/research/apps-onedrive-copilot-store.md` §3: there is a Recap app (MC1404322) and a per-meeting Recap tab.
- The token probe: Graph `OnlineMeeting*` is refused; files work through Graph and SharePoint.

**Licensing.**
- Recordings and transcripts: for everyone, by policy.
- AI notes, tasks, chapters, mentions and the speaker timeline: Teams Premium or Copilot.
- Audio and video recaps: Copilot.

kelma shows what the user's license allows and labels the rest.

## Phase R0: discovery

- [ ] **R0.1** Capture the Recap app and a meeting's Recap tab: the list, notes, tasks, chapters, mentions, timeline and share. Document it in `docs/protocol-meetings.md`.

## Phase R1: recaps from files (no AI license needed)

- [ ] **R1.1 Recap list.** Impact 3, effort 3.
  - **What.**
    - "Good afternoon, <name>".
    - All and Followed.
    - Search for meetings.
    - List and grid views.
    - The past 30 days of meetings that have a recording or transcript.
    - Filters for date, people and keywords.
  - **How.** Calendar events (`plans/calendar.md`) joined with the `Recordings` files (OneDrive and channel libraries). Followed comes from R0.
- [ ] **R1.2 Recording player.** Impact 3, effort 3.
  - **What.**
    - Plays the recording (GStreamer or libmpv) with seek and speed.
    - Captions from the transcript.
    - A speaker timeline built from the VTT voice tags.
    - "Your mentions" markers, found where the transcript names you.
  - **How.** `/content` streams with HTTP range requests on the Graph token.
- [ ] **R1.3 Transcript pane.** Impact 3, effort 2.
  - **What.** Speaker, time and text; search; click to seek; download.

## Phase R2: AI recap (license-gated)

- [ ] **R2.1 Notes, tasks and chapters** through the R0 endpoint.
  - **What.** AI notes (titles, bullets, sub-points), suggested tasks with owners, chapters, and Ask Copilot (`plans/copilot.md`).
  - **Fallback.** Open the `recapUrl` deep link in the web host.
- [ ] **R2.2 Share recap.** Copy link and share by email; deleting the recap is for organizers only.
- [ ] **R2.3 Audio and video recap.** Play them when they exist (`Recordings/AudioRecaps`). Generating them stays in Teams.

## Order

R1.1, R1.3 and R1.2, then R0 and R2.

## Endpoints already documented by open-source clients

See `docs/research/open-source.md` §3.14. These remove most of R0.
- **Recordings and transcripts list:** MCPS `GET teams.microsoft.com/api/mcps/<geo>/user/meetings/contents?top=20&startDateTime&endDateTime&includeTypes=Recording,TranscriptV2`, and per meeting `…/contents/?threadId=&iCalUid=&organizerId=<tid>@<oid>`. The resource types include `AISummary`.
- **Transcript JSON:** substrate `GET /api/beta/me/WorkingSetFiles/?$filter=ItemProperties/Default/MeetingThreadId eq '<thread>'`, or SharePoint `_api/v2.1/drives/<d>/items/<i>/media/transcripts`.
- **AI notes and tasks:** substrate `POST /search/api/v1/recommendations?setflight=AiTasksV3,AiNotesV3,PeopleMentionsV2` (license-gated).
- **Sources:** rusty (MIT), mautrix OpenAPI, better-teams.
