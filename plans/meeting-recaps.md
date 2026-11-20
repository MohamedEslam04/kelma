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

- [ ] **R0.1** DEFERRED: needs test tenant (recaps need test meetings, not allowed on the work account). Capture the Recap app and a meeting's Recap tab: the list, notes, tasks, chapters, mentions, timeline and share. Document it in `docs/protocol-meetings.md`.

## Phase R1: recaps from files (no AI license needed)

- [ ] **R1.1 Recap list.** DEFERRED: needs test tenant (recaps need test meetings, not allowed on the work account). Impact 3, effort 3.
  - **What.**
    - "Good afternoon, <name>".
    - All and Followed.
    - Search for meetings.
    - List and grid views.
    - The past 30 days of meetings that have a recording or transcript.
    - Filters for date, people and keywords.
  - **How.** Calendar events (`plans/calendar.md`) joined with the `Recordings` files (OneDrive and channel libraries). Followed comes from R0.
- [ ] **R1.2 Recording player.** DEFERRED: needs test tenant (recaps need test meetings, not allowed on the work account). Impact 3, effort 3.
  - **What.**
    - Plays the recording (GStreamer or libmpv) with seek and speed.
    - Captions from the transcript.
    - A speaker timeline built from the VTT voice tags.
    - "Your mentions" markers, found where the transcript names you.
  - **How.** `/content` streams with HTTP range requests on the Graph token.
- [ ] **R1.3 Transcript pane.** DEFERRED: needs test tenant (recaps need test meetings, not allowed on the work account). Impact 3, effort 2.
  - **What.** Speaker, time and text; search; click to seek; download.

## Phase R2: AI recap (license-gated)

- [ ] **R2.1 Notes, tasks and chapters** DEFERRED: needs test tenant (recaps need test meetings, not allowed on the work account). through the R0 endpoint.
  - **What.** AI notes (titles, bullets, sub-points), suggested tasks with owners, chapters, and Ask Copilot (`plans/copilot.md`).
  - **Fallback.** Open the `recapUrl` deep link in the web host.
- [ ] **R2.2 Share recap.** DEFERRED: needs test tenant (recaps need test meetings, not allowed on the work account). Copy link and share by email; deleting the recap is for organizers only.
- [ ] **R2.3 Audio and video recap.** DEFERRED: needs test tenant (recaps need test meetings, not allowed on the work account). Play them when they exist (`Recordings/AudioRecaps`). Generating them stays in Teams.

## Status (2026-11-20, build and offline tests only)

A native Meeting Recaps screen (rail or "…": the installed Meeting Recaps app opens it) is built; it is not ticked because nothing was checked on a real tenant.
- **R1.1 list.** The last 30 days of meetings with a recording or transcript, a search box (subject, organizer), the newest first. From MCPS (UNVERIFIED audience and geo), else from the OneDrive `Recordings` files. Not built: All or Followed (no endpoint known), the grid view, filters for date and people, the greeting.
- **R1.2 recording.** "Play recording" opens the recording in the browser (SharePoint or Stream); kelma has no player, so no seek, speed, captions, speaker timeline or mentions.
- **R1.3 transcript.** Speaker, time and text of the WebVTT, shown as text. Not built: search, click to seek, download. The SharePoint `streamContent` path is UNVERIFIED; a `.docx` transcript is not read.
- **Meeting notes** link from the meeting chat's "Notes" tab (UNVERIFIED tab name). **Share** opens the OneDrive share dialog for the recording.
- **R2 AI recap.** Not built, by license: the screen says "Not available" (or "Available in Teams", with an Open link when the service sends a URL, if the meeting has an `AISummary`).
- Endpoints and their status: `docs/protocol.md`, "Meeting Recaps". Tests: `teams_proto::recaps`, `backend::appstore::recaps`, `meeting_recaps_list_meetings_with_a_recording_or_transcript_and_open_their_parts`.

## Order

R1.1, R1.3 and R1.2, then R0 and R2.

## Endpoints already documented by open-source clients

See `docs/research/open-source.md` §3.14. These remove most of R0.
- **Recordings and transcripts list:** MCPS `GET teams.microsoft.com/api/mcps/<geo>/user/meetings/contents?top=20&startDateTime&endDateTime&includeTypes=Recording,TranscriptV2`, and per meeting `…/contents/?threadId=&iCalUid=&organizerId=<tid>@<oid>`. The resource types include `AISummary`.
- **Transcript JSON:** substrate `GET /api/beta/me/WorkingSetFiles/?$filter=ItemProperties/Default/MeetingThreadId eq '<thread>'`, or SharePoint `_api/v2.1/drives/<d>/items/<i>/media/transcripts`.
- **AI notes and tasks:** substrate `POST /search/api/v1/recommendations?setflight=AiTasksV3,AiNotesV3,PeopleMentionsV2` (license-gated).
- **Sources:** rusty (MIT), mautrix OpenAPI, better-teams.
