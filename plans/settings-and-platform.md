# kelma plan: settings, accounts and platform

Part of the Teams parity plans (see `plans/README.md`). Format as in `plans/chat.md`. Chat-specific
settings (chats, notifications, appearance) are in `plans/chat.md` phase 9. This file covers the
rest of Teams' settings plus app-wide platform work.

**Sources.** UI survey of every settings page, a token probe and `docs/research/apps-onedrive-copilot-store.md` §6. The settings pages are:
- General, Appearance, Chats and channels
- Notifications and activity, Accounts and orgs
- Privacy, Accessibility, Devices, Calls
- Files and links, App permissions, About

## Phase X1: settings pages

- [ ] **X1.1 Settings shell.** Impact 3, effort 2.
  - **What.** A Settings page (Ctrl+Shift+,) with "Find in Settings" (Ctrl+Alt+F), the left list above and About.
- [ ] **X1.2 General.** Impact 2, effort 2.
  - **What.**
    - App language, date format and time format (following the Microsoft 365 account language, or overridden).
    - Time zone.
    - Translation preferences.
    - Auto-start, open in background, keep running on close.
  - **How.** Mostly local. Mailbox time zone and language come from Outlook REST on the `outlook.office.com` token (Graph MailboxSettings is not granted).
- [ ] **X1.3 Accessibility.** Impact 2, effort 2.
  - **What.**
    - Captions and transcripts: always show captions, identify me, filter profanity.
    - Muted notification.
    - Single-pointer controls.
    - TTY mode.
    - Sign-language mode.
  - **How.** Local, applied to the meeting host where relevant. Slint accessibility goes through AccessKit.
- [ ] **X1.4 Devices.** Impact 3, effort 3.
  - **What.**
    - Speaker and microphone with "Make a test call".
    - Camera with preview, mirror, and low-light brightness.
    - Connected devices; meeting room mode.
  - **How.** PipeWire device enumeration; the settings are passed to the media host (`plans/calls.md` K2.3).
  - **Evidence (data layer only, UI missing, box stays open).** 2026-11-19: new crate `crates/devices` (no dependency on `calling`). `audio_inputs()` and `audio_outputs()` list cpal devices as `AudioDevice {id, name, is_default, channels, sample_rates}` (cpal `device.id()`, stable across runs, for example `alsa:hw:CARD=0,DEV=0`; ALSA through PipeWire). `cameras()` lists capture-capable V4L2 nodes as `Camera {path, name, driver, bus, formats: [{fourcc, sizes}]}` using only sysfs and read-only `VIDIOC_QUERYCAP`, `ENUM_FMT` and `ENUM_FRAMESIZES` (no system library, nothing streams, so the camera light stays off); a node that cannot be opened falls back to its sysfs name; the bus string (`usb-0000:00:14.0-6`) is the stable camera id. `test_sound(output)` plays a 600 ms 440 Hz tone, `mic_level(input)` gives RMS and peak readings every 50 ms (`MicLevelStream::latest` for a UI timer), `loopback(input, output, duration)` is the optional echo test (at most 200 ms delay, headphones advised). `DevicePrefs {microphone, speaker, camera}` is the serde record the UI persists (`None` is the system default; `find_*` resolve a saved choice against the current lists). Live, enumeration only: 8 inputs, 38 outputs (mostly raw ALSA `hw`/`plughw`/HDMI entries the UI should filter or group) and 1 camera (`uvcvideo`, MJPG and YUYV, 1280x720 down to 320x180). `kelma-cli devices [audio|cameras|test-sound|mic-level]`. Not run: `test_sound`, `mic_level` and `loopback` against real hardware (no sound, no microphone opened); run `cargo test -p devices -- --ignored --nocapture` (or one test by name). Offline tests cover the tone, level meter, remix and resampling, playout and loopback queues, camera node and sysfs handling, the V4L2 ioctl numbers and struct sizes, and the prefs. Not done: camera preview (needs streaming), mirror and low-light brightness, the Flatpak PipeWire camera portal path, connected devices and meeting room mode, and passing the choice to the media host (`plans/calls.md` K2.3). Tests: `cargo test -p devices -p kelma-cli`.
- [ ] **X1.5 Privacy.** Impact 2, effort 3.
  - **What.** Manage priority contacts (who reaches you during Do not disturb); blocked contacts; block calls with no caller ID; surveys; diagnostic data; read receipts.
  - **How.** Discover the endpoints (Teams middle tier); read receipts are in the chat plan.
  - **Evidence (data layer only, UI missing, box stays open).** `crates/teams-proto/src/user_settings.rs`. Read receipts, "block calls from people outside my contacts" and "block adding me to group chats" are bag properties of `GET /v1/users/ME/properties`, kept as the strings `"true"`/`"false"` (`readReceiptsEnabled`, `disableNonContactCall`, `disableNonContactAddToGroup`), plus `userEnableSurveysNotification` (a bool); the write is `PUT /v1/users/ME/properties?name=<key>` with `{"<key>": "<value>"}`. Blocked contacts and "Manage priority access" are two middle-tier lists the web client calls: `GET MT/beta/userSettings/blocklist` and `GET MT/beta/userSettings/breakthroughlist` answer `{"value":[...]}`; the change is `POST …/manage` with `{"add":["<mri>"]}` or `{"remove":["<mri>"]}`, answered `{"value":{"status":"Success"}}` (`Error` is a failure; the web client also sends `x-ms-client-caller`). This corrects the earlier note that no block endpoint is known for work accounts: the web client uses `beta/userSettings/blocklist` for all account types (the config entry that names `teams.live.com` is only for personal accounts), but it was not run against a work account. Priority people also have a notification type (`VIP`, in the 9.1 settings). `PrivacySettings {blocked, read_receipts, block_non_contact_calls, block_non_contact_group_adds, surveys}`, `PriorityAccess`, `PrivacyFlag`, `manage_request`, `privacy_flag_request` (requests are built, never sent). Not found: a "show when I am available" presence-sharing setting, and diagnostic data (that is local). Not run live: the keyring failed twice (DBus, no reply), so the list item shape (strings or objects with `mri`; both are parsed) and whether a work account may call the lists (the client treats 403 as a handled case) are unverified. Sources: Teams web client JS bundle (facts only). Tests: `cargo test -p teams-proto user_settings`. CLI: `settings privacy|blocked|priority` (counts; `--ids` lists MRIs) and `settings set privacy|block|unblock|priority-add|priority-remove ... --dry-run`.
- [ ] **X1.6 Files and links.** Impact 2, effort 1.
  - **What.** Open Office files in kelma's viewer, the browser or a desktop app; open links in the default browser.
- [ ] **X1.7 App permissions.** Impact 2, effort 2.
  - **What.** Per-app grants for media, location, notifications and external links, used by the web host.

## Phase X2: accounts and organizations

- [ ] **X2.1 Accounts and orgs.** Impact 4, effort 4.
  - **What.**
    - Several accounts, and guest organizations per account.
    - Switch from the profile menu.
    - Per-org notifications and badges.
    - "Refresh license and restart".
    - Sign out of one account.
  - **How.**
    - Account means (home account, tenant).
    - Guest tenants: redeem the refresh token at `login.microsoftonline.com/{guestTenant}`, expecting a CA or MFA step-up to fall back to device code.
    - The org list comes from the Teams middle tier (discover).
    - Each tenant gets an isolated store and keyring entry. Changes `crates/session`.
  - **Done when.** Two orgs show side by side with separate unread counts.
  - **Evidence (session layer).** 2026-11-19: `session::Config.accounts` (label, upn, display name, home tenant, chosen tenant, epid, last used, cached org list) with `add_account`, `switch_account`, `remove_account`, `switch_tenant`, `set_orgs`; the legacy `account`/`tenant`/`epid`/`self_mri`/`display_name` fields mirror the active account, and an old `config.json` loads as one account with the old label and keyring entry (no writes). Keyring access goes through a `TokenStore` trait (`SystemKeyring`, in-memory `MemoryStore` for tests); the token key is `label` for the home tenant and `label@tenant` for a guest tenant, and a guest tenant without its own token starts from the home token. `Client::tenants()` (`GET MT/beta/users/tenants`, `teams_proto::tenants`) run live read-only: 1 tenant, the home one. `kelma-cli accounts [tenants|use|add|remove|org]`. Not run live: `add`, `remove`, `use`, `org`, and any guest tenant (the account has none, so the guest shape and the CA/MFA step-up are unverified). The box stays unticked: no profile menu, per-org unread counts or "Refresh license and restart" exist yet. The UI still has to show it.

## Phase X3: platform

- [x] **X3.1 Web host component.** Impact 5, effort 4.
  - **What.**
    - A shared Chromium-based host for the "W" apps, meeting and call media, Office web and the Copilot fallback.
    - One process on demand, with a separate profile.
    - Closed when idle so kelma's memory stays near its baseline.
  - **How.** ADR 0003 decides between CEF (cef-rs), a Chromium `--app` window, and wry/WebKitGTK (unsupported by Teams and ACS for media, but fine for some tabs).
  - **Evidence.** 2026-11-19: `docs/adr/0003-web-host.md` picks a Chromium-family `--app` window driven over CDP. Crate `webhost`: `webhost::open(url, opts) -> WebWindow` (`eval`, `on_message` bridge, `events`, `set_theme`, `close`, `closed()`), profile `<data dir>/webhost-profile`, one browser process per window, stopped (process group) when the window closes or the `WebWindow` drops. 28 unit tests (browser discovery, launch args, `DevToolsActivePort`, profile lock, CDP client against a fake endpoint, shim). Live, no sign-in, throwaway profile: `cargo test -p webhost --test live -- --ignored` passed on Brave and Vivaldi (window opens, CDP attaches, bridge round-trips, page close resolves `closed()`, no browser left running); about 495 MB PSS for Brave with a trivial page, freed on close. `kelma-cli web <url> [--shim]`. Commits 3c5a382, c29f4cd.
  - **Still open.** The UI does not call it yet; one process per window (no sharing); DevTools listens on 127.0.0.1 (`--remote-debugging-pipe` is the follow-up); popups (`window.open`, `authentication.authenticate`) are not tracked.
- [x] **X3.2 Auth broker.** Impact 4, effort 2.
  - **What.** One place mints and caches tokens per audience: spaces, chatsvcagg, presence, Graph, substrate, sydney, outlook, SharePoint (per host), yammer, flow, tasks, onenote, ic3. Each has a wall-clock expiry and lazy refresh, and the rotated refresh token is persisted.
  - **How.** Extend `teams_proto::client` (Graph is already there) into a generic per-audience cache.
- [ ] **X3.3 Tray, autostart and badge.** Shared with the chat plan 9.2.
  - **Evidence (tray crate only, app wiring missing, box stays open).** 2026-11-19: new crate `crates/tray` (`ksni` 0.3.7 over zbus 5, pure Rust, no new system package). `Tray::start(TrayOptions) -> TrayHandle` runs on its own thread with a private runtime, so it sits beside the Slint loop. `TrayHandle` (cheap to clone): `set_unread(n)` (overlay badge pixmap in 4 sizes drawn in code, title `kelma (12 unread)`, tooltip), `set_muted(bool)`, `set_launcher_count(n)`, `events()` (a cloneable crossbeam `Receiver<TrayEvent {Open, ToggleMute(bool), Quit}>`), `tray_error()`, `launcher_ready()`, `shutdown()` (removes the icon and clears the launcher badge). The icon is the `kelma` theme name with a pixmap fallback drawn from the `packaging/kelma.svg` shapes. Menu: Open kelma, Mute notifications (checkmark), separator, Quit; left click is `Open`. The badge is 0 hidden, 1 to 99 as is, `99+` above. `launcher::launcher_update` builds the `com.canonical.Unity.LauncherEntry.Update` body (`application://kelma.desktop`, `count` int64, `count-visible`), and `set_unread` sends it too unless `sync_launcher_badge` is off. `autostart::enable(exec, hidden_flag)`/`disable()`/`is_enabled()` write `$XDG_CONFIG_HOME/autostart/kelma.desktop` from the fields of `packaging/kelma.desktop` plus `X-GNOME-Autostart-enabled=true`. Live on KDE Plasma Wayland (`kelma-cli tray-demo`): the item showed up in `org.kde.StatusNotifierWatcher.RegisteredStatusNotifierItems` as `org.kde.StatusNotifierItem-<pid>-1/StatusNotifierItem` and left it on exit; `Activate` produced `Open`, the dbusmenu `Event` on Mute produced `ToggleMute(true)`; the item's Title, ToolTip and `OverlayIconPixmap` (4 sizes) followed 3, 12, 120 and 0; `dbus-monitor` saw LauncherEntry updates 3, 12, 120, 0 and a final 0 on shutdown. Autostart enabled, validated with `desktop-file-validate` and disabled under a scratch `XDG_CONFIG_HOME`. Not verified: how Plasma draws the overlay (no screenshot taken), the task manager badge on a pinned launcher, GNOME (needs the AppIndicator extension). Not done: the app has to call `Tray::start`, handle the events (show and focus the window, mute, quit), pass a `--hidden` flag it does not parse yet, close to tray, and a Settings toggle for autostart; the Flatpak uses the Background portal for autostart instead of the file. Tests: `cargo test -p tray -p kelma-cli`.
- [ ] **X3.4 Global search** across apps: chats, files, people, calendar (substrate). Shared with the chat plan 7.1.
  - **Evidence (data layer only, UI missing, box stays open).** 2026-11-19: `teams_proto::global_search`: `Client::global_search(self_mri, query, &SearchOptions {kinds, page_size, from}) -> SearchResults {messages, files, people, events, totals, more}` sends one substrate `searchservice/api/v2/query` (scenario `powerbar`) with an entity request per kind and parses typed hits (`MessageHit` with highlight ranges, `FileHit` with site/web/list/item ids, `PersonHit`, `EventHit`). Paging is per kind: `more` holds the next offset of each kind that has more and goes back in as `from`; `top_results(n)` interleaves the kinds for a "Top results" list; `dedup_against` drops hits the service repeats on a later page. Checked read-only against the work account: one query answers messages, files and events (people only for a name-like query); a combined query returns 8 messages (5 top + 3) and 3 files whatever the size, a single-kind query honours the size and pages cleanly (page 2 of messages from offset 20 after two pages), and an event query for more than 8 hits answers with hundreds of recurrences, so event pages are capped at 8. `kelma-cli search <query> [--kind K] [--page N]`.
  - **Still open.** No UI; chats and channels (`Chat`, `TeamsChannel` entities), in-chat search and KQL filters (`from:`, `in:`, `hasattachment:`) are not in the data layer yet; file hits carry SharePoint ids, not a Graph drive id (resolve with the onedrive crate).
- [ ] **X3.5 Packaging.** Flatpak (with the PipeWire and portal permissions the meeting host needs), AUR/RPM, and auto-update notices.
  - **Done so far.** Manifests are in `packaging/`: Flatpak (`flatpak/`, with network, Wayland/X11, PipeWire, notification and Secret Service permissions, and `build.sh` that generates `cargo-sources.json`), an AUR `PKGBUILD` and `.SRCINFO` (`aur/`), an RPM spec (`rpm/`), AppStream metadata, and `packaging/README.md` with the build steps. The Flatpak and the RPM build here, and the AUR package builds in an Arch container (see Evidence). An optional `packaging.yml` workflow (manual, and on `packaging/` changes on main) builds all three.
  - **Still open.** (1) Auto-update notices: an in-app "new version" check needs a decision, because it would contact a server (the package managers update the Flatpak, AUR and RPM installs). (2) The Flatpak has not been launched on a desktop, the metainfo has no screenshots (Flathub needs them) and the runtime is 25.08 while 26.08 is out. (3) The RPM was built with `--nodeps` because this machine has rustup instead of the `cargo` and `rust` RPMs, and the new `packaging.yml` workflow has not run yet.
  - **Evidence:** on 2026-11-19 at `d31bc90`, `packaging/flatpak/build.sh` built the Flatpak (offline, 1485 vendored sources, 42 MB binary, `ldd` finds every library in the 25.08 runtime); `rpmbuild -bb --nodeps` built `kelma-0.1.0-1.fc44.x86_64.rpm` (15 MB, `%check` passed 27 `diag` tests); `makepkg` in an `archlinux:base-devel` podman container built `kelma-git` r86.d31bc90 (17 MiB, `check()` passed 27 tests, `namcap PKGBUILD` clean). Full commands and caveats are in `packaging/README.md`.
- [x] **X3.6 Telemetry-free diagnostics.** A "Report a problem" that writes a local log bundle with tokens and content scrubbed.
  - **Done.** The `diag` crate redacts bearer, skype and refresh tokens, cookies, emails, MRIs, conversation ids, GUIDs (tenant ids) and tenant domains, with unit tests. `kelma-cli diag` and `kelma --report-problem` (also a desktop action) write a private `.tar.gz` to Downloads. Nothing is uploaded. The app has no log file of its own yet, so logs come from `--log FILE` or `<data_dir>/logs/`. A Settings > About button comes with X1.1.

## Order

X3.2 first (every app needs it), then X1.1, X3.1, X2.1 and the rest.

## Endpoints already documented by open-source clients

See `docs/research/open-source.md` §3.2, §3.8 and §3.20.
- **Org list for X2.1:** `GET MT/beta/users/tenants` and `tenantsv2`.
- **User settings bag:** `GET /v1/users/ME/properties` and `PUT …?name=<key>`. It holds `teamsOrder`, `simpleCollabUserSettings`, `preferredSkinToneIndex`, `defaultMessageReactions` and `readReceiptsEnabled`.
- **Other settings:** `POST MT/beta/users/useraggregatesettings`, `PATCH MT/beta/users/languageAccountSettings` (translation) and `POST MT/beta/usersettings/<mri>/ringtonesettings`.
- **Blocking:** `GET/POST MT/beta/userSettings/blocklist[/manage]` (and `breakthroughlist` for priority access) from the web client; see the X1.5 evidence. Not yet run against a work account.
- **Personal Microsoft accounts:** these could be added later. They use a different client id and the `teams.live.com` hosts (§3.20).
