# kelma: plan to a Teams-shaped MVP

Spec: `/home/mohamed/.claude/plans/compressed-shimmying-flurry.md` (acceptance criteria 1–9).
Visual target: the Teams PWA dark theme, i.e. left rail, chat list with filter pills and sections,
header with tabs, right-aligned own bubbles, date separators, file cards and a rounded composer.
UI is Slint with the software renderer (ADR 0001).

Checks (workspace root): `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, `cargo build --workspace --release`.

## /finish — 2026-11-18: all phases to Teams shape

### Phase 0: scaffold
- [ ] Manual Arabic typing test in `spikes/slint-spike` (needs the user), then set ADR 0001 to `accepted`
- [ ] Initial commit, `gh repo create MohamedEslam04/kelma --public`, push `main` (only after confirming no tokens are in the tree)
- [ ] CI: `.github/workflows/ci.yml` runs fmt check, clippy `-D warnings` and tests on PRs and pushes to `main`

### Phase 1: protocol core (`crates/teams-proto`)
- [ ] `client_info.rs` (version, user-agent, client ids) plus an `http` per-host header policy, with unit tests
- [ ] `auth`: device code start and poll, refresh per resource, skypetoken authz with region to chat host, with fixture tests
- [ ] Refresh token kept in the system keyring (`keyring` crate); epid persisted in the config dir
- [ ] `chatsvc`: conversations list, history paging, send, consumption horizon, typing, with fixture tests
- [ ] `csa`: teams, channels and chat metadata list, with fixture tests
- [ ] `mt`: fetchShortProfile plus avatar fetch
- [ ] `trouter`: socket.io v0 frame codec and gzip/`cp`/`gp` payload decode, with unit tests
- [ ] `trouter`: connect, authenticate, 3 registrations, ack, 30 s ping, reconnect loop
- [ ] `model`: Event enum parsed from trouter payloads (NewMessage, MessageUpdate, ConversationUpdate), with tests
- [ ] `cli/` binary `kelma-cli`: `login | chats | read <id> | send <id> <text> | tail`
- [ ] Manual e2e (needs the user for the device code): login, list chats, send to `48:notes`, receive a live message

### Phase 2: protocol gaps
- [ ] Capture Teams web traffic for reactions, replies, edit, delete, channel post and file attach. Document it in `docs/protocol.md`. Sanitized fixtures go in `fixtures/` with no tokens.
- [ ] Reactions add and remove
- [ ] Quoted replies, parse and send
- [ ] Edit and delete own messages
- [ ] Channel posts: list, reply, new post
- [ ] Images: AMS upload, plus download with a disk cache
- [ ] Files: SharePoint/OneDrive file cards with "open in browser". Upload, or `BLOCKED` with the fallback from the spec.

### Shared crates
- [ ] `crates/render`, with tests:
  - converts Teams HTML into a rich-text AST: bold, italic, links, lists, code, emoji `alt`, `<at>` mentions, reply blockquote, AMS images and file cards
  - per-paragraph direction
- [ ] `crates/store`: SQLite schema for conversations, messages, users and avatars, with upsert and windowed queries, plus tests

### Phase 3: UI (`app/`, Slint, software renderer)
- [ ] Dark Teams-like theme tokens: background, panels, accent `#7f85f5`, own and other bubbles. Bundle a color emoji font.
- [ ] Login screen that shows the device code and verification URL, with a copy button
- [ ] Left rail with Chat and Teams icons and an active indicator
- [ ] Chat list:
  - search
  - filter pills: Unread, Channels, Chats
  - sections: Favorites, Chats, Teams and channels
  - avatars, unread rows in bold with a badge, sorted by activity
- [ ] Conversation header: avatar, name, tabs (Chat, Shared)
- [ ] Message list:
  - virtualized
  - own messages right, others left with avatar, name and time
  - date separators, "Edited" marker
  - reactions row, quoted reply, inline images, file cards, links
  - RTL handled per paragraph
- [ ] Hover actions on a message: react, reply, edit or delete own messages
- [ ] Composer:
  - rounded box
  - emoji picker
  - attach image or file
  - reply preview strip
  - Enter sends, Shift+Enter adds a newline
  - sends the typing indicator
- [ ] Teams view: teams, then channels, then posts with replies; new post
- [ ] Desktop notifications when the window is unfocused; mark-read when a chat is viewed
- [ ] Screenshots of kelma next to the Teams reference in `docs/screenshots/`

### Phase 4: hardening
- [ ] Reconnect after network loss or sleep, with history catch-up
- [ ] Skypetoken re-mint before expiry; the rotated refresh token is persisted
- [ ] Memory: after 10 min idle with about 20 chats, under 120 MB PSS (`portstat -a kelma`). Record the result in the README.
- [ ] `.desktop` file, an original icon (not Microsoft's), and release build and install instructions
- [ ] README: features, install, screenshots, disclaimer; roadmap ticked
