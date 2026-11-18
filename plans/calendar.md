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
    `config.json`. Events are cached in memory and synced by delta on start and every 2 minutes. Also built: the
    date-range picker (the range label opens a month picker; two picks make a Day view of up to 7 days) and
    saved views (Views ▾ saves the current view and scale by name, applies and deletes them; kept in
    `config.json`). Still open: the store `events` table and the check against Teams for a real week.
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
  - **Status.** Built in `ui/calendar_event.slint` and `backend/cal/actions.rs`: click an event for the peek with
    Join ▾ (Join Teams meeting, Copy join link) and Chat (opens the meeting thread), the time with the series
    icon, location, "X invited you", response counts, Email organizer and a message, Accept, Tentative,
    Decline, Propose new time (C2.5), Show as, category and a … menu (Edit, Duplicate, Forward, Export .ics,
    Cancel meeting, Delete, offered only where the user may do them). Covered by UI tests and engine tests
    that run offline. Not run live: no test answers a real invitation (live rules), so the Done-when is open.
- [ ] **C1.4 Full event view.**
  - **What.** Everything in the peek, plus:
    - the attendee list with each person's response (Tracking)
    - the body (rendered HTML) and attachments
    - dial-in information
    - for an occurrence: edit this one or the whole series
  - **How.** `GET /me/events/{id}?$expand=attachments` and `/instances`.
  - **Status.** Built: "Open" in the peek shows the same actions plus the attendee list with each answer, dial-in
    numbers, attachments (click to open) and the body. The body is the HTML turned into plain text (no rich
    layout yet). Edit this occurrence or the whole series is in the … menu. Tested offline; the detail
    parser is the data layer's, checked live before. Not compared with Teams for a real event, so left open.
- [x] **C1.5 Filters.** Impact 3, effort 2.
  - **What.** Clear filters; Appointments, Meetings, Poll holds, Reservations; Categories, Show as, Recurrence, In-person.
  - **How.** Client-side, over the cached events.
  - **Status.** Done: the Filter button opens a panel with Type (Appointments, Meetings), Show as, Recurrence,
    In-person and the categories found in the events, plus Clear filters; the grid and month view narrow at
    once. Poll holds and Reservations are not offered (no marker in the Graph event). Covered by pure,
    engine and UI tests; it is client-side, so there is no live step.
- [ ] **C1.6 Calendars list.**
  - **What.** My calendars with a checkbox and color per calendar, other calendars and group calendars, Show all.
  - **Add calendar:** Recommended, Edit my calendars, Create blank calendar, Add from directory (a person's or room's calendar), Subscribe from web (ICS URL), Upload from file (.ics), Holidays, Birthdays.
  - **How.** `GET /me/calendars`, `/me/calendarGroups`, `POST /me/calendars`. Shared calendars come through `Calendars.ReadWrite.Shared`. ICS subscribe and holidays go through the Outlook REST calendar endpoints (discover).
  - **Done when.** Adding a colleague's calendar shows their free/busy or details, depending on their permissions.
  - **Status.** Built: the left pane lists My calendars and Other calendars with a color and a checkbox each, and
    Show all; ticking one fetches its events in its color (read only). Add calendar ▾: Create blank calendar and
    Add from directory (a colleague's events, or Busy blocks when only free/busy is shared) are native;
    Subscribe from web, Upload from file, Holidays, Birthdays and Edit my calendars open Outlook on the web.
    Not run against a real colleague's calendar, so left open.

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
  - **Status.** Built in `ui/calendar_form.slint` and `backend/cal/forms.rs`: New event ▾ opens the form with title,
    required and optional attendees (typed addresses, not a people picker yet), start and end, All day, Teams
    meeting, Repeat (daily, weekdays, weekly on days, monthly on the date or the weekday, yearly, every N, end
    never / on a date / after N), Scheduler, Rooms, location, Show as, reminder, category, Private, response
    options and a body (the chat formatter's markdown). It says what is wrong before sending. Not built: a
    side day preview (the Scheduler's grid shows the day), a rich-text body, a time zone picker (local only),
    and the channel pick for channel meetings only sets the location. A private appointment made through the
    form's code was created, read back, moved and deleted live; no meeting with attendees or a Teams link was
    made (live rules), so the Done-when is open.
- [ ] **C2.2 Edit, cancel and other organizer actions.** Impact 4, effort 3.
  - **What.**
    - Edit sends an update.
    - Cancel meeting with a message, for one occurrence or the whole series.
    - Delete your own appointments.
    - Forward.
    - Drag to move and drag the edge to resize (with confirmation for meetings).
    - Duplicate.
  - **How.** `PATCH /me/events/{id}`, `POST …/cancel`, `DELETE`, `POST …/forward`.
  - **Status.** Built: the peek's … menu has Edit (this occurrence), Edit series, Duplicate, Forward, Cancel
    meeting (with a message and a whole-series choice) and Delete, each asking first; dragging a block moves
    it (snapped to 15 minutes, across days) and dragging its bottom edge resizes it, asking first when it is a
    meeting with attendees. Tested offline end to end. Live: a private appointment was created, moved and
    deleted; cancel and forward were not run (live rules).
- [ ] **C2.3 Scheduling assistant ("Scheduler").** Impact 4, effort 3.
  - **What.** A grid of attendees by time slot with busy, tentative and out-of-office shading, each attendee's time zone, suggested times, and room availability.
  - **How.** `POST /me/calendar/getSchedule` and `POST /me/findMeetingTimes`.
  - **Status.** Built: the form's Scheduler shows me, each attendee and the room by 30 minute slot from 06:00 to
    22:00 with busy, tentative, out-of-office and unknown shading, work hours dimmed, each person's time
    zone when Graph gives it, the form's slot outlined (click a time to move the event there) and suggested
    times from findMeetingTimes. Tested against a getSchedule fixture. Shading uses my work hours for every
    row; per-person hours need a time zone table.
- [ ] **C2.4 Rooms.** Impact 3, effort 2.
  - **What.** Room lists, with capacity, building and floor, and live availability for the chosen slot.
  - **How.** `GET /places/microsoft.graph.roomList` and `/rooms`, plus getSchedule.
  - **Status.** Built: Rooms in the form lists the room lists, then each room with seats, building and floor
    and Free or Busy for the form's slot; clicking one fills the room. Tested with fixtures.
- [ ] **C2.5 Propose a new time,** as an attendee or as the organizer reviewing proposals. Impact 2, effort 2.
  - **How.** `proposedNewTime` on tentativelyAccept and decline.
  - **Status.** Built the attendee side: "Propose new time" in the peek asks for a start and an end and sends it
    with a tentative or decline answer. Not built: the organizer reviewing proposals (the data layer does not
    read `proposedNewTime` yet).
- [ ] **C2.6 Event types.** Impact 3, effort 3.
  - **What.** The New ▾ menu offers Event and Channel meeting. Webinar, Town hall, Virtual appointment and Live event open in the web (Graph `VirtualEvent.*` needs admin consent).
  - **Status.** Built: the New event ▾ menu lists all six; Event and Channel meeting open the form (channel
    meetings pick a team and channel, but the channel is only written into the location because the endpoint
    is not discovered), the other four open the Teams calendar in the browser.

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
  - **Status.** Built: Meet now asks for an optional title, creates the meeting through the middle tier and shows
    its link with Copy link and Join now; Join with an ID takes the ID and passcode and opens the Teams join
    page (it rejects an ID with odd characters). Not run live: Meet now and meetings are never exercised on
    this account.
- [ ] **C3.3 Share, Print, Calendar settings.** Impact 2, effort 2.
  - **What.**
    - Share: calendar permissions (free/busy, titles, all details, edit, delegate).
    - Print: export the visible range to PDF.
    - Settings: first day of week, work hours, second time zone, week numbers, show weekends.
  - **How.** Use `/me/calendars/{id}/calendarPermissions`. Work hours come from the Outlook mailbox settings (EWS or Outlook REST; Graph MailboxSettings is not in the token, so probe it).
  - **Status.** Built, under the toolbar's ⋯ menu: Share (list who can see the calendar, change a role among
    free/busy, titles, all details, edit and delegate, remove, add by address), Print to PDF (a plain agenda of
    the visible range, written without a PDF library; Latin-1 text only), Export visible range (.ics), and
    Calendar settings (first day of the week, work hours, show weekends, week numbers, a second time zone as a
    fixed UTC offset), saved in `config.json` with defaults. Work hours start from the mailbox's until set.
    Sharing was not run live (it changes permissions).
- [ ] **C3.4 Work location.** Impact 3, effort 2.
  - **What.** A per-day work location pin (Office, Remote, a specific building) in the day header and in the profile menu. Colleagues' locations show in profile cards.
  - **How.** Discover the endpoint; purple-teams uses `/v1/me/workLocation` on the presence service.
  - **Status.** UI only, as the endpoint is unverified: a day header opens Office, Remote and Clear, the pin shows
    in the header, and ⋯ has the same for today. The pin is kept in memory; the call to the presence service
    is made but never run live (live rules), and a refusal leaves a note. The profile menu and colleagues'
    pins are not built.
- [ ] **C3.5 Calendar in chat.** Impact 3, effort 1.
  - **What.** "Schedule a meeting" from a chat's header pre-fills its members.
  - **Status.** Built: a calendar button in the chat header opens the new-event form in the calendar with the
    chat's other members as required attendees (their addresses come from the profile cache), a Teams
    meeting on, and the chat's title for a group. Tested offline with a seeded cache; not run against a real
    chat.

## Order

C1.1, C1.3 and C3.1 first: they cover what most people use the calendar for. Then C2.1, C2.2, C2.3 and C2.4, then the rest.

## Endpoints already documented by open-source clients

See `docs/research/open-source.md` §3.14.
- **Teams calendar view:** `GET MT/v2.1/me/calendars/calendarView?startDate&endDate`. Unlike Graph, it carries the Teams join data (`skypeTeamsData`, thread id). Use it with Graph for C1.1 and C1.3. Sources: purple, rusty.
- **Meet now:** `POST MT/beta/me/calendarEvents/privateMeeting/schedulingService/create` `{"meetingType":"MeetNow","subject":…}` returns the thread id and the join and short links. Sources: mautrix, rusty.
- **Out of office and schedule:** `calendarData` in `getpresence`, and Loki `POST …/api/v1/schedule?smtp=` and `/oofstatus`.
