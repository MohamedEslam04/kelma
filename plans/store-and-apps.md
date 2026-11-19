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

- [x] **S1.1 App bar.** Impact 4, effort 2.
  - **What.**
    - The left rail has Activity, Chat, Calendar, Calls, OneDrive, Copilot, Meeting Recaps, Store and "…".
    - Pin and unpin (right click), drag to reorder, collapse the app bar.
    - Shortcuts Ctrl+Shift+1…9.
    - Badges (Activity count).
  - **How.**
    - Discover the user's pinned-apps setting (Teams middle tier); otherwise keep the list in local config.
    - Admin-pinned apps come from the setup policy (discover).
  - **Evidence.** Data layer in `crates/apps` (`bar.rs`), UI still to wire. `POST MT/beta/users/apps/aggregatedEntitlements?appbarview=userpinned` with body `[]` (Spaces bearer + skypetoken, header `x-ms-client-caller`) returns every entitled app (`userEntitlements`) with its definition; pinned apps carry `isAppBarPinned`, `appBarOrder`, `isAdminPinned` (setup policy) or `isUserPinned`. `beta/me/settings/appPolicy` gives `isUserPinningAllowed`. Pin, unpin and reorder build `POST MT/beta/users/apps/pinned` with the ordered `[{appId, appBarOrder}]` list, printed as a dry run only (not sent). Live, read-only: `kelma-cli store bar` lists Activity, Chat, Teams, Calendar, Calling, OneDrive, Copilot and Meeting Recaps in order, plus "Chat and Channels" (the unified view's entry; the client shows either it or Chat + Teams). Endpoints come from the Teams web client's public JS bundle. Tests: `cargo test -p apps`.
- [x] **S1.2 "View more apps" flyout.** Impact 3, effort 2.
  - **What.** "Search apps", then the installed list. Seen: Linear, Avatars, Shifts, Planner, OneNote, Help, Clipchamp, Whiteboard, Events, People, Visio, Word, Excel, PowerPoint, Loop, Workflows, Approvals, Viva Learning, Updates, Power BI, SharePoint, Viva Insights. Opening an app shows it in the main area.
  - **How.** The installed apps come from the Teams middle tier (discover; Graph `ReadForUser` is not granted).
  - **Evidence.** `InstalledApps::more_apps(query)` in `crates/apps`, from the same aggregated entitlements: installed (`Installed`, `InstalledAndPermanent`), not pinned, not core, with a personal tab or personal bot, by name. `AppDefinition::personal_tabs()` gives the tab URLs for the web host and `open_link` the `l/entity` deep link. Live, read-only: `kelma-cli store more` matches the surveyed list (Planner, OneNote, Shifts, Word, Excel, PowerPoint, Loop, Workflows, …); `store more note` filters. People (a core app) and Events (not entitled for this user) are not in it.

## Phase S2: Store

- [x] **S2.1 Browse and search.** Impact 3, effort 3.
  - **What.**
    - Home with hero search, "Popular on Teams", "Trending with your peers" and See all.
    - The Featured and Categories menus.
    - Devices (certified hardware) links out.
    - Filter by works-in (personal, chat, team, meeting) and agent or app.
  - **How.** Graph `GET /appCatalogs/teamsApps` (granted) for the catalog. Categories, ratings and screenshots come from the Teams internal store service (discover).
  - **Evidence.** Data layer in `crates/apps` (`store.rs`). Home: `POST MT/beta/users/apps/store` (`appCategories`, `appsStoreOptions`) returns every category (`all`, `popularApps`, `productivity`, `personalApps`, `bots`, industries, …) with totals, blocked counts and up to 100 ranked app ids with ratings; names and icons come from `POST MT/beta/users/apps/batchedDefinitions` (body: the ids). Search: `POST MT/beta/users/apps/search` `{searchQuery}`; its `features` give works-in, price and certification. `Filter` handles works-in and agent or app. "Built for your org": Graph `appCatalogs/teamsApps?$filter=distributionMethod eq 'organization'` (`$top` is refused). Live, read-only: `kelma-cli store home` (44 categories), `store search planner` (78 hits), `store search planner --team --apps`, `store org`. No separate "Trending with your peers" list was found (`includeSuggestedApps` adds nothing); Devices stays a link.
- [x] **S2.2 App page.**
  - **What.** Icon, name and publisher; Add ▾ (Add for me, to a team, to a chat, to a meeting); Share; rating; installs; works in; price; screenshot carousel; description; privacy, terms and permissions (RSC) links.
  - **Evidence.** `AppDetails::new(definition, store entry, search hit)` in `crates/apps`: icons, developer, long description, screenshots, video, privacy and terms URLs, manifest, RSC and device permissions, works-in, `add_targets()` for Add ▾, rating (Store home), price and certification (search), `share_link()` (`l/app/<id>`). Install counts are not served to the client. Live, read-only: `kelma-cli store app com.microsoft.teamspace.tab.planner` and `store app <OneNote id>` print every field.
- [ ] **S2.3 Install and uninstall.** Impact 3, effort 3.
  - **What.** Add for me, to a chat, to a team or to a meeting; Manage your apps (installed apps, updates, Remove).
  - **How.** Use the internal install endpoint the web client calls (discover). If that is blocked, "Open in Teams web to install".
  - **Status.** Request builders only, not ticked: `POST MT/beta/{users|teams/<id>|chats/<id>}/apps/entitlements` with the app definition as body (a meeting uses its chat), `DELETE …/apps/entitlements/<appId>`, `POST …/apps/Upgrade/<appId>?version=`, from the web client's code. `kelma-cli store install|uninstall|update` print them as dry runs; none was sent (never on the work tenant). Manage your apps reads live: `GET MT/beta/users/apps/store/entitlements` (`store mine`), and updates come from `installedVersion` against the definition's version. Next step: send once on a test tenant.
- [x] **S2.4 Store settings** (gear): app permissions per app (notifications, location, media, files, external links). Local grants are used by the web host.
  - **Evidence.** `crates/apps` `settings.rs`: `app_policy` (`GET MT/beta/me/settings/appPolicy`: custom app upload, user pinning) and `Grants`, the per-app grants kept in kelma's config as JSON (denied by default). The permissions an app asks for are `AppDefinition::device_permissions` (manifest `devicePermissions`: media, geolocation, notifications, midi, openExternal). There is no "files" device permission in manifests. Live, read-only: `kelma-cli store settings`. Unit tests cover the JSON round trip.

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
  - **Evidence (shim).** 2026-11-19: `webhost::shim` answers teams-js 2.x in a top-level window through its frameless transport (`nativeInterface.framelessPostMessage` / `onNativeMessage`): `initialize` (frame context `content`, host `desktop`, v4 runtime), `getContext`, `authentication.getAuthToken` (to a Rust `TokenCallback`), `appInitialization.*`, `executeDeepLink`, `themeChange`; other calls get a "not supported" SDK error. Only the app's origin (from the CDP execution context) is answered. `webhost::shim::expand_url` fills `contentUrl` placeholders. Unit tests (message in, reply out) in `shim.rs`; live with the real `@microsoft/teams-js` 2.59.0 UMD on a local page (no sign-in) on Brave and Vivaldi: initialize, getContext, getAuthToken, notifySuccess and a theme change all round-trip. Commit 3c5a382.
  - **Still open.** Per-app checks against the table (need sign-in); app SSO tokens (the broker needs a per-resource audience for `api://<domain>/<app id>`); the UI wiring.
- [ ] **S3.6 Bots and message extensions:**
  - bots chat like people (28: participants, already handled by chat)
  - message extensions ("…" › More actions › Create Jira issue, Polly…) open their task modules in the web host

  Impact 3, effort 4 (discover the invoke API).
  - **Evidence (data layer, 2026-11-19, no screen yet).** `crates/apps` `bots.rs`, `kelma-cli store bots|extensions|ext-query`. Installed apps of a place: chat `GET MT/beta/chats/<id>/apps/chatentitlements` (also `48:notes`, 495 definitions, 45 with bots, 59 with messaging extensions) and team or channel `GET MT/beta/channels/<id>/apps/entitlements` (200 on a private channel; `beta/teams/<id>/apps/entitlements` is POST only, 405 on GET); both give `value.entitlements` `[{id,state}]` plus `value.definitions`. A definition's `bots[]` has `id`, `scopes`, `isNotificationOnly`, `supportsFiles`, `commandLists[].commands[{title,description,type basic|prompt}]`; its `inputExtensions[]` (the manifest's `composeExtensions`, `type botBased`) has `botId`, `scopes`, `canUpdateConfiguration`, `fetchCommands` (commands served at run time through `composeExtension/fetchCommands`, then `commands` is empty) and `commands[]` with `id`, `title`, `type` (`Action`, absent means query), `context` (`Compose`, `Commandbox`, `Message`; absent means compose and commandbox), `parameters[{name,title,isRequired,inputType Textarea|Toggle|Choiceset,choices}]`, `fetchTask` and `taskInfo`. Query: `POST <chat service>/v1/agents/28:<botId>/invoke` `{name:"composeExtension/query", appId, value:{commandId, commandContext, parameters:[{name,value}], queryOptions:{skip,count}}, conversation?}` (envelope from the web client's `postInvoke`). Live, read-only: a first-party search extension answered 200 `composeExtension.type = "silentAuth"` (no `conversation`; with `conversation: 48:notes` the service answers 404 `ConversationNotFound`), so the route and envelope are right; result cards, `auth`, `config` and the single sign-on retry (`userAadToken` from the app's `webApplicationInfo`) are only unit-tested against the web client's response code and the Bot Framework schema. Action commands are not sent (`query_extension` refuses them); task modules (`composeExtension/fetchTask`) and `fetchCommands` are not built yet. UI: list from `place_apps`, filter with `Bot::in_scope`/`Extension::in_scope` (`personal`, `team`, `groupchat`) and `ExtensionCommand::contexts`; action commands open `task.url` or the fetched task in the web host (S3.5). Tests: `cargo test -p apps`.

## Order

S1.1 and S1.2, then S3.5 (it unlocks most apps at once), then S3.1, S2, S3.2, S3.3 and the rest.

## Endpoints already documented by open-source clients

See `docs/research/open-source.md` §3.5.
- **Bot and card actions:** `POST CS/agents/<botMri>/invoke`.
- **App entitlements in a chat:** `GET MT/beta/chats/<id>/apps/chatentitlements`.
- **Meeting tabs:** `GET MT/beta/chats/<meetingThread>/tabs`.
- **Giphy message extension:** `POST MT/beta/commands/com.microsoft.teamspace.inputextension.giphy/execute`. GIFs could therefore be added later without a separate Giphy key.
- **Install, uninstall and app-bar pinning:** found in the Teams web client's JS bundle (2026-11-19), see S1.1 and S2.3. Reads verified live, writes not sent.
