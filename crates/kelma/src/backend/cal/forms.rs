//! The new-event form as plain strings (what the Slint fields hold), and how it becomes a
//! `NewEvent`. Pure, so the validation and the recurrence picker are tested without a window.

use super::grid::Offset;
use ::calendar::{
    Date, Event, NewEvent, Pattern, Person, Recurrence, RecurrenceEnd, Reminder, Sensitivity,
    ShowAs, WeekIndex, Weekday, format_iso_ms,
};

/// The form's fields. Dates are `2026-12-18`, times `09:30` (24 hours), both local.
#[derive(Debug, Clone, PartialEq)]
pub struct FormFields {
    pub subject: String,
    pub start_date: String,
    pub start_time: String,
    pub end_date: String,
    pub end_time: String,
    pub all_day: bool,
    /// Addresses separated by `,` `;` or new lines; `Name <a@b.com>` works too.
    pub required: String,
    pub optional: String,
    pub location: String,
    pub room_name: String,
    pub room_email: String,
    pub teams_meeting: bool,
    /// One of [`REPEAT_KINDS`]; `keep` on an edited series leaves its pattern alone.
    pub repeat: String,
    pub interval: String,
    /// Weekly: the chosen days, Sunday first.
    pub repeat_days: Vec<bool>,
    /// `never`, `on` or `after`.
    pub end_kind: String,
    /// A date for `on`, a count for `after`.
    pub end_value: String,
    /// Plain text with the chat formatter's markdown.
    pub body: String,
    /// `default`, `off` or minutes before the start.
    pub reminder: String,
    /// `busy`, `tentative`, `free`, `oof` or `elsewhere`.
    pub show_as: String,
    pub private: bool,
    pub category: String,
    pub response_requested: bool,
    pub allow_proposals: bool,
    pub hide_attendees: bool,
    /// Channel meetings: where it is held, `Team › Channel`.
    pub channel: String,
}

impl FormFields {
    /// A blank form for a 30 minute slot starting at `start_min` minutes after midnight.
    pub fn blank(date: Date, start_min: u32) -> FormFields {
        let end_min = start_min + 30;
        let (end_date, end_min) = if end_min >= 24 * 60 {
            (date.add_days(1), end_min - 24 * 60)
        } else {
            (date, end_min)
        };
        FormFields {
            subject: String::new(),
            start_date: date.to_string(),
            start_time: shown(start_min),
            end_date: end_date.to_string(),
            end_time: shown(end_min),
            all_day: false,
            required: String::new(),
            optional: String::new(),
            location: String::new(),
            room_name: String::new(),
            room_email: String::new(),
            teams_meeting: false,
            repeat: "none".into(),
            interval: "1".into(),
            repeat_days: vec![false; 7],
            end_kind: "never".into(),
            end_value: String::new(),
            body: String::new(),
            reminder: "default".into(),
            show_as: "busy".into(),
            private: false,
            category: String::new(),
            response_requested: true,
            allow_proposals: true,
            hide_attendees: false,
            channel: String::new(),
        }
    }

    /// The form for editing or duplicating `e`, with times in local time (`off` is the local
    /// UTC offset at an instant, in milliseconds). `body` is the event's text.
    pub fn from_event(e: &Event, body: &str, off: &dyn Fn(i64) -> i64, series: bool) -> FormFields {
        let wall = |ms: i64| {
            let s = format_iso_ms(ms + off(ms));
            (
                s[..10].to_string(),
                shown(parse_hm(&s[11..16]).unwrap_or(0)),
            )
        };
        let (sd, st) = wall(e.start_ms);
        let (mut ed, mut et) = wall(e.end_ms);
        if e.is_all_day {
            // The form shows the last day, Graph stores the next midnight.
            let last = Date::from_ms(e.end_ms + off(e.end_ms) - 1);
            ed = last.to_string();
            et = shown(0);
        }
        let join = |kind: ::calendar::AttendeeKind| {
            e.attendees
                .iter()
                .filter(|a| a.kind == kind)
                .map(|a| {
                    if a.name.is_empty() || a.name == a.email {
                        a.email.clone()
                    } else {
                        format!("{} <{}>", a.name, a.email)
                    }
                })
                .collect::<Vec<_>>()
                .join("; ")
        };
        let room = e
            .attendees
            .iter()
            .find(|a| a.kind == ::calendar::AttendeeKind::Resource);
        let mut f = FormFields::blank(Date::new(2000, 1, 1), 0);
        f.subject = e.subject.clone();
        f.start_date = sd;
        f.start_time = st;
        f.end_date = ed;
        f.end_time = et;
        f.all_day = e.is_all_day;
        f.required = join(::calendar::AttendeeKind::Required);
        f.optional = join(::calendar::AttendeeKind::Optional);
        f.location = e.location.clone();
        f.room_name = room.map(|r| r.name.clone()).unwrap_or_default();
        f.room_email = room.map(|r| r.email.clone()).unwrap_or_default();
        f.teams_meeting = e.is_online_meeting;
        f.repeat = if series { "keep" } else { "none" }.into();
        f.body = body.into();
        f.show_as = show_as_key(e.show_as).into();
        f.private = e.sensitivity_private;
        f.category = e.categories.first().cloned().unwrap_or_default();
        f.reminder = match e.reminder_minutes {
            Some(m) => m.to_string(),
            None => "off".into(),
        };
        f
    }

    /// Validates the form and builds the event. `tz` is the zone the wall-clock times are in.
    /// `original_body` is `(plain, html)` of an edited event: when the text was not touched the
    /// original HTML is sent back, so editing a title never flattens a rich body.
    pub fn to_new_event(
        &self,
        tz: &str,
        original_body: Option<(&str, &str)>,
    ) -> Result<NewEvent, String> {
        if self.subject.trim().is_empty() {
            return Err("Add a title".into());
        }
        let start_date =
            parse_date(&self.start_date).ok_or("The start date should look like 2026-12-18")?;
        let end_date =
            parse_date(&self.end_date).ok_or("The end date should look like 2026-12-18")?;
        let (start, end) = if self.all_day {
            if end_date < start_date {
                return Err("The end is before the start".into());
            }
            (
                format!("{start_date}T00:00:00"),
                format!("{}T00:00:00", end_date.add_days(1)),
            )
        } else {
            let st = parse_hm(&self.start_time).ok_or("The start time should look like 9:30 AM")?;
            let et = parse_hm(&self.end_time).ok_or("The end time should look like 10:30 AM")?;
            (
                format!("{start_date}T{}:00", hm(st)),
                format!("{end_date}T{}:00", hm(et)),
            )
        };
        if end < start {
            return Err("The end is before the start".into());
        }
        let required = parse_people(&self.required)?;
        let optional = parse_people(&self.optional)?;
        let room = (!self.room_email.trim().is_empty())
            .then(|| Person::new(self.room_name.trim(), self.room_email.trim()));
        let recurrence = self.recurrence(start_date)?;
        let reminder = match self.reminder.as_str() {
            "off" => Reminder::Off,
            "" | "default" => Reminder::Default,
            m => Reminder::Minutes(
                m.parse()
                    .map_err(|_| "The reminder is not a number of minutes")?,
            ),
        };
        let body_html = match original_body {
            Some((plain, html)) if plain == self.body => html.to_string(),
            _ => ::render::markdown_to_teams_html(&self.body),
        };
        let mut location = self.location.trim().to_string();
        if location.is_empty() && !self.channel.is_empty() {
            location = self.channel.clone();
        }
        let ev = NewEvent {
            subject: self.subject.trim().into(),
            body_html,
            start,
            end,
            tz: tz.into(),
            all_day: self.all_day,
            required,
            optional,
            location,
            room,
            is_teams_meeting: self.teams_meeting,
            recurrence,
            reminder,
            show_as: show_as(&self.show_as),
            sensitivity: if self.private {
                Sensitivity::Private
            } else {
                Sensitivity::Normal
            },
            categories: if self.category.trim().is_empty() {
                vec![]
            } else {
                vec![self.category.trim().into()]
            },
            response_requested: self.response_requested,
            allow_new_time_proposals: self.allow_proposals,
            hide_attendees: self.hide_attendees,
            transaction_id: None,
        };
        ev.validate().map_err(|e| {
            e.to_string()
                .replace("protocol: ", "")
                .replace("event: ", "")
        })?;
        Ok(ev)
    }

    fn recurrence(&self, start: Date) -> Result<Option<Recurrence>, String> {
        let pattern = match self.repeat.as_str() {
            "" | "none" | "keep" => return Ok(None),
            "daily" => Pattern::Daily,
            "weekdays" => Pattern::weekdays(),
            "weekly" => {
                let mut days: Vec<Weekday> = (0..7)
                    .filter(|&i| self.repeat_days.get(i).copied().unwrap_or(false))
                    .map(weekday_of)
                    .collect();
                if days.is_empty() {
                    days.push(start.weekday());
                }
                Pattern::Weekly {
                    days,
                    first_day: Weekday::Sunday,
                }
            }
            "monthly" => Pattern::MonthlyOnDate {
                day_of_month: start.day,
            },
            "monthly-nth" => Pattern::MonthlyOnWeekday {
                index: week_index(start.day),
                day: start.weekday(),
            },
            "yearly" => Pattern::YearlyOnDate {
                month: start.month,
                day_of_month: start.day,
            },
            other => return Err(format!("Unknown repeat choice {other}")),
        };
        let interval = match self.interval.trim() {
            "" => 1,
            s => s
                .parse::<u32>()
                .ok()
                .filter(|n| *n >= 1)
                .ok_or("Repeat every needs a number of at least 1")?,
        };
        let end = match self.end_kind.as_str() {
            "on" => {
                let d = parse_date(&self.end_value)
                    .ok_or("The series end date should look like 2026-12-18")?;
                if d < start {
                    return Err("The series ends before it starts".into());
                }
                RecurrenceEnd::On(d)
            }
            "after" => RecurrenceEnd::After(
                self.end_value
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|n| *n >= 1)
                    .ok_or("After needs a number of occurrences of at least 1")?,
            ),
            _ => RecurrenceEnd::Never,
        };
        Ok(Some(Recurrence {
            pattern,
            interval,
            end,
        }))
    }
}

fn weekday_of(i: usize) -> Weekday {
    [
        Weekday::Sunday,
        Weekday::Monday,
        Weekday::Tuesday,
        Weekday::Wednesday,
        Weekday::Thursday,
        Weekday::Friday,
        Weekday::Saturday,
    ][i % 7]
}

/// Which occurrence of its weekday a day of the month is; the fifth means "last".
fn week_index(day_of_month: u32) -> WeekIndex {
    match (day_of_month - 1) / 7 {
        0 => WeekIndex::First,
        1 => WeekIndex::Second,
        2 => WeekIndex::Third,
        3 => WeekIndex::Fourth,
        _ => WeekIndex::Last,
    }
}

/// `09:30`: the 24-hour form the service takes.
pub fn hm(min: u32) -> String {
    format!("{:02}:{:02}", min / 60, min % 60)
}

/// What the form shows for `min` minutes after midnight: "9:30 AM", or "09:30" on the 24-hour
/// clock (Settings).
pub fn shown(min: u32) -> String {
    appearance::formats::clock(min as i32)
}

/// A typed time as minutes after midnight: `09:30`, `9:30 pm`, `2130`, `٩:٣٠` (see
/// `send_options::parse_time`).
pub fn parse_hm(s: &str) -> Option<u32> {
    crate::send_options::parse_time(s).map(|m| m as u32)
}

/// `2026-12-18`, with a real calendar date.
pub fn parse_date(s: &str) -> Option<Date> {
    let mut it = s.trim().splitn(3, '-');
    let (y, m, d) = (it.next()?, it.next()?, it.next()?);
    let (y, m, d) = (
        y.parse::<i32>().ok()?,
        m.parse::<u32>().ok()?,
        d.parse::<u32>().ok()?,
    );
    if y.to_string().len() != 4 || !(1..=12).contains(&m) || d == 0 {
        return None;
    }
    let date = Date::new(y, m, d);
    // Rejects 2026-02-30: the date must survive a round trip through the day count.
    (Date::from_ms(date.midnight_ms()) == date).then_some(date)
}

pub fn parse_people(s: &str) -> Result<Vec<Person>, String> {
    let mut out: Vec<Person> = Vec::new();
    for raw in s.split([',', ';', '\n']) {
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        let (name, email) = match (t.find('<'), t.rfind('>')) {
            (Some(a), Some(b)) if a < b => (t[..a].trim(), t[a + 1..b].trim()),
            _ => ("", t),
        };
        let ok = email.split_once('@').is_some_and(|(l, d)| {
            !l.is_empty() && d.contains('.') && !email.contains(char::is_whitespace)
        });
        if !ok {
            return Err(format!("\"{t}\" is not an email address"));
        }
        if !out.iter().any(|p| p.email.eq_ignore_ascii_case(email)) {
            out.push(Person::new(name, email));
        }
    }
    Ok(out)
}

fn show_as(s: &str) -> ShowAs {
    match s {
        "free" => ShowAs::Free,
        "tentative" => ShowAs::Tentative,
        "oof" => ShowAs::Oof,
        "elsewhere" => ShowAs::WorkingElsewhere,
        _ => ShowAs::Busy,
    }
}

pub fn show_as_key(s: ShowAs) -> &'static str {
    match s {
        ShowAs::Free => "free",
        ShowAs::Tentative => "tentative",
        ShowAs::Oof => "oof",
        ShowAs::WorkingElsewhere => "elsewhere",
        ShowAs::Busy | ShowAs::Unknown => "busy",
    }
}

/// The Graph `showAs` value for a form key.
pub fn show_as_graph(key: &str) -> &'static str {
    match key {
        "free" => "free",
        "tentative" => "tentative",
        "oof" => "oof",
        "elsewhere" => "workingElsewhere",
        _ => "busy",
    }
}

/// Everything a chat header's "Schedule a meeting" fills in: the people and a Teams meeting.
pub fn from_chat(
    subject: &str,
    members: &[Person],
    my_email: &str,
    date: Date,
    start_min: u32,
    tz: &str,
) -> FormFields {
    let ev = NewEvent::from_chat_members(subject, members, my_email, tz);
    let mut f = FormFields::blank(date, start_min);
    f.subject = ev.subject;
    f.required = ev
        .required
        .iter()
        .map(|p| {
            if p.name.is_empty() {
                p.email.clone()
            } else {
                format!("{} <{}>", p.name, p.email)
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    f.teams_meeting = ev.is_teams_meeting;
    f
}

/// The people in `s` that look like addresses, leaving out what does not (the Scheduler
/// asks about whoever is typed so far).
pub fn people_lenient(s: &str) -> Vec<Person> {
    s.split([',', ';', '\n'])
        .filter_map(|t| parse_people(t).ok()?.into_iter().next())
        .fold(Vec::new(), |mut v: Vec<Person>, p| {
            if !v.iter().any(|x| x.email.eq_ignore_ascii_case(&p.email)) {
                v.push(p);
            }
            v
        })
}

/// Unix milliseconds of a local `date` (`2026-12-18`) and `time` (`09:30`); `off` is the local
/// UTC offset at an instant.
pub fn local_ms(date: &str, time: &str, off: Offset) -> Option<i64> {
    let d = parse_date(date)?;
    let m = parse_hm(time)?;
    Some(super::grid::local_midnight_ms(d, off) + i64::from(m) * 60_000)
}

/// The length of the form's event in minutes, 30 when the times are not usable yet.
pub fn slot_minutes(f: &FormFields) -> u32 {
    match (
        parse_date(&f.start_date),
        parse_date(&f.end_date),
        parse_hm(&f.start_time),
        parse_hm(&f.end_time),
    ) {
        (Some(sd), Some(ed), Some(st), Some(et)) => {
            let days = (ed.midnight_ms() - sd.midnight_ms()) / 86_400_000;
            let l = days * 1440 + i64::from(et) - i64::from(st);
            if l > 0 { l as u32 } else { 30 }
        }
        _ => 30,
    }
}

/// After a click on the Scheduler at `pick_min` minutes after midnight: the new start time and
/// the end date and time that keep the event's length (30 minutes when the form has none yet).
pub fn slot_for_pick(f: &FormFields, pick_min: u32) -> (String, String, String) {
    let start_date = parse_date(&f.start_date).unwrap_or(Date::new(2000, 1, 1));
    let end = pick_min + slot_minutes(f);
    let end_date = start_date.add_days(i64::from(end / 1440));
    (shown(pick_min), end_date.to_string(), shown(end % 1440))
}

/// The IANA name of the time zone chosen in Settings, else the machine's; `UTC` when it cannot
/// be read.
pub fn local_tz() -> String {
    appearance::formats::time_zone()
        .map(|z| z.name().to_string())
        .or_else(|| iana_time_zone::get_timezone().ok())
        .unwrap_or_else(|| "UTC".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::calendar::parse_events;
    use serde_json::json;

    const DEC_18: Date = Date {
        year: 2026,
        month: 12,
        day: 18,
    }; // a Friday

    fn form() -> FormFields {
        let mut f = FormFields::blank(DEC_18, 10 * 60);
        f.subject = "Design sync".into();
        f
    }

    fn build(f: &FormFields) -> NewEvent {
        f.to_new_event("Africa/Cairo", None).unwrap()
    }

    #[test]
    fn a_blank_form_is_half_an_hour_with_sensible_defaults() {
        let e = build(&form());
        assert_eq!(e.start, "2026-12-18T10:00:00");
        assert_eq!(e.end, "2026-12-18T10:30:00");
        assert_eq!(e.tz, "Africa/Cairo");
        assert_eq!(e.show_as, ShowAs::Busy);
        assert!(e.response_requested && e.allow_new_time_proposals && !e.hide_attendees);
        assert!(e.recurrence.is_none() && !e.is_teams_meeting);
        // A slot that starts in the last half hour ends after midnight.
        let late = FormFields::blank(DEC_18, 23 * 60 + 45);
        assert_eq!(
            (late.end_date.as_str(), late.end_time.as_str()),
            ("2026-12-19", shown(15).as_str())
        );
    }

    #[test]
    fn the_form_says_what_is_wrong_before_anything_is_sent() {
        let err = |f: &FormFields| f.to_new_event("UTC", None).unwrap_err();
        let mut f = form();
        f.subject = "  ".into();
        assert_eq!(err(&f), "Add a title");
        let mut f = form();
        f.start_time = "25:00".into();
        assert!(err(&f).contains("start time"));
        let mut f = form();
        f.end_date = "2026-02-30".into();
        assert!(err(&f).contains("end date"));
        let mut f = form();
        f.end_time = "09:00".into();
        assert_eq!(err(&f), "The end is before the start");
        let mut f = form();
        f.required = "alice@contoso.com; nobody".into();
        assert_eq!(err(&f), "\"nobody\" is not an email address");
        let mut f = form();
        f.reminder = "soon".into();
        assert!(err(&f).contains("reminder"));
    }

    #[test]
    fn attendees_accept_names_dedupe_and_split_on_commas_semicolons_and_lines() {
        let mut f = form();
        f.required = "alice@contoso.com; Bob Lee <bob@contoso.com>,\nALICE@contoso.com".into();
        f.optional = "carol@contoso.com".into();
        let e = build(&f);
        assert_eq!(
            e.required,
            [
                Person::new("", "alice@contoso.com"),
                Person::new("Bob Lee", "bob@contoso.com")
            ]
        );
        assert_eq!(e.optional, [Person::new("", "carol@contoso.com")]);
    }

    #[test]
    fn all_day_events_end_on_the_next_midnight() {
        let mut f = form();
        f.all_day = true;
        f.end_date = "2026-12-20".into();
        let e = build(&f);
        assert_eq!(
            (e.start.as_str(), e.end.as_str()),
            ("2026-12-18T00:00:00", "2026-12-21T00:00:00")
        );
        f.end_date = "2026-12-17".into();
        assert_eq!(
            f.to_new_event("UTC", None).unwrap_err(),
            "The end is before the start"
        );
    }

    #[test]
    fn the_recurrence_picker_maps_to_patterns() {
        let rec = |f: &FormFields| build(f).recurrence;
        let mut f = form();
        assert_eq!(rec(&f), None, "Never repeat");
        f.repeat = "daily".into();
        assert_eq!(
            rec(&f),
            Some(Recurrence {
                pattern: Pattern::Daily,
                interval: 1,
                end: RecurrenceEnd::Never
            })
        );
        f.repeat = "weekdays".into();
        assert_eq!(rec(&f).unwrap().pattern, Pattern::weekdays());
        // Weekly with no day ticked repeats on the start's weekday (Friday).
        f.repeat = "weekly".into();
        f.interval = "2".into();
        let r = rec(&f).unwrap();
        assert_eq!(r.interval, 2);
        assert_eq!(
            r.pattern,
            Pattern::Weekly {
                days: vec![Weekday::Friday],
                first_day: Weekday::Sunday
            }
        );
        f.repeat_days = vec![false, true, false, true, false, false, false];
        assert_eq!(
            rec(&f).unwrap().pattern,
            Pattern::Weekly {
                days: vec![Weekday::Monday, Weekday::Wednesday],
                first_day: Weekday::Sunday
            }
        );
        // 2026-12-18 is the third Friday of the month.
        f.repeat = "monthly-nth".into();
        assert_eq!(
            rec(&f).unwrap().pattern,
            Pattern::MonthlyOnWeekday {
                index: WeekIndex::Third,
                day: Weekday::Friday
            }
        );
        f.repeat = "monthly".into();
        assert_eq!(
            rec(&f).unwrap().pattern,
            Pattern::MonthlyOnDate { day_of_month: 18 }
        );
        f.repeat = "yearly".into();
        assert_eq!(
            rec(&f).unwrap().pattern,
            Pattern::YearlyOnDate {
                month: 12,
                day_of_month: 18
            }
        );
    }

    #[test]
    fn the_series_end_is_never_on_a_date_or_after_a_count() {
        let mut f = form();
        f.repeat = "daily".into();
        f.end_kind = "after".into();
        f.end_value = "10".into();
        assert_eq!(build(&f).recurrence.unwrap().end, RecurrenceEnd::After(10));
        f.end_value = "0".into();
        assert!(f.to_new_event("UTC", None).is_err());
        f.end_kind = "on".into();
        f.end_value = "2027-01-31".into();
        assert_eq!(
            build(&f).recurrence.unwrap().end,
            RecurrenceEnd::On(Date::new(2027, 1, 31))
        );
        f.end_value = "2026-12-01".into();
        assert_eq!(
            f.to_new_event("UTC", None).unwrap_err(),
            "The series ends before it starts"
        );
        f.interval = "0".into();
        f.end_value = "2027-01-31".into();
        assert!(
            f.to_new_event("UTC", None)
                .unwrap_err()
                .contains("at least 1")
        );
    }

    #[test]
    fn the_fifth_weekday_of_a_month_means_last() {
        let mut f = FormFields::blank(Date::new(2026, 12, 31), 600); // the fifth Thursday
        f.subject = "x".into();
        f.repeat = "monthly-nth".into();
        assert_eq!(
            build(&f).recurrence.unwrap().pattern,
            Pattern::MonthlyOnWeekday {
                index: WeekIndex::Last,
                day: Weekday::Thursday
            }
        );
    }

    #[test]
    fn options_room_category_and_body_reach_the_event() {
        let mut f = form();
        f.teams_meeting = true;
        f.private = true;
        f.show_as = "oof".into();
        f.category = "Blue category".into();
        f.reminder = "30".into();
        f.room_name = "Atlas".into();
        f.room_email = "atlas@contoso.com".into();
        f.hide_attendees = true;
        f.allow_proposals = false;
        f.response_requested = false;
        f.body = "**Agenda**".into();
        let e = build(&f);
        assert!(e.is_teams_meeting && e.hide_attendees);
        assert!(!e.allow_new_time_proposals && !e.response_requested);
        assert_eq!(
            (e.show_as, e.sensitivity),
            (ShowAs::Oof, Sensitivity::Private)
        );
        assert_eq!(e.categories, ["Blue category"]);
        assert_eq!(e.reminder, Reminder::Minutes(30));
        assert_eq!(e.room, Some(Person::new("Atlas", "atlas@contoso.com")));
        assert!(e.body_html.contains("Agenda"), "{}", e.body_html);
        f.reminder = "off".into();
        assert_eq!(build(&f).reminder, Reminder::Off);
    }

    #[test]
    fn an_untouched_body_keeps_its_original_html() {
        let mut f = form();
        f.body = "Agenda".into();
        let e = f
            .to_new_event("UTC", Some(("Agenda", "<table>rich</table>")))
            .unwrap();
        assert_eq!(e.body_html, "<table>rich</table>");
        f.body = "Agenda 2".into();
        let e = f
            .to_new_event("UTC", Some(("Agenda", "<table>rich</table>")))
            .unwrap();
        assert!(!e.body_html.contains("table"));
    }

    #[test]
    fn editing_fills_the_form_in_local_time_and_keeps_series_and_people() {
        let evs = parse_events(&json!({"value": [{
            "id": "E1", "subject": "Planning",
            "start": {"dateTime": "2026-12-18T08:00:00.0000000", "timeZone": "UTC"},
            "end": {"dateTime": "2026-12-18T09:30:00.0000000", "timeZone": "UTC"},
            "location": {"displayName": "Room 4"},
            "isOnlineMeeting": true,
            "showAs": "tentative",
            "categories": ["Red category"],
            "attendees": [
                {"emailAddress": {"name": "Alice", "address": "alice@contoso.com"}, "type": "required", "status": {"response": "accepted"}},
                {"emailAddress": {"name": "bob@contoso.com", "address": "bob@contoso.com"}, "type": "optional", "status": {"response": "none"}},
                {"emailAddress": {"name": "Atlas", "address": "atlas@contoso.com"}, "type": "resource", "status": {"response": "accepted"}}
            ]
        }]}));
        let two_hours = |_: i64| 2 * 3_600_000;
        let f = FormFields::from_event(&evs[0], "Notes", &two_hours, true);
        assert_eq!(
            (
                f.start_date.as_str(),
                f.start_time.as_str(),
                f.end_time.as_str()
            ),
            (
                "2026-12-18",
                shown(10 * 60).as_str(),
                shown(11 * 60 + 30).as_str()
            )
        );
        assert_eq!(f.required, "Alice <alice@contoso.com>");
        assert_eq!(f.optional, "bob@contoso.com");
        assert_eq!(
            (f.room_email.as_str(), f.location.as_str()),
            ("atlas@contoso.com", "Room 4")
        );
        assert_eq!(
            (f.show_as.as_str(), f.category.as_str(), f.repeat.as_str()),
            ("tentative", "Red category", "keep")
        );
        assert!(f.teams_meeting);
        // Saving it back is a valid event with no recurrence change.
        let e = f.to_new_event("Africa/Cairo", None).unwrap();
        assert_eq!(
            (e.start.as_str(), e.end.as_str()),
            ("2026-12-18T10:00:00", "2026-12-18T11:30:00")
        );
        assert_eq!(e.recurrence, None);
        assert_eq!(e.required.len(), 1);
    }

    #[test]
    fn schedule_a_meeting_from_a_chat_prefills_the_people_without_me() {
        let members = [
            Person::new("Me", "me@contoso.com"),
            Person::new("Alice", "alice@contoso.com"),
            Person::new("Ghost", ""),
        ];
        let f = from_chat("Planning", &members, "ME@contoso.com", DEC_18, 540, "UTC");
        assert_eq!(f.subject, "Planning");
        assert_eq!(f.required, "Alice <alice@contoso.com>");
        assert!(f.teams_meeting);
        assert_eq!(f.start_time, shown(540));
    }

    #[test]
    fn dates_and_times_are_parsed_strictly() {
        assert_eq!(parse_hm("9:05"), Some(545));
        assert_eq!(parse_hm("24:00"), None);
        assert_eq!(parse_hm("09:5x"), None);
        assert_eq!(parse_date("2026-12-18"), Some(DEC_18));
        assert_eq!(parse_date("2026-2-3"), Some(Date::new(2026, 2, 3)));
        assert_eq!(parse_date("2026-02-30"), None);
        assert_eq!(parse_date("26-12-18"), None);
    }

    #[test]
    fn the_scheduler_click_keeps_the_events_length_and_can_cross_midnight() {
        let mut f = form();
        f.end_time = "11:30".into(); // 10:00 to 11:30, an hour and a half
        assert_eq!(
            slot_for_pick(&f, 14 * 60),
            (shown(14 * 60), "2026-12-18".into(), shown(15 * 60 + 30))
        );
        assert_eq!(
            slot_for_pick(&f, 23 * 60),
            (shown(23 * 60), "2026-12-19".into(), shown(30))
        );
        f.end_time = "08:00".into(); // nonsense: fall back to half an hour
        assert_eq!(
            slot_for_pick(&f, 9 * 60),
            (shown(9 * 60), "2026-12-18".into(), shown(9 * 60 + 30))
        );
    }

    #[test]
    fn typed_attendees_are_read_leniently_for_the_scheduler() {
        let p =
            people_lenient("alice@contoso.com; nobody, Bob <bob@contoso.com>, ALICE@contoso.com");
        assert_eq!(
            p.iter().map(|x| x.email.as_str()).collect::<Vec<_>>(),
            ["alice@contoso.com", "bob@contoso.com"]
        );
        assert!(people_lenient("").is_empty());
    }

    #[test]
    fn local_times_become_instants_through_the_offset() {
        let cairo = |_: i64| 2 * 3_600_000;
        assert_eq!(
            local_ms("2026-11-18", "10:00", &cairo),
            Some(Date::new(2026, 11, 18).midnight_ms() + 8 * 3_600_000)
        );
        assert_eq!(local_ms("2026-11-18", "xx", &cairo), None);
    }

    /// The form's output against the live service. It makes two private appointments with no
    /// attendees (the second repeating weekly), reads them back, moves one, and deletes both
    /// before it ends. Only counts and yes/no are printed.
    /// `cargo test -p kelma live_form_round_trip -- --ignored --nocapture`
    #[test]
    #[ignore = "talks to the live service: makes and deletes two private appointments"]
    fn live_form_round_trip() {
        use ::calendar::{
            Calendar, create_event, delete_event, event_detail, instances, move_event,
        };
        const DAY: i64 = 86_400_000;
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let paths = session::Paths::new().unwrap();
            let cfg = paths.load_config().unwrap();
            let sess = session::stored_session(&cfg).unwrap().expect("signed in");
            let client = teams_proto::Client::resume(sess).await.unwrap();
            session::persist(&cfg, &client).await.unwrap();

            let off = &super::super::local_offset;
            let day = super::super::grid::local_date(teams_proto::time::now_ms() + 40 * DAY, off);
            let tz = local_tz();
            let made = std::cell::RefCell::new(Vec::<String>::new());
            let outcome: Result<(), String> = async {
                macro_rules! check {
                    ($c:expr, $m:expr) => {
                        if !$c {
                            return Err($m.to_string());
                        }
                    };
                }
                let mut f = FormFields::blank(day, 12 * 60);
                f.subject = "kelma test form".into();
                f.private = true;
                f.show_as = "free".into();
                f.reminder = "30".into();
                f.body = "**bold** line".into();
                let new = f.to_new_event(&tz, None)?;
                let created = create_event(&client, &new)
                    .await
                    .map_err(|e| e.to_string())?;
                made.borrow_mut().push(created.id.clone());
                println!("created with time zone {tz}");
                check!(created.sensitivity_private, "not private");
                check!(created.attendees.is_empty(), "has attendees");
                check!(created.show_as == ShowAs::Free, "show as");
                check!(created.reminder_minutes == Some(30), "reminder");
                // The form reads its own event back to the same local time.
                let back = FormFields::from_event(&created, "", off, false);
                println!(
                    "local time round trip: {}",
                    (back.start_date == f.start_date && back.start_time == f.start_time)
                );
                check!(
                    back.start_date == f.start_date && back.start_time == f.start_time,
                    "time"
                );
                check!(back.end_time == f.end_time, "end time");
                let detail = event_detail(&client, &created.id)
                    .await
                    .map_err(|e| e.to_string())?;
                let plain = super::super::model::detail_data(&detail).body;
                println!("body round trip: {}", plain.contains("bold"));
                check!(plain.contains("bold"), "body");
                let hour = 3_600_000;
                let wall = |ms: i64| ::calendar::format_iso_ms(ms)[..19].to_string();
                let moved = move_event(
                    &client,
                    &created.id,
                    &wall(created.start_ms + hour),
                    &wall(created.end_ms + hour),
                    "UTC",
                )
                .await
                .map_err(|e| e.to_string())?;
                println!(
                    "moved an hour: {}",
                    moved.start_ms == created.start_ms + hour
                );
                check!(moved.start_ms == created.start_ms + hour, "move");

                // A weekly series of three.
                let mut series = f.clone();
                series.subject = "kelma test series".into();
                series.start_date = day.add_days(3).to_string();
                series.end_date = series.start_date.clone();
                series.repeat = "weekly".into();
                series.end_kind = "after".into();
                series.end_value = "3".into();
                let sn = series.to_new_event(&tz, None)?;
                let master = create_event(&client, &sn)
                    .await
                    .map_err(|e| e.to_string())?;
                made.borrow_mut().push(master.id.clone());
                let from = master.start_ms - DAY;
                let found = instances(&client, &master.id, from, from + 40 * DAY)
                    .await
                    .map_err(|e| e.to_string())?;
                println!("series occurrences: {}", found.len());
                check!(found.len() == 3, "occurrences");
                Ok(())
            }
            .await;
            let ids: Vec<String> = made.borrow().clone();
            for id in &ids {
                if let Err(e) = delete_event(&client, id).await {
                    println!("cleanup failed: {e}");
                }
            }
            let start = super::super::grid::local_midnight_ms(day, off) - DAY;
            let left = Calendar::view(&client, start, start + 50 * DAY)
                .await
                .unwrap()
                .iter()
                .filter(|e| e.subject.starts_with("kelma test"))
                .count();
            println!("cleanup proof: {left} events named 'kelma test*' left");
            session::persist(&cfg, &client).await.unwrap();
            assert_eq!(left, 0, "cleanup");
            outcome.unwrap();
        });
    }
}
