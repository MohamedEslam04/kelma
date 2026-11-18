# kelma research: Microsoft Teams Calendar and Meetings (feature inventory, APIs, feasibility)

Date of research: 2026-11-18. Confidence tags used below:
- [V] verified this session against a Microsoft Learn / support page (URL in Sources)
- [K] from general product knowledge, not re-verified this session; confirm by capturing the Teams web client network traffic before building
- [?] uncertain / needs a live probe with kelma's token

Scope note: "Teams" here means the new Teams (work/school, Entra ID) desktop/web experience. Personal (teams.live.com) accounts differ and are mostly out of scope.

---------------------------------------------------------------------------------------------------

## 1. Feature inventory: Calendar app

### 1.1 Shell and navigation
- Calendar app in left rail (new Teams: Outlook-powered calendar, same engine as new Outlook / OWA) [V: third-party + MS docs; K]
  - Entry points to "schedule": Calendar tab "New meeting" button, click/drag a time slot, Chat -> "More chat options" -> "Schedule a meeting", Teams/Meet-now area dropdown -> "Schedule a meeting", channel "Meet" dropdown -> "Schedule a meeting" [V]
  - "Meet now" button (instant meeting) [V]
  - "Join with an ID" button (meeting ID + passcode) [K]
  - "Today" button, prev/next arrows, date picker mini-calendar (left pane) [K]
  - Calendar list pane (left): My calendars, Other calendars/Shared with me, People calendars (add a coworker), Group/Team calendars, Room/Resource calendars, calendar groups, checkbox per calendar to overlay (color per calendar) [K]
  - Split view / saved views across several calendars [V third-party: Virto guide; K]
  - Add calendar: from directory (people), from room list, from Microsoft 365 group, "Discover calendars" (holidays, sports) [K]

### 1.2 Views
- Day [V]
- Work week (default; honors Exchange working-days setting) [V]
- Week (7 days) [V]
- Month [V]
- Agenda / list view (upcoming events list) [K; classic Teams had no agenda, new calendar has "Agenda" in view dropdown] [?]
- Mini-calendar month picker (left)
- Split view (multiple calendars side by side) [V third-party]
- Time-scale zoom (30/15 min, etc.) [K]
- Second time zone column (add extra time zone in calendar settings) [K]
- Week numbers toggle [K]
- Show weekends toggle [K]
- Current-time indicator line [K]
- Drag and drop event to reschedule, drag edge to resize [V]

### 1.3 Filters and display
- Show working hours only / show full day (working hours synced from Exchange mailboxSettings.workingHours) [V third-party, K]
- Categories: color-code, filter by category, right-click -> Categorize (one or more, choose color; channel meetings cannot be categorized) [V]
- "Show as" status: free / tentative / busy / OOF / working elsewhere (default Busy) [V]
- Filter: "Meetings" vs "All events" / "Only meetings I organize" / "Hide declined" [?]
- Search events (search box in calendar header) [K]
- Declined events shown struck-through; tentative hatched; private lock icon [K]
- Online meeting icon (Teams logo) on events; "Join" button appears on event card ~5 min before start; recurring icon; response-status chips [K]
- Hover card / quick-view popup: title, time, organizer, Join button, RSVP buttons [K]

### 1.4 New meeting form (new Teams / Outlook-style form)
Fields, in form order [V where marked; rest K]:
- Title [V]
- Required attendees (people picker; external needs full email; also "Add channel" to invite a channel to a regular meeting, cannot be edited after send) [V]
- Optional attendees (Response options -> "Add optional attendees") [V]
- Date, start time, end time [V partly]
- All day toggle [K]
- Time zone selector (and "Show time zones" for second zone) [K; support page only mentions time zones in Scheduler]
- Repeat / recurrence: Does not repeat, daily, every weekday, weekly, monthly, yearly, custom (interval, weekdays, nth weekday of month, end by date/count/no end) [K]
- Scheduling assistant / "Scheduler": grid of attendees with free/busy per slot, time zone per attendee, busy/tentative shading, suggested times, room finder [V]
- Location: free text, room picker (rooms/equipment from Places/room lists), "Add location" with multiple locations, "Add room" button, room availability indicator [K]
- Teams meeting toggle (online meeting on/off); meeting sets joinUrl, dial-in conference id, phone numbers [V]
- Meeting details/body (rich text editor, attachments, Loop component insert) [K]
- Category [V]
- Response options: Request responses, Allow new time proposals, Hide attendee list, optional attendees [K; Graph: responseRequested, allowNewTimeProposals, hideAttendees]
- Show as / busy status, Importance (high/normal/low), Sensitivity/Private, Reminder (minutes) [K]
- Channel meeting: pick Team and Channel (meeting posts in channel, all team members invited; can add outside attendees) [V]
- Meeting options link (opens Teams meeting options page, organizer only) [V]
- Meeting template picker (Teams Premium / custom meeting templates) [V: Graph meetingTemplateId; K]
- Copy join link / Share -> "Copy join link" [V]
- Dial-in number + Conference ID (needs Audio Conferencing license / PSTN) [V]
- Send / Save / Discard; "Send update" when editing; "Cancel meeting" [V]
- Co-organizers (via meeting options) [V]
- Webinar / Town hall: separate entry points (Calendar -> New meeting dropdown -> "Webinar" / "Town hall"; or Calendar "Event" types) with own forms (registration, speakers, attendee access, event group, theme, Q&A, recording) [K]

### 1.5 Event types
- Standard meeting (calendar event + Teams meeting) [V]
- Appointment (no attendees) / personal event [K]
- Channel meeting [V]
- Meet now (adhoc meeting, "meetnow" meetingType) [V: Graph meetingType]
- Scheduled recurring meeting (recurring with one chat) [V]
- Webinar (registration, presenters, Teams Premium licensing for some features) [K; Graph virtualEvents beta/1.0]
- Town hall (replaces Teams live events, which stopped returning Graph data 2024-09-30) [V]
- Live event (retired for creation; legacy) [V]
- Teams Rooms / Surface Hub meetings (room as attendee) [K]
- Personal meeting ID / PMI-style "Join with ID" (Teams "meeting ID + passcode" per meeting) [V: joinMeetingIdSettings]

### 1.6 Event card / details view (open an event)
- Title, time, time zone, recurrence summary, organizer, attendees with per-attendee response status and tracking tab [K]
- Join button, "Join with ID" info, dial-in info, "Copy link", "Meeting options" (organizer), "Chat with participants" / "Open meeting chat" [K]
- Meeting details/recap tabs (Recap, Chat, Files, Whiteboard, Notes, Recordings/Transcripts, Attendance) in the meeting's chat after meeting [K]
- Edit (organizer), Delete / Cancel meeting (organizer sends cancellation), Cancel occurrence vs series [V Graph: cancel]
- Forward (Graph forward), "Add attendees" (organizer) [V]
- RSVP: Accept, Tentative, Decline each with: send/no response, add a message; Propose new time (accept/decline/tentative + proposed new time) [V Graph: allowNewTimeProposals; accept/decline with proposedNewTime]
- Reminders: snooze, dismiss [V Graph]
- Attachments, Loop pages/notes, whiteboard link, agenda [K]
- Print, duplicate event, "Mark as private", "Show as" [K]

### 1.7 Integration, sync, sharing
- Sync with Outlook: single Exchange Online mailbox/calendar, identical data both ways, no separate sync (Teams reads Exchange) [V]
- Shared calendars (delegate / shared access, "Calendars.ReadWrite.Shared"), delegate scheduling on behalf of someone [V]
- Group/Team calendars (M365 group calendar; channel calendar tab not available in private channels) [V third-party]
- Room and equipment resources: availability, room lists, room suggestions, Teams Rooms [V: Place.Read.All]
- Places / work location: set work location per day (office/remote/specific building) and show colleagues' locations; "Places" app; work hours and location as part of presence [K, V partial: Presence includes location]
- Calendar permissions dialog (share my calendar: can view free/busy, titles, all details, edit, delegate) [K]
- Calendar notifications and meeting-start toasts, "join in X minutes" banner in activity feed [K]
- Time zone and working hours settings (Settings -> Calendar) [K]
- Out of office / automatic replies (mailboxSettings) [K]
- Booking/Bookings app, Shifts schedules (Schedule.Read.All) as adjacent apps [V scope]

---------------------------------------------------------------------------------------------------

## 2. Feature inventory: Meeting options (organizer settings)

From the official "Meeting options" support page [V] plus Graph onlineMeeting properties [V]. Graph property in brackets.

### 2.1 Meeting access
- Sensitivity label (Teams Premium); locks some other options [sensitivityLabelAssignment]
- Who can bypass the lobby: Everyone / People in my org and guests / People in my org / People in my org, trusted orgs and guests / Only me and co-organizers / Specific people [lobbyBypassSettings.scope, isDialInBypassEnabled]
- People dialing in can bypass the lobby (toggle) [lobbyBypassSettings.isDialInBypassEnabled]
- Who can admit from the lobby: organizers, co-organizers, presenters (default) or organizers and co-organizers [allowedLobbyAdmitters]
- Show meeting info on join screen (title, attendee count, organizer) 
- Reject anyone who can't bypass the lobby (toggle)
- Announce when people dialing in join or leave [isEntryExitAnnounced]
- Require unverified participants to verify info (OTP via email; Teams Premium)
- Verify external presenters before joining an event (verification level)

### 2.2 Advanced protection (mostly Teams Premium)
- Turn off copying and forwarding of meeting chat and transcripts [allowCopyingAndSharingMeetingContent]
- End-to-end encryption (toggle; limits features; ACS cannot join E2EE meetings) [isEndToEndEncryptionEnabled]
- Watermark on shared content, watermark on everyone's video [watermarkProtection]
- Prevent screen captures (unsupported platforms join audio-only)
- Detect sensitive content during screen sharing

### 2.3 Roles
- Choose co-organizers
- Who can present: Everyone / People in my org / Specific people / Only me [allowedPresenters: everyone, organization, roleIsPresenter, organizer]
- Enable language interpretation (interpreter role, language channels)

### 2.4 Production tools
- Manage what attendees see (limits attendee mic/camera view; for large meetings)
- RTMP-In (external encoder)
- Enable Green room (organizer/presenters join before attendees)

### 2.5 Participation
- Allow mic for attendees [allowAttendeeToEnableMic]
- Allow camera for attendees [allowAttendeeToEnableCamera]
- Meeting chat: Enabled (anytime) / In-meeting only / Disabled [allowMeetingChat: enabled, limited, disabled]
- Q&A (on/off; unmoderated or moderated by co-organizers)
- Allow reactions [allowTeamworkReactions]
- Hide attendee names (presenters must be set before the meeting)
- Allow attendance report [attendanceReports]
- Allow breakout rooms [allowBreakoutRooms]
- Allow participants to change their name [allowParticipantsToChangeName]
- Allow PowerPoint Live sharing [allowPowerPointSharing]; Allow Whiteboard [allowWhiteboard]; Allow Live Share (Loop/apps) [allowLiveShare]
- Chat restrictions (who can post: everyone, organizers only, etc.) [chatRestrictions]
- Share meeting chat history default with new participants: all / none [shareMeetingChatHistoryDefault]

### 2.6 Recording and transcription
- Record and transcribe automatically [recordAutomatically]
- Who can record and transcribe: organizers + co-organizers / plus presenters / no one (needs Teams Premium or Copilot license) [allowRecording, allowTranscription]
- Who has access to recording and transcript (everyone / organizers + co-organizers / specific people; not for channel meetings)
- Enable multilingual speech recognition (Premium/Copilot; not in town halls)
- Spoken language of the meeting [meetingSpokenLanguageTag]

### 2.7 Copilot and AI
- Allow Copilot: Off / Only during the meeting / During and after the meeting (needs transcript for post-meeting)
- Facilitator (AI notetaker agent, needs Copilot)

### 2.8 Accessibility
- Provide CART captions (human captioner link)

### 2.9 Other Graph-visible properties
- joinMeetingIdSettings (meeting ID, passcode, isPasscodeRequired; immutable after creation)
- audioConferencing (conferenceId, tollNumber, tollFreeNumber, dialinUrl)
- videoTeleconferenceId (CVI), cloudVideoInteropInfo
- meetingTemplateId, meetingType (adhoc, scheduled, recurring, broadcast, meetnow)
- expiryDateTime (online meeting resource expires 60 days after start/end unless updated)
- meetingOptionsWebUrl (organizer-only web page for options)

---------------------------------------------------------------------------------------------------

## 3. Feature inventory: In-meeting

### 3.1 Join flows
- Join from calendar card, from meeting chat ("Join" banner), from activity notification, from link (teams.microsoft.com/l/meetup-join/..., msteams:// protocol), via Join with ID (ID + passcode), from phone dial-in, from Teams Rooms/"Join from room" proximity [K]
- Meet now: instant meeting from calendar/chat/Teams home; "Get a link to share", "Start meeting" [K]
- Join as: signed-in user, guest, anonymous (name only; requires policy allow) [V ACS]

### 3.2 Pre-join screen
- Name display; camera on/off toggle; mic on/off toggle; speaker/mic/camera device pickers; speaker/mic test ("Make a test call"); background effects (None, Blur, preset images, custom upload, Teams Premium branded); video effects (soften, brighten, spotlight/touch-up); noise suppression (Auto, Low, High, Off); audio options: Computer audio / Phone audio ("Call me" / "Dial in") / Room audio / Don't use audio; "Join from room" (Bluetooth/ultrasound proximity); "Join now" / "Ask to join" (lobby); meeting info panel (title, organizer, attendee count; org-controllable); Avatars (Mesh), Add Avatar [K]
- Lobby waiting screen ("Someone in the meeting should let you in soon"); can chat in lobby for some configurations [K]

### 3.3 Audio and video
- Mic mute/unmute; push-to-talk (hold Space to temporarily unmute); camera on/off; device switch in-meeting; Noise suppression (AI) levels; Voice isolation / speech enhancement; Music mode / high-fidelity audio; Spatial audio; Studio-quality; Original sound for musicians; Speaker coach (private AI pace/filler feedback, Premium) [K]
- Video: HD; video filters/effects (blur, background images, Copilot-generated backgrounds, branded backgrounds), auto framing, mirror, pop-out video, picture-in-picture/compact meeting window [K]
- Meeting AI: Intelligent speakers / Teams Rooms voice recognition [K]
- Audio via PSTN: dial-in number + conference ID, "Call me at" (Teams dials your phone), Audio conferencing license needed for dial-in; phone user controls via *6 mute, *3 raise hand, etc. [K]
- Transfer to another device: "Join on this device"/"Switch to this device" prompts when joined elsewhere; "Add room"/"Add this device to your meeting" companion mode (second device audio muted, for content/chat) [K]
- Companion mode: join via second device for chat/content/reactions with mic+speakers disabled [K]

### 3.4 Meeting toolbar (top bar, new Teams)
Chat, People, Raise, React, View, Notes, Whiteboard (via Share), Apps, More (...), Camera, Mic, Share, Leave (Leave / End meeting for everyone). Plus the call-health indicator [K]

- More (...) menu: Meeting info, Meeting options (organizer), Record and transcribe (Start recording, Start transcription), Live captions (Turn on live captions, Language settings, Caption settings), Language and speech, Spoken language, Closed captions with translation (Premium), Live translation of captions, Device settings (Audio, camera, speaker), Video effects and settings, Noise suppression, Apps, Report a problem, Hide meeting controls, Full screen, Pop out chat/People, Dial pad, Together mode (removed), Turn off incoming video, Turn off incoming audio, Meeting notes, Attendance, Presenter modes, Live reactions on/off, Meeting recap [K]

### 3.5 Screen sharing and content (Share tray)
- Share: Screen (entire), Window (specific app), Tab via browser (web only), Whiteboard (Microsoft Whiteboard / Freehand), PowerPoint Live (choose from recent / OneDrive / Browse), Excel Live, Microsoft Loop?, Copilot Pages?, Video/Camera-as-content, Apps (Live Share, Polls?), Include computer sound toggle, Optimize for video clip toggle, Presenter modes: Content only, Standout, Side-by-side, Reporter [K, V partial: ACS lists Content-only/Standout/Side-by-side/Reporter]
- Give control / Take control / Request control (remote control of shared screen) [V partial]
- Annotation on shared screen (collaborative annotations) [V partial]
- Stop sharing; Pause; Hide presenter video; "Sharing in progress" frame border [K]
- Live reactions toggle during share; Presenter toolbar minimized; "Focus on content" [V]
- Follow-presenter in PowerPoint Live; private browse of slides; accessibility alt text; slide navigation; laser pointer; speaker notes [K]
- Sensitive content detection [V]
- Block screen capture protection [V]

### 3.6 Meeting chat (in-meeting)
- Chat side panel (or full tab in meeting chat); messages persist after meeting; reply, @mention, reactions, files, polls from Forms/Polls app, Loop components, GIF/emoji/stickers, formatting, urgent/important delivery, "Meeting chat history shared with new joiners" [K]
- Chat restrictions per meeting options; chat in lobby; chat before/after [V]
- Guests and anonymous: chat only during meeting (ACS limitation) [V]

### 3.7 Reactions, raise hand
- Raise hand (queue ordering in People panel; organizer can lower hands) [V ACS: raise hand supported]
- Live reactions: Like, Heart, Applause, Laugh, Surprised (new set includes Hand wave, Celebrate, Sad, Fire), skin tone, "Turn off live reactions" [K]
- Reactions toggled off for the meeting via option [V]

### 3.8 Participants (People panel)
- Roster sections: In this meeting (Presenters, Attendees), Invited, Lobby/Waiting, Raised hands; search; participants count [K]
- Organizer/presenter actions per participant: Mute, Mute all, Mute and disable mic (attendees), Disable camera (attendees), Spotlight, Pin, Add to view, Make a presenter, Make an attendee, Make a co-organizer, Remove from meeting, Admit/Deny from lobby, Admit all, Lock meeting, Rename (if allowed), Allow/disallow mic and camera for all attendees, Allow/disallow reactions/chat from meeting, Hide attendee names, Lock lobby ("Lock the meeting") [K, V partial]
- Invite: Add people (search), Copy join info, Share invite via email, Call someone to join (by phone), Add room [K]
- Spotlight (for everyone) and Pin (for me), Multi-pin, Exit spotlight, Hide for me, Manage what attendees see [V]
- Presence/role badges, Guests, External, Bot labels, Audio source icon (computer/phone/room) [K]
- Raise hand queue (ordered) [K]
- Attendees see up to 4 speakers in large meetings (>100) [V]

### 3.9 View modes
- Dynamic (default) [V]
- Gallery (max gallery size selector) [V]
- Large gallery (needs 10+ cameras on; up to 49 tiles, navigation >49) [V]
- Speaker view (active speaker, filmstrip) [K]
- Focus on content (maximizes shared content) [V]
- Gallery at top [V]
- Swap content with gallery (move shared content to side/top) [V]
- Together mode (scenes: auditorium, coffee shop, etc.): RETIRED, removed from Teams 2026-06 (Message Center MC1296478); no policy to keep it; Gallery becomes the primary layout [V]
- Hide non-video participants; Pop-out meeting window, compact meeting view/PiP; Full screen; Front row (Teams Rooms) [K]
- Speaker view with spotlight, Focus mode in town halls [K]

### 3.10 Breakout rooms
- Organizer/co-organizer: Create rooms (up to 50), assign automatically or manually, rename rooms, set timer, open/close rooms, move participants between rooms, reassign, re-create, send announcement to all rooms, join a room as organizer, Allow participants to choose room, Allow participants to return to main meeting, Pre-assign rooms before the meeting (scheduled meeting only), Presenter assist (co-organizers/presenters can join and manage), Leave breakout room, Recording of rooms (separate), Shared notes per room [K]
- Participants: join assigned room, rename, ask for help, chat in room [K]
- Not available in channel meetings; not in ACS [V ACS says unsupported]

### 3.11 Polls, Q&A, apps
- Polls (Microsoft Forms/Polls app): create live poll during meeting or pre-create in meeting chat, multiple choice, quiz mode, word cloud, ranking, rating, results live/after, launch/close, share results [K]
- Q&A: ask question, anonymous option, reply, upvote, moderation (publish/dismiss), mark answered, filter (all / mine / pending), organizers can turn on moderation [V option; K details]
- Apps in meetings: Apps tray, Add apps to meeting (tabs, side panel, stage via "share to stage"), Live Share apps, Whiteboard, Notes, Lists, Tasks, Forms, Third-party meeting extensions, Meeting-stage apps, Copilot Pages, Loop components, "Meeting extensions" and in-meeting dialog [K]
- Whiteboard (Microsoft Whiteboard in Teams; also meeting "Whiteboard" tab; accessible after meeting) [K]
- Meeting notes (Loop-based; share notes; agenda; action items; "Notes" button; Copilot-generated notes) [K]
- Microsoft Loop / Copilot Pages [K]
- Language interpretation: interpreter role, choose "Hear in" language channel, mute original audio [K, V option]

### 3.12 Captions, transcription, recording, translation
- Live captions (spoken language selection; caption settings: position, size, color, show speaker name; "Translate to" language, Premium/Copilot translated captions in 40+ languages; Live captions in channel meetings) [K, V partial]
- Live transcription (Start transcription; transcript pane with speaker attribution; download; auto-start; consent notice) [K]
- Recording (Start recording; consent notice and explicit consent in some tenants; recording banner; recording stored in OneDrive (non-channel) or SharePoint (channel); retention 120 days default for new, auto-expiration; Recording permissions; Download; Share link; Stop; Pause; "Recording and transcription started" notices for all) [K]
- Recording view layouts and Speaker coach, Chapters, AI-generated chapters, Topic search, Captions in Stream player [K]
- Live translation / Interpreter agent: Copilot "Interpreter" speech-to-speech in meetings (preview/GA 2025-26) [K; verify current status]
- Compliance recording indicator [V]
- Data residency/store recordings outside country setting [V]

### 3.13 Copilot and AI in meetings
- Copilot in meetings (Copilot icon; "Summarize so far", ask questions about what was said, catch up on missed parts, "What questions are unanswered", action items) [K]
- Intelligent recap (AI notes, chapters, speaker timeline, name mentions, follow-up tasks, "Ask Copilot" in Recap tab; available after meeting for Copilot or Premium users) [K]
- Facilitator agent (live notes, agenda tracking, timekeeper) [V option: Facilitator]
- Meeting AI insights via Graph (summary, action items, meeting notes), user must be Copilot licensed; available up to ~4 h after meeting; no channel meetings [V]
- Speaker coach [K]
- Voice isolation / AI noise suppression [K]
- Smart meeting recap with AI notes, tasks assignment to Planner/To Do [K]

### 3.14 Phone, PSTN, dial-in
- Dial-in numbers (toll/toll-free) and Conference ID (Audio Conferencing license required) [V]
- Call me / Call me back (Teams calls your phone) [K]
- Add a phone number to invite (dial-out) [V ACS: add phone number supported]
- Phone controls via DTMF (*6, *5 raise hand, *3 lobby?) [K]
- Entry/exit announcements [V]
- Join by VTC/SIP (CVI: videoTeleconferenceId) [V]
- Transfer call between devices (PC <-> mobile <-> desk phone) [K]

### 3.15 After the meeting
- Meeting recap tab in meeting chat (Recap: recording, transcript, AI notes, attendance, shared files, whiteboard, chapters) [K]
- Meeting chat remains accessible (history sharing option) [V]
- Attendance report (names, join/leave timestamps, duration, role, email; downloadable CSV; organizer only; per-session reports) [V Graph: attendanceReports]
- Recordings stored in OneDrive/SharePoint; transcripts as .vtt/.docx in same location (organizer's OneDrive, or stored in Exchange for non-recorded transcripts) [K]
- Meeting expires 60 days after start/end per Graph [V]
- Whiteboard and Notes accessible later [K]
- Copilot AI notes and action items (up to ~4 hours after) [V]

### 3.16 Webinars and town halls (in-event)
- Attendee: join via link, register (webinars), view-only with Q&A, reactions, captions, live translation (Premium); Presenters: Green room, content share, RTMP-in, producer role, Manage what attendees see, Event group, eCDN, Spotlight/Focus mode [K, V partial]
- Town hall: up to 10,000 (20,000 with Premium), Producer mode, Q&A moderation, RTMP-out, Event recap, Attendee reports, Customizable themes, Event Group, Registration (webinar) [K; verify numbers]
- Graph: virtualEvents (townhalls, webinars, sessions, registrations), delegated VirtualEvent.Read / ReadWrite, admin consent required; many beta [V]

### 3.17 Accessibility, Misc
- Keyboard shortcuts (Ctrl+Shift+M mute, Ctrl+Shift+O camera, Ctrl+Shift+K raise hand, Ctrl+Shift+E share, Ctrl+Shift+Y accept audio, Ctrl+Shift+H hang up, Ctrl+Shift+P/L etc.) [K]
- Meeting quality indicator and call health, network diagnostics, "Report a problem" [K]
- Cloud-based accessibility captions in sign language / Sign language view (spotlight interpreter) [K]
- Meeting themes and branding (Premium) [V]
- Mesh avatars and immersive spaces (Premium/Mesh; low priority) [K]
- Teams Premium: watermark, E2EE, custom branding, advanced town hall, intelligent recap, personalized backgrounds [V]

---------------------------------------------------------------------------------------------------

## 4. API and protocol notes

### 4.1 Which Graph scopes the verified token already has (from the task brief) and what they unlock
- Calendars.ReadWrite: /me/events CRUD, /me/calendarView, /me/calendars, calendarGroups, accept/decline/tentativelyAccept (incl. proposedNewTime), cancel, forward, snooze/dismissReminder, event instances, attachments, delta (/me/calendarView/delta), categories via outlookCategory (needs MailboxSettings.ReadWrite for /me/outlook/masterCategories [?]) [V event resource]
- Calendars.ReadWrite.Shared: shared/delegate calendars; covers findMeetingTimes (least privilege Calendars.Read.Shared) and getSchedule [V findMeetingTimes page]
- Place.Read.All: /places/microsoft.graph.room, /places/microsoft.graph.roomList (rooms, room lists, capacity, building, floor) [V; delegated listed Admin consent Yes but already held]
- Mail.ReadWrite / Mail.Send: meeting invitations arrive as eventMessage, eventMessageRequest/Response in Inbox; can read them and send invitations/forward [K]
- Files.ReadWrite.All / Sites.ReadWrite.All: access recordings and transcripts as files in OneDrive/SharePoint (path /Recordings in organizer's OneDrive, SharePoint team site "Recordings" folder for channel meetings) and meeting Whiteboard/Notes/Loop files; can also create/share the Loop notes file [K]
- ChatMessage.Send, ChannelMessage.Send, Team.ReadBasic.All, TeamMember.ReadWrite.All: channel meetings, post to meeting chat via Graph, enumerate team/channel for channel meeting picker [V descriptions]
- People.Read, User.ReadBasic.All: attendee picker / people search [K]
- Tasks.ReadWrite: Planner/To Do tasks (action items follow-up) [V]

### 4.2 Scopes NOT in the verified list, and what they gate
(Admin consent column is from Microsoft permissions reference [V].)
- OnlineMeetings.ReadWrite (delegated, admin consent NOT required): POST /me/onlineMeetings (create), PATCH onlineMeeting (change options: lobby, presenters, mic/camera, chat mode, recordAutomatically, allowTranscription, watermark etc.), createOrGet, GET by id, filter by joinWebUrl or joinMeetingId; also GET /me/onlineMeetings/{id}/attendanceReports
- OnlineMeetings.Read (delegated, no admin consent): read meeting details only
- OnlineMeetingArtifact.Read.All (delegated, no admin consent): read online meeting artifacts incl. attendance reports [V]
- OnlineMeetingRecording.Read.All (delegated, admin consent REQUIRED): GET /me/onlineMeetings/{id}/recordings and content [V]
- OnlineMeetingTranscript.Read.All (delegated, admin consent REQUIRED): GET /me/onlineMeetings/{id}/transcripts and /content (VTT), getAllTranscripts [V]
- OnlineMeetingAiInsight.Read.All (delegated, admin consent REQUIRED): GET /copilot/users/{id}/onlineMeetings/{id}/aiInsights (summary, action items, notes), needs Copilot license [V]
- Presence.Read / Presence.ReadWrite (delegated, no admin consent) and Presence.Read.All (delegated, no admin consent): GET /me/presence, POST /communications/getPresencesByUserId, setPresence/setUserPreferredPresence [V]
- VirtualEvent.Read / VirtualEvent.ReadWrite (delegated, admin consent required): webinars/town halls [V]
- Calls.* (JoinGroupCall, AccessMedia, Initiate): APPLICATION-only, for app-hosted-media bots; not usable as a signed-in client [V]
- CallRecords.Read.All: application only [V]

Important: the first-party Teams web client (1fec8e78-...) has a fixed set of pre-authorized scopes. Absence from the verified list does NOT prove it cannot get them; but it also cannot be extended by kelma (no admin consent flow for first-party). Run a probe: request a Graph token with `scope=https://graph.microsoft.com/OnlineMeetings.ReadWrite` etc. and see whether AAD returns it (invalid_grant / AADSTS65001 means not pre-authorized). [?]

### 4.3 Calendar and scheduling endpoints (Graph v1.0)
- GET /me/calendarView?startDateTime=&endDateTime= (expands recurrences; supports `Prefer: outlook.timezone="..."`, `Prefer: outlook.body-content-type="text"`); delegated Calendars.ReadBasic (least) / Calendars.Read / Calendars.ReadWrite [V]
- GET /me/events, POST /me/events, PATCH/DELETE /me/events/{id}; POST /me/calendars/{id}/events [V]
- Create a Teams meeting through the calendar API (no OnlineMeetings scope required): POST /me/events with `"isOnlineMeeting": true, "onlineMeetingProvider": "teamsForBusiness"`. Result includes onlineMeeting.joinUrl, conferenceId, tollNumber, tollFreeNumbers, quickDial. Cannot be undone (cannot go back to offline). Attendees get full Outlook-style invite with join blob. [V]
  - Does NOT expose lobby/presenter/recording options via the API: options only editable through the meeting options web page or via onlineMeeting PATCH (needs OnlineMeetings.ReadWrite) [V: choose-online-meeting-api]
- Event fields to expose in UI: subject, body, start/end (DateTimeTimeZone), isAllDay, recurrence (patternedRecurrence), attendees[type required|optional|resource], location/locations (Graph location with locationType, uniqueId from Place), categories, showAs, sensitivity, importance, reminderMinutesBeforeStart, isReminderOn, responseRequested, allowNewTimeProposals, hideAttendees, isOnlineMeeting, onlineMeeting, transactionId (idempotent create), seriesMasterId, type, iCalUId, webLink [V]
- RSVP: POST /me/events/{id}/accept | tentativelyAccept | decline with `{comment, sendResponse, proposedNewTime}` [V]
- Organizer: POST /me/events/{id}/cancel `{comment}`, POST /forward, /snoozeReminder, /dismissReminder, GET /instances [V]
- Delta sync: GET /me/calendarView/delta; Change notifications via subscriptions /me/events (needs a webhook/websocket endpoint: not viable for desktop; poll delta or use Teams trouter/Exchange notifications [?]) [V event supports delta + notifications]
- Find times: POST /me/findMeetingTimes {attendees, timeConstraint(activityDomain work|personal|unrestricted, timeSlots), meetingDuration (ISO8601), locationConstraint, isOrganizerOptional, minimumAttendeePercentage, returnSuggestionReasons, maxCandidates}; delegated Calendars.Read.Shared / ReadWrite.Shared [V]
- Free/busy grid for Scheduling Assistant: POST /me/calendar/getSchedule {schedules[], startTime, endTime, availabilityViewInterval} (Calendars.Read.Shared) [K]
- Calendars: GET /me/calendars, /me/calendarGroups, POST create calendar, /me/calendars/{id}/calendarPermissions (share) [K]
- Working hours / time zone: GET /me/mailboxSettings (workingHours, timeZone, automaticRepliesSetting) needs MailboxSettings.Read(Write) (delegated no admin consent [V]); not in verified list but Mail.ReadWrite alone does not cover it [?]
- Rooms: GET /places/microsoft.graph.room, /places/microsoft.graph.roomList, then `GET /places/{roomList}/microsoft.graph.roomList/rooms` (Place.Read.All) [K]
- Categories: GET/POST /me/outlook/masterCategories (MailboxSettings.ReadWrite) [K]
- People picker: GET /me/people?$search= (People.Read), /users?$search= (User.ReadBasic.All) [K]
- Meeting attachments: /events/{id}/attachments (Calendars.ReadWrite) [V]

### 4.4 Online meeting endpoints (Graph v1.0)
- POST /me/onlineMeetings {startDateTime, endDateTime, subject, participants.attendees[], lobbyBypassSettings{scope,isDialInBypassEnabled}, allowedPresenters, allowAttendeeToEnableMic/Camera, allowMeetingChat, allowTeamworkReactions, recordAutomatically, ...} (OnlineMeetings.ReadWrite) [V]
- POST /me/onlineMeetings/createOrGet {externalId, ...}: idempotent for 3rd party calendars [V]
- GET /me/onlineMeetings/{id}; GET /me/onlineMeetings?$filter=JoinWebUrl eq '{url}' (find meeting from a calendar event's joinUrl; requires OnlineMeetings.Read) [K]
- PATCH /me/onlineMeetings/{id} to update options (OnlineMeetings.ReadWrite); joinMeetingIdSettings immutable [V]
- GET /me/onlineMeetings/{id}/attendanceReports and /{reportId}/attendanceRecords (OnlineMeetingArtifact.Read.All; docs page 404 under fetch, endpoints known from product) [K]
- GET /me/onlineMeetings/{id}/recordings and /recordings/{id}/content (OnlineMeetingRecording.Read.All, admin consent) [V]
- GET /me/onlineMeetings/{id}/transcripts, /transcripts/{id}/content?$format=text/vtt (OnlineMeetingTranscript.Read.All, admin consent) [V]
  - Limits: not for meetings created via POST /onlineMeetings that have no calendar event; not for live events; tenant admin may disable transcripts API (403 GraphAccessToTranscriptsDisabled); only for unexpired meetings [V]
- GET /copilot/users/{id}/onlineMeetings/{id}/aiInsights (OnlineMeetingAiInsight.Read.All; Copilot license; up to 4h delay) [V]
- Meeting options web URL: `meetingOptionsWebUrl` property is only returned by the onlineMeeting resource; for calendar-only access, a URL of the shape https://teams.microsoft.com/meetingOptions/?organizerId=<oid>&tenantId=<tid>&threadId=<19:meeting_...@thread.v2>&messageId=0&language=<lang> can be built from the join URL context [K]. Open it in an embedded webview as a fallback for the "Meeting options" button.
- Webinars/town halls: /solutions/virtualEvents/townhalls, /webinars (+ publish/cancel, sessions, presenters, registrations); delegated VirtualEvent.ReadWrite (admin consent) [V]
- Teams live events API stopped returning data 2024-09-30 [V]

### 4.5 Meeting chat
- A Teams meeting's chat thread id is `19:meeting_<base64>@thread.v2` (private) or the channel thread (channel meetings). It is embedded in the join URL path (`/l/meetup-join/19%3ameeting_...%40thread.v2/0?context={"Tid":...,"Oid":...}`) and in onlineMeeting.chatInfo.threadId [V chatInfo; K URL shape]. kelma already speaks chatsvc, so "Open meeting chat", post-meeting chat, recap-card messages, and meeting chat history can reuse the chat stack directly with no Graph scope.
- Chat thread members changes, "meeting started/ended" system messages, recording/transcript events appear as thread events (RecordingStarted/Ended, Call, "Meeting ended" with duration) in chatsvc [K]; use to drive the "Join" banner and "meeting started" indicator.

### 4.6 How joining a Teams meeting works technically (what is publicly knowable)
Public Microsoft documentation does NOT describe the first-party Teams client protocol; the following is general knowledge, [K]/[?], and must be confirmed by capturing real traffic (HAR from the web client while joining). Treat as hypotheses.
- Identity and tokens: join uses a Skype/Teams token (the "skypetoken" kelma already uses for chatsvc) plus an AAD access token for a Teams-calling audience; meeting is addressed by thread id + organizer oid + tenant id parsed from the join URL (or by meeting ID + passcode resolved server-side).
- Signaling plane: HTTP calls to the conversation/call controller (region-specific `*.conv.skype.com` / `*.conv.teams.microsoft.com` style hosts; "conversation controller" and "NGC / next-gen calling" naming from the user's own notes) create a call leg, then receive roster/events over trouter (WebSocket push) as call-notification messages (call state, participants, roster updates, lobby, hand raise, captions, recording state, spotlight, etc.). Control actions (mute, spotlight, admit, end) are further HTTP POSTs to the controller with the call id [?].
- Media plane: ICE (STUN/TURN via Teams relays, "transport relay"), DTLS-SRTP, RTP with Opus/SILK/G.722 audio and H.264/VP8 video; screen sharing as a separate video-based screen-sharing (VBSS) stream; SDP-like offers carry proprietary attributes and RTCP feedback/extensions; data channels for some features (captions, reactions, hand raise ride on signaling, not media) [?]. Browser Teams uses the browser's WebRTC stack with a Teams-specific negotiation; native clients use the Skype "SkyLib" media stack, not exposed publicly.
- Roster/state: roster is a stateful stream (participants with endpoint ids, roles, mute state, audio/video modalities, raised hand, etc.) delivered via trouter/long-poll and diffed client-side.
- Consequence: a from-scratch native implementation must reproduce undocumented signaling AND media negotiation quirks; any service-side change breaks it; no documented, supported route exists for first-party identity.
- Supported (documented) alternatives are:
  1. Graph Cloud Communications calls API (Calls.JoinGroupCall.All etc.): APPLICATION permissions only; the bot joins as an app/bot, not as the signed-in user, requires admin consent + application access policy; app-hosted media SDK is Windows-only (.NET) [V from bots docs, search result].
  2. Azure Communication Services (ACS) Calling SDK interop (below).

### 4.7 Azure Communication Services (ACS) Teams interop
Three identities, all documented [V]:
1. ACS identity (Teams external user / "guest"): any developer-owned ACS identity can join a Teams meeting by URL or meeting ID + passcode if the meeting allows it (policy "Let anonymous people join a meeting"; lobby rules apply). No Teams license needed, but the user appears as an EXTERNAL participant.
2. Teams user / Microsoft 365 identity ("Custom Teams endpoint"): the real signed-in Entra user participates as themselves (lobby bypass for same-org, role honored). Requires: an ACS resource, an Entra app registration that kelma OWNS with delegated permissions `https://auth.msft.communication.azure.com/Teams.ManageCalls` and `Teams.ManageChats` (admin consent per tenant), and an Entra token for that app exchanged for an ACS token (Identity SDK `getTokenForTeamsUser`, TTL max 24 h). This CANNOT use the Teams web client id 1fec8e78-...; it needs the product's own app registration, so every customer tenant must consent. [V]
3. Teams phone (PSTN) users via ACS: out of scope.

Platform support [V]:
- Calling SDK available for: JavaScript (web), Windows (.NET), Android (Java), iOS (Obj-C). There is NO native Linux SDK and no C/C++ desktop SDK.
- JS SDK on Linux: supported only for Chrome on Ubuntu/Linux in a regular browser; Electron on Linux: NOT supported; other embedded webviews (CEF, WebView2, GeckoView) not tested/supported (may or may not work). A WebKitGTK/Tauri webview on Linux is not on the support list.

Capability matrix (as ACS Teams-user / external user joining a meeting) [V]:
- Join by URL, by meeting ID + passcode (work meetings), channel meeting (audio/video only for external), join lobby/admitted, leave, end meeting for everyone, honors roles and lobby rules, display name changes
- Mute/unmute, camera on/off, switch devices (web), dominant speaker, participant states, mute indicators, call states, toll/toll-free numbers, hold (local)
- Screen share out (entire screen, app, browser tab; content-only mode; no standout/side-by-side/reporter; no remote control; system audio only on web); receive screen share, PowerPoint Live stream receive (external users: yes; Teams users: no)
- Roster: list, add Teams user, add phone number, remove participant, admit from lobby, be promoted; cannot promote/demote others, cannot enable/disable attendee mic/camera for others, cannot manage breakout rooms
- Video: render remote, spotlight add/remove, together mode (web, external users; retired in Teams), blur and custom background (web), no large gallery
- Recording/transcription: can SEE recording/transcription status and give explicit consent; cannot start/stop recording or transcription; can receive and manage closed captions (external) / receive captions (Teams user); Teams-user capability sheet says no live captions mgmt
- Engagement: raise/lower hand (yes); reactions (external: yes; Teams user: no)
- Chat: ACS Chat SDK joins meeting chat for external users (text, inline images receive, files receive, emoji, typing; no reply/reaction); chat history unavailable after the call; Teams-user can use Graph/Teams chat
- Integrations: no polls, Q&A, Notes, Whiteboard, apps, Speaker Coach; attendance report entry works; no language interpretation, no CART
- Not supported to join at all: end-to-end encrypted meetings, webinars, town halls, live events, meetings scheduled in Teams for home (teams.live.com)
- Limits: 350 participants; max call 30 h; incoming video: 16 (25 in newer web) tiles
- Client SDK is closed-source JavaScript running inside a browser engine (WebRTC); no Rust bindings.

### 4.8 Open-source landscape (none implements Teams calling or meeting join)
- teams-for-linux (IsmaelMartinez, Electron): wraps Teams web app; calls/meetings work because the real web client runs; most popular unofficial Linux client [V search]
- fossteams/teams-api, teams-cli, teams-token (Go): reverse-engineered chat/Teams-list endpoints only (JWT acquisition and "GetConversations"); no calling, no calendar [V fetch]
- Squads (IanTerzo, Rust): alternative Teams client for org accounts; README lists no calendar or calling features [V fetch]
- Teams-Auto-Joiner (Python, kescherCode): browser/UI automation to click Join, not a protocol implementation [V search]
- Microsoft's own: Teams removed its native Linux client in 2022-2023 in favor of the PWA/web client [V search]
- No GitHub project found that implements Teams meeting-join signaling/media natively. Conclusion: kelma would be first; high cost.

---------------------------------------------------------------------------------------------------

## 5. Feasibility and recommended architecture

### 5.1 Tiering by feasibility (native vs webview vs ACS vs skip)

Tier A: Fully native with Graph, scopes already held (do first)
- All calendar views, event CRUD, recurrence editor, attendee picker (People.Read/User.ReadBasic.All), RSVP with propose new time, cancel, forward, reminders, categories (needs MailboxSettings, probe), shared/delegate calendars, rooms picker (Place.Read.All), findMeetingTimes and getSchedule (Scheduling Assistant), time zone handling, working-hours overlays (probe MailboxSettings.Read), Meet-now via calendar event with Teams meeting created through isOnlineMeeting.
- Create a Teams meeting from an event (isOnlineMeeting + teamsForBusiness), show join URL, dial-in info, copy link, open meeting chat via chatsvc (thread id from join URL), post-meeting chat, join banner driven by chat thread events and calendar.
- Notifications: poll calendarView/delta or trouter; local reminder timers (no webhook required).

Tier B: Native, needs one extra scope (probe the first-party client's pre-authorization)
- OnlineMeetings.ReadWrite (no admin consent): meeting options read/write natively (lobby, presenters, mic/camera, chat mode, reactions, record automatically, breakout rooms, etc.), createOrGet, attendanceReports (with OnlineMeetingArtifact.Read.All, no admin consent), join with ID lookup.
- Presence.Read(.All)/ReadWrite: probably unnecessary because Teams presence service is already usable with the chat token [?].
- If not pre-authorized: fall back to opening the organizer-only meeting options web page in an embedded webview (works with the user's session), or an in-app webview for options only.

Tier C: Needs admin consent (likely not available to first-party client)
- Recordings/transcripts via Graph (OnlineMeetingRecording.Read.All, OnlineMeetingTranscript.Read.All), AI insights, virtual events (webinars/town halls). Workarounds: read recordings from OneDrive/SharePoint files with Files.ReadWrite.All (already held), parse transcripts from files, or read recap data through Teams internal APIs (same as web recap tab) after HAR capture [?].

Tier D: In-meeting (audio/video/screen-share) cannot be native in Rust without huge reverse engineering. Options:
1. Embedded webview of the Teams web app meeting join (teams.microsoft.com/l/meetup-join/... or /v2/?meetingjoin=true) in a separate window using the user's session cookies: full feature parity (all of section 3), Microsoft supports the web client for Linux (Chrome/Edge); wry/Tauri WebKitGTK is unlikely to be supported (Teams web requires Chromium-based browsers; WebKitGTK lacks needed WebRTC/codec/features; may show "unsupported browser"). Use a Chromium-based engine: CEF (via cef-rs/`tauri-runtime-cef`), Electron-like sidecar, or hand off to the system browser (Chrome/Edge PWA) with the msteams/https join link. Hand-off to Edge/Chrome PWA is simplest and lowest risk. [K, ?]
2. ACS Calling SDK (JS) in a Chromium webview: documented, supported route for Chrome on Linux; needs a product-owned Entra app + ACS resource + per-tenant admin consent for Teams.ManageCalls/ManageChats (Teams-user identity), or external-guest identity without consent (but joins as "external"). Feature gaps vs real Teams are large (no polls, Q&A, reactions for Teams users, breakout rooms, notes, whiteboard, together mode, large gallery, recording control, attendee mic control, E2EE). Wrong choice if the goal is parity; right choice if goal is custom branded minimal call UI.
3. Reverse-engineer the Teams calling protocol natively in Rust (webrtc-rs/libwebrtc + custom signaling): theoretically possible (browser web client does it), but multi-year, brittle, ToS/legal risk, no public references, would still miss most advanced features. Not recommended.

### 5.2 Recommended architecture
1. Build Calendar as a first-class native module on Graph (Tier A) with a local cache (sqlite) fed by calendarView/delta; reuse the existing token broker; add `Prefer: outlook.timezone` and `outlook.body-content-type`; use `transactionId` for idempotent create; store immutable IDs (`Prefer: IdType="ImmutableId"`).
2. Treat "meeting join" as a pluggable `MeetingLauncher`: default = open the join URL in the user's browser/installed Teams PWA (zero risk, full parity); optional = embedded Chromium window (CEF) authenticated with the same session; leave ACS as a later, opt-in "lite call" mode only if there is a product reason.
3. Keep meeting chat, recap messages, meeting-started indicators, and pre-/post-meeting experience native by reusing the chat stack (chatsvc thread from join URL). This gives a convincing "meetings" experience without the media plane.
4. Probe scopes early (one afternoon): attempt token mint for OnlineMeetings.ReadWrite, OnlineMeetings.Read, OnlineMeetingArtifact.Read.All, MailboxSettings.ReadWrite, Presence.Read.All, OnlineMeetingRecording.Read.All, OnlineMeetingTranscript.Read.All, OnlineMeetingAiInsight.Read.All, VirtualEvent.ReadWrite. Record which succeed; that decides Tier B/C scope.
5. Capture the Teams web client's own calendar and meeting-options API calls (HAR via claude-in-chrome network capture) to find first-party endpoints (new Teams calendar uses Outlook services; meeting options and recap use Teams middle-tier `teams.microsoft.com/api/mt/...`) that bypass Graph scope limits; mark them "internal, may break".
6. Schedule: Phase 1 = views, form, RSVP, rooms, scheduling assistant, Meet now/join with link; Phase 2 = meeting options, attendance, recordings/transcripts via OneDrive, recap tab (native chat-based); Phase 3 = optional embedded join window; do NOT plan native audio/video in the first year.

### 5.3 Risks
- Pre-authorization: first-party client scopes cannot be expanded; OnlineMeeting* may be unavailable [?].
- Using the Teams web client id from a third-party app may violate Microsoft terms and can be blocked by Conditional Access (device compliance, managed-app requirements) or token binding changes; ACS and Graph with own app registration is the supported path but needs tenant admin consent.
- Internal APIs are undocumented and change without notice; maintenance burden.
- Linux webview limits: WebKitGTK not a supported Teams/ACS browser; Chromium (CEF) heavy (~150+ MB), needs screen-capture portal (PipeWire/xdg-desktop-portal) for Wayland sharing.
- Licensing gates: many options (recording/transcription control, watermark, E2EE, town halls, Copilot recap) need Teams Premium or Copilot; UI must read tenant capability and hide/disable.
- Online meeting resource expires 60 days after start/end; recordings/transcripts via Graph only for unexpired calendar-linked meetings; admins can disable the transcripts API.
- Together mode removed (2026-06): do not build it. Teams live events replaced by town halls.
- Event messages: cancelling vs deleting an event; organizers vs attendees permission differences; must handle series master vs occurrence vs exception.
- ACS Teams-user identity: no capability to change meeting options, no reactions, no polls/Q&A; E2EE meetings blocked.

---------------------------------------------------------------------------------------------------

## 6. Sources

Microsoft Learn (fetched this session):
- Calendar / online meeting choice: https://learn.microsoft.com/en-us/graph/choose-online-meeting-api
- onlineMeeting resource: https://learn.microsoft.com/en-us/graph/api/resources/onlinemeeting?view=graph-rest-1.0
- Create onlineMeeting: https://learn.microsoft.com/en-us/graph/api/application-post-onlinemeetings?view=graph-rest-1.0
- List transcripts: https://learn.microsoft.com/en-us/graph/api/onlinemeeting-list-transcripts?view=graph-rest-1.0
- List recordings: https://learn.microsoft.com/en-us/graph/api/onlinemeeting-list-recordings?view=graph-rest-1.0
- event resource: https://learn.microsoft.com/en-us/graph/api/resources/event?view=graph-rest-1.0
- List calendarView: https://learn.microsoft.com/en-us/graph/api/user-list-calendarview?view=graph-rest-1.0
- findMeetingTimes: https://learn.microsoft.com/en-us/graph/api/user-findmeetingtimes?view=graph-rest-1.0
- Graph permissions reference (OnlineMeeting*, Presence*, Place*, Calendars*, Calls*, VirtualEvent*): https://learn.microsoft.com/en-us/graph/permissions-reference
- ACS Teams-user meeting capabilities: https://learn.microsoft.com/en-us/azure/communication-services/concepts/interop/teams-user/meeting-capabilities
- ACS Teams external-user capabilities: https://learn.microsoft.com/en-us/azure/communication-services/concepts/interop/guest/capabilities
- ACS Calling SDK overview and platform/browser support: https://learn.microsoft.com/en-us/azure/communication-services/concepts/voice-video-calling/calling-sdk-features
- ACS Microsoft 365 user authentication (Teams.ManageCalls/ManageChats): https://learn.microsoft.com/en-us/azure/communication-services/concepts/interop/custom-teams-endpoint-authentication-overview
- ACS Teams interop overview: https://learn.microsoft.com/en-us/azure/communication-services/concepts/teams-interop
- Meeting AI insights (callAiInsight): https://learn.microsoft.com/en-us/microsoft-365-copilot/extensibility/api/ai-services/meeting-insights/callaiinsight-get
- Teams meeting insights via Graph: https://learn.microsoft.com/en-us/microsoftteams/platform/graph-api/meeting-transcripts/meeting-insights
- Virtual events (town hall / webinar) use cases: https://learn.microsoft.com/graph/cloud-communications-virtual-events-townhall-usecases , https://learn.microsoft.com/graph/cloud-communications-virtual-events-webinar-usecases
- Calls and meetings bots overview: https://learn.microsoft.com/microsoftteams/platform/bots/calls-and-meetings/calls-meetings-bots-overview

Microsoft support:
- Meeting options in Teams: https://support.microsoft.com/topic/53261366-dbd5-45f9-aae9-a70e6354f88e
- Schedule a meeting in Teams: https://support.microsoft.com/en-us/office/schedule-a-meeting-in-microsoft-teams-943507a9-8583-4c58-b5d2-8ec8265e04e5
- Customize your meeting view: https://support.microsoft.com/office/95aaeaf8-0f22-46cf-a6f9-34ca9b04a1b2
- Together mode retirement (Message Center MC1296478): https://mc.merill.net/message/MC1296478 ; https://office365itpros.com/2026/05/04/together-mode-retirement/

GitHub / community:
- teams-for-linux: https://github.com/IsmaelMartinez/teams-for-linux
- fossteams/teams-api: https://github.com/fossteams/teams-api
- Squads (Rust): https://github.com/IanTerzo/Squads
- Teams-Auto-Joiner: https://git.kescher.at/kescher/Teams-Auto-Joiner

Third-party calendar guides (lower confidence, used only for calendar view/filter hints):
- https://www.virtosoftware.com/events/microsoft-teams-calendar-guide/
- https://www.k-state.edu/news/it/articles/2025/06/start-using-the-new-calendar-in-microsoft-teams.html

Not verified this session (marked [K]/[?] above): exact new-Teams button labels, agenda view, Teams internal calendar/meeting endpoints, the join-signaling details in 4.6, town hall attendee caps, Copilot/Interpreter agent GA status, MailboxSettings coverage by the Teams client token, getSchedule and attendanceReports page details (the attendanceReports Learn URL returned 404 on fetch).
