# Teams chat protocol notes

Everything here is the **internal** API of the Teams web client. Microsoft does not document it
and can change it at any time. kelma implements it in `crates/teams-proto`. The baseline comes
from [purple-teams](https://github.com/EionRobb/purple-teams) (GPL-3.0). The rest comes from
observing a real work account.

Status legend:
- ✅ verified against a real account (date)
- 🧪 unit-tested with fixtures only
- ⚠️ unverified

Fixtures: `crates/teams-proto/src/*` tests hold sanitized JSON. Raw captures (`fixtures/raw/`,
`*.har`) are git-ignored. They contain message content and must never be committed.

## Sign-in ✅ 2026-11-18

| Step | Request | Notes |
|---|---|---|
| Device code | `POST login.microsoftonline.com/organizations/oauth2/devicecode`, form `client_id=1fec8e78-…&resource=https://api.spaces.skype.com` | v1 endpoint. `interval` and `expires_in` may be strings |
| Poll | `POST …/oauth2/token`, `grant_type=urn:ietf:params:oauth:grant-type:device_code` | `authorization_pending`, `slow_down`, `expired_token` |
| Refresh | `POST …/oauth2/v2.0/token`, `grant_type=refresh_token&scope=<resource>/.default openid profile offline_access` | Done once per resource (spaces, `chatsvcagg`, presence). Persist the rotated refresh token |
| Skype token | `POST teams.microsoft.com/api/authsvc/v1.0/authz` with `Authorization: Bearer <spaces token>` | `tokens.skypeToken`, `tokens.expiresIn`, `region`, `regionGtms.chatService`, `regionGtms.middleTier` |

The region matters. The test tenant is in `amer`, while purple-teams hardcodes `apac`. kelma takes
the chat-service and middle-tier hosts from `regionGtms`.

## Chat service (`<chatService>` = `https://<region>.ng.msg.teams.microsoft.com`)

Headers: `X-Skypetoken`, `BehaviorOverride: redirectAs404`, `X-Stratus-Caller: skypeteams`,
`Origin`/`Referer: https://teams.microsoft.com`.

| Action | Request | Status |
|---|---|---|
| Conversations | `GET /v1/users/ME/conversations?view=msnp24Equivalent&pageSize=200&startTime=1&targetType=…` | ✅ |
| History | `GET /v1/users/ME/conversations/{id}/messages?view=msnp24Equivalent\|supportsMessageProperties&pageSize=N&startTime=1[&endTime=ms]` | ✅ |
| Send | `POST …/{id}/messages` `{clientmessageid, content, messagetype:"RichText/Html", contenttype:"text", imdisplayname[, properties.subject]}` | ✅ (self notes) |
| Edit | `PUT …/{id}/messages/{msgId}` `{content, messagetype, contenttype, imdisplayname}` | ✅ (self notes) |
| Delete | `DELETE …/{id}/messages/{msgId}?behavior=softDelete` | ✅ (self notes) |
| React | `PUT …/{id}/messages/{msgId}/properties?name=emotions` `{"emotions":{"key":"like","value":<ms>}}` | ✅ (self notes) |
| Un-react | `DELETE …/{id}/messages/{msgId}/properties?name=emotions` `{"emotions":{"key":"like"}}` | ✅ (self notes) |
| Read marker | `PUT …/{id}/properties?name=consumptionhorizon` `{"consumptionhorizon":"<id>;<ms>;<cmid>"}` | ✅ (horizon equals the newest message after the app opened the chat) |
| Typing | `POST …/{id}/messages` `{messagetype:"Control/Typing", contenttype:"Application/Message", content:""}` | ⚠️ |
| Self | `GET /v1/users/ME/properties` → `skypeName` (`orgid:<guid>`) | ✅ |

### Channels ✅

- The channel id is `19:…@thread.tacv2`. History returns root posts and replies mixed together.
  Each message's `conversationLink` ends with `;messageid=<rootId>`, and a root's
  `rootMessageId` equals its own `id`.
- Reading a thread: `GET …/conversations/{channel};messageid={root}/messages`.
- Posting a reply: `POST …/conversations/{channel};messageid={root}/messages`.
- Posting a new post: `POST …/conversations/{channel}/messages`, with `properties.subject` for the
  title.
- Posting verified 2026-11-18 in a test channel: a new post with `properties.subject` (stored as `subject`, `rootMessageId` equal to its own id) and a reply in its thread.

### Message content

| `messagetype` | Rendering |
|---|---|
| `RichText/Html`, `Text` | HTML subset: `<p> <b> <i> <a> <ul> <pre>`, mentions `itemtype=…/Mention`, emoji `<img itemtype=…/Emoji alt=…>`, reply `<blockquote itemtype=…/Reply>` |
| `RichText/Media_Card` | `<URIObject type="SWIFT.1">…<Swift b64="…">` holds the Adaptive Card JSON in base64. Some messages carry `properties.cards` instead ✅ |
| `properties.cards` → `application/vnd.microsoft.card.audio` / `.video` | Voice message or video. `content.media[0].url` is an AMS object and `duration` is ISO-8601 ✅ |
| `properties.files` | JSON string, `[{fileName, fileType, objectUrl, fileInfo.shareUrl}]` (SharePoint/OneDrive) ✅ |
| `properties.emotions` | Array, or a JSON string of an array, of `{key, users:[{mri,time,value}]}` ✅ |
| `properties.edittime` / `deletetime` | Edited / deleted markers ✅ |
| `ThreadActivity/*`, `Event/Call` | System lines |

## Aggregator (CSA) ✅

`GET teams.microsoft.com/api/csa/api/v1/teams/users/me?isPrefetch=false&enableMembershipSummary=true`,
with `Authorization: Bearer <chatsvcagg token>`.

The response has `teams[].channels[]` and `chats[]` (`title`, `isOneOnOne`, `members[].mri`).

`hidden: true` is set on many active 1:1 chats, so it is not a visibility flag. Use
`isConversationDeleted` instead.

## Profiles and avatars ✅

- Profiles: `POST <middleTier>/beta/users/fetchShortProfile?…` with a JSON array of MRIs. The
  response is `value[]` with `displayName` and `email`.
- Avatars: `GET <middleTier>/beta/users/{selfGuid}/profilepicturev2/{mri}?size=HR64x64`.

## AMS (images, voice) ✅

- **Download:** `GET https://{us,eu,as,…}-api.asm.skype.com/v1/objects/{id}/views/imgo` (or
  `/views/audio`) with `Cookie: skypetoken_asm=<skypetoken>`. kelma only sends this credential to
  AMS hosts.
- **Upload an image:**
  1. `POST as-prod.asyncgw.teams.microsoft.com/v1/objects/` `{type:"pish/image", permissions:{<conv>:["read"]}, filename, sharingMode:"Inline"}` → `id`
  2. `PUT …/v1/objects/{id}/content/imgpsh` with the bytes
  3. Send `<img itemtype="http://schema.skype.com/AMSImage" src=".../views/imgo">` ✅ (self notes)
- **Files** are not AMS: see [Files](#files--2026-11-18) below.

## Files ✅ 2026-11-18

Chat files live in the sender's OneDrive, in `Microsoft Teams Chat Files`. The Teams client id
gets a Graph token through the usual refresh (`scope=https://graph.microsoft.com/.default`), and it
carries `Files.ReadWrite.All`.

1. Upload:
   - Up to 4 MB: `PUT graph.microsoft.com/v1.0/me/drive/root:/Microsoft Teams Chat Files/<name>:/content?@microsoft.graph.conflictBehavior=rename`.
   - Larger files: `POST …:/createUploadSession`, then `PUT` chunks (multiples of 320 KiB) with
     `Content-Range` to the returned `uploadUrl`. That URL is pre-authenticated, so it gets no token.
2. `GET /me/drive/items/{id}?$select=id,name,webUrl,sharepointIds`.
3. Share with the other chat members: `POST /me/drive/items/{id}/invite`
   `{recipients:[{objectId}], requireSignIn:true, sendInvitation:false, roles:["write"]}`. The object
   id is the GUID in `8:orgid:<guid>`. Self notes skip this step.
4. Send a message with empty content and `properties.files` = a JSON string of
   `[{"@type":"http://schema.skype.com/File", id: listItemUniqueId, baseUrl: siteUrl/, objectUrl: webUrl, fileName, fileType, fileInfo:{fileUrl, siteUrl, shareUrl}, …}]`.

Verified in self notes: a 55-byte file with an Arabic name, and a 5.5 MB file uploaded in two
chunks. The sizes in OneDrive matched, and both came back as file cards. The invite step (step 3)
is unit-tested only, because a live test would share a file with a colleague. Channel files live
in the team's SharePoint site and are not supported: kelma says so instead of sending.

## Auth broker ✅ 2026-11-18

`teams_proto::broker` mints one bearer per audience from the refresh token on first use, caches it with
a wall-clock expiry (refreshed 5 minutes early) and rotates the refresh token on every mint.
`Client::request_as(audience, …)` only attaches the bearer to an https URL on that audience's own host.
`kelma-cli audiences` minted all of these live: `api.spaces.skype.com`, `chatsvcagg`, `presence`,
Graph, `substrate.office.com`, `substrate.office.com/sydney`, `outlook.office.com`, `api.yammer.com`,
`service.flow.microsoft.com`, `tasks.office.com`, `onenote.com`, `ic3.teams.office.com`,
`loki.delve.office.com`, `teams.microsoft.com`, and the user's own SharePoint (OneDrive) host.
Access tokens live about 2 hours (`ttl_s` ≈ 7465 in the "tokens refreshed" log).

## Message actions (`teams_proto::actions`) 2026-11-18

All on the chat service unless noted. Verified live in `48:notes` and a private test channel; test
messages were deleted afterwards.

| Action | Request | Live |
|---|---|---|
| Forward | `POST /v1/users/ME/conversations/<src>/messages/forward` `{targetThreadIds, messageIds (≤5), additionalMessage, templateId:"basic_forward_message_template"}` → `[{statusCode, threadId, messageId}]`. The new message carries `<blockquote itemtype="http://schema.skype.com/Forward">` and `properties.forwardTemplateId`/`originalMessageContext` | ✅ 200 |
| Save / unsave | `PUT …/conversations/<conv>/rcmetadata/<rootId>` `{"s":1\|0,"mid":<msgId>}` | ✅ 200 |
| Saved list | `GET …/conversations/48:saved/messages` | ❌ 404 `LocationLookupFailed` for this tenant, and the Teams web app no longer shows a Saved view. kelma keeps its own saved list in the store and still sends the server flag |
| Pin for everyone | CSA `POST/DELETE /api/csa/api/v1/chats/<chat>/pinbar` `{pinbarItemType:"pinmsg", pinbarOperation:"add"\|"remove", …}`; list `GET` same URL (`children[]`, folders nest) | ✅ pin, list, unpin |
| Mark as unread | `PUT …/properties?name=consumptionHorizonBookmark` `"<prevId>;<nowMs>;<cmid>"`. `mark_read` now also writes `"0;<nowMs>;0"`, because a read horizon alone does not clear the bookmark | ✅ 200 |
| Mute | `PUT …/properties?name=alerts` `{"alerts":"false"}` (`"true"` unmutes) | unit-tested |
| Hide | `PUT …/properties?name=unpinnedTime` `{unpinnedTime:<ms>}` then `historyHiddenTime` `"<ms>"`; unhide `{unpinnedTime:null}`. A hidden chat comes back with its next message | unit-tested |

## OneDrive (`crates/onedrive`) ✅ 2026-11-18

All calls go through the auth broker. Graph calls use the Graph bearer (`Files.ReadWrite.All`,
`Sites.ReadWrite.All`). The recycle bin uses the bearer for the user's own `<tenant>-my.sharepoint.com`
host. The bearer is only attached to the audience's own host, and pre-authenticated URLs (upload
sessions, copy monitors, download redirects) get no token. `kelma-cli drive …` runs the read-only
calls, and `kelma-cli drive selftest` runs every write below inside one throwaway folder and purges
it afterwards.

**O0 finding.** The plan assumed `/me/drive/recent`, `/me/drive/sharedWithMe` and `/me/insights/used`
return nothing. On 2026-11-18 they all answered with data (recent 140 items, shared 500, insights
2 per page), so Home and Shared use them and fall back to Search only when they are empty or fail.
The internal endpoints that the Teams web OneDrive app calls were **not captured** (no browser
capture was run), and none is needed. If Microsoft retires the Graph endpoints, capture the web app
and swap them in `browse::recent` and `browse::shared_with_me`.

Browse (Graph):

| Call | Use | Status |
|---|---|---|
| `GET /me/drive?$select=id,name,webUrl,driveType` | the user's drive | ✅ |
| `GET /me/drive/root?$select=sharepointIds` | site URL and host of the `-my` site (`siteUrl`) | ✅ |
| `GET /me/drive/root/children?$expand=thumbnails&$top=200`, `GET /drives/{d}/items/{i}/children` | My files and folders; `@odata.nextLink` pages | ✅ |
| `GET /me/drive/root:/<path>` and `…:/children` | open a path (for example `Recordings`; 404 when it does not exist) | ✅ |
| `GET /me/drive/following` | Favorites | ✅ (0 items; one appeared after `follow`) |
| `GET /me/followedSites` | Libraries | ❌ 403 `accessDenied` for this token |
| `GET /me/joinedTeams?$select=id`, `GET /groups/{id}/sites/root`, `GET /groups/{id}/drive` | fallback for Libraries and Quick access: the teams' sites and Documents libraries | ✅ |
| `GET /sites/{id}/drives` | libraries of a site | ✅ |
| `GET /me/drive/recent` | Home / Recent | ✅ 140 items (no `$top`: it answers the whole list) |
| `GET /me/drive/sharedWithMe` | Shared with you | ✅ 500 items, no paging link |
| `POST /search/query` `{requests:[{entityTypes:["driveItem","listItem"], query:{queryString}, from, size, sortProperties?}]}` | search; `sortProperties:[{name:"lastModifiedDateTime",isDescending:true}]` is the recent fallback; `* AND NOT path:"<my site url>"` is the shared fallback | ✅ search and sorted search; the exclusion query is accepted (200, 50 hits) but its result was not compared with the real list |
| `GET …/thumbnails` | small, medium and large URLs; `$expand=thumbnails` on listings gives them in one call | ✅ |
| `GET /me/insights/used` | not used | ✅ answers |

A search hit is a `driveItem` (with `parentReference.driveId` and `id`) or a `listItem` whose file
name is in `fields.fileLeafRef` and whose drive item is in `driveItem.id`. Remote items (shared
entries, shortcuts) carry the target in `remoteItem`; kelma points `DriveItem.id` and `drive_id`
at the target and keeps the stub in `DriveItem.stub`. Delete or rename the stub, never the target.

Act (Graph):

| Call | Use | Status |
|---|---|---|
| `POST …/children` `{name, folder:{}, "@microsoft.graph.conflictBehavior":"rename"}` | new folder | ✅ |
| `PUT …/<parent>:/<name>:/content?@microsoft.graph.conflictBehavior=rename` | upload up to 4 MB | ✅ |
| `POST …/<parent>:/<name>:/createUploadSession`, `PUT` chunks to `uploadUrl` | larger files (shared with chat files, `teams_proto::files::upload_chunks`) | ✅ 4.5 MB; two chunks verified earlier |
| `PATCH …/items/{id}` `{name}` | rename | ✅ |
| `PATCH …/items/{id}` `{parentReference:{id}, name?}` | move inside a drive | ✅ |
| `POST …/items/{id}/copy` `{parentReference:{driveId,id}, name?}` | copy: 202 with a `Location` monitor URL. Poll it with no token and no redirects until `status: completed` + `resourceId` (or a 303 whose `Location` ends in the item id) | ✅ |
| `DELETE …/items/{id}` | delete (goes to the recycle bin) | ✅ |
| `GET …/items/{id}/content` | download; answers 302 to a pre-authenticated URL | ✅ byte-identical round trip |
| folder download | the client walks `children` and builds a zip (`zip` crate) | ✅ |
| `POST …/items/{id}/follow`, `…/unfollow` | Favorites | ✅ (the list shows the change after a delay) |
| `POST …/children` `{name, remoteItem:{id, parentReference:{driveId}}}` | add a shortcut | ❌ 400 `invalidRequest` in 5 body variants (id plus drive id, with `folder`, `sharepointIds`, both, the whole remote item) against a folder shared with the user. Not usable until a working body is captured |
| `GET …/items/{id}/versions`, `POST …/versions/{id}/restoreVersion` | version history | ✅ list, 🧪 restore (a new file has one version) |
| `POST …/items/{id}/preview` `{}` | embeddable `getUrl` | ✅ for docx, xlsx, pptx |
| `GET …/content?format=pdf` | server-side PDF, used to prove the blank Word and PowerPoint files are valid | ✅ |
| `GET …/workbook/worksheets` | proves the blank Excel file opens (an empty sheet gives HTTP 406 for `format=pdf`) | ✅ |

Blank documents (`onedrive::package`) are generated in code: Word (3 parts), Excel (5 parts) and
PowerPoint (11 parts: presentation, master, layout, slide, theme and their relationships). Word and PowerPoint convert to
PDF on the server, Excel opens in the workbook API, and all three preview. Markdown and Text are
empty files.

Share (Graph):

| Call | Use | Status |
|---|---|---|
| `POST …/invite` `{recipients:[{email}\|{objectId}], message, requireSignIn, sendInvitation, roles:["read"\|"write"]}` | share with people | 🧪 (a live call would share a file) |
| `POST …/createLink` `{type:"view"\|"edit"\|"blocksDownload", scope:"anonymous"\|"organization"\|"users", expirationDateTime?}` | link; blocking download is the type `blocksDownload`, valid for view links only | 🧪 |
| `GET …/permissions` | who has access | ✅ |
| `PATCH …/permissions/{id}` `{roles:[…]}`, `DELETE …/permissions/{id}` | change or remove access | 🧪 |

Recycle bin (SharePoint REST on `<tenant>-my.sharepoint.com`, `Accept: application/json;odata=nometadata`):

| Call | Use | Status |
|---|---|---|
| `GET <siteUrl>/_api/web/RecycleBin?$top=500` | list: `Id`, `LeafName`, `DirName`, `Size`, `DeletedDate`, `DeletedByName`, `ItemType` (5 = folder) | ✅ |
| `POST <siteUrl>/_api/web/RecycleBin('<Id>')/restore` | restore to the original place | ✅ |
| `POST <siteUrl>/_api/web/RecycleBin('<Id>')/deleteObject` | delete for good | ✅ |

Deleting a folder puts one folder entry (and, depending on timing, its children) in the bin.

Around files: meeting recordings are the children of `Recordings` in My files (✅ 3 items). Sync is
a text (`onedrive::sync_help`) that points at the `onedrive` client and `rclone`.

Pitfalls found live:
- A `reqwest` error message contains the request URL, and upload session and monitor URLs carry a
  temporary `tempauth` token. kelma strips the URL from those errors.
- Uploads and the recycle bin call sometimes ran past the client's 60 s timeout during the live runs.
  Retrying worked.

## Real-time (trouter) ✅

1. `POST go.trouter.teams.microsoft.com/v4/a?epid=<epid>` → `socketio`, `surl`, `ccid`, `connectparams`.
2. `GET {socketio}socket.io/1/?v=v4&<connectparams>&tc=…&con_num=…&epid&ccid&auth=true&timeout=40` → `sid:…`.
3. WebSocket `wss://…/socket.io/1/websocket/{sid}?<same query>`.
4. On `1::`, send `user.authenticate` (Bearer spaces token) and `user.activity`, then register 3
   apps at `teams.microsoft.com/registrar/prod/V2/registrations`. `TeamsCDLWebWorker` must be
   registered last.
5. Ping every 30 s with `5:<n>+::{"name":"ping"}`. Ack every `3:::{id,…}` with
   `3:::{"id":<id>,"status":200,"body":""}`.
6. Bodies may be gzip (`X-Microsoft-Skype-Content-Encoding`), or wrapped in `cp` (gzip+base64) or
   `gp` (base64).
7. The registrar throttles at `HTTP 429 {"retryAfter":"00:02:52"}`. kelma keeps the socket open
   and retries after that delay. `trouter.message_loss` arrives right after connecting, so
   kelma re-registers at most once a minute and resyncs history.
8. After suspend, the server resets the socket (`Connection reset by peer`). kelma reconnects and
   resyncs. This was verified by freezing the process for 3 minutes: a message sent during the
   freeze was in the cache after resume, and a message sent after resume arrived live. Token expiry
   uses wall-clock time, because a monotonic clock stops during suspend.
