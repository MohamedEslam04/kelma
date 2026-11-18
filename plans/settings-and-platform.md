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
- [ ] **X1.5 Privacy.** Impact 2, effort 3.
  - **What.** Manage priority contacts (who reaches you during Do not disturb); blocked contacts; block calls with no caller ID; surveys; diagnostic data; read receipts.
  - **How.** Discover the endpoints (Teams middle tier); read receipts are in the chat plan.
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

## Phase X3: platform

- [ ] **X3.1 Web host component.** Impact 5, effort 4.
  - **What.**
    - A shared Chromium-based host for the "W" apps, meeting and call media, Office web and the Copilot fallback.
    - One process on demand, with a separate profile.
    - Closed when idle so kelma's memory stays near its baseline.
  - **How.** ADR 0003 decides between CEF (cef-rs), a Chromium `--app` window, and wry/WebKitGTK (unsupported by Teams and ACS for media, but fine for some tabs).
- [ ] **X3.2 Auth broker.** Impact 4, effort 2.
  - **What.** One place mints and caches tokens per audience: spaces, chatsvcagg, presence, Graph, substrate, sydney, outlook, SharePoint (per host), yammer, flow, tasks, onenote, ic3. Each has a wall-clock expiry and lazy refresh, and the rotated refresh token is persisted.
  - **How.** Extend `teams_proto::client` (Graph is already there) into a generic per-audience cache.
- [ ] **X3.3 Tray, autostart and badge.** Shared with the chat plan 9.2.
- [ ] **X3.4 Global search** across apps: chats, files, people, calendar (substrate). Shared with the chat plan 7.1.
- [ ] **X3.5 Packaging.** Flatpak (with the PipeWire and portal permissions the meeting host needs), AUR/RPM, and auto-update notices.
- [ ] **X3.6 Telemetry-free diagnostics.** A "Report a problem" that writes a local log bundle with tokens and content scrubbed.

## Order

X3.2 first (every app needs it), then X1.1, X3.1, X2.1 and the rest.

## Endpoints already documented by open-source clients

See `docs/research/open-source.md` §3.2, §3.8 and §3.20.
- **Org list for X2.1:** `GET MT/beta/users/tenants` and `tenantsv2`.
- **User settings bag:** `GET /v1/users/ME/properties` and `PUT …?name=<key>`. It holds `teamsOrder`, `simpleCollabUserSettings`, `preferredSkinToneIndex`, `defaultMessageReactions` and `readReceiptsEnabled`.
- **Other settings:** `POST MT/beta/users/useraggregatesettings`, `PATCH MT/beta/users/languageAccountSettings` (translation) and `POST MT/beta/usersettings/<mri>/ringtonesettings`.
- **Blocking:** no server endpoint is known for blocking a user on a work account, so X1.5 blocking stays a discovery item.
- **Personal Microsoft accounts:** these could be added later. They use a different client id and the `teams.live.com` hosts (§3.20).
