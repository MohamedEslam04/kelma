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
| Read marker | `PUT …/{id}/properties?name=consumptionhorizon` `{"consumptionhorizon":"<id>;<ms>;<cmid>"}` | ⚠️ |
| Typing | `POST …/{id}/messages` `{messagetype:"Control/Typing", contenttype:"Application/Message", content:""}` | ⚠️ |
| Self | `GET /v1/users/ME/properties` → `skypeName` (`orgid:<guid>`) | ✅ |

### Channels ✅ read / ⚠️ post

- The channel id is `19:…@thread.tacv2`. History returns root posts and replies mixed together.
  Each message's `conversationLink` ends with `;messageid=<rootId>`, and a root's
  `rootMessageId` equals its own `id`.
- Reading a thread: `GET …/conversations/{channel};messageid={root}/messages`.
- Posting a reply: `POST …/conversations/{channel};messageid={root}/messages`.
- Posting a new post: `POST …/conversations/{channel}/messages`, with `properties.subject` for the
  title.
- Posting into a real channel was not tested, because a test post is visible to colleagues.

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
- **Files:** sending real file attachments goes through OneDrive/SharePoint (Graph scopes). That is
  not implemented: kelma shows file cards and opens them in the browser.

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
