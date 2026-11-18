//! Client-side calendar filters (C1.5), applied over the cached events.
//!
//! Inside one group the choices are alternatives (Appointments or Meetings); different groups
//! must all match. An empty group matches everything. Poll holds and Reservations have no
//! marker in the Graph event, so they are not offered.

use crate::event::{Event, ShowAs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// Nobody else is invited and it is not a Teams meeting.
    Appointment,
    Meeting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recurrence {
    Single,
    Recurring,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventFilter {
    pub kinds: Vec<EventKind>,
    /// Case-insensitive category names.
    pub categories: Vec<String>,
    pub show_as: Vec<ShowAs>,
    pub recurrence: Vec<Recurrence>,
    /// Only events with a physical place (not just the Teams meeting).
    pub in_person: bool,
}

pub fn kind_of(e: &Event) -> EventKind {
    if e.attendees.is_empty() && !e.is_online_meeting {
        EventKind::Appointment
    } else {
        EventKind::Meeting
    }
}

fn is_in_person(e: &Event) -> bool {
    let place = e.location.trim();
    !place.is_empty() && !place.eq_ignore_ascii_case("microsoft teams meeting")
}

impl EventFilter {
    /// "Clear filters".
    pub fn clear(&mut self) {
        *self = EventFilter::default();
    }

    pub fn is_empty(&self) -> bool {
        *self == EventFilter::default()
    }

    pub fn matches(&self, e: &Event) -> bool {
        (self.kinds.is_empty() || self.kinds.contains(&kind_of(e)))
            && (self.categories.is_empty()
                || e.categories
                    .iter()
                    .any(|c| self.categories.iter().any(|f| f.eq_ignore_ascii_case(c))))
            && (self.show_as.is_empty() || self.show_as.contains(&e.show_as))
            && (self.recurrence.is_empty()
                || self.recurrence.contains(&if e.is_recurring {
                    Recurrence::Recurring
                } else {
                    Recurrence::Single
                }))
            && (!self.in_person || is_in_person(e))
    }

    pub fn apply<'a>(&self, events: &'a [Event]) -> Vec<&'a Event> {
        events.iter().filter(|e| self.matches(e)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn events() -> Vec<Event> {
        crate::parse_events(&json!({"value": [
            {"id": "focus", "subject": "Focus time", "showAs": "busy"},
            {"id": "standup", "type": "occurrence", "isOnlineMeeting": true, "showAs": "busy",
             "categories": ["Blue"],
             "attendees": [{"emailAddress": {"address": "a@x.example"}}]},
            {"id": "lunch", "showAs": "free", "categories": ["Green", "blue"],
             "location": {"displayName": "Cafeteria"}},
            {"id": "review", "isOnlineMeeting": true, "showAs": "tentative",
             "location": {"displayName": "Microsoft Teams Meeting"},
             "attendees": [{"emailAddress": {"address": "b@x.example"}}]}
        ]}))
    }

    fn ids(f: &EventFilter, ev: &[Event]) -> Vec<String> {
        f.apply(ev).iter().map(|e| e.id.clone()).collect()
    }

    #[test]
    fn empty_filter_keeps_everything() {
        let ev = events();
        let f = EventFilter::default();
        assert!(f.is_empty());
        assert_eq!(ids(&f, &ev), ["focus", "standup", "lunch", "review"]);
    }

    #[test]
    fn appointments_vs_meetings() {
        let ev = events();
        let f = EventFilter {
            kinds: vec![EventKind::Appointment],
            ..Default::default()
        };
        assert_eq!(ids(&f, &ev), ["focus", "lunch"]);
        let f = EventFilter {
            kinds: vec![EventKind::Meeting],
            ..Default::default()
        };
        assert_eq!(ids(&f, &ev), ["standup", "review"]);
        let f = EventFilter {
            kinds: vec![EventKind::Meeting, EventKind::Appointment],
            ..Default::default()
        };
        assert_eq!(ids(&f, &ev).len(), 4, "alternatives in one group");
    }

    #[test]
    fn categories_ignore_case_and_groups_combine_with_and() {
        let ev = events();
        let f = EventFilter {
            categories: vec!["BLUE".into()],
            ..Default::default()
        };
        assert_eq!(ids(&f, &ev), ["standup", "lunch"]);
        let f = EventFilter {
            categories: vec!["blue".into()],
            kinds: vec![EventKind::Meeting],
            ..Default::default()
        };
        assert_eq!(ids(&f, &ev), ["standup"]);
    }

    #[test]
    fn show_as_recurrence_and_in_person() {
        let ev = events();
        let f = EventFilter {
            show_as: vec![ShowAs::Free, ShowAs::Tentative],
            ..Default::default()
        };
        assert_eq!(ids(&f, &ev), ["lunch", "review"]);
        let f = EventFilter {
            recurrence: vec![Recurrence::Recurring],
            ..Default::default()
        };
        assert_eq!(ids(&f, &ev), ["standup"]);
        let f = EventFilter {
            recurrence: vec![Recurrence::Single],
            ..Default::default()
        };
        assert_eq!(ids(&f, &ev), ["focus", "lunch", "review"]);
        let f = EventFilter {
            in_person: true,
            ..Default::default()
        };
        assert_eq!(
            ids(&f, &ev),
            ["lunch"],
            "the Teams placeholder is not a place"
        );
    }

    #[test]
    fn clear_resets() {
        let mut f = EventFilter {
            in_person: true,
            ..Default::default()
        };
        assert!(!f.is_empty());
        f.clear();
        assert!(f.is_empty());
    }
}
