//! The new-event form as data, and the Graph body it turns into (C2.1), plus the pure helpers
//! that prefill one: duplicate (C2.2) and "schedule a meeting" from a chat (C3.5).

use serde_json::{Value, json};
use teams_proto::{Error, Result};

use crate::event::{AttendeeKind, Event, ShowAs};
use crate::graph::{date_time, email_address, wall_utc};
use crate::range::{Date, Weekday};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Person {
    pub name: String,
    pub email: String,
}

impl Person {
    pub fn new(name: &str, email: &str) -> Person {
        Person {
            name: name.into(),
            email: email.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reminder {
    /// Leave the mailbox default.
    #[default]
    Default,
    Off,
    Minutes(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sensitivity {
    #[default]
    Normal,
    Personal,
    Private,
    Confidential,
}

impl Sensitivity {
    pub fn graph(self) -> &'static str {
        match self {
            Sensitivity::Normal => "normal",
            Sensitivity::Personal => "personal",
            Sensitivity::Private => "private",
            Sensitivity::Confidential => "confidential",
        }
    }
}

/// Which weekday of the month a relative pattern means ("the second Tuesday").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeekIndex {
    First,
    Second,
    Third,
    Fourth,
    Last,
}

impl WeekIndex {
    fn graph(self) -> &'static str {
        match self {
            WeekIndex::First => "first",
            WeekIndex::Second => "second",
            WeekIndex::Third => "third",
            WeekIndex::Fourth => "fourth",
            WeekIndex::Last => "last",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    Daily,
    /// Weekly on these days (Monday to Friday is `Pattern::weekdays()`).
    Weekly {
        days: Vec<Weekday>,
        first_day: Weekday,
    },
    /// On the Nth of every month.
    MonthlyOnDate {
        day_of_month: u32,
    },
    /// On the Nth weekday of every month.
    MonthlyOnWeekday {
        index: WeekIndex,
        day: Weekday,
    },
    YearlyOnDate {
        month: u32,
        day_of_month: u32,
    },
    YearlyOnWeekday {
        month: u32,
        index: WeekIndex,
        day: Weekday,
    },
}

impl Pattern {
    /// Every weekday, Monday to Friday.
    pub fn weekdays() -> Pattern {
        Pattern::Weekly {
            days: vec![
                Weekday::Monday,
                Weekday::Tuesday,
                Weekday::Wednesday,
                Weekday::Thursday,
                Weekday::Friday,
            ],
            first_day: Weekday::Sunday,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecurrenceEnd {
    Never,
    /// The last day an occurrence can fall on.
    On(Date),
    /// After this many occurrences.
    After(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recurrence {
    pub pattern: Pattern,
    /// Every N days, weeks, months or years; at least 1.
    pub interval: u32,
    pub end: RecurrenceEnd,
}

pub(crate) fn weekday_name(d: Weekday) -> &'static str {
    match d {
        Weekday::Sunday => "sunday",
        Weekday::Monday => "monday",
        Weekday::Tuesday => "tuesday",
        Weekday::Wednesday => "wednesday",
        Weekday::Thursday => "thursday",
        Weekday::Friday => "friday",
        Weekday::Saturday => "saturday",
    }
}

fn days_json(days: &[Weekday]) -> Value {
    days.iter().map(|d| weekday_name(*d)).collect()
}

impl Recurrence {
    /// Graph's `patternedRecurrence`. `start_date` is the event's first day in `tz`.
    fn to_json(&self, start_date: &str, tz: &str) -> Value {
        let mut p = json!({"interval": self.interval});
        match &self.pattern {
            Pattern::Daily => p["type"] = "daily".into(),
            Pattern::Weekly { days, first_day } => {
                p["type"] = "weekly".into();
                p["daysOfWeek"] = days_json(days);
                p["firstDayOfWeek"] = weekday_name(*first_day).into();
            }
            Pattern::MonthlyOnDate { day_of_month } => {
                p["type"] = "absoluteMonthly".into();
                p["dayOfMonth"] = (*day_of_month).into();
            }
            Pattern::MonthlyOnWeekday { index, day } => {
                p["type"] = "relativeMonthly".into();
                p["daysOfWeek"] = days_json(&[*day]);
                p["index"] = index.graph().into();
            }
            Pattern::YearlyOnDate {
                month,
                day_of_month,
            } => {
                p["type"] = "absoluteYearly".into();
                p["month"] = (*month).into();
                p["dayOfMonth"] = (*day_of_month).into();
            }
            Pattern::YearlyOnWeekday { month, index, day } => {
                p["type"] = "relativeYearly".into();
                p["month"] = (*month).into();
                p["daysOfWeek"] = days_json(&[*day]);
                p["index"] = index.graph().into();
            }
        }
        let mut range = json!({"startDate": start_date, "recurrenceTimeZone": tz});
        match self.end {
            RecurrenceEnd::Never => range["type"] = "noEnd".into(),
            RecurrenceEnd::On(d) => {
                range["type"] = "endDate".into();
                range["endDate"] = d.to_string().into();
            }
            RecurrenceEnd::After(n) => {
                range["type"] = "numbered".into();
                range["numberOfOccurrences"] = n.into();
            }
        }
        json!({"pattern": p, "range": range})
    }

    fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::Protocol(format!("recurrence: {m}")));
        if self.interval == 0 {
            return bad("interval must be at least 1");
        }
        let day_ok = |d: u32| (1..=31).contains(&d);
        let month_ok = |m: u32| (1..=12).contains(&m);
        match &self.pattern {
            Pattern::Weekly { days, .. } if days.is_empty() => return bad("no weekdays"),
            Pattern::MonthlyOnDate { day_of_month } if !day_ok(*day_of_month) => {
                return bad("day of month out of range");
            }
            Pattern::YearlyOnDate {
                month,
                day_of_month,
            } if !day_ok(*day_of_month) || !month_ok(*month) => {
                return bad("month or day out of range");
            }
            Pattern::YearlyOnWeekday { month, .. } if !month_ok(*month) => {
                return bad("month out of range");
            }
            _ => {}
        }
        if self.end == RecurrenceEnd::After(0) {
            return bad("zero occurrences");
        }
        Ok(())
    }
}

/// Everything the new-event form collects. Times are wall-clock strings
/// (`2026-12-18T10:00:00`) in `tz`, the way the form shows them; all-day events use midnight
/// and an end on the next midnight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEvent {
    pub subject: String,
    pub body_html: String,
    pub start: String,
    pub end: String,
    /// A Windows or IANA zone name, for example `Egypt Standard Time` or `Africa/Cairo`.
    pub tz: String,
    pub all_day: bool,
    pub required: Vec<Person>,
    pub optional: Vec<Person>,
    /// A free-text place; a chosen room is `room` and fills this when it is empty.
    pub location: String,
    pub room: Option<Person>,
    pub is_teams_meeting: bool,
    pub recurrence: Option<Recurrence>,
    pub reminder: Reminder,
    pub show_as: ShowAs,
    pub sensitivity: Sensitivity,
    pub categories: Vec<String>,
    pub response_requested: bool,
    pub allow_new_time_proposals: bool,
    pub hide_attendees: bool,
    /// Makes a retried create return the first event instead of a duplicate. Filled in by
    /// [`crate::create_event`] when `None`.
    pub transaction_id: Option<String>,
}

impl Default for NewEvent {
    fn default() -> Self {
        NewEvent {
            subject: String::new(),
            body_html: String::new(),
            start: String::new(),
            end: String::new(),
            tz: "UTC".into(),
            all_day: false,
            required: vec![],
            optional: vec![],
            location: String::new(),
            room: None,
            is_teams_meeting: false,
            recurrence: None,
            reminder: Reminder::Default,
            show_as: ShowAs::Busy,
            sensitivity: Sensitivity::Normal,
            categories: vec![],
            response_requested: true,
            allow_new_time_proposals: true,
            hide_attendees: false,
            transaction_id: None,
        }
    }
}

fn show_as_graph(s: ShowAs) -> Option<&'static str> {
    Some(match s {
        ShowAs::Free => "free",
        ShowAs::Tentative => "tentative",
        ShowAs::Busy => "busy",
        ShowAs::Oof => "oof",
        ShowAs::WorkingElsewhere => "workingElsewhere",
        ShowAs::Unknown => return None,
    })
}

impl NewEvent {
    /// Checks what Graph would reject, so the form can say so before sending.
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::Protocol(format!("event: {m}")));
        if self.start.len() < 19 || self.end.len() < 19 {
            return bad("start and end must look like 2026-12-18T10:00:00");
        }
        // Same zone and same format, so text order is time order.
        if self.end < self.start || (self.all_day && self.end <= self.start) {
            return bad("the end is before the start");
        }
        if let Some(r) = &self.recurrence {
            r.validate()?;
        }
        for p in self.required.iter().chain(&self.optional) {
            if !p.email.contains('@') {
                return bad("an attendee has no email address");
            }
        }
        Ok(())
    }

    fn body(&self, with_transaction: bool) -> Value {
        let attendee = |p: &Person, kind: &str| json!({"emailAddress": email_address(&p.name, &p.email), "type": kind});
        let mut attendees: Vec<Value> = self
            .required
            .iter()
            .map(|p| attendee(p, "required"))
            .collect();
        attendees.extend(self.optional.iter().map(|p| attendee(p, "optional")));
        let location = if self.location.is_empty() {
            self.room.as_ref().map_or("", |r| r.name.as_str())
        } else {
            self.location.as_str()
        };
        if let Some(r) = &self.room {
            attendees.push(attendee(r, "resource"));
        }
        let mut b = json!({
            "subject": self.subject,
            "body": {"contentType": "html", "content": self.body_html},
            "start": date_time(&self.start, &self.tz),
            "end": date_time(&self.end, &self.tz),
            "isAllDay": self.all_day,
            "sensitivity": self.sensitivity.graph(),
            "categories": self.categories,
            "attendees": attendees,
            "location": {"displayName": location},
            "isOnlineMeeting": self.is_teams_meeting,
            "responseRequested": self.response_requested,
            "allowNewTimeProposals": self.allow_new_time_proposals,
            "hideAttendees": self.hide_attendees,
        });
        if let Some(s) = show_as_graph(self.show_as) {
            b["showAs"] = s.into();
        }
        if self.is_teams_meeting {
            b["onlineMeetingProvider"] = "teamsForBusiness".into();
        }
        match self.reminder {
            Reminder::Default => {}
            Reminder::Off => b["isReminderOn"] = false.into(),
            Reminder::Minutes(m) => {
                b["isReminderOn"] = true.into();
                b["reminderMinutesBeforeStart"] = m.into();
            }
        }
        if let Some(r) = &self.recurrence {
            b["recurrence"] = r.to_json(&self.start[..10], &self.tz);
        }
        if with_transaction && let Some(t) = &self.transaction_id {
            b["transactionId"] = t.as_str().into();
        }
        b
    }

    /// The `POST /me/events` body, with `transactionId` when set.
    pub fn create_body(&self) -> Value {
        self.body(true)
    }

    /// The `PATCH /me/events/{id}` body: the same fields, no `transactionId`.
    pub fn update_body(&self) -> Value {
        self.body(false)
    }

    /// A copy of `event` as a new event: same title, time, place, people, show-as and
    /// categories, no transaction id. Series settings and the body are not on [`Event`]; set
    /// `recurrence` and `body_html` after, from the detail, when they matter.
    pub fn duplicate(event: &Event) -> NewEvent {
        let person = |a: &crate::event::Attendee| Person::new(&a.name, &a.email);
        let of_kind = |k: AttendeeKind| {
            event
                .attendees
                .iter()
                .filter(|a| a.kind == k)
                .map(person)
                .collect::<Vec<_>>()
        };
        NewEvent {
            subject: event.subject.clone(),
            start: wall_utc(event.start_ms),
            end: wall_utc(event.end_ms),
            tz: "UTC".into(),
            all_day: event.is_all_day,
            required: of_kind(AttendeeKind::Required),
            optional: of_kind(AttendeeKind::Optional),
            location: event.location.clone(),
            room: event
                .attendees
                .iter()
                .find(|a| a.kind == AttendeeKind::Resource)
                .map(person),
            is_teams_meeting: event.is_online_meeting,
            show_as: event.show_as,
            sensitivity: if event.sensitivity_private {
                Sensitivity::Private
            } else {
                Sensitivity::Normal
            },
            categories: event.categories.clone(),
            ..NewEvent::default()
        }
    }

    /// "Schedule a meeting" from a chat's header: the other members as required attendees and
    /// a Teams meeting on. Members without an email, and the signed-in user, are left out.
    pub fn from_chat_members(
        subject: &str,
        members: &[Person],
        my_email: &str,
        tz: &str,
    ) -> NewEvent {
        let mut seen = std::collections::BTreeSet::new();
        let required = members
            .iter()
            .filter(|m| m.email.contains('@') && !m.email.eq_ignore_ascii_case(my_email))
            .filter(|m| seen.insert(m.email.to_ascii_lowercase()))
            .cloned()
            .collect();
        NewEvent {
            subject: subject.into(),
            tz: tz.into(),
            required,
            is_teams_meeting: true,
            ..NewEvent::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> NewEvent {
        NewEvent {
            subject: "Design sync".into(),
            body_html: "<p>Agenda</p>".into(),
            start: "2026-12-18T10:00:00".into(),
            end: "2026-12-18T10:30:00".into(),
            tz: "Egypt Standard Time".into(),
            required: vec![Person::new("Sara Hassan", "sara@contoso.example")],
            optional: vec![Person::new("", "omar@contoso.example")],
            ..NewEvent::default()
        }
    }

    #[test]
    fn private_appointment_body() {
        let e = NewEvent {
            subject: "kelma test".into(),
            start: "2026-12-18T10:00:00".into(),
            end: "2026-12-18T10:30:00".into(),
            sensitivity: Sensitivity::Private,
            ..NewEvent::default()
        };
        let b = e.create_body();
        assert_eq!(b["isOnlineMeeting"], false);
        assert!(b.get("onlineMeetingProvider").is_none());
        assert_eq!(b["sensitivity"], "private");
        assert_eq!(b["attendees"], json!([]));
        assert_eq!(
            b["start"],
            json!({"dateTime": "2026-12-18T10:00:00", "timeZone": "UTC"})
        );
        assert!(b.get("transactionId").is_none());
        assert!(b.get("recurrence").is_none());
        assert!(b.get("isReminderOn").is_none(), "mailbox default kept");
    }

    #[test]
    fn meeting_body_with_teams_attendees_room_and_options() {
        let mut e = base();
        e.is_teams_meeting = true;
        e.room = Some(Person::new("Room 4B", "room4b@contoso.example"));
        e.reminder = Reminder::Minutes(15);
        e.show_as = ShowAs::Tentative;
        e.categories = vec!["Blue".into()];
        e.hide_attendees = true;
        e.allow_new_time_proposals = false;
        e.transaction_id = Some("tx-1".into());
        let b = e.create_body();
        assert_eq!(b["onlineMeetingProvider"], "teamsForBusiness");
        assert_eq!(b["isOnlineMeeting"], true);
        assert_eq!(b["transactionId"], "tx-1");
        assert_eq!(
            b["location"]["displayName"], "Room 4B",
            "room name fills an empty location"
        );
        assert_eq!(
            b["attendees"],
            json!([
                {"emailAddress": {"address": "sara@contoso.example", "name": "Sara Hassan"}, "type": "required"},
                {"emailAddress": {"address": "omar@contoso.example"}, "type": "optional"},
                {"emailAddress": {"address": "room4b@contoso.example", "name": "Room 4B"}, "type": "resource"},
            ])
        );
        assert_eq!(b["isReminderOn"], true);
        assert_eq!(b["reminderMinutesBeforeStart"], 15);
        assert_eq!(b["showAs"], "tentative");
        assert_eq!(b["categories"], json!(["Blue"]));
        assert_eq!(b["hideAttendees"], true);
        assert_eq!(b["allowNewTimeProposals"], false);
        assert_eq!(b["responseRequested"], true);
        assert!(e.update_body().get("transactionId").is_none());
        e.reminder = Reminder::Off;
        assert_eq!(e.create_body()["isReminderOn"], false);
    }

    #[test]
    fn explicit_location_wins_over_the_room_name() {
        let mut e = base();
        e.location = "Cafeteria".into();
        e.room = Some(Person::new("Room 4B", "room4b@contoso.example"));
        assert_eq!(e.create_body()["location"]["displayName"], "Cafeteria");
    }

    fn recurrence(pattern: Pattern, interval: u32, end: RecurrenceEnd) -> Value {
        let mut e = base();
        e.recurrence = Some(Recurrence {
            pattern,
            interval,
            end,
        });
        e.create_body()["recurrence"].clone()
    }

    #[test]
    fn daily_and_weekdays() {
        let r = recurrence(Pattern::Daily, 2, RecurrenceEnd::Never);
        assert_eq!(
            r,
            json!({"pattern": {"type": "daily", "interval": 2},
                   "range": {"type": "noEnd", "startDate": "2026-12-18",
                             "recurrenceTimeZone": "Egypt Standard Time"}})
        );
        let r = recurrence(Pattern::weekdays(), 1, RecurrenceEnd::After(10));
        assert_eq!(r["pattern"]["type"], "weekly");
        assert_eq!(
            r["pattern"]["daysOfWeek"],
            json!(["monday", "tuesday", "wednesday", "thursday", "friday"])
        );
        assert_eq!(r["pattern"]["firstDayOfWeek"], "sunday");
        assert_eq!(r["range"]["type"], "numbered");
        assert_eq!(r["range"]["numberOfOccurrences"], 10);
    }

    #[test]
    fn monthly_and_yearly_patterns() {
        let r = recurrence(
            Pattern::MonthlyOnDate { day_of_month: 15 },
            3,
            RecurrenceEnd::On(Date::new(2027, 6, 30)),
        );
        assert_eq!(r["pattern"]["type"], "absoluteMonthly");
        assert_eq!(r["pattern"]["dayOfMonth"], 15);
        assert_eq!(r["pattern"]["interval"], 3);
        assert_eq!(
            r["range"],
            json!({"type": "endDate", "startDate": "2026-12-18",
            "endDate": "2027-06-30", "recurrenceTimeZone": "Egypt Standard Time"})
        );

        let r = recurrence(
            Pattern::MonthlyOnWeekday {
                index: WeekIndex::Second,
                day: Weekday::Tuesday,
            },
            1,
            RecurrenceEnd::Never,
        );
        assert_eq!(r["pattern"]["type"], "relativeMonthly");
        assert_eq!(r["pattern"]["index"], "second");
        assert_eq!(r["pattern"]["daysOfWeek"], json!(["tuesday"]));

        let r = recurrence(
            Pattern::YearlyOnDate {
                month: 3,
                day_of_month: 8,
            },
            1,
            RecurrenceEnd::Never,
        );
        assert_eq!(r["pattern"]["type"], "absoluteYearly");
        assert_eq!(
            (
                r["pattern"]["month"].clone(),
                r["pattern"]["dayOfMonth"].clone()
            ),
            (3.into(), 8.into())
        );

        let r = recurrence(
            Pattern::YearlyOnWeekday {
                month: 11,
                index: WeekIndex::Last,
                day: Weekday::Friday,
            },
            1,
            RecurrenceEnd::Never,
        );
        assert_eq!(r["pattern"]["type"], "relativeYearly");
        assert_eq!(r["pattern"]["index"], "last");
    }

    #[test]
    fn validation_catches_what_graph_would_refuse() {
        assert!(base().validate().is_ok());
        let mut e = base();
        e.end = "2026-12-18T09:00:00".into();
        assert!(e.validate().is_err(), "ends before it starts");
        let mut e = base();
        e.all_day = true;
        e.end = e.start.clone();
        assert!(e.validate().is_err(), "all-day needs a next-day end");
        let mut e = base();
        e.required.push(Person::new("x", "not-an-email"));
        assert!(e.validate().is_err());
        for bad in [
            Recurrence {
                pattern: Pattern::Daily,
                interval: 0,
                end: RecurrenceEnd::Never,
            },
            Recurrence {
                pattern: Pattern::Weekly {
                    days: vec![],
                    first_day: Weekday::Monday,
                },
                interval: 1,
                end: RecurrenceEnd::Never,
            },
            Recurrence {
                pattern: Pattern::MonthlyOnDate { day_of_month: 32 },
                interval: 1,
                end: RecurrenceEnd::Never,
            },
            Recurrence {
                pattern: Pattern::Daily,
                interval: 1,
                end: RecurrenceEnd::After(0),
            },
        ] {
            let mut e = base();
            e.recurrence = Some(bad);
            assert!(e.validate().is_err());
        }
    }

    fn parsed(item: Value) -> Event {
        crate::parse_events(&json!({"value": [item]})).remove(0)
    }

    #[test]
    fn duplicate_copies_the_event_without_ids() {
        let ev = parsed(json!({
        "id": "AAA", "subject": "Planning",
        "start": {"dateTime": "2026-12-18T09:00:00.0000000", "timeZone": "UTC"},
        "end": {"dateTime": "2026-12-18T10:00:00.0000000", "timeZone": "UTC"},
        "showAs": "tentative", "sensitivity": "private", "categories": ["Red"],
        "location": {"displayName": "Room 4B"}, "isOnlineMeeting": true,
        "attendees": [
            {"emailAddress": {"name": "Sara", "address": "sara@contoso.example"}, "type": "required"},
            {"emailAddress": {"name": "Omar", "address": "omar@contoso.example"}, "type": "optional"},
            {"emailAddress": {"name": "Room 4B", "address": "r@contoso.example"}, "type": "resource"}
        ]}));
        let d = NewEvent::duplicate(&ev);
        assert_eq!(d.subject, "Planning");
        assert_eq!(
            (d.start.as_str(), d.end.as_str()),
            ("2026-12-18T09:00:00", "2026-12-18T10:00:00")
        );
        assert_eq!(d.required, [Person::new("Sara", "sara@contoso.example")]);
        assert_eq!(d.optional, [Person::new("Omar", "omar@contoso.example")]);
        assert_eq!(d.room, Some(Person::new("Room 4B", "r@contoso.example")));
        assert!(d.is_teams_meeting);
        assert_eq!(d.sensitivity, Sensitivity::Private);
        assert_eq!(d.show_as, ShowAs::Tentative);
        assert_eq!(d.categories, ["Red"]);
        assert!(d.transaction_id.is_none());
        assert!(d.validate().is_ok());
    }

    #[test]
    fn prefill_from_chat_members() {
        let members = [
            Person::new("Me", "Me@contoso.example"),
            Person::new("Sara", "sara@contoso.example"),
            Person::new("Sara again", "SARA@contoso.example"),
            Person::new("Bot", ""),
            Person::new("Omar", "omar@contoso.example"),
        ];
        let e =
            NewEvent::from_chat_members("Chat with Sara", &members, "me@contoso.example", "UTC");
        assert_eq!(e.subject, "Chat with Sara");
        assert_eq!(
            e.required
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["Sara", "Omar"]
        );
        assert!(e.is_teams_meeting);
    }
}
