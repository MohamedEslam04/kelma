# kelma plan: Activity

Part of the Teams parity plans (see `plans/README.md`). Format as in `plans/chat.md`. It replaces
item 7.3 of the chat plan.

**Sources.** The survey of the Activity rail item in Teams web (2026-11-18), with an unread badge on
the rail.
- Header: "Activity", "…" (Mark all as read Shift+Esc, Notification settings), and search.
- Filters: Unread (Ctrl+Alt+U), @Mentions (Ctrl+Alt+M), Tag mentions.
- Item types seen:
  - "X posted a new message" (followed channels), with a channel path such as "<Team> › <channel>"
  - "X reacted to your message", with the emoji and "In chat with you"
  - "X mentioned you" and "X mentioned Everyone"
  - Workflows cards ("Workflows posted a new message · Card")
  - replies to your messages and followed threads
- Right pane: a preview of the conversation around the item, with an inline reply box ("Send to: thread only", or also the channel).

## Phase A1

- [x] **A1.1 Activity feed.** Impact 5, effort 3.
  - **What.**
    - A rail item with an unread badge.
    - The list shows each item's avatar with a type badge (mention @, reaction emoji, reply, post), a bold title when unread, a preview line, the time, and where it happened.
    - Clicking an item marks it read and opens the preview.
  - **How.**
    - Read the system conversation `48:notifications` through the chat service (`/v1/users/ME/conversations/48:notifications/messages`). It is the feed Teams uses, and each item's JSON carries the activity type, the source thread and message, and the actor.
    - Trouter `NewMessage` events on `48:notifications` update it live.
    - Read state is the consumption horizon of `48:notifications`.
  - **Done when.** The last 20 items match Teams exactly, including reactions and Workflows cards.
- [x] **A1.2 Filters.** Impact 4, effort 1. Unread, @Mentions and Tag mentions, with their shortcuts.
- [x] **A1.3 Preview pane with reply.** Impact 4, effort 2.
  - **What.**
    - The right pane shows the source conversation scrolled to the item, with the item highlighted.
    - For channel posts there is a reply box with "Send to: thread only / Also send to channel".
    - "Go to chat" or "Go to channel" opens it in Chat.
  - **How.** Reuse the chat message view in a compact mode.
- [x] **A1.4 Mark all as read** (Shift+Esc) and per-item mark as read or unread.
- [x] **A1.5 Notification settings link** that opens the notification settings (chat plan 9.1).
  - **Evidence.** 2026-11-19: Activity "…" › Notification settings opens Settings on Notifications and activity (`ui/settings.rs` handles `activity-settings` with `SettingsStore.show("notifications")`, replacing the old "coming with Settings" banner). Verified: `activity_notification_settings_entry_opens_the_notifications_page` clicks the real menu. Tests: `cargo test -p kelma settings`.
- [x] **A1.6 Search in Activity.**
- [x] **A1.7 Badge sync.** The rail badge and tray count equal the unread Activity count plus unread chats, like Teams.

## Order

A1.1, then A1.2–A1.4, then the rest.

## Endpoints already documented by open-source clients

See `docs/research/open-source.md` §3.9.
- **Feed:** `48:notifications`, with `properties.activity` as an object or a JSON string: `{activityType, activitySubtype, sourceThreadId, sourceMessageId, sourceReplyChainId, sourceUserId, sourceUserImDisplayName, sourceThreadTopic, messagePreview, activityTimestamp}`, plus `properties.isread`.
- **Activity types:** mention, reply, reaction, everyoneMention, teamMention, channelMention, tagMention, missedCall, channelNewMessage, meeting and app notifications.
- **Mark one read:** `PUT …/48:notifications/messages/<id>/properties?name=isread` with `"true"`.
- **Mark all read:** `consumptionHorizonBookmark` `"0;<now>;0"` on `48:notifications`.
- **Filters:** `48:mentions` (full content) for @Mentions.
- **Sources:** rusty `activity_feed.rs` (MIT), teams-lite, better-teams.
