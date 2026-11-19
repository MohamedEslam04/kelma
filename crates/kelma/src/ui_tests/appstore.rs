//! The app bar, the flyout, the Store and the built-in app screens (Planner and To Do, Workflows,
//! OneNote, Lists) driven through the real window. The engine runs behind it in demo mode (no
//! network): every `command` goes through `AppsCmd::parse` and the engine, and what the window
//! shows is read back from the globals and from the rendered elements. Nothing here can reach a
//! server; the install, remove and pin requests are only text.

use super::*;
use crate::backend::AppsCmd;
use crate::{AppsStore, FlowsStore, ListsStore, NotesStore, TasksStore};
use i_slint_backend_testing::AccessibleRole;

struct AppsHarness {
    h: Harness,
    /// Every `name argument` the screens sent, in order.
    log: Log,
}

impl AppsHarness {
    /// A window with the app globals wired to the engine, and the app bar loaded like at start.
    fn open() -> Self {
        let h = Harness::new();
        h.app.window().set_size(LogicalSize::new(1400., 900.));
        let log = Log::default();
        macro_rules! wire {
            ($global:ident) => {{
                let (engine, rt, log) = (h.engine.clone(), h.rt.clone(), log.clone());
                h.app.global::<$global>().on_command(move |name, arg| {
                    log.borrow_mut()
                        .push(format!("{name} {arg}").trim().to_owned());
                    if let Some(c) = AppsCmd::parse(&name, &arg) {
                        rt.block_on(engine.clone().handle(Cmd::Apps(c)));
                    }
                });
            }};
        }
        wire!(AppsStore);
        wire!(TasksStore);
        wire!(FlowsStore);
        wire!(NotesStore);
        wire!(ListsStore);
        let a = AppsHarness { h, log };
        a.run(AppsCmd::Load);
        a
    }

    fn run(&self, cmd: AppsCmd) {
        self.h
            .rt
            .block_on(self.h.engine.clone().handle(Cmd::Apps(cmd)));
    }

    fn send(&self, name: &str, arg: &str) {
        self.run(AppsCmd::parse(name, arg).unwrap_or_else(|| panic!("no command {name}")));
    }

    fn store(&self) -> AppsStore<'_> {
        self.h.app.global::<AppsStore>()
    }

    fn view(&self) -> String {
        self.h.app.get_view().to_string()
    }

    fn find(&self, label: &str) -> Vec<ElementHandle> {
        ElementHandle::find_by_accessible_label(&self.h.app, label).collect()
    }

    fn has_label(&self, label: &str) -> bool {
        !self.find(label).is_empty()
    }

    /// Clicks the first button with this label (a heading or text can carry the same label).
    fn click(&self, label: &str) {
        self.click_with(label, PointerEventButton::Left);
    }

    fn click_with(&self, label: &str, button: PointerEventButton) {
        self.find(label)
            .into_iter()
            .find(|e| e.accessible_role() == Some(AccessibleRole::Button))
            .unwrap_or_else(|| panic!("no button labelled {label:?}"))
            .mock_single_click(button);
    }

    /// Clicks the first element of a component type with this label.
    fn click_in(&self, label: &str, component: &str) {
        self.find(label)
            .into_iter()
            .find(|e| e.type_name().as_deref() == Some(component))
            .unwrap_or_else(|| panic!("no {component} labelled {label:?}"))
            .mock_single_click(PointerEventButton::Left);
    }

    /// Clicks whatever carries the label (a text field, a check box).
    fn click_any(&self, label: &str) {
        self.find(label)
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("nothing labelled {label:?}"))
            .mock_single_click(PointerEventButton::Left);
    }

    /// A menu row (a rail entry or a page can carry the same label).
    fn click_menu(&self, label: &str) {
        self.find(label)
            .into_iter()
            .find(|e| e.type_name().as_deref() == Some("KitMenuRow"))
            .unwrap_or_else(|| panic!("no menu row {label:?}"))
            .mock_single_click(PointerEventButton::Left);
    }

    /// The names on the rail after the fixed entries.
    fn rail(&self) -> Vec<String> {
        self.store()
            .get_rail()
            .iter()
            .map(|r| r.name.to_string())
            .collect()
    }

    fn card_names(cards: ModelRc<crate::AppCard>) -> Vec<String> {
        cards.iter().map(|c| c.name.to_string()).collect()
    }

    fn banner(&self) -> String {
        self.h.app.get_banner().to_string()
    }
}

#[test]
fn an_app_view_loads_itself_when_it_appears() {
    // Like `KELMA_VIEW=store`: the view is switched without a rail click.
    let a = AppsHarness::open();
    a.h.app.set_view("store".into());
    assert!(a.has_label("Search the Store"), "the view is on screen");
    assert_eq!(a.store().get_popular().row_count(), 4);
    a.h.app.set_view("planner".into());
    assert!(a.has_label("Refresh tasks"));
    assert_eq!(a.h.app.global::<TasksStore>().get_title(), "Tasks");
    // Appearing again does not load it twice.
    let loads = |a: &AppsHarness| {
        a.log
            .borrow()
            .iter()
            .filter(|l| l.starts_with("tasks-enter"))
            .count()
    };
    a.h.app.set_view("chat".into());
    assert!(!a.has_label("Refresh tasks"), "the screen is gone");
    a.h.app.set_view("planner".into());
    assert!(a.has_label("Refresh tasks"));
    assert_eq!(loads(&a), 2, "asked twice, answered from what was loaded");
    assert_eq!(a.h.app.global::<TasksStore>().get_tasks().row_count(), 3);
}

// -- S1.1 the app bar ----------------------------------------------------------------------

#[test]
fn the_rail_lists_the_pinned_apps_after_the_fixed_entries_with_store_and_more() {
    let a = AppsHarness::open();
    // Activity, Chat, Teams, Calendar and OneDrive are kelma's own screens; the rest of the
    // user's bar follows in the bar's order.
    assert_eq!(a.rail(), ["Calling", "Planner", "Workflows"]);
    for label in [
        "Activity",
        "Calling",
        "Planner",
        "Workflows",
        "Store",
        "More apps",
    ] {
        assert!(a.has_label(label), "rail entry {label}");
    }
}

#[test]
fn right_click_unpins_and_the_request_is_only_shown() {
    let a = AppsHarness::open();
    a.click_with("Planner", PointerEventButton::Right);
    a.click_menu("Unpin");
    assert_eq!(a.rail(), ["Calling", "Workflows"]);
    let banner = a.banner();
    assert!(banner.contains("Unpinned Planner"), "{banner}");
    assert!(banner.contains("not sent"), "{banner}");
    assert!(banner.contains("DRY RUN POST") && banner.contains("/beta/users/apps/pinned"));
    assert_eq!(
        *a.log.borrow(),
        vec!["unpin com.microsoft.teamspace.tab.planner"]
    );
}

#[test]
fn an_app_the_admin_pinned_cannot_be_unpinned() {
    let a = AppsHarness::open();
    a.click_with("Calling", PointerEventButton::Right);
    assert!(
        a.find("Unpin").is_empty(),
        "no Unpin for an admin pinned app"
    );
    a.click_menu("Open");
    assert_eq!(a.view(), "app");
    a.send("unpin", "demo-calling");
    assert!(
        a.banner().contains("pinned by your admin"),
        "{}",
        a.banner()
    );
    assert_eq!(a.rail(), ["Calling", "Planner", "Workflows"]);
}

#[test]
fn move_down_and_move_up_reorder_the_pinned_apps() {
    let a = AppsHarness::open();
    a.click_with("Workflows", PointerEventButton::Right);
    a.click_menu("Move up");
    assert_eq!(a.rail(), ["Calling", "Workflows", "Planner"]);
    a.click_with("Workflows", PointerEventButton::Right);
    a.click_menu("Move down");
    assert_eq!(a.rail(), ["Calling", "Planner", "Workflows"]);
}

#[test]
fn dragging_a_rail_entry_reorders_it() {
    let a = AppsHarness::open();
    let planner = a.find("Planner").into_iter().next().expect("Planner entry");
    let (pos, size) = (planner.absolute_position(), planner.size());
    let start = LogicalPosition::new(pos.x + size.width / 2., pos.y + size.height / 2.);
    let window = a.h.app.window();
    window.dispatch_event(WindowEvent::PointerMoved { position: start });
    window.dispatch_event(WindowEvent::PointerPressed {
        position: start,
        button: PointerEventButton::Left,
    });
    // One slot (an entry and its gap) down.
    let end = LogicalPosition::new(start.x, start.y + 60.);
    window.dispatch_event(WindowEvent::PointerMoved { position: end });
    window.dispatch_event(WindowEvent::PointerReleased {
        position: end,
        button: PointerEventButton::Left,
    });
    assert_eq!(a.rail(), ["Calling", "Workflows", "Planner"]);
    assert!(
        a.log.borrow().iter().any(|l| l.starts_with("move ")),
        "{:?}",
        a.log.borrow()
    );
    assert_ne!(a.view(), "planner", "a drag is not a click");
}

#[test]
fn ctrl_shift_digits_open_the_nth_entry_and_the_bar_collapses() {
    let a = AppsHarness::open();
    a.h.select(NOTES);
    a.h.type_in_composer("");
    // 1 to 5 are the fixed entries, 6 is the first pinned app.
    a.send("shortcut", "1");
    assert_eq!(a.view(), "activity");
    a.send("shortcut", "7");
    assert_eq!(a.view(), "planner");
    a.send("shortcut", "9");
    assert_eq!(a.view(), "planner", "no ninth entry: nothing changes");

    // The key chord itself: Shift turns the 6 into ^ on a US layout.
    let w = a.h.app.window();
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Shift.into(),
    });
    w.dispatch_event(WindowEvent::KeyPressed { text: "^".into() });
    assert_eq!(a.view(), "app", "Ctrl+Shift+6 opens Calling");
    w.dispatch_event(WindowEvent::KeyReleased { text: "^".into() });
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Shift.into(),
    });
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });

    assert_eq!(a.store().get_rail_width(), 68.);
    a.send("rail-collapse", "");
    assert_eq!(a.store().get_rail_width(), 52.);
    a.send("rail-collapse", "");
    assert_eq!(a.store().get_rail_width(), 68.);
}

// -- S1.2 the flyout ------------------------------------------------------------------------

#[test]
fn more_opens_a_searchable_list_of_installed_apps_that_are_not_pinned() {
    let a = AppsHarness::open();
    a.click("More apps");
    assert!(a.store().get_flyout_open());
    // Installed, not pinned, with a personal tab: not Planner (pinned) or Contoso Tickets (a
    // message extension only).
    let names = AppsHarness::card_names(a.store().get_flyout_apps());
    assert_eq!(names, ["Lists", "OneNote", "Shifts"]);

    type_text(&a.h.app, "note");
    assert_eq!(a.store().get_flyout_query(), "note");
    assert_eq!(
        AppsHarness::card_names(a.store().get_flyout_apps()),
        ["OneNote"]
    );

    type_text(&a.h.app, "zzz");
    assert!(a.store().get_flyout_apps().row_count() == 0);
    assert!(a.has_label("Search apps"));
}

#[test]
fn opening_an_app_from_the_flyout_shows_it_in_the_main_area() {
    let a = AppsHarness::open();
    a.click("More apps");
    a.click("OneNote");
    assert!(!a.store().get_flyout_open(), "the flyout closes");
    assert_eq!(a.view(), "onenote");
    // Shifts has no screen of its own: its page opens in Teams on the web.
    a.click("More apps");
    a.click("Shifts");
    assert_eq!(a.view(), "app");
    assert_eq!(a.store().get_host_name(), "Shifts");
    a.click("Open in Teams on the web");
    let opened = a.h.engine.apps_opened();
    assert!(
        opened
            .iter()
            .any(|u| u.starts_with("https://teams.microsoft.com/l/entity/demo-shifts/")),
        "{opened:?}"
    );
}

#[test]
fn pin_from_the_flyout_adds_the_app_to_the_rail() {
    let a = AppsHarness::open();
    a.click("More apps");
    a.click("Pin Shifts");
    assert_eq!(a.rail(), ["Calling", "Planner", "Workflows", "Shifts"]);
    assert!(a.banner().contains("Pinned Shifts"), "{}", a.banner());
    assert!(a.banner().contains("not sent"));
    // It is on the bar now, so it leaves the flyout.
    a.send("flyout-open", "");
    assert!(!AppsHarness::card_names(a.store().get_flyout_apps()).contains(&"Shifts".to_owned()));
}

// -- S2.1 to S2.4 the Store -------------------------------------------------------------------

#[test]
fn the_store_home_shows_popular_apps_with_their_facts() {
    let a = AppsHarness::open();
    a.click("Store");
    assert_eq!(a.view(), "store");
    assert_eq!(a.store().get_store_page(), "home");
    let popular = a.store().get_popular();
    let names: Vec<String> = popular.iter().map(|c| c.name.to_string()).collect();
    assert_eq!(
        names,
        ["Planner", "OneNote", "Contoso Polls", "Fabrikam Agent"]
    );
    let planner = popular.row_data(0).unwrap();
    assert!(planner.meta.contains("★ 4.5"), "{}", planner.meta);
    assert!(
        planner.installed && planner.update,
        "installed one version behind"
    );
    assert_eq!(
        a.store().get_org_apps().row_count(),
        1,
        "Built for your org"
    );
    assert!(a.has_label("Search the Store"));
    // Devices links out; the gear and Manage open their pages.
    a.click("Devices");
    assert!(
        a.h.engine
            .apps_opened()
            .iter()
            .any(|u| u.contains("microsoft.com"))
    );
}

#[test]
fn search_filters_by_works_in_and_by_apps_or_agents() {
    let a = AppsHarness::open();
    a.send("store-search", "contoso");
    assert_eq!(a.store().get_store_page(), "results");
    assert_eq!(
        AppsHarness::card_names(a.store().get_cards()),
        ["Contoso Tickets", "Contoso Polls"]
    );
    a.send("store-scope", "team");
    assert_eq!(
        AppsHarness::card_names(a.store().get_cards()),
        ["Contoso Polls"]
    );
    a.send("store-scope", "all");

    a.send("store-search", "agent");
    assert_eq!(
        AppsHarness::card_names(a.store().get_cards()),
        ["Fabrikam Agent"]
    );
    a.send("store-kind", "apps");
    assert!(
        a.store().get_cards().row_count() == 0,
        "an agent is not an app"
    );
    assert_eq!(a.store().get_store_status(), "No apps match.");
    a.send("store-kind", "agents");
    assert_eq!(a.store().get_cards().row_count(), 1);

    // The search box sends the query on Enter.
    a.send("store-kind", "all");
    a.send("store-home", "");
    a.click_any("Search the Store");
    type_text(&a.h.app, "polls");
    enter(&a.h.app);
    assert_eq!(a.store().get_store_page(), "results");
    assert_eq!(
        AppsHarness::card_names(a.store().get_cards()),
        ["Contoso Polls"]
    );
    assert!(a.log.borrow().iter().any(|l| l == "store-search polls"));
}

#[test]
fn categories_and_featured_open_a_listing() {
    let a = AppsHarness::open();
    a.send("store-open", "");
    let cats: Vec<String> = a
        .store()
        .get_categories()
        .iter()
        .map(|n| n.label.to_string())
        .collect();
    assert!(cats.contains(&"Productivity".to_owned()), "{cats:?}");
    assert!(!cats.contains(&"All".to_owned()));
    a.send("store-category", "productivity");
    assert_eq!(a.store().get_store_page(), "category");
    assert_eq!(a.store().get_store_title(), "Productivity");
    assert_eq!(
        AppsHarness::card_names(a.store().get_cards()),
        ["Planner", "OneNote", "Lists", "Shifts"]
    );
    a.send("store-category", "org");
    assert_eq!(
        AppsHarness::card_names(a.store().get_cards()),
        ["Contoso Intranet"]
    );
    a.send("store-category", "agents");
    assert_eq!(a.store().get_kind_filter(), "agents");
    assert_eq!(
        AppsHarness::card_names(a.store().get_cards()),
        ["Fabrikam Agent"]
    );
}

#[test]
fn an_app_page_shows_the_listing_facts() {
    let a = AppsHarness::open();
    a.send("store-open", "");
    a.click_in("Planner", "AppCardView");
    let s = a.store();
    assert_eq!(s.get_store_page(), "app");
    assert_eq!(s.get_app_name(), "Planner");
    assert_eq!(s.get_app_developer(), "Microsoft Corporation");
    assert_eq!(s.get_app_rating(), "★ 4.5 (119 ratings)");
    assert_eq!(s.get_app_works_in(), "Personal, Team");
    assert!(s.get_app_installed() && s.get_app_update() && s.get_app_pinned());
    assert_eq!(s.get_app_rsc().row_count(), 1);
    assert!(!s.get_app_privacy().is_empty() && !s.get_app_terms().is_empty());
    for label in [
        "Add to another place",
        "Update",
        "Remove",
        "Share",
        "Privacy policy",
        "Terms of use",
    ] {
        assert!(a.has_label(label), "{label}");
    }

    a.click("Share");
    assert!(a.h.engine.apps_opened().iter().any(
        |u| u == "copied:https://teams.microsoft.com/l/app/com.microsoft.teamspace.tab.planner"
    ));
    a.click("Privacy policy");
    assert!(
        a.h.engine
            .apps_opened()
            .iter()
            .any(|u| u.ends_with("/privacy"))
    );
}

#[test]
fn add_shows_the_request_and_never_sends_it() {
    let a = AppsHarness::open();
    a.send("store-search", "polls");
    a.click("Contoso Polls");
    assert_eq!(a.store().get_app_name(), "Contoso Polls");
    assert!(!a.store().get_app_installed());
    let targets: Vec<String> = a
        .store()
        .get_app_targets()
        .iter()
        .map(|n| n.label.to_string())
        .collect();
    assert_eq!(
        targets,
        ["Add to a team", "Add to a chat"],
        "its works-in decides the menu"
    );

    a.click("Add");
    a.click_menu("Add to a team");
    let s = a.store();
    assert_eq!(s.get_install_title(), "Add Contoso Polls to a team");
    let req = s.get_install_request().to_string();
    assert!(req.contains("DRY RUN POST"), "{req}");
    assert!(
        req.contains("/beta/teams/") && req.contains("/apps/entitlements"),
        "{req}"
    );
    assert!(req.contains("Nothing was sent"), "{req}");
    assert!(req.contains("Pick the team in Teams."), "{req}");
    a.click("Open in Teams on the web");
    assert!(
        a.h.engine
            .apps_opened()
            .iter()
            .any(|u| u.ends_with("/l/app/demo-polls"))
    );
    assert_eq!(s.get_install_title(), "", "the dialog closes");

    // Add for me posts to the user's own scope.
    a.send("app-page", "demo-agent");
    a.send("app-add", "me");
    let req = a.store().get_install_request().to_string();
    assert!(req.contains("/beta/users/apps/entitlements"), "{req}");
}

#[test]
fn manage_your_apps_lists_installed_apps_with_updates_and_shows_remove_as_a_request() {
    let a = AppsHarness::open();
    a.send("store-open", "");
    a.click("Manage your apps");
    assert_eq!(a.store().get_store_page(), "manage");
    let manage = a.store().get_manage();
    let planner = manage
        .iter()
        .find(|c| c.name == "Planner")
        .expect("Planner is installed");
    assert!(planner.update, "1.8.0 installed, 1.9.0 available");
    assert!(manage.iter().any(|c| c.name == "OneNote" && !c.update));

    a.send("app-remove", "com.microsoft.teamspace.tab.planner");
    let s = a.store();
    assert_eq!(s.get_install_title(), "Remove Planner");
    assert!(
        s.get_install_request().contains("DRY RUN DELETE"),
        "{}",
        s.get_install_request()
    );
    a.send("app-update", "com.microsoft.teamspace.tab.planner");
    assert!(
        a.store().get_install_request().contains("/apps/Upgrade/"),
        "{}",
        a.store().get_install_request()
    );
}

#[test]
fn store_settings_show_the_policy_and_per_app_permissions() {
    let a = AppsHarness::open();
    a.send("store-open", "");
    a.send("store-settings", "");
    let s = a.store();
    assert_eq!(s.get_store_page(), "settings");
    assert_eq!(
        s.get_policy_text(),
        "Custom app upload: not allowed. Pinning apps to the bar: allowed."
    );
    let rows: Vec<(String, String, bool)> = s
        .get_perms()
        .iter()
        .map(|p| (p.app_name.to_string(), p.label.to_string(), p.allowed))
        .collect();
    assert_eq!(
        rows,
        [
            ("OneNote".into(), "Camera and microphone".into(), false),
            ("Shifts".into(), "Location".into(), false),
            ("Shifts".into(), "Notifications".into(), false),
        ],
        "everything starts denied"
    );
    a.click_any("Shifts: Location");
    let after: Vec<bool> = a.store().get_perms().iter().map(|p| p.allowed).collect();
    assert_eq!(after, [false, true, false]);
    assert_eq!(
        a.log.borrow().last().unwrap(),
        "grant demo-shifts|geolocation|on"
    );
    a.click_any("Shifts: Location");
    assert!(!a.store().get_perms().row_data(1).unwrap().allowed);
}

// -- S3 the built-in apps ---------------------------------------------------------------------

#[test]
fn planner_opens_to_do_with_a_board_and_plans() {
    let a = AppsHarness::open();
    a.click("Planner");
    assert_eq!(a.view(), "planner");
    let t = a.h.app.global::<TasksStore>();
    let nav: Vec<String> = t.get_nav().iter().map(|r| r.title.to_string()).collect();
    assert_eq!(
        nav,
        [
            "Planner",
            "My tasks",
            "Sprint board",
            "Launch plan",
            "To Do",
            "Tasks",
            "Groceries",
            "Flagged email"
        ]
    );
    // The default To Do list opens first.
    assert_eq!(t.get_title(), "Tasks");
    let tasks: Vec<String> = t.get_tasks().iter().map(|r| r.title.to_string()).collect();
    assert_eq!(
        tasks,
        [
            "Send the quarterly report",
            "Book the team offsite",
            "Review the design doc"
        ]
    );
    let badges: Vec<String> = t.get_tasks().iter().map(|r| r.badge.to_string()).collect();
    assert_eq!(badges, ["Not started", "In progress", "Completed"]);
    assert!(t.get_summary().starts_with("3 tasks"));

    a.click("Sprint board");
    assert_eq!(t.get_title(), "Sprint board");
    a.click("Board");
    assert_eq!(t.get_mode(), "board");
    let cols: Vec<String> = t
        .get_columns()
        .iter()
        .map(|c| c.title.to_string())
        .collect();
    assert_eq!(cols, ["To do", "Doing", "Done"]);
    a.click("List");
    assert_eq!(t.get_mode(), "list");

    a.click("My tasks");
    assert_eq!(t.get_tasks().row_count(), 4);
    // An empty list says so.
    a.click("Flagged email");
    assert_eq!(t.get_tasks().row_count(), 0);
    assert_eq!(t.get_status(), "No tasks here.");
}

#[test]
fn workflows_list_flows_with_their_run_history_and_offer_no_way_to_run_one() {
    let a = AppsHarness::open();
    a.click("Workflows");
    assert_eq!(a.view(), "workflows");
    let f = a.h.app.global::<FlowsStore>();
    let flows: Vec<(String, String)> = f
        .get_flows()
        .iter()
        .map(|r| (r.title.to_string(), r.badge.to_string()))
        .collect();
    assert_eq!(flows.len(), 3);
    assert_eq!(flows[0].1, "On");
    assert_eq!(flows[2].1, "Off");
    // The first flow is selected and its runs shown, newest first.
    let runs: Vec<(String, String)> = f
        .get_runs()
        .iter()
        .map(|r| (r.badge.to_string(), r.tone.to_string()))
        .collect();
    assert_eq!(
        runs,
        [
            ("Succeeded".into(), "ok".into()),
            ("Failed".into(), "bad".into()),
            ("Succeeded".into(), "ok".into())
        ]
    );
    assert_eq!(f.get_environment(), "Demo (default)");
    // A flow that never ran says so.
    a.click("Copy a file to OneDrive");
    assert_eq!(f.get_runs().row_count(), 0);
    assert_eq!(f.get_status(), "This flow has not run yet.");
    // Read only: nothing to turn on or run.
    for label in ["Run", "Turn on", "Turn off", "Run flow", "Delete"] {
        assert!(!a.has_label(label), "{label}");
    }
    assert!(
        a.log
            .borrow()
            .iter()
            .all(|l| l.starts_with("open-app") || l.starts_with("flows-")),
        "{:?}",
        a.log.borrow()
    );
}

#[test]
fn onenote_reads_notebooks_sections_pages_and_page_text() {
    let a = AppsHarness::open();
    a.send("open-app", "demo-onenote");
    assert_eq!(a.view(), "onenote");
    let n = a.h.app.global::<NotesStore>();
    assert_eq!(n.get_notebooks().row_count(), 1);
    let sections: Vec<String> = n
        .get_sections()
        .iter()
        .map(|r| r.title.to_string())
        .collect();
    assert_eq!(sections, ["Meeting notes", "Ideas"]);
    // The first section's pages are listed.
    let pages: Vec<String> = n.get_pages().iter().map(|r| r.title.to_string()).collect();
    assert_eq!(pages, ["Weekly sync", "Retro"]);
    a.click("Weekly sync");
    assert_eq!(n.get_page_title(), "Weekly sync");
    assert!(
        n.get_page_text().contains("Ship the Store view & Planner"),
        "{}",
        n.get_page_text()
    );
    // An empty section says so.
    a.click("Ideas");
    assert_eq!(n.get_pages().row_count(), 0);
    assert_eq!(n.get_status(), "This section has no pages.");
}

#[test]
fn lists_show_a_lists_columns_and_items() {
    let a = AppsHarness::open();
    a.send("open-app", "demo-lists");
    assert_eq!(a.view(), "lists");
    let l = a.h.app.global::<ListsStore>();
    assert_eq!(l.get_sites().row_count(), 1);
    // The user list opens before the document library.
    assert_eq!(l.get_title(), "Issue tracker");
    let cols: Vec<String> = l.get_columns().iter().map(|c| c.to_string()).collect();
    assert_eq!(cols, ["Title", "Status", "Owner", "Done"]);
    assert_eq!(l.get_items().row_count(), 3);
    let first = l.get_items().row_data(0).unwrap().cells;
    assert_eq!(first.row_data(0).unwrap(), "Login fails on mobile");
    a.click("Documents");
    assert_eq!(l.get_title(), "Documents");
}
