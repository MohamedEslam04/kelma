//! The Outlook calendar over Microsoft Graph: events, side-by-side layout, the time ranges of
//! the calendar views, and everything that changes or surrounds them (RSVP, create and edit,
//! scheduling, rooms, calendars, sharing, settings, export).

mod api;
mod detail;
mod event;
mod filter;
mod graph;
mod layout;
mod new_event;
mod range;
mod teams_meet;
mod write;

pub use api::{Calendar, EventCache, delta_url, view_url};
pub use detail::{
    Attachment, AttachmentKind, DialIn, EventDetail, ResponseCounts, download_attachment,
    event_detail, instances, parse_attachments, parse_event_detail, response_counts,
};
pub use event::{
    Attendee, AttendeeKind, DeltaPage, Event, EventChange, Response, ShowAs, delta_link, next_link,
    parse_delta_page, parse_events,
};
pub use filter::{EventFilter, EventKind, Recurrence as RecurrenceFilter, kind_of};
pub use layout::layout_columns;
pub use new_event::{
    NewEvent, Pattern, Person, Recurrence, RecurrenceEnd, Reminder, Sensitivity, WeekIndex,
};
pub use range::{Date, View, Weekday, format_iso_ms, week_range};
pub use teams_meet::{
    EventType, MeetNow, join_by_id_url, meet_now, meet_now_body, meeting_chat_thread_id,
    parse_meet_now,
};
pub use write::{
    Rsvp, cancel_body, cancel_event, create_event, delete_event, forward_body, forward_event,
    move_body, move_event, respond, respond_body, target_id, update_event,
};
