//! Letting someone else drive the screen we share, through the xdg-desktop-portal RemoteDesktop
//! portal (KDE Plasma and GNOME implement it; Wayland has no other way in).
//!
//! [`pick_with_control`] opens ONE portal session that is both a RemoteDesktop and a ScreenCast
//! session, so the desktop asks once ("share this screen and allow remote control") and the
//! pointer coordinates of [`InputInjector`] are those of the very stream being shared. The
//! injector maps the controller's normalized 0..1 points onto that stream
//! ([`crate::remote_control::stream_point`]).

use std::sync::Arc;

use ashpd::desktop::Session;
use ashpd::desktop::remote_desktop::{
    Axis, DeviceType, KeyState, NotifyPointerAxisOptions, RemoteDesktop, SelectDevicesOptions,
};
use ashpd::desktop::screencast::{Screencast, SourceType};

use super::control_overlay::ShareArea;
use super::screencast::{
    CaptureSettings, PortalSession, ScreenCapture, ShareKind, capture_stream, portal_err,
    select_sources, stream_size,
};
use crate::remote_control::{InputEvent, MouseButton, stream_point};
use crate::{Error, Result};

/// Injects the controller's input into the shared stream. Cheap to clone.
#[derive(Clone)]
pub struct InputInjector {
    proxy: Arc<RemoteDesktop>,
    session: Arc<Session<RemoteDesktop>>,
    /// The PipeWire node of the shared stream (pointer coordinates are relative to it).
    stream: u32,
    /// The stream's size, in its own coordinates.
    size: (u32, u32),
    /// What the stream covers on the desktop, for the controller's drawn cursor.
    area: ShareArea,
}

fn state(pressed: bool) -> KeyState {
    if pressed {
        KeyState::Pressed
    } else {
        KeyState::Released
    }
}

impl InputInjector {
    /// What the shared stream covers (`video::control_overlay`).
    pub fn area(&self) -> ShareArea {
        self.area
    }

    /// Moves the pointer to a normalized point (0..=1) of the shared stream.
    pub async fn pointer_motion_absolute(&self, x: f64, y: f64) -> Result<()> {
        let (x, y) = stream_point(x, y, self.size);
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

/// Shows the desktop's dialog for a screen or window AND remote control in one session, and
/// starts capturing. `Ok(None)` when the user cancelled. The capture and the injector share the
/// portal session; stopping the capture ends both.
pub async fn pick_with_control(
    kind: ShareKind,
    settings: CaptureSettings,
) -> Result<Option<(ScreenCapture, InputInjector)>> {
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
        tracing::warn!("the RemoteDesktop portal granted no pointer: control will not move it");
    }
    let node = stream.pipe_wire_node_id();
    let size = stream_size(stream);
    let area = ShareArea {
        monitor: stream.source_type() == Some(SourceType::MONITOR),
        position: stream.position(),
        size,
    };
    let mut capture = match capture_stream(&cast, &session, stream, settings).await {
        Ok(c) => c,
        Err(e) => return fail(session, e).await,
    };
    let session = Arc::new(session);
    capture.attach(PortalSession::Remote(Arc::clone(&session)));
    tracing::info!(node, ?area, "screen capture with remote control started");
    Ok(Some((
        capture,
        InputInjector {
            proxy: Arc::new(remote),
            session,
            stream: node,
            size,
            area,
        },
    )))
}
