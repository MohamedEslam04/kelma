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
    let mut child = Command::new("gst-launch-1.0")
        .args(args)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Error::Media(format!("gst-launch-1.0 (GStreamer) not started: {e}")))?;
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
