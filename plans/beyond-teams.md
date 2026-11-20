# kelma: beyond Teams

The other plans chase Teams parity. This one holds what kelma does that Teams does not: small
features that make a desktop client faster to live in. None of them writes anything new to
Teams. Their state stays on this computer (the store's key/value table or the config
directory), per account where it belongs to an account.

Each item has the same parts as the other plans: a checkbox, impact and effort (1–5),
**What**, **How**, **Done when** and **Status** with evidence.

Live checks run on the integrated build `feat/call-settings` 89dbf86. Until they are done, the
items below are ticked on the unit and UI tests alone.

---

## B1. Catch-up

- [x] Impact 5, effort 2.
  - **What.**
    - Alt+Down / Alt+Up opens the next or previous unread chat in the order the list draws them, with no wrap around. The end of the list shows a notice.
    - A "Catch up · N" chip above the chat list opens the first unread chat, @mentions first. It is hidden when nothing is unread.
    - After a long time away, a "While you were away" card waits for the first sync, then offers to start catching up. Dismissing it lasts for the session.
  - **How.** `src/catch_up.rs` (pure: next, previous and chip target over the drawn rows); the effects live in `ui/app.slint`.
  - **Done when.** Alt+Down walks every unread chat once and stops at the end, from any view.
  - **Status.** Built. Evidence: unit tests in `catch_up.rs` (`next_is_the_first_unread_below_the_open_chat`, `there_is_no_wrap_around`, `the_chip_prefers_a_mention_then_the_first_unread`…) and UI tests in `ui_tests/catch_up.rs` (`alt_down_and_alt_up_walk_the_unread_chats`, `alt_down_works_from_another_view_and_brings_the_chat_list_back`, `the_away_card_waits_for_the_first_sync_and_starts_catching_up`…).

## B2. Remind me and Snooze

- [x] Impact 4, effort 3.
  - **What.**
    - "Remind me" in a message's menu: in 20 minutes, in 1 hour, in 3 hours or tomorrow at 9:00. It fires as a desktop notification whose "Open" jumps to the message and "Snooze 1 hour" moves it.
    - "Snooze" in the chat list menu: for 1 hour, until tomorrow 9:00 or until next week (Monday 9:00). The chat leaves the list and the unread counts until its time, then comes back marked unread.
    - A Reminders panel lists both, with Cancel for a reminder and Unsnooze for a chat.
    - A custom time is not offered yet.
  - **How.** `backend/remind.rs`, `ui/remind.rs`, `ui/remind.slint`. Saved as JSON under the kv key `remind:<own MRI>`, so accounts never mix. A 15 second tick fires what is due, and what came due while kelma was closed fires once at start. Commands run in their own `remind` lane.
  - **Done when.** A reminder set for a minute ahead fires once, and a snoozed chat returns unread at its time, also across a restart.
  - **Status.** Built. Evidence: unit tests in `backend/remind.rs` (`tomorrow_is_nine_the_next_local_day_also_across_midnight`, `next_week_is_the_next_monday_at_nine`, `only_reminders_at_or_before_now_are_due_oldest_first`, `snoozed_chats_leave_the_list_until_their_time`…).

## B3. Command palette

- [x] Impact 4, effort 2.
  - **What.** Ctrl+Shift+P opens a palette of actions (go to a view, open a Settings page, set status or theme, new chat, keyboard shortcuts, Calls when available) followed by matching chats. Enter runs the best match. Actions used before come first. Arabic labels are matched in Arabic.
  - **How.** `ui/palette.slint` (`PaletteCatalog`, labels are `@tr`), ranking in `src/ui/palette.rs`. Running an action reuses the callback or store command that already does the same thing (`run-palette` in `ui/app.slint`), so the palette sends nothing of its own.
  - **Done when.** Every action in the catalogue does the same thing as its button or menu item.
  - **Status.** Built. Evidence: ranking tests in `ui/palette.rs` (`prefix_beats_word_start_beats_inside_beats_subsequence`, `arabic_labels_match_on_the_translated_text`…) and UI tests in `ui_tests/palette.rs` (`ctrl_shift_p_opens_the_palette_and_esc_or_the_chord_closes_it`, `enter_runs_the_best_match_and_closes`, `used_actions_come_first_when_the_palette_opens_again`…).

## B4. Quiet hours, Focus mode and keyword alerts

- [x] Impact 4, effort 3.
  - **What.** Settings › Notifications and activity:
    - Quiet hours: days and a window (overnight windows run into the next morning). No banners inside it.
    - Focus mode for a chosen time (also from the tray): only @mentions of me, urgent or important messages, priority people, calls and keyword hits get a banner.
    - Keyword alerts: a message from someone else with one of the words shows a banner even in a muted chat, but not during quiet hours.
  - **How.** `backend/quiet.rs`: `should_banner` is the one decision, made in `live.rs` where a banner is raised. "Paused" (`settings::notifications_paused`, the Teams pause and the tray's Mute) beats everything. Nothing writes to Teams or changes the presence: messages and unread counts still arrive, only the banner and its sound are held back. Prefs live in `GeneralPrefs` (`quiet_hours`, `focus_until_ms`, `focus_duration`, `keyword_alerts`); the tray sends `TrayEvent::ToggleFocus`.
  - **Done when.** A message inside quiet hours raises no banner, and a keyword hit in a muted chat does.
  - **Status.** Built. Evidence: unit tests in `backend/quiet.rs` (`an_overnight_window_runs_into_the_next_morning`, `focus_ends_at_its_time`…) and the UI test `quiet_hours_and_keyword_alerts_save_to_the_temp_home` in `ui_tests/settings.rs`.

## B5. Offline message search

- [x] Impact 4, effort 3.
  - **What.** Search finds cached messages at once and without a network, by words of the text or the sender's name. Words match as prefixes and all are required. Arabic matches across letter variants (أ إ آ ا, ى ي, ة ه) and tashkeel.
  - **How.** Store migration v7: an FTS5 table `messages_fts` (unicode61, remove_diacritics) kept by triggers. The triggers call Rust SQL functions from `store/src/fts.rs` (registered by `fts::register`) that strip markup and fold the text; the same folding runs on the query, and the snippet is mapped back to the message's own spelling. `Store::search_messages_local` feeds the Search page's local hits before Substrate adds older ones. Only the store crate opens SQLite.
  - **Done when.** With the network off, a phrase from a cached message is found and the jump lands on it.
  - **Status.** Built. Evidence: store tests `search_finds_words_of_the_text_and_the_sender_with_a_marked_snippet`, `words_match_as_prefixes_and_all_are_required` and `arabic_matches_across_letter_variants_and_tashkeel` in `crates/store/src/tests.rs`.

## B6. Snippets (saved quick replies)

- [x] Impact 3, effort 2.
  - **What.** Typing `/brb` at the start of a line offers the snippets whose name fits; arrows or Tab choose, Enter inserts, Escape closes. `{first}` becomes the other person's first name in a 1:1 chat and `{me}` my own. URLs and paths never open the picker. Settings › Chats › Snippets adds, edits and deletes them.
  - **How.** `appearance/src/snippets.rs` (the list, kept in `snippets.json` in the config directory, per device; built-in examples until the list is first saved), `src/snippets.rs` (pure matching and placeholders, plus `install`), `ui/snippet_picker.slint`, `ui/settings_snippets.slint`.
  - **Done when.** A saved snippet is inserted with its placeholders filled, and the list survives a restart.
  - **Status.** Built. Evidence: UI tests in `ui_tests/snippets.rs` (`slash_br_then_enter_puts_the_snippet_in_the_composer`, `placeholders_use_the_other_persons_first_name_in_a_one_to_one_chat`, `urls_and_paths_do_not_open_the_picker`, `settings_add_edit_delete_persist_and_reload`…).

---

## Later

- [ ] **B7. Global "show kelma" shortcut.** Impact 3, effort 3. A system-wide key that raises the window or starts a new chat. **How:** the xdg-desktop-portal `GlobalShortcuts` interface (KDE and GNOME 48+), with nothing on desktops that lack it. Not started.

Two items of the same list were built on other branches and are tracked there: "Pop out chat" (`ui/chat_window.slint`) and spell check (Settings › Chats › Check spelling, `ui/spelling.slint`).
