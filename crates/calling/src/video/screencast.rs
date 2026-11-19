//! A screen or window from the xdg-desktop-portal ScreenCast dialog, as raw I420 frames.
//!
//! The portal (D-Bus, through `ashpd`) shows the desktop's own picker (on KDE Plasma, KWin's) and
//! hands back a PipeWire node and a file descriptor for a PipeWire connection restricted to it.
//! Reading PipeWire in-process needs libpipewire's development files at build time, so kelma
//! starts `gst-launch-1.0` instead: `pipewiresrc` reads the node through the portal's descriptor
//! (the child's stdin), GStreamer converts and scales to I420 at a fixed size and rate, and the
//! frames come back on the child's stdout. Runtime needs GStreamer's PipeWire plugin
//! (`pipewire-gstreamer` on Fedora, `gstreamer1.0-pipewire` on Debian/Ubuntu).

use std::process::Stdio;

use ashpd::desktop::PersistMode;
use ashpd::desktop::Session;
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use tokio::process::Child;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::{I420Frame, fit_within};
use crate::{Error, Result};

/// The size used when the portal names none.
const FALLBACK_SIZE: (u32, u32) = (1280, 720);

/// What the picker offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareKind {
    Screen,
    Window,
    /// Screens and windows; the desktop's dialog lets the user choose.
    Any,
}

/// How the frames come out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureSettings {
    /// The largest size sent; the source is scaled down to fit, keeping its aspect ratio.
    pub max_size: (u32, u32),
    pub fps: u32,
}

impl Default for CaptureSettings {
    /// 720p at 15 frames a second: the media server's answer limits the sharing line to
    /// `max-fs=3600` macroblocks (1280x720; rusty-teams `teams_answer.sdp`), and 15 is the frame
    /// rate the web client asks a share for (`max-fps=1500`, hundredths).
    fn default() -> Self {
        CaptureSettings {
            max_size: (1280, 720),
            fps: 15,
        }
    }
}

/// A running capture. Dropping it stops GStreamer and closes the portal session.
pub struct ScreenCapture {
    pub frames: mpsc::Receiver<I420Frame>,
    /// The size of every frame.
    pub size: (u32, u32),
    child: Child,
    reader: JoinHandle<()>,
    session: Option<Session<Screencast>>,
}

impl ScreenCapture {
    /// The frames, for a consumer that outlives this borrow (an encoder task); `frames` is left
    /// closed.
    pub fn take_frames(&mut self) -> mpsc::Receiver<I420Frame> {
        std::mem::replace(&mut self.frames, mpsc::channel(1).1)
    }

    /// Stops the capture and ends the portal session (the desktop's "sharing" indicator goes).
    pub async fn stop(mut self) {
        self.reader.abort();
        let _ = self.child.kill().await;
        if let Some(session) = self.session.take() {
            let _ = session.close().await;
        }
    }
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        self.reader.abort();
        let _ = self.child.start_kill();
    }
}

fn portal_err(e: ashpd::Error) -> Error {
    Error::Media(format!("ScreenCast portal: {e}"))
}

/// Shows the desktop's picker and starts capturing what the user chose. `Ok(None)` when the user
/// cancelled the dialog.
pub async fn pick(kind: ShareKind, settings: CaptureSettings) -> Result<Option<ScreenCapture>> {
    let proxy = Screencast::new().await.map_err(portal_err)?;
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(portal_err)?;
    let sources = match kind {
        ShareKind::Screen => SourceType::MONITOR,
        ShareKind::Window => SourceType::WINDOW,
        ShareKind::Any => SourceType::MONITOR | SourceType::WINDOW,
    };
    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(CursorMode::EMBEDDED)
                .set_sources(sources)
                .set_multiple(false)
                .set_persist_mode(PersistMode::DoNot),
        )
        .await
        .map_err(portal_err)?;
    let streams = match proxy
        .start(&session, None, Default::default())
        .await
        .map_err(portal_err)?
        .response()
    {
        Ok(s) => s,
        Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) => {
            let _ = session.close().await;
            return Ok(None);
        }
        Err(e) => {
            let _ = session.close().await;
            return Err(portal_err(e));
        }
    };
    let Some(stream) = streams.streams().first() else {
        let _ = session.close().await;
        return Err(Error::Media("the portal started no stream".into()));
    };
    let node = stream.pipe_wire_node_id();
    let source = stream
        .size()
        .and_then(|(w, h)| Some((u32::try_from(w).ok()?, u32::try_from(h).ok()?)))
        .filter(|(w, h)| *w > 0 && *h > 0)
        .unwrap_or(FALLBACK_SIZE);
    let fd = proxy
        .open_pipe_wire_remote(&session, Default::default())
        .await
        .map_err(portal_err)?;
    let size = fit_within(source, settings.max_size);
    let mut capture = spawn_gstreamer(Stdio::from(fd), node, size, settings.fps)?;
    capture.session = Some(session);
    tracing::info!(node, ?source, ?size, "screen capture started");
    Ok(Some(capture))
}

/// The `gst-launch-1.0` arguments reading PipeWire node `node` through fd 0 as I420 frames of
/// `size` at `fps` on fd 1.
pub fn gstreamer_args(node: u32, size: (u32, u32), fps: u32) -> Vec<String> {
    let caps = format!(
        "video/x-raw,format=I420,width={},height={},framerate={fps}/1,pixel-aspect-ratio=1/1",
        size.0, size.1
    );
    [
        "-q",
        "pipewiresrc",
        "fd=0",
        &format!("path={node}"),
        "do-timestamp=true",
        "always-copy=true",
        // A still screen sends no buffers; repeat the last one so the receiver keeps a picture.
        "keepalive-time=1000",
        "!",
        "videoconvert",
        "!",
        "videoscale",
        "add-borders=true",
        "!",
        "videorate",
        "drop-only=true",
        "!",
        &caps,
        "!",
        "fdsink",
        "fd=1",
        "sync=false",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn spawn_gstreamer(
    pipewire: Stdio,
    node: u32,
    size: (u32, u32),
    fps: u32,
) -> Result<ScreenCapture> {
    let g = super::gst::spawn(
        gstreamer_args(node, size, fps),
        pipewire,
        size,
        "screen capture",
    )?;
    Ok(ScreenCapture {
        frames: g.frames,
        size,
        child: g.child,
        reader: g.reader,
        session: None,
    })
}
