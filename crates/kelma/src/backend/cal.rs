//! Calendar: the view state, delta sync, reminders and the Join banner.
//!
//! Events live in memory (`EventCache`), filled by `/me/calendarView` for what is on screen and
//! `/me/calendarView/delta` for a window around today, every two minutes. A short tick turns the
//! cache into desktop reminders and the in-app Join banner.

mod grid;
mod reminders;

use super::Engine;
use ::calendar::{Calendar, Date, Event, EventCache, View, Weekday};
use chrono::{Local, TimeZone};
use grid::DAY_MS;
pub use grid::{CalData, Ctx};
#[cfg(test)]
pub use grid::{build, parse_view};
use reminders::Key;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use teams_proto::broker::Audience;
use teams_proto::time::now_ms;

const FIRST_DAY: Weekday = Weekday::Monday;
const SYNC_EVERY: Duration = Duration::from_secs(120);
const TICK_EVERY: Duration = Duration::from_secs(15);
/// The delta window: this many days back and forward from today.
const PAST_DAYS: i64 = 35;
const FUTURE_DAYS: i64 = 90;
/// Events further than this from now are not considered for reminders and banners.
const SCAN_PAST_MS: i64 = DAY_MS;
const SCAN_FUTURE_MS: i64 = 8 * DAY_MS;
const SNOOZE_MS: i64 = 5 * 60_000;
const SCALES: [u32; 3] = [15, 30, 60];
const GRAPH_EVENTS: &str = "https://graph.microsoft.com/v1.0/me/events";

/// Milliseconds local time is ahead of UTC at the UTC instant `ms`.
fn local_offset(ms: i64) -> i64 {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map_or(0, |t| i64::from(t.offset().local_minus_utc()) * 1000)
}

pub fn today() -> Date {
    grid::local_date(now_ms(), &local_offset)
}

/// Unix milliseconds of 00:00 local time on this week's Monday (for seeding demo events).
pub fn local_week_start() -> i64 {
    let t = today();
    let back = (t.weekday() as i64 - 1).rem_euclid(7);
    grid::local_midnight_ms(t.add_days(-back), &local_offset)
}

fn parse_date(s: &str) -> Option<Date> {
    let mut it = s.splitn(3, '-');
    let (y, m, d) = (it.next()?, it.next()?, it.next()?);
    let (m, d) = (m.parse::<u32>().ok()?, d.parse::<u32>().ok()?);
    let y = y.parse().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then(|| Date::new(y, m, d))
}

/// Commands from the calendar view.
#[derive(Debug, Clone, PartialEq)]
pub enum CalCmd {
    Today,
    Prev,
    Next,
    SetView(String),
    /// Add or remove days from the Day view.
    Days(i32),
    SetScale(u32),
    Goto(Date),
    /// Move the mini calendar by months.
    Mini(i32),
    Join(String),
    DismissBanner,
    Snooze(String, i64),
    Dismiss(String, i64),
}

impl CalCmd {
    /// The commands the UI sends, as `(name, argument)`.
    pub fn parse(name: &str, arg: &str) -> Option<CalCmd> {
        Some(match name {
            "today" => CalCmd::Today,
            "prev" => CalCmd::Prev,
            "next" => CalCmd::Next,
            "view" => CalCmd::SetView(arg.into()),
            "days" => CalCmd::Days(arg.parse().ok()?),
            "scale" => CalCmd::SetScale(arg.parse().ok()?),
            "goto" => CalCmd::Goto(parse_date(arg)?),
            "mini" => CalCmd::Mini(arg.parse().ok()?),
            "join" => CalCmd::Join(arg.into()),
            "dismiss-banner" => CalCmd::DismissBanner,
            _ => return None,
        })
    }
}

struct Inner {
    cache: EventCache,
    date: Date,
    view: View,
    scale: u32,
    mini: Date,
    /// The delta window `[start, end)`, once the first delta finished.
    window: Option<(i64, i64)>,
    /// Ranges fetched with calendarView outside the delta window.
    fetched: Vec<(i64, i64)>,
    fired: HashSet<Key>,
    snoozed: HashMap<Key, i64>,
    dismissed: HashSet<Key>,
    banner: Option<Key>,
    status: String,
    last: Option<CalData>,
}

/// Calendar state shared by the engine.
pub struct CalState(Mutex<Inner>);

impl CalState {
    pub fn new(cfg: &session::Config) -> CalState {
        let today = today();
        CalState(Mutex::new(Inner {
            cache: EventCache::default(),
            date: today,
            view: cfg
                .calendar_view
                .as_deref()
                .map_or(View::WorkWeek, grid::parse_view),
            scale: cfg
                .calendar_scale
                .filter(|s| SCALES.contains(s))
                .unwrap_or(30),
            mini: today,
            window: None,
            fetched: Vec::new(),
            fired: HashSet::new(),
            snoozed: HashMap::new(),
            dismissed: HashSet::new(),
            banner: None,
            status: String::new(),
            last: None,
        }))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap()
    }
}

impl Inner {
    fn range(&self) -> (i64, i64) {
        grid::local_range(self.date, self.view, FIRST_DAY, &local_offset)
    }

    /// The visible range and the same length ahead.
    fn wanted(&self) -> (i64, i64) {
        let (s, e) = self.range();
        (s, e + (e - s))
    }

    fn covered(&self, (s, e): (i64, i64)) -> bool {
        let inside = |(ws, we): (i64, i64)| ws <= s && e <= we;
        self.window.is_some_and(inside) || self.fetched.iter().copied().any(inside)
    }

    /// Applies a navigation command; whether the view or scale changed and must be saved.
    fn apply(&mut self, cmd: &CalCmd, today: Date) -> bool {
        match cmd {
            CalCmd::Today => {
                self.date = today;
                self.mini = today;
            }
            CalCmd::Prev | CalCmd::Next => {
                let dir = if *cmd == CalCmd::Prev { -1 } else { 1 };
                self.date = grid::step_date(self.date, self.view, dir);
                self.mini = self.date;
            }
            CalCmd::SetView(k) => {
                self.view = grid::parse_view(k);
                return true;
            }
            CalCmd::Days(d) => {
                if let View::Day(n) = self.view {
                    let n = (n as i32 + d).clamp(1, grid::MAX_DAYS as i32);
                    self.view = View::Day(n as u32);
                    return true;
                }
            }
            CalCmd::SetScale(sc) if SCALES.contains(sc) => {
                self.scale = *sc;
                return true;
            }
            CalCmd::Goto(d) => {
                self.date = *d;
                self.mini = *d;
            }
            CalCmd::Mini(m) => self.mini = grid::add_months(self.mini, *m),
            CalCmd::DismissBanner => {
                if let Some(k) = self.banner.take() {
                    self.dismissed.insert(k);
                }
            }
            _ => {}
        }
        false
    }

    fn data(&self, now: i64) -> CalData {
        let (s, e) = self.range();
        let events = self.cache.in_range(s, e);
        let mut data = grid::build(
            &events,
            &Ctx {
                date: self.date,
                view: self.view,
                scale: self.scale,
                now_ms: now,
                first_day: FIRST_DAY,
                mini_anchor: self.mini,
                off: &local_offset,
            },
        );
        data.status = self.status.clone();
        data
    }

    /// Events near `now` that can remind or banner.
    fn nearby(&self, now: i64) -> Vec<&Event> {
        self.cache
            .in_range(now - SCAN_PAST_MS, now + SCAN_FUTURE_MS)
    }
}

/// `…/me/events/{id}/{action}`, with the id escaped for a path segment.
fn reminder_url(id: &str, action: &str) -> String {
    let mut esc = String::new();
    for b in id.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            esc.push(b as char);
        } else {
            esc.push_str(&format!("%{b:02X}"));
        }
    }
    format!("{GRAPH_EVENTS}/{esc}/{action}")
}

fn snooze_body(until_ms: i64) -> serde_json::Value {
    let iso = ::calendar::format_iso_ms(until_ms);
    serde_json::json!({"newReminderTime": {
        "dateTime": iso.trim_end_matches('Z'),
        "timeZone": "UTC",
    }})
}

impl Engine {
    /// Starts the calendar sync and reminder loops.
    pub(super) fn start_calendar(self: &Arc<Self>) {
        let e = self.clone();
        tokio::spawn(async move {
            loop {
                if let Err(err) = e.sync_calendar().await {
                    tracing::warn!(%err, "calendar sync failed");
                }
                tokio::time::sleep(SYNC_EVERY).await;
            }
        });
        let e = self.clone();
        tokio::spawn(async move { e.calendar_tick_loop(true).await });
    }

    /// Demo mode: invented events, no network and no desktop notifications.
    pub(super) fn start_calendar_offline(self: &Arc<Self>) {
        let changes =
            ::calendar::parse_events(&crate::demo::calendar_events(local_week_start(), now_ms()))
                .into_iter()
                .map(::calendar::EventChange::Upsert)
                .collect();
        self.cal.lock().cache.apply(changes);
        let e = self.clone();
        tokio::spawn(async move { e.calendar_tick_loop(false).await });
    }

    async fn calendar_tick_loop(self: Arc<Self>, notify: bool) {
        loop {
            self.calendar_tick(notify);
            tokio::time::sleep(TICK_EVERY).await;
        }
    }

    /// Fires due reminders, updates the Join banner and refreshes the view (the now line moves).
    fn calendar_tick(self: &Arc<Self>, notify: bool) {
        let now = now_ms();
        let (due, banner) = {
            let mut s = self.cal.lock();
            let s = &mut *s;
            let due = reminders::due_reminders(s.nearby(now), now, &s.fired, &s.snoozed);
            for r in &due {
                let k = (r.id.clone(), r.start_ms);
                s.snoozed.remove(&k);
                s.fired.insert(k);
            }
            let banner = reminders::join_banner(s.nearby(now), now, &s.dismissed);
            s.banner = banner.as_ref().map(|b| (b.id.clone(), b.start_ms));
            (due, banner)
        };
        for r in due {
            tracing::info!(
                minutes_to_start = (r.start_ms - now) / 60_000,
                "calendar reminder"
            );
            if notify {
                self.notify_reminder(r, now);
            }
        }
        self.ui
            .join_banner(banner.map(|b| (b.text, b.join_url)).unwrap_or_default());
        self.push_calendar();
    }

    /// Sends the current view to the UI when it changed.
    pub(super) fn push_calendar(&self) {
        let data = {
            let mut s = self.cal.lock();
            let data = s.data(now_ms());
            if s.last.as_ref() == Some(&data) {
                return;
            }
            s.last = Some(data.clone());
            data
        };
        self.ui.calendar(data);
    }

    /// Loads what the view needs, then the delta window; applies Graph's changes.
    async fn sync_calendar(self: &Arc<Self>) -> anyhow::Result<()> {
        self.ensure_calendar_range().await?;
        let (window, link) = {
            let mut s = self.cal.lock();
            let today = grid::local_midnight_ms(today(), &local_offset);
            let want = (today - PAST_DAYS * DAY_MS, today + FUTURE_DAYS * DAY_MS);
            // The window follows today; a week of drift is cheap to ignore.
            if s.window.is_none_or(|w| (w.0 - want.0).abs() > 7 * DAY_MS) {
                s.cache.delta_link = None;
                s.window = None;
            }
            (s.window.unwrap_or(want), s.cache.delta_link.clone())
        };
        let mut result = Calendar::delta(&self.client, window.0, window.1, link.as_deref()).await;
        if matches!(result, Err(teams_proto::Error::Status { status: 410, .. })) {
            // The delta link expired: start the window over.
            result = Calendar::delta(&self.client, window.0, window.1, None).await;
        }
        let mut s = self.cal.lock();
        match result {
            Ok((changes, delta_link)) => {
                s.cache.apply(changes);
                s.cache.delta_link = Some(delta_link);
                s.window = Some(window);
                s.status.clear();
                tracing::info!(events = s.cache.len(), "calendar synced");
                drop(s);
                self.push_calendar();
                Ok(())
            }
            Err(err) => {
                s.status = "Couldn't sync the calendar".into();
                drop(s);
                self.push_calendar();
                Err(err.into())
            }
        }
    }

    /// Fetches the visible range (and one more ahead) when the cache does not cover it.
    async fn ensure_calendar_range(self: &Arc<Self>) -> anyhow::Result<()> {
        let want = {
            let s = self.cal.lock();
            let want = s.wanted();
            if s.covered(want) {
                return Ok(());
            }
            want
        };
        if self.offline.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
        let events = Calendar::view(&self.client, want.0, want.1).await?;
        let mut s = self.cal.lock();
        tracing::info!(events = events.len(), "calendar range loaded");
        s.cache.apply(
            events
                .into_iter()
                .map(::calendar::EventChange::Upsert)
                .collect(),
        );
        s.fetched.push(want);
        drop(s);
        self.push_calendar();
        Ok(())
    }

    pub(super) async fn calendar_cmd(self: &Arc<Self>, cmd: CalCmd) -> anyhow::Result<()> {
        match cmd {
            CalCmd::Join(url) => {
                if url.starts_with("https://") {
                    let _ = open::that_detached(&url);
                }
                return Ok(());
            }
            CalCmd::Snooze(id, start) => return self.snooze_reminder(id, start).await,
            CalCmd::Dismiss(id, start) => return self.dismiss_reminder(id, start).await,
            _ => {}
        }
        let persist = self.cal.lock().apply(&cmd, today());
        if cmd == CalCmd::DismissBanner {
            self.ui.join_banner(Default::default());
        }
        self.push_calendar();
        if persist {
            self.persist_calendar_view();
        }
        if let Err(err) = self.ensure_calendar_range().await {
            tracing::warn!(%err, "calendar range load failed");
            self.cal.lock().status = "Couldn't load these dates".into();
            self.push_calendar();
        }
        Ok(())
    }

    /// Remembers the view and time scale in the config file (not in demo mode).
    fn persist_calendar_view(&self) {
        if self.offline.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let (view, scale) = {
            let s = self.cal.lock();
            (grid::view_key(s.view), s.scale)
        };
        let cfg = {
            let mut cfg = self.cfg.lock().unwrap();
            cfg.calendar_view = Some(view);
            cfg.calendar_scale = Some(scale);
            cfg.clone()
        };
        let saved = session::Paths::new().and_then(|p| p.save_config(&cfg));
        if let Err(err) = saved {
            tracing::warn!(%err, "could not save the calendar view");
        }
    }

    /// Desktop notification for a reminder, with Join, Snooze and Dismiss.
    fn notify_reminder(self: &Arc<Self>, r: reminders::Reminder, now: i64) {
        let engine = self.clone();
        let rt = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let mut n = notify_rust::Notification::new();
            n.appname("kelma")
                .summary(if r.subject.is_empty() {
                    "Meeting"
                } else {
                    &r.subject
                })
                .body(&reminders::reminder_body(&r, now))
                .icon("appointment-soon")
                .timeout(notify_rust::Timeout::Milliseconds(60_000));
            if r.join_url.is_some() {
                n.action("join", "Join");
            }
            n.action("snooze", "Snooze 5 min")
                .action("dismiss", "Dismiss");
            let Ok(handle) = n.show() else { return };
            handle.wait_for_action(|action| {
                let cmd = match action {
                    "join" | "default" => r.join_url.clone().map(CalCmd::Join),
                    "snooze" => Some(CalCmd::Snooze(r.id.clone(), r.start_ms)),
                    "dismiss" => Some(CalCmd::Dismiss(r.id.clone(), r.start_ms)),
                    _ => None,
                };
                if let Some(cmd) = cmd {
                    rt.spawn(async move {
                        let _ = engine.calendar_cmd(cmd).await;
                    });
                }
            });
        });
    }

    async fn reminder_call(
        &self,
        id: &str,
        action: &str,
        body: Option<&serde_json::Value>,
    ) -> anyhow::Result<()> {
        if self.offline.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
        let (status, text) = self
            .client
            .request_as(
                &Audience::Graph,
                reqwest::Method::POST,
                &reminder_url(id, action),
                body,
                &[],
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(teams_proto::Error::status("reminder", status, &text).into());
        }
        Ok(())
    }

    /// `POST /me/events/{id}/snoozeReminder`: remind again in five minutes.
    async fn snooze_reminder(&self, id: String, start_ms: i64) -> anyhow::Result<()> {
        let until = now_ms() + SNOOZE_MS;
        self.reminder_call(&id, "snoozeReminder", Some(&snooze_body(until)))
            .await?;
        self.cal.lock().snoozed.insert((id, start_ms), until);
        Ok(())
    }

    /// `POST /me/events/{id}/dismissReminder`.
    async fn dismiss_reminder(&self, id: String, start_ms: i64) -> anyhow::Result<()> {
        self.reminder_call(&id, "dismissReminder", None).await?;
        let mut s = self.cal.lock();
        s.snoozed.remove(&(id.clone(), start_ms));
        s.fired.insert((id, start_ms));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_commands_parse() {
        assert_eq!(CalCmd::parse("today", ""), Some(CalCmd::Today));
        assert_eq!(
            CalCmd::parse("view", "day:3"),
            Some(CalCmd::SetView("day:3".into()))
        );
        assert_eq!(CalCmd::parse("scale", "15"), Some(CalCmd::SetScale(15)));
        assert_eq!(CalCmd::parse("days", "-1"), Some(CalCmd::Days(-1)));
        assert_eq!(
            CalCmd::parse("goto", "2026-11-18"),
            Some(CalCmd::Goto(Date::new(2026, 11, 18)))
        );
        assert_eq!(CalCmd::parse("goto", "nonsense"), None);
        assert_eq!(CalCmd::parse("scale", "x"), None);
        assert_eq!(CalCmd::parse("bogus", ""), None);
    }

    #[test]
    fn reminder_requests_target_the_event_and_escape_its_id() {
        assert_eq!(
            reminder_url("AAMk=/x_y-z", "snoozeReminder"),
            "https://graph.microsoft.com/v1.0/me/events/AAMk%3D%2Fx_y-z/snoozeReminder"
        );
        assert_eq!(
            snooze_body(1_794_996_300_000),
            serde_json::json!({"newReminderTime":
                {"dateTime": "2026-11-18T10:05:00.000", "timeZone": "UTC"}})
        );
    }

    #[test]
    fn the_view_is_restored_from_the_config() {
        let cfg = session::Config {
            calendar_view: Some("day:3".into()),
            calendar_scale: Some(15),
            ..Default::default()
        };
        let s = CalState::new(&cfg);
        let g = s.lock();
        assert_eq!((g.view, g.scale), (View::Day(3), 15));
        drop(g);
        let s = CalState::new(&session::Config {
            calendar_scale: Some(7),
            ..Default::default()
        });
        let g = s.lock();
        assert_eq!((g.view, g.scale), (View::WorkWeek, 30));
    }

    fn state(cfg: session::Config) -> CalState {
        CalState::new(&cfg)
    }

    const NOV_18: Date = Date {
        year: 2026,
        month: 11,
        day: 18,
    };

    #[test]
    fn switching_views_changes_the_columns_and_is_saved() {
        let s = state(session::Config::default());
        let mut s = s.lock();
        s.date = NOV_18;
        let cols = |s: &Inner| s.data(0).days.len();
        assert_eq!((s.view, cols(&s)), (View::WorkWeek, 5));
        assert!(s.apply(&CalCmd::SetView("week".into()), NOV_18));
        assert_eq!((s.view, cols(&s)), (View::Week, 7));
        assert!(s.apply(&CalCmd::SetView("day:3".into()), NOV_18));
        assert_eq!(cols(&s), 3);
        assert!(s.apply(&CalCmd::Days(1), NOV_18));
        assert_eq!(cols(&s), 4);
        assert!(s.apply(&CalCmd::SetView("month".into()), NOV_18));
        assert_eq!(s.data(0).view_kind, "month");
        assert!(
            !s.apply(&CalCmd::Days(1), NOV_18),
            "Days only applies to the Day view"
        );
        assert!(s.apply(&CalCmd::SetScale(15), NOV_18));
        assert!(
            !s.apply(&CalCmd::SetScale(20), NOV_18),
            "unknown scales are ignored"
        );
        assert_eq!(s.scale, 15);
    }

    #[test]
    fn navigation_moves_by_the_view_and_today_comes_back() {
        let s = state(session::Config::default());
        let mut s = s.lock();
        s.date = NOV_18;
        assert!(!s.apply(&CalCmd::Next, NOV_18));
        assert_eq!(s.date, Date::new(2026, 11, 25));
        s.apply(&CalCmd::Prev, NOV_18);
        s.apply(&CalCmd::Prev, NOV_18);
        assert_eq!(s.date, Date::new(2026, 11, 11));
        s.apply(&CalCmd::Goto(Date::new(2027, 1, 5)), NOV_18);
        assert_eq!(
            (s.date, s.mini),
            (Date::new(2027, 1, 5), Date::new(2027, 1, 5))
        );
        s.apply(&CalCmd::Mini(-1), NOV_18);
        assert_eq!(s.mini, Date::new(2026, 12, 5));
        assert_eq!(
            s.date,
            Date::new(2027, 1, 5),
            "the mini calendar moves on its own"
        );
        s.apply(&CalCmd::Today, NOV_18);
        assert_eq!((s.date, s.mini), (NOV_18, NOV_18));
    }
}
