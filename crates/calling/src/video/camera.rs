//! The camera, as raw I420 frames: the xdg-desktop-portal Camera portal first, a V4L2 device
//! when the portal has no camera (or no Camera interface at all).
//!
//! The portal (D-Bus, through `ashpd`) asks the user once and hands back a file descriptor for a
//! PipeWire connection that only shows camera nodes. As for the screen ([`super::screencast`]),
//! `gst-launch-1.0` reads it: `pipewiresrc` on fd 0 with no node named, so it connects to the
//! first camera that connection offers (UNVERIFIED on a machine with several cameras). The V4L2
//! fallback runs `v4l2src` on a device path instead. Either way `decodebin` turns a camera's
//! MJPEG into raw video, and GStreamer scales to a fixed size and rate (black borders keep a 4:3
//! camera's shape in the 16:9 frame).
//!
//! Runtime needs GStreamer's PipeWire plugin (`pipewire-gstreamer` on Fedora,
//! `gstreamer1.0-pipewire` on Debian/Ubuntu) for the portal, `gstreamer1-plugins-good`
//! (`gstreamer1.0-plugins-good`) for V4L2 and a JPEG decoder.
//!
//! Windows and macOS read the system's default camera with GStreamer's own source (`mfvideosrc`,
//! `avfvideosrc`); runtime needs the GStreamer runtime installed (gstreamer.freedesktop.org).

use std::process::Stdio;

#[cfg(target_os = "linux")]
use ashpd::desktop::camera::Camera;
use tokio::process::Child;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::I420Frame;
#[cfg(target_os = "linux")]
use crate::Error;
use crate::Result;

/// Where the frames come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CameraSource {
    /// The Camera portal's PipeWire remote.
    Portal,
    /// A V4L2 device such as `/dev/video0`.
    V4l2(String),
    /// The system's default camera through its own GStreamer source (Windows, macOS).
    System,
}

/// How the frames come out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraSettings {
    pub size: (u32, u32),
    pub fps: u32,
}

impl Default for CameraSettings {
    /// 640x360 at 15 frames a second: small enough for OpenH264 in real time on one core and for
    /// any `max-fs` a media server answers (UNVERIFIED: no capture shows the main-video line's
    /// limits being enforced).
    fn default() -> Self {
        CameraSettings {
            size: (640, 360),
            fps: 15,
        }
    }
}

/// A running camera. Dropping it stops GStreamer (the camera light goes off).
pub struct CameraCapture {
    pub frames: mpsc::Receiver<I420Frame>,
    /// The size of every frame.
    pub size: (u32, u32),
    pub source: CameraSource,
    child: Child,
    reader: JoinHandle<()>,
}

impl CameraCapture {
    /// The frames, for a consumer that outlives this borrow; `frames` is left closed.
    pub fn take_frames(&mut self) -> mpsc::Receiver<I420Frame> {
        std::mem::replace(&mut self.frames, mpsc::channel(1).1)
    }

    pub async fn stop(mut self) {
        self.reader.abort();
        let _ = self.child.kill().await;
    }
}

impl Drop for CameraCapture {
    fn drop(&mut self) {
        self.reader.abort();
        let _ = self.child.start_kill();
    }
}

/// What the portal answered.
#[cfg(target_os = "linux")]
enum Portal {
    /// A PipeWire remote with the cameras.
    Remote(std::os::fd::OwnedFd),
    /// No camera (or no Camera portal): try V4L2.
    NoCamera(String),
}

#[cfg(target_os = "linux")]
async fn portal() -> Result<Portal> {
    super::portal::ensure_registered().await;
    let camera = match Camera::new().await {
        Ok(c) => c,
        Err(e) => return Ok(Portal::NoCamera(format!("no Camera portal: {e}"))),
    };
    match camera.is_present().await {
        Ok(true) => {}
        Ok(false) => return Ok(Portal::NoCamera("the portal sees no camera".into())),
        Err(e) => return Ok(Portal::NoCamera(format!("Camera portal: {e}"))),
    }
    let request = camera
        .request_access(Default::default())
        .await
        .map_err(|e| Error::Media(format!("Camera portal: {e}")))?;
    if let Err(e) = request.response() {
        // The user said no: no V4L2 behind their back.
        return Err(Error::Media(format!("camera access refused: {e}")));
    }
    let fd = camera
        .open_pipe_wire_remote(Default::default())
        .await
        .map_err(|e| Error::Media(format!("Camera portal: {e}")))?;
    Ok(Portal::Remote(fd))
}

/// Starts the camera: through the portal, or `v4l2_device` (default `/dev/video0`) when the
/// portal has none.
pub async fn open(v4l2_device: Option<&str>, settings: CameraSettings) -> Result<CameraCapture> {
    #[cfg(not(target_os = "linux"))]
    let (source, stdin) = {
        let _ = v4l2_device;
        (CameraSource::System, Stdio::null())
    };
    #[cfg(target_os = "linux")]
    let (source, stdin) = match portal().await? {
        Portal::Remote(fd) => (CameraSource::Portal, Stdio::from(fd)),
        Portal::NoCamera(why) => {
            let device = v4l2_device.unwrap_or("/dev/video0").to_owned();
            tracing::info!(%why, %device, "camera from V4L2");
            (CameraSource::V4l2(device), Stdio::null())
        }
    };
    let g = super::gst::spawn(
        gstreamer_args(&source, settings.size, settings.fps),
        stdin,
        settings.size,
        "camera capture",
    )?;
    tracing::info!(?source, size = ?settings.size, "camera started");
    Ok(CameraCapture {
        frames: g.frames,
        size: settings.size,
        source,
        child: g.child,
        reader: g.reader,
    })
}

/// The `gst-launch-1.0` arguments reading `source` as I420 frames of `size` at `fps` on fd 1.
pub fn gstreamer_args(source: &CameraSource, size: (u32, u32), fps: u32) -> Vec<String> {
    let caps = format!(
        "video/x-raw,format=I420,width={},height={},framerate={fps}/1,pixel-aspect-ratio=1/1",
        size.0, size.1
    );
    let mut args: Vec<String> = vec!["-q".into()];
    match source {
        CameraSource::Portal => args.extend(
            [
                "pipewiresrc",
                "fd=0",
                "do-timestamp=true",
                "always-copy=true",
            ]
            .map(String::from),
        ),
        CameraSource::V4l2(device) => {
            args.extend(["v4l2src".into(), format!("device={device}")]);
        }
        CameraSource::System => args.push(
            if cfg!(windows) {
                "mfvideosrc"
            } else if cfg!(target_os = "macos") {
                "avfvideosrc"
            } else {
                "autovideosrc"
            }
            .into(),
        ),
    }
    args.extend(
        [
            "!",
            "decodebin",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_portal_pipeline_reads_fd_0_without_naming_a_node() {
        let args = gstreamer_args(&CameraSource::Portal, (640, 360), 15);
        assert_eq!(args[..3], ["-q", "pipewiresrc", "fd=0"]);
        assert!(!args.iter().any(|a| a.starts_with("path=")));
        assert!(
            args.iter()
                .any(|a| a.contains("width=640,height=360,framerate=15/1"))
        );
        assert_eq!(args.last().map(String::as_str), Some("sync=false"));
    }

    #[test]
    fn the_v4l2_pipeline_names_the_device_and_decodes_mjpeg() {
        let args = gstreamer_args(&CameraSource::V4l2("/dev/video2".into()), (320, 180), 10);
        assert_eq!(args[1..3], ["v4l2src", "device=/dev/video2"]);
        assert!(args.contains(&"decodebin".to_owned()));
    }

    #[test]
    fn the_system_camera_uses_the_platform_source() {
        let args = gstreamer_args(&CameraSource::System, (640, 360), 15);
        let source = if cfg!(windows) {
            "mfvideosrc"
        } else if cfg!(target_os = "macos") {
            "avfvideosrc"
        } else {
            "autovideosrc"
        };
        assert_eq!(args[..2], ["-q", source]);
        assert!(args.contains(&"decodebin".to_owned()));
    }
}
