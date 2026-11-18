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

- [ ] **T1.1 "Your teams and channels" page.** Impact 4, effort 2.
  - **What.** The Teams, Pending invites and Analytics tabs, the filters, sort and search, and per-team rows with their channels.
  - **How.** Graph `/me/joinedTeams`, `/teams/{id}/channels` and `/teams/{id}/members` (counts), plus CSA (already used).
- [ ] **T1.2 Discover and join.** Impact 3, effort 3.
  - **What.** Browse public teams and join them; "Join team" with a code.
  - **How.** Browsing: `GET /groups?$filter=resourceProvisioningOptions/Any(x:x eq 'Team') and visibility eq 'Public'`. Joining: T0.
- [ ] **T1.3 Channel list control.** Impact 3, effort 2.
  - **What.**
    - Show and hide channels and teams.
    - See all channels, with the hidden ones.
    - Leave a channel (private) or a team.
    - Go to team.
  - **How.** T0 (user state); leaving uses `DELETE /teams/{id}/members/{me}`.

## Phase T2: create and manage

- [ ] **T2.1 New channel.** Impact 4, effort 2.
  - **What.**
    - Name, description.
    - Type: Standard, Private (choose members) or Shared.
    - Posts or threaded-chat layout.
    - "Show for everyone automatically".
  - **How.** `POST /teams/{id}/channels` with `membershipType` and `members` (granted).
- [ ] **T2.2 Edit, delete, restore and archive channels; moderation.** Impact 3, effort 3.
  - **What.**
    - Edit name and description.
    - Delete (with Deleted channels and Restore within 21 days).
    - Archive a channel.
    - Moderation: who can start posts, reply permissions, bot posts, moderators.
  - **How.** T0 (the Graph scopes are missing).
- [ ] **T2.3 Members.** Impact 4, effort 2.
  - **What.** Team and channel members with Owner / Member / Guest roles: add, remove, change role, pending requests.
  - **How.** `/teams/{id}/members` and `/channels/{id}/members` (granted). Inviting a new guest goes to the web host.
- [ ] **T2.4 Tags.** Impact 3, effort 3.
  - **What.** Create, edit and delete tags, and add members. Tags can be @mentioned in chat (chat plan 6.1).
  - **How.** T0.
- [ ] **T2.5 Channel email address.** Impact 2, effort 2.
  - **What.** Get the address, copy it, remove it, and restrict senders.
  - **How.** Read it from Graph `channel.email`; write through T0.
- [ ] **T2.6 New team.** Impact 3, effort 3.
  - **What.** From scratch (private, public or org-wide), from a group or team, or from a template; name, description, privacy.
  - **How.** T0, with the web host as a fallback.
- [ ] **T2.7 Manage team.** Impact 3, effort 4.
  - **What.**
    - Members, pending requests, channels.
    - Settings: team picture, member permissions, guest permissions, @mentions, team code, fun stuff.
    - Apps, tags, analytics.
    - Archive and unarchive; delete the team; get a link to the team.
  - **How.** T0, with the web host as a fallback.
- [ ] **T2.8 Channel notifications** (all activity, off, custom) and per-channel settings. Shared with the chat plan.
- [ ] **T2.9 Storyline posts** (the "New storyline post" in the new menu) through the Viva Engage token (`plans/store-and-apps.md`). Impact 2, effort 3.

## Phase T3: Communities

- [ ] **T3.1** The Communities section in the chat list (All Company, Join communities), opening a community's feed natively through `api.yammer.com`, with a web host fallback. Impact 3, effort 3.

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
