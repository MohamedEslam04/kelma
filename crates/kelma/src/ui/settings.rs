//! UI-thread side of Settings and the profile menu: fills the `SettingsStore` global, applies the
//! appearance and accessibility preferences to the `Theme` global (live, including desktop portal
//! changes), saves local preferences, and turns Teams changes into dry runs.
//!
//! Nothing here talks to Teams. Reads go through the engine (`SettingsCmd::Load`); every Teams
//! write is built as a `PlannedRequest`, logged and shown on the page, never sent, except my own
//! status and work location from the profile menu, which the engine sets (`SettingsCmd::SetStatus`).
//!
//! Commands from `SettingsStore.command(name, argument)`:
//! open, close, page, query, profile, theme, accent, density, font-step, font-scale, skin-tone,
//! language, date-format, time-format, time-zone, startup, high-contrast, reduced-motion,
//! larger-text, a11y, notify, pause, test-notification, privacy, unblock, priority-remove,
//! speaker, mic, camera, noise-suppression, echo-cancellation, all-devices, test-sound, mic-test,
//! open-files, perm, switch-account, switch-org, add-account, remove-account, refresh-license,
//! restart, sign-out, view-account, report-problem, check-updates, check-now, release-notes,
//! status, work-location.

use super::{UiHandle, model, rgb};
use crate::backend::Cmd;
use crate::backend::settings::{
    self as st, DeviceLists, Local, Look, OpenFilesIn, ServerData, SettingsCmd,
};
use crate::update_check::{self, Release};
use crate::{
    AppWindow, SAccount, SAppPerm, SDevice, SFind, SNotify, SOption, SOrg, SPerson, SToggle,
    SettingsStore, Theme,
};
use app_store::DevicePermission;
use appearance::{Density, PortalChange, PortalSettings, Rgb, Theme as ThemeChoice};
use slint::{ComponentHandle, ModelRc, SharedString};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use teams_proto::user_settings::{ContactList, Delivery, Kind, NotificationSettings, PrivacyFlag};

/// Polls a running microphone test; `None` when there is no new reading.
pub type MicPoll = Box<dyn FnMut() -> Option<f32>>;

/// Opens a microphone (`None`: the default one) for the level meter.
pub type OpenMic = Box<dyn Fn(Option<&str>) -> Result<MicPoll, String>>;

/// Plays the test sound on an output (`None`: the default one).
pub type PlaySound = Arc<dyn Fn(Option<String>) -> Result<(), String> + Send + Sync>;

/// Turns the autostart entry on or off, and says whether it is on.
pub struct Autostart {
    pub set: Box<dyn Fn(bool) -> Result<(), String>>,
    pub get: Box<dyn Fn() -> bool>,
}

impl Autostart {
    /// The desktop autostart entry (`tray::autostart`), in `$XDG_CONFIG_HOME/autostart`, or under
    /// `config_home` when given (tests). kelma starts with `--hidden` from it.
    pub fn desktop(config_home: Option<std::path::PathBuf>) -> Autostart {
        let home = config_home.clone();
        Autostart {
            set: Box::new(move |on| {
                let exe = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.to_str().map(str::to_owned))
                    .unwrap_or_else(|| "kelma".into());
                let r = match (&home, on) {
                    (Some(h), true) => {
                        tray::autostart::enable_in(h, &exe, Some("--hidden")).map(|_| ())
                    }
                    (Some(h), false) => tray::autostart::disable_in(h),
                    (None, true) => tray::autostart::enable(&exe, Some("--hidden")).map(|_| ()),
                    (None, false) => tray::autostart::disable(),
                };
                r.map_err(|e| e.to_string())
            }),
            get: Box::new(move || match &config_home {
                Some(h) => tray::autostart::is_enabled_in(h),
                None => tray::autostart::is_enabled(),
            }),
        }
    }
}

/// What Settings does outside the window. `system()` has the real ones; tests record calls.
pub struct Hooks {
    pub autostart: Autostart,
    pub test_notification: Box<dyn Fn()>,
    pub open_url: Box<dyn Fn(&str)>,
    /// Starts kelma again and quits this one.
    pub restart: Box<dyn Fn()>,
    /// Runs after the tokens were removed: the app closes.
    pub signed_out: Box<dyn Fn()>,
    pub report_problem: Arc<dyn Fn() -> Result<String, String> + Send + Sync>,
    pub enumerate: Arc<dyn Fn() -> DeviceLists + Send + Sync>,
    pub test_sound: PlaySound,
    pub mic_level: OpenMic,
    /// Asks for the latest release (blocking, runs on a thread). Only called when the user has
    /// turned the update check on.
    pub check_update: Arc<dyn Fn() -> Result<Release, String> + Send + Sync>,
}

pub struct SettingsEnv {
    /// Config, data and cache dirs (`KELMA_HOME` applies, as everywhere).
    pub paths: session::Paths,
    /// Where sign-out removes the tokens from.
    pub tokens: Arc<dyn session::TokenStore>,
    /// Read and watch the desktop portal for the system theme, accent and contrast.
    pub portal: bool,
    /// Run slow work (device lists, sign-out, reports) on threads. Off in tests: inline.
    pub background: bool,
    /// Apply the date and time format and "keep running on close" to the process-wide settings
    /// that the time helpers and the tray read. Off in tests, which share one process.
    pub apply_globals: bool,
    pub hooks: Hooks,
}

impl SettingsEnv {
    /// The real environment of the app.
    pub fn system() -> Option<SettingsEnv> {
        let paths = session::Paths::new().ok()?;
        let report_paths = session::Paths::new().ok()?;
        Some(SettingsEnv {
            paths,
            tokens: Arc::new(session::SystemKeyring),
            portal: true,
            background: true,
            apply_globals: true,
            hooks: Hooks {
                autostart: Autostart::desktop(None),
                test_notification: Box::new(|| {
                    let _ = notify_rust::Notification::new()
                        .appname("kelma")
                        .summary("kelma")
                        .body("This is a test notification.")
                        .show();
                }),
                open_url: Box::new(|u| {
                    let _ = crate::external::open(u);
                }),
                restart: Box::new(|| {
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe).spawn();
                    }
                    let _ = slint::quit_event_loop();
                }),
                signed_out: Box::new(|| {
                    let _ = slint::quit_event_loop();
                }),
                report_problem: Arc::new(move || {
                    diag::report_from_args(&report_paths, env!("CARGO_PKG_VERSION"), &[])
                        .map(|p| p.display().to_string())
                        .map_err(|e| e.to_string())
                }),
                enumerate: Arc::new(|| {
                    let mut l = DeviceLists::default();
                    let mut errs = vec![];
                    match devices::audio_inputs() {
                        Ok(v) => l.inputs = v,
                        Err(e) => errs.push(e.to_string()),
                    }
                    match devices::audio_outputs() {
                        Ok(v) => l.outputs = v,
                        Err(e) => errs.push(e.to_string()),
                    }
                    match devices::cameras() {
                        Ok(v) => l.cameras = v,
                        Err(e) => errs.push(e.to_string()),
                    }
                    l.error = (!errs.is_empty()).then(|| errs.join("; "));
                    l
                }),
                test_sound: Arc::new(|out| {
                    devices::test_sound(out.as_deref()).map_err(|e| e.to_string())
                }),
                mic_level: Box::new(|input| {
                    let s = devices::mic_level(input).map_err(|e| e.to_string())?;
                    Ok(Box::new(move || s.latest().map(|l| l.peak)) as MicPoll)
                }),
                check_update: Arc::new(update_check::fetch_latest),
            },
        })
    }
}

struct Ctl {
    env: SettingsEnv,
    local: Local,
    portal: Option<PortalSettings>,
    server: Option<ServerData>,
    tx: Option<tokio::sync::mpsc::UnboundedSender<Cmd>>,
    last_load: Option<Instant>,
    dry_runs: Vec<String>,
    lists: Option<DeviceLists>,
    listing: bool,
    mic: Option<slint::Timer>,
    /// An update check is running.
    checking: bool,
    /// The one automatic check of this start has been made.
    update_auto_done: bool,
    /// The release page of the newer version found, for "Release notes".
    update_url: Option<String>,
}

type Shared = Rc<RefCell<Ctl>>;

thread_local! {
    static CTL: RefCell<Option<Shared>> = const { RefCell::new(None) };
}

fn ctl() -> Option<Shared> {
    CTL.with(|c| c.borrow().clone())
}

fn s(v: impl Into<SharedString>) -> SharedString {
    v.into()
}

fn bit(a: &str) -> bool {
    a == "1" || a == "true"
}

/// Wires Settings with the real environment (`main`).
pub fn install_system(window: &AppWindow, tx: tokio::sync::mpsc::UnboundedSender<Cmd>) {
    match SettingsEnv::system() {
        Some(env) => install(window, Some(tx), env),
        None => tracing::warn!("settings: no config directory, Settings stays empty"),
    }
}

/// Loads the local preferences, applies them to the theme, and wires `SettingsStore`. The
/// Activity "…" › Notification settings entry opens Settings on Notifications.
pub fn install(
    window: &AppWindow,
    tx: Option<tokio::sync::mpsc::UnboundedSender<Cmd>>,
    env: SettingsEnv,
) {
    let mut local = Local::load(&env.paths.config_dir);
    // The autostart entry is the truth: it may have been added or removed outside kelma.
    local.general.auto_start = (env.hooks.autostart.get)();
    let portal = if env.portal { read_portal() } else { None };
    let watch = env.portal && env.background;
    if env.apply_globals {
        apply_general(&local.general);
    }
    let shared: Shared = Rc::new(RefCell::new(Ctl {
        env,
        local,
        portal,
        server: None,
        tx,
        last_load: None,
        dry_runs: Vec::new(),
        lists: None,
        listing: false,
        mic: None,
        checking: false,
        update_auto_done: false,
        update_url: None,
    }));
    CTL.with(|c| *c.borrow_mut() = Some(shared.clone()));
    {
        let c = shared.borrow();
        apply_look(window, &c);
        push_local(window, &c);
        push_server(window, &c);
        push_accounts(window, &c);
    }
    window
        .global::<SettingsStore>()
        .set_version(s(env!("CARGO_PKG_VERSION")));
    // Opt-in: only when the switch is on, and then once per start.
    check_updates(window, &mut shared.borrow_mut(), false);
    let release_shared = shared.clone();
    window.on_banner_action_clicked(move || {
        let c = release_shared.borrow();
        if let Some(url) = &c.update_url {
            (c.env.hooks.open_url)(url);
        }
    });

    let weak = window.as_weak();
    window
        .global::<SettingsStore>()
        .on_command(move |name, arg| {
            if let Some(w) = weak.upgrade() {
                command(&shared, &w, &name, &arg);
            }
        });
    let weak = window.as_weak();
    window.on_activity_settings(move || {
        if let Some(w) = weak.upgrade() {
            w.global::<SettingsStore>().invoke_show(s("notifications"));
        }
    });
    if watch {
        watch_portal(window.as_weak());
    }
}

/// The portal state at start, so the first frame already has the right theme.
fn read_portal() -> Option<PortalSettings> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    rt.block_on(async {
        tokio::time::timeout(Duration::from_millis(400), appearance::portal::read())
            .await
            .ok()?
            .inspect_err(|e| tracing::debug!(%e, "settings: no portal"))
            .ok()
            .flatten()
    })
}

/// Follows portal changes (theme, accent, contrast, animations) on a thread of its own.
fn watch_portal(weak: slint::Weak<AppWindow>) {
    std::thread::Builder::new()
        .name("kelma-portal".into())
        .spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            rt.block_on(async move {
                use futures_util::StreamExt;
                let Ok(Some(mut stream)) = appearance::portal::watch().await else {
                    return;
                };
                while let Some(change) = stream.next().await {
                    let _ = weak.upgrade_in_event_loop(move |w| portal_changed(&w, change));
                }
            });
        })
        .ok();
}

fn portal_changed(w: &AppWindow, change: PortalChange) {
    let Some(shared) = ctl() else { return };
    let mut c = shared.borrow_mut();
    c.portal.get_or_insert_with(Default::default).apply(change);
    apply_look(w, &c);
}

/// Sets the `Theme` global from the preferences and the portal.
fn apply_look(w: &AppWindow, c: &Ctl) {
    let lk: Look = st::look(&c.local, c.portal.as_ref());
    let t = w.global::<Theme>();
    let p = lk.palette;
    t.set_rail(rgb(p.rail));
    t.set_sidebar(rgb(p.sidebar));
    t.set_main(rgb(p.main));
    t.set_topbar(rgb(p.topbar));
    t.set_surface(rgb(p.surface));
    t.set_surface_hover(rgb(p.surface_hover));
    t.set_selected(rgb(p.selected));
    t.set_border(rgb(p.border));
    t.set_accent(rgb(p.accent));
    t.set_accent_strong(rgb(p.accent_strong));
    t.set_fg(rgb(p.fg));
    t.set_fg_muted(rgb(p.fg_muted));
    t.set_fg_subtle(rgb(p.fg_subtle));
    t.set_bubble_mine(rgb(p.bubble_mine));
    t.set_bubble_other(rgb(p.bubble_other));
    t.set_mention(rgb(p.mention));
    t.set_danger(rgb(p.danger));
    t.set_unread_badge(rgb(p.unread_badge));
    t.set_dark(lk.dark);
    t.set_font_scale(lk.font_scale);
    t.set_compact(lk.compact);
    t.set_high_contrast(lk.high_contrast);
    t.set_reduced_motion(lk.reduced_motion);

    let g = w.global::<SettingsStore>();
    g.set_theme_shown(s(if lk.dark { "dark" } else { "light" }));
    g.set_accent_shown(s(match (c.local.appearance.accent, lk.accent) {
        (Some(a), _) => a.to_hex(),
        (None, Some(a)) => format!("desktop accent {}", a.to_hex()),
        (None, None) => "kelma purple".into(),
    }));
    g.set_a11y_shown(s(format!(
        "high contrast {}, reduced motion {}, text {}%",
        if lk.high_contrast { "on" } else { "off" },
        if lk.reduced_motion { "on" } else { "off" },
        (lk.font_scale * 100.0).round()
    )));
}

/// Hands the general preferences to what reads them outside Settings: the date and time helpers
/// and the tray's close-to-tray decision, plus the interface language (translation and direction).
fn apply_general(g: &appearance::GeneralPrefs) {
    appearance::formats::set_from(g);
    crate::language::apply(g);
    crate::tray_wiring::set_keep_running(g.keep_running_on_close);
}

fn toggles(rows: Vec<st::ToggleRow>) -> ModelRc<SToggle> {
    model(
        rows.into_iter()
            .map(|r| SToggle {
                key: r.key.into(),
                label: r.label.into(),
                hint: r.hint.into(),
                on: r.on,
                enabled: r.enabled,
            })
            .collect(),
    )
}

/// Pushes every local preference into `SettingsStore`.
fn push_local(w: &AppWindow, c: &Ctl) {
    let g = w.global::<SettingsStore>();
    let l = &c.local;
    let a = &l.appearance;
    g.set_theme(s(match a.theme {
        ThemeChoice::Light => "light",
        ThemeChoice::Dark => "dark",
        ThemeChoice::System => "system",
    }));
    g.set_accent(s(a.accent.map(Rgb::to_hex).unwrap_or_default()));
    g.set_density(s(match a.density {
        Density::Comfy => "comfy",
        Density::Compact => "compact",
    }));
    g.set_font_scale(a.font_scale);
    g.set_skin_tone(w.get_skin_tone());

    let gp = &l.general;
    let system = appearance::system_language()
        .map(|t| appearance::general::best_match(&t).native_name)
        .unwrap_or("English");
    let mut langs = vec![SOption {
        v: s(""),
        l: s(format!("System ({system})")),
    }];
    langs.extend(appearance::available_languages().iter().map(|l| SOption {
        v: s(l.tag),
        l: s(l.native_name),
    }));
    g.set_languages(model(langs));
    g.set_language(s(gp.language.clone().unwrap_or_default()));
    g.set_language_shown(s(appearance::effective_language(gp).english_name));
    g.set_date_format(s(st::date_slug(gp.date_format)));
    g.set_time_format(s(st::time_slug(gp.time_format)));
    g.set_time_zone(s(gp.time_zone.clone().unwrap_or_default()));
    g.set_system_time_zone(s(iana_time_zone::get_timezone().unwrap_or_default()));
    g.set_startup(toggles(st::startup_rows(gp)));
    g.set_check_updates(gp.check_updates);

    let x = &l.a11y;
    g.set_high_contrast(s(st::switch_slug(x.high_contrast)));
    g.set_reduced_motion(s(st::switch_slug(x.reduced_motion)));
    g.set_larger_text(x.larger_text);
    g.set_a11y_flags(toggles(st::a11y_rows(x)));

    g.set_mic(s(l.devices.microphone.clone().unwrap_or_default()));
    g.set_speaker(s(l.devices.speaker.clone().unwrap_or_default()));
    g.set_camera(s(l.devices.camera.clone().unwrap_or_default()));
    g.set_noise_suppression(l.devices.noise_suppression);
    g.set_echo_cancellation(l.devices.echo_cancellation);
    g.set_open_files(s(l.files.open_office_in.slug()));
    push_devices(w, c);
    push_perms(w, c);
}

fn push_devices(w: &AppWindow, c: &Ctl) {
    let g = w.global::<SettingsStore>();
    let all = g.get_all_devices();
    let rows = |v: Vec<st::DeviceRow>| {
        model(
            v.into_iter()
                .map(|(id, name)| SDevice {
                    id: id.into(),
                    name: name.into(),
                })
                .collect(),
        )
    };
    match &c.lists {
        Some(l) => {
            let d = &c.local.devices;
            g.set_mics(rows(st::audio_rows(
                &l.inputs,
                all,
                d.microphone.as_deref(),
            )));
            g.set_speakers(rows(st::audio_rows(&l.outputs, all, d.speaker.as_deref())));
            g.set_cameras(rows(st::camera_rows(&l.cameras)));
            g.set_devices_status(s(l.error.clone().unwrap_or_default()));
        }
        None => g.set_devices_status(s(if c.listing {
            "Looking for devices…"
        } else {
            ""
        })),
    }
}

fn push_perms(w: &AppWindow, c: &Ctl) {
    let asks = c
        .server
        .as_ref()
        .map(|d| d.apps.clone())
        .unwrap_or_default();
    let saved = c.local.grants.to_json();
    let rows = st::permission_rows(&asks, &c.local.grants, &saved);
    w.global::<SettingsStore>().set_app_perms(model(
        rows.into_iter()
            .map(|(app, name, perms)| SAppPerm {
                app: app.into(),
                name: name.into(),
                perms: toggles(
                    perms
                        .into_iter()
                        .map(|(p, on)| st::ToggleRow {
                            key: p.as_str().into(),
                            label: st::permission_label(p).into(),
                            hint: String::new(),
                            on,
                            enabled: true,
                        })
                        .collect(),
                ),
            })
            .collect(),
    ));
}

/// Pushes what was read from Teams (or that nothing was).
fn push_server(w: &AppWindow, c: &Ctl) {
    let g = w.global::<SettingsStore>();
    let d = c.server.as_ref();
    let n = d.and_then(|d| d.notifications.as_ref());
    g.set_notify(model(
        st::notify_rows(n)
            .into_iter()
            .map(|r| SNotify {
                key: r.key.into(),
                label: r.label.into(),
                group: r.group.into(),
                value: r.value.into(),
            })
            .collect(),
    ));
    g.set_paused(n.is_some_and(|n| n.paused_until().is_some()));
    let status = match d {
        None => "Reading your Teams settings…".to_string(),
        Some(d) if d.offline => "Offline: your Teams settings were not read.".to_string(),
        Some(d) => d.errors.join(" "),
    };
    g.set_notify_status(s(if n.is_none() || d.is_some_and(|d| d.offline) {
        status.clone()
    } else {
        String::new()
    }));
    let p = d.and_then(|d| d.privacy.as_ref());
    g.set_privacy(toggles(st::privacy_rows(p)));
    let people = |v: &[(String, String)]| {
        model(
            v.iter()
                .map(|(m, n)| SPerson {
                    mri: m.into(),
                    name: n.into(),
                })
                .collect(),
        )
    };
    g.set_blocked(people(d.map(|d| d.blocked.as_slice()).unwrap_or(&[])));
    g.set_priority(people(d.map(|d| d.priority.as_slice()).unwrap_or(&[])));
    g.set_privacy_status(s(
        if p.is_none() || d.is_some_and(|d| !d.errors.is_empty()) {
            status
        } else {
            String::new()
        },
    ));
    g.set_dry_runs(model(c.dry_runs.iter().map(s).collect()));
    push_profile(w, c);
    push_perms(w, c);
}

fn config(c: &Ctl) -> Option<session::Config> {
    c.env
        .paths
        .load_config()
        .inspect_err(|e| tracing::warn!(%e, "settings: config"))
        .ok()
}

fn push_profile(w: &AppWindow, c: &Ctl) {
    let g = w.global::<SettingsStore>();
    let cfg = config(c);
    let acct = cfg.as_ref().and_then(|c| c.active_account().cloned());
    let d = c.server.as_ref();
    let name = d
        .map(|d| d.me_name.clone())
        .filter(|n| !n.is_empty())
        .or_else(|| acct.as_ref().and_then(|a| a.display_name.clone()))
        .unwrap_or_default();
    let email = d
        .and_then(|d| d.me_email.clone())
        .or_else(|| acct.as_ref().and_then(|a| a.upn.clone()))
        .unwrap_or_default();
    g.set_me_name(s(name));
    g.set_me_email(s(email));
    g.set_me_status(s(d.and_then(|d| d.me_status.clone()).unwrap_or_default()));
    g.set_me_presence(s(d.and_then(|d| d.me_presence.clone()).unwrap_or_default()));
    g.set_me_work_location(s(d
        .and_then(|d| d.me_work_location.as_deref())
        .map(
            |w| match ::calendar::WorkLocation::from_value(&serde_json::json!(w)) {
                ::calendar::WorkLocation::Office => "office",
                ::calendar::WorkLocation::Remote => "remote",
                ::calendar::WorkLocation::Unset => "",
            },
        )
        .unwrap_or_default()));
    g.set_me_initials(w.get_me_initials());
}

fn push_accounts(w: &AppWindow, c: &Ctl) {
    let g = w.global::<SettingsStore>();
    let Some(cfg) = config(c) else {
        g.set_accounts_status(s("Could not read the accounts."));
        return;
    };
    g.set_accounts(model(
        st::account_rows(&cfg)
            .into_iter()
            .map(|a| SAccount {
                label: a.label.into(),
                name: a.name.into(),
                upn: a.upn.into(),
                active: a.active,
            })
            .collect(),
    ));
    let live = c.server.as_ref().and_then(|d| d.tenants.as_deref());
    g.set_orgs(model(
        st::org_rows(&cfg, live)
            .into_iter()
            .map(|o| SOrg {
                tenant: o.tenant.into(),
                name: o.name.into(),
                home: o.home,
                active: o.active,
            })
            .collect(),
    ));
}

impl UiHandle {
    /// A message from the engine for the Settings window and the profile menu.
    pub fn settings_notice(&self, text: String) {
        self.run(move |w| notice(&w, text));
    }

    /// "Refresh license and restart" finished in the engine: restart, or say why not.
    pub fn settings_license(&self, r: Result<(), String>) {
        self.run(move |w| license_done(&w, r));
    }

    /// The Teams side of Settings, read by the engine.
    pub fn settings_server(&self, data: ServerData) {
        self.run(move |w| {
            let Some(shared) = ctl() else { return };
            let mut c = shared.borrow_mut();
            c.server = Some(data);
            push_server(&w, &c);
            push_accounts(&w, &c);
        });
    }
}

fn notice(w: &AppWindow, text: impl Into<SharedString>) {
    w.global::<SettingsStore>().set_notice(text.into());
}

/// Records a Teams write that kelma only plans, and shows it.
fn dry_run(w: &AppWindow, c: &mut Ctl, req: teams_proto::user_settings::PlannedRequest) {
    let text = st::dry_run_text(&req);
    tracing::info!(request = %text, "settings: dry run, not sent to Teams");
    let first = text.lines().next().unwrap_or_default().to_string();
    c.dry_runs.insert(0, text);
    c.dry_runs.truncate(5);
    let g = w.global::<SettingsStore>();
    g.set_dry_runs(model(c.dry_runs.iter().map(s).collect()));
    notice(
        w,
        format!("Dry run: Teams was not changed. kelma would send {first}"),
    );
    // Teams still has the old value: show it again.
    push_server(w, c);
}

fn saved(w: &AppWindow, r: anyhow::Result<()>) {
    match r {
        Ok(()) => notice(w, "Saved."),
        Err(e) => notice(w, format!("Could not save: {e}")),
    }
}

fn request_load(c: &mut Ctl, force: bool) {
    let Some(tx) = &c.tx else { return };
    let stale = c
        .last_load
        .is_none_or(|t| t.elapsed() > Duration::from_secs(60));
    if force || stale {
        c.last_load = Some(Instant::now());
        let _ = tx.send(Cmd::Settings(SettingsCmd::Load));
    }
}

fn list_devices(w: &AppWindow, shared: &Shared) {
    let mut c = shared.borrow_mut();
    if c.lists.is_some() || c.listing {
        return;
    }
    let enumerate = c.env.hooks.enumerate.clone();
    if c.env.background {
        c.listing = true;
        push_devices(w, &c);
        let weak = w.as_weak();
        std::thread::spawn(move || {
            let lists = enumerate();
            let _ = weak.upgrade_in_event_loop(move |w| {
                if let Some(shared) = ctl() {
                    let mut c = shared.borrow_mut();
                    c.listing = false;
                    c.lists = Some(lists);
                    push_devices(&w, &c);
                }
            });
        });
    } else {
        c.lists = Some(enumerate());
        push_devices(w, &c);
    }
}

fn stop_mic(w: &AppWindow, c: &mut Ctl) {
    c.mic = None;
    let g = w.global::<SettingsStore>();
    g.set_mic_testing(false);
    g.set_mic_level(0.0);
}

fn start_mic(w: &AppWindow, c: &mut Ctl) {
    let input = c.local.devices.microphone.clone();
    match (c.env.hooks.mic_level)(input.as_deref()) {
        Ok(mut poll) => {
            let weak = w.as_weak();
            let started = Instant::now();
            let timer = slint::Timer::default();
            timer.start(
                slint::TimerMode::Repeated,
                Duration::from_millis(50),
                move || {
                    let Some(w) = weak.upgrade() else { return };
                    if started.elapsed() > Duration::from_secs(15) {
                        if let Some(shared) = ctl() {
                            stop_mic(&w, &mut shared.borrow_mut());
                        }
                        return;
                    }
                    if let Some(level) = poll() {
                        w.global::<SettingsStore>().set_mic_level(level);
                    }
                },
            );
            c.mic = Some(timer);
            w.global::<SettingsStore>().set_mic_testing(true);
            notice(w, "Speak: the bar shows what the microphone hears.");
        }
        Err(e) => notice(w, format!("Could not open the microphone: {e}")),
    }
}

/// Starts an update check if the switch is on and none is running. The automatic one happens at
/// most once per start (`force` is false); "Check now" forces another.
fn check_updates(w: &AppWindow, c: &mut Ctl, force: bool) {
    if !c.local.general.check_updates || c.checking || (!force && c.update_auto_done) {
        return;
    }
    if !force {
        c.update_auto_done = true;
    }
    c.checking = true;
    let g = w.global::<SettingsStore>();
    g.set_update_checking(true);
    g.set_update_available(false);
    g.set_update_status(s("Checking for updates…"));
    let fetch = c.env.hooks.check_update.clone();
    if c.env.background {
        let weak = w.as_weak();
        std::thread::spawn(move || {
            let r = fetch();
            let _ = weak.upgrade_in_event_loop(move |w| {
                if let Some(shared) = ctl() {
                    update_checked(&w, &mut shared.borrow_mut(), r);
                }
            });
        });
    } else {
        update_checked(w, c, fetch());
    }
}

/// Shows the answer of an update check on the About page, and as an info banner when newer.
fn update_checked(w: &AppWindow, c: &mut Ctl, r: Result<Release, String>) {
    c.checking = false;
    let g = w.global::<SettingsStore>();
    g.set_update_checking(false);
    if !c.local.general.check_updates {
        // Switched off while the answer was on its way.
        g.set_update_status(s(""));
        return;
    }
    match r {
        Ok(rel) if update_check::is_newer(env!("CARGO_PKG_VERSION"), &rel.version) => {
            g.set_update_status(s(format!("Version {} available", rel.version)));
            g.set_update_available(true);
            w.set_banner(s(format!("kelma {} is available", rel.version)));
            w.set_banner_action(s("Release notes"));
            w.set_banner_info(true);
            c.update_url = Some(rel.url);
        }
        Ok(_) => g.set_update_status(s("Up to date")),
        Err(e) => {
            tracing::debug!(error = %e, "update check failed");
            g.set_update_status(s("Could not check for updates"));
        }
    }
}

fn with_server_note(c: &Ctl) -> Option<(String, NotificationSettings)> {
    let d = c.server.as_ref()?;
    Some((d.chat_service.clone(), d.notifications.clone()?))
}

/// One command from the page. Runs on the UI thread.
fn command(shared: &Shared, w: &AppWindow, name: &str, arg: &str) {
    let g = w.global::<SettingsStore>();
    let parts: Vec<&str> = arg.split('\u{1f}').collect();
    match name {
        "open" | "page" => {
            notice(w, "");
            {
                let mut c = shared.borrow_mut();
                if name == "open" {
                    push_local(w, &c);
                    push_accounts(w, &c);
                    request_load(&mut c, false);
                }
                if arg != "devices" && c.mic.is_some() {
                    stop_mic(w, &mut c);
                }
            }
            if arg == "devices" {
                list_devices(w, shared);
            }
        }
        "close" => {
            let mut c = shared.borrow_mut();
            stop_mic(w, &mut c);
        }
        "profile" => {
            let mut c = shared.borrow_mut();
            push_profile(w, &c);
            request_load(&mut c, false);
        }
        "query" => {
            g.set_found(model(
                st::find(arg)
                    .into_iter()
                    .map(|f| SFind {
                        page: f.page.into(),
                        page_title: f.page_title.into(),
                        label: f.label.into(),
                    })
                    .collect(),
            ));
        }
        // --- Appearance ---
        "theme" | "accent" | "density" | "font-step" | "font-scale" => {
            let mut c = shared.borrow_mut();
            let a = &mut c.local.appearance;
            match name {
                "theme" => {
                    a.theme = match arg {
                        "light" => ThemeChoice::Light,
                        "dark" => ThemeChoice::Dark,
                        _ => ThemeChoice::System,
                    }
                }
                "accent" => a.accent = Rgb::from_hex(arg),
                "density" => {
                    a.density = if arg == "compact" {
                        Density::Compact
                    } else {
                        Density::Comfy
                    }
                }
                "font-step" => {
                    let v = st::step_font_scale(a.font_scale, arg.parse().unwrap_or(0));
                    a.set_font_scale(v);
                }
                _ => a.set_font_scale(arg.parse().unwrap_or(1.0)),
            }
            let r = c.local.save_appearance();
            apply_look(w, &c);
            push_local(w, &c);
            saved(w, r);
        }
        "skin-tone" => {
            let t = arg.parse::<i32>().unwrap_or(0).clamp(0, 5);
            w.set_skin_tone(t);
            g.set_skin_tone(t);
            // The engine keeps the skin tone with the other chat preferences.
            w.invoke_skin_tone_changed(t);
            notice(w, "Saved.");
        }
        // --- General ---
        "language" | "date-format" | "time-format" | "time-zone" | "startup" => {
            let mut c = shared.borrow_mut();
            let gp = &mut c.local.general;
            let mut autostart = None;
            match name {
                "language" => gp.language = appearance::general::normalize_tag(arg),
                "date-format" => gp.date_format = st::date_from_slug(arg).unwrap_or_default(),
                "time-format" => gp.time_format = st::time_from_slug(arg).unwrap_or_default(),
                "time-zone" => {
                    let z = arg.trim();
                    gp.time_zone = (!z.is_empty()).then(|| z.to_string());
                }
                _ => {
                    let on = bit(parts.get(1).copied().unwrap_or(""));
                    let key = parts[0];
                    st::set_startup(gp, key, on);
                    if key == "auto-start" {
                        autostart = Some(on);
                    }
                }
            }
            let formats_changed = matches!(name, "date-format" | "time-format");
            let mut r = c.local.save_general();
            if c.env.apply_globals {
                apply_general(&c.local.general);
            }
            if formats_changed && let Some(tx) = &c.tx {
                // Rows already drawn show the old format until the engine draws them again.
                let _ = tx.send(Cmd::Settings(SettingsCmd::Redraw));
            }
            if let Some(on) = autostart
                && r.is_ok()
            {
                r = (c.env.hooks.autostart.set)(on).map_err(|e| anyhow::anyhow!(e));
            }
            push_local(w, &c);
            saved(w, r);
        }
        // --- Accessibility ---
        "high-contrast" | "reduced-motion" | "larger-text" | "a11y" => {
            let mut c = shared.borrow_mut();
            let x = &mut c.local.a11y;
            match name {
                "high-contrast" => x.high_contrast = st::switch_from_slug(arg).unwrap_or_default(),
                "reduced-motion" => {
                    x.reduced_motion = st::switch_from_slug(arg).unwrap_or_default()
                }
                "larger-text" => x.larger_text = bit(arg),
                _ => {
                    st::set_a11y_flag(x, parts[0], bit(parts.get(1).copied().unwrap_or("")));
                }
            }
            let r = c.local.save_a11y();
            apply_look(w, &c);
            push_local(w, &c);
            saved(w, r);
        }
        // --- Notifications (Teams: dry run) ---
        "notify" | "pause" => {
            let mut c = shared.borrow_mut();
            let Some((base, current)) = with_server_note(&c) else {
                notice(
                    w,
                    "Your Teams settings were not read, so nothing can be changed.",
                );
                push_server(w, &c);
                return;
            };
            let req = if name == "notify" {
                let kind = Kind::from_slug(parts[0]);
                let delivery = parts.get(1).and_then(|d| Delivery::from_slug(d));
                let (Some(kind), Some(delivery)) = (kind, delivery) else {
                    return;
                };
                st::dry_run_notify(&base, &current, kind, delivery)
            } else {
                let until = bit(arg).then(|| {
                    (chrono::Utc::now() + chrono::Duration::days(1))
                        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                        .to_string()
                });
                st::dry_run_pause(&base, &current, until)
            };
            dry_run(w, &mut c, req);
        }
        "test-notification" => {
            (shared.borrow().env.hooks.test_notification)();
            notice(w, "Sent a test notification to your desktop.");
        }
        // --- Privacy (Teams: dry run) ---
        "privacy" | "unblock" | "priority-remove" => {
            let mut c = shared.borrow_mut();
            let Some(d) = c.server.as_ref().filter(|d| d.privacy.is_some()) else {
                notice(
                    w,
                    "Your Teams settings were not read, so nothing can be changed.",
                );
                push_server(w, &c);
                return;
            };
            let req = match name {
                "privacy" => {
                    let Some(flag) = PrivacyFlag::from_slug(parts[0]) else {
                        return;
                    };
                    let on = bit(parts.get(1).copied().unwrap_or(""));
                    st::dry_run_privacy(&d.chat_service, flag, on)
                }
                "unblock" => st::dry_run_remove(&d.middle_tier, ContactList::Blocked, arg),
                _ => st::dry_run_remove(&d.middle_tier, ContactList::Priority, arg),
            };
            dry_run(w, &mut c, req);
        }
        // --- Devices (local) ---
        "speaker" | "mic" | "camera" => {
            let mut c = shared.borrow_mut();
            let v = (!arg.is_empty()).then(|| arg.to_string());
            match name {
                "speaker" => c.local.devices.speaker = v,
                "mic" => c.local.devices.microphone = v,
                _ => c.local.devices.camera = v,
            }
            let r = c.local.save_devices();
            if name == "mic" && c.mic.is_some() {
                stop_mic(w, &mut c);
            }
            push_local(w, &c);
            saved(w, r);
        }
        "noise-suppression" | "echo-cancellation" => {
            let mut c = shared.borrow_mut();
            if name == "noise-suppression" {
                c.local.devices.noise_suppression = bit(arg);
            } else {
                c.local.devices.echo_cancellation = bit(arg);
            }
            let r = c.local.save_devices();
            push_local(w, &c);
            saved(w, r);
        }
        "all-devices" => {
            g.set_all_devices(bit(arg));
            push_devices(w, &shared.borrow());
        }
        "test-sound" => {
            let c = shared.borrow();
            let out = c.local.devices.speaker.clone();
            let play = c.env.hooks.test_sound.clone();
            if c.env.background {
                notice(w, "Playing a test sound…");
                let weak = w.as_weak();
                std::thread::spawn(move || {
                    let r = play(out);
                    let _ = weak.upgrade_in_event_loop(move |w| {
                        if let Err(e) = r {
                            notice(&w, format!("Could not play the test sound: {e}"));
                        }
                    });
                });
            } else {
                match play(out) {
                    Ok(()) => notice(w, "Played a test sound."),
                    Err(e) => notice(w, format!("Could not play the test sound: {e}")),
                }
            }
        }
        "mic-test" => {
            let mut c = shared.borrow_mut();
            if bit(arg) {
                start_mic(w, &mut c);
            } else {
                stop_mic(w, &mut c);
            }
        }
        // --- Files and app permissions (local) ---
        "open-files" => {
            let mut c = shared.borrow_mut();
            c.local.files.open_office_in = OpenFilesIn::from_slug(arg).unwrap_or_default();
            let r = c.local.save_files();
            push_local(w, &c);
            saved(w, r);
        }
        "perm" => {
            let mut c = shared.borrow_mut();
            let (Some(app), Some(p)) = (
                parts.first(),
                parts.get(1).and_then(|p| DevicePermission::parse(p)),
            ) else {
                return;
            };
            let on = bit(parts.get(2).copied().unwrap_or(""));
            let r = c.local.set_grant(app, p, on);
            push_perms(w, &c);
            saved(w, r);
        }
        // --- Accounts ---
        "switch-account" | "switch-org" => {
            let c = shared.borrow();
            let Some(cfg) = config(&c) else { return };
            let r = if name == "switch-account" {
                if cfg.account == arg {
                    return;
                }
                // Read, change and save under the config lock: the running engine saves its
                // own preferences into the same file and must not overwrite the switch.
                c.env.paths.update_config(|cfg| cfg.switch_account(arg))
            } else {
                let current =
                    st::org_rows(&cfg, c.server.as_ref().and_then(|d| d.tenants.as_deref()));
                if current.iter().any(|o| o.active && o.tenant == arg) {
                    return;
                }
                c.env.paths.update_config(|cfg| cfg.switch_tenant(arg))
            };
            match r {
                Ok(()) => {
                    g.set_restart_needed(true);
                    g.set_accounts_status(s("Saved. Restart kelma to switch."));
                }
                Err(e) => g.set_accounts_status(s(format!("Could not switch: {e}"))),
            }
            push_accounts(w, &c);
        }
        // --- Updates (opt-in; local) ---
        "check-updates" => {
            let mut c = shared.borrow_mut();
            c.local.general.check_updates = bit(arg);
            let r = c.local.save_general();
            push_local(w, &c);
            if c.local.general.check_updates {
                check_updates(w, &mut c, false);
            } else {
                g.set_update_status(s(""));
                g.set_update_available(false);
            }
            saved(w, r);
        }
        "check-now" => check_updates(w, &mut shared.borrow_mut(), true),
        "release-notes" => {
            let c = shared.borrow();
            if let Some(url) = &c.update_url {
                (c.env.hooks.open_url)(url);
            }
        }
        "add-account" => {
            let c = shared.borrow();
            let label = arg.trim();
            // The new account becomes the active one; kelma has no tokens for it, so the next
            // start runs the device-code sign-in (the code is shown in the login screen).
            let r = c.env.paths.update_config(|cfg| {
                cfg.add_account(label)?;
                cfg.switch_account(label)
            });
            match r {
                Ok(()) => {
                    session::allow_account_tokens(label);
                    g.set_restart_needed(true);
                    g.set_accounts_status(s(format!(
                        "Added \"{label}\". Restart kelma to sign in to it."
                    )));
                }
                Err(e) => g.set_accounts_status(s(format!("Could not add the account: {e}"))),
            }
            push_accounts(w, &c);
        }
        "remove-account" => {
            let c = shared.borrow();
            let Some(acct) = config(&c).and_then(|cfg| cfg.account_by_label(arg).cloned()) else {
                g.set_accounts_status(s("That account is not in the list."));
                push_accounts(w, &c);
                return;
            };
            // Tokens first: the running engine saves its token every few minutes, and the
            // forget marks the account so a save that is already running skips it.
            let was_active = config(&c).is_some_and(|cfg| cfg.account == arg);
            let r = session::forget_account_tokens(&*c.env.tokens, &acct)
                .and_then(|()| c.env.paths.update_config(|cfg| cfg.remove_account(arg)));
            match r {
                Ok(()) => {
                    remove_account_cache(&c.env.paths, &acct);
                    // The engine runs on the account that was active at start: when that one is
                    // gone, kelma has to start again on another.
                    if was_active {
                        let now = config(&c).map(|cfg| cfg.account).unwrap_or_default();
                        g.set_restart_needed(true);
                        g.set_accounts_status(s(format!(
                            "Removed \"{arg}\" and its saved sign-in. Restart kelma: it uses \"{now}\" next."
                        )));
                    } else {
                        g.set_accounts_status(s(format!(
                            "Removed \"{arg}\" and its saved sign-in."
                        )));
                    }
                }
                Err(e) => g.set_accounts_status(s(format!("Could not remove the account: {e}"))),
            }
            push_accounts(w, &c);
        }
        // --- Profile menu: my status and work location (sent to Teams) ---
        "status" | "work-location" => {
            let c = shared.borrow();
            let Some(tx) = &c.tx else {
                notice(w, "Not signed in, so your status can't change.");
                return;
            };
            let cmd = if name == "status" {
                SettingsCmd::SetStatus(arg.to_string())
            } else {
                SettingsCmd::SetWorkLocation(arg.to_string())
            };
            let _ = tx.send(Cmd::Settings(cmd));
            g.set_status_open(false);
        }
        "refresh-license" => {
            let c = shared.borrow();
            let Some(tx) = &c.tx else {
                notice(w, "Not signed in, so there is no license to refresh.");
                return;
            };
            let _ = tx.send(Cmd::Settings(SettingsCmd::RefreshLicense));
            notice(w, "Refreshing your license…");
        }
        "restart" => (shared.borrow().env.hooks.restart)(),
        "sign-out" => {
            let c = shared.borrow();
            let Some(acct) = config(&c).and_then(|cfg| cfg.active_account().cloned()) else {
                notice(w, "Could not read the account to sign out of.");
                return;
            };
            g.set_profile_open(false);
            let tokens = c.env.tokens.clone();
            if c.env.background {
                let weak = w.as_weak();
                std::thread::spawn(move || {
                    let r = session::forget_account_tokens(&*tokens, &acct);
                    let _ = weak.upgrade_in_event_loop(move |w| signed_out(&w, r));
                });
            } else {
                let r = session::forget_account_tokens(&*tokens, &acct);
                drop(c);
                signed_out(w, r);
            }
        }
        "view-account" => {
            (shared.borrow().env.hooks.open_url)("https://myaccount.microsoft.com/");
            g.set_profile_open(false);
        }
        "report-problem" => {
            let c = shared.borrow();
            let report = c.env.hooks.report_problem.clone();
            if c.env.background {
                notice(w, "Writing the problem report…");
                let weak = w.as_weak();
                std::thread::spawn(move || {
                    let r = report();
                    let _ = weak.upgrade_in_event_loop(move |w| report_done(&w, r));
                });
            } else {
                report_done(w, report());
            }
        }
        other => tracing::debug!(command = other, "settings: unknown command"),
    }
}

/// Deletes the message cache of a removed account, unless the account that is active afterwards
/// uses the same file (removing the only account leaves a fresh `default`).
fn remove_account_cache(paths: &session::Paths, removed: &session::Account) {
    let gone = session::Config {
        account: removed.label.clone(),
        tenant: removed.tenant.clone(),
        accounts: vec![removed.clone()],
        ..Default::default()
    };
    let file = paths.db_path_for(&gone);
    if paths
        .load_config()
        .is_ok_and(|now| paths.db_path_for(&now) == file)
    {
        return;
    }
    for suffix in ["", "-wal", "-shm"] {
        let mut name = file.clone().into_os_string();
        name.push(suffix);
        let _ = std::fs::remove_file(name);
    }
}

fn license_done(w: &AppWindow, r: Result<(), String>) {
    match r {
        Ok(()) => {
            notice(w, "License refreshed. Restarting kelma…");
            if let Some(shared) = ctl() {
                (shared.borrow().env.hooks.restart)();
            }
        }
        Err(e) => notice(w, format!("Could not refresh the license: {e}")),
    }
}

fn report_done(w: &AppWindow, r: Result<String, String>) {
    match r {
        Ok(p) => notice(w, format!("Saved the report to {p}. Nothing was uploaded.")),
        Err(e) => notice(w, format!("Could not write the report: {e}")),
    }
}

fn signed_out(w: &AppWindow, r: session::Result<()>) {
    match r {
        Ok(()) => {
            tracing::info!("signed out");
            if let Some(shared) = ctl() {
                (shared.borrow().env.hooks.signed_out)();
            }
        }
        Err(e) => {
            let g = w.global::<SettingsStore>();
            g.set_profile_open(true);
            notice(w, format!("Could not sign out: {e}"));
        }
    }
}
