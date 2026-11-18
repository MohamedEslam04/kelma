# Token probe (2026-11-18, Teams client id 1fec8e78-…, refresh token from the keyring)
- OK  https://graph.microsoft.com/.default — scopes listed in PLAN (Files.ReadWrite.All, Calendars.ReadWrite, Mail.*, Tasks.ReadWrite, Notes.ReadWrite.All, Channel.Create, TeamMember.ReadWrite.All, TeamsTab.*, AppCatalog.Read.All, People.Read, Place.Read.All…)
- FAIL Graph OnlineMeetings.ReadWrite, Presence.ReadWrite, OnlineMeetingRecording.Read.All, CallRecords.Read.All — AADSTS65002 (first-party pre-consent missing). Not obtainable with this client id.
- OK  https://substrate.office.com/.default — Files.ReadWrite(.Shared), Notes-Internal, OfficeFeed-Internal, PeoplePredictions-Internal, Place.*, Signals.ReadWrite, … (search, people suggestions, feed)
- OK  https://outlook.office.com/.default — Calendars.ReadWrite, Calendar.ReadWrite.Shared, Contacts.ReadWrite, EWS.AccessAsUser.All, Files.*, Mail…, (Outlook REST / calendar sharing / rooms)
- OK  https://teams.microsoft.com/.default — Authorization.ReadWrite Region.ReadWrite user_impersonation
- OK  https://loki.delve.office.com/.default — profile cards (LLM.Read, User.Read.All)
- Already used: api.spaces.skype.com (skypetoken), chatsvcagg, presence.teams.microsoft.com

# Browser UI survey of rail apps (teams.cloud.microsoft)
## Calendar (Outlook calendar module in an iframe)
- Toolbar: collapse pane, Today, ‹ ›, date-range picker, view (Day[>N days], Work week, Week, Month, Split view, Time scale 30 min, Saved views), Filter (Clear; Appointments, Meetings›, Poll holds›, Reservations›, Categories›, Show as›, Recurrence›, In-person›), … (Share, Print, Calendar settings), Meet now ▾ (Join with an ID), New ▾ (Event, Channel meeting; templates: Webinar, Town hall, Virtual appointment, Live Event)
- Left: mini month calendar, Add calendar (Recommended, Edit my calendars, Create blank calendar, Add from directory, Subscribe from web, Upload from file, Holidays, Birthdays), My calendars (checkbox per calendar), Show all
- Grid: current-time line, day headers, today marker, work location pin per day
- Event peek: title, expand, Join ▾ (Join Teams meeting, Copy join link), Chat, time + series icon, location, organizer "X invited you", response counts (Accepted 7, Didn't respond 26), Email organizer + optional message, Accept ▾, Decline ▾, Show-as/category ▾, … (Tentative ›)
- New event form: Event / Series, Show as (Busy), reminder, category, (sensitivity/share), Save; Add title; Invite required attendees (+ optional); date/time; Scheduler; room or location; Microsoft Teams meeting toggle; rich body (attach, image, Loop, format, …); Add an agenda; right-side day preview with the new slot
## Calls
- Tab Personal; "Type a name" + Call; View contacts (external); Speed dial (Add); forwarding menu (Don't forward, Forward to voicemail, More settings); device menu (current device, Custom setup, Device settings)
- History: All, Missed, Incoming, Outgoing, Voicemail, Filter; rows with direction + duration; item menu: Call back, Chat, Remove from view, Add to speed dial, Add contact, Report call
- Settings › Calls: forward all calls to voicemail; when you can't answer → redirect to voicemail; ring seconds; ringtone (+preview); Voicemail: record greeting, greeting language, what happens on redirect, TTS greeting; Out-of-office voicemail (all the time / when Outlook auto-replies on; custom greeting)
## OneDrive (iframe)
- Create or upload: Folder, Files upload, Folder upload, Word, Excel, PowerPoint, OneNote notebook, Forms survey, Visio drawing, Markdown document, Text document, Link
- Nav: Home, My files, Shared, Favorites, Libraries, Recycle bin; Browse files by: People, Meetings, Media; Quick access (team sites)
- Home: "For you" cards (frequently open, shared with you), Recent with type pills (All, Word, Excel, PowerPoint, PDF, More), filter by name or person; columns Name, Opened, Owner, Activity ("You shared this in a Teams chat")
- Row hover: …, Share, Favorite; selection toolbar: Open ▾, Open location, Share, Copy link, Add to favorites, Add shortcut ▾, Rename, Move to, Copy to, Download
## Copilot
- New chat, Agents, Notebooks, Chats history; model picker "Auto"; "+" attach; enterprise data protection shield; prompt chips Refine writing, Draft content, Summarize, Find content, …
## Meeting Recaps
- All / Followed, Search for meetings, List / Grid view, pin/unpin from app bar
## Store
- Home, Featured ▾, Categories ▾, Devices ▾, search "agents and apps", settings gear, Manage your apps; Popular on Teams (Polls, Jira, YouTube, Forms, Lists, Copilot, Power BI, SharePoint, Cowork, Avatars, Polly, Planner…); app page: Add, share, rating, installs, works in, price, screenshots, privacy/terms/permissions
## View more apps (…)
- Search apps; Linear, Avatars, Shifts, Planner, OneNote, Help, Clipchamp, Whiteboard, Events, People, Visio, Word, Excel, PowerPoint, Loop, Workflows, Approvals, Viva Learning, Updates, Power BI, SharePoint, Viva Insights
## Teams management ("Your teams and channels")
- Tabs Teams, Pending invites, Analytics; filters All, Teams you own, 3 more; Sort A–Z; Search teams; Create team ▾; rows: name, Public, people, channels, owner, role; "See all channels", "See all your teams"; Communities (All Company), Join communities
## Settings not covered by chat plan
- Devices: speaker, microphone, test call, camera, mirror, brightness/low light, connected devices, meeting room mode
- Accessibility: sign language mode, always show captions, identify me in captions, profanity filter, muted notification, single pointer, TTY
- Accounts and orgs: account + org list, Manage, Refresh license and restart
- General: meeting controls, language/date/time format, time zone, translation

# Token probe round 2
- OK  substrate.office.com/sydney — CopilotPlatformContent.Process.All, CopilotPlatformFiles.Read(Write)… (M365 Copilot chat hub)
- OK  <tenant>-my.sharepoint.com and <tenant>.sharepoint.com — Sites.FullControl.All, Sites.Manage.All, MyFiles.Write (SharePoint REST: recycle bin, versions, sharing, recordings)
- OK  api.yammer.com — access_as_user (Viva Engage / Communities)
- OK  service.flow.microsoft.com — Flows.Manage.All, Flows.Read.All (Workflows)
- OK  tasks.office.com — Group.ReadWrite.All (Planner)
- OK  onenote.com — Notes.ReadWrite.All
- OK  ic3.teams.office.com — Teams.AccessAsUser.All (Teams calling/ic3 backend)
- FAIL config.teams.microsoft.com — resource not found
