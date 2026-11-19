# kelma plan: Teams and channels management

Part of the Teams parity plans (see `plans/README.md`). Format as in `plans/chat.md`. Chat inside
channels (posts, replies, files, cards) is in `plans/chat.md` phase 8. This file covers management.

**Sources.**
- UI survey (2026-11-18):
  - "Your teams and channels": Teams, Pending invites and Analytics tabs; All, Teams you own and 3 more filters; sort A–Z; Search teams; Create team ▾; rows with name, public or private, people, channels and role; See all channels; See all your teams.
  - The channel menu: Manage notifications, Move to section, Hide, Add members, Edit channel, Manage channel, Manage channel tags, Copy link, Get email address, Workflows, Go to team, Leave, Archive, Delete.
  - The new menu: New channel, New storyline post, Join team, New team, Join communities.
  - The Communities section ("All Company").
- `docs/research/apps-onedrive-copilot-store.md` §5.
- The token probe: Graph `Team.ReadBasic.All`, `TeamMember.ReadWrite.All`, `ChannelMember.ReadWrite.All`, `Channel.Create`, `Channel.ReadBasic.All` and `Group.Read.All` are granted.

**Missing Graph scopes.**
- `Team.Create`, `TeamSettings.ReadWrite.All`, `ChannelSettings.ReadWrite.All`, `Channel.Delete.All`, `TeamworkTag.ReadWrite`, `Group.ReadWrite.All`.
- Those operations use the internal endpoints the Teams web client calls (discover). The SharePoint `Sites.FullControl.All` token may also cover parts of them.

## Phase T0: discovery

- [x] **T0.1** Capture the web client creating a team (in a throwaway private team the user then deletes), joining with a code, editing, archiving and deleting a channel, moderation, tags, the channel email address, hide and show, and team settings. Document all of it.
  - **Evidence.** Taken from the web client's own JS service chunks instead of a HAR, then every call run live in the sandbox team. Documented in `docs/protocol.md` § Teams and channels, with live status per endpoint. Team archive (MT 500) and storyline post are not verified.

## Phase T1: see and join

- [ ] BLOCKED: pending join requests need the owner-side `findJoinRequests` body, unknown after nine read-only tries (400/500); the rest of the page is built and tested. **T1.1 "Your teams and channels" page.** Impact 4, effort 2.
  - **What.** The Teams, Pending invites and Analytics tabs, the filters, sort and search, and per-team rows with their channels.
  - **How.** Graph `/me/joinedTeams`, `/teams/{id}/channels` and `/teams/{id}/members` (counts), plus CSA (already used).
  - **Status.** Done: the Teams, Discover, Analytics and Communities tabs, filters, sort, search, rows with their channels (`ui_tests::teams::page_lists_teams_with_counts_and_filters_sorts_and_searches`, `expanding_a_team_shows_all_its_channels_with_the_hidden_one_flagged`, `tabs_load_their_data_and_analytics_counts_the_list`; live: the list loads and filters in the sandbox run).
  - **BLOCKED: pending invites.** The tab lists only the join requests made in this session (with Cancel). Nothing returns the invites sent to you, and the owner view `POST MT/beta/teams/findJoinRequests` answered `500 groupList cannot be null` for nine body shapes (`docs/protocol.md`). Not ticked until a body is found.
- [x] **T1.2 Discover and join.** Impact 3, effort 3.
  - **What.** Browse public teams and join them; "Join team" with a code.
  - **How.** Browsing: `GET /groups?$filter=resourceProvisioningOptions/Any(x:x eq 'Team') and visibility eq 'Public'`. Joining: T0.
  - **Evidence.** `ui_tests::teams::discover_lists_public_teams_and_joins_requests_or_cancels` and `join_with_a_code_sends_the_code_and_shows_the_answer`; live check in the sandbox team: `cargo test -p kelma live_teams -- --ignored` with `KELMA_SANDBOX_FILE`: suggestions and keyword search listed, and a join by the sandbox team's own code answered with its info. Joining or requesting a real team was not run (rule: no joins), so `joinRequest` is covered by unit tests of its URL only.
- [x] **T1.3 Channel list control.** Impact 3, effort 2.
  - **What.**
    - Show and hide channels and teams.
    - See all channels, with the hidden ones.
    - Leave a channel (private) or a team.
    - Go to team.
  - **How.** T0 (user state); leaving uses `DELETE /teams/{id}/members/{me}`.
  - **Evidence.** `ui_tests::teams::channel_menu_shows_hides_and_pins`, `team_menu_goes_to_the_team_hides_it_and_leaving_asks_first`, `hidden_channels_and_teams_leave_the_sidebar_and_pinned_come_first`; live check in the sandbox team: `cargo test -p kelma live_teams -- --ignored` with `KELMA_SANDBOX_FILE`: hide, show, pin, unpin, "show for everyone" on and off, for a standard and a private channel. Leaving a team or a private channel is unit and UI tested only: it would remove the account from the sandbox team.

## Phase T2: create and manage

- [x] **T2.1 New channel.** Impact 4, effort 2.
  - **What.**
    - Name, description.
    - Type: Standard, Private (choose members) or Shared.
    - Posts or threaded-chat layout.
    - "Show for everyone automatically".
  - **How.** `POST /teams/{id}/channels` with `membershipType` and `members` (granted).
  - **Evidence.** `ui_tests::teams::new_channel_dialog_sends_name_description_layout_and_default`, `private_channel_asks_for_the_roster_and_picks_people`; live check in the sandbox team: `cargo test -p kelma live_teams -- --ignored` with `KELMA_SANDBOX_FILE`: a standard channel in the threaded layout and a private channel were created and removed. A shared channel is UI and unit tested only.
- [x] **T2.2 Edit, delete, restore and archive channels; moderation.** Impact 3, effort 3.
  - **What.**
    - Edit name and description.
    - Delete (with Deleted channels and Restore within 21 days).
    - Archive a channel.
    - Moderation: who can start posts, reply permissions, bot posts, moderators.
  - **How.** T0 (the Graph scopes are missing).
  - **Evidence.** `ui_tests::teams::edit_channel_sends_the_new_name_and_description`, `delete_and_archive_ask_first_and_deleted_channels_can_be_restored`, `moderation_dialog_changes_who_can_post_reply_and_pin`; live check in the sandbox team: `cargo test -p kelma live_teams -- --ignored` with `KELMA_SANDBOX_FILE`: rename (standard and private), moderation stored and read back, archive and unarchive, delete, listed under deleted, restore.
- [ ] BLOCKED: pending requests need the same unknown `findJoinRequests` body; add, remove and role change are built but not run live (no member changes on the work tenant). **T2.3 Members.** Impact 4, effort 2.
  - **What.** Team and channel members with Owner / Member / Guest roles: add, remove, change role, pending requests.
  - **How.** `/teams/{id}/members` and `/channels/{id}/members` (granted). Inviting a new guest goes to the web host.
  - **Status.** Done in the UI: team and channel rosters with names, add (people search), change role, remove (`ui_tests::teams::members_dialog_changes_roles_removes_and_adds_people`, `a_plain_member_cannot_change_roles_or_remove_team_members`). Live in the sandbox: rosters of the team and of a private channel, and the people search (read only). Adding, removing and changing members were not run live (rule: no changes to the sandbox members).
  - **BLOCKED: pending requests.** Same endpoint as T1.1: `findJoinRequests` answers `500 groupList cannot be null` for every body tried. Guests: the invite call is not wired.
- [x] **T2.4 Tags.** Impact 3, effort 3.
  - **What.** Create, edit and delete tags, and add members. Tags can be @mentioned in chat (chat plan 6.1).
  - **How.** T0.
  - **Evidence.** `ui_tests::teams::tags_dialog_creates_selects_adds_removes_and_deletes`; live check in the sandbox team: `cargo test -p kelma live_teams -- --ignored` with `KELMA_SANDBOX_FILE`: create (with ourselves as first member), add, rename, delete. A deleted tag keeps its name until purged, so creating the same name again answers `409 DeletedTagHasSameTagName`; the dialog shows that text.
- [x] **T2.5 Channel email address.** Impact 2, effort 2.
  - **What.** Get the address, copy it, remove it, and restrict senders.
  - **How.** Read it from Graph `channel.email`; write through T0.
  - **Evidence.** `ui_tests::teams::email_dialog_gets_copies_restricts_and_removes_the_address`; live check in the sandbox team: `cargo test -p kelma live_teams -- --ignored` with `KELMA_SANDBOX_FILE`: address created, senders changed to members, address removed. Copy goes to the clipboard through `arboard`, not run headless.
- [ ] DEFERRED: needs test tenant (creating a team outside the sandbox is not allowed on the work account; the dialog and name check are built, create was run once in T0.1). **T2.6 New team.** Impact 3, effort 3.
  - **What.** From scratch (private, public or org-wide), from a group or team, or from a template; name, description, privacy.
  - **How.** T0, with the web host as a fallback.
  - **Status.** Done: the New team dialog (name, description, private, public or org-wide), which checks the name first (`ui_tests::teams::new_team_dialog_sends_name_description_and_privacy`; the create call itself was run live by T0.1). Not run again live: a new team is created outside the sandbox team. Not wired: from a group, a team or a template.
- [ ] BLOCKED: archive team answers `500 ArchiveUnarchiveEnqueuePsOperationFailed` on every try; delete is UI-tested only because the sandbox must stay. Details, permissions, code, members, tags and deleted channels work. **T2.7 Manage team.** Impact 3, effort 4.
  - **What.**
    - Members, pending requests, channels.
    - Settings: team picture, member permissions, guest permissions, @mentions, team code, fun stuff.
    - Apps, tags, analytics.
    - Archive and unarchive; delete the team; get a link to the team.
  - **How.** T0, with the web host as a fallback.
  - **Status.** Done in the UI and live in the sandbox: edit name and description, member and guest permissions (10 and 3 stored keys), the team code (generate, reset, remove), members, tags, deleted channels, link to the team (`ui_tests::teams::manage_team_edits_details_code_permissions_and_copies_the_link`).
  - **BLOCKED: archive team.** `POST MT/beta/teams/<id>/archive` answers `500 ArchiveUnarchiveEnqueuePsOperationFailed` on three more tries from the UI command. The chat-level flag does not archive the group, so the UI menu offers it and shows that error. Delete team is UI and unit tested only (the sandbox team must stay). Not wired: team picture, apps, retention labels.
- [x] **T2.8 Channel notifications** (all activity, off, custom) and per-channel settings. Shared with the chat plan.
  - **Evidence.** `ui_tests::teams::notification_dialog_sets_the_level_and_replies`; live check in the sandbox team: `cargo test -p kelma live_teams -- --ignored` with `KELMA_SANDBOX_FILE`: banner and feed, off and feed set and read back.
- [ ] DEFERRED: needs test tenant (the read-only settings query is verified live; the one throwaway draft post waits for the test tenant, by the user's choice). **T2.9 Storyline posts** (the "New storyline post" in the new menu) through the Viva Engage token (`plans/store-and-apps.md`). Impact 2, effort 3.
  - **Read-only check, live (2026-11-19).** `kelma-cli team storyline-settings` runs the Engage web app's own `StorylineSettingsClients` query (text and persisted-query hash from its public bundle) as `POST https://engage.cloud.microsoft/graphql` with a new `Audience::Engage` token (resource `https://www.yammer.com`, sent only to `engage.cloud.microsoft`). Answer: storyline access true, can post true, announcements false, stories true. This settles the token audience and the transport (query text plus `extensions.persistedQuery` is accepted). Tests: `engage::tests::storyline_settings_*`, `client::tests` (Engage owns only the GraphQL host).
  - **Still open.** The draft itself (`createUserWallMessage` with `isDraft:true`, then `deleteThread`) is a write; the user chose to run it on the test tenant, not the work account.
  - **Before (2026-11-19).** It cannot be verified without a write. The only unpublished state is a draft (`createUserWallMessage` with `isDraft:true` over Engage GraphQL), which is still a write on the work account, so it waits for the user to approve one throwaway draft. The current REST body is a guess that may post to All Company: `team storyline` now refuses to run. Evidence: `docs/research/storyline.md`.

## Phase T3: Communities

- [x] **T2.10 Channel tabs and "Add a tab".** Impact 4, effort 2. Seen: the channel header (Posts, Files, then the tabs and "+").
  - **What.** The channel header lists the channel's other tabs (Notes, Planner, websites, apps) after Posts and Files; "+" adds a tab.
  - **How.** Graph `GET teams/{group}/channels/{channel}/tabs?$expand=teamsApp` (the middle tier's `beta/teams/…/tabs` answers 405); a website tab is `POST …/tabs` with the website app (`com.microsoft.teamspace.tab.web`) bound.
  - **Evidence (2026-11-19).** `teams_proto::channel_tabs` (tests for parsing, the Files tab left out whatever its app id, the website body), `backend/channel_tabs.rs`, the header tabs and the Add a tab dialog in `ui/find.slint` / `app.slint`; UI test `a_channel_shows_its_other_tabs_and_add_a_tab_sends_a_name_and_address`. Live, read only: `kelma-cli team tabs` on the sandbox channel listed "Notes (OneNote), opens www.onenote.com" and the Files tab (now filtered). Clicking a tab opens its website, else the tab in Teams on the web. Adding a tab was not run live: Teams posts "… added a tab" in the channel, which has other people in it. Apps' tab pages do not open inside kelma (the web host covers personal tabs only).
- [x] **T3.1** The Communities section in the chat list (All Company, Join communities), opening a community's feed natively through `api.yammer.com`, with a web host fallback. Impact 3, effort 3.
  - **Evidence.** `ui_tests::teams::communities_tab_lists_communities_and_opens_a_read_only_feed`, `the_chat_list_has_a_communities_section_that_opens_the_feed`; live check in the sandbox team: `cargo test -p kelma live_teams -- --ignored` with `KELMA_SANDBOX_FILE`: the community list and the first page of its feed loaded (read only). Each post has an "Open in browser" link as the web fallback.

## Order

T1.1, T2.1 and T2.3, then T0, then the rest.

## Endpoints already documented by open-source clients

See `docs/research/open-source.md` §3.18. These remove most of T0.

| Item | Endpoint (`MT` = `regionGtms.middleTier`) | Source |
|---|---|---|
| T1.1 list | `GET CSA/api/v3/teams/users/me?…` (teams, channels, folders, chats in one call) | rusty |
| T1.3 hide channel, pinned channels | channel `favorite` plus member `interest`; `GET/POST/DELETE CSA/api/v1/teams/users/me/pinnedChannels` | Jezza, rusty |
| T2.1 new channel | `POST MT/beta/teams/<teamThreadId>/channels` `{displayName, description, groupId, channelType}`; private channels `POST teams.microsoft.com/fabric/<geo>/templates/api/teams/<groupId>/channels` (audience `templates.teams.microsoft.com`) | lazyteams (GPL), rusty |
| T2.2 edit and delete channel | `PATCH` and `DELETE MT/beta/teams/<t>/channels/<c>` | lazyteams |
| T2.3 members | `PUT MT/beta/teams/<t>/bulkUpdateRoledMembers?allowAsyncAddition=true` `{users:[{mri,role}], groupId}`; leave `PUT MT/beta/teams/<t>/members` `{userMri, updateType:"Left", groupId}` | lazyteams, AADInternals |
| T2.4 tags | `GET MT/beta/teams/<groupId>/tags`; mention MRI `tag:<tagId>` | m0nkmaster |
| T2.6 new team | `POST MT/beta/teams/create` | lazyteams |
| T2.7 delete team | `DELETE MT/beta/teams/<t>/delete` | lazyteams |
