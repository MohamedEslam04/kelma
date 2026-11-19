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

- [ ] **C1.1 Views.** DEFERRED: needs test tenant (the Done-when is a visual comparison of a real week with Teams). Impact 5, effort 3.
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
    `config.json`). Still open: the store `events` table (events live in memory) and the check against Teams for a real week.
- [ ] **C1.2 Event look.** DEFERRED: needs test tenant (the Done-when is a comparison with Teams for the user's real week).
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
- [ ] **C1.3 Event peek** (click an event). DEFERRED: needs test tenant (an RSVP on a real invitation is a write the live rules forbid).
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
- [ ] **C1.4 Full event view.** DEFERRED: needs test tenant (the check is a real event side by side with Teams; the body shows as plain text from the HTML).
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
- [ ] **C1.6 Calendars list.** DEFERRED: needs test tenant (the Done-when adds a real colleague's calendar).
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

- [ ] **C2.1 New event form.** DEFERRED: waits for the tester session (the Done-when needs a meeting with a Teams link and an attendee; private appointments on the user's own calendar now pass live).
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
    and the channel pick for channel meetings only sets the location. Not run live: an
    ignored test (`live_form_round_trip` in `backend/cal/forms.rs`) makes, moves and deletes private
    appointments through the form's code, but it has not been run yet; no meeting with attendees or a Teams
    link is made from tests (live rules), so the Done-when is open.
  - **Evidence (own calendar, 2026-11-19).** `cargo test -p kelma --bin kelma live_form_round_trip -- --ignored --nocapture`
    on the user's own calendar (private, no attendees, 40 days out): local time round trip true, body round trip
    true, moved an hour true, weekly series of 3 occurrences, cleanup proof 0 events left. Two bugs found and fixed
    on the way, each with a test first:
    - A `POST /me/events` answer carries the zone the event was sent in (`Africa/Cairo`), not UTC, so
      `parse_event` read it 2 hours off. Times are now read in their own `timeZone`
      (`event::tests::times_are_read_in_their_own_zone`).
    - Exchange refuses a series in an IANA zone (`400 ErrorPropertyValidationFailure`, `TimeZoneDefinitionStart`).
      A series now goes out in the Windows zone (`Egypt Standard Time`, `tz::windows_name`;
      `new_event::tests::a_series_goes_out_in_the_windows_zone`).
  - **Tester session.** `kelma-cli calendar tester invite <address> | status | accept-proposal | move | forward <address> | cancel | cleanup`
    runs the meeting checks of C2.1, C2.2 and C2.5 with one consenting tester, one step at a time.
- [ ] **C2.2 Edit, cancel and other organizer actions.** DEFERRED: waits for the tester session (cancel, forward and moving a meeting mail the attendee; create, move and delete of private appointments pass live, see C2.1). Impact 4, effort 3.
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
    meeting with attendees. Tested offline end to end. Not run live (cancel and forward never are, per the
    live rules; the ignored `live_form_round_trip` covers create, move and delete of a private appointment).
- [x] **C2.3 Scheduling assistant ("Scheduler").** Impact 4, effort 3.
  - **What.** A grid of attendees by time slot with busy, tentative and out-of-office shading, each attendee's time zone, suggested times, and room availability.
  - **How.** `POST /me/calendar/getSchedule` and `POST /me/findMeetingTimes`.
  - **Status.** Built: the form's Scheduler shows me, each attendee and the room by 30 minute slot from 06:00 to
    22:00 with busy, tentative, out-of-office and unknown shading, work hours dimmed, each person's time
    zone when Graph gives it, the form's slot outlined (click a time to move the event there) and suggested
    times from findMeetingTimes. Tested against a getSchedule fixture. Each row is dimmed by that person's
    own work hours (see Evidence); a row with no hours from Graph (an unreadable calendar) is not dimmed.
  - **Evidence (data layer, 2026-11-19).** `calendar::work_slots(hours, start_ms, interval, slots)` and
    `work_intervals` place each attendee's `workingHours` (from getSchedule) in their own zone: Windows
    names map to IANA (`tz.rs`, CLDR table plus three Exchange-only names) and a `customTimeZone` (bias and
    transition rules) is parsed into `WorkingHours.custom_zone`. Unit tests cover a named zone, a custom
    zone across daylight saving and slot shading. Live, read-only: `kelma-cli calendar schedule` reads my
    getSchedule work hours (a named zone that maps to IANA) and shades 18 of 48 half-hours; all 143 Windows
    zones Graph offers map to IANA (`kelma-cli calendar timezones`).
  - **Evidence (UI, 2026-11-19).** `model::shade_work_hours` calls `work_slots` per row with that row's
    `getSchedule` `workingHours` (my row falls back to my own hours); the grid dims where the row's `work`
    flag is false. Unit test `each_scheduler_row_is_shaded_by_that_persons_own_work_hours` (Pacific, Cairo,
    no-hours and unreadable rows against one UTC window).
- [x] **C2.4 Rooms.** Impact 3, effort 2.
  - **What.** Room lists, with capacity, building and floor, and live availability for the chosen slot.
  - **How.** `GET /places/microsoft.graph.roomList` and `/rooms`, plus getSchedule.
  - **Status.** Built: Rooms in the form lists the room lists, then each room with seats, building and floor
    and Free or Busy for the form's slot; clicking one fills the room. Tested with fixtures. The list scrolls,
    so a building with many rooms is all reachable.
  - **Evidence (data layer, 2026-11-19).** `room_lists` and `rooms` fall back to Graph beta
    `/me/findRoomLists` and `/me/findRooms(RoomList='…')` (no Place scope) when places is refused or empty;
    `room_schedules` reads any number of rooms 20 at a time and `free_between` says whether a slot is free.
    Live, read-only: `/places/microsoft.graph.room` and `…roomList` answer 200 (so places is readable
    without Place.Read.All), and so do beta `findRooms`/`findRoomLists`, but all four are empty for this
    tenant: it publishes no room mailboxes, so no room data could be checked live. Outlook REST
    `api/v2.0/me/findrooms` is a 400 (only `api/beta` answers).
  - **Evidence (UI, 2026-11-19).** The picker's `cal_rooms` reads every room of the list through
    `room_schedules` (20 addresses a call, no cap at 20 any more) and `model::room_status` answers Free,
    Busy or unknown with `free_between` for the form's slot. Unit test
    `rooms_show_capacity_place_and_availability_for_the_slot` (free, busy by overlap, no answer, denied, and
    the same room at another slot) and the UI test
    `rooms_show_capacity_and_availability_and_picking_one_fills_the_form`. Not checked against a real room:
    this tenant has none, so the live step is fixtures only.
- [ ] **C2.5 Propose a new time,** as an attendee or as the organizer reviewing proposals. DEFERRED: waits for the tester session (a real proposal from the tester is needed, and Accept moves the meeting; `calendar tester status` and `accept-proposal` are ready and mails its attendees). Impact 2, effort 2.
  - **How.** `proposedNewTime` on tentativelyAccept and decline.
  - **Status.** Built both sides. Attendee: "Propose new time" in the peek asks for a start and an end and sends
    it with a tentative or decline answer. Organizer: opening one of my meetings with attendees reads
    `event_proposals`, and the full view lists "Proposed new times" (who, which slot in local time, tentative or
    declined) with an Accept button; Accept runs the same move as dragging the meeting there, so it asks first
    ("The attendees get an update") and then sends the `PATCH`. Tested offline: `proposals_read_as_who_proposed_which_slot_in_local_time`,
    `accepting_a_proposed_time_asks_first_then_moves_the_meeting_there` (engine, offline path only) and the UI
    test `the_full_view_lists_proposed_times_and_accept_sends_the_one_clicked`. Never run live (live rules).
  - **Evidence (data layer, 2026-11-19).** `parse_event_proposals` reads `proposedNewTime` on an event's
    attendees and `parse_proposal_messages` reads `eventMessageResponse` answers (Graph or Outlook REST case,
    any zone converted to UTC); `event_proposals(client, id)` and `proposal_messages(client, top)` fetch them,
    and `accept_proposal_request` builds the move to the proposed slot for a dry run (it is the same PATCH
    as `move_event`). Graph's v1.0 enum is spelled `meetingTenativelyAccepted` (the correct spelling is a
    400) and `$orderby` with that filter is refused (`InefficientFilter`). Live, read-only: `kelma-cli
    calendar proposals` runs both reads (HTTP 200); the mailbox holds no proposals, so parsing is checked
    by fixtures only.
- [ ] **C2.6 Event types.** BLOCKED: the channel meeting endpoint is not discovered (a channel is only written into the location; finding it needs a real channel meeting). Impact 3, effort 3.
  - **What.** The New ▾ menu offers Event and Channel meeting. Webinar, Town hall, Virtual appointment and Live event open in the web (Graph `VirtualEvent.*` needs admin consent).
  - **Status.** Built: the New event ▾ menu lists all six; Event and Channel meeting open the form (channel
    meetings pick a team and channel, but the channel is only written into the location because the endpoint
    is not discovered), the other four open the Teams calendar in the browser.

## Phase C3: around the calendar

- [ ] **C3.1 Reminders and join banners.** DEFERRED: needs test tenant (the "Meeting started" part: a meeting's start can only be seen in a real meeting, and the `Event/Call` payload has not been captured; today the chat shows it as a plain "Call" line in `backend/util.rs`). Impact 5, effort 2.
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
    (`snoozeReminder`, `dismissReminder`) and have not been run against a live account. Not built: the
    "Meeting started" notification (deferred, see above; it would hook the trouter `Event/Call` message in
    `backend/live.rs`).
  - **Status (2026-11-19, `feat/calls-page`).** A calendar-driven "started" notification now goes out at a Teams
    meeting's start time (once, within its first 5 minutes) with Join, Snooze and Dismiss
    (`reminders::just_started`). The Join banner and that notification skip the meeting that is already running,
    being joined or on the pre-join screen (`CallManager::is_in_meeting`), and Join opens the pre-join screen
    titled with the meeting's subject (`reminders::subject_for_link`). Tests: `cargo test -p kelma reminders`.
    Still open: the chat's "Meeting started" from the trouter `Event/Call` push (the payload is not captured).
- [ ] **C3.2 Meet now and Join with an ID.** DEFERRED: needs test tenant (Meet now and meetings are never exercised on the work account). Impact 4, effort 2.
  - **What.**
    - Meet now creates an instant meeting with a share link.
    - Join with an ID takes a meeting ID and passcode.
  - **How.**
    - Meet now: an event starting now with a Teams meeting, then the meeting launcher.
    - Join by ID resolves through the Teams web join page (`https://teams.microsoft.com/meet/<id>?p=<passcode>`).
  - **Status.** Built: Meet now asks for an optional title, creates the meeting through the middle tier and shows
    its link with Copy link and Join now; Join with an ID takes the ID and passcode and opens the Teams join
    page (it rejects an ID with odd characters). DEFERRED: needs test tenant (Meet now and meetings are never exercised on the
    work account).
  - **Evidence (Join and Join with an ID, 2026-11-19).** UI built; live check pending with tester. With the native
    call stack, the peek's Join, "Join Teams meeting" and "Join with an ID" open kelma's pre-join screen and join by
    the meeting's link or by code (`backend/call.rs`, `ui_tests/call.rs`
    `join_with_an_id_opens_the_prejoin_for_that_meeting`); without it they open the Teams web join page as before.
- [ ] **C3.3 Share, Print, Calendar settings.** DEFERRED: needs test tenant (changing who can see the calendar is a real permission write; every other part is built and tested offline). Impact 2, effort 2.
  - **What.**
    - Share: calendar permissions (free/busy, titles, all details, edit, delegate).
    - Print: export the visible range to PDF.
    - Settings: first day of week, work hours, second time zone, week numbers, show weekends.
  - **How.** Use `/me/calendars/{id}/calendarPermissions`. Work hours come from the Outlook mailbox settings (EWS or Outlook REST; Graph MailboxSettings is not in the token, so probe it).
  - **Status.** Built, under the toolbar's ⋯ menu: Share (list who can see the calendar, change a role among
    free/busy, titles, all details, edit and delegate, remove, add by address), Print to PDF (a plain agenda of
    the visible range, written without a PDF library; Latin-1 text only), Export visible range (.ics), and
    Calendar settings (first day of the week, work hours, show weekends, week numbers, a second time zone),
    saved in `config.json` with defaults. Work hours start from the mailbox's (Graph `mailboxSettings`, read in
    the mailbox's zone and shown in local time) until set. The second time zone is picked by name from the
    mailbox's list (search box, Windows names) and stored as a zone name, so its offset follows daylight saving;
    an older config with only `second_tz_min` keeps showing that fixed offset until a zone is picked. The Share
    dialog offers only the roles Graph's `allowedRoles` allows (all five when it says nothing).
    Sharing was not run live (it changes permissions).
  - **Evidence (data layer, 2026-11-19).** Work hours: Graph `GET /me/mailboxSettings` and
    `/me/mailboxSettings/workingHours` answer 200 for this client (the earlier 403 was Outlook REST
    `api/v2.0/me/MailboxSettings`, still 403), so `mailbox_settings` now asks Graph first, then Outlook
    REST, then getSchedule; EWS was not needed. Time zones: `iana_name`, `utc_offset_min` and
    `local_to_utc_ms` (chrono-tz) turn Windows or IANA names into offsets with daylight saving, and
    `supported_time_zones` lists 143 Windows or 553 IANA zones for a picker (all known to chrono-tz).
    Sharing: `calendar_permissions` now keeps `allowedRoles`; `add/update/remove_permission_request` build
    the writes for `kelma-cli calendar permissions --dry-run <role>`, which prints them without sending.
    Live, read-only: the default calendar's permissions and both zone lists were read. Sharing writes were
    never sent (live rules).
  - **Evidence (UI, 2026-11-19).** Tests: `mailbox_work_hours_are_read_in_the_mailbox_zone_then_shown_in_local_time`,
    `a_second_time_zone_name_follows_daylight_saving_and_beats_the_old_offset` (New York is UTC-5 in January and
    UTC-4 in July; an unknown name falls back to the old offset), `settings_parse_validate_and_keep_the_rest`
    (a name or the old minutes are accepted, an unknown name is refused), the session test that an old config
    with only `second_tz_min` still loads, `the_zone_picker_finds_zones_by_name_or_display_name`,
    `share_roles_are_limited_to_what_the_permission_allows` and the UI test
    `the_share_dialog_offers_only_the_roles_the_permission_allows`.
- [ ] **C3.4 Work location.** DEFERRED: needs test tenant (work location is never exercised on the work account). Impact 3, effort 2.
  - **What.** A per-day work location pin (Office, Remote, a specific building) in the day header and in the profile menu. Colleagues' locations show in profile cards.
  - **How.** Discover the endpoint; purple-teams uses `/v1/me/workLocation` on the presence service.
  - **Status.** UI only, as the endpoint is unverified: a day header opens Office, Remote and Clear, the pin shows
    in the header, and ⋯ has the same for today. The pin is kept in memory; the call to the presence service
    is made but never run. DEFERRED: needs test tenant (work location is never exercised on the work account);
    a refusal leaves a note. The profile menu and colleagues'
    pins are not built.
- [x] **C3.5 Calendar in chat.** Impact 3, effort 1.
  - **What.** "Schedule a meeting" from a chat's header pre-fills its members.
  - **Status.** Built: a calendar button in the chat header opens the new-event form in the calendar with the
    chat's other members as required attendees (their addresses come from the profile cache), a Teams
    meeting on, and the chat's title for a group. Tested offline with a seeded cache; not run against a real
    chat.
  - **Evidence.** Engine test `schedule_a_meeting_from_a_chat_opens_the_form_with_the_other_member` (opens the
    calendar view with the form filled: the other member as required attendee, me left out, Teams meeting on).
    Opening the form sends nothing, so no live step is needed.

## Order

C1.1, C1.3 and C3.1 first: they cover what most people use the calendar for. Then C2.1, C2.2, C2.3 and C2.4, then the rest.

## Endpoints already documented by open-source clients

See `docs/research/open-source.md` §3.14.
- **Teams calendar view:** `GET MT/v2.1/me/calendars/calendarView?startDate&endDate`. Unlike Graph, it carries the Teams join data (`skypeTeamsData`, thread id). Use it with Graph for C1.1 and C1.3. Sources: purple, rusty.
- **Meet now:** `POST MT/beta/me/calendarEvents/privateMeeting/schedulingService/create` `{"meetingType":"MeetNow","subject":…}` returns the thread id and the join and short links. Sources: mautrix, rusty.
- **Out of office and schedule:** `calendarData` in `getpresence`, and Loki `POST …/api/v1/schedule?smtp=` and `/oofstatus`.
