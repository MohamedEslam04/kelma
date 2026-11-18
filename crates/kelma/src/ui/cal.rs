//! UI-thread side of the calendar: turns `CalData` into the `CalStore` global.

use super::{UiHandle, model, rgb};
use crate::backend::CalData;
use crate::{CalAllDay, CalBlock, CalDay, CalMiniCell, CalMonthCell, CalMonthItem, CalStore};
use slint::{ComponentHandle, SharedString};

fn int(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

fn set(w: &crate::AppWindow, d: CalData) {
    let s = w.global::<CalStore>();
    s.set_title(d.title.into());
    s.set_view_kind(d.view_kind.into());
    s.set_day_count(d.day_count as i32);
    s.set_scale(d.scale as i32);
    s.set_status(d.status.into());
    s.set_days(model(
        d.days
            .into_iter()
            .map(|x| CalDay {
                iso: x.iso.into(),
                label: x.label.into(),
                number: x.number as i32,
                today: x.today,
                weekend: x.weekend,
            })
            .collect(),
    ));
    s.set_blocks(model(
        d.blocks
            .into_iter()
            .map(|b| CalBlock {
                id: b.id.into(),
                title: b.title.into(),
                organizer: b.organizer.into(),
                time: b.time.into(),
                col: int(b.col),
                start_min: b.start_min,
                dur_min: b.dur_min,
                lane: int(b.lane),
                lanes: int(b.lanes),
                kind: b.look.kind.as_str().into(),
                declined: b.look.declined,
                private: b.look.private,
                online: b.look.online,
                recurring: b.look.recurring,
                tone: rgb(b.look.color),
            })
            .collect(),
    ));
    s.set_all_day(model(
        d.all_day
            .into_iter()
            .map(|a| CalAllDay {
                id: a.id.into(),
                title: a.title.into(),
                col: int(a.col),
                span: int(a.span),
                row: int(a.row),
                kind: a.look.kind.as_str().into(),
                declined: a.look.declined,
                private: a.look.private,
                online: a.look.online,
                recurring: a.look.recurring,
                tone: rgb(a.look.color),
            })
            .collect(),
    ));
    s.set_all_day_rows(int(d.all_day_rows));
    s.set_month(model(
        d.month
            .into_iter()
            .map(|c| CalMonthCell {
                iso: c.iso.into(),
                number: c.number as i32,
                in_month: c.in_month,
                today: c.today,
                items: model(
                    c.items
                        .into_iter()
                        .map(|i| CalMonthItem {
                            title: i.title.into(),
                            time: i.time.into(),
                            declined: i.look.declined,
                            tone: rgb(i.look.color),
                        })
                        .collect(),
                ),
                more: int(c.more),
            })
            .collect(),
    ));
    s.set_mini_title(d.mini_title.into());
    s.set_mini(model(
        d.mini
            .into_iter()
            .map(|c| CalMiniCell {
                iso: c.iso.into(),
                number: c.number as i32,
                in_month: c.in_month,
                today: c.today,
                selected: c.selected,
            })
            .collect(),
    ));
    s.set_now_min(d.now_min.unwrap_or(-1));
    s.set_now_col(d.now_col.map_or(-1, int));
}

impl UiHandle {
    pub fn calendar(&self, d: CalData) {
        self.run(move |w| set(&w, d));
    }

    /// Shows the Join banner (`text`, `url`), or hides it when `text` is empty.
    pub fn join_banner(&self, (text, url): (String, String)) {
        self.run(move |w| {
            let s = w.global::<CalStore>();
            s.set_join_text(SharedString::from(text));
            s.set_join_url(url.into());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppWindow;
    use crate::backend::{Ctx, build};
    use calendar::{Date, Event, View, Weekday, parse_events};
    use i_slint_backend_testing::{self as testing, ElementHandle};
    use serde_json::json;
    use slint::platform::PointerEventButton;
    use slint::{ComponentHandle, LogicalSize};
    use std::cell::RefCell;
    use std::rc::Rc;

    const H: i64 = 3_600_000;

    /// An event on Wednesday 2026-11-18 plus `day` days, from `start_h` to `end_h` (UTC).
    fn ev(id: &str, day: i64, start_h: f64, end_h: f64, extra: serde_json::Value) -> Event {
        let base = Date::new(2026, 11, 18).add_days(day).midnight_ms();
        let t = |h: f64| {
            json!({"dateTime": calendar::format_iso_ms(base + (h * H as f64) as i64),
            "timeZone": "UTC"})
        };
        let mut v = json!({"id": id, "subject": id, "start": t(start_h), "end": t(end_h)});
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        parse_events(&json!({ "value": [v] })).remove(0)
    }

    fn app(view: View, evs: &[Event]) -> AppWindow {
        thread_local! { static INIT: std::sync::Once = const { std::sync::Once::new() }; }
        INIT.with(|once| once.call_once(testing::init_no_event_loop));
        let app = AppWindow::new().unwrap();
        app.window().set_size(LogicalSize::new(1400., 900.));
        app.set_login_state("ready".into());
        app.set_view("calendar".into());
        let today = Date::new(2026, 11, 18);
        let refs: Vec<&Event> = evs.iter().collect();
        let data = build(
            &refs,
            &Ctx {
                date: today,
                view,
                scale: 30,
                now_ms: today.midnight_ms() + 10 * H,
                first_day: Weekday::Monday,
                mini_anchor: today,
                off: &|_| 0,
            },
        );
        set(&app, data);
        app.show().unwrap();
        app
    }

    fn label(app: &AppWindow, title: &str) -> ElementHandle {
        ElementHandle::find_by_accessible_label(app, title)
            .next()
            .unwrap_or_else(|| panic!("no event block labelled {title}"))
    }

    fn commands(app: &AppWindow) -> Rc<RefCell<Vec<(String, String)>>> {
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        app.global::<CalStore>()
            .on_command(move |n, a| l.borrow_mut().push((n.to_string(), a.to_string())));
        log
    }

    #[test]
    fn overlapping_events_sit_side_by_side_and_a_lone_event_fills_its_column() {
        let app = app(
            View::WorkWeek,
            &[
                ev("Alpha", 0, 9.0, 10.0, json!({})),
                ev("Beta", 0, 9.5, 10.5, json!({})),
                ev("Gamma", 1, 9.0, 10.0, json!({})),
            ],
        );
        let (a, b, g) = (
            label(&app, "Alpha"),
            label(&app, "Beta"),
            label(&app, "Gamma"),
        );
        let (ap, bp, gp) = (
            a.absolute_position(),
            b.absolute_position(),
            g.absolute_position(),
        );
        let (asz, bsz, gsz) = (a.size(), b.size(), g.size());

        assert!((asz.width - bsz.width).abs() < 0.5, "equal widths");
        assert!(
            ap.x + asz.width <= bp.x,
            "Beta starts right of Alpha: {} + {} <= {}",
            ap.x,
            asz.width,
            bp.x
        );
        assert!(
            (bp.y - ap.y - 32.0).abs() < 0.5,
            "30 minutes later is 32 px lower at the 30 minute scale"
        );
        assert!(gsz.width > asz.width * 1.8, "a lone event spans the column");
        assert!(gp.x > bp.x + bsz.width, "Thursday is the next column");
        assert!((ap.y - gp.y).abs() < 0.5, "same start, same height");
    }

    #[test]
    fn declined_events_are_struck_through_and_others_are_not() {
        let app = app(
            View::WorkWeek,
            &[
                ev("Kept", 0, 9.0, 10.0, json!({})),
                ev(
                    "Skipped",
                    0,
                    11.0,
                    12.0,
                    json!({"responseStatus": {"response": "declined"}}),
                ),
            ],
        );
        assert_eq!(
            ElementHandle::find_by_element_id(&app, "CalBox::strike").count(),
            1,
            "only the declined event has a strike line"
        );
        assert!(
            label(&app, "Skipped")
                .accessible_description()
                .unwrap()
                .starts_with("Declined")
        );
        assert!(
            !label(&app, "Kept")
                .accessible_description()
                .unwrap()
                .starts_with("Declined")
        );
    }

    #[test]
    fn columns_follow_the_view() {
        let heads = |view| {
            let app = app(view, &[]);
            ElementHandle::find_by_element_id(&app, "TimeGrid::today-marker").count()
        };
        assert_eq!(heads(View::WorkWeek), 5);
        assert_eq!(heads(View::Week), 7);
        assert_eq!(heads(View::Day(3)), 3);
    }

    #[test]
    fn the_toolbar_sends_navigation_and_view_commands() {
        let app = app(View::WorkWeek, &[]);
        let log = commands(&app);
        let mut pills: Vec<_> = ElementHandle::find_by_element_type_name(&app, "Pill")
            .filter(|p| (48.0..100.0).contains(&p.absolute_position().y))
            .collect();
        pills.sort_by(|a, b| a.absolute_position().x.total_cmp(&b.absolute_position().x));
        assert_eq!(pills.len(), 5, "Today, Day, Work week, Week, Month");
        for p in &pills {
            p.mock_single_click(PointerEventButton::Left);
        }
        let got: Vec<_> = log
            .borrow()
            .iter()
            .map(|(n, a)| format!("{n} {a}"))
            .collect();
        assert_eq!(
            got,
            [
                "today ",
                "view day",
                "view workweek",
                "view week",
                "view month"
            ]
        );
    }

    #[test]
    fn the_join_banner_shows_only_when_there_is_a_meeting_and_joins_it() {
        let app = app(View::WorkWeek, &[]);
        let banner = || ElementHandle::find_by_element_id(&app, "JoinBanner::join-touch").count();
        assert_eq!(banner(), 0, "no banner without a meeting");

        let log = commands(&app);
        let store = app.global::<CalStore>();
        store.set_join_text("Standup starts in 3 min".into());
        store.set_join_url("https://teams.example/j".into());
        assert_eq!(banner(), 1);
        ElementHandle::find_by_element_id(&app, "JoinBanner::join-touch")
            .next()
            .unwrap()
            .mock_single_click(PointerEventButton::Left);
        assert_eq!(
            *log.borrow(),
            [("join".to_string(), "https://teams.example/j".to_string())]
        );
    }

    /// Renders the demo week offscreen to a PNG, for looking at the design without a display:
    /// `KELMA_SHOT=/tmp/cal.png cargo test -p kelma render_demo_calendar -- --ignored`
    /// (`KELMA_SHOT_VIEW=week|month|day:3` picks the view).
    #[test]
    #[ignore = "writes a PNG; run by hand"]
    fn render_demo_calendar_png() {
        use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
        use slint::{PhysicalSize, Rgb8Pixel};

        struct Shot(Rc<MinimalSoftwareWindow>);
        impl slint::platform::Platform for Shot {
            fn create_window_adapter(
                &self,
            ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
                Ok(self.0.clone())
            }
        }
        let out = std::env::var("KELMA_SHOT").expect("set KELMA_SHOT to the output PNG path");
        let view =
            crate::backend::parse_view(&std::env::var("KELMA_SHOT_VIEW").unwrap_or_default());
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(Shot(window.clone()))).unwrap();
        let app = AppWindow::new().unwrap();
        let (w, h) = (1400u32, 900u32);
        window.set_size(PhysicalSize::new(w, h));
        app.set_login_state("ready".into());
        app.set_view("calendar".into());

        let monday = Date::new(2026, 11, 16);
        let now = monday.add_days(2).midnight_ms() + 10 * H;
        let events = parse_events(&crate::demo::calendar_events(monday.midnight_ms(), now));
        let refs: Vec<&Event> = events.iter().collect();
        let data = build(
            &refs,
            &Ctx {
                date: monday.add_days(2),
                view,
                scale: 30,
                now_ms: now,
                first_day: Weekday::Monday,
                mini_anchor: monday.add_days(2),
                off: &|_| 0,
            },
        );
        set(&app, data);
        let store = app.global::<CalStore>();
        store.set_join_text("Customer sync starts in 3 min".into());
        store.set_join_url("https://teams.example/j".into());
        app.show().unwrap();

        let mut buf = vec![Rgb8Pixel::default(); (w * h) as usize];
        slint::platform::update_timers_and_animations();
        window.draw_if_needed(|r| {
            r.render(&mut buf, w as usize);
        });
        let raw: Vec<u8> = buf.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
        image::save_buffer(&out, &raw, w, h, image::ColorType::Rgb8).unwrap();
    }
}
