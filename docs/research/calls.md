# kelma: Microsoft Teams Calls app and calling features. Inventory, protocol notes, feasibility

Research date: 2026-11-18. Method: Microsoft Learn / support.microsoft.com / GitHub / community writeups via web search and fetch.
Evidence tags used below:
- [V] verified in a cited source
- [C] community or third-party source (blog, forum, forensic writeup)
- [U] unverified: from general knowledge or inference; must be confirmed by capturing real traffic (HAR / CDP) from teams.microsoft.com before relying on it

Headline: nobody has published a reverse-engineered Teams calling protocol (signaling or media). The only fully supported programmatic route to a Teams-user call endpoint is the Azure Communication Services (ACS) "Teams user" (CTE) Calling SDK, which is JavaScript-only on Linux and needs a tenant-admin-consented Entra app plus a billed ACS resource. The data-plane parts of the Calls app (call history, voicemail list, settings) are reachable with APIs kelma already has or nearly has.

---

## 1. Feature inventory

### 1.1 Calls app UI (Teams desktop/web "Calls" tab)

Layout: left rail with Speed dial, Contacts, History (with voicemail as a filter), plus a dial pad and a quick-settings dropdown at the lower left. Sources: https://support.microsoft.com/en-us/office/get-to-know-the-calling-experience-in-teams-5c71cfd7-0553-4816-aa35-1fdae1ba2365 [V]

- Dial pad
  - Type a number or a name into the dial pad text box, press Call [V]
  - Multiple names or numbers entered together start a group call [V]
  - Calling by phone number needs a calling plan / Teams Phone (Enterprise Voice). Dial pad is only shown if the user is Enterprise Voice enabled; the delegates doc uses "if a Dial Pad shows in the Calls app" as the test for correct EV config [V] https://learn.microsoft.com/en-us/microsoftteams/user-call-settings
  - DTMF keypad is available in-call (see 1.2)
  - E.164 and dial-string handling, number normalization via tenant dial plans [U]
- Contacts
  - Contact groups ("contact groups you manage") organized by team/role; More actions > Add a contact [V]
  - "View contacts" opens the People app [C]
  - Personal contacts and org directory; Outlook contacts sync [U]
- Speed dial
  - Pinned people shown at the right/top of Calls; add from history More actions > Add to speed dial, or via search [V/C]
  - Speed dial shows presence and one-click call [U]
- Call history
  - Filters at top: all, missed, voicemail (past calls, missed calls, voicemail) [V]
  - Per-entry More actions: Call back, Chat, Add to speed dial, Add to contacts [V]
  - Entry types: outgoing, incoming (answered), missed, declined, voicemail, call-queue and auto-attendant calls, group calls (multiParty) [C] (call-log JSON fields in 2.3)
  - Recap button on recorded or transcribed 1:1 and group VoIP/PSTN calls (transcript, recording, notes, shared files); transcript-only Recap for PSTN; several recaps per call listed like chat threads [V] https://support.microsoft.com/en-us/office/tips-for-teams-calls-d3fb0e97-4afa-4919-86e8-021102ed5c2b
  - Delegates also get "shared call history" (desktop and web only) [V]
  - Admin-side retention is 30 days in Teams admin center call logs; client history depth is separate [C]
- Voicemail
  - Voicemail list with summary and transcript, audio playback, adjustable playback speed, Call back, delete [V/C] (https://support.microsoft.com/en-us/office/get-to-know-the-calling-experience-in-teams-5c71cfd7-0553-4816-aa35-1fdae1ba2365, https://bowdoin.teamdynamix.com/TDClient/1814/Portal/KB/Article/137899/Record-or-Change-Your-Voicemail-Greeting-in-Microsoft-Teams)
  - Transcription only if enabled by the tenant online voicemail policy; may be missing if audio unclear [V/C] https://learn.microsoft.com/en-us/microsoftteams/change-the-default-language-for-greetings-and-emails
  - Greetings: system default, recorded custom greeting, text-to-speech custom greeting (TTS plays only if no recorded greeting), out-of-office greeting with schedule and custom text, greeting language selection [V]
  - "Call answer rules" after voicemail picks up: caller records a message; records or is transferred; call ends without greeting; greeting then end; transfer to someone (needs destination) [V] https://support.microsoft.com/en-us/office/manage-your-call-settings-in-teams-456cb611-3477-496f-b31a-6ab752a7595f
  - Auto-attendant / call-queue shared voicemail history (Queues app, not Calls) [C]
  - RMS-protected voicemails are not viewable in Teams, only Outlook [C]
- Quick settings menu (lower left of Calls)
  - Switch connected audio device; call forwarding quick menu: Don't forward / Forward to [name or number] (up to 3 recent targets) / Forward to voicemail / call group / delegates (the last two only if configured) / More settings [V] https://support.microsoft.com/en-us/office/call-forwarding-call-groups-and-simultaneous-ring-in-teams-a88da9e8-1343-4d3c-9bda-4b9615e4183e
  - Call group notification toggle (off by default), "Edit notifications" [V]
- Parked calls: separate "Parked calls" entry in Calls where a parked call is picked up by code [C] https://kb.uwm.edu/page.php?id=73785
- Call queues (agent): per-queue notification toggles in Settings > Calls [V]
- Other related Calls surfaces: SMS send/receive on PSTN numbers; call context during a call; E2E encrypted 1:1 calls; Teams phones, displays, panels, Rooms, CarPlay, Surface Hub [V, listed as topic titles only]

### 1.2 In-call features: 1:1 and group calls (audio/video), started from chat or Calls

Starting
- Phone or video icon in a chat header (1:1 or group chat); dial pad; call back from history; speed dial; deep link https://teams.microsoft.com/l/call/0/0?users=... (documented as a "deep link to a workflow") [V for existence] https://learn.microsoft.com/en-us/microsoftteams/platform/concepts/build-and-test/deep-links
- Group call from group chat; add people during the call; 1:1 call promoted to group call [V] (ACS table below confirms promote and add)
- Test call (Windows/Mac only) and echo test 8:echo123 in ACS [V]

Incoming call
- Toast / incoming-call window; large or small incoming window setting (large always used with RTT or captions) [V]
- Actions: Accept (audio), Accept with video, Decline; also message-reply / "Chat" and forward on some builds [U]
- Rings all registered endpoints (desktop, mobile, web, desk phone, SIP device) incl. simultaneous ring targets and call-group members [V in concept]
- Call waiting beep / "Play sounds while I'm busy"; Do Not Disturb honored; busy-on-busy [V/C]

In-call controls
- Mute / unmute; camera on/off; switch camera [V]
- Share screen / window / browser tab; system audio share (web only in ACS); content-only mode [V]
- Hold / resume (local hold); music on hold (tenant setting) [V]
- Transfer: blind ("Transfer now") and consult-first ("Consult then transfer"); transfer to user, number, voicemail, auto attendant, call queue [V ACS table, C for UI labels]
- Add people / add phone number; remove participant; merge ongoing calls (Merge calls); make multiple calls at once [V/C]
- Keypad / DTMF [V]
- Call park (More actions > Park call, generates a code; retrieved from Calls > Parked calls on any device; rings back after timeout) [C] https://kb.uwm.edu/page.php?id=73785
- Record and transcribe (needs "Cloud recording for calling" and transcription policies; explicit-consent prompt for 1:1 VoIP calls when admin enables it, PSTN auto-consented; consent not supported for call-queue calls) [V] https://mc.merill.net/message/MC1163766
- Live captions (real-time captions on by default; not saved), Real Time Text (RTT) [V] https://learn.microsoft.com/microsoftteams/call-recording-transcription-captions
- Device settings in call (speaker, mic, camera); noise suppression; echo cancel; AGC [V]
- Video effects: background blur, custom background image; together mode in meetings [V/ACS]
- Spotlight / pin; raise hand; reactions (in meetings, 1:1 limited) [V/ACS]
- Chat during call (the call is attached to the chat thread; call chat opens alongside) [V in ACS thread-ID model]
- Convert to meeting / "Meet now" escalation of a group call [U]
- Call quality: network indicator, user-facing diagnostics, post-call quality survey (PCF) [V UFD in ACS; survey U]
- Join call on second device / "Join on another device" [V topic title]
- Delegation: delegate makes/receives calls on behalf, joins active calls, holds/resumes manager's call [V] https://learn.microsoft.com/en-us/microsoftteams/user-call-settings
- Max 30 h call; 350 participants for group call on ACS; 85 s 1:1 establishment timeout; 60 s transfer timeout [V ACS]

### 1.3 Call settings (Settings > Calls)

Source: https://support.microsoft.com/en-us/office/manage-your-call-settings-in-teams-456cb611-3477-496f-b31a-6ab752a7595f [V] and https://support.microsoft.com/en-us/office/call-forwarding-call-groups-and-simultaneous-ring-in-teams-a88da9e8-1343-4d3c-9bda-4b9615e4183e [V]

- Call answering rules ("Call handling and forwarding")
  - Forward my calls (immediate): to voicemail, new number, contact, call group, delegates. Applies to 1:1 calls only [V]
  - Calls ring me + "Also ring" (simultaneous ring): a contact, a call group, or delegates; optional delay up to 20 s (default none) [V]
  - If unanswered: voicemail, forward to number/contact, call group; ring seconds before redirect (admin console: selectable; default behavior is voicemail after about 30 s; PowerShell shows 20 s delay) [V] https://learn.microsoft.com/en-us/microsoftteams/user-call-settings
  - External-call routing to voicemail (off by default; admin-enabled) [V]
  - Requires Enterprise Voice / Business Voice license; license change can take 24 h [V]
- Call groups: up to 25 members from the org (no external numbers); ring "everyone at once" or "in order" (drag priority); membership notifications off by default [V]
- Delegates ("Manage delegates"): add by search; per-delegate flags MakeCalls, ReceiveCalls, ManageSettings; recommended max 25 relationships [V] (PowerShell shape: `Get-CsUserCallingSettings` returns IsForwardingEnabled, ForwardingType, ForwardingTargetType, UnansweredTargetType, UnansweredDelay, Delegates, CallGroupOrder, CallGroupTargets, GroupNotificationOverride)
- Voicemail ("Manage voicemail"): record greeting, greeting language, TTS greeting, out-of-office greeting and schedule, call answer rules (see 1.1) [V]
- Ringtones: separate ringtone for normal calls and for forwarded calls; per-device setting [V]
- Call display: large vs small incoming window [V]
- Call-waiting sounds [V]
- Accessibility: TTY mode [V]
- Privacy / blocking: edit blocked contacts; block calls with no caller ID (only Direct Routing today) [V]
- Devices: default speaker/mic/camera; secondary ringer (certified device); test call [V]
- Call queue notifications per queue (agent) [V]
- "Open apps in browser" for PSTN calls (admin-enabled) [V]
- SIP device feature codes: `*34*` forward on timeout, `*33*` forward always, `*32*` reset (desk-phone only) [V]
- Admin-controlled policies that gate client behavior: calling policy (private calls, cloud recording, transcription, forwarding to org/external, voicemail, call groups, delegation, prevent toll bypass, music on hold, busy-on-busy, spam filtering, SIP devices), voicemail policy, emergency calling policy [V] https://learn.microsoft.com/en-us/azure/communication-services/concepts/interop/teams-user-calling

### 1.4 PSTN, phone numbers, emergency calling

- PSTN options: Calling Plans, Operator Connect, Direct Routing (customer SBC), Teams Phone Mobile [V] https://learn.microsoft.com/en-us/microsoftteams/what-are-emergency-locations-addresses-and-call-routing
- Assigned phone number shown/hidden as caller ID; E.164 dialing; Teams Phone license (Enterprise/Business Voice) required for dial pad [V]
- Emergency calling
  - Registered (static) emergency address per user; dynamic emergency calling via network sites/trusted IPs and client location; security-desk conference-in and chat notification [V]
  - Documentation states dynamic emergency calling (incl. security desk notification) is not supported on the Teams web client [C: secondhand summary of Learn; confirm at https://learn.microsoft.com/microsoftteams/configure-dynamic-emergency-calling]
  - Location sharing consent prompt on first launch for numbered users [C]
  - Cannot place 911 without internet [C]
  - Implication for kelma: a third-party/web-based client is not a compliant emergency-calling endpoint. Block or redirect emergency numbers to the user's phone, and say so in the UI.
- SMS on PSTN numbers; call forwarding to external numbers is policy-gated [V/C]

### 1.5 Other adjacent calling features (completeness)
- Call queues and auto attendants (Queues app, agent opt-in, call-queue calls in history as multiParty) [C]
- Group call pickup; shared line appearance (delegation) [V]
- Call park / retrieve with code; parked call timeout ring-back [C]
- Call recording Recap in Calls history [V]
- Meeting-from-call escalation; Together mode; large gallery (meetings) [V ACS]

---

## 2. API and protocol notes

### 2.1 What kelma already has (per brief) and what it unlocks
- Chat service + CSA + trouter push with web client id 1fec8e78-bce4-4aaf-ab1b-5451cc387264; tokens for skypetoken, chatsvcagg, presence, Graph.
- This covers all "data" parts of Calls. Calling "control and media" is the hard part (2.4 onward).

### 2.2 Call history: native and verified
- Call history lives in a system chat thread `48:calllogs` (the "48:" prefix is Teams' system-stream prefix; others: 48:notifications, 48:mentions, 48:threads, 48:generic). [C] https://github.com/neicureuil/oxide/pull/62, https://heusser.pro/p/get-missed-call-notifications-for-teams-call-queues-free-community-solution-cozo3bhojb7t/
- Read path (Teams web client, undocumented): `GET https://teams.microsoft.com/api/csa/{region}/api/v1/chats/48%3Acalllogs/messages?messagePageSize=200` [C, one blog; same CSA host kelma already uses]. Each message carries `properties['call-log']` as a JSON string with fields `callType` (`multiParty` for call-queue), `startTime`, `callDirection`, `callState`, `callId`, `participants` (caller MRI-like, `4:<number>` for PSTN), call-queue id. [C]
- Caveats from the same source: the endpoint "does not return missed calls" for queue scenarios and iOS shows differently; Graph `GET /chats/48:calllogs/messages` works with delegated auth but returns less (no callState) and not at all with application permissions. [C] Treat "missed" detection as needing verification with a real tenant; the Calls app evidently shows missed calls, so they may be a different message type/state or a different endpoint [U].
- A forensic writeup of the desktop client confirms conversationId `48:calllogs` records and a voicemail-typed field in the local cache [C] https://www.alexbilz.com/post/2021-09-09-forensic-artifacts-microsoft-teams/ (could not fetch the page; reported via search snippet only).
- Live updates: call-log messages arrive as chat messages on 48:calllogs, so the existing trouter/chat-service subscription should receive them [U, test].
- Graph alternative for org-wide records: `callRecords` (CallRecords.Read.All, admin-only, post-call, minutes of delay). Not a user feature. [V] https://learn.microsoft.com/en-us/graph/api/resources/communications-api-overview

### 2.3 Voicemail
- Storage: Cloud Voicemail writes messages into the user's Exchange Online mailbox; Teams/Outlook read them from there. Message class `IPM.Note.Microsoft.Voicemail.UM` (or `.UM.CA`) is what makes Teams display it; transport-rule header changes can hide messages from Teams. [V] https://learn.microsoft.com/en-us/troubleshoot/microsoftteams/phone-system/voicemails-not-delivered
- Each voicemail email has an audio attachment (reported as `audio.mp3`) and transcription text in the body [C].
- No documented Teams voicemail API; a Microsoft reply (2023) says none exists [C] https://techcommunity.microsoft.com/discussions/teamsdeveloper/is-there-any-api-to-access-teams-voicemail/3852163
- Practical native paths:
  1. Graph Mail: list messages in the mailbox filtered by PidTagMessageClass starts-with `IPM.Note.Microsoft.Voicemail` (via `singleValueLegacyExtendedProperty` filter, id `String 0x001A`), fetch audio attachment, parse transcript from body. Needs Mail.Read delegated scope; whether the Teams web client id is pre-authorized for Mail.Read is unknown [U]. Microsoft's own FastTrack script reads voicemails this way with application permissions [C] (https://source-hub.etos-pro.ru/microsoft/FastTrack/src/branch/master/scripts/Get-MgUserVoicemailReport).
  2. Capture the exact REST calls the Teams Calls > Voicemail UI makes (likely Exchange/OWA-style or a Teams middle-tier endpoint) and call them with the tokens kelma already mints [U].
- Greetings, TTS text, out-of-office, answer rules: read/write through the Teams middle-tier "user settings" endpoints used by Settings > Calls [U]; admin analogue is `Get/Set-CsOnlineVoicemailUserSettings`.

### 2.4 Call settings (forwarding, simultaneous ring, unanswered, delegates, call groups)
- Admin API shape is documented via PowerShell (`Get-CsUserCallingSettings`, `Set-CsUserCallingSettings`, `New/Set/Remove-CsUserCallingDelegate`) and the property set listed in 1.3. [V] https://learn.microsoft.com/en-us/microsoftteams/user-call-settings
- The client uses a user-scoped middle-tier endpoint. I could not find it documented publicly [U]. Capture via HAR while toggling "Forward my calls" in teams.microsoft.com Settings > Calls, then replay with kelma tokens. Expect a small JSON model mirroring the PowerShell fields (ForwardingType Immediate/Simultaneous, ForwardingTargetType Voicemail/SingleTarget/MyDelegates/Group, UnansweredDelay, delegate permission bits).
- Changes need Teams Phone licensing server-side; enforce read-only UI when the user is not EV-enabled.

### 2.5 Presence of calls
- "In a call" presence is derived server-side; kelma already mints presence tokens. `Presence.Read` / `presence` resource in Graph gives availability + activity (InACall, InAConferenceCall) [V listing in https://learn.microsoft.com/en-us/graph/api/resources/communications-api-overview; activity values U].

### 2.6 Signaling and media: what is publicly known
Official (Microsoft Learn) [V] https://learn.microsoft.com/en-us/microsoftteams/microsoft-teams-online-call-flows and https://learn.microsoft.com/en-us/azure/communication-services/concepts/detailed-call-flows:
- Signaling is "HTTPS-based REST interfaces" (SIP only toward SBCs). Media is RTP secured with SRTP.
- 1:1 flow: caller does DNS, allocates a relay on the Teams Transport Relay, sends "invite" with ICE candidates to Microsoft 365, M365 notifies callee, callee allocates its relay and sends "answer" with ICE candidates, then ICE connectivity checks pick the best path (host, server-reflexive, relay), telemetry is sent afterwards. SDP offer/answer over HTTPS (RFC 8866). ACS docs state the DTLS handshake derives SRTP keys for ACS/Teams client interop.
- Transport relay gives ICE/STUN/TURN; access uses a credential token over a TLS channel; UDP 3478 (STUN), 3479 (audio), 3480 (video), 3481 (sharing/VBSS); TCP 443 as last resort fallback. Teams client-side media port range UDP 50000-50059 is commonly documented. [V/C]
- Teams is full-ICE; Direct Routing SBC is ICE-lite. [V]
- Third-party media relays are unsupported, only Teams/SfB native relays. [V]
- Group calls and meetings: conference server / media processor in M365 with public IP; client sends invite with candidates, server answers with its host candidate. [V]
- Call notifications to the callee go over the endpoint's push channel (trouter) [U but consistent with step 4 "Microsoft 365 sends notification to Teams User B"].
Community / forum [C]:
- Codecs: a Microsoft Q&A thread says Satin replaced SILK and G.722 as the primary Teams audio codec (desktop SlimCore), H.264 primary video, AV1 only when all participants support it. Not an official page. https://learn.microsoft.com/en-us/answers/questions/5849006/media-relay-fallback-and-satin-codec-degradation-o
- The only deep public look at the shared Skype/Teams/ACS calling stack as exposed to a browser is a 2020 webrtchacks analysis of the ACS web SDK (beta): signaling over HTTP with proprietary JSON; single RTCPeerConnection, Plan B (not Unified Plan); bundle, rtcp-mux, rtcp-rsize; SDES key exchange to the SFU (Chrome only); G.722 offered first then PCMU/PCMA/CN/telephone-event, no Opus; H.264 + RTX (VP8 P2P only); no simulcast; REMB receiver-side BWE; non-standard `x-` SDP attributes (`x-mediabw`, `x-signaling-fb`); TURN UDP 3478 and TCP 443. 2020 and beta, so treat as a hint of the lineage not current truth. https://webrtchacks.com/how-does-azure-communication-services-implement-webrtc-gustavo-garcia/
- "NGC", "conversation controller" and trouter call-notification terms from the brief: I found no public source documenting them. [U] Do not plan on public docs.
- No open-source project implementing Teams calling was found (see 2.8).

Likely shape of the Teams web calling flow [U, to confirm by capture]:
1. Browser registers an endpoint with chat service (already done in kelma) and registers calling capability/trouter path.
2. Place call: POST to a call-controller REST endpoint with SDP offer (+ ICE candidates, TURN creds fetched from relay config), receives call id + callee state events via trouter and/or long-poll.
3. Incoming: trouter push with invite and remote SDP; client POSTs answer/accept/decline.
4. Mid-call control (hold, transfer, add participant, DTMF, mute reports, roster): REST calls plus roster events; in-call data channel(s) possible.
5. ECS config service supplies feature flags and calling endpoints per tenant.
Plan to produce a HAR of: 1:1 audio call out, incoming accept/decline, add person, transfer, hold, DTMF, then diff against ACS SDK verbose logs (the ACS SDK is a legitimate reference for message shapes since "call flows ... call SDK and Teams clients are based on SDP offer/answer over HTTPS").

### 2.7 Azure Communication Services (ACS) Teams interop
Doc sources: https://learn.microsoft.com/en-us/azure/communication-services/concepts/teams-interop , .../concepts/interop/teams-user-calling , .../concepts/voice-video-calling/calling-sdk-features , .../quickstarts/manage-teams-identity [all V]

- Identity models: External user (BYOI; can join Teams meetings; "direct calling and chat" to Teams users listed as future; a Teams user cannot join a call started by an ACS user) vs Teams user (CTE: Entra-authenticated, Teams-licensed, acts as that Teams user: place/receive calls with other Teams users, receive PSTN, transfer to queue, share screen).
- "Teams user" (CTE) 1:1 and group calling capabilities: place 1:1 call to Teams user; group call up to 350; promote 1:1 to group; add Teams user or phone number; accept/reject incoming; local hold; mute/unmute; video on/off; screen share (platform-dependent; web full); background blur; custom background (web); captions manage (all); raise hand; honor forwarding, simultaneous ring, DND, call groups, delegation, shared line; be parked; transfer to user, to call, to voicemail, to auto attendant/queue; receive from auto attendant and queue; honor recording/transcription state info.
- NOT available via CTE SDK: park a call; merge ongoing calls; read/configure forwarding rules, simultaneous ring, shared-line config; start call on behalf of delegator; manage cloud recording or transcription (only get notified); reactions; join an existing group call after start (listed as unsupported for Teams users); receive Together/large gallery; PowerPoint Live, Whiteboard; Teams real-time analytics.
- Platforms: JavaScript (web), Windows (.NET/WinUI), Android (Java), iOS (Obj-C). Linux appears only as a browser target for the JS SDK: Ubuntu/Linux = Chrome only; Firefox/Edge/Safari/Webview/Electron are marked unsupported on Linux. Embedded WebViews other than Android System WebView and iOS WKWebView (preview) are unsupported (CEF, WebView2, GeckoView "might or might not work"). Therefore: no native Linux SDK, no Rust SDK, JS in Chrome tab only.
- Prerequisites (this is the blocker for a general desktop client):
  - Your own Entra app registration with API permissions `Teams.ManageCalls` and `Teams.ManageChats` (resource app id 1fd5118e-2576-4263-8130-9503064c837a, scope URIs `https://auth.msft.communication.azure.com/Teams.ManageCalls`), public client flow enabled, localhost redirect
  - Tenant admin consent (Global/Application/Cloud Application admin) in every customer tenant; may need to create the ACS service principal there
  - An ACS resource in your Azure subscription; token exchange `getTokenForTeamsUser({teamsUserAadToken, clientId, userObjectId})` signed by ACS key/RBAC (recommended server-side)
  - Teams-licensed user; GCC (government) tenants unsupported
  - Billing: ACS calling minutes metered to the developer's resource; Teams-side minutes covered by the user's license; no extra interop fee. Token TTL for CTE max 24 h.
  - Teams-client features such as dynamic E911 are not exposed; ACS Teams-user calling inherits tenant policies though.
- Notifications: incoming calls via `TeamsCallAgent.on('incomingCall')` while the page is alive; push notification integration exists for web/mobile.
- Verdict: legitimate and robust but JS-in-Chrome on Linux, per-tenant admin consent, billing. It does not remove the need for an embedded Chromium. It is only advantageous over "hosting teams.microsoft.com" in that you control the UI (call controls, layout) and use a supported API surface.

### 2.8 Microsoft Graph cloud-communications APIs
- Purpose: service apps (bots) that act as call participants: create/answer/reject/redirect/transfer calls, invite/mute participants, play prompt, record response, DTMF, app-hosted or service-hosted media, call records, online meetings. Permissions are application permissions granted by admin (Calls.Initiate.All, Calls.InitiateGroupCall.All, Calls.JoinGroupCall.All, Calls.AccessMedia.All, CallRecords.Read.All, OnlineMeetings.*). It makes the app a participant, not a user client; Microsoft forbids recording or persisting call media via these APIs. [V] https://learn.microsoft.com/en-us/graph/api/resources/communications-api-overview , https://learn.microsoft.com/en-us/graph/api/resources/call?view=graph-rest-1.0
- Not usable to make kelma the user's softphone. Useful only for: `callRecords` (admin), `onlineMeetings` create/lookup, presence.

### 2.9 Existing open source / prior art
- purple-teams (EionRobb): Pidgin/libpurple plugin, chat only. "Calls, voice, and video: not supported" and lists calls as a drawback pointing users to the Teams website. [V] https://github.com/EionRobb/purple-teams
- fossteams/teams-api (Go): "can only handle a couple of endpoints"; consumes pre-obtained tokens (chatsvcagg, skype, teams jwt); no calling. [V] https://github.com/fossteams/teams-api
- teams-for-linux (Electron wrapper of the Teams web app, GPLv3): calling works because it is the real web client in Chromium; limitations tied to what web Teams offers; Wayland screen share needed PipeWire flags / Electron bug 37463 historically. [C] https://github.com/IsmaelMartinez/teams-for-linux , https://github.com/IsmaelMartinez/teams-for-linux/issues/886
- Matrix bridges: Element's Teams bridge (2021) explicitly did not support calls ("a much more technologically hard problem"); mx-puppet-teams and mautrix-teams list messaging only. [C] https://www.theregister.com/2021/04/14/matrix_teams/ , https://github.com/neilsb/mx-puppet-teams
- Old Skype protocol reverse-engineering (2000s-2011) is a different, deprecated stack; legal note that Skype's license prohibited RE. [C] https://en.wikipedia.org/wiki/Skype_protocol
- No public GitHub project for "ngc", conversation controller or Teams call signaling was found.
- Teams VDI: Microsoft moved VDI optimization from WebRTC redirection to SlimCore (the desktop client's own media engine, MSIX); Linux thin clients still rely on the older WebRTC redirection and Microsoft's own Linux timeline is unclear. Citrix docs: https://docs.citrix.com/en-us/citrix-daas/multimedia/opt-ms-teams-new/ms-slimcore-optimization ; Microsoft: https://learn.microsoft.com/MicrosoftTeams/vdi-2 [C/V mixed]. Shows the web client's calling stack is WebRTC-based, and that the desktop app's media stack (SlimCore/Satin) is separate and proprietary.

### 2.10 Webview embedding facts
- Teams web client supports calling on Linux in Chromium-based browsers (Edge/Chrome); Microsoft ended the native Linux client in Dec 2022 and recommends the PWA/web app. [C] https://linuxiac.com/linux-users-can-now-enjoy-ms-teams-as-pwa-app/ , https://petri.com/microsoft-teams-pwa-linux-machines/
- Dynamic E911 not supported on web client. [C]
- ACS JS SDK only supports Chrome on Linux; WebViews unsupported. [V]
- Deep links: `https://teams.microsoft.com/l/call/0/0?users=<mri-or-upn>&withVideo=true` opens a call in web client; `msteams://` skips picker for desktop client only. [V for deep links in general; exact call params U]
- Screen sharing on Wayland requires the PipeWire portal capturer in Chromium (`--enable-webrtc-pipewire-capturer`). [C] https://aur.archlinux.org/pkgbase/teams-for-linux
- WebKitGTK: Teams web gating (UA sniffing for Chromium) and WebRTC maturity make it a poor target [U]. Use Chromium (CEF via cef-rs / CEF-based wrapper, or a spawned Chrome/Edge `--app=` window with its own `--user-data-dir`).

---

## 3. Feasibility and recommended architecture

### 3.1 Verdict table

| Area | Path | Risk | Notes |
|---|---|---|---|
| Call history list (missed/incoming/outgoing/declined, filters, call back, add to speed dial) | Native Rust via CSA `48:calllogs` + chat service/trouter | Low-Med | Missed-call representation needs verification; Graph fallback has less data. |
| Contacts, speed dial, contact groups | Native (People/CSA/Graph; speed dial via a Teams user-properties endpoint) | Low-Med | Speed dial store endpoint unknown [U]; capture. |
| Presence incl. "in a call" | Native, already working | Low | |
| Voicemail list, transcript, audio playback, delete, call back | Native: Graph Mail (message class) or captured Teams endpoint; Rust audio via symphonia/rodio/GStreamer | Medium | Needs Mail.Read availability for web client id; confirm. Playback is trivial once audio bytes fetched. |
| Voicemail greetings and voicemail settings | Native via captured settings endpoint; recording a greeting via mic capture and upload | Med-High | Greeting upload format/endpoint unknown. |
| Call forwarding, simultaneous ring, unanswered, delegates, call groups, ringtones (local), call-queue opt-in | Native UI over captured middle-tier settings endpoints | Medium | JSON shape documented via PowerShell parity; endpoint capture needed. Ringtone, call-display, devices are client-local. |
| Incoming-call detection and toast (caller, video requested, queue/group context) | Native: consume trouter push already connected | Medium | Need to decode call-invite payload; no public docs [U]. |
| Place / accept / decline / mid-call control / media (1:1, group, audio/video, screen share, hold, transfer, DTMF, add people, captions, record) | Needs call signaling reverse engineering + native WebRTC (libwebrtc via `livekit`/webrtc-sys bindings, or webrtc-rs, or GStreamer webrtcbin), TURN credentials from relay config, Teams-flavored SDP (Plan B / `x-` attrs), codec set (H.264 + G.722/PCMU or whatever Teams accepts from browsers), roster/control protocol | High-Very High | No prior art; ToS and change risk; Microsoft can change the protocol at any time. Full feature parity (park, merge, recording, E911) is not realistic. |
| Same via ACS CTE SDK in embedded Chromium | Supported API but JS-only, per-tenant admin consent, ACS billing | Medium (technical) / High (distribution) | Covers 1:1 & group calls, transfer, hold, DTMF? (PSTN dial-out/DTMF listed under PSTN SDK), add participants, captions, blur. Missing park/merge/forwarding config. Chrome only on Linux; CEF/WebView "might not work". |
| Same via official Teams web client in embedded Chromium (CEF or Chrome `--app`) | Fallback; zero reverse engineering | Low (technical) / Medium (UX seams, ToS-neutral since it is the real web client) | Gets full web feature set incl. park, merge, record, captions, settings. No dynamic E911. Brittle if automated via DOM/CDP. |
| Emergency calling | Do not offer. Block/redirect to a phone, show notice. | n/a | Web client lacks dynamic E911; unofficial client cannot be a compliant endpoint. |

### 3.2 Recommended phased approach

Phase 0: capture and decide (about 1 week, no product code)
- Record HARs from teams.microsoft.com (Chromium, DevTools or CDP) for: open Calls tab, history scroll, play voicemail, change forwarding/unanswered/delegates/voicemail greeting, add speed dial, place 1:1 audio call, incoming accept/decline, hold, transfer, add person, DTMF, park, merge.
- Record ACS verbose logs from a CTE sample app for the same calls to learn message shapes (ACS is the documented sibling).
- Confirm: `48:calllogs` shape and missed calls; voicemail source; Mail.Read availability for 1fec8e78 client id; settings endpoints; which trouter message types carry call invites.
- Check tenant licensing for test users (Teams Phone Standard needed for forwarding, PSTN, voicemail transcription).

Phase 1: native "Calls data" tabs (the safe, high-value 70%)
- History (all/missed/voicemail filters, call back via deep link/webview handoff, chat, add to speed dial/contacts, Recap link).
- Voicemail list + transcript + audio playback (+ speed control, delete, call back).
- Contacts/speed dial/contact groups; presence badges.
- Read-only call settings view first (forwarding status, unanswered, delegates, call groups, voicemail greeting status), then writes (forward on/off, simultaneous ring, delay, unanswered target, delegate permission bits, ringtone local).
- Incoming-call native toast driven by trouter; Accept/Decline initially hand off to the webview call surface (Phase 2).
- Guard rails: hide dial pad when not EV-enabled; show "emergency calls unsupported" if an emergency number is typed.

Phase 2: calls via embedded Chromium, behind a trait (`CallSurface`) so the engine can change later
- Option A (default, fastest): CEF window (or system Chrome/Edge `--app` window with a kelma-owned profile) loading the official Teams web client; launch calls with the `l/call/0/0?users=...&withVideo=` deep link; native toast for incoming; Accept focuses the web call window (the web endpoint also rings). Use CDP only for coarse signals (call started/ended, window focus) rather than pixel-level DOM automation.
- Option B (own UI, supported API): CEF/Chrome tab hosting an ACS Teams-user page, driven over a local bridge (wry/CEF messaging), kelma renders its own call chrome in Rust. Only for tenants (the user's own first) that can grant admin consent and take ACS billing; adds a second sign-in (Entra app of yours, not the Teams client id).
- Linux specifics: PipeWire portal screen share on Wayland, camera via V4L2, audio via PipeWire/PulseAudio, echo cancel/noise suppression done by Chromium (webrtc audio processing).
- Delivers: 1:1/group audio+video, mute/camera/share/hold/transfer/DTMF/add/captions/record, call-from-chat, devices, effects. Park/merge/record exist only in Option A.

Phase 3 (optional, R&D, only if business case): native Rust media
- Spike with libwebrtc bindings (`webrtc-sys`/LiveKit's rust-sdks `libwebrtc`) for audio-only 1:1 first (G.722/Opus/PCMU as accepted, SRTP via DTLS), TURN from Teams relay config, SDP shaped like a captured browser offer; then video H.264. Set kill criteria after a 2-3 week spike (if the call controller rejects non-browser fingerprints, or requires attestation, stop).
- Never ship E911 or PSTN as a headline; keep as experimental.

### 3.3 Risks and open questions
- Terms/legal: undocumented endpoint use may breach Microsoft terms; Cloud Communications API license also forbids persisting call media. Get a position before shipping beyond internal use. (Not researched exhaustively.)
- Conditional Access: tenants requiring compliant devices or specific client apps can block non-Microsoft clients and web client id reuse (affects every phase).
- Endpoint churn: Teams middle-tier and trouter payloads change without notice; keep decoders tolerant, add contract tests from captured fixtures.
- Voicemail path depends on Graph Mail consent; may need a separate app registration if web client id lacks scope.
- ACS CTE: token TTL 24 h, GCC unsupported, ACS billing, Chrome-only on Linux, WebView/CEF unsupported ("might or might not work").
- Emergency calling: unsupported; communicate clearly.
- Roster of missed calls vs `48:calllogs` completeness to be tested on a real tenant.

---

## 4. Sources

Microsoft Learn / support
- Get to know the calling experience in Teams: https://support.microsoft.com/en-us/office/get-to-know-the-calling-experience-in-teams-5c71cfd7-0553-4816-aa35-1fdae1ba2365
- Tips for Teams calls: https://support.microsoft.com/en-us/office/tips-for-teams-calls-d3fb0e97-4afa-4919-86e8-021102ed5c2b
- Manage your call settings in Teams: https://support.microsoft.com/en-us/office/manage-your-call-settings-in-teams-456cb611-3477-496f-b31a-6ab752a7595f
- Call forwarding, call groups, simultaneous ring: https://support.microsoft.com/en-us/office/call-forwarding-call-groups-and-simultaneous-ring-in-teams-a88da9e8-1343-4d3c-9bda-4b9615e4183e
- Configure call forwarding and delegation settings (admin, PowerShell shape): https://learn.microsoft.com/en-us/microsoftteams/user-call-settings
- Voicemail greetings and language: https://learn.microsoft.com/en-us/microsoftteams/change-the-default-language-for-greetings-and-emails
- Voicemails not delivered (message class): https://learn.microsoft.com/en-us/troubleshoot/microsoftteams/phone-system/voicemails-not-delivered
- Voicemail API question (no API): https://techcommunity.microsoft.com/discussions/teamsdeveloper/is-there-any-api-to-access-teams-voicemail/3852163
- Call recording, transcription, captions: https://learn.microsoft.com/microsoftteams/call-recording-transcription-captions
- Explicit consent for 1:1 recording (Message Center MC1163766): https://mc.merill.net/message/MC1163766
- Emergency calling concepts: https://learn.microsoft.com/en-us/microsoftteams/what-are-emergency-locations-addresses-and-call-routing
- Dynamic emergency calling: https://learn.microsoft.com/MicrosoftTeams/configure-dynamic-emergency-calling
- Teams call flows (signaling, relay, ports): https://learn.microsoft.com/en-us/microsoftteams/microsoft-teams-online-call-flows
- Teams deep links: https://learn.microsoft.com/en-us/microsoftteams/platform/concepts/build-and-test/deep-links
- New VDI solution for Teams (SlimCore): https://learn.microsoft.com/MicrosoftTeams/vdi-2
- Q&A on Satin codec / relay fallback: https://learn.microsoft.com/en-us/answers/questions/5849006/media-relay-fallback-and-satin-codec-degradation-o

Azure Communication Services
- Teams interoperability: https://learn.microsoft.com/en-us/azure/communication-services/concepts/teams-interop
- Calling capabilities for Teams users (feature matrix): https://learn.microsoft.com/en-us/azure/communication-services/concepts/interop/teams-user-calling
- Calling SDK overview (platforms, Linux browser matrix, limits): https://learn.microsoft.com/en-us/azure/communication-services/concepts/voice-video-calling/calling-sdk-features
- Call flow topologies (SDP over HTTPS, DTLS-SRTP, relay): https://learn.microsoft.com/en-us/azure/communication-services/concepts/detailed-call-flows
- Set up access tokens for Teams users (Entra app, Teams.ManageCalls, consent): https://learn.microsoft.com/en-us/azure/communication-services/quickstarts/manage-teams-identity
- Teams-user calling quickstart: https://learn.microsoft.com/en-us/azure/communication-services/quickstarts/voice-video-calling/get-started-with-voice-video-calling-custom-teams-client
- Manage calls for Teams users: https://learn.microsoft.com/en-us/azure/communication-services/how-tos/cte-calling-sdk/manage-calls
- Identity model: https://learn.microsoft.com/en-us/azure/communication-services/concepts/identity-model

Microsoft Graph
- Cloud communications API overview: https://learn.microsoft.com/en-us/graph/api/resources/communications-api-overview
- call resource: https://learn.microsoft.com/en-us/graph/api/resources/call?view=graph-rest-1.0

Community / open source
- WebRTC Hacks, ACS WebRTC analysis (2020): https://webrtchacks.com/how-does-azure-communication-services-implement-webrtc-gustavo-garcia/
- 48:calllogs call history via CSA/Graph: https://heusser.pro/p/get-missed-call-notifications-for-teams-call-queues-free-community-solution-cozo3bhojb7t/
- 48: system streams list: https://github.com/neicureuil/oxide/pull/62
- Teams desktop forensic artifacts (not fetched, snippet only): https://www.alexbilz.com/post/2021-09-09-forensic-artifacts-microsoft-teams/
- purple-teams (no calls): https://github.com/EionRobb/purple-teams
- fossteams/teams-api: https://github.com/fossteams/teams-api
- teams-for-linux: https://github.com/IsmaelMartinez/teams-for-linux and issue https://github.com/IsmaelMartinez/teams-for-linux/issues/886
- Element Teams bridge, no calls (2021): https://www.theregister.com/2021/04/14/matrix_teams/
- mx-puppet-teams: https://github.com/neilsb/mx-puppet-teams
- Teams PWA on Linux: https://linuxiac.com/linux-users-can-now-enjoy-ms-teams-as-pwa-app/ , https://petri.com/microsoft-teams-pwa-linux-machines/
- Call park/transfer/merge steps (university KB): https://kb.uwm.edu/page.php?id=73785
- Voicemail greeting steps (university KB): https://bowdoin.teamdynamix.com/TDClient/1814/Portal/KB/Article/137899/Record-or-Change-Your-Voicemail-Greeting-in-Microsoft-Teams
- FastTrack voicemail report script mirror: https://source-hub.etos-pro.ru/microsoft/FastTrack/src/branch/master/scripts/Get-MgUserVoicemailReport
- Citrix SlimCore doc: https://docs.citrix.com/en-us/citrix-daas/multimedia/opt-ms-teams-new/ms-slimcore-optimization
- Wayland PipeWire flags (AUR comments): https://aur.archlinux.org/pkgbase/teams-for-linux
