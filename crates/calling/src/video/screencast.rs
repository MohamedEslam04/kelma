//! A screen or window from the xdg-desktop-portal ScreenCast dialog, as raw I420 frames.
//!
//! The portal (D-Bus, through `ashpd`) shows the desktop's own picker (on KDE Plasma, KWin's) and
//! hands back a PipeWire node and a file descriptor for a PipeWire connection restricted to it.
//! Reading PipeWire in-process needs libpipewire's development files at build time, so kelma
//! starts `gst-launch-1.0` instead: `pipewiresrc` reads the node through the portal's descriptor
//! (the child's stdin), GStreamer converts and scales to I420 at a fixed size and rate, and the
//! frames come back on the child's stdout. Runtime needs GStreamer's PipeWire plugin
//! (`pipewire-gstreamer` on Fedora, `gstreamer1.0-pipewire` on Debian/Ubuntu).
//!
//! Windows and macOS have no portal: GStreamer's own source captures the main screen
//! (`d3d11screencapturesrc` on Windows, `avfvideosrc` on macOS; [`native_gstreamer_args`]) with no
//! picker, so a window share sends the whole screen. Runtime needs the GStreamer runtime installed
//! (gstreamer.freedesktop.org); without it starting a share fails with a message saying so.

use std::process::Stdio;

#[cfg(target_os = "linux")]
use ashpd::desktop::PersistMode;
#[cfg(target_os = "linux")]
use ashpd::desktop::Session;
#[cfg(target_os = "linux")]
use ashpd::desktop::screencast::{
    CursorMode, IsScreencastSession, Screencast, SelectSourcesOptions, SourceType, Stream,
};
use tokio::process::Child;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::I420Frame;
use super::control_overlay::ShareArea;
#[cfg(target_os = "linux")]
use super::fit_within;
#[cfg(target_os = "linux")]
use crate::Error;
use crate::Result;

/// The size used when the portal names none.
#[cfg(target_os = "linux")]
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
    /// What the chosen screen or window covers, as the portal described its stream.
    pub area: ShareArea,
    child: Child,
    reader: JoinHandle<()>,
    #[cfg(target_os = "linux")]
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
        #[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
pub(crate) fn portal_err(e: ashpd::Error) -> Error {
    Error::Media(format!("ScreenCast portal: {e}"))
}

/// Shows the desktop's picker and starts capturing what the user chose. `Ok(None)` when the user
/// cancelled the dialog.
#[cfg(target_os = "linux")]
pub async fn pick(kind: ShareKind, settings: CaptureSettings) -> Result<Option<ScreenCapture>> {
    super::portal::ensure_registered().await;
    let proxy = Screencast::new().await.map_err(portal_err)?;
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(portal_err)?;
    select_sources(&proxy, &session, kind).await?;
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
    let mut capture = match capture_stream(&proxy, &session, stream, settings).await {
        Ok(c) => c,
        Err(e) => {
            let _ = session.close().await;
            return Err(e);
        }
    };
    capture.session = Some(session);
    Ok(Some(capture))
}

/// Asks the portal for one screen or window (`kind`), the cursor drawn into the picture.
#[cfg(target_os = "linux")]
pub(crate) async fn select_sources(
    proxy: &Screencast,
    session: &Session<impl IsScreencastSession>,
    kind: ShareKind,
) -> Result<()> {
    let sources = match kind {
        ShareKind::Screen => SourceType::MONITOR,
        ShareKind::Window => SourceType::WINDOW,
        ShareKind::Any => SourceType::MONITOR | SourceType::WINDOW,
    };
    proxy
        .select_sources(
            session,
            SelectSourcesOptions::default()
                .set_cursor_mode(CursorMode::EMBEDDED)
                .set_sources(sources)
                .set_multiple(false)
                .set_persist_mode(PersistMode::DoNot),
        )
        .await
        .map_err(portal_err)?;
    Ok(())
}

/// The size the portal gave `stream`, or [`FALLBACK_SIZE`].
#[cfg(target_os = "linux")]
pub(crate) fn stream_size(stream: &Stream) -> (u32, u32) {
    stream
        .size()
        .and_then(|(w, h)| Some((u32::try_from(w).ok()?, u32::try_from(h).ok()?)))
        .filter(|(w, h)| *w > 0 && *h > 0)
        .unwrap_or(FALLBACK_SIZE)
}

/// Starts GStreamer on a stream the portal started; the caller keeps the session.
#[cfg(target_os = "linux")]
pub(crate) async fn capture_stream(
    proxy: &Screencast,
    session: &Session<impl IsScreencastSession>,
    stream: &Stream,
    settings: CaptureSettings,
) -> Result<ScreenCapture> {
    let node = stream.pipe_wire_node_id();
    let source = stream_size(stream);
    let fd = proxy
        .open_pipe_wire_remote(session, Default::default())
        .await
        .map_err(portal_err)?;
    let size = fit_within(source, settings.max_size);
    let area = ShareArea {
        monitor: stream.source_type() == Some(SourceType::MONITOR),
        position: stream.position(),
        size: source,
    };
    let capture = spawn_gstreamer(
        gstreamer_args(node, size, settings.fps),
        Stdio::from(fd),
        size,
        area,
    )?;
    tracing::info!(node, ?source, ?size, "screen capture started");
    Ok(capture)
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

/// Windows and macOS: starts capturing the main screen at once (there is no picker to cancel).
/// A window share sends the whole screen too.
#[cfg(not(target_os = "linux"))]
pub async fn pick(kind: ShareKind, settings: CaptureSettings) -> Result<Option<ScreenCapture>> {
    if kind == ShareKind::Window {
        tracing::info!("no window picker on this system: sharing the whole screen");
    }
    // The screen's own size is not known before GStreamer runs, so the picture is scaled into
    // the largest size, black borders keeping its shape.
    let size = settings.max_size;
    let area = ShareArea {
        monitor: true,
        position: None,
        size,
    };
    let capture = spawn_gstreamer(
        native_gstreamer_args(size, settings.fps),
        Stdio::null(),
        size,
        area,
    )?;
    tracing::info!(?size, "screen capture started");
    Ok(Some(capture))
}

/// The `gst-launch-1.0` arguments capturing the main screen with the system's own source
/// (Windows: Desktop Duplication through `d3d11screencapturesrc`, GStreamer 1.20 or newer; macOS:
/// `avfvideosrc`, which needs the Screen Recording permission) as I420 frames of `size` at `fps`
/// on fd 1. UNVERIFIED on real hardware: only checked to build.
pub fn native_gstreamer_args(size: (u32, u32), fps: u32) -> Vec<String> {
    let source: &[&str] = if cfg!(windows) {
        &[
            "d3d11screencapturesrc",
            "monitor-index=0",
            "show-cursor=true",
        ]
    } else {
        &[
            "avfvideosrc",
            "capture-screen=true",
            "capture-screen-cursor=true",
        ]
    };
    let caps = format!(
        "video/x-raw,format=I420,width={},height={},framerate={fps}/1,pixel-aspect-ratio=1/1",
        size.0, size.1
    );
    let mut args: Vec<String> = vec!["-q".into()];
    args.extend(source.iter().map(|s| s.to_string()));
    args.extend(
        [
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
        .map(str::to_owned),
    );
    args
}

fn spawn_gstreamer(
    args: Vec<String>,
    stdin: Stdio,
    size: (u32, u32),
    area: ShareArea,
) -> Result<ScreenCapture> {
    let g = super::gst::spawn(args, stdin, size, "screen capture")?;
    Ok(ScreenCapture {
        frames: g.frames,
        size,
        area,
        child: g.child,
        reader: g.reader,
        #[cfg(target_os = "linux")]
        session: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_portal_pipeline_reads_the_node_through_fd_0() {
        let args = gstreamer_args(42, (1280, 720), 15);
        assert_eq!(args[..4], ["-q", "pipewiresrc", "fd=0", "path=42"]);
        assert_eq!(args.last().map(String::as_str), Some("sync=false"));
    }

    #[test]
    fn the_native_pipeline_scales_the_screen_into_the_frame_size() {
        let args = native_gstreamer_args((1280, 720), 15);
        let source = if cfg!(windows) {
            "d3d11screencapturesrc"
        } else {
            "avfvideosrc"
        };
        assert_eq!(args[..2], ["-q", source]);
        assert!(args.contains(&"add-borders=true".to_owned()));
        assert!(
            args.iter()
                .any(|a| a.contains("format=I420,width=1280,height=720,framerate=15/1"))
        );
        assert_eq!(args.last().map(String::as_str), Some("sync=false"));
    }
}
