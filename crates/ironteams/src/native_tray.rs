//! The tray icon on Windows (notification area) and macOS (menu bar): Slint's `SystemTrayIcon`
//! with the same icon, unread badge, tooltip and menu as the Linux StatusNotifierItem (`tray`).
//! It lives on the UI thread; `tray_wiring` calls in here instead of the `tray` crate's handle.

use std::cell::RefCell;
use std::rc::Rc;
use tray::TrayEvent;

slint::slint! {
    #[edition = "1.19"]
    export component IronTeamsTray inherits SystemTrayIcon {
        in property <image> picture;
        in property <string> tip;
        in-out property <bool> mute-checked;
        in-out property <bool> focus-checked;
        callback open();
        callback toggle-mute();
        callback toggle-focus();
        callback quit();
        icon: root.picture;
        tooltip: root.tip;
        clicked => { root.open(); }
        Menu {
            MenuItem {
                title: "Open IronTeams";
                activated => { root.open(); }
            }
            MenuItem {
                title: "Mute notifications";
                checkable: true;
                checked <=> root.mute-checked;
                activated => { root.toggle-mute(); }
            }
            MenuItem {
                title: "Focus mode";
                checkable: true;
                checked <=> root.focus-checked;
                activated => { root.toggle-focus(); }
            }
            MenuSeparator { }
            MenuItem {
                title: "Quit";
                activated => { root.quit(); }
            }
        }
    }
}

/// Icon edge in pixels; the system scales it to the tray's size.
const ICON_SIZE: u32 = 32;

struct State {
    ui: IronTeamsTray,
    unread: u32,
    muted: bool,
    focus: bool,
}

thread_local! {
    static TRAY: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn picture(unread: u32) -> slint::Image {
    let p = tray::icon::badged_icon(unread, ICON_SIZE);
    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        &p.to_rgba(),
        p.width,
        p.height,
    );
    slint::Image::from_rgba8(buffer)
}

fn refresh(s: &State) {
    s.ui.set_picture(picture(s.unread));
    s.ui.set_tip(tray::tooltip_text(s.unread, s.muted).into());
    s.ui.set_mute_checked(s.muted);
    s.ui.set_focus_checked(s.focus);
}

/// Flips a checkmark kept here (the source of truth, whatever the menu did with its own state)
/// and reports the new value.
fn flip(which: fn(&mut State) -> &mut bool) -> Option<bool> {
    TRAY.with(|t| {
        let mut t = t.borrow_mut();
        let s = t.as_mut()?;
        let v = which(s);
        *v = !*v;
        let now = *v;
        refresh(s);
        Some(now)
    })
}

/// Shows the tray icon. `on_event` runs on the UI thread for every click and menu pick. An error
/// means there is no icon (the caller then quits on close instead of hiding to the tray).
pub fn start(on_event: impl Fn(TrayEvent) + 'static) -> Result<(), String> {
    let ui = IronTeamsTray::new().map_err(|e| e.to_string())?;
    let on_event = Rc::new(on_event);
    let f = on_event.clone();
    ui.on_open(move || f(TrayEvent::Open));
    let f = on_event.clone();
    ui.on_toggle_mute(move || {
        if let Some(m) = flip(|s| &mut s.muted) {
            f(TrayEvent::ToggleMute(m));
        }
    });
    let f = on_event.clone();
    ui.on_toggle_focus(move || {
        if let Some(on) = flip(|s| &mut s.focus) {
            f(TrayEvent::ToggleFocus(on));
        }
    });
    ui.on_quit(move || on_event(TrayEvent::Quit));
    let state = State {
        ui,
        unread: 0,
        muted: false,
        focus: false,
    };
    refresh(&state);
    TRAY.with(|t| *t.borrow_mut() = Some(state));
    Ok(())
}

/// Whether [`start`] put an icon up.
pub fn running() -> bool {
    TRAY.with(|t| t.borrow().is_some())
}

/// The unread count on the icon's badge and in its tooltip (0 hides the badge).
pub fn set_unread(count: u32) {
    TRAY.with(|t| {
        if let Some(s) = t.borrow_mut().as_mut() {
            s.unread = count;
            refresh(s);
        }
    });
}

/// The Focus mode checkmark, when the app changes it elsewhere.
pub fn set_focus(on: bool) {
    TRAY.with(|t| {
        if let Some(s) = t.borrow_mut().as_mut() {
            s.focus = on;
            refresh(s);
        }
    });
}

/// Removes the icon.
pub fn shutdown() {
    TRAY.with(|t| t.borrow_mut().take());
}
