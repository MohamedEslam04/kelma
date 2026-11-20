//! Letting someone else drive the screen we share, through the xdg-desktop-portal RemoteDesktop
//! portal (KDE Plasma and GNOME implement it; Wayland has no other way in).
//!
//! Sharing never asks for control (it is the plain ScreenCast picker, `screencast::pick`: one
//! screen or window). Control is a later step: when the sharer lets someone drive the screen,
//! [`request_control`] opens a RemoteDesktop session (keyboard + pointer) and asks the desktop
//! for input consent then. Absolute pointer positions only exist relative to a stream OF THE SAME
//! RemoteDesktop session, so that session also selects a screen source; the share's own capture
//! keeps running in its ScreenCast session and the control session's stream is never read, only
//! used as the coordinate space. [`control_target`] maps the shared picture onto that stream (it
//! is the same size when the user picks the same screen or window again, or the whole desktop
//! when the desktop offers no picker, as KDE did).

use std::sync::Arc;

use ashpd::desktop::Session;
use ashpd::desktop::remote_desktop::{
    Axis, DeviceType, KeyState, NotifyPointerAxisOptions, RemoteDesktop, SelectDevicesOptions,
};
use ashpd::desktop::screencast::{Screencast, SourceType};

use super::control_overlay::ShareArea;
use super::screencast::{ShareKind, portal_err, select_sources, stream_size};
use crate::remote_control::{InputEvent, MouseButton, stream_point};
use crate::{Error, Result};

/// Injects the controller's input into the shared stream. Cheap to clone.
#[derive(Clone)]
pub struct InputInjector {
    proxy: Arc<RemoteDesktop>,
    session: Arc<Session<RemoteDesktop>>,
    /// The PipeWire node of the control session's stream (pointer coordinates are relative to it).
    stream: u32,
    /// Where the shared picture sits in that stream, in its coordinates.
    target: Target,
}

/// The shared picture's rectangle in the control session's stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    pub offset: (f64, f64),
    pub size: (u32, u32),
}

/// Where the shared picture (`shared`, from the share's own session) sits in the stream the
/// control session got (`control`), or `None` when that can't be told, so no click lands
/// somewhere the viewer can't see:
/// - same size: the same screen or window, the stream is the picture;
/// - a monitor share inside a larger stream (the whole desktop): offset by the difference of the
///   two positions (the stream's own position is taken as the origin when it names none).
///
/// UNVERIFIED on a live desktop: sizes (pixels) and positions (logical) are assumed to be in the
/// same units.
pub fn control_target(shared: &ShareArea, control: &ShareArea) -> Option<Target> {
    if shared.size == control.size {
        return Some(Target {
            offset: (0.0, 0.0),
            size: shared.size,
        });
    }
    let (sx, sy) = shared.position.filter(|_| shared.monitor)?;
    let (cx, cy) = control.position.unwrap_or((0, 0));
    let fits = shared.size.0 <= control.size.0 && shared.size.1 <= control.size.1;
    fits.then(|| Target {
        offset: (f64::from(sx - cx), f64::from(sy - cy)),
        size: shared.size,
    })
}

fn state(pressed: bool) -> KeyState {
    if pressed {
        KeyState::Pressed
    } else {
        KeyState::Released
    }
}

impl InputInjector {
    /// Ends the control session (the desktop's "remote control" indicator goes).
    pub async fn close(&self) {
        let _ = self.session.close().await;
    }

    /// Moves the pointer to a normalized point (0..=1) of the shared stream.
    pub async fn pointer_motion_absolute(&self, x: f64, y: f64) -> Result<()> {
        let (x, y) = stream_point(x, y, self.target.size);
        let (x, y) = (x + self.target.offset.0, y + self.target.offset.1);
        self.proxy
            .notify_pointer_motion_absolute(&self.session, self.stream, x, y, Default::default())
            .await
            .map_err(portal_err)
    }

    pub async fn pointer_button(&self, button: MouseButton, pressed: bool) -> Result<()> {
        self.proxy
            .notify_pointer_button(
                &self.session,
                button.evdev(),
                state(pressed),
                Default::default(),
            )
            .await
            .map_err(portal_err)
    }

    /// Scrolls by `dx`, `dy` pixels (positive: right, down).
    pub async fn axis(&self, dx: f64, dy: f64) -> Result<()> {
        self.proxy
            .notify_pointer_axis(
                &self.session,
                dx,
                dy,
                NotifyPointerAxisOptions::default().set_finish(true),
            )
            .await
            .map_err(portal_err)
    }

    /// Scrolls by whole wheel steps (positive: down, right).
    pub async fn axis_discrete(&self, vertical: bool, steps: i32) -> Result<()> {
        let axis = if vertical {
            Axis::Vertical
        } else {
            Axis::Horizontal
        };
        self.proxy
            .notify_pointer_axis_discrete(&self.session, axis, steps, Default::default())
            .await
            .map_err(portal_err)
    }

    /// A key by its Linux evdev code.
    pub async fn keyboard_keycode(&self, keycode: i32, pressed: bool) -> Result<()> {
        self.proxy
            .notify_keyboard_keycode(&self.session, keycode, state(pressed), Default::default())
            .await
            .map_err(portal_err)
    }

    /// A key by its X keysym (layout independent: what a remote controller sends).
    pub async fn keyboard_keysym(&self, keysym: i32, pressed: bool) -> Result<()> {
        self.proxy
            .notify_keyboard_keysym(&self.session, keysym, state(pressed), Default::default())
            .await
            .map_err(portal_err)
    }

    /// One event of the controller.
    pub async fn inject(&self, ev: &InputEvent) -> Result<()> {
        match *ev {
            InputEvent::PointerMove { x, y } => self.pointer_motion_absolute(x, y).await,
            InputEvent::PointerButton { button, pressed } => {
                self.pointer_button(button, pressed).await
            }
            InputEvent::Axis { dx, dy } => self.axis(dx, dy).await,
            InputEvent::Key { keysym, pressed } => {
                let sym = i32::try_from(keysym)
                    .map_err(|_| Error::Media(format!("keysym {keysym:#x} out of range")))?;
                self.keyboard_keysym(sym, pressed).await
            }
        }
    }
}

/// Closes `session` and fails with `e`.
async fn fail<T>(session: Session<RemoteDesktop>, e: Error) -> Result<T> {
    let _ = session.close().await;
    Err(e)
}

/// Asks the desktop for permission to drive the screen described by `shared` (what the share's
/// capture covers) and returns the injector. `Ok(None)` when the user declined. The desktop may
/// show its screen picker again: the same screen or window should be chosen.
pub async fn request_control(shared: ShareArea) -> Result<Option<InputInjector>> {
    super::portal::ensure_registered().await;
    let remote = RemoteDesktop::new().await.map_err(portal_err)?;
    let cast = Screencast::new().await.map_err(portal_err)?;
    let session = remote
        .create_session(Default::default())
        .await
        .map_err(portal_err)?;
    if let Err(e) = remote
        .select_devices(
            &session,
            SelectDevicesOptions::default().set_devices(DeviceType::KEYBOARD | DeviceType::POINTER),
        )
        .await
    {
        return fail(session, portal_err(e)).await;
    }
    let kind = if shared.monitor {
        ShareKind::Screen
    } else {
        ShareKind::Window
    };
    if let Err(e) = select_sources(&cast, &session, kind).await {
        return fail(session, e).await;
    }
    let started = match remote.start(&session, None, Default::default()).await {
        Ok(r) => r.response(),
        Err(e) => return fail(session, portal_err(e)).await,
    };
    let devices = match started {
        Ok(d) => d,
        Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) => {
            let _ = session.close().await;
            return Ok(None);
        }
        Err(e) => return fail(session, portal_err(e)).await,
    };
    let Some(stream) = devices.streams().first() else {
        return fail(session, Error::Media("the portal started no stream".into())).await;
    };
    if !devices.devices().contains(DeviceType::POINTER) {
        let _ = session.close().await;
        return Ok(None);
    }
    let control = ShareArea {
        monitor: stream.source_type() == Some(SourceType::MONITOR),
        position: stream.position(),
        size: stream_size(stream),
    };
    let Some(target) = control_target(&shared, &control) else {
        return fail(
            session,
            Error::Media(format!(
                "can't tell where the shared picture ({shared:?}) is on the controlled screen ({control:?})"
            )),
        )
        .await;
    };
    let node = stream.pipe_wire_node_id();
    tracing::info!(node, ?target, ?control, "remote control allowed");
    Ok(Some(InputInjector {
        proxy: Arc::new(remote),
        session: Arc::new(session),
        stream: node,
        target,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(monitor: bool, position: Option<(i32, i32)>, size: (u32, u32)) -> ShareArea {
        ShareArea {
            monitor,
            position,
            size,
        }
    }

    #[test]
    fn the_same_screen_again_is_the_picture_itself() {
        let a = area(true, Some((1920, 0)), (1920, 1080));
        let b = area(true, Some((1920, 0)), (1920, 1080));
        assert_eq!(
            control_target(&a, &b),
            Some(Target {
                offset: (0.0, 0.0),
                size: (1920, 1080)
            })
        );
        // A window share has no position; the same size is still the same window.
        let w = area(false, None, (800, 600));
        assert!(control_target(&w, &w).is_some());
    }

    #[test]
    fn one_monitor_of_a_whole_desktop_is_offset_by_its_position() {
        let shared = area(true, Some((1920, 0)), (1920, 1080));
        let desktop = area(false, Some((0, 0)), (3840, 1080));
        assert_eq!(
            control_target(&shared, &desktop),
            Some(Target {
                offset: (1920.0, 0.0),
                size: (1920, 1080)
            })
        );
        // The desktop stream names no position: its origin is taken as (0, 0).
        let unplaced = area(false, None, (3840, 1080));
        assert_eq!(
            control_target(&shared, &unplaced).map(|t| t.offset),
            Some((1920.0, 0.0))
        );
    }

    #[test]
    fn a_window_in_a_bigger_stream_is_refused() {
        let window = area(false, None, (800, 600));
        let desktop = area(false, Some((0, 0)), (3840, 1080));
        assert_eq!(control_target(&window, &desktop), None);
        // Nor a monitor that doesn't fit in the stream.
        let big = area(true, Some((0, 0)), (3840, 1080));
        let small = area(true, Some((0, 0)), (1920, 1080));
        assert_eq!(control_target(&big, &small), None);
    }
}
