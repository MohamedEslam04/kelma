# kelma plan: Calendar

Part of the Teams parity plans (see `plans/README.md`). Format, scoring and the definition of done
are the same as in `plans/chat.md`.

**Sources.**
- UI survey of the Calendar app in Teams web (2026-11-18). The calendar is the Outlook calendar
  module in an iframe.
- Microsoft Learn and Graph docs, in `docs/research/calendar-meetings.md` (summarized below).
- A token probe with kelma's client id.

**Access.**
- The Graph token already carries `Calendars.ReadWrite`, `Calendars.ReadWrite.Shared`,
  `Place.Read.All`, `People.Read`, `User.ReadBasic.All` and `Mail.ReadWrite`.
- An `outlook.office.com` token also works: `Calendars.ReadWrite`, `Calendar.ReadWrite.Shared`,
  `EWS.AccessAsUser.All`. It is the fallback for what Graph lacks, such as categories and mailbox
  settings.
- Graph `OnlineMeetings.*` is **refused** for this client (AADSTS65002), so meeting options are a
  web page (see `plans/meetings.md`).

**Everything here can be native.** No webview is needed.

## Architecture

- A new crate `crates/calendar`. It talks to Graph `/me/calendarView` with
  `Prefer: outlook.timezone="<local tz>"` and `Prefer: IdType="ImmutableId"`.
- The store gets an `events` table, synced with `/me/calendarView/delta` on start, every 2 minutes,
  and when trouter or calendar notifications arrive.
- A Calendar rail item in `app.slint` with its own view tree, sharing `Theme`, `Avatar` and the
  people picker with chat.

## Phase C1: read the calendar

- [ ] **C1.1 Views.** Impact 5, effort 3.
  - **What.** Day (with "N days"), Work week (default), Week, Month.
    - Time scale 15, 30 or 60 minutes.
    - A current-time line and a today marker.
    - All-day row at the top.
    - Overlapping events side by side.
    - Toolbar: Today, ‹ ›, and a date-range picker.
    - Mini month calendar on the left.
    - Remember the view and saved views.
  - **How.** `GET /me/calendarView?startDateTime&endDateTime` for the visible range plus one range ahead; cache in the store.
  - **Done when.** The same week shows the same events as Teams, including recurring and all-day events.
  - **Status.** Built in `crates/kelma/ui/calendar.slint` and `crates/kelma/src/backend/cal/`: Day (1 to 7 days),
    Work week (default), Week and Month; 15, 30 or 60 minute scale; current-time line and today marker; all-day
    row; overlapping events side by side; Today, ‹ ›; mini month; the last view and scale are saved in
    `config.json`. Events are cached in memory and synced by delta on start and every 2 minutes. Still open: the
    date-range picker (the range label is not clickable yet), saved views, the store `events` table, and the
    check against Teams for a real week.
- [ ] **C1.2 Event look.**
  - **What.**
    - Show as Busy, Tentative (hatched), Free, Out of office and Working elsewhere.
    - Declined events struck through; private events show a lock.
    - Category colors.
    - A Teams meeting icon and a series icon.
    - The organizer's name under the title.
  - **Done when.** It matches Teams for the user's real week.
  - **Status.** Built: Busy, Tentative (hatched, also for a tentative reply), Free (outline), Out of office
    (purple) and Working elsewhere; declined and cancelled events struck through; lock, Teams meeting and
    series icons; the organizer under the title. Category colors use Outlook's preset names, and a stable color
    from the name for custom ones; the real colors live in the mailbox master list (not read yet).
- [ ] **C1.3 Event peek** (click an event).
  - **What.**
    - Title and expand.
    - Join ▾ (Join Teams meeting, Copy join link) and Chat.
    - Time with the series icon, location, and "X invited you".
    - Response counts ("Accepted 7, Didn't respond 26").
    - Email organizer (checkbox) and "Add a message (optional)".
    - Accept ▾, Decline ▾, Show as and category ▾, … (Tentative).
  - **How.**
    - RSVP: `POST /me/events/{id}/accept|tentativelyAccept|decline` with `{comment, sendResponse}`.
    - Chat opens the meeting thread (`19:meeting_…@thread.v2` from the join URL) in kelma's chat.
    - Join: `plans/meetings.md`.
  - **Done when.** An RSVP on a test invite shows in Outlook.
- [ ] **C1.4 Full event view.**
  - **What.** Everything in the peek, plus:
    - the attendee list with each person's response (Tracking)
    - the body (rendered HTML) and attachments
    - dial-in information
    - for an occurrence: edit this one or the whole series
  - **How.** `GET /me/events/{id}?$expand=attachments` and `/instances`.
- [ ] **C1.5 Filters.** Impact 3, effort 2.
  - **What.** Clear filters; Appointments, Meetings, Poll holds, Reservations; Categories, Show as, Recurrence, In-person.
  - **How.** Client-side, over the cached events.
- [ ] **C1.6 Calendars list.**
  - **What.** My calendars with a checkbox and color per calendar, other calendars and group calendars, Show all.
  - **Add calendar:** Recommended, Edit my calendars, Create blank calendar, Add from directory (a person's or room's calendar), Subscribe from web (ICS URL), Upload from file (.ics), Holidays, Birthdays.
  - **How.** `GET /me/calendars`, `/me/calendarGroups`, `POST /me/calendars`. Shared calendars come through `Calendars.ReadWrite.Shared`. ICS subscribe and holidays go through the Outlook REST calendar endpoints (discover).
  - **Done when.** Adding a colleague's calendar shows their free/busy or details, depending on their permissions.

## Phase C2: write the calendar

- [ ] **C2.1 New event form.** Impact 5, effort 4.
  - **What.**
    - Event / Series, Show as (Busy), reminder, category, sensitivity (private), Save.
    - Title.
    - Required attendees, plus "optional".
    - Start and end date and time, All day, time zone.
    - Repeat (daily, weekdays, weekly on days, monthly on date or nth weekday, yearly, custom interval and end).
    - Scheduler (C2.3).
    - Room or location (C2.4).
    - Microsoft Teams meeting toggle.
    - Rich body (reuses the chat formatter) and "Add an agenda".
    - A side day preview with the new slot.
    - Response options: request responses, allow new time proposals, hide the attendee list.
    - Channel meeting: pick a team and channel.
  - **How.**
    - `POST /me/events` with `transactionId` so a retry never creates a duplicate.
    - The Teams meeting needs `isOnlineMeeting: true, onlineMeetingProvider: "teamsForBusiness"` (no OnlineMeetings scope needed); the result has `onlineMeeting.joinUrl` and dial-in details.
    - Channel meetings: discover, probably through the Teams middle tier.
  - **Done when.** A meeting created in kelma with a Teams link shows correctly in Teams and Outlook, and its Join works.
- [ ] **C2.2 Edit, cancel and other organizer actions.** Impact 4, effort 3.
  - **What.**
    - Edit sends an update.
    - Cancel meeting with a message, for one occurrence or the whole series.
    - Delete your own appointments.
    - Forward.
    - Drag to move and drag the edge to resize (with confirmation for meetings).
    - Duplicate.
  - **How.** `PATCH /me/events/{id}`, `POST …/cancel`, `DELETE`, `POST …/forward`.
- [ ] **C2.3 Scheduling assistant ("Scheduler").** Impact 4, effort 3.
  - **What.** A grid of attendees by time slot with busy, tentative and out-of-office shading, each attendee's time zone, suggested times, and room availability.
  - **How.** `POST /me/calendar/getSchedule` and `POST /me/findMeetingTimes`.
- [ ] **C2.4 Rooms.** Impact 3, effort 2.
  - **What.** Room lists, with capacity, building and floor, and live availability for the chosen slot.
  - **How.** `GET /places/microsoft.graph.roomList` and `/rooms`, plus getSchedule.
- [ ] **C2.5 Propose a new time,** as an attendee or as the organizer reviewing proposals. Impact 2, effort 2.
  - **How.** `proposedNewTime` on tentativelyAccept and decline.
- [ ] **C2.6 Event types.** Impact 3, effort 3.
  - **What.** The New ▾ menu offers Event and Channel meeting. Webinar, Town hall, Virtual appointment and Live event open in the web (Graph `VirtualEvent.*` needs admin consent).

## Phase C3: around the calendar

- [ ] **C3.1 Reminders and join banners.** Impact 5, effort 2.
  - **What.**
    - Desktop notifications at each event's reminder time, with Join, Snooze and Dismiss.
    - A "Join" banner in kelma 5 minutes before a Teams meeting.
    - "Meeting started" when the meeting chat posts the start event.
  - **How.**
    - Local timers from the cached events.
    - `POST …/snoozeReminder|dismissReminder`.
    - The start event comes from the meeting chat thread over trouter.
  - **Status.** Built: a 15 second tick over the cached events fires a desktop notification at
    `reminderMinutesBeforeStart` (default 15) with Join, Snooze 5 min and Dismiss actions, and an in-app Join
    banner from 5 minutes before a Teams meeting until it ends. Snooze and Dismiss call Graph
    (`snoozeReminder`, `dismissReminder`) and have not been run against a live account. Still open: the
    "Meeting started" notification.
- [ ] **C3.2 Meet now and Join with an ID.** Impact 4, effort 2.
  - **What.**
    - Meet now creates an instant meeting with a share link.
    - Join with an ID takes a meeting ID and passcode.
  - **How.**
    - Meet now: an event starting now with a Teams meeting, then the meeting launcher.
    - Join by ID resolves through the Teams web join page (`https://teams.microsoft.com/meet/<id>?p=<passcode>`).
- [ ] **C3.3 Share, Print, Calendar settings.** Impact 2, effort 2.
  - **What.**
    - Share: calendar permissions (free/busy, titles, all details, edit, delegate).
    - Print: export the visible range to PDF.
    - Settings: first day of week, work hours, second time zone, week numbers, show weekends.
  - **How.** Use `/me/calendars/{id}/calendarPermissions`. Work hours come from the Outlook mailbox settings (EWS or Outlook REST; Graph MailboxSettings is not in the token, so probe it).
- [ ] **C3.4 Work location.** Impact 3, effort 2.
  - **What.** A per-day work location pin (Office, Remote, a specific building) in the day header and in the profile menu. Colleagues' locations show in profile cards.
  - **How.** Discover the endpoint; purple-teams uses `/v1/me/workLocation` on the presence service.
- [ ] **C3.5 Calendar in chat.** Impact 3, effort 1.
  - **What.** "Schedule a meeting" from a chat's header pre-fills its members.

## Order

C1.1, C1.3 and C3.1 first: they cover what most people use the calendar for. Then C2.1, C2.2, C2.3 and C2.4, then the rest.

## Endpoints already documented by open-source clients

See `docs/research/open-source.md` §3.14.
- **Teams calendar view:** `GET MT/v2.1/me/calendars/calendarView?startDate&endDate`. Unlike Graph, it carries the Teams join data (`skypeTeamsData`, thread id). Use it with Graph for C1.1 and C1.3. Sources: purple, rusty.
- **Meet now:** `POST MT/beta/me/calendarEvents/privateMeeting/schedulingService/create` `{"meetingType":"MeetNow","subject":…}` returns the thread id and the join and short links. Sources: mautrix, rusty.
- **Out of office and schedule:** `calendarData` in `getpresence`, and Loki `POST …/api/v1/schedule?smtp=` and `/oofstatus`.
