//! Reading the calendar from Microsoft Graph.

use std::collections::BTreeMap;

use reqwest::Method;
use serde_json::Value;
use teams_proto::broker::Audience;
use teams_proto::{Client, Error, Result};

use crate::event::{Event, EventChange, delta_link, next_link, parse_delta_page, parse_events};
use crate::range::format_iso_ms;

const BASE: &str = "https://graph.microsoft.com/v1.0/me/calendarView";

/// Fields the views and the peek need. Attendees and the body preview are small enough to
/// carry along so the full view needs no second request.
pub(crate) const SELECT: &str = "id,iCalUId,seriesMasterId,subject,start,end,isAllDay,showAs,responseStatus,\
isCancelled,sensitivity,categories,organizer,location,isOnlineMeeting,onlineMeeting,webLink,\
type,bodyPreview,attendees,isReminderOn,reminderMinutesBeforeStart";

/// Times come back in UTC and ids stay the same when an event moves between folders.
const PREFER: (&str, &str) = ("Prefer", "outlook.timezone=\"UTC\", IdType=\"ImmutableId\"");

/// A safety stop so a server that keeps returning `nextLink` cannot loop forever.
const MAX_PAGES: usize = 100;

/// Stateless entry points for reading the calendar.
pub struct Calendar;

fn range_query(start_ms: i64, end_ms: i64) -> String {
    format!(
        "startDateTime={}&endDateTime={}",
        format_iso_ms(start_ms),
        format_iso_ms(end_ms)
    )
}

/// The URL of the first calendarView page.
pub fn view_url(start_ms: i64, end_ms: i64) -> String {
    format!(
        "{BASE}?{}&$top=200&$select={SELECT}",
        range_query(start_ms, end_ms)
    )
}

/// The URL that starts a calendarView delta sync.
pub fn delta_url(start_ms: i64, end_ms: i64) -> String {
    format!("{BASE}/delta?{}", range_query(start_ms, end_ms))
}

async fn get(client: &Client, context: &'static str, url: &str) -> Result<Value> {
    let (status, text) = client
        .request_as(&Audience::Graph, Method::GET, url, None, &[PREFER])
        .await?;
    if !(200..300).contains(&status) {
        return Err(Error::status(context, status, &text));
    }
    Ok(serde_json::from_str(&text)?)
}

impl Calendar {
    /// Every event that overlaps `[start_ms, end_ms)`, recurring series expanded into
    /// occurrences, following `@odata.nextLink`. Read-only.
    pub async fn view(client: &Client, start_ms: i64, end_ms: i64) -> Result<Vec<Event>> {
        let mut url = view_url(start_ms, end_ms);
        let mut events = Vec::new();
        for _ in 0..MAX_PAGES {
            let page = get(client, "calendar view", &url).await?;
            events.extend(parse_events(&page));
            match next_link(&page) {
                Some(next) => url = next,
                None => return Ok(events),
            }
        }
        Err(Error::Protocol(format!(
            "calendar view: more than {MAX_PAGES} pages"
        )))
    }

    /// Changes since `delta_link` (or since nothing, for the first sync of the range), and the
    /// new delta link to pass next time. A stale link makes Graph answer 410; start over with
    /// `None` then.
    pub async fn delta(
        client: &Client,
        start_ms: i64,
        end_ms: i64,
        delta_link_in: Option<&str>,
    ) -> Result<(Vec<EventChange>, String)> {
        let mut url = match delta_link_in {
            Some(link) => link.to_string(),
            None => delta_url(start_ms, end_ms),
        };
        let mut changes = Vec::new();
        for _ in 0..MAX_PAGES {
            let page = get(client, "calendar delta", &url).await?;
            let parsed = parse_delta_page(&page);
            changes.extend(parsed.changes);
            if let Some(next) = parsed.next_link {
                url = next;
            } else if let Some(done) = delta_link(&page) {
                return Ok((changes, done));
            } else {
                return Err(Error::Protocol(
                    "calendar delta: page has neither nextLink nor deltaLink".into(),
                ));
            }
        }
        Err(Error::Protocol(format!(
            "calendar delta: more than {MAX_PAGES} pages"
        )))
    }
}

/// Events by id, kept up to date from delta changes.
#[derive(Debug, Default)]
pub struct EventCache {
    events: BTreeMap<String, Event>,
    /// The delta link to pass to the next [`Calendar::delta`].
    pub delta_link: Option<String>,
}

impl EventCache {
    pub fn apply(&mut self, changes: Vec<EventChange>) {
        for change in changes {
            match change {
                EventChange::Upsert(e) => {
                    self.events.insert(e.id.clone(), e);
                }
                EventChange::Removed(id) => {
                    self.events.remove(&id);
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Events overlapping `[start_ms, end_ms)`, earliest first.
    pub fn in_range(&self, start_ms: i64, end_ms: i64) -> Vec<&Event> {
        let mut v: Vec<&Event> = self
            .events
            .values()
            .filter(|e| e.start_ms < end_ms && e.end_ms.max(e.start_ms + 1) > start_ms)
            .collect();
        v.sort_by_key(|e| (e.start_ms, e.end_ms));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builds_the_view_url() {
        let url = view_url(1_794_960_000_000, 1_795_564_800_000);
        assert!(url.starts_with(
            "https://graph.microsoft.com/v1.0/me/calendarView?\
             startDateTime=2026-11-18T00:00:00.000Z&endDateTime=2026-11-25T00:00:00.000Z&$top=200&$select=id,"
        ));
        assert!(url.contains(",isOnlineMeeting,onlineMeeting,"));
        assert_eq!(
            delta_url(1_794_960_000_000, 1_795_564_800_000),
            "https://graph.microsoft.com/v1.0/me/calendarView/delta?\
             startDateTime=2026-11-18T00:00:00.000Z&endDateTime=2026-11-25T00:00:00.000Z"
        );
    }

    fn upsert(item: Value) -> EventChange {
        crate::parse_delta_page(&json!({"value": [item]}))
            .changes
            .remove(0)
    }

    #[test]
    fn cache_applies_upserts_and_removals_and_filters_by_range() {
        let ev = |id: &str, s: &str, e: &str| {
            upsert(json!({"id": id,
                "start": {"dateTime": s, "timeZone": "UTC"},
                "end": {"dateTime": e, "timeZone": "UTC"}}))
        };
        let mut cache = EventCache::default();
        assert!(cache.is_empty());
        cache.apply(vec![
            ev("b", "2026-11-18T10:00:00", "2026-11-18T11:00:00"),
            ev("a", "2026-11-18T09:00:00", "2026-11-18T10:00:00"),
            ev("c", "2026-11-19T09:00:00", "2026-11-19T10:00:00"),
        ]);
        assert_eq!(cache.len(), 3);
        let day = |d: i64| {
            (
                1_794_960_000_000 + d * 86_400_000,
                1_794_960_000_000 + (d + 1) * 86_400_000,
            )
        };
        let (s, e) = day(0);
        let ids: Vec<_> = cache.in_range(s, e).iter().map(|e| e.id.clone()).collect();
        assert_eq!(ids, ["a", "b"], "sorted by start, other day excluded");

        cache.apply(vec![
            EventChange::Removed("a".into()),
            ev("b", "2026-11-20T10:00:00", "2026-11-20T11:00:00"),
        ]);
        assert_eq!(cache.len(), 2);
        assert!(cache.in_range(s, e).is_empty());
        let (s, e) = day(2);
        assert_eq!(cache.in_range(s, e)[0].id, "b");
    }
}
