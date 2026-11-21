//! A `gst-launch-1.0` child writing raw I420 frames of one fixed size on its stdout, as the
//! screen and the camera capture both run it (no GStreamer or PipeWire development files at build
//! time).

use std::process::Stdio;

use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::I420Frame;
use crate::{Error, Result};

/// Frames buffered between the capture and its consumer; older ones are dropped, not queued.
const FRAME_QUEUE: usize = 2;

/// Overrides where `gst-launch-1.0` is found.
pub const GST_LAUNCH_ENV: &str = "KELMA_GST_LAUNCH";

/// The `gst-launch-1.0` to run: `$KELMA_GST_LAUNCH`, else where the official GStreamer installers
/// put it when that is not on `PATH` (an app started from Finder or the Start menu does not see a
/// shell's `PATH`), else the bare name, found on `PATH`.
pub(crate) fn gst_launch() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os(GST_LAUNCH_ENV).filter(|p| !p.is_empty()) {
        return p.into();
    }
    installed_gst_launch()
        .filter(|p| p.is_file())
        .unwrap_or_else(|| "gst-launch-1.0".into())
}

#[cfg(windows)]
fn installed_gst_launch() -> Option<std::path::PathBuf> {
    // The MSVC installer sets this to its root (for example `C:\gstreamer\1.0\msvc_x86_64\`).
    [
        "GSTREAMER_1_0_ROOT_MSVC_X86_64",
        "GSTREAMER_1_0_ROOT_X86_64",
    ]
    .into_iter()
    .find_map(std::env::var_os)
    .map(|root| {
        std::path::Path::new(&root)
            .join("bin")
            .join("gst-launch-1.0.exe")
    })
}

#[cfg(target_os = "macos")]
fn installed_gst_launch() -> Option<std::path::PathBuf> {
    [
        "/Library/Frameworks/GStreamer.framework/Versions/1.0/bin/gst-launch-1.0",
        "/opt/homebrew/bin/gst-launch-1.0",
        "/usr/local/bin/gst-launch-1.0",
    ]
    .into_iter()
    .map(std::path::PathBuf::from)
    .find(|p| p.is_file())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn installed_gst_launch() -> Option<std::path::PathBuf> {
    None
}

/// A running `gst-launch-1.0` and the frames it writes.
pub(crate) struct GstFrames {
    pub frames: mpsc::Receiver<I420Frame>,
    pub child: Child,
    pub reader: JoinHandle<()>,
}

/// Starts `gst-launch-1.0 <args>` with `stdin` as its fd 0 and reads `size` I420 frames from its
/// stdout. `what` names the capture in logs.
pub(crate) fn spawn(
    args: Vec<String>,
    stdin: Stdio,
    size: (u32, u32),
    what: &'static str,
) -> Result<GstFrames> {
    let mut command = Command::new(gst_launch());
    // No console window flashing up next to the app on Windows.
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command
        .args(args)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            Error::Media(format!(
                "gst-launch-1.0 (GStreamer) not started: {e}. {}",
                if cfg!(target_os = "linux") {
                    "Install GStreamer and its PipeWire plugin."
                } else {
                    "Install the GStreamer runtime from gstreamer.freedesktop.org."
                }
            ))
        })?;
    let mut out = child
        .stdout
        .take()
        .ok_or_else(|| Error::Media("gst-launch-1.0 has no stdout".into()))?;
    let (tx, frames) = mpsc::channel(FRAME_QUEUE);
    let (w, h) = (size.0 as usize, size.1 as usize);
    let reader = tokio::spawn(async move {
        let len = I420Frame::len_for(w, h);
        loop {
            let mut data = vec![0; len];
            if let Err(e) = out.read_exact(&mut data).await {
                tracing::info!(%e, "{what} ended");
                return;
            }
            let Ok(frame) = I420Frame::new(w, h, data) else {
                return;
            };
            match tx.try_send(frame) {
                // The consumer is behind: this frame is dropped, a newer one follows.
                Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {}
                Err(mpsc::error::TrySendError::Closed(_)) => return,
            }
        }
    });
    Ok(GstFrames {
        frames,
        child,
        reader,
    })
}
