//! kelma tray: a StatusNotifierItem tray icon with an unread badge, the Unity launcher badge
//! and XDG autostart.
//!
//! * [`Tray::start`] runs the tray on its own thread (a private single-threaded runtime), so it
//!   sits next to the Slint event loop without touching it. The handle sets the unread count and
//!   the mute flag, and hands out the events (Open, ToggleMute, Quit) to poll or block on.
//! * The same thread emits `com.canonical.Unity.LauncherEntry.Update`, which the KDE task
//!   manager shows as a badge on the launcher ([`launcher`]).
//! * [`autostart`] writes and removes `$XDG_CONFIG_HOME/autostart/kelma.desktop`.

pub mod autostart;
pub mod icon;
pub mod launcher;
pub mod menu;
pub mod screen;

use crossbeam_channel::{Receiver, Sender};
use ksni::TrayMethods;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// Something the user did in the tray. The app decides what it means (show the window, stop
/// notifying, shut down).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// Left click on the icon, or the Open menu entry: show and focus the main window.
    Open,
    /// The Mute notifications checkmark was toggled; carries the new state.
    ToggleMute(bool),
    /// The Quit menu entry.
    Quit,
}

#[derive(Clone, Debug)]
pub struct TrayOptions {
    /// StatusNotifierItem id, unique to the app and stable between sessions.
    pub id: String,
    pub title: String,
    /// Themed icon name tried first; the built-in pixmap is the fallback when it does not
    /// resolve (for example before the package installs the icon).
    pub icon_name: String,
    /// Launcher URI the badge is addressed to (the desktop file id).
    pub launcher_uri: String,
    /// Initial state of the Mute notifications checkmark.
    pub muted: bool,
    /// Also send every `set_unread` count to the launcher badge.
    pub sync_launcher_badge: bool,
}

impl Default for TrayOptions {
    fn default() -> Self {
        Self {
            id: "kelma".into(),
            title: "kelma".into(),
            icon_name: "kelma".into(),
            launcher_uri: launcher::APP_URI.into(),
            muted: false,
            sync_launcher_badge: true,
        }
    }
}

enum Cmd {
    Unread(u32),
    Muted(bool),
    Launcher(u32),
    Shutdown,
}

/// Title shown by the host: the name, plus the count when there is one.
fn title_text(title: &str, unread: u32) -> String {
    match icon::badge_label(unread) {
        Some(n) => format!("{title} ({n} unread)"),
        None => title.to_string(),
    }
}

/// The tooltip body: the count, or a quiet "No unread messages".
fn tooltip_text(unread: u32, muted: bool) -> String {
    let mut s = match unread {
        0 => "No unread messages".to_string(),
        1 => "1 unread message".to_string(),
        n => format!(
            "{} unread messages",
            icon::badge_label(n).unwrap_or_default()
        ),
    };
    if muted {
        s.push_str(" (notifications muted)");
    }
    s
}

fn to_ksni(p: icon::Pixmap) -> ksni::Icon {
    ksni::Icon {
        width: p.width as i32,
        height: p.height as i32,
        data: p.argb,
    }
}

struct SniTray {
    opts: TrayOptions,
    unread: u32,
    muted: bool,
    events: Sender<TrayEvent>,
}

impl ksni::Tray for SniTray {
    fn id(&self) -> String {
        self.opts.id.clone()
    }

    fn title(&self) -> String {
        title_text(&self.opts.title, self.unread)
    }

    fn icon_name(&self) -> String {
        self.opts.icon_name.clone()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        icon::SIZES
            .into_iter()
            .map(|s| to_ksni(icon::app_icon(s)))
            .collect()
    }

    fn overlay_icon_pixmap(&self) -> Vec<ksni::Icon> {
        icon::SIZES
            .into_iter()
            .filter_map(|s| icon::badge_overlay(self.unread, s))
            .map(to_ksni)
            .collect()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            icon_name: self.opts.icon_name.clone(),
            icon_pixmap: vec![to_ksni(icon::app_icon(32))],
            title: self.opts.title.clone(),
            description: tooltip_text(self.unread, self.muted),
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.events.send(TrayEvent::Open);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::{CheckmarkItem, StandardItem};
        use menu::MenuEntry;
        menu::menu_model(self.muted)
            .into_iter()
            .map(|entry| match entry {
                MenuEntry::Separator => ksni::MenuItem::Separator,
                MenuEntry::Item {
                    label,
                    icon,
                    checked: Some(checked),
                    action,
                } => CheckmarkItem {
                    label: label.into(),
                    icon_name: icon.into(),
                    checked,
                    activate: Box::new(move |t: &mut Self| t.run(action)),
                    ..Default::default()
                }
                .into(),
                MenuEntry::Item {
                    label,
                    icon,
                    checked: None,
                    action,
                } => StandardItem {
                    label: label.into(),
                    icon_name: icon.into(),
                    activate: Box::new(move |t: &mut Self| t.run(action)),
                    ..Default::default()
                }
                .into(),
            })
            .collect()
    }
}

impl SniTray {
    fn run(&mut self, action: menu::MenuAction) {
        let event = match action {
            menu::MenuAction::Open => TrayEvent::Open,
            menu::MenuAction::ToggleMute => {
                self.muted = !self.muted;
                TrayEvent::ToggleMute(self.muted)
            }
            menu::MenuAction::Quit => TrayEvent::Quit,
        };
        let _ = self.events.send(event);
    }
}

/// What the worker thread reports once it has tried to come up.
struct Startup {
    /// Why the tray icon could not be registered, if it could not.
    tray_error: Option<String>,
    launcher_ready: bool,
}

struct Inner {
    cmds: UnboundedSender<Cmd>,
    events: Receiver<TrayEvent>,
    thread: Mutex<Option<JoinHandle<()>>>,
    tray_error: Option<String>,
    launcher_ready: AtomicBool,
    sync_launcher: bool,
}

impl Inner {
    fn stop(&self) {
        let _ = self.cmds.send(Cmd::Shutdown);
        if let Some(t) = self.thread.lock().ok().and_then(|mut t| t.take()) {
            let _ = t.join();
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A running tray. Cheap to clone; every clone drives the same icon, and the tray goes away
/// when [`shutdown`](Self::shutdown) is called or the last clone is dropped.
#[derive(Clone)]
pub struct TrayHandle {
    inner: Arc<Inner>,
}

/// Entry point: [`Tray::start`].
pub struct Tray;

impl Tray {
    /// Start the tray on its own thread and return once it has tried to register. It never
    /// fails: a desktop with no StatusNotifier host still gets the launcher badge, and the
    /// reason is in [`TrayHandle::tray_error`]. A host that appears later (for example a panel
    /// that starts after the app at login) picks the icon up.
    pub fn start(opts: TrayOptions) -> TrayHandle {
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let (ready_tx, ready_rx) = crossbeam_channel::unbounded();
        let sync_launcher = opts.sync_launcher_badge;
        let thread = std::thread::Builder::new()
            .name("kelma-tray".into())
            .spawn(move || worker(opts, cmd_rx, event_tx, ready_tx))
            .ok();
        let startup = ready_rx.recv().unwrap_or(Startup {
            tray_error: Some("the tray thread did not start".into()),
            launcher_ready: false,
        });
        TrayHandle {
            inner: Arc::new(Inner {
                cmds: cmd_tx,
                events: event_rx,
                thread: Mutex::new(thread),
                tray_error: startup.tray_error,
                launcher_ready: AtomicBool::new(startup.launcher_ready),
                sync_launcher,
            }),
        }
    }
}

impl TrayHandle {
    /// Set the unread count: the badge on the icon, the title and the tooltip (0 hides the
    /// badge). Also updates the launcher badge unless `sync_launcher_badge` is off.
    pub fn set_unread(&self, count: u32) {
        let _ = self.inner.cmds.send(Cmd::Unread(count));
        if self.inner.sync_launcher {
            self.set_launcher_count(count);
        }
    }

    /// Set the state of the Mute notifications checkmark (when the app changes it elsewhere).
    pub fn set_muted(&self, muted: bool) {
        let _ = self.inner.cmds.send(Cmd::Muted(muted));
    }

    /// Set only the launcher (task manager) badge; 0 hides it.
    pub fn set_launcher_count(&self, count: u32) {
        let _ = self.inner.cmds.send(Cmd::Launcher(count));
    }

    /// Clicks and menu picks. Poll with `try_recv` from a UI timer or block on `recv` in a
    /// thread. Receivers share one queue: each event goes to exactly one of them.
    pub fn events(&self) -> Receiver<TrayEvent> {
        self.inner.events.clone()
    }

    /// Why the tray icon is not registered (no StatusNotifier host, no session bus), or `None`
    /// when it is.
    pub fn tray_error(&self) -> Option<&str> {
        self.inner.tray_error.as_deref()
    }

    /// Whether the session bus was reachable for the launcher badge.
    pub fn launcher_ready(&self) -> bool {
        self.inner.launcher_ready.load(Ordering::Relaxed)
    }

    /// Remove the icon, clear the launcher badge and stop the thread. Waits for it.
    pub fn shutdown(&self) {
        self.inner.stop();
    }
}

fn worker(
    opts: TrayOptions,
    mut cmds: UnboundedReceiver<Cmd>,
    events: Sender<TrayEvent>,
    ready: Sender<Startup>,
) {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        let _ = ready.send(Startup {
            tray_error: Some("could not start the tray runtime".into()),
            launcher_ready: false,
        });
        return;
    };
    rt.block_on(async move {
        let launcher_uri = opts.launcher_uri.clone();
        let conn = zbus::Connection::session().await;
        let tray = SniTray {
            unread: 0,
            muted: opts.muted,
            events,
            opts,
        };
        // Inside a Flatpak the well-known name is not allowed; the portal proxies the item.
        let sandboxed = std::path::Path::new("/.flatpak-info").exists();
        let handle = tray
            .disable_dbus_name(sandboxed)
            .assume_sni_available(true)
            .spawn()
            .await;
        let _ = ready.send(Startup {
            tray_error: handle.as_ref().err().map(|e| e.to_string()),
            launcher_ready: conn.is_ok(),
        });
        let emit = |count: u32| {
            let conn = conn.as_ref().ok();
            let update = launcher::launcher_update(&launcher_uri, count);
            async move {
                if let Some(c) = conn {
                    let _ = update.emit(c).await;
                }
            }
        };
        while let Some(cmd) = cmds.recv().await {
            match cmd {
                Cmd::Unread(n) => {
                    if let Ok(h) = &handle {
                        h.update(|t| t.unread = n).await;
                    }
                }
                Cmd::Muted(m) => {
                    if let Ok(h) = &handle {
                        h.update(|t| t.muted = m).await;
                    }
                }
                Cmd::Launcher(n) => emit(n).await,
                Cmd::Shutdown => break,
            }
        }
        emit(0).await;
        if let Ok(h) = handle {
            h.shutdown().await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_and_tooltip_show_the_count() {
        assert_eq!(title_text("kelma", 0), "kelma");
        assert_eq!(title_text("kelma", 3), "kelma (3 unread)");
        assert_eq!(title_text("kelma", 250), "kelma (99+ unread)");
        assert_eq!(tooltip_text(0, false), "No unread messages");
        assert_eq!(tooltip_text(1, false), "1 unread message");
        assert_eq!(tooltip_text(12, false), "12 unread messages");
        assert_eq!(tooltip_text(100, false), "99+ unread messages");
        assert_eq!(
            tooltip_text(2, true),
            "2 unread messages (notifications muted)"
        );
    }

    fn sni(muted: bool) -> (SniTray, Receiver<TrayEvent>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let t = SniTray {
            opts: TrayOptions::default(),
            unread: 0,
            muted,
            events: tx,
        };
        (t, rx)
    }

    #[test]
    fn menu_actions_become_events_and_mute_flips_the_checkmark() {
        use ksni::Tray as _;
        let (mut t, rx) = sni(false);
        t.run(menu::MenuAction::Open);
        t.run(menu::MenuAction::ToggleMute);
        t.run(menu::MenuAction::ToggleMute);
        t.run(menu::MenuAction::Quit);
        let got: Vec<_> = rx.try_iter().collect();
        assert_eq!(
            got,
            [
                TrayEvent::Open,
                TrayEvent::ToggleMute(true),
                TrayEvent::ToggleMute(false),
                TrayEvent::Quit
            ]
        );
        assert_eq!(t.menu().len(), 4);
    }

    #[test]
    fn left_click_opens_the_window() {
        use ksni::Tray as _;
        let (mut t, rx) = sni(false);
        t.activate(10, 20);
        assert_eq!(rx.try_recv(), Ok(TrayEvent::Open));
    }

    #[test]
    fn overlay_only_exists_when_there_is_something_unread() {
        use ksni::Tray as _;
        let (mut t, _rx) = sni(false);
        assert!(t.overlay_icon_pixmap().is_empty());
        t.unread = 5;
        assert_eq!(t.overlay_icon_pixmap().len(), icon::SIZES.len());
        assert_eq!(t.icon_pixmap().len(), icon::SIZES.len());
    }
}
