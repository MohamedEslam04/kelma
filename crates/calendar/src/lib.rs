//! Read the Outlook calendar over Microsoft Graph: events, side-by-side layout and the time
//! ranges of the calendar views.

mod api;
mod event;
mod layout;
mod range;

pub use api::{Calendar, EventCache, delta_url, view_url};
pub use event::{
    Attendee, AttendeeKind, DeltaPage, Event, EventChange, Response, ShowAs, delta_link, next_link,
    parse_delta_page, parse_events,
};
pub use layout::layout_columns;
pub use range::{Date, View, Weekday, format_iso_ms, week_range};
