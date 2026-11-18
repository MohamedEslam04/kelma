# kelma research: OneDrive / Copilot / Recaps / Store / Teams-channel management / Settings

Date of research: 2026-11-18. Method: Microsoft Learn, support.microsoft.com, Message Center mirrors, community write-ups.
Legend: [V] = verified against a fetched Microsoft page; [S] = secondary/community source; [K] = from prior knowledge, NOT re-verified this session (check before building); [TBV] = to be verified by traffic capture or a live call.

## Held Graph scopes (baseline, from the brief)
AppCatalog.Read.All, Calendars.ReadWrite, Channel.Create, Channel.ReadBasic.All, ChannelMember.ReadWrite.All, ChannelMessage.Read.All, ChannelMessage.Send, ChatMessage.Send, Contacts.ReadWrite.Shared, Files.ReadWrite.All, FileStorageContainer.Selected, Group.Read.All, Mail.ReadWrite, Mail.Send, Notes.ReadWrite.All, Organization.Read.All, People.Read, Place.Read.All, Sites.ReadWrite.All, Tasks.ReadWrite, Team.ReadBasic.All, TeamMember.ReadWrite.All, TeamsActivity.Send, TeamsAppInstallation.ReadForTeam, TeamsTab.Create, TeamsTab.ReadWrite.All, User.ReadBasic.All.

Important constraint: the scope set is fixed by what the first-party client id (1fec8e78-...) is pre-authorized for. We cannot add scopes by asking nicely; a scope not in that list will fail at token mint (AADSTS65001/consent) because no tenant admin can consent for a first-party app. So every "MISSING scope" below means: use the Teams internal APIs instead (the same ones chat already uses), a web fallback, or drop the feature.

---

# 1. OneDrive app in Teams

## 1.1 What the Teams app exposes (UI inventory)
Source: support.microsoft.com "Explore the Files list in Microsoft Teams" [V], UC Today / Windows Central launch coverage [S]. The OneDrive app replaced the old "Files" app in the left rail (Dec 2023 new Teams, early 2024 classic). Requires OneDrive for Business (work/school), not personal.

Left navigation inside the app:
- Home: recently viewed/edited files; redesign adds "For you" AI recommendations with thumbnails, file activity updates, and Loop, Lists, Power BI, Whiteboard content regardless of storage location [S].
- My files: root of the user's OneDrive (folders + files).
- Shared: sub-views "Shared with you" and "Shared by you" (files/folders you created that are shared) [V].
- Favorites: files/folders marked favorite (same favorites list used by OneDrive web, File Explorer, M365 apps) [S].
- Recycle bin: deleted files with restore / delete forever [S].
- Quick access: files in specific Teams channels the user belongs to (channel document libraries) [V].
- Browse files by: People (who it was shared between), Meetings (where shared), Media (file type) [V].
- Libraries (SharePoint document libraries of followed sites/teams): no Microsoft support page found describing a "Libraries" nav item in the Teams app; in OneDrive web it exists as "Libraries" in the left rail. Treat as optional/phase 2 [S/TBV].
- Mobile equivalent: Places, Search, Recent [V].

Content-pane behaviour:
- List/grid view, sort by name/modified/modified by/size, column picker, group by, filter bar (file type, people, modified date, meetings) [K].
- Selection toolbar: Open, Share, Copy link, Download, Delete, Move to, Copy to, Rename, Version history (via "..."), Details, Favorite/unfavorite, Add shortcut to My files [K].
- + New: Folder, Word/Excel/PowerPoint/OneNote/Forms/Loop/Whiteboard/Lists, link; Upload: files, folder [K].
- Open actions: open in Teams (in-app Office viewer/editor tab), open in desktop app, open in browser, preview (inline viewer) [K].
- Details pane: properties, activity feed, manage access, "Has access" list [K].
- Search box: searches file names/content across OneDrive + SharePoint (Microsoft Search) [K].
- Sync: "Sync" button launches the OneDrive sync client via odopen:// protocol; kelma cannot do it natively (no Linux first-party sync). Alternative: link to `onedrive` Linux client or rclone docs [K].
- Meeting recordings: new recordings live in the organizer's OneDrive under `Recordings/` (channel meetings: channel SharePoint library `Recordings` folder); audio recaps under `Recordings/AudioRecaps` (kept 60 days) [V: intelligent-recap doc]. Transcripts are stored with the recording (OneDrive/SharePoint) [V].

## 1.2 Graph / API mapping
Base: `/me/drive`, `/drives/{id}`, `/sites/{id}/drive(s)`, `/groups/{id}/drive`, `/shares/{id}`, `/me/drive/special/{name}` [V onedrive overview].

| Feature | Endpoint | Scope (held?) | Notes |
|---|---|---|---|
| My files list | `GET /me/drive/root/children` ; `GET /me/drive/items/{id}/children` (+`$top`, `@odata.nextLink`) | Files.ReadWrite.All (yes) | |
| Home / Recent | `GET /me/drive/recent` | Files.Read (yes) | DEPRECATED, "degraded until November 2026, then stops returning data" [V]. As of today (2026-11-18) assume dead/empty. Replacement: Graph Search `POST /search/query` entityTypes `driveItem` with `region` + sort/filters (no 1:1 replacement, community consensus [S]); or `GET /me/drive/root/delta` and sort by lastModifiedDateTime client side; or the internal Office "recommended/MRU" service Teams itself uses [TBV]. |
| Shared with you | `GET /me/drive/sharedWithMe?allowexternal=true` | Files.Read.All (yes via ReadWrite.All) | DEPRECATED, stops returning data Nov 2026 [V]. `/me/insights/shared` also deprecated, same date [V]. Replacement: Search API (`driveItem` + query like `*` with KQL `-path:"{myOneDriveUrl}"`), or substrate/ OneDrive "shared" internal API used by Teams [TBV]. |
| Shared by you | no first-class API. Options: Search `driveItem` filtered to own drive + inspect `GET /me/drive/items/{id}/permissions` (expensive), or OneDrive internal `_api/v2.0/drive/...?$filter=shared` [TBV] | | Build client-side index via delta + permissions only for items with `shared` facet (driveItem.shared is present on shared items) [K]. |
| Favorites | `GET /me/drive/following` ; `POST /me/drive/items/{id}/follow` ; `POST .../unfollow` | Files.ReadWrite.All (yes) | Exists on v1.0 and beta [S]; Microsoft says delegated support works for work/school. NOTE: "following" is the Graph name for OneDrive Favorites (this is the same list as the "Favorites" nav). Verify it matches web Favorites [TBV]. |
| Recycle bin list | OneDrive/SharePoint REST: `GET https://{tenant}-my.sharepoint.com/_api/v2.0/drive/special/recyclebin`? [TBV]; Graph has restore only: `POST /drives/{d}/items/{id}/restore` (beta) | | Graph docs: restore of driveItem is "OneDrive Personal only" [S]; permanentDelete beta `POST .../permanentDelete` [S]. Recycle-bin LISTING is documented only for SharePoint Embedded containers (`/storage/fileStorage/containers/{id}/recycleBin/items`) [S]. So for OneDrive for Business the listing must use SharePoint REST (`/_api/web/RecycleBin` with a SharePoint-audience token) or the Teams-internal call. Needs a SharePoint-resource token (https://{tenant}-my.sharepoint.com/.default) minted from the same refresh token: [TBV whether first-party client id allows]. |
| Libraries (SP sites) | `GET /me/followedSites` (v1.0) ; `GET /sites/{id}/drives` ; `GET /groups/{teamId}/drive` ; for channel: `GET /teams/{t}/channels/{c}/filesFolder` | Sites.ReadWrite.All (yes) ; Team.ReadBasic.All | Quick access = channel filesFolder per channel. |
| Search | `POST /search/query` (entityTypes: driveItem, listItem, site) or `GET /me/drive/root/search(q='..')` | Files.Read.All / Sites.Read.All (yes) | Search API is the right primary search; drive `search(q=)` is OneDrive-only and slower to index [K]. |
| Download | `GET /drives/{d}/items/{id}/content` (302 to pre-authed URL) or `@microsoft.graph.downloadUrl` | yes | Folder download: no API; zip client side by recursing children. Convert: `/content?format=pdf`. |
| Upload small | `PUT /drives/{d}/items/{parent}:/{name}:/content` (<=250 MB) | yes | |
| Upload large / folder | `POST .../createUploadSession` then chunked `PUT` (multiples of 320 KiB, <=60 MiB per chunk) | yes | Folder upload = recursive mkdir + per-file sessions. |
| New folder | `POST .../children` body `{name, folder:{}, "@microsoft.graph.conflictBehavior":"rename"}` | yes | |
| New Office doc | `PUT .../{name}.docx:/content` with an empty template file (ship 3 minimal OOXML blobs in kelma) | yes | There is no Graph "create blank doc" action; Office web does it internally. |
| Rename / Move | `PATCH /drives/{d}/items/{id}` body `{name}` / `{parentReference:{id}}` | yes | |
| Copy | `POST .../copy` body `{parentReference:{driveId,id}, name}` -> 202 + monitor URL | yes | async, poll Location. |
| Delete | `DELETE .../items/{id}` (to recycle bin) | yes | |
| Share / copy link | `POST .../createLink` body `{type:view|edit, scope:anonymous|organization|users, expirationDateTime, password(personal only)}` [V] | Files.ReadWrite (yes) | Anonymous scope may be disabled by admin. `embed` only OneDrive personal [V]. |
| Share to people | `POST .../invite` (`recipients`, `roles`, `requireSignIn`, `sendInvitation`, `message`) | yes | |
| Manage access | `GET .../permissions`; `PATCH .../permissions/{id}` (roles); `DELETE .../permissions/{id}`; `POST /shares/{encodedUrl}/permission/grant` | yes | |
| Version history | `GET .../versions` [V, Files.Read]; restore: `POST .../versions/{id}/restoreVersion`; version content: `GET .../versions/{id}/content` | yes | Newest-first; `$orderby` unsupported [V]. |
| Details pane | `GET .../items/{id}?$expand=thumbnails,permissions` + `lastModifiedBy`, `createdBy`, `size`, `parentReference.path`, `file.mimeType` | yes | Activity feed: `GET .../items/{id}/activities` (beta, `ItemActivity`) [K]. |
| Preview | `POST .../items/{id}/preview` -> `getUrl` (embeddable iframe URL, Office viewer) | yes | Works for Office/PDF/images; webview needed. Alt: thumbnails `GET .../thumbnails/0/{small|medium|large}`. |
| Open in Office web | `driveItem.webUrl` ; `?web=1` ; for edit-in-desktop `ms-word:ofe|u|{url}` (no Linux handler) | | |
| Delta sync (for local cache) | `GET /me/drive/root/delta` (token) | yes | Use for incremental index. |
| Add shortcut to My files | `POST /me/drive/root/children` with `remoteItem` (OneDrive Business supports via beta "createShortcut"/`remoteItem`) [K] | yes | |
| Meeting recordings folder | `GET /me/drive/root:/Recordings:/children` | yes | Plus channel meetings in team drive `Recordings`. |

Scope gaps for area 1: none of the file operations lack a scope. Gaps are API removals (recent, sharedWithMe, insights) and the recycle-bin list.

## 1.3 Feasibility (OneDrive)
- Overall: HIGH. Files.ReadWrite.All + Sites.ReadWrite.All cover browse, CRUD, upload, share and versions natively.
- Native rendering: file list, details pane, share dialog, version list, upload/download can be fully native in Rust. Preview/edit of Office docs needs a webview or "open in browser" (Office web apps are not embeddable natively). Image/PDF/text previews can be done natively via downloaded content.
- Riskiest items: (a) Home/Recent and Shared-with-you after the November 2026 retirement of `recent`, `sharedWithMe` and `insights` (Search API substitute; Teams itself likely uses an internal Substrate/OneDrive endpoint, capture it); (b) Recycle bin list; (c) Shared-by-you; (d) Favorites parity.
- "Sync" is not feasible natively; offer "Open in browser" / "Download" / documented rclone/onedrive-linux.
- Phase suggestion: P1 My files + Quick access (channel files) + Download/Upload/New folder/Rename/Move/Delete/Copy link; P2 Search, Shared, Favorites, Version history, Details; P3 Recycle bin, Recent, Home "For you", Office-doc creation.

---

# 2. Copilot app in Teams (Microsoft 365 Copilot Chat)

## 2.1 What it is / features
- First-party Teams app, pinned automatically in the left rail for users with a Microsoft 365 Copilot license; others can pin from Apps or an admin can pin via setup policy [V: MC922627].
- "Microsoft 365 Copilot Chat" (renamed "Microsoft Copilot Chat" in some 2026 docs [S]) chat UI: multi-turn chat, history sidebar (pinned/recent chats), prompt gallery (Copilot Lab), file upload/attach, image generation, Copilot Pages (Loop-based durable canvas), agents (declarative agents; agent store), web grounding, and, with the paid license, work grounding (mail, files, meetings, chats, people via Microsoft Graph + Work IQ).
- 2026 changes: Work/Web toggle replaced by "Work IQ" that auto-decides work vs web [S, single source]. MC1477999 (Sep 2026): the Copilot entry point in the chat list is being removed in Oct 2026; the Copilot app is the full experience [S].
- In-meeting and in-chat Copilot (separate from the app): "Copilot in Teams" bot, interaction history appClass `IPM.SkypeTeams.Message.Copilot.Teams` / `...BizChat` [V].

## 2.2 Licensing
- Free tier: YES, Copilot Chat is included at no extra cost for Entra (work/school) users with an eligible Microsoft 365 / Office 365 subscription. It is web-grounded, has enterprise data protection, file upload, image generation, Pages and declarative agents grounded on instructions + public websites. It cannot reach org data (mail/files/chats) unless the user uploads it; custom agents are pay-as-you-go [S per Microsoft overview snippets, learn.microsoft.com/copilot/overview + support.microsoft.com "Get started with Microsoft Copilot Chat"].
- Paid add-on (Microsoft 365 Copilot): unlocks Graph/Work-IQ grounding, Copilot in Teams meetings/chats, recaps AI, agents with org data.
- Retrieval API and meeting-insights API both require the paid license (below).

## 2.3 How the Teams client talks to it
Not documented by Microsoft. Community research (Zenity labs, PyRIT) shows [S]:
- WebSocket: `wss://substrate.office.com/m365chat/SecuredChathub/{userObjectId}@{tenantId}?X-ClientRequestId=..&X-SessionId=..&access_token=<substrate token>&source=teamshub&scenario=teamshub&X-variants=..` (BizChat web uses `source=officeweb`).
- Token audience `https://substrate.office.com/sydney`, found in `teams.microsoft.com` local storage.
- Protocol: SignalR-style JSON over WebSocket (frames delimited by 0x1E record separator): handshake `{"protocol":"json","version":1}`, ping type 6, invocation type 4 `target:"chat"` with `arguments[0]` = {source, sessionId, clientCorrelationId, conversationId, traceId, isStartOfSession, optionsSets, sliceIds, allowedMessageTypes, productThreadType:"Office", clientInfo, plugins[], message{author,text,locale,locationInfo,messageType,requestId,...}}; server streams type 1 partials, type 2 final (messages with `text`, `adaptiveCards`, `sourceAttributions`, `suggestedResponses`), then type 3.
- Plugin/agent discovery: `POST https://substrate.office.com/search/api/v1/userconfig` with `RequestedConfigTypes:["CopilotPlugins"]`, `Scenario.Name:"sydney/officeweb"`.
- Unknowns [TBV]: whether the 1fec8e78 refresh token can mint `https://substrate.office.com/.default` or `.../sydney/.default` (most first-party Teams clients can, since Teams itself calls substrate for search/people; chat-list "Copilot" and the Copilot app use it), exact `X-variants`/`optionsSets`, file upload protocol, conversation history endpoints (likely substrate `/m365chat/...` REST), Pages. Capture the Teams web Copilot app with the browser MCP (`read_network_requests`) to pin all of it.
- Risk: undocumented, can change without notice; may violate ToS for third-party clients; server may reject mismatched `X-variants`/client fingerprint; paid-vs-free entitlement is enforced server-side per user.

## 2.4 Official Graph Copilot APIs
Docs live under learn.microsoft.com/microsoft-365/copilot/extensibility/api/...

1. Copilot Chat API (beta, "use in production not supported") [V]
   - `POST /beta/copilot/conversations` (create), `POST /beta/copilot/conversations/{id}/chat` (sync), `.../chatOverStream` (SSE), body {message{text}, locationHint{timeZone} REQUIRED, additionalContext[], contextualResources{files[{uri}], webContext{isWebEnabled}}}.
   - Response: conversation with messages, adaptiveCards, attributions (citation/annotation), sensitivityLabel.
   - Delegated work/school only; no app-only; no personal accounts. ALL of these scopes are required: Sites.Read.All, Mail.Read, People.Read.All, OnlineMeetingTranscript.Read.All, Chat.Read, ChannelMessage.Read.All, ExternalItem.Read.All. "Higher privileged permissions: not supported" (so Sites.ReadWrite.All/Mail.ReadWrite do not substitute officially).
   - Held: ChannelMessage.Read.All only (Sites.ReadWrite.All, Mail.ReadWrite arguably imply Sites.Read.All/Mail.Read at runtime but docs say not supported). MISSING: People.Read.All (we have People.Read), OnlineMeetingTranscript.Read.All, Chat.Read, ExternalItem.Read.All. => NOT callable with our token set.
   - Licensing: Copilot add-on license required [S third-party, not on the fetched page; assume yes].
2. Retrieval API [V]
   - `POST /beta/copilot/retrieval` (SharePoint, OneDrive, Copilot connectors; KQL filter; <=25 results; 200 req/user/hour; query <=1500 chars).
   - Scopes: Files.Read.All + Sites.Read.All for SharePoint/OneDrive; ExternalItem.Read.All for connectors. We hold Files.ReadWrite.All and Sites.ReadWrite.All (superset; likely accepted, [TBV]); missing ExternalItem.Read.All (connectors only).
   - Licensing: included with Microsoft 365 Copilot add-on; without it pay-as-you-go preview for tenant-level sources (SharePoint, connectors) only, OneDrive excluded. This is a retrieval (RAG) API, it does not generate answers; kelma would need its own LLM.
3. Meeting AI Insights API (v1.0 and beta, under `/copilot/users/{id}/onlineMeetings/{id}/aiInsights`): see section 3.
4. Interaction export `GET /copilot/users/{id}/interactionHistory/getAllEnterpriseInteractions`: application permission AiEnterpriseInteraction.Read.All only; delegated not supported [V]. Not usable.
5. Change notifications for aiInteraction: app-level [S], not usable.

## 2.5 Feasibility (Copilot)
- Official API route: NOT feasible with the held scopes (Chat API needs 4 missing delegated scopes; beta; paid license for work grounding).
- Substrate "Sydney" WebSocket route: technically feasible if the substrate token can be minted from the same refresh token [TBV]; native Rust is fine (tokio-tungstenite, JSON frames with 0x1E separator, streaming render of markdown + adaptive-card citations). Undocumented and fragile; works for both free (web-grounded) and licensed users because entitlement is server-side. Recommended plan: spike by capturing Teams web's Copilot traffic; if token mint fails, fall back to "open Copilot in browser/webview" (copilot.cloud.microsoft or m365.cloud.microsoft/chat via the `teams.microsoft.com/v2/?app=...` deep link).
- Cheapest MVP: webview to `https://m365.cloud.microsoft/chat` (SSO via existing session cookies is not guaranteed in a non-browser client; needs cookie injection or interactive login).
- Do NOT scrape/rely on Copilot for correctness-critical flows.

---

# 3. Meeting Recaps ("Recap" app + per-meeting Recap tab)

## 3.1 What exists (corrected naming)
Two surfaces [V]:
a) Per-meeting "Recap" tab in the Teams calendar event and meeting chat (since Teams Premium era).
b) A dedicated first-party "Recap" app (Message Center MC1404322, roadmap 564614; preinstalled, not pinned by default, enabled by default; appears via Apps > Recap and in the left rail once pinned). GA worldwide July-Aug 2026; Windows, Mac, web; mobile "coming soon" [V]. It lists recaps from the past 30 days, with filters by date, participants, keywords and quick filters; shows only meetings that have a recording or transcript; can play video recaps / audio recaps and generate a combined podcast-style audio recap across multiple meetings.

## 3.2 Feature inventory (per-meeting recap) [V: support.microsoft.com recap page + learn intelligent-recap doc]
- Summary: AI-generated notes (titles, bullets, sub-points), AI recommended tasks/action items (owner), custom summaries and agenda (desktop). Needs transcription and meeting >= 5 min.
- Recording: playback of the recording stored in OneDrive/SharePoint.
- Transcript: searchable, speaker-labelled, timestamped; language support ~25 languages; multilingual recap (preview) translated per participant (en, es, ja, fr, de, pt, it, zh-Hans, ko).
- Chapters and topics: jump points derived from transcript keywords and PowerPoint Live content (needs recording + transcript).
- Speaker timeline: who spoke when (select a bar to seek).
- Personalized timeline markers (visible only to the viewer): name mentions, screen shares, own join/leave.
- Mentions: transcript utterances where the viewer was named (matching display name).
- Shared content: files attached/shared in chat; screen-share markers.
- Attendance info feeds join/leave markers.
- Audio recap (preview): podcast-style summary of up to 8 meetings, stored 60 days in OneDrive `Recordings/AudioRecaps`; requires Copilot license.
- Video recap: narrated highlights with clips (meeting >= 10 min, recorded + transcribed; up to 100 highlights stored inside the recording file); Copilot license.
- Recap without transcript saved (scheduled meetings): needs Copilot license.
- Share: Share to org (link), Share > Share via Outlook / email (organizers/co-organizers/presenters; external recipients allowed), copy link.
- Delete recap content (organizer/co-organizer; permanent).
- Calls: PSTN and 1:1/group VoIP calls have a Recap button too (AI notes, tasks, topics, mentions, chapters).
- Events (town hall/webinar): recap limited to organizers, co-organizers, presenters.
- Meeting Memory (TAP preview).
- Not supported: instant/ad-hoc meetings for the per-meeting tab, channel meetings for AI Insights API [V].

## 3.3 Licensing
- Intelligent recap (notes, tasks, chapters, speaker/mention timelines): Teams Premium OR Microsoft 365 Copilot license [V]. A TechCommunity thread reports some tenants where AI notes stay locked with Copilot-only [S].
- Audio recap, video recap, recap without transcript, Recap app's AI features: Microsoft 365 Copilot license [V]. Without a Copilot license the Recap app still lists recordings and transcripts under existing permissions [V].
- Transcription must be enabled by policy; recording policy needed for full experience [V].

## 3.4 Graph APIs and scopes
| Data | Endpoint | Delegated scope | Held? | Notes |
|---|---|---|---|---|
| Resolve meeting id | `GET /me/onlineMeetings?$filter=JoinWebUrl eq '...'` | OnlineMeetings.Read | NO | Alternative: encode thread/organizer into meeting id (base64 `1*{organizerOid}*0**{threadId}`), or take from chat/calendar event (`onlineMeeting.joinUrl` via Calendars.ReadWrite). |
| List transcripts | `GET /me/onlineMeetings/{id}/transcripts` ; `GET /users/{u}/onlineMeetings/getAllTranscripts(meetingOrganizerUserId='..')` | OnlineMeetingTranscript.Read.All | NO | v1.0 [V]. 403 `GraphAccessToTranscriptsDisabled` if tenant disabled. Meeting must not be expired. |
| Transcript content | `GET .../transcripts/{id}/content?$format=text/vtt` ; `.../metadataContent` | same | NO | |
| List recordings | `GET /me/onlineMeetings/{id}/recordings` | OnlineMeetingRecording.Read.All | NO | v1.0 [V]; delegated content download works only for the organizer [S]; scheduled (calendar-backed) meetings only. |
| AI insights | `GET /copilot/users/{userId}/onlineMeetings/{id}/aiInsights` and `/{aiInsightId}` (v1.0+beta) | OnlineMeetingAiInsight.Read.All | NO | [V] Needs Copilot license (usage-based/eval not supported). Returns meetingNotes[{title,text,subpoints}], actionItems[{title,text,ownerDisplayName}], viewpoint.mentionEvents[{eventDateTime,transcriptUtterance,speaker}], beta `recapUrl` deep link (`teams.microsoft.com/l/meetingrecap?...`). Available up to 4 h after meeting end; transcribed meetings only; no channel meetings. |
| Call records | `GET /communications/callRecords` | CallRecords.Read.All | NO (and admin-only) | Not a recap source (QoS/participants only). |
| Recording/transcript FILES | `GET /me/drive/root:/Recordings:/children` ; item `/content` | Files.ReadWrite.All | YES | Recordings (.mp4) and transcripts are stored as files in the organizer's OneDrive (non-channel) or channel SharePoint library. For recordings WE ORGANIZED we can list, stream and download natively; for meetings organized by others, access is via the shared link in the meeting chat message (`/shares/{encoded}/driveItem`, needs Files.Read.All, held). |
| Attendance | `GET /me/onlineMeetings/{id}/attendanceReports` | OnlineMeetingArtifact.Read.All | NO | |
| Per-meeting chat | existing chat APIs | held via Teams internal chat service | | Recap card posts to the meeting chat; the "Recap" tab payload is rendered from a Teams-internal service. |

Summary of scope gaps: OnlineMeetings.Read, OnlineMeetingTranscript.Read.All, OnlineMeetingRecording.Read.All, OnlineMeetingAiInsight.Read.All, OnlineMeetingArtifact.Read.All are all MISSING. Chat-only meeting-scoped RSC variants (`...Read.Chat`) are app-only, not delegated.

## 3.5 Feasibility (Recaps)
- File-based recap (recording playback + transcript): FEASIBLE via Files.ReadWrite.All. Recording `.mp4` through `/content` (supports HTTP range, play with GStreamer/libmpv in kelma), transcript `.vtt` / `.docx` stored beside recording (name `<meeting>-<timestamp>-Meeting Transcript.docx` / `.vtt`) [K/TBV]. Gives Recordings + Transcript + a DIY speaker timeline (parse VTT voice tags) + search. Missing without extra API: official AI notes, tasks, chapters, mention markers.
- AI notes/tasks/mentions: the official API is out of reach (scope + Copilot license). The Teams client reads recap content from an internal service (likely `teams.microsoft.com/api/...` / `substrate`); needs traffic capture [TBV]. If unavailable: either (a) deep link out via `recapUrl`/`teams.microsoft.com/l/meetingrecap?...` in browser, or (b) run our own LLM over the VTT for a "kelma notes" summary (clearly labelled not Microsoft AI).
- Recap app list view (past 30 days): can be rebuilt from calendar (Calendars.ReadWrite) + presence of recording/transcript files; filters by date/people/keywords are client-side.
- Audio/video recap generation is a server feature behind Copilot license; not reproducible; link out only.

---

# 4. Store / Apps

## 4.1 Feature inventory (Teams "Apps" left-rail item)
- Browse: categories (e.g. Featured, Popular, Top picks, Productivity, Education, ... ), Built for your org (admin-uploaded/LOB apps), Agents (Copilot agents), search box, filter by capability (Personal app, Bot, Tab, Message extension, Meeting extension/Meeting stage), app detail page (description, screenshots, permissions, privacy/terms, publisher cert level, version) [K/S].
- Install targets: "Add" (personal scope), "Add to a team" (channel tab/bot/connector), "Add to a chat", "Add to a meeting" (meeting side panel / stage / pre-meeting) [S].
- Manage your apps (bottom of Apps rail): apps you added, apps added on your behalf, uninstall, update available, "Built for your org" [S].
- App permissions: resource-specific consent (RSC) shown before install; org admin controls (allow/block, tenant-wide, per-user app permission policy, "Request approval" flow for blocked apps) [K].
- Pinning: right-click app in rail > Pin / Unpin; "More added apps" flyout (the `...` item in the rail); drag to reorder; admin app setup policy pins apps for everyone and can disable user pinning [V: setup-policy doc via search].
- Auto-install and org-pinned apps via setup policies [V]. Guest users get a reduced default set [S, 2020].
- Apps in external meetings/chats allowed since 2024 [S].
- Teams team owners can restrict who can add apps; some apps are owner-only [S].

## 4.2 Built-in / commonly pinned apps: what they are and how to host
(Hosting classes: A = pure Graph/REST native UI feasible; B = web app, host in webview via its tab `contentUrl`/deep link or open in browser; C = no public API, link out only.)

| App | What it does | Delivery | Native/hosting options for kelma | Held scope? |
|---|---|---|---|---|
| Viva Engage / "Communities" | Yammer-based communities, storylines, announcements, leadership corner. Communities integrated into Teams "Communities" section from Apr-May 2026 [S]. | Web app (tab content from engage.cloud.microsoft) | B/A: Viva Engage REST (legacy Yammer API, `https://www.yammer.com/api/v1/...`, token resource `https://api.yammer.com/user_impersonation`) is a separate audience; Graph v1.0 has no Engage coverage (beta `/employeeExperience/communities` exists [K]). Webview is the realistic path. | Graph: Group.Read.All (read community Microsoft 365 groups); Engage API token TBV |
| Planner / Tasks by Planner and To Do | Personal + team tasks; Planner plans (Premium/Project for web) | Web app; data in Graph | A: Graph `/me/planner/tasks`, `/planner/plans`, `/planner/tasks`, `/me/todo/lists`. Premium plans (Dataverse) are not in Graph; use webview. | Tasks.ReadWrite YES (To Do + Planner); Group.ReadWrite.All needed for some plan creation (MISSING; creating a plan via `/planner/plans` with `owner` group needs Group.ReadWrite.All) |
| Approvals | Create/track approval requests, templates, e-sign | Web app (Power Platform/Dataverse; Graph beta `/solutions/approval/approvalItems`) | A/B: Graph beta `approvalItem` needs `ApprovalSolution.ReadWrite` (MISSING). Fall back to webview of the app tab. | NO |
| Shifts | Schedule, swaps, open shifts, time clock | Web app; Graph `/teams/{id}/schedule/...` | A possible but scope gap | `Schedule.ReadWrite.All` MISSING |
| Lists | Microsoft Lists (SharePoint list items) | Web app on SharePoint | A: Graph `/sites/{id}/lists/{id}/items`, columns, views (read-only), `listItem` | Sites.ReadWrite.All YES |
| Bookings | Appointment pages, staff, services | Web app; Graph `/solutions/bookingBusinesses` | A possible | `Bookings.Read.All`/`BookingsAppointment.ReadWrite.All` MISSING |
| Whiteboard | Freeform canvas (stored as `.whiteboard` file in OneDrive) | Web app (whiteboard.office.com) | B only; no public Graph API for content. Files list via OneDrive. | Files YES (file only) |
| Wiki | Channel wiki tab (legacy; stored in channel library) | Tab (web) | B/C; being deprecated in favor of Loop/OneNote; treat as optional tab type | TeamsTab.* YES |
| Praise | Send praise badges (Viva) | Bot + web | C; not verified current availability (I could not confirm retirement; search returned nothing) [TBV]. Skip. | |
| Updates | Status update/check-in app (legacy) | Web app | C; not verified; skip. [TBV] | |
| Workflows (Power Automate) | Templates, flows triggered from messages/channels, approvals | Web app; flow.microsoft.com / `api.flow.microsoft.com` | B only (separate audience `https://service.flow.microsoft.com/`) | NO |
| Viva Insights | Wellbeing, focus plans, quiet time | Web app (insights.cloud.microsoft) | B | NO |
| Viva Learning | Learning content, assignments | Web app | B | NO |
| Viva Connections | Intranet dashboard/feed | Web app (SharePoint-based) | B; or site pages via Sites.ReadWrite.All | Sites YES (partial) |
| OneNote | Notebooks | Web app; Graph `/me/onenote` | A/B: Graph notebooks/sections/pages (HTML pages); editor needs OneNote web | Notes.ReadWrite.All YES |
| Forms | Surveys/quizzes | Web (forms.office.com); no public Graph | B | NO |
| Polls | Forms-backed poll bot/message extension in chat/channel | Bot + Forms | C; poll messages are normal chat messages with adaptive cards. | |
| Loop | Loop components/workspaces | Web; Graph via SharePoint Embedded containers (FileStorageContainer.Selected helps only for own container types) | B | partial |
| Calendar/Calls/Chat/Teams/Activity | core native UI | | already in kelma (calendar, chat); Calls is out of scope. | |

How a non-browser client hosts a Teams app (general):
1. Teams apps are manifest-declared: `staticTabs` (personal), `configurableTabs`/`tabs` (team/chat/meeting; `contentUrl`, `websiteUrl`, `entityId`), bots (messages through Bot Framework), message extensions, meeting extensions. Read definitions via `GET /appCatalogs/teamsApps/{id}?$expand=appDefinitions($expand=bot)` or `GET /me/teamwork/installedApps?$expand=teamsAppDefinition` [K].
2. Tabs load `contentUrl` in an iframe with the Teams JS SDK (`microsoftTeams.app.initialize()`, `authentication.getAuthToken()` SSO). A non-Teams host can embed the URL in a webview (wry/WebKitGTK, or CEF) BUT: the app requires the `window.parent` postMessage bridge (TeamsJS v2 / `https://teams.microsoft.com` origin checks, `validDomains`), `getAuthToken` needs the Entra token for the app's own client id (via SSO exchange with our refresh token, requires pre-authorization of Teams client on that app, which is true for most apps), and many apps check `window.name`/`ancestorOrigins`. Cost: implementing a TeamsJS host shim (JSON message protocol: `{id, func, args}`) = medium-large. Realistic scope: support popular first-party tabs via "open in browser" deep link (`https://teams.microsoft.com/l/entity/{appId}/{entityId}`) and a limited webview for apps that need no SDK handshake.
3. Bots: message through the chat service already used for chat; bots appear as 28:/ `28:{appId}` participants; messages work like chat (adaptive cards need a card renderer; kelma probably already needs it).
4. Message extensions and meeting apps: require invoke APIs to the Teams service, out of scope for v1.

## 4.3 Graph / API mapping for the Store
| Feature | Endpoint | Scope | Held? |
|---|---|---|---|
| Browse/search store apps | `GET /appCatalogs/teamsApps?$filter=distributionMethod eq 'store'` (+`$expand=appDefinitions`); `$filter=displayName eq`; org apps `distributionMethod eq 'organization'` | AppCatalog.Read.All (delegated, no admin consent needed per Merill) [S]; official table lists AppCatalog.Submit as least-privileged and AppCatalog.Read.All as higher [S] | YES (AppCatalog.Read.All). A Q&A reports 403 for some tenants despite the scope [S]; the Teams client uses its internal store service (`teams.microsoft.com/api/mt/...appstore`/ "apps" Substrate) which is richer (categories, ratings, screenshots) [TBV]. |
| List my installed apps | `GET /me/teamwork/installedApps?$expand=teamsApp,teamsAppDefinition` | TeamsAppInstallation.ReadForUser (least) | NO (we hold ReadForTeam only). Use internal API or the chat-service app list. |
| Install to personal scope | `POST /users/{id}/teamwork/installedApps` body `{ "teamsApp@odata.bind": ".../appCatalogs/teamsApps/{id}", consentedPermissionSet? }` [V] | TeamsAppInstallation.ReadWriteSelfForUser (least) | NO |
| Install to team | `POST /teams/{id}/installedApps` | TeamsAppInstallation.ReadWriteSelfForTeam / ReadWriteForTeam | NO (only ReadForTeam) |
| Install to chat / meeting | `POST /chats/{id}/installedApps` (installing to a meeting chat installs to the meeting [S]) | TeamsAppInstallation.ReadWriteSelfForChat/ReadWriteForChat | NO |
| List apps in a team | `GET /teams/{id}/installedApps?$expand=teamsApp` | TeamsAppInstallation.ReadForTeam | YES |
| List/uninstall/upgrade | `DELETE .../installedApps/{id}`, `POST .../upgrade` | same ReadWrite* | NO |
| RSC permission display | `$select=consentedPermissionSet` on installedApps [S] | | |
| Pin to app bar | NOT in Graph. User-level pinned apps live in the Teams-internal user settings (`/api/mt/part/.../userSettings` or similar) [TBV]; admin pins via setup policy (admin-only, Teams PowerShell/TAC). | | |
| Tabs | `GET/POST /teams/{t}/channels/{c}/tabs`, `/chats/{c}/tabs` | TeamsTab.ReadWrite.All / TeamsTab.Create | YES |
| Org app approval requests / app permission policy | admin center only | | n/a |

## 4.4 Feasibility (Store/Apps)
- Browse/search store (read-only) : MEDIUM-HIGH; AppCatalog.Read.All covers catalog listing.
- Install/uninstall: LOW via Graph (no ReadWrite*ForUser/Team/Chat). Fall back to the Teams-internal install endpoints (same ones teams.microsoft.com calls, TBV by capture) or "Open in Teams web/browser to install".
- Pin/unpin/reorder app bar: only via internal user-settings API (TBV); otherwise keep kelma's own local pin list (simplest; app bar is client-side state).
- Hosting third-party/first-party apps: Phase approach: (1) "Open in browser" deep links for everything; (2) Graph-native screens where the scope exists (Planner/To Do, Lists, OneNote, Files); (3) webview + TeamsJS shim for tabs that only need `app.initialize`, `getContext`, `getAuthToken`, `openLink`. Do not promise parity.

---

# 5. Teams and channels management

## 5.1 Feature inventory (Teams client)
Teams list / discovery:
- Your teams list, reorder, hide/show (hidden teams section), pin channels, favourite channels, "Manage teams" page.
- Join or create a team: Create team (From scratch -> Private / Public / Org-wide; From a group or team (existing M365 group, existing team); From a template (Manage a project, Onboard employees, Organize a help desk, Event, Crisis response, Manage a store, ... plus custom/org templates)), "Join a team with a code" (7-character code; guests cannot) [V], "Browse/discover public teams" [S].
- Team "..." menu: Add member, Add channel, Join/Leave team, Manage team, Edit team, Get link to team, Manage tags, Delete the team, Archive.
- Manage team tabs: Members (roles Owner/Member/Guest, add/remove, change role, pending invites, guests), Pending requests, Channels (Add/edit/delete, Deleted channels with restore, show/hide), Settings (Team picture, Member permissions, Guest permissions, @mentions, Team code, Fun stuff (Giphy/stickers/memes), Tags, Email integration, Team analytics, Apps), Analytics, Apps.
- Member permissions (all in `team.memberSettings`/`guestSettings`/`messagingSettings`/`funSettings`/`discoverySettings`): create/update channels, delete/restore channels, add/remove apps, create/update/remove tabs, create/update/remove connectors, edit own/delete own messages, owner delete messages, @team/@channel mentions, giphy+rating, stickers, custom memes, team discoverable in search.
- Team code: generate/reset/remove [V].
- Archive team (read-only, optional SharePoint read-only), Unarchive; Delete team (30-day group soft delete); Clone team (parts: apps, tabs, settings, channels, members).
Channels:
- Create channel: Standard (everyone), Private (selected members, own SharePoint site), Shared (cross-team and cross-tenant via B2B direct connect); optional "Favorite this channel automatically for all members"; channel description; layout (Posts vs Chat/"threaded chat" layout).
- Edit channel (name/description), Delete (soft delete 21 days in Deleted channels, restore), Archive? (no channel archive; hide/show only), Channel moderation (Off/On; who can start posts: everyone / only owners & moderators; reply permissions; bot posts; moderators list), Get link to channel, Get email address (generate/remove, restrict senders: anyone / team members / specific domains), Manage channel (members & owners, shared-with teams), Pin channel (user-level), Notifications settings, Open in SharePoint (files folder), Channel analytics? (team-level only).
- Tags (@mentions groups): create/edit/delete, add members, who can create tags (owners / owners + members), tag mentions in channels.
- Team templates (admin-published) in create flow; org-wide team (up to 10k members; one per tenant).
- Communities (Viva Engage) in Teams: "Communities" nav lists Engage communities; each community has its own feed, events, files. Backed by Viva Engage, not Teams channels (UI delegated to Engage web) [S].
- Team analytics: Insights tab (Activity: message/meeting counts; Analytics only in the Teams admin / Viva Insights). Teams Admin: Reports (admin only).

## 5.2 API notes (endpoints and scopes)
All v1.0 unless noted. "Held?" refers to delegated scopes from the brief.

| Operation | Endpoint | Least delegated scope | Held? | Notes |
|---|---|---|---|---|
| List my teams | `GET /me/joinedTeams` | Team.ReadBasic.All | YES | Also `/me/teamwork/associatedTeams` (TeamMember.Read.All?) [K] for shared-channel hosts. |
| Team details | `GET /teams/{id}` | Team.ReadBasic.All (basic), Team.ReadBasic.All+... full needs TeamSettings.Read.All | partial | memberSettings etc need TeamSettings.Read.All (MISSING). |
| List channels | `GET /teams/{id}/channels` (+`/allChannels` for incoming shared channels) | Channel.ReadBasic.All | YES | Includes `email`, `membershipType`, `isFavoriteByDefault`. |
| Create team (scratch/template) | `POST /teams` (+`template@odata.bind: teamsTemplates('standard'\|'educationClass'...)`), async 202 + operations URL [V] | Team.Create | NO | Higher: Group.ReadWrite.All/Directory.ReadWrite.All (not held). |
| Create team from group | `POST /teams` with `group@odata.bind` (inherits name/visibility/members; group must have owner) [V]; older `PUT /groups/{id}/team` | Team.Create | NO | |
| Clone team | `POST /teams/{id}/clone` | Team.Create | NO | |
| Custom/org template list | `GET /teamwork/teamTemplates` (v1.0) | TeamTemplates.Read.All? [K] | NO | |
| Join team with code | NO Graph API. Teams-internal ("join by code") call [TBV]. For public teams: `POST /me/teamwork/...`? none. Group join: `POST /groups/{id}/members/$ref` needs Group.ReadWrite.All | | NO | Codes: seven characters, guests cannot, generated by owners [V]. |
| Discover public teams | `GET /groups?$filter=resourceProvisioningOptions/Any(x:x eq 'Team') and visibility eq 'Public'` | Group.Read.All | YES | Browse only; join needs internal join endpoint or Group.ReadWrite.All / owner approval. |
| Members: list/add/remove/role | `GET/POST /teams/{id}/members`, `PATCH /teams/{id}/members/{membershipId}` (roles owner/member/guest), `DELETE`; bulk `POST .../members/add` | TeamMember.ReadWrite.All | YES | Adding guests requires the user to exist (invite via `POST /invitations`, needs User.Invite.All, MISSING). |
| Channel members | `GET/POST/PATCH/DELETE /teams/{t}/channels/{c}/members` | ChannelMember.ReadWrite.All | YES | Private/shared channels. |
| Create channel (standard/private/shared) | `POST /teams/{id}/channels` body `{displayName, description, membershipType: standard\|private\|shared, members:[{...owner}], isFavoriteByDefault}` | Channel.Create | YES | Creator becomes owner; private/shared requires an owner in `members`. |
| Edit channel | `PATCH /teams/{t}/channels/{c}` `{displayName, description, layoutType}` [V] ; cannot change membershipType [V] | ChannelSettings.ReadWrite.All | NO | Hybrid: Teams-internal PATCH may work. |
| Delete channel | `DELETE /teams/{t}/channels/{c}` [V] | Channel.Delete.All | NO | |
| Restore deleted channel | `POST /teams/{t}/channels/deletedTeams`? actually `POST /teamwork/deletedTeams/{id}/channels/{id}/undelete` | | NO | |
| Archive / unarchive team | `POST /teams/{id}/archive` (+`shouldSetSpoSiteReadOnlyForMembers`) / `/unarchive` | TeamSettings.ReadWrite.All | NO | Async 202. |
| Delete team | `DELETE /groups/{id}` | Group.ReadWrite.All | NO | |
| Team settings (member/guest/messaging/fun) | `PATCH /teams/{id}` | TeamSettings.ReadWrite.All | NO | |
| Channel moderation | `PATCH /teams/{t}/channels/{c}` `{moderationSettings:{userNewMessageRestriction, replyRestriction, allowNewMessageFromBots, allowNewMessageFromConnectors}}` (beta) | ChannelSettings.ReadWrite.All | NO | Moderators = channel owners. |
| Channel email address | read: `channel.email` on GET (Channel.ReadBasic.All ok); generate/remove/restrict senders: Teams-internal only (no Graph write) [K] | | read YES | |
| Channel files folder (SharePoint) | `GET /teams/{t}/channels/{c}/filesFolder` | Files.Read.All | YES | Triggers library provisioning; used by Quick access. |
| Tags | `GET/POST/PATCH/DELETE /teams/{id}/tags`, `/tags/{id}/members` (200 members/tag) [S] | TeamworkTag.ReadWrite (read: TeamworkTag.Read) | NO | Tag mentions in messages still work for sending if we have tag ids via internal chat service. |
| Apps in team | see 4.3 | | | |
| Tabs | `/teams/{t}/channels/{c}/tabs` | TeamsTab.ReadWrite.All | YES | |
| Team picture | `PUT /teams/{id}/photo/$value` | TeamSettings.ReadWrite.All? (Group.ReadWrite.All) | NO | |
| Hide/show teams, pin channels, favorites, order | NO Graph. User-level state in Teams-internal roster/"user preferences" service [TBV] | | | Keep in kelma local config otherwise. |
| Team analytics | admin reports only (`/reports/getTeamsUserActivity...` needs Reports.Read.All) | | NO | Not a user-facing Graph feature; Teams "Analytics" tab uses internal service. |
| Shared channel across tenant | `GET /teams/{t}/channels/{c}/sharedWithTeams`, `POST .../sharedWithTeams` | Channel.ReadBasic.All / ChannelMember.ReadWrite.All | read YES, share needs ChannelMember.ReadWrite.All | Cross-tenant requires B2B direct connect policy. |
| Communities (Viva Engage) | Engage REST audience `api.yammer.com` ; Graph beta `/employeeExperience/communities` | Community.Read.All? [K/TBV] | NO | |

## 5.3 Feasibility (Teams & channel mgmt)
- Read-only management (list teams/channels/members/tags via `members`, filesFolder): HIGH.
- Membership management (add/remove, owner/member role, channel members): HIGH via TeamMember.ReadWrite.All + ChannelMember.ReadWrite.All (note: adding guests/external needs the invite flow, missing User.Invite.All).
- Create channel (standard/private/shared): HIGH with Channel.Create.
- Create team: LOW via Graph (Team.Create missing). Teams-internal team-creation endpoint (`teams.microsoft.com/api/mt/.../teams` or the `/api/csa/.../teams`) is what the client uses; capture [TBV]. Fallback: "Create in browser".
- Edit / delete / archive channel or team, team settings, moderation, tags: LOW via Graph (ChannelSettings.ReadWrite.All, Channel.Delete.All, TeamSettings.ReadWrite.All, TeamworkTag.ReadWrite all missing). Try the Teams-internal (chat service/`mt`) endpoints because the web client itself performs those with the same tokens; each one is a TBV capture item.
- Join with code, hide/show, pin: internal only.
- Communities: webview/open in browser.
- Priority order: P1 team/channel list + members + create channel; P2 internal-API captured: edit/delete/restore channel, moderation, tags, join with code, create team; P3 team settings page, archive, analytics, templates.

---

# 6. Settings areas not covered by chat

Teams desktop (new Teams) Settings sections [S: multiple university QRGs; exact names vary by build]: Accounts and orgs, General, Notifications and activity, Privacy, Appearance and accessibility (earlier: Accessibility), Files and links, Devices, Captions and transcripts, Calls, Apps (App permissions), Meetings (some builds fold into General), Storage, About Teams.

## 6.1 Accounts and orgs
- Multi-account: add work, school and personal accounts to one desktop app; switch from profile picture > account; pin up to 3 accounts to the sidebar [V: support.microsoft.com manage accounts].
- Guest organizations: appear automatically when an account has guest access in another tenant; per-org notification toggles; org switcher lists home tenant and guest tenants [V/S].
- Sign out / remove account; manage "Sign out everywhere" no.
- Implementation: tenant switch = new token set per tenant: mint tokens for the guest tenant with the same refresh token using `https://login.microsoftonline.com/{guestTenantId}/oauth2/v2.0/token` (refresh tokens are redeemable cross-tenant when the home account is a B2B guest; some tenants require CA/MFA again, expect interactive fallback). List orgs: `GET /me/tenants`? not in Graph; use ARM `GET https://management.azure.com/tenants?api-version=2022-12-01` (needs ARM scope, probably not available) or Teams internal `tenants` list from the chat service's `/api/mt/part/.../users/me/tenants` [TBV]. Per-tenant Graph audience is the same host, token differs per tenant. kelma should model "account = (homeAccount, tenant)" with isolated stores.

## 6.2 General
- Language (app language, spelling language), Time zone (Teams uses Outlook mailbox time zone; setting: Graph `PATCH /me/mailboxSettings {timeZone, language{locale}}` with MailboxSettings.ReadWrite, MISSING; Mail.ReadWrite does not cover mailboxSettings [K]) , Date/time format (mailbox).
- App: Auto-start Teams, Open in background, On close keep running, Register as chat app for Office, Disable GPU hardware acceleration, Enable new meeting experience (legacy), Show unread-count badge, Theme (Default/Dark/High contrast), Density (Compact/Default), Read receipts toggle (tenant policy), Turn on Do-not-disturb-style status behaviors, Reset to defaults [S].
- In kelma: all desktop-local settings (autostart via XDG autostart `.desktop`, tray keep-running, theme, density) are native and need no API. Language/time zone: local override + read mailbox via `GET /me/mailboxSettings` (MailboxSettings.Read, MISSING; Mail.ReadWrite may still allow `/me/mailboxSettings` GET [TBV]). Fallback: local OS timezone/locale.

## 6.3 Accessibility / Appearance
- Captions language and live captions size/position, TTY mode (Calls settings), Sign-language view, Screen-reader/keyboard shortcuts, Reduce motion, High contrast theme, Text size zoom, Show keyboard shortcut hints [S].
- In kelma: purely client-side (AT-SPI via accesskit/GTK, high-contrast follows OS). No API.

## 6.4 Devices
- Audio (speaker, microphone, noise suppression, echo cancellation), Camera (device, preview, background effects), Secondary ringer, Meeting-room device pairing (Teams Rooms), Test call [S].
- In kelma: only relevant if we implement calls/meetings (out of scope; PipeWire/WebRTC). Chat-only kelma should hide it or provide only "notification sound output device" via PipeWire/PulseAudio.

## 6.5 Privacy
- Priority access: contacts who can reach you during Do Not Disturb (priority list up to 25) [S].
- Blocked contacts: block/unblock callers (call blocking; "Blocked contacts" list) [S].
- Who can contact you / find you in search / @mention you in group chats [S].
- Read receipts, surveys (Teams surveys), Diagnostic data, Connected experiences, "Manage who can see your presence" is admin-controlled.
- API: priority access and blocked contacts are stored in the Teams-internal settings (`/api/mt/part/...` privacy/preferences; the chat service already holds `blocklist`/`blockedUsers` and `priority` lists) [TBV]. Presence/DND via Teams presence API (not in held Graph scopes: Presence.ReadWrite missing, but internal presence service via the chat token works for chat already). Read receipts is tenant/user preference via the chat service `consumptionhorizon` settings [K].

## 6.6 App permissions (Settings > Apps / "App permissions")
- Per-app resource access: location, notifications, media (camera/mic), files, external links; user toggles for each app and per-meeting-app; "Allow apps to use..." [S]. Not a Graph surface; implemented by the Teams client per-app host permissions.
- In kelma: store local per-app grants (notifications, location, files) in config; relevant only once webview-hosted apps exist. Org-level app permission policies are enforced server-side via Teams admin.

## 6.7 Files and links settings
- Choose how to open files and links: in Teams, in browser, in desktop app; separate for Office files vs other; "Open links in default browser", "Open Office docs in desktop app" [S].
- Cloud storage accounts (Dropbox, Google Drive, Box, ShareFile) in the Files app [K, deprecated for OneDrive app].
- In kelma: local preference mapping to `xdg-open`, in-app viewer (preview), or Office web URL; no API.

## 6.8 Other (for completeness)
- Notifications and activity (covered by chat team presumably), Captions and transcripts (live caption language; transcript language: `PATCH /me/onlineMeetings` n/a), Calls (voicemail, call forwarding; Teams Phone), Storage/Clear cache (local), Manage keyboard shortcuts, Report a problem, About.
- Time zone/working hours: `GET/PATCH /me/mailboxSettings` (workingHours, timeZone) uses MailboxSettings.ReadWrite (MISSING). Fallback local.

## 6.9 Feasibility (Settings)
- General/Accessibility/Files-and-links/App-permissions: HIGH (local config, no API).
- Accounts and orgs: MEDIUM; per-tenant refresh redemption is workable; org list requires internal API [TBV]; expect CA/MFA step-up for guest tenants.
- Privacy (priority, blocked): MEDIUM; depends on capturing the internal endpoints (the chat service already used by kelma probably exposes them).
- Devices: only if calls implemented.
- Time zone/language sync with Outlook: LOW via Graph (scope missing); use local OS fallback.

---

# 7. Cross-cutting recommendations

1. Build a "capture list" now: with the claude-in-chrome network tools against teams.microsoft.com (new Teams web), record the internal endpoints for: Copilot websocket (substrate), channel edit/delete/moderation/archive/tags, create team, join by code, favorites/hide/pin, app install, app pin, recycle bin, shared/recent, recap content, privacy lists, tenant list. Everything marked [TBV] above depends on this.
2. Token audiences to probe from the existing refresh token (each is one `POST /oauth2/v2.0/token` call): `https://substrate.office.com/.default`, `https://{tenant}-my.sharepoint.com/.default`, `https://api.spaces.skype.com/.default` (already used), `https://teams.microsoft.com/.default`, `https://api.yammer.com/.default`, `https://service.flow.microsoft.com/.default`, `https://management.azure.com/.default`. Record which succeed; that decides most "TBV" items.
3. Deprecations already in effect (Nov 2026): `/me/drive/recent`, `/me/drive/sharedWithMe`, `/me/insights/{used,shared,trending}`; do not build the Home/Shared views on them.
4. Missing Graph scopes that gate multiple features (cannot be added): TeamSettings.ReadWrite.All, ChannelSettings.ReadWrite.All, Channel.Delete.All, Team.Create, TeamworkTag.ReadWrite, TeamsAppInstallation.ReadWrite*, OnlineMeetings.Read, OnlineMeetingTranscript/Recording/AiInsight.Read.All, MailboxSettings.ReadWrite, Presence.*, Group.ReadWrite.All, ExternalItem.Read.All, People.Read.All, Chat.Read, User.Invite.All, Schedule.ReadWrite.All, Bookings.*, ApprovalSolution.ReadWrite.
5. Licensing reality check: Copilot-backed features (AI recap, Copilot app work grounding, insights API) depend on the user's license; design every such screen with a graceful "not licensed" state (the free Copilot Chat tier still works for web-grounded chat).

# Sources
- Files overview: https://learn.microsoft.com/en-us/graph/api/resources/onedrive
- sharedWithMe deprecated: https://learn.microsoft.com/en-us/graph/api/drive-sharedwithme
- recent deprecated: https://learn.microsoft.com/en-us/graph/api/drive-recent
- insights/shared deprecated: https://learn.microsoft.com/en-us/graph/api/insights-list-shared
- Versions: https://learn.microsoft.com/en-us/graph/api/driveitem-list-versions
- createLink: https://learn.microsoft.com/en-us/graph/api/driveitem-createlink
- Teams Files list: https://support.microsoft.com/en-us/office/4c1b6c25-3794-49f9-93aa-bf6fdc211299
- Replacement thread (community): https://techcommunity.microsoft.com/discussions/skills-hub-discussions/what-is-the-microsoft-graph-api-replacement-for-deprecated-recent-sharedwithme-a/4551294
- Copilot Chat API: https://learn.microsoft.com/microsoft-365/copilot/extensibility/api/ai-services/chat/copilotconversation-chat
- Retrieval API: https://learn.microsoft.com/microsoft-365/copilot/extensibility/api/ai-services/retrieval/overview
- Interaction export: https://learn.microsoft.com/en-us/graph/api/aiinteractionhistory-getallenterpriseinteractions?view=graph-rest-beta
- Copilot in Teams pinning (MC922627): https://mc.merill.net/message/MC922627
- Copilot chat list removal (MC1477999): https://mc.merill.net/message/MC1477999
- Copilot Chat overview/licensing: https://learn.microsoft.com/en-us/copilot/overview ; https://support.microsoft.com/en-us/microsoft-365-copilot/get-started-with-microsoft-365-copilot-chat
- Zenity substrate/Sydney research: https://labs.zenity.io/p/access-copilot-m365-terminal
- PyRIT websocket target (reference): https://azure.github.io/PyRIT/_autosummary/pyrit.prompt_target.WebSocketCopilotTarget.html
- Intelligent recap (admin): https://learn.microsoft.com/microsoftteams/intelligent-recap-calls-meetings
- Recap app (MC1404322): https://mc.merill.net/message/MC1404322
- Recap user guide: https://support.microsoft.com/en-us/teams/meetings/recap-in-microsoft-teams
- Meeting AI insights: https://learn.microsoft.com/microsoftteams/platform/graph-api/meeting-transcripts/meeting-insights ; https://learn.microsoft.com/en-us/microsoft-365/copilot/extensibility/api/ai-services/meeting-insights/onlinemeeting-list-aiinsights
- Transcripts: https://learn.microsoft.com/en-us/graph/api/onlinemeeting-list-transcripts
- Recordings: https://learn.microsoft.com/en-us/graph/api/onlinemeeting-list-recordings
- Install app for user: https://learn.microsoft.com/en-us/graph/api/userteamwork-post-installedapps
- App setup policies: https://learn.microsoft.com/microsoftteams/teams-app-setup-policies
- Create team: https://learn.microsoft.com/en-us/graph/api/team-post
- Delete channel: https://learn.microsoft.com/en-us/graph/api/channel-delete
- Patch channel: https://learn.microsoft.com/en-us/graph/api/channel-patch
- Team tags permissions: https://learn.microsoft.com/en-us/graph/api/resources/teamworktag
- Join with a code: https://support.microsoft.com/en-us/office/join-a-team-with-a-code-2459d918-61df-4b36-b2fd-f5d96beb6394
- Accounts and orgs: https://support.microsoft.com/office/manage-accounts-and-organizations-in-microsoft-teams-7b221128-6643-465c-a317-679e48cd2ce9
- Viva Engage/Communities in Teams (secondary): https://community.advania.co.uk/discussion/658/what-s-new-in-microsoft-365-march-2026

Items I did NOT verify (marked [K]/[TBV] in the text): exact Teams settings menu names for the current build, Praise/Updates/Wiki lifecycle status, Approvals/Shifts/Bookings/Forms scope names, team/channel Graph endpoints beyond create/patch/delete pages, `/me/drive/following` parity with web Favorites, SharePoint REST recycle bin audience, whether substrate/sharepoint audiences can be minted by client id 1fec8e78.
