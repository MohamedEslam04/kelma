# kelma plan: Store, app bar and built-in apps

Part of the Teams parity plans (see `plans/README.md`). Format as in `plans/chat.md`.

**Sources.**
- UI survey (2026-11-18):
  - the Store: Home, Featured ▾, Categories ▾, Devices ▾, search "agents and apps", settings gear, Manage your apps, "Popular on Teams", "Trending with your peers"
  - an app page: Add, share, rating, installs, works in, price, screenshots, privacy, terms and permissions
  - the "View more apps" (…) flyout with search
- `docs/research/apps-onedrive-copilot-store.md` §4.
- The token probe. These all mint:
  - Graph `AppCatalog.Read.All`, `TeamsTab.*`, `Tasks.ReadWrite`, `Notes.ReadWrite.All`, `Sites.ReadWrite.All`
  - `tasks.office.com` (Planner)
  - `onenote.com`
  - `api.yammer.com` (Viva Engage)
  - `service.flow.microsoft.com` (Workflows)
  - SharePoint

  Graph `TeamsAppInstallation.ReadWrite*` is **not** available.

**How apps run in kelma.** Three classes, decided per app:
- **N (native):** a kelma screen over an API the token reaches.
- **W (web host):** the app's tab URL in the Chromium/webview host, with a small TeamsJS host shim (`app.initialize`, `getContext`, `authentication.getAuthToken` through the refresh token, `openLink`). See ADR 0003 in `plans/README.md`.
- **B (browser):** the deep link `https://teams.microsoft.com/l/entity/{appId}/{entityId}` opens in the system browser.

## Phase S1: app bar and flyout

- [ ] **S1.1 App bar.** Impact 4, effort 2.
  - **What.**
    - The left rail has Activity, Chat, Calendar, Calls, OneDrive, Copilot, Meeting Recaps, Store and "…".
    - Pin and unpin (right click), drag to reorder, collapse the app bar.
    - Shortcuts Ctrl+Shift+1…9.
    - Badges (Activity count).
  - **How.**
    - Discover the user's pinned-apps setting (Teams middle tier); otherwise keep the list in local config.
    - Admin-pinned apps come from the setup policy (discover).
- [ ] **S1.2 "View more apps" flyout.** Impact 3, effort 2.
  - **What.** "Search apps", then the installed list. Seen: Linear, Avatars, Shifts, Planner, OneNote, Help, Clipchamp, Whiteboard, Events, People, Visio, Word, Excel, PowerPoint, Loop, Workflows, Approvals, Viva Learning, Updates, Power BI, SharePoint, Viva Insights. Opening an app shows it in the main area.
  - **How.** The installed apps come from the Teams middle tier (discover; Graph `ReadForUser` is not granted).

## Phase S2: Store

- [ ] **S2.1 Browse and search.** Impact 3, effort 3.
  - **What.**
    - Home with hero search, "Popular on Teams", "Trending with your peers" and See all.
    - The Featured and Categories menus.
    - Devices (certified hardware) links out.
    - Filter by works-in (personal, chat, team, meeting) and agent or app.
  - **How.** Graph `GET /appCatalogs/teamsApps` (granted) for the catalog. Categories, ratings and screenshots come from the Teams internal store service (discover).
- [ ] **S2.2 App page.**
  - **What.** Icon, name and publisher; Add ▾ (Add for me, to a team, to a chat, to a meeting); Share; rating; installs; works in; price; screenshot carousel; description; privacy, terms and permissions (RSC) links.
- [ ] **S2.3 Install and uninstall.** Impact 3, effort 3.
  - **What.** Add for me, to a chat, to a team or to a meeting; Manage your apps (installed apps, updates, Remove).
  - **How.** Use the internal install endpoint the web client calls (discover). If that is blocked, "Open in Teams web to install".
- [ ] **S2.4 Store settings** (gear): app permissions per app (notifications, location, media, files, external links). Local grants are used by the web host.

## Phase S3: built-in apps, one by one

| App | Class | What kelma does |
|---|---|---|
| Planner / Tasks | **N** | Plans and tasks: board, list, charts, My tasks, To Do lists. Uses Graph `/me/planner/tasks`, `/planner/plans` and `/me/todo/lists` plus the `tasks.office.com` token for group plans. Premium plans go to the web host. |
| OneNote | N (read) + W (edit) | Notebooks, sections and pages read through `onenote.com` and Graph. Editing goes to the web host. |
| Lists | **N** | SharePoint lists: items, views and columns through Graph `/sites/{id}/lists`; forms to add and edit items. |
| Word, Excel, PowerPoint, Visio | W | Open documents in Office web in the host; "New" uses the OneDrive plan. |
| SharePoint | N (pages list) + W | Followed sites, news and pages; reading pages goes to the web host. |
| Viva Engage / Communities | N (feed) + W | Communities list, feeds, posts and replies through the `api.yammer.com` token; the full experience goes to the web host. The "Communities" section in the chat list also uses this. |
| Workflows | N (list) + W | Your flows, run history, turn on and off, and run a manual flow through `service.flow.microsoft.com`. Building a flow goes to the web host. "Workflows" in the message and chat menus triggers flows. |
| Approvals | W | Web host (Graph approvals scope not granted). |
| Shifts | W | Web host (`Schedule.*` not granted). |
| Whiteboard | W | Web host; whiteboards are listed from OneDrive. |
| Loop | W | Web host; Loop components in chat render as links to it. |
| Forms, Polls | W | Web host; poll cards in chat render as adaptive cards (chat plan 8.8). |
| Viva Learning, Viva Insights, Updates, Events, Help, Clipchamp, Avatars, Power BI, People, Linear and third-party apps | W / B | Web host when the tab URL works with the shim, otherwise the browser. |

- [ ] **S3.1 Planner and Tasks**, native. Impact 4, effort 4.
  - Data layer done 2026-11-19, no screen yet (`teams_proto::planner`, `kelma-cli tasks`): To Do lists live (200, 1 list); Planner endpoints 200 but 0 plans and 0 tasks on this tenant, so those parsers rest on synthetic fixtures. The `tasks.office.com` token has no usable path (404/503 after 3 attempts); Graph is used instead.
- [ ] **S3.2 Viva Engage / Communities**, native feed. Impact 3, effort 4.
- [ ] **S3.3 Workflows**, native list and run. Impact 3, effort 3.
  - Data layer done 2026-11-19, no screen yet (`teams_proto::workflows`, `kelma-cli flows`): environments, 4 flows and their run history read live (200). Read only by design: kelma has no turn on/off or run call yet.
- [ ] **S3.4 OneNote read and Lists**, native. Impact 2, effort 3.
  - Data layer done 2026-11-19, no screen yet (`teams_proto::onenote`, `teams_proto::lists`, `kelma-cli onenote|lists`): Lists read live (2 team sites, a library's columns and items). OneNote notebooks read live (2), but both are empty, so section, page and page-content parsing is only unit-tested.
- [ ] **S3.5 Web host with a TeamsJS shim** for every W app, verified one by one against the table above. Impact 4, effort 4.
- [ ] **S3.6 Bots and message extensions:**
  - bots chat like people (28: participants, already handled by chat)
  - message extensions ("…" › More actions › Create Jira issue, Polly…) open their task modules in the web host

  Impact 3, effort 4 (discover the invoke API).

## Order

S1.1 and S1.2, then S3.5 (it unlocks most apps at once), then S3.1, S2, S3.2, S3.3 and the rest.

## Endpoints already documented by open-source clients

See `docs/research/open-source.md` §3.5.
- **Bot and card actions:** `POST CS/agents/<botMri>/invoke`.
- **App entitlements in a chat:** `GET MT/beta/chats/<id>/apps/chatentitlements`.
- **Meeting tabs:** `GET MT/beta/chats/<meetingThread>/tabs`.
- **Giphy message extension:** `POST MT/beta/commands/com.microsoft.teamspace.inputextension.giphy/execute`. GIFs could therefore be added later without a separate Giphy key.
- **Not yet documented anywhere:** the install, uninstall and app-bar pinning endpoints. They stay in S1/S2 discovery.
