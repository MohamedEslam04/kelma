# kelma: plans to full Microsoft Teams parity

`PLAN.md` at the repository root is the finished MVP: chat, channels, files, images, live updates.
The files in this folder plan **everything else Teams does**, one file per app in Teams' left bar,
plus files for teams management and for settings and platform work.

## Where the content came from

- **Open-source clients.** About 35 projects were reviewed for documented internal endpoints, chiefly rusty-teams (MIT), mautrix-teams (AGPL), purple-teams (GPL), Jezza/ic3, eisbaw/ost (MIT) and m0nkmaster's API reference. Each plan ends with a table mapping its items to those endpoints, with source and license. Code from MIT/Apache projects can be reused with attribution. AGPL, "MIT + Microsoft exclusion" and unlicensed projects are used for facts only.
- **A survey of the real Teams web app** (`teams.cloud.microsoft`, 2026-11-18) with the user's own
  account. It covered every rail app, menu, hover bar, toolbar, dialog, settings page and the
  keyboard-shortcut list. Nothing was sent, saved or changed during it.
- **Research agents** read Microsoft Learn, support.microsoft.com, Message Center, Graph and ACS
  docs, and open-source clients. Their notes have source URLs, are in `docs/research/`
  and are summarized in each plan.
- **A token probe**, one refresh per audience with kelma's client id. It decides what can be
  native.

## The plans

| Plan | Teams app | Open items | Native? |
|---|---|---:|---|
| [`chat.md`](chat.md) | Chat (and channel posts) | 40 | Yes |
| [`activity.md`](activity.md) | Activity | 7 | Yes |
| [`calendar.md`](calendar.md) | Calendar | 17 | Yes |
| [`meetings.md`](meetings.md) | Meetings: join, in-meeting, options, recording, events | 26 | Data native; media native via the documented `epconv` protocol, browser or Chromium until then |
| [`calls.md`](calls.md) | Calls | 13 | Data native; calls native via `epconv`, browser or Chromium until then |
| [`onedrive.md`](onedrive.md) | OneDrive | 14 | Yes, except Office editing |
| [`copilot.md`](copilot.md) | Copilot | 9 | Native over an undocumented WebSocket, with a web fallback |
| [`meeting-recaps.md`](meeting-recaps.md) | Meeting Recaps | 7 | Files native; AI parts license-gated |
| [`store-and-apps.md`](store-and-apps.md) | Store, app bar, "…" apps (Planner, Engage, Workflows, Office…) | 12 | Mixed, decided per app |
| [`teams-and-channels.md`](teams-and-channels.md) | Teams and channels management, Communities | 14 | Mostly native after discovery |
| [`settings-and-platform.md`](settings-and-platform.md) | Settings, accounts and orgs, web host, auth broker, packaging | 14 | Yes |
| [`beyond-teams.md`](beyond-teams.md) | None: kelma's own features (catch-up, reminders, palette, Focus, offline search, snippets) | 1 | Yes, all local |

Each item has the same parts:
- a checkbox
- impact and effort scores (1–5)
- **What** (the Teams behavior, with Teams' own labels)
- **How** (endpoint, token and kelma component)
- **Done when** (the acceptance check)

Endpoints are marked **known** when kelma, purple-teams or Microsoft docs already show them, and
**discover** when they must be captured from the Teams web client first.

## What the token can reach

These are verified by minting each audience from kelma's refresh token (Teams web client id
`1fec8e78-…`) on 2026-11-18.

| Audience | Status | Used by |
|---|---|---|
| `api.spaces.skype.com`, `chatsvcagg`, `presence.teams.microsoft.com` | ✅ in use | chat, CSA, presence |
| Graph `.default` (Files, Sites, Calendars, Mail, Contacts, Tasks, Notes, Channel.Create, TeamMember, TeamsTab, AppCatalog, People, Place…) | ✅ | files, calendar, voicemail, teams, apps |
| `substrate.office.com` | ✅ | search, people suggestions |
| `substrate.office.com/sydney` | ✅ | Copilot |
| `outlook.office.com` (Calendars, EWS, Contacts) | ✅ | calendar extras, mailbox settings |
| `<tenant>.sharepoint.com`, `<tenant>-my.sharepoint.com` (Sites.FullControl.All) | ✅ | recycle bin, sharing, channel files |
| `api.yammer.com` | ✅ | Communities / Viva Engage |
| `service.flow.microsoft.com` | ✅ | Workflows |
| `tasks.office.com` | ✅ | Planner |
| `onenote.com` | ✅ | OneNote |
| `ic3.teams.office.com` | ✅ | calling backend (to explore) |
| `loki.delve.office.com` | ✅ | profile cards |
| Graph `OnlineMeetings.*`, `Presence.*`, `OnlineMeetingRecording/Transcript`, `CallRecords` | ❌ AADSTS65002 | use internal endpoints or files instead |

## Shared decisions

- **ADR 0002, meeting and call media.**
  - Microsoft does not document Teams calling, but open-source clients do. mautrix-teams has verified native audio, video and screen share, and rusty-teams and ost are MIT.
  - kelma builds a native `crates/calling` on that protocol (meetings M5, calls K3.2).
  - Until it works, the `MeetingLauncher` hands calls to the browser or Teams PWA, or to an on-demand Chromium window.
  - ACS stays optional.
- **ADR 0003, web host.**
  - One on-demand Chromium-based host with its own profile serves:
    - "W" apps (Approvals, Shifts, Whiteboard, Loop, Forms, Viva…)
    - Office web editing
    - meeting options
    - the Copilot fallback
  - A small TeamsJS shim lets app tabs initialize and get SSO tokens.
  - It exits when idle, so kelma's memory baseline (~30 MB idle) stays.
- **Auth broker** (`settings-and-platform.md` X3.2): every audience above goes through one cache with wall-clock expiry. This comes first because every app needs it.
- **Discovery rule.**
  - Capture an internal endpoint from the Teams web client before using it, and write it into `docs/protocol*.md` without tokens or message content.
  - Live tests only touch the user's own data, `48:notes` or a private test channel the user named, and are cleaned up afterwards.
- **Licensing.** Copilot and Premium features show a clear "not available for your license" state instead of breaking.

## Order across plans

1. Auth broker (X3.2), then chat phase 5, then Activity.
2. Calendar C1 and C3.1 (see and get reminded), then Meetings M1 (meeting chat, join through the browser).
3. Chat phase 6 (@mentions, new chat, presence) and OneDrive O1 and O2.
4. Web host (X3.1) and ADR 0002/0003; then Meetings M2, Calls K1 and K2, and the "W" apps.
5. Search, Copilot, Recaps, Store, teams management, settings and accounts.

The non-blocked items in `PLAN.md` stay the gate for the MVP. These plans start after it.
