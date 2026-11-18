# kelma: plan to a Teams-shaped MVP

Spec: `/home/mohamed/.claude/plans/compressed-shimmying-flurry.md` (acceptance criteria 1–9).
Visual target: the Teams PWA dark theme, i.e. left rail, chat list with filter pills and sections,
header with tabs, right-aligned own bubbles, date separators, file cards and a rounded composer.
UI is Slint with the software renderer (ADR 0001).

Checks (workspace root): `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, `cargo build --workspace --release`.

## /finish — 2026-11-18: all phases to Teams shape

### Phase 0: scaffold
- [ ] Manual Arabic typing test in `spikes/slint-spike` (needs the user), then set ADR 0001 to `accepted` — BLOCKED: needs the user (typing with an Arabic keyboard layout)
- [x] Initial commit, `gh repo create MohamedEslam04/kelma --public`, push `main` (only after confirming no tokens are in the tree)
- [x] CI: `.github/workflows/ci.yml` runs fmt check, clippy `-D warnings` and tests on PRs and pushes to `main`

### Phase 1: protocol core (`crates/teams-proto`)
- [x] `client_info.rs` (version, user-agent, client ids) plus an `http` per-host header policy, with unit tests
- [x] `auth`: device code start and poll, refresh per resource, skypetoken authz with region to chat host, with fixture tests
- [x] Refresh token kept in the system keyring (`keyring` crate); epid persisted in the config dir
- [x] `chatsvc`: conversations list, history paging, send, consumption horizon, typing, with fixture tests
- [x] `csa`: teams, channels and chat metadata list, with fixture tests
- [x] `mt`: fetchShortProfile plus avatar fetch
- [x] `trouter`: socket.io v0 frame codec and gzip/`cp`/`gp` payload decode, with unit tests
- [x] `trouter`: connect, authenticate, 3 registrations, ack, 30 s ping, reconnect loop — verified live 2026-11-18 (echo, MessageUpdate, throttling retry)
- [x] `model`: Event enum parsed from trouter payloads (NewMessage, MessageUpdate, ConversationUpdate), with tests
- [x] `crates/kelma-cli` binary `kelma-cli`: `login | chats | read <id> | send <id> <text> | tail`
- [x] Manual e2e (needs the user for the device code): login, list chats, send to `48:notes`, receive a live message — verified 2026-11-18: login (amer region), chats, teams, send/receive in 48:notes

### Phase 2: protocol gaps
- [x] Capture Teams web traffic for reactions, replies, edit, delete, channel post and file attach. Document it in `docs/protocol.md`. Sanitized fixtures go in `fixtures/` with no tokens. — done by inspecting the live API with `kelma-cli raw/csa-raw` instead of browser HAR
- [x] Reactions add and remove — verified live in 48:notes
- [x] Quoted replies, parse and send — verified live in 48:notes
- [x] Edit and delete own messages — verified live in 48:notes
- [ ] Channel posts: list, reply, new post — BLOCKED: listing posts and threads verified live; posting into a real channel needs the user's OK because colleagues would see the test post
- [x] Images: AMS upload, plus download with a disk cache — verified live (upload, send, download)
- [ ] Files: SharePoint/OneDrive file cards with "open in browser". Upload, or `BLOCKED` with the fallback from the spec. — BLOCKED: file cards + open in browser verified live; upload needs OneDrive/Graph scopes, fallback from the spec in place (attach explains it)

### Shared crates
- [x] `crates/render`, with tests:
  - converts Teams HTML into a rich-text AST: bold, italic, links, lists, code, emoji `alt`, `<at>` mentions, reply blockquote, AMS images and file cards
  - per-paragraph direction
- [x] `crates/store`: SQLite schema for conversations, messages, users and avatars, with upsert and windowed queries, plus tests

### Phase 3: UI (`app/`, Slint, software renderer)
- [x] Dark Teams-like theme tokens: background, panels, accent `#7f85f5`, own and other bubbles. Bundle a color emoji font. — Twemoji COLRv0 via SLINT_FONT_PATH
- [x] Login screen that shows the device code and verification URL, with a copy button
- [x] Left rail with Chat and Teams icons and an active indicator
- [x] Chat list: — rendering verified with the real account; pill/search clicks part of the user check below
  - search
  - filter pills: Unread, Channels, Chats
  - sections: Favorites, Chats, Teams and channels
  - avatars, unread rows in bold with a badge, sorted by activity
- [x] Conversation header: avatar, name, tabs (Chat, Shared)
- [x] Message list: — verified with the real account (images, cards, voice, files, Arabic)
  - virtualized
  - own messages right, others left with avatar, name and time
  - date separators, "Edited" marker
  - reactions row, quoted reply, inline images, file cards, links
  - RTL handled per paragraph
- [ ] Hover actions on a message: react, reply, edit or delete own messages — BLOCKED: needs the user to hover and click (the reaction/edit/delete APIs are verified live)
- [ ] Composer: — BLOCKED: needs the user for an interactive check (send/reply APIs verified live)
  - rounded box
  - emoji picker
  - attach image or file
  - reply preview strip
  - Enter sends, Shift+Enter adds a newline
  - sends the typing indicator
- [x] Teams view: teams, then channels, then posts with replies; new post — verified with the real account, including the thread view
- [ ] Desktop notifications when the window is unfocused; mark-read when a chat is viewed — BLOCKED: needs an incoming message from someone else while kelma is unfocused
- [x] Screenshots of kelma next to the Teams reference in `docs/screenshots/` — demo-mode screenshots only: real-account screenshots and the Teams reference contain colleagues' data and are not committed

### Phase 4: hardening
- [ ] Reconnect after network loss or sleep, with history catch-up — BLOCKED: reconnect and resync verified for socket/registration failures; a real network drop needs the user's OK to take the network down
- [ ] Skypetoken re-mint before expiry; the rotated refresh token is persisted
- [x] Memory: after 10 min idle with about 20 chats, under 120 MB PSS (`portstat -a kelma`). Record the result in the README. — ~29 MB PSS idle, ~62 MB PSS+swap with an image chat open
- [x] `.desktop` file, an original icon (not Microsoft's), and release build and install instructions
- [x] README: features, install, screenshots, disclaimer; roadmap ticked
