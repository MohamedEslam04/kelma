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
    /// A camera GStreamer's device monitor listed: its source element and properties
    /// ([`SystemCamera::source`]; Windows, macOS).
    Device(Vec<String>),
}

/// Which camera to open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CameraChoice {
    /// The portal's camera on Linux (a V4L2 device when the portal has none), the system's
    /// default elsewhere.
    #[default]
    Default,
    /// This V4L2 device (Linux). With a Camera portal, only once it allowed the camera.
    V4l2(String),
    /// A camera the device monitor listed ([`SystemCamera::source`]; Windows, macOS).
    Device(Vec<String>),
}

/// A camera `gst-device-monitor-1.0` lists (Windows, macOS): its name, and the GStreamer source
/// with the properties that open it (`mfvideosrc device-path="..."`, `avfvideosrc device-index=1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemCamera {
    pub name: String,
    pub source: Vec<String>,
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
    open_choice(&CameraChoice::Default, v4l2_device, settings).await
}

/// Starts the camera `choice` names. On Linux the Camera portal asks first either way (the user
/// answers; a refusal is an error, with no V4L2 behind their back); once it allows the camera a
/// chosen V4L2 device is read directly (the portal's PipeWire remote does not say which node is
/// which camera; UNVERIFIED in a Flatpak sandbox, where `/dev/video*` is not visible). Without a
/// portal camera, the chosen device, else `v4l2_fallback` (default `/dev/video0`).
pub async fn open_choice(
    choice: &CameraChoice,
    v4l2_fallback: Option<&str>,
    settings: CameraSettings,
) -> Result<CameraCapture> {
    #[cfg(not(target_os = "linux"))]
    let (source, stdin) = {
        let _ = v4l2_fallback;
        match choice {
            CameraChoice::Device(source) => (CameraSource::Device(source.clone()), Stdio::null()),
            _ => (CameraSource::System, Stdio::null()),
        }
    };
    #[cfg(target_os = "linux")]
    let (source, stdin) = match (portal().await?, choice) {
        (Portal::Remote(_), CameraChoice::V4l2(device))
            if std::path::Path::new(device).exists() =>
        {
            (CameraSource::V4l2(device.clone()), Stdio::null())
        }
        (Portal::Remote(fd), _) => (CameraSource::Portal, Stdio::from(fd)),
        (Portal::NoCamera(why), _) => {
            let device = match choice {
                CameraChoice::V4l2(d) => d.clone(),
                _ => v4l2_fallback.unwrap_or("/dev/video0").to_owned(),
            };
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
        CameraSource::Device(source) => args.extend(source.iter().cloned()),
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

/// The cameras `gst-device-monitor-1.0 Video/Source` lists; empty when it is not installed.
/// Blocks while the monitor runs (well under a second).
pub fn system_cameras() -> Vec<SystemCamera> {
    let monitor = super::gst::gst_launch().with_file_name(if cfg!(windows) {
        "gst-device-monitor-1.0.exe"
    } else {
        "gst-device-monitor-1.0"
    });
    let mut command = std::process::Command::new(monitor);
    // No console window flashing up on Windows.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    match command
        .arg("Video/Source")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    {
        Ok(out) => parse_device_monitor(&String::from_utf8_lossy(&out.stdout)),
        Err(e) => {
            tracing::info!(%e, "no GStreamer device monitor: only the default camera");
            Vec::new()
        }
    }
}

/// The cameras in `gst-device-monitor-1.0`'s listing: each device's `name :` line and the source
/// part of its `gst-launch-1.0 ... ! ...` line.
pub fn parse_device_monitor(text: &str) -> Vec<SystemCamera> {
    let mut cameras = Vec::new();
    let mut name: Option<String> = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with("Device found") {
            name = None;
        } else if let Some(n) = line
            .strip_prefix("name")
            .map(str::trim_start)
            .and_then(|r| r.strip_prefix(':'))
        {
            name.get_or_insert_with(|| n.trim().to_owned());
        } else if let Some(launch) = line.strip_prefix("gst-launch-1.0 ") {
            let source = split_launch(launch.split(" ! ").next().unwrap_or_default());
            if let Some(name) = name.take().filter(|_| !source.is_empty()) {
                cameras.push(SystemCamera { name, source });
            }
        }
    }
    cameras
}

/// A launch line's words: split at spaces outside double quotes, quotes and escapes kept as
/// `gst-launch-1.0` reads them.
fn split_launch(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut escaped) = (false, false);
    for c in s.chars() {
        if escaped {
            word.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' => {
                word.push(c);
                escaped = true;
            }
            '"' => {
                word.push(c);
                quoted = !quoted;
            }
            ' ' if !quoted => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            _ => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_monitor_listing_gives_each_camera_and_its_source() {
        let text = r#"Probing devices...


Device found:

	name  : Integrated Camera
	class : Video/Source
	caps  : video/x-raw, format=NV12, width=1280, height=720, framerate=30/1
	properties:
		device.api = mediafoundation
		device.path = "\\?\usb#vid_04f2&pid_b6d9&mi_00#6&1a2b3c4d&0&0000#{e5323777-f976-4f5b-9b55-b94699c46e44}\global"
	gst-launch-1.0 mfvideosrc device-path="\\\\\?\\usb\#vid_04f2\&pid_b6d9\&mi_00\#6\&1a2b3c4d\&0\&0000\#\{e5323777-f976-4f5b-9b55-b94699c46e44\}\\global" ! ...

Device found:

	name  : USB Camera
	class : Video/Source
	caps  : video/x-raw
	gst-launch-1.0 avfvideosrc device-index=1 ! ...
"#;
        let cams = parse_device_monitor(text);
        assert_eq!(cams.len(), 2);
        assert_eq!(cams[0].name, "Integrated Camera");
        assert_eq!(cams[0].source[0], "mfvideosrc");
        assert_eq!(cams[0].source.len(), 2, "the quoted path is one word");
        assert!(cams[0].source[1].starts_with("device-path=\""));
        assert_eq!(cams[1].name, "USB Camera");
        assert_eq!(cams[1].source, ["avfvideosrc", "device-index=1"]);
        assert!(parse_device_monitor("Probing devices...\n").is_empty());
    }

    #[test]
    fn a_listed_camera_is_read_with_its_own_source() {
        let source = CameraSource::Device(vec!["avfvideosrc".into(), "device-index=1".into()]);
        let args = gstreamer_args(&source, (640, 360), 15);
        assert_eq!(args[..3], ["-q", "avfvideosrc", "device-index=1"]);
        assert_eq!(args[3], "!");
        assert!(args.contains(&"decodebin".to_owned()));
    }

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
