# kelma: plan to Teams chat parity

The MVP in `PLAN.md` is done. This plan covers everything Teams chat does that kelma does not. It is one of the parity plans indexed in `plans/README.md`.

**Source.** A survey of the real Teams web app (`teams.cloud.microsoft`, 2026-11-18). It covered every
menu, hover bar, toolbar, dialog, settings page and the keyboard shortcut list (Ctrl+.). Each item
says where it was seen, so the expected behavior can be checked against Teams.

**Out of scope.** Calls, meetings, Calendar, Copilot and suggested replies, Loop, Notes, Storyline,
Recap, Store, app extensions (Planner, Jira, Polly, Workflows), and team or channel admin. kelma
opens those in the browser.

## How to read an item

| Field | Meaning |
|---|---|
| Impact | 1–5, value for daily use |
| Effort | 1–5, where 5 is large or risky |
| Seen | where in Teams the behavior was observed |
| What | the behavior to build, matching Teams |
| How | the endpoint and the kelma pieces. **known**: the endpoint is in purple-teams or already used by kelma. **discover**: capture it from the Teams web client first (DevTools, then sanitize) and document it in `docs/protocol.md` |
| Done when | the acceptance check |

Every item also needs:
- tests: unit tests for protocol and parsing, and UI tests in `crates/kelma/src/ui_tests.rs` for UI behavior
- a live check against the real account: post only in `48:notes` or a private test channel the user named, and delete the test messages afterwards

Checks: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, `cargo build --workspace --release`.

---

## Phase 5: everyday messaging

### 5.1 Paste and drag-drop
- [x] Impact 5, effort 2. Seen: composer (Ctrl+V, drop zone).
  - **What.**
    - Ctrl+V with an image on the clipboard attaches it as an inline image, with a preview chip above the composer before sending.
    - Dropping files on the chat attaches them: images inline, other files as file cards.
    - Several attachments can wait in the chip row, and each has ✕ to remove it.
  - **How.**
    - Read the clipboard image with `arboard::Clipboard::get_image` and encode it as PNG.
    - Get drops from winit `WindowEvent::DroppedFile` through the `unstable-winit-030` hook. Spike this first; if Slint swallows the event, ship paste only and leave drop as BLOCKED with the reason.
    - Upload with the existing `upload_image` and `send_file`.
    - Add a `pending-attachments` model in the UI.
  - **Done when.**
    - A pasted screenshot shows in 48:notes as an image.
    - A dropped PDF shows as a file card.
    - A UI test covers the chip row and removing an item.

### 5.2 Hover bar like Teams
- [x] Impact 5, effort 2. Seen: message hover.
  - **What.**
    - The bar reads 👍 ❤️ 😆 😮, then "More reactions", "Reply with quote" (❝), Edit (own messages only), "…".
    - More reactions opens a searchable picker with every Teams reaction key.
    - Hovering a reaction chip shows who reacted.
  - **How.**
    - Restyle `MessageRow` in `app.slint`.
    - Map reaction keys (`like`, `heart`, `laugh`, `surprised`, `sad`, `angry`, plus codepoint keys such as `1f525_fire`). `reaction_emoji` already decodes them.
    - Names for the tooltip come from `emotions[].users[].mri`, resolved through the profile cache.
  - **Done when.**
    - A UI test checks the order of the bar and the picker search.
    - The tooltip lists names.

### 5.3 Message "…" menu
- [x] Impact 5, effort 3. Seen: message menu, own and others' messages.
  - **What.** One menu, like Teams:
    - Reply with quote
    - Forward, with Forward options: this message or multiple messages
    - Copy message link
    - Save this message (others' messages)
    - Pin for everyone
    - Mark as unread
    - Translation
    - Delete (own messages)
    - Edit (own messages, also in the bar)

    Right click on a message opens the same menu.
  - **How.**
    - **Forward** (discover): Teams sends a new message with the original embedded as a forwarded quote. The dialog has "Add recipients" (people, chats, channels), "Add a message (optional)" and a preview of the quoted message, then Forward. Recipient search reuses 6.2.
    - **Copy link** (known shape): `https://teams.microsoft.com/l/message/{conv}/{msgId}?context=…`, copied with arboard.
    - **Save** (discover): a message property; it also feeds a "Saved" view (9.8).
    - **Pin** (discover): a thread property. It also feeds the pinned banner (6.6).
    - **Mark as unread** (known): set `consumptionhorizon` to the message before this one, and set the chat's unread flag in the store.
    - **Translation**: 9.6.
    - **Multiple messages**: a checkbox mode in the list, then forward them together.
  - **Done when.** Each action works live in 48:notes or the test channel, and pinning shows in Teams web.

### 5.4 Composer formatting
- [ ] Impact 4, effort 4. Seen: composer toolbar, overflow, shortcuts.
  - **What.**
    - Toolbar: Bold, Italic, Underline, Strikethrough, bullet and numbered lists, text highlight, font color, font size, Quote, Insert link, Code block.
    - Overflow: Paragraph and headings, inline Code, Clear formatting, indent and outdent, horizontal rule, table, undo and redo.
    - Shortcuts: Ctrl+B, Ctrl+I, Ctrl+U, Ctrl+Alt+X (strike), Ctrl+K (link), Ctrl+Alt+Shift+C (inline code), Ctrl+Alt+Shift+B (code block), Ctrl+Alt+1/2/3 (headings), Ctrl+Alt+P (paragraph), Ctrl+Alt+4 (quote), Ctrl+Shift+7/8 (lists), Ctrl+Z, Ctrl+Y.
    - "Show formatting options" expands the composer, like Teams.
  - **How.**
    - Slint's `TextInput` is plain text, so the composer edits **Markdown-style source with a live preview strip**. Toolbar buttons and shortcuts wrap the selection in markers.
    - On send, `render::markdown_to_teams_html` (new) converts the source to Teams HTML: `<b> <i> <u> <s> <ul> <ol> <blockquote> <pre> <code> <h1–3> <table> <span style="color|background|font-size">`.
    - Write an ADR that records why it is not WYSIWYG.
  - **Done when.**
    - Every toolbar action round-trips: the sent HTML renders correctly in Teams web, and kelma's renderer renders it identically.
    - Unit tests cover each construct.
  - **Evidence (data layer only, toolbar UI missing, box stays open).** `crates/render/src/rich/` and `crates/render/tests/rich_format.rs`. `parse_rich_html` decodes into a richer model than `RichText` (`RichDoc`: headings 1-3 with `h4`-`h6` folded to 3, nested lists with `<ol start>`, code blocks with language, quotes, rules, tables with header cells and `colspan`/`rowspan`, plus `Style.color`, `highlight`, `size` next to bold, italic, underline, strike, code, links, mentions and emoji); `RichDoc::to_html` writes what the Teams web composer sends; `markdown_to_rich` and `markdown_to_composer_html` take the Markdown-style source (the existing `markdown_to_teams_html` syntax plus a language after the code fence) through the same model. The old `RichText` API is unchanged. Real shapes from the Teams web bundle (CKEditor 5): `strong`, `i`, `u`, `s`, `code`; `h1`-`h3` (`h4`-`h6` upcast to heading 3); code block `<pre class="language-<lang> skipProofing" spellcheck="false"><code>` with `<br>` between lines (a stored message echoes `class="language-plaintext" itemid="…"`, no `skipProofing`); table `<figure class="table"><table class="copy-paste-table">`, an empty cell gets `&nbsp;` (the `itemprop` form exists only for Kusto tables); text color `<span style="color:<css>">` and font size `x-large`/`xx-small`/`inherit`, both passed through from the editor; highlight from a fixed 8-color palette as `<span itemtype="http://schema.skype.com/HighlightColor" itemid="<slug>" style="background-color:#RRGGBB">` (any other color is a plain `background-color` span). Lists, quotes and `ol start` follow rusty-teams `draft.rs` (MIT). Read-only against 48:notes (tag shapes only, nothing stored): links carry `target`, `rel`, `title`, `itemtype=…/HyperLink`; code blocks use `<br>`. Colors are validated (`#rgb`, `#rrggbb`, `rgb()`, `rgba()` only), links to http, https, mailto and tel only, and the encoder re-validates the model, so script, event attributes, unknown styles and `javascript:` do not survive. Not run live: nothing was sent, so how Teams web renders each encoded construct is still unchecked. Still to do: the toolbar and shortcuts, the live preview strip, and the ADR (this box stays unticked until then). Tests: `cargo test -p render`.

### 5.5 Emoji like Teams
- [x] Impact 4, effort 2. Seen: emoji panel.
  - **What.**
    - The panel has an Emoji tab with categories, recent emoji and search ("Find something fun").
    - Typing `:name:` autocompletes, for example `:heart:` or `:like:`. A popup appears after `:` plus two letters.
    - A skin tone preference.
    - GIFs and stickers are out (see Later).
  - **How.**
    - Generate a static emoji table (name, keywords, category) from Unicode CLDR data at build time, with no runtime network.
    - Store recent emoji and the skin tone in config.
  - **Done when.** UI tests cover search, `:heart` autocomplete and inserting at the cursor.

### 5.6 Unread handling
- [x] Impact 4, effort 2. Seen: settings › Chats ("When I open a chat, take me to"), chat menu, shortcuts.
  - **What.**
    - A "New messages" divider sits at the read marker.
    - On opening a chat, jump to the last read message or the newest one (a setting).
    - A floating "↓ Jump to newest" button appears when scrolled up, with the count of new messages.
    - Mark all as read: Shift+Esc, and an item in the chat list "…".
    - Mark as unread from the chat item menu.
  - **How.**
    - The read marker comes from the conversation `consumptionhorizon`, which the store already reads.
    - Mark all as read PUTs the horizon on every unread conversation, throttled.
  - **Done when.** A UI test covers the divider position and the jump button; the live check marks all read and Teams web agrees.
  - **Status.** Built and unit/UI tested. The setting is `open_chat_at` in `config.json` (`first_unread`, the default, or `newest`); there is no settings page for it yet (9.x). A manual "Mark as unread" is remembered locally (`kv unread_mark:<conv>`) because the conversations list does not return the bookmark. Not run live: Mark all as read (it changes the real read state of every chat), so "Teams web agrees" is still open.

### 5.7 Drafts
- [x] Impact 4, effort 2. Seen: quick view "Drafts".
  - **What.**
    - Each chat keeps its unsent text (and reply target) when you switch chats or restart.
    - The chat list shows "Draft:" in the preview.
    - A "Drafts" quick view lists them.
  - **How.**
    - Store the draft in the store's `kv` as `draft:<conv>`, saved with a debounce from `edited`.
    - Teams syncs drafts server-side. Only discover that if it turns out to be cheap; local is enough.
  - **Done when.** A UI test types, switches chat and comes back, and the draft is still there.

### 5.8 Failed sends and offline queue
- [x] Impact 4, effort 2.
  - **What.**
    - Optimistic messages that fail show "Failed to send · Retry · Delete".
    - While disconnected, sends wait in a queue and go out when the connection comes back.
  - **How.**
    - Add a `pending` state on the row (it exists), a failure state, and a retry command.
    - The queue lives in the store so it survives a restart.
  - **Done when.** Freezing the network (as in the reconnect test) and sending queues the message, and it goes out on reconnect.
  - **Status.** Every new message (text, image, file) goes through the store's outbox; files are copied to `<cache>/outbox/<id>/` and removed once sent. A network error keeps the item queued (flushed in order on Connected, Resync, startup and every 30 s); a refusal shows "Failed to send · Retry · Delete". Teams does not deduplicate a repeated `clientmessageid`, so flushes after a restart or a lost answer read the newest history first and drop items whose echo is already there. Checked live in `48:notes` (`cargo test -p kelma live_outbox -- --ignored`); the offline path is covered by unit and engine tests, not by freezing the real network.

### 5.9 Keyboard shortcuts like Teams
- [ ] Impact 4, effort 2. Seen: the Ctrl+. dialog.
  - **What.**

    | Shortcut | Action |
    |---|---|
    | Ctrl+. | shortcut list dialog |
    | Ctrl+Alt+E | search |
    | Ctrl+Alt+G | go to chat or channel |
    | Alt+Shift+N | new chat |
    | Ctrl+Shift+F | open filter |
    | Ctrl+Alt+U | Unread filter |
    | Ctrl+Alt+C | Chats filter |
    | Ctrl+Alt+A | Channels filter |
    | Ctrl+Alt+Z | clear filters |
    | Alt+Shift+R | focus the compose box |
    | Alt+Shift+M | focus the message pane |
    | Shift+Esc | mark all as read |
    | Alt+Q | collapse all sections |
    | Ctrl+Shift+X | expand the composer |
    | Alt+Shift+O | attach a file |
    | Ctrl+Shift+I | mark important |
    | ↑ in an empty composer | edit your last message |
    | Esc | close the thread, popup or reply |

    In the message menu: R for reply, F for forward, S for save, P for pin, U for unread.
  - **How.** Add a window-level `FocusScope` in `AppWindow` that routes keys to commands.
  - **Done when.** A UI test covers each shortcut's effect.

### 5.10 Enter behavior setting
- [x] Impact 2, effort 1. Seen: settings › Chats ("When writing a message, press Enter to").
  - **What.** "Send the message" (default), or "Start a new line", where Ctrl+Enter sends.
  - **Done when.** A UI test covers both modes.

---

## Phase 6: people and conversations

### 6.1 @mentions
- [x] Impact 5, effort 3. Seen: messages, Activity, settings › Notifications (personal, Everyone, channel and team, tag).
  - **What.**
    - Typing `@` opens a picker:
      - group chats: members
      - 1:1: the other person
      - channels: org people plus "team" and "channel" entries
    - Choosing a person inserts a chip-like name token that Backspace removes as one unit.
    - @Everyone in group chats.
    - Incoming mentions of you are highlighted, and the chat shows an @ marker in the list.
  - **How (known).** Send `<span itemscope itemtype="http://schema.skype.com/Mention" itemid="N">Name</span>` plus a `properties.mentions` JSON string: `[{"@type":"http://schema.skype.com/Mention","itemid":N,"mri":"8:orgid:…","mentionType":"person","displayName":"Name"}]`. Team and channel mentions use `mentionType: "team"` or `"channel"` (verify).
  - **Done when.** A mention sent from kelma notifies the person in Teams, which is verified by mentioning yourself in a test channel.
  - **Status.** Built and tested. `@` in the composer opens a picker (group: members and Everyone; 1:1: the other person; channel: channel, team and org people; notes: you). A picked name is plain `@Name` text that one Backspace removes whole; the engine keeps who each token is and turns them into the mention spans and `properties.mentions` when sent. Mentions go through the store outbox (migration v4, `outbox.mentions`) and an edit resends them. Incoming mentions of you (or `@Everyone`) tint the bubble and put `@` in the chat list. Live: a self-mention sent through the outbox to the sandbox channel came back parsed as a mention of me and one new Mention item reached the Activity feed (`cargo test -p kelma live_people_mention -- --ignored`, message deleted afterwards). The chip is text, not a drawn pill: Slint's `TextInput` cannot style a span. A restored draft or a new chat loses the picked mentions, so they go out as plain text.

### 6.2 New chat
- [ ] DEFERRED: needs test tenant (the remaining live checks message a colleague's 1:1; built and tested offline). Impact 5, effort 3. Seen: Alt+Shift+N, "New message".
  - **What.**
    - The "To:" field searches people, chats, channels, emails and tags, with suggestions as you type.
    - Adding several people creates a group chat, with "Add group name".
    - If a chat with exactly those people exists, open it instead.
    - The thread is created when the first message is sent.
  - **How (known).**
    - People search: `GET <middleTier>/beta/users/searchV2?includeDLs=true&includeBots=true&enableGuest=true&source=newChat&skypeTeamsInfo=true` with the query.
    - Create: `POST <chatService>/v1/threads` with `{members:[{id,role}], properties:{threadType:"chat", topic?}}`. The new id is in the `Location` header.
    - A 1:1 uses the deterministic id `19:<guidA>_<guidB>@unq.gbl.spaces` and needs no create.
  - **Done when.** Starting a 1:1 with yourself and a group with one colleague works. Only do the group test with that colleague's OK; otherwise test it with the user only.
  - **Status.** Built, not ticked. Alt+Shift+N or the compose button opens the pane: To: suggestions (people search), chips, "Add group name" from two people, and a hint when a group with exactly that roster exists (it is opened instead of creating one). One person gives the deterministic 1:1 id, you alone gives `48:notes`, and the thread is created when the first message is sent. Tested offline for existing roster, new roster, 1:1 and notes. Live, with the one allowed test group, only `create_group_chat` with one colleague was run; sending to a never-used 1:1 id (which would message a colleague) and the UI path for a new 2+ person group were not. Matching the roster needs the members of every group, which come from the aggregator snapshot at sync.

### 6.3 Group chat management
- [ ] DEFERRED: needs test tenant (Add people would add a real person to a chat; built and tested offline, group photo endpoint unknown). Impact 4, effort 3. Seen: chat header (pencil, photo), the participants button ("20").
  - **What.**
    - Rename with the pencil.
    - Change the group photo.
    - The participants popover lists people with presence and work location, and has "Add people" (optionally with chat history) and "Leave".
    - The chat item menu also has Leave.
  - **How.**
    - Rename (known): `PUT /v1/threads/{id}/properties?name=topic`.
    - Add (known): `PUT /v1/threads/{id}/members/{mri}` with `{role:"User"}`.
    - Leave (known): `DELETE /v1/threads/{id}/members/{selfMri}`.
    - Group photo (discover): probably an AMS upload plus a thread property.
  - **Done when.** Rename and add/leave work on a test group created with the user only.
  - **Status.** Built, not ticked. Pencil renames in place; the people button opens the participants popover (members, Add people with an "Include chat history" switch, Leave with a confirmation); Leave is also in the chat item menu of groups. Live in the one test group: rename and leave worked. Add people was not run live (it would add a person to a chat). The group photo is not built: no endpoint is known, it needs a capture from Teams web first.

### 6.4 Presence
- [ ] DEFERRED: needs test tenant (presence and status are not allowed on the work account). Impact 5, effort 3. Seen: profile menu, participants list, avatars everywhere.
  - **What.**
    - Status dots on every avatar: green Available, red Busy or Do not disturb, yellow Away or Be right back, grey Offline, with an out-of-office marker.
    - The profile menu changes your status: Available, Busy, Do not disturb, Be right back, Appear away, Appear offline, Reset status.
    - "Set status message", with an expiry.
    - "Pause notifications".
    - Automatic Away when idle.
    - "Notify me when this person is available" (settings › Presence).
  - **How (known).**
    - Read: `POST presence.teams.microsoft.com/v1/presence/getpresence/` with `[{mri}]`, batched over visible people, plus presence change events from trouter (already registered).
    - Set: `PUT /v1/me/forceavailability/` with `{availability}`.
    - Note: `PUT /v1/me/publishnote`.
    - Idle: `POST /v1/me/reportmyactivity/`.
  - **Done when.** Dots match Teams web for 5 people, and setting Busy in kelma shows Busy in Teams web.
  - **Status.** Not built, on purpose. The presence endpoint answers 401 on the work account and presence experiments are not allowed on it, so this stays open until a token that works is available. `crates/teams-proto/src/presence.rs` is unused.

### 6.5 Profile card
- [ ] DEFERRED: needs test tenant (comparing the card needs a second account; built and tested offline). Impact 4, effort 3. Seen: avatars and names.
  - **What.** Clicking an avatar or name opens a card with:
    - a large photo, name and title
    - presence and status message
    - email (click to copy), local time and work location
    - buttons: Chat, and Email (mailto)
  - **How.** Use `fetchShortProfile`, which already exists, and add `jobTitle` and `email`. Presence comes from 6.4.
  - **Done when.** The card matches the Teams card for 3 colleagues.
  - **Status.** Built, not ticked. Clicking an avatar or a sender name opens a card with photo, name, title and department, email (click to copy), company, office, and the Chat and Email buttons; Chat opens the 1:1. Live (read-only): 3 cards read, all with an email, none with a job title, so the title row stays empty on this tenant until Delve answers. Local time is not shown (no source for a person's time zone), and presence needs 6.4. The comparison with Teams for 3 colleagues was not done.

### 6.6 Pinned messages banner
- [ ] DEFERRED: needs test tenant (comparing the banner needs a real group with a second account; built and tested offline). Impact 3, effort 2. Seen: the group chat banner ("📌 <sender> … 9/6" with "…").
  - **What.**
    - A strip under the header shows the current pinned message.
    - It cycles through all of them with a counter and click-to-jump.
    - The "…" menu has Unpin.
  - **How.** Discover the thread property; pinning itself is 5.3.
  - **Done when.** It shows the same pinned messages as Teams for a real group chat.
  - **Status.** Built, not ticked. A strip under the header shows the newest pin with `k/N`, arrows to cycle, a click jumps to the message (loading older history when needed) and moves to the next pin, and "…" has Unpin. Pins come from `Client::pinned` and are resolved from the store or up to 3 older pages. Live: in the test group, pin then unpin worked (1 then 0 pins) and 1 of 8 real chats has pins (read-only). Not compared with the Teams banner on a real group.

### 6.7 Read receipts
- [ ] DEFERRED: needs test tenant (Seen needs a second account that reads; built and tested offline). Impact 3, effort 2. Seen: settings › Chats "Read receipts".
  - **What.** On your newest message in 1:1 and small groups: ✓ for sent, an eye icon for "Seen by …". The setting turns it off, which also stops sending your receipts.
  - **How.** Probably `GET /v1/threads/{id}/consumptionhorizons` (discover or verify) to compare each member's horizon with your message.
  - **Done when.** "Seen" appears after a colleague reads (observed naturally).
  - **Status.** Built, not ticked. My newest message in a 1:1 or a group of up to 20 shows a check once sent and an eye with "Seen by …" when members' consumption horizons reach it; the horizons are polled every 20 s for the open chat. The setting is the composer "…" menu, "Read receipts". It honors the account's `readReceiptsEnabled` (read: true) and a local off switch; it does not write the account setting (no setter is known, and writing it is a change to the real account). Live: horizons read for 5 of 8 chats; "Seen" was not observed (the colleague in the test group had not read).

---

## Phase 7: finding things

### 7.1 Search page
- [ ] Impact 5, effort 4. Seen: Search (Ctrl+Alt+E).
  - **What.**
    - Results tabs: All, Messages, Files, People, Channels, Group chats, Meetings (and 2 more).
    - Filters: In (chat or channel), Person, Date.
    - Message results show a highlighted snippet; clicking jumps to the message in its chat.
    - "We couldn't find any results" empty state.
  - **How (discover).**
    - `substrate.office.com/search/api/v2/query`, with a substrate token (`https://substrate.office.com/.default`; check that the refresh grants it).
    - People results reuse `searchV2`.
    - Chats and channels can be searched locally.
  - **Done when.** Searching a phrase from the cache finds it, and the jump lands on it.

### 7.2 Find in chat
- [ ] Impact 4, effort 3. Seen: chat header 🔍 ("Find in chat"), and "Find in channel".
  - **What.** A search box above the conversation with ↑ and ↓ through matches; the list scrolls to each match and highlights it.
  - **How.** Add SQLite FTS5 over cached messages. When a match is older than the cache, page backward with the existing history paging.
  - **Done when.** A UI test plus a live check on a long chat.

### 7.3 Activity feed
Moved to its own plan: `plans/activity.md`.

### 7.4 Quick views: Mentions and Followed threads
- [ ] Impact 4, effort 3. Seen: chat list "Quick views", post menu "Follow thread", settings › Chats (auto-follow).
  - **What.**
    - Mentions lists every message that mentions you.
    - Followed threads lists the channel threads you follow, with new replies on top.
    - Channel posts have "Follow thread" and "Unfollow thread" in their menu.
    - Auto-follow covers threads you started, threads you replied to and threads where you are @mentioned (settings).
  - **How.** Mentions is a filtered Activity. Followed threads needs discovery (a thread subscription property).
  - **Done when.** Following a test post in kelma shows it in Teams' Followed threads.

### 7.5 Shared and Files tabs
- [ ] Impact 4, effort 3. Seen: chat tab "Shared", channel tab "Shared".
  - **What.**
    - A table of Name, Date shared and Shared by, filtered by Recent, Files or Links.
    - "Filter by keyword".
    - "Upload" (same flow as 5.1 and send_file).
    - Clicking opens the file or link.
  - **How.**
    - Chats: collect from cached messages (`properties.files` and links in the content).
    - Channels: Graph `GET /teams/{groupId}/channels/{channelId}/filesFolder`, then its children. `groupId` comes from CSA `teamSiteInformation` (verify).
  - **Done when.** It lists the same files as Teams for 48:notes and the test channel.

### 7.6 Chat details pane
- [ ] Impact 2, effort 2. Seen: header button "Open chat details".
  - **What.** A right panel "In this chat" with the recent shared items and "See all", which goes to 7.5.
  - **Done when.** It toggles, and its items open.

---

## Phase 8: chat list and channels

### 8.1 Custom sections
- [ ] Impact 4, effort 3. Seen: Favorites, Groups, DevOps, Operation and Chats in the list; "New section"; "Move to section" and "Remove from section".
  - **What.**
    - User sections that mix chats and channels, each collapsible.
    - "Collapse all sections" (Alt+Q).
    - New section from the new menu.
    - Move and remove from the item menu.
    - Sections sync with Teams.
  - **How.** Discover the CSA folders API (the sections already show in Teams' list). Replace the local favorites star with the real Favorites section.
  - **Done when.** kelma shows the same sections and order as Teams, and a move shows up in Teams.

### 8.2 Chat item menu
- [ ] Impact 4, effort 2. Seen: right click on a chat.
  - **What.** Mark as unread, Move to section, Remove from section, Mute, Copy link, Leave (group), Hide.
  - **How.**
    - Mute and Hide: discover (conversation properties).
    - Leave: 6.3.
    - Copy link: `https://teams.microsoft.com/l/chat/{id}/0`.
  - **Done when.** Mute in kelma silences notifications both in kelma and in Teams web.

### 8.3 Filters and list options
- [ ] Impact 3, effort 1. Seen: filter pills, "…" › Customize view, settings › Chats.
  - **What.**
    - Pills: Unread, Channels, Chats, Meeting chats, plus the filter text box.
    - Options: show message previews, show time stamps, chats and channels combined or separate.
  - **Done when.** UI tests cover each pill and toggle.

### 8.4 Channel posts like Teams
- [ ] Impact 4, effort 3. Seen: channel Posts tab.
  - **What.**
    - Posts are cards with replies shown inline under each post (collapsed after 2, with "See N more replies"), and "Reply in thread" opens an inline reply box.
    - The "Post in channel" button opens a composer with "Add a subject", formatting, and the Announcement type (headline and background banner).
    - Thread replies have "Also send to channel".
    - The menu has "See new posts at top" and "In post channels, show recent activity at the top or bottom".
    - The post menu has: Follow thread, Copy post link, Save, Pin, Mark as unread.
  - **How.**
    - Replies come from the existing `;messageid=` threads, and subjects already work.
    - Announcement and "also send to channel" need discovery.
  - **Done when.** The test channel looks and behaves like Teams.

### 8.5 Channel menu
- [ ] Impact 3, effort 2. Seen: right click on a channel.
  - **What.** Manage notifications (all activity, off, custom), Hide, Copy link, Leave channel (private channels), Go to team. Admin items stay in the browser.
  - **Done when.** Hiding and per-channel notifications work and sync.

### 8.6 Files in channels
- [ ] Impact 4, effort 3.
  - **What.** Attach in a channel uploads to the channel's SharePoint folder and posts the file card.
  - **How.** Graph `PUT /drives/{driveId}/items/{folderId}:/{name}:/content`, where the folder comes from `filesFolder` (7.5), then `properties.files` as for chats. No member invite is needed.
  - **Done when.** A test file posts in the test channel and opens for others. Delete it afterwards.

### 8.7 Attach cloud files
- [ ] Impact 3, effort 3. Seen: attach menu "Attach cloud files".
  - **What.** A picker with Recent, My files, Shared with me and team libraries; choose files, then send them as file cards.
  - **How.** Graph `/me/drive/recent`, `/me/drive/root/children` and `/me/drive/sharedWithMe`, then share with members and send as in `files.rs`.
  - **Done when.** Attaching an existing OneDrive file works in 48:notes.

### 8.8 Adaptive cards like Teams
- [ ] Impact 4, effort 3. Seen: Workflows posts (Release readiness, run reports).
  - **What.**
    - Container styles (the colored "AT RISK" band).
    - FactSet as a two-column table.
    - TextBlock sizes, weights and colors.
    - ColumnSet.
    - ActionSet as real buttons (`Action.OpenUrl` opens the browser).
    - Images.
  - **How.** Add a structured card model in `render` (instead of today's flattened text) and a `CardView` Slint component.
  - **Done when.** The Workflows cards in the test channel match Teams side by side.

---

## Phase 9: notifications, settings, polish

### 9.1 Notification settings
- [ ] Impact 4, effort 2. Seen: settings › Notifications and activity.
  - **What.**
    - Mute all.
    - Sounds on or off.
    - "Show message and content previews".
    - Test notification.
    - Per type: chat messages; @mentions (personal, Everyone, channel and team, tag); replies to followed threads; likes and reactions; posts in shown channels (with replies). Each is off, Activity only, or Activity and banner.
    - Per-chat mute (8.2) and per-channel settings (8.5).
  - **How.** Store the settings locally in config and route them through `should_notify`.
  - **Done when.** Unit tests cover the routing, and the test notification button works.
  - **Evidence (data layer only, UI missing, box stays open).** `crates/teams-proto/src/user_settings.rs`. The per-type settings are not local: they are the `userPersonalSettings` JSON string in the `GET /v1/users/ME/properties` bag. `toastSettings.<Type>` is the banner flag and `activityGenerationSettings.<Type>` the Activity-feed flag, so banner and feed is both true, feed only is the feed flag alone, off is neither (`emailSettings` mirrors the keys for mail). Types (`Kind`): `Chat`, `MentionInChat_Person_v2`, `MentionInChat_Everyone`, `Mention_Person`, `Mention_Channel`, `Mention_Team`, `Mention_Tag`, `Reply`, `ReplyToReply`, `Like`, `LikeInChat_v2`, `Follow_ChannelNewMessage`, `Follow_ChannelReplyMessage`, `Visible_ChannelNewMessage_v2`, `Visible_ChannelReplyMessage_v2`, `Visible_ChannelMention_v2`, `MeetingWentLiveNotification` (meeting started), `Priority`, `VIP`. `selectedPreset` is 1 all activity, 2 my @mentions, 3 custom (any per-type change sets 3). "Pause notifications" is `toastSettings.temporarilyPauseUntil` (ISO time). Sounds are `soundsSettings.{alertSound,incomingCallsAndMeetingStartSound,showNotificationDuringCallsAndMeetings}` and `toastSettings.{alertSoundType,alertUrgentSoundType,PlayNotificationSoundsWhenBusyOrInMeetings,PlayNotificationSoundsForUrgentAndPriority,GlobalSoundAndPreview,ShowPreviewInToasts}`. Meeting-chat notifications are `chatSettings.meetingChatNotification` (`MeetingsUserParticipatesIn`, `MeetingsUserIsInvitedTo`, `MeetingsManuallyUnmuted`) and `meetingChatNotificationRSVPv2{Accepted,Tentative,Follow,NoResponse,Declined}`. Writing is `PUT /v1/users/ME/properties?name=userPersonalSettings` with `{"userPersonalSettings":"<whole object as a string>"}`; `NotificationSettings` keeps every key it does not know, and `write_request` only builds the request (never sent). Channel settings stay with NSS (§3.8, `channel_notifications`). Quiet hours: the web client has no such server setting (no `quietHours`, `dndSchedule` or similar in the bundle), so `QuietHours` (days, start and end, overnight windows, `mobile`) is a local schedule the app must store itself. Not run live: the keyring failed twice (DBus, no reply), so the real answer shape of `userPersonalSettings` is unverified; `kelma-cli settings shape` prints key names and types only, to check it. Sources: Teams web client JS bundle (facts only), rusty-teams `chatsvc/src/settings.rs` (MIT, property-bag access adapted). Tests: `cargo test -p teams-proto user_settings`, `cargo test -p kelma-cli settings`. CLI: `settings notifications|quiet|shape` and `settings set notify|pause|sound|meeting-chat|meeting-rsvp ... --dry-run`.

### 9.2 Tray icon and badge
- [ ] Impact 4, effort 3.
  - **What.** A tray icon with the unread count, a menu (Open, Status, Quit), close to tray, and optional autostart. The taskbar badge shows the unread count.
  - **How.** Use the `ksni` crate (StatusNotifierItem), an autostart `.desktop` file, and the Unity launcher entry API for the badge.
  - **Done when.** It works on KDE Plasma.
  - **Evidence (tray crate only, app wiring missing, box stays open; same work as X3.3 in `plans/settings-and-platform.md`).** 2026-11-19: new crate `crates/tray` (`ksni` 0.3.7 over zbus 5, pure Rust, no new system package). `Tray::start(TrayOptions) -> TrayHandle` runs on its own thread with a private runtime, so it sits beside the Slint loop. `TrayHandle` (cheap to clone): `set_unread(n)` (overlay badge pixmap in 4 sizes drawn in code, title `kelma (12 unread)`, tooltip), `set_muted(bool)`, `set_launcher_count(n)`, `events()` (a cloneable crossbeam `Receiver<TrayEvent {Open, ToggleMute(bool), Quit}>`), `tray_error()`, `launcher_ready()`, `shutdown()` (removes the icon and clears the launcher badge). The icon is the `kelma` theme name with a pixmap fallback drawn from the `packaging/kelma.svg` shapes. Menu: Open kelma, Mute notifications (checkmark), separator, Quit; left click is `Open`. The badge is 0 hidden, 1 to 99 as is, `99+` above. `launcher::launcher_update` builds the `com.canonical.Unity.LauncherEntry.Update` body (`application://kelma.desktop`, `count` int64, `count-visible`), and `set_unread` sends it too unless `sync_launcher_badge` is off. `autostart::enable(exec, hidden_flag)`/`disable()`/`is_enabled()` write `$XDG_CONFIG_HOME/autostart/kelma.desktop` from the fields of `packaging/kelma.desktop` plus `X-GNOME-Autostart-enabled=true`. Live on KDE Plasma Wayland (`kelma-cli tray-demo`): the item showed up in `org.kde.StatusNotifierWatcher.RegisteredStatusNotifierItems` as `org.kde.StatusNotifierItem-<pid>-1/StatusNotifierItem` and left it on exit; `Activate` produced `Open`, the dbusmenu `Event` on Mute produced `ToggleMute(true)`; the item's Title, ToolTip and `OverlayIconPixmap` (4 sizes) followed 3, 12, 120 and 0; `dbus-monitor` saw LauncherEntry updates 3, 12, 120, 0 and a final 0 on shutdown. Autostart enabled, validated with `desktop-file-validate` and disabled under a scratch `XDG_CONFIG_HOME`. Not verified: how Plasma draws the overlay (no screenshot taken), the task manager badge on a pinned launcher, GNOME (needs the AppIndicator extension). Not done: the app has to call `Tray::start`, handle the events (show and focus the window, mute, quit), pass a `--hidden` flag it does not parse yet, close to tray, and a Settings toggle for autostart; the Flatpak uses the Background portal for autostart instead of the file. Tests: `cargo test -p tray -p kelma-cli`.

### 9.3 Appearance
- [ ] Impact 3, effort 3. Seen: settings › Appearance and Chats.
  - **What.** Theme (light, dark, follow the OS), message density (Comfy or Compact), skin tone, font size.
  - **How.** Make the `Theme` global switchable at runtime and read the OS theme through the XDG desktop portal.
  - **Done when.** Screenshots of both themes are in `docs/screenshots`.

### 9.4 Delivery options
- [ ] Impact 2, effort 1. Seen: composer "+" › "Set delivery options", Ctrl+Shift+I.
  - **What.** Standard, Important (a red "IMPORTANT!" label) and Urgent (repeated notifications on the receiving side), sent and shown.
  - **How.** Discover the exact property; purple-teams notes `importance`.
  - **Done when.** An Important message shows as important in Teams web.

### 9.5 Image viewer and grid
- [ ] Impact 3, effort 3. Seen: multi-image messages (2-column grid), link preview cards.
  - **What.**
    - Several images in one message form a grid.
    - Clicking an image opens a viewer with zoom, next and previous, Save as, and Copy image.
    - Videos and voice play inside kelma, or keep the system player as a fallback.
  - **Done when.** A UI test covers the viewer navigation.

### 9.6 Translate
- [ ] Impact 2, effort 3. Seen: "Translate" link on messages in another language, the message menu "Translation", settings.
  - **What.** "Translate" under a message in another language replaces its text, with "See original".
  - **How.** Discover the Teams translation endpoint.
  - **Done when.** An Arabic message translates to English like in Teams.
  - **Evidence (data layer only, UI missing, box stays open).** `crates/teams-proto/src/translate.rs`. Teams translates by message id, not by text: `POST CSA/api/v1/chats/<chat>/messages/languages/<to>` (channel chains: `teams/<team>/channels/<channel>/<root>/messages/languages/<to>`) with body `[{"id":"<messageId>","version":"<optional>"}]` (up to 30), headers `x-ms-use-acs-translation: true`, `x-ms-request-id` and either `x-ms-on-demand-translation: true` (the user clicked) or `x-ms-knownLanguages: en,ar` (automatic; optional `x-ms-sourceLanguage`). The answer lists `{id, version, resultCode, translatedContent, translatedSubject, translatedTitle}`. The middle tier `beta/translate` (plain text) answers 404 today, so there is no text-only call. Languages: `GET CSA/api/v1/translator/languages?locale=<ui>`. The user setting is the `userLanguageAccountSettings` key of `POST MT/beta/users/useraggregatesettings` (`defaultTranslationLanguage`, `translationPreferences.translationBehavior` Yes, No or Ask, `authoringLanguages`); when unset Teams translates to the UI language and suggests. A message's detected language is in its property `languageStamp` (`languages=de:100;length:82;&detector=Bling`); `Settings::should_translate` says when to offer the Translate link. Live, read-only: `kelma-cli translate languages [locale]` (the supported list, named in the UI locale) and `translate settings` work; the translate route accepts a request and answers 200 in the documented shape, but only a made-up message id was sent (it answers `UnknownError`), so a real translation was not checked (no real chat content translated). Still to do: expose `properties.languageStamp` on `model::Message`, then the UI link, "See original" and the Settings control. Sources: Teams web client JS bundle, rusty-teams `translate.rs` (MIT). Tests: `cargo test -p teams-proto translate`.

### 9.7 Link previews
- [ ] Impact 3, effort 2. Seen: a link preview card in a group chat.
  - **What.** URLs in messages show a preview card (image, title, description, domain), and sent links get the same preview, with a dismiss ✕ in the composer.
  - **How.** Discover the Teams URL preview service and the message property it uses. Render from the property when present; never fetch arbitrary URLs with credentials.
  - **Done when.** The same link shows the same card as Teams.
  - **Evidence (data layer only, UI missing, box stays open).** `crates/teams-proto/src/urlpreview.rs`. `GET https://urlp.asm.skype.com/v1/url/info?url=<url>` (the web client also uses `v1/url/richInfo?url=` and the `urlp`/`urlpV2` host from `regionGtms`; `https://<geo>-prod.asyncgw.teams.microsoft.com/urlp/v1/url/info` answers the same), skypetoken auth through the existing `Ams` header policy, headers `RequestId`, `CorrelationId` and `TransactionId`. The answer has `target_url` (after redirects), `title`, `description`, `site`, `favicon`, `thumbnail` (240 px) and `largethumbnail` (480 px) with `*_meta {width,height}`, and `status_code` (a page it cannot reach answers 200 with `status_code` 502 and nothing to show). The image URLs are on the same service and return raw JPEG or PNG bytes with the skypetoken; `fetch_image` only sends the token to service hosts. A sent link keeps its card in the message property `links` (`PUT …/messages/<id>/properties?name=links`, body `{"links":"[{\"@type\":\"http://schema.skype.com/HyperLink\",\"url\",\"preview\":{previewurl,previewmeta,title,description},\"previewenabled\":true}]"}`; `urlpreviews` is the newer variant); `parse_links_property` reads it and `set_message_preview` writes it (the write was not run live). Live, read-only: `kelma-cli preview https://www.rust-lang.org --image` returns title, description, site, a 240x240 thumbnail (8811 bytes downloaded) and favicon. Still to do: expose `properties.links` on `model::Message`, then render the card, the dismiss ✕ in the composer and the call on send. Sources: Teams web client JS bundle, rusty-teams `links.rs` (MIT). Tests: `cargo test -p teams-proto urlpreview`.

### 9.8 Saved messages
- [ ] Impact 2, effort 2. Seen: "Save this message".
  - **What.** A "Saved" view lists saved messages and jumps to them.
  - **How.** Needs 5.3's save property plus a query for saved messages (discover).

### 9.9 Profile menu
- [ ] Impact 2, effort 2. Seen: profile menu.
  - **What.** Name and email, presence (6.4), Pause notifications, status message, Change profile photo, View account (browser), Sign out.

### 9.10 Window state
- [ ] Impact 2, effort 1.
  - **What.** Restore the size, position, last open chat and collapsed sections on start.

---

## Later ⏭️ (dropped for now, with the reason)

- GIFs and stickers: need Giphy and Microsoft's sticker service, and add little value for work chat.
- Loop components, Notes, Storyline, Recap, custom tabs ("Add a tab"): web apps, opened in the browser.
- Record video clip, Share to Outlook, Immersive Reader, Report message: rarely used, or they need other services.
- Message extensions (Create Planner task, Jira, Polly, Workflows): third-party apps.
- Team and channel admin: new team, join team, new channel, Edit, Manage, tags, email address, Archive, Delete, Add members.
- Communities, Copilot, Meeting Recaps, work location, suggested replies, priority and blocked contacts, missed-activity emails.
- Schedule send, spell check (Slint has no hook), and Home/End in RTL lines (upstream Slint, see ADR 0001).

## Order

1. Phase 5, in order. It is daily friction and mostly local work.
2. 6.1 @mentions, 6.2 new chat and 6.4 presence, which is what a daily user misses most.
3. 7.3 Activity and 7.1 search, which need endpoint discovery.
4. The rest of phases 6–9.

## Endpoints already documented by open-source clients

These items no longer need discovery. Details, bodies and sources are in
`docs/research/open-source.md` §3. Licensing:
- rusty-teams (MIT): code may be reused with attribution.
- mautrix-teams (AGPL): use the facts, not the code.
- better-teams ("MIT + Microsoft exclusion") and teams-lite (no license): facts only.

| Item | Endpoint (chat service `CS` unless noted) | Source |
|---|---|---|
| 5.2 reactions catalog | `emotions` keys `like heart laugh surprised sad angry`, others `<hex>_<name>`; catalog JSON on `statics.teams.cdn.office.net/evergreen-assets/personal-expressions/…`; custom emoji `CSA/api/v1/customemoji/metadata` | rusty, Maxim, m0nkmaster |
| 5.3 forward | `POST /v1/users/ME/conversations/<src>/messages/forward` `{targetThreadIds, messageIds (≤5), additionalMessage, templateId:"basic_forward_message_template"}` | rusty `forward.rs` |
| 5.3 save | `PUT /v1/users/ME/conversations/<conv>/rcmetadata/<rootId>` `{"s":1,"mid":<msgId>}`; list `48:saved` | rusty `saved.rs`, m0nkmaster |
| 5.3 pin | CSA pinbar: `GET/POST/DELETE CSA/api/v1/chats/<chatId>/pinbar` (`pinbarItemType:"pinmsg"`); channel posts `…/channels/<c>/posts/<id>/pin` | rusty `pins/` |
| 5.3 / 5.6 mark unread | `PUT …/properties?name=consumptionHorizonBookmark` `"<id-1>;<now>;<cmid>"`; on read also reset it to `"0;<now>;0"` | rusty, Jezza (verified) |
| 5.4 reply with quote | `<blockquote itemtype=…/Reply itemid=…>` plus `properties.qtdMsgs` JSON string | Jezza, rusty |
| 5.7 drafts | `GET/POST/PUT/DELETE /v1/users/ME/drafts` (`draftType`, `innerThreadId`), `48:drafts` | rusty `drafts.rs` |
| 6.1 mentions | `<readonly …><span itemtype=…/Mention itemid=N>Name</span></readonly>` plus `properties.mentions` JSON string; `mentionType` `person`/`channel`/`team`; tags use `tag:<tagId>`; an edit replaces `properties`, so resend the mentions | xteams, Jezza |
| 6.2 new chat | people: substrate `POST /search/api/v1/suggestions?scenario=peoplepicker.newChat` (needs `x-anchormailbox: Oid:<oid>@<tid>`); create: `POST /v1/threads` with `threadType:"chat"`, `fixedRoster` and `uniquerosterthread` for 1:1 | better-teams, purple |
| 6.3 members | add `PUT /v1/threads/<id>/members/<mri>` `{"role":"Admin","shareHistoryTime":<ms>}`; leave `DELETE` on your own MRI; rename `PUT …/properties?name=topic` | better-teams, purple |
| 6.4 presence | read `POST presence/v1/presence/getpresence/`; push `POST …/pubsub/subscriptions/<epid>`; set `PUT …/me/forceavailability/` (an empty body clears it); note `PUT …/me/publishnote` (`<pinnednote></pinnednote>` suffix = show when messaged); idle `POST …/me/reportmyactivity/`. Writes need the **presence** token | rusty, teams-lite, purple |
| 6.5 profile card | `fetchShortProfile` (body array starting with `""`, SPACES + `x-skypetoken`); Loki `POST …/api/v2/person?smtp=` | teams-lite, m0nkmaster |
| 6.6 pinned banner | CSA pinbar, as for 5.3 | rusty |
| 6.7 read receipts | `GET /v1/threads/<id>/consumptionhorizons`; trouter `ThreadActivity/MemberConsumptionHorizonUpdate`; setting `readReceiptsEnabled` in `/v1/users/ME/properties` | teams-lite, rusty |
| 7.1 search | substrate `POST /searchservice/api/v2/query`, `entityType` Message/People/File/Chat/TeamsChannel; needs `x-anchormailbox`, otherwise it returns 200 with empty results | rusty, m0nkmaster, xteams |
| 7.2 find in chat | the same query with KQL `clientthreadid:<id>` | rusty |
| 7.4 followed threads | `48:threads`; follow and unfollow through the `emotions` key `follow` | m0nkmaster, Maxim |
| 7.5 shared tab | substrate `GET /AllFiles/api/users('OID:<oid>@<tid>')/AllShared?ThreadId=<conv>&ItemTypes=File&ItemTypes=Link` | m0nkmaster |
| 8.1 sections | `GET/POST CSA/api/v1/teams/users/me/conversationFolders` (actions AddItem, RemoveItem, ReorderItems, AddFolder; 412 means a version conflict, so retry) | rusty, better-teams |
| 8.2 mute and hide | mute `PUT …/properties?name=alerts` `{"alerts":"false"}`; hide `unpinnedTime` then `historyHiddenTime` | teams-lite, Jezza (verified) |
| 8.5 channel notifications and hide | `POST /api/nss/<geo>/v1/me/notificationSettings/channels/batch` (audience `uis.teams.microsoft.com`), `PUT …/properties?name=channelNotificationSettings`; hide a channel with `favorite` plus the member `interest` | rusty, Jezza |
| 8.6 channel files | `GET MT/beta/channels/<id>/documentlibraries` or Graph `filesFolder` | Squads |
| 8.8 card actions | `POST CS/agents/<botMri>/invoke` for `Action.Submit` | rusty `cards.rs` |
| 9.4 delivery options | `properties.importance` `high` or `urgent` on send | better-teams, Maxim |
| 9.6 translate | `POST CSA/api/v1/chats/<chatId>/messages/languages/<to>` with `x-ms-use-acs-translation: true`; languages `GET CSA/api/v1/translator/languages` | rusty `translate.rs` |
| 9.7 link previews | `GET <geo>-prod.asyncgw.teams.microsoft.com/urlp/v1/url/info?url=`; attach with `PUT …/messages/<id>/properties?name=links` | rusty `links.rs` |
| 9.8 saved view | `48:saved` (see 5.3) | rusty |
