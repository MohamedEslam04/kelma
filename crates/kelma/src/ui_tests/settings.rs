//! Settings and the profile menu driven through the real window (plans/settings-and-platform.md
//! X1, X2.1; chat plan 9.1, 9.3, 9.9; activity plan A1.5). Each test gets its own temporary
//! KELMA_HOME-style directory (`<tmp>/config`, `data`, `cache`), an in-memory token store and
//! hooks that only record what they were asked to do. There is no engine and no async runtime
//! here, so nothing can reach the network.

use super::*;
use crate::backend::settings::{DeviceLists, Local, ServerData};
use crate::ui::settings::{Autostart, Hooks, MicPoll, SettingsEnv, install};
use crate::{SettingsStore, Theme};
use i_slint_backend_testing::AccessibleRole;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use teams_proto::user_settings::{NotificationSettings, PrivacySettings};

/// A temporary home, removed when dropped.
struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Home {
        let d = std::env::temp_dir().join(format!(
            "kelma-ui-settings-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Home(d)
    }

    /// The same layout `session::Paths` uses under `KELMA_HOME`.
    fn paths(&self) -> session::Paths {
        session::Paths {
            config_dir: self.0.join("config"),
            data_dir: self.0.join("data"),
            cache_dir: self.0.join("cache"),
        }
    }

    /// A config with two fake accounts; the first is active.
    fn with_accounts(self) -> Home {
        let p = self.paths();
        let mut cfg = session::Config {
            display_name: Some("Test User".into()),
            ..Default::default()
        };
        cfg.normalize();
        cfg.accounts[0].upn = Some("test.user@example.invalid".into());
        cfg.accounts[0].orgs = vec![session::OrgRef {
            tenant_id: "home-tenant".into(),
            name: "Example Org".into(),
            is_home: true,
        }];
        cfg.add_account("second").unwrap();
        p.save_config(&cfg).unwrap();
        self
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn env(home: &Home, log: &Log, tokens: Arc<session::MemoryStore>) -> SettingsEnv {
    let rec = |l: &Log, what: &'static str| {
        let l = l.clone();
        move || l.borrow_mut().push(what.into())
    };
    let l2 = log.clone();
    SettingsEnv {
        paths: home.paths(),
        tokens,
        portal: false,
        background: false,
        hooks: Hooks {
            // The real autostart entry, written under the temp home instead of ~/.config.
            autostart: Autostart::desktop(Some(home.0.join("xdg"))),
            test_notification: Box::new(rec(log, "test-notification")),
            open_url: Box::new(move |u| l2.borrow_mut().push(format!("open {u}"))),
            restart: Box::new(rec(log, "restart")),
            signed_out: Box::new(rec(log, "signed-out")),
            report_problem: Arc::new(|| Ok("/tmp/report.tar.gz".into())),
            enumerate: Arc::new(|| DeviceLists {
                inputs: vec![devices::AudioDevice {
                    id: "alsa:default".into(),
                    name: "Default microphone".into(),
                    is_default: true,
                    channels: 1,
                    sample_rates: vec![48_000],
                }],
                outputs: vec![devices::AudioDevice {
                    id: "alsa:pipewire".into(),
                    name: "PipeWire".into(),
                    is_default: false,
                    channels: 2,
                    sample_rates: vec![48_000],
                }],
                cameras: vec![],
                error: None,
            }),
            test_sound: Arc::new(|_| Ok(())),
            mic_level: Box::new(|_| Ok(Box::new(|| Some(0.5_f32)) as MicPoll)),
            // Never the network: a check that was not given a fake answer fails.
            check_update: Arc::new(|| Err("no network in tests".into())),
        },
    }
}

struct Ui {
    app: AppWindow,
    log: Log,
    tokens: Arc<session::MemoryStore>,
    home: Home,
}

fn window(home: Home) -> Ui {
    window_with(home, |_| {})
}

/// `window` with a change to the environment (a fake update check, say).
fn window_with(home: Home, tweak: impl FnOnce(&mut SettingsEnv)) -> Ui {
    let app = new_window();
    app.window().set_size(LogicalSize::new(1200., 800.));
    app.set_login_state("ready".into());
    app.set_me_initials("TU".into());
    let log = Log::default();
    let tokens = Arc::new(session::MemoryStore::default());
    let mut e = env(&home, &log, tokens.clone());
    tweak(&mut e);
    install(&app, None, e);
    app.show().unwrap();
    Ui {
        app,
        log,
        tokens,
        home,
    }
}

fn store(app: &AppWindow) -> SettingsStore<'_> {
    app.global::<SettingsStore>()
}

/// The one element with this label that is not plain text (a control and its caption share it).
fn control(app: &AppWindow, label: &str) -> ElementHandle {
    let v: Vec<ElementHandle> = ElementHandle::find_by_accessible_label(app, label)
        .filter(|e| e.accessible_role() != Some(AccessibleRole::Text))
        .collect();
    assert_eq!(
        v.len(),
        1,
        "expected one control labelled {label:?}, found {}",
        v.len()
    );
    v.into_iter().next().unwrap()
}

fn press(app: &AppWindow, label: &str) {
    control(app, label).invoke_accessible_default_action();
}

fn value_of(app: &AppWindow, label: &str) -> String {
    ElementHandle::find_by_accessible_label(app, label)
        .next()
        .unwrap_or_else(|| panic!("no element labelled {label:?}"))
        .accessible_value()
        .unwrap_or_default()
        .to_string()
}

fn checked(app: &AppWindow, label: &str) -> bool {
    control(app, label).accessible_checked().unwrap_or(false)
}

fn open_page(app: &AppWindow, page: &str) {
    store(app).invoke_show(page.into());
}

#[test]
fn profile_menu_shows_who_is_signed_in_and_opens_settings() {
    let ui = window(Home::new("profile").with_accounts());
    let app = &ui.app;
    press(app, "Profile menu");
    assert!(store(app).get_profile_open());
    assert_eq!(value_of(app, "Name"), "Test User");
    assert_eq!(value_of(app, "Email"), "test.user@example.invalid");
    assert_eq!(value_of(app, "Status message"), "", "read-only, none set");
    let photo = control(app, "Change profile photo (not available yet)");
    assert_eq!(photo.accessible_enabled(), Some(false));

    press(app, "Settings");
    assert!(!store(app).get_profile_open());
    assert!(store(app).get_open());
    assert_eq!(value_of(app, "Settings page"), "General");
}

#[test]
fn activity_notification_settings_entry_opens_the_notifications_page() {
    let ui = window(Home::new("activity"));
    let app = &ui.app;
    app.set_view("activity".into());
    ElementHandle::find_by_element_id(app, "ActivityView::more")
        .next()
        .unwrap()
        .mock_single_click(PointerEventButton::Left);
    ElementHandle::find_by_element_id(app, "ActivityView::settings-entry")
        .next()
        .unwrap()
        .mock_single_click(PointerEventButton::Left);
    assert!(store(app).get_open());
    assert_eq!(store(app).get_page(), "notifications");
    assert_eq!(value_of(app, "Settings page"), "Notifications and activity");
    // Not read from Teams yet: the types are listed, nothing is selected.
    assert!(!checked(app, "Chat messages: Off"));
}

#[test]
fn the_page_list_and_find_in_settings_switch_pages() {
    let ui = window(Home::new("nav"));
    let app = &ui.app;
    open_page(app, "general");
    for (tab, title) in [
        ("Privacy", "Privacy"),
        ("Devices", "Devices"),
        ("Accounts and orgs", "Accounts and orgs"),
        ("About", "About"),
        ("Appearance", "Appearance"),
    ] {
        press(app, tab);
        assert_eq!(value_of(app, "Settings page"), title);
        assert!(checked(app, tab), "{tab} is the selected tab");
    }
    // The Devices page listed the (fake) devices, raw entries hidden.
    press(app, "Devices");
    assert!(
        ElementHandle::find_by_accessible_label(app, "Speaker: PipeWire")
            .next()
            .is_some()
    );

    // Find in Settings.
    let find = control(app, "Find in Settings");
    find.set_accessible_value("captions");
    assert!(store(app).get_found().row_count() >= 3);
    press(app, "Result: Always show captions in Accessibility");
    assert_eq!(store(app).get_query(), "");
    assert_eq!(value_of(app, "Settings page"), "Accessibility");

    store(app).invoke_close();
    assert!(!store(app).get_open());
}

#[test]
fn theme_choice_changes_the_theme_global_live() {
    let ui = window(Home::new("theme"));
    let app = &ui.app;
    open_page(app, "appearance");
    press(app, "Theme: Light");
    let t = app.global::<Theme>();
    assert!(!t.get_dark());
    assert_eq!(t.get_main(), slint::Color::from_rgb_u8(0xff, 0xff, 0xff));
    assert_eq!(t.get_fg(), slint::Color::from_rgb_u8(0x24, 0x24, 0x24));
    assert!(checked(app, "Theme: Light"));
    assert_eq!(store(app).get_theme_shown(), "light");

    press(app, "Theme: Dark");
    assert!(t.get_dark());
    assert_eq!(t.get_main(), slint::Color::from_rgb_u8(0x14, 0x14, 0x14));

    // An accent colour and high contrast reach the same global.
    press(app, "Accent colour: Blue");
    assert_eq!(
        t.get_accent_strong(),
        slint::Color::from_rgb_u8(0x0f, 0x6c, 0xbd)
    );
    open_page(app, "accessibility");
    press(app, "High contrast: On");
    assert!(t.get_high_contrast());
    assert_eq!(t.get_main(), slint::Color::from_rgb_u8(0, 0, 0));
    assert_eq!(t.get_border(), slint::Color::from_rgb_u8(0xff, 0xff, 0xff));
}

#[test]
fn font_size_and_larger_text_scale_the_theme_font() {
    let ui = window(Home::new("font"));
    let app = &ui.app;
    open_page(app, "appearance");
    let t = app.global::<Theme>();
    assert_eq!(t.get_font_scale(), 1.0);
    press(app, "Larger font");
    press(app, "Larger font");
    assert!((t.get_font_scale() - 1.1).abs() < 1e-6);
    assert_eq!(value_of(app, "Current font size"), "110%");
    press(app, "Smaller font");
    assert!((t.get_font_scale() - 1.05).abs() < 1e-6);
    // The window's default font follows the scale.
    open_page(app, "accessibility");
    press(app, "Larger text");
    assert!((t.get_font_scale() - 1.3125).abs() < 1e-5);
    open_page(app, "appearance");
    press(app, "Reset font size");
    assert!(
        (t.get_font_scale() - 1.25).abs() < 1e-6,
        "larger text stays on"
    );
}

#[test]
fn local_preferences_round_trip_through_the_temp_home() {
    let home = Home::new("roundtrip");
    let dir = home.paths().config_dir;
    let ui = window(home);
    let app = &ui.app;
    open_page(app, "appearance");
    press(app, "Message density: Compact");
    assert!(app.global::<Theme>().get_compact());
    open_page(app, "files");
    press(app, "Open Word, Excel and PowerPoint files in: Desktop app");
    open_page(app, "general");
    press(app, "Time format: 13:30");
    press(app, "Start kelma when I sign in");
    open_page(app, "devices");
    press(app, "Microphone: Default microphone (default)");

    let saved = Local::load(&dir);
    assert_eq!(saved.appearance.density, appearance::Density::Compact);
    assert_eq!(
        saved.files.open_office_in,
        crate::backend::settings::OpenFilesIn::Desktop
    );
    assert_eq!(saved.general.time_format, appearance::TimeFormat::H24);
    assert!(saved.general.auto_start);
    assert_eq!(saved.devices.microphone.as_deref(), Some("alsa:default"));
    assert!(dir.join("appearance.json").is_file());
    // The toggle wrote the real autostart entry (under the temp home), starting hidden.
    let entry = tray::autostart::entry_path_in(&ui.home.0.join("xdg"));
    let desktop = std::fs::read_to_string(&entry).unwrap();
    assert!(desktop.contains("--hidden"), "{desktop}");
    assert!(ui.log.borrow().is_empty());

    // A new window over the same home starts with the saved choices.
    let again = window(Home(ui.home.0.clone()));
    assert_eq!(store(&again.app).get_density(), "compact");
    assert_eq!(store(&again.app).get_time_format(), "h24");
    assert!(again.app.global::<Theme>().get_compact());
    std::mem::forget(again.home); // `ui.home` removes the directory
}

fn server(privacy: PrivacySettings) -> ServerData {
    ServerData {
        chat_service: "https://chat.invalid".into(),
        middle_tier: "https://mt.invalid".into(),
        notifications: Some(NotificationSettings::from_properties(
            &serde_json::json!({}),
        )),
        privacy: Some(privacy),
        blocked: vec![("8:orgid:blocked-1".into(), "Blocked Person".into())],
        me_name: "Test User".into(),
        ..Default::default()
    }
}

#[test]
fn privacy_toggle_plans_a_dry_run_and_changes_nothing() {
    let ui = window(Home::new("privacy"));
    let app = &ui.app;
    open_page(app, "privacy");
    // Nothing read from Teams: the switches are disabled.
    assert_eq!(
        control(app, "Read receipts").accessible_enabled(),
        Some(false)
    );

    UiHandle::new(app.as_weak()).settings_server(server(PrivacySettings {
        read_receipts: true,
        ..Default::default()
    }));
    assert!(checked(app, "Read receipts"));
    press(app, "Read receipts");

    let runs: Vec<String> = store(app)
        .get_dry_runs()
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        runs,
        [
            "PUT https://chat.invalid/v1/users/ME/properties?name=readReceiptsEnabled\n{\"readReceiptsEnabled\":\"false\"}"
        ]
    );
    assert!(
        store(app)
            .get_notice()
            .starts_with("Dry run: Teams was not changed.")
    );
    assert!(
        checked(app, "Read receipts"),
        "Teams still has read receipts on"
    );

    press(app, "Unblock Blocked Person");
    let first = store(app).get_dry_runs().row_data(0).unwrap();
    assert_eq!(
        first,
        "POST https://mt.invalid/beta/userSettings/blocklist/manage\n{\"remove\":[\"8:orgid:blocked-1\"]}"
    );

    // A notification type is planned the same way.
    open_page(app, "notifications");
    press(app, "Chat messages: Off");
    let first = store(app).get_dry_runs().row_data(0).unwrap();
    assert!(
        first.starts_with(
            "PUT https://chat.invalid/v1/users/ME/properties?name=userPersonalSettings"
        )
    );
    assert!(!checked(app, "Chat messages: Off"), "not changed in Teams");
    assert_eq!(store(app).get_dry_runs().row_count(), 3);
}

#[test]
fn sign_out_removes_the_tokens_after_confirming() {
    let ui = window(Home::new("signout").with_accounts());
    let app = &ui.app;
    use session::TokenStore;
    ui.tokens.set("default", "fake-refresh-token").unwrap();
    press(app, "Profile menu");
    press(app, "Sign out");
    assert_eq!(ui.tokens.keys(), ["default"], "asks first");
    press(app, "Cancel");
    press(app, "Sign out");
    press(app, "Yes, sign out");
    assert!(ui.tokens.keys().is_empty());
    assert_eq!(*ui.log.borrow(), ["signed-out"]);
}

#[test]
fn switching_account_or_org_saves_it_and_asks_for_a_restart() {
    let home = Home::new("switch").with_accounts();
    let paths = home.paths();
    let ui = window(home);
    let app = &ui.app;
    open_page(app, "accounts");
    assert!(checked(app, "Account: Test User"));
    assert!(checked(app, "Organization: Example Org"));
    assert!(!store(app).get_restart_needed());

    press(app, "Account: second");
    assert!(store(app).get_restart_needed());
    assert_eq!(paths.load_config().unwrap().account, "second");
    assert!(checked(app, "Account: second"));
    press(app, "Restart kelma");
    assert_eq!(*ui.log.borrow(), ["restart"]);
}

#[test]
fn autostart_switch_follows_and_writes_the_desktop_entry() {
    let home = Home::new("autostart");
    let xdg = home.0.join("xdg");
    // An entry added outside kelma shows as on.
    tray::autostart::enable_in(&xdg, "kelma", None).unwrap();
    let ui = window(home);
    let app = &ui.app;
    open_page(app, "general");
    assert!(checked(app, "Start kelma when I sign in"));
    press(app, "Start kelma when I sign in");
    assert!(!tray::autostart::is_enabled_in(&xdg));
    assert!(!checked(app, "Start kelma when I sign in"));
    press(app, "Start kelma when I sign in");
    assert!(tray::autostart::is_enabled_in(&xdg));
    let desktop = std::fs::read_to_string(tray::autostart::entry_path_in(&xdg)).unwrap();
    assert!(desktop.contains("--hidden"));
}

const FAKE_RELEASE_URL: &str = "https://github.com/MohamedEslam04/kelma/releases/tag/v999.0.0";

/// A fake update check that counts its calls and answers `r` (never the network).
fn fake_update(
    calls: &Arc<AtomicUsize>,
    r: Result<crate::update_check::Release, String>,
) -> impl FnOnce(&mut SettingsEnv) {
    let calls = calls.clone();
    move |e: &mut SettingsEnv| {
        e.hooks.check_update = Arc::new(move || {
            calls.fetch_add(1, Ordering::SeqCst);
            r.clone()
        });
    }
}

fn release(version: &str) -> crate::update_check::Release {
    crate::update_check::Release {
        version: version.into(),
        url: FAKE_RELEASE_URL.into(),
    }
}

#[test]
fn update_check_is_off_by_default_and_the_switch_stores_the_pref() {
    let home = Home::new("update-off");
    let dir = home.paths().config_dir;
    let calls = Arc::new(AtomicUsize::new(0));
    let ui = window_with(home, fake_update(&calls, Ok(release("999.0.0"))));
    let app = &ui.app;
    open_page(app, "about");
    assert!(!checked(app, "Check for updates"));
    assert!(!store(app).get_check_updates());
    assert!(!Local::load(&dir).general.check_updates);
    assert_eq!(calls.load(Ordering::SeqCst), 0, "off: no check at start");
    assert_eq!(app.get_banner().as_str(), "");
    assert!(
        ElementHandle::find_by_accessible_label(app, "Check now")
            .next()
            .is_none(),
        "no Check now while the switch is off"
    );

    press(app, "Check for updates");
    assert!(checked(app, "Check for updates"));
    assert!(Local::load(&dir).general.check_updates);
    press(app, "Check for updates");
    assert!(!Local::load(&dir).general.check_updates);
    assert_eq!(store(app).get_update_status().as_str(), "");
}

#[test]
fn a_newer_release_shows_the_banner_and_release_notes_opens_its_page() {
    let home = Home::new("update-newer");
    let calls = Arc::new(AtomicUsize::new(0));
    let ui = window_with(home, fake_update(&calls, Ok(release("999.0.0"))));
    let app = &ui.app;
    open_page(app, "about");
    press(app, "Check for updates");
    // Turning it on makes the one automatic check of this start.
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(app.get_banner().as_str(), "kelma 999.0.0 is available");
    assert_eq!(app.get_banner_action().as_str(), "Release notes");
    assert!(app.get_banner_info());
    assert_eq!(
        store(app).get_update_status().as_str(),
        "Version 999.0.0 available"
    );

    app.invoke_banner_action_clicked();
    assert_eq!(*ui.log.borrow(), [format!("open {FAKE_RELEASE_URL}")]);

    // "Check now" is an explicit extra request; the switch alone does not repeat it.
    press(app, "Check now");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn the_automatic_check_runs_once_at_start_when_the_pref_is_on() {
    let home = Home::new("update-start");
    let dir = home.paths().config_dir;
    std::fs::create_dir_all(&dir).unwrap();
    let mut local = Local::load(&dir);
    local.general.check_updates = true;
    local.save_general().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let ui = window_with(
        home,
        fake_update(&calls, Ok(release(env!("CARGO_PKG_VERSION")))),
    );
    let app = &ui.app;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    open_page(app, "about");
    assert!(checked(app, "Check for updates"));
    assert_eq!(store(app).get_update_status().as_str(), "Up to date");
    assert_eq!(app.get_banner().as_str(), "", "same version: no banner");
    // Switching off and on again does not make a second automatic request.
    press(app, "Check for updates");
    press(app, "Check for updates");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_failed_check_only_says_so_on_the_about_page() {
    let home = Home::new("update-fail");
    let calls = Arc::new(AtomicUsize::new(0));
    let ui = window_with(home, fake_update(&calls, Err("offline".into())));
    let app = &ui.app;
    open_page(app, "about");
    press(app, "Check for updates");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        store(app).get_update_status().as_str(),
        "Could not check for updates"
    );
    assert_eq!(app.get_banner().as_str(), "");
    assert!(!store(app).get_update_available());
}
