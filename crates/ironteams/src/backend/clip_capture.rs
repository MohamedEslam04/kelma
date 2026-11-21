//! The real camera and microphone behind the video clip recorder (`video_clips`).
//!
//! The camera is the one calls use (`calling::video::camera`: the xdg-desktop-portal Camera
//! portal and PipeWire, else a V4L2 device), as raw I420 frames at 640x360, 15 fps. While
//! recording, every frame goes to the system `ffmpeg` on stdin, which encodes H.264 into a
//! temporary MP4 (stamped with the arrival time, so dropped frames do not shorten the video); the
//! microphone is recorded by `voice::Recorder`. On Stop a second `ffmpeg` run adds the audio as
//! AAC. IronTeams is GPL-3.0-or-later and links no H.264 or AAC encoder: ffmpeg picks the H.264
//! encoder it has (`-c:v h264`: libx264, OpenH264, …).
//!
//! Builds without the `calls` feature have no camera: opening the recorder says so.

use super::video_clips::{ClipCamera, ClipDevices};
use futures_util::future::BoxFuture;
use std::ffi::OsString;
use std::path::Path;

pub(super) struct RealDevices;

impl ClipDevices for RealDevices {
    fn open(&self) -> BoxFuture<'static, anyhow::Result<Box<dyn ClipCamera>>> {
        Box::pin(open())
    }
}

#[cfg(not(feature = "calls"))]
async fn open() -> anyhow::Result<Box<dyn ClipCamera>> {
    anyhow::bail!("This build of IronTeams has no camera support.")
}

#[cfg(feature = "calls")]
async fn open() -> anyhow::Result<Box<dyn ClipCamera>> {
    live::open().await
}

/// The program to run for ffmpeg (`IRONTEAMS_FFMPEG`, else one shipped beside IronTeams, else `ffmpeg` on
/// `PATH`), as voice messages. Windows and macOS packages ship none: video clips need ffmpeg
/// installed there.
#[cfg_attr(not(feature = "calls"), allow(dead_code))]
fn ffmpeg() -> std::process::Command {
    let mut command = std::process::Command::new(voice::ffmpeg_program());
    voice::no_console(&mut command);
    command
}

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

/// ffmpeg's command line for the picture: raw I420 frames of `size` on stdin, H.264 in MP4 at
/// `output`, no longer than the clip limit.
pub(super) fn video_args(size: (u32, u32), fps: u32, output: &Path) -> Vec<OsString> {
    let mut a = os(&[
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "yuv420p",
        "-video_size",
    ]);
    a.push(format!("{}x{}", size.0, size.1).into());
    a.extend(os(&["-framerate"]));
    a.push(fps.to_string().into());
    a.extend(os(&[
        "-use_wallclock_as_timestamps",
        "1",
        "-i",
        "pipe:0",
        "-t",
        "60",
        "-an",
        "-c:v",
        "h264",
        "-pix_fmt",
        "yuv420p",
        "-b:v",
        "1M",
        "-r",
    ]));
    a.push(fps.to_string().into());
    a.extend(os(&["-f", "mp4"]));
    a.push(output.into());
    a
}

/// ffmpeg's command line joining the picture with the microphone: `audio` is raw mono f32 at
/// `rate`, encoded as AAC; the picture is copied. The result is at `output`.
pub(super) fn mux_args(video: &Path, audio: &Path, rate: u32, output: &Path) -> Vec<OsString> {
    let mut a = os(&["-hide_banner", "-loglevel", "error", "-y", "-i"]);
    a.push(video.into());
    a.extend(os(&["-f", "f32le", "-ar"]));
    a.push(rate.to_string().into());
    a.extend(os(&["-ac", "1", "-i"]));
    a.push(audio.into());
    a.extend(os(&[
        "-map",
        "0:v:0",
        "-map",
        "1:a:0",
        "-c:v",
        "copy",
        "-c:a",
        "aac",
        "-b:a",
        "96k",
        "-shortest",
        "-movflags",
        "+faststart",
        "-f",
        "mp4",
    ]));
    a.push(output.into());
    a
}

#[cfg(feature = "calls")]
mod live {
    use super::super::video_clips::{ClipCamera, Picture, RecordedClip};
    use super::{ffmpeg, mux_args, video_args};
    use calling::video::camera::{CameraCapture, CameraSettings};
    use calling::video::{I420Frame, i420_to_rgb, step_for};
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::{Child, Stdio};
    use std::sync::mpsc::{SyncSender, sync_channel};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// The preview is drawn no wider than this.
    const PREVIEW_WIDTH: usize = 360;
    /// Frames waiting for ffmpeg; more are dropped (a slow encoder must not grow memory).
    const BACKLOG: usize = 30;

    type Latest = Arc<Mutex<Option<I420Frame>>>;
    type Sink = Arc<Mutex<Option<SyncSender<Vec<u8>>>>>;

    struct Recording {
        started: Instant,
        dir: PathBuf,
        ffmpeg: Child,
        writer: std::thread::JoinHandle<()>,
        mic: voice::Recorder,
        first: Option<I420Frame>,
    }

    pub(super) struct LiveCamera {
        _capture: CameraCapture,
        latest: Latest,
        sink: Sink,
        pump: tokio::task::JoinHandle<()>,
        settings: CameraSettings,
        rec: Option<Recording>,
    }

    /// True when ffmpeg can be run.
    fn encoder_present() -> bool {
        ffmpeg()
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    pub(super) async fn open() -> anyhow::Result<Box<dyn ClipCamera>> {
        if !tokio::task::spawn_blocking(encoder_present).await? {
            anyhow::bail!("Install ffmpeg to record video clips: it makes the clip Teams plays.");
        }
        let device = tokio::task::spawn_blocking(crate::backend::call::camera::v4l2_device)
            .await
            .ok()
            .flatten();
        let settings = CameraSettings::default();
        let mut capture = calling::video::camera::open(device.as_deref(), settings)
            .await
            .map_err(|e| {
                anyhow::anyhow!(crate::backend::call::camera::open_error_text(
                    &e.to_string()
                ))
            })?;
        let mut frames = capture.take_frames();
        let latest: Latest = Arc::default();
        let sink: Sink = Arc::default();
        let pump = {
            let (latest, sink) = (latest.clone(), sink.clone());
            tokio::spawn(async move {
                while let Some(frame) = frames.recv().await {
                    if let Some(tx) = sink.lock().unwrap().as_ref() {
                        let _ = tx.try_send(frame.data.clone());
                    }
                    *latest.lock().unwrap() = Some(frame);
                }
            })
        };
        Ok(Box::new(LiveCamera {
            _capture: capture,
            latest,
            sink,
            pump,
            settings,
            rec: None,
        }))
    }

    /// `frame` as RGB no wider than `max_width`; the live preview is `mirror`ed, as a self-view.
    fn picture(frame: &I420Frame, max_width: usize, mirror: bool) -> Picture {
        let rgb = i420_to_rgb(frame, step_for(frame.width, max_width), mirror);
        Picture {
            width: rgb.width as u32,
            height: rgb.height as u32,
            rgb: rgb.data,
        }
    }

    impl ClipCamera for LiveCamera {
        fn picture(&mut self) -> Option<Picture> {
            let latest = self.latest.lock().unwrap();
            latest.as_ref().map(|f| picture(f, PREVIEW_WIDTH, true))
        }

        fn record(&mut self) -> anyhow::Result<()> {
            if self.rec.is_some() {
                return Ok(());
            }
            let dir = session::Paths::new()?.cache_dir.join("clip-recording");
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir)?;
            let mic = session::Paths::new()
                .map(|p| crate::backend::settings::Local::load(&p.config_dir).devices)
                .unwrap_or_default()
                .microphone;
            let mic = voice::Recorder::start(mic.as_deref())
                .map_err(|e| anyhow::anyhow!("Couldn't open the microphone: {e}"))?;
            let mut child = ffmpeg()
                .args(video_args(
                    self.settings.size,
                    self.settings.fps,
                    &dir.join("video.mp4"),
                ))
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| anyhow::anyhow!("ffmpeg has no stdin"))?;
            let (tx, rx) = sync_channel::<Vec<u8>>(BACKLOG);
            // Closing the channel (Stop) ends this thread, which closes ffmpeg's stdin.
            let writer = std::thread::spawn(move || {
                for frame in rx {
                    if stdin.write_all(&frame).is_err() {
                        break;
                    }
                }
            });
            let first = self.latest.lock().unwrap().clone();
            *self.sink.lock().unwrap() = Some(tx);
            self.rec = Some(Recording {
                started: Instant::now(),
                dir,
                ffmpeg: child,
                writer,
                mic,
                first,
            });
            Ok(())
        }

        fn elapsed(&self) -> Duration {
            self.rec
                .as_ref()
                .map_or(Duration::ZERO, |r| r.started.elapsed())
        }

        fn stop(&mut self) -> anyhow::Result<RecordedClip> {
            let mut rec = self
                .rec
                .take()
                .ok_or_else(|| anyhow::anyhow!("not recording"))?;
            *self.sink.lock().unwrap() = None;
            let duration = rec.started.elapsed().min(super::super::video_clips::LIMIT);
            let _ = rec.writer.join();
            let status = rec.ffmpeg.wait()?;
            let audio = rec.mic.finish()?;
            let dir = rec.dir.clone();
            let result = (|| -> anyhow::Result<Vec<u8>> {
                if !status.success() {
                    anyhow::bail!("ffmpeg could not encode the video ({status})");
                }
                let video = dir.join("video.mp4");
                let output = if audio.samples.is_empty() {
                    video
                } else {
                    let raw = dir.join("audio.f32");
                    let pcm: Vec<u8> = audio.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
                    std::fs::write(&raw, pcm)?;
                    let output = dir.join("clip.mp4");
                    let out = ffmpeg()
                        .args(mux_args(&video, &raw, audio.sample_rate, &output))
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::piped())
                        .output()?;
                    if !out.status.success() {
                        anyhow::bail!(
                            "ffmpeg could not add the sound: {}",
                            String::from_utf8_lossy(&out.stderr).trim()
                        );
                    }
                    output
                };
                Ok(std::fs::read(output)?)
            })();
            let _ = std::fs::remove_dir_all(&dir);
            let first = rec
                .first
                .take()
                .or_else(|| self.latest.lock().unwrap().clone());
            Ok(RecordedClip {
                mp4: result?,
                // As the clip shows it: not mirrored.
                thumbnail: first.as_ref().map(|f| picture(f, f.width, false)),
                duration,
                size: self.settings.size,
            })
        }
    }

    impl Drop for LiveCamera {
        fn drop(&mut self) {
            self.pump.abort();
            *self.sink.lock().unwrap() = None;
            if let Some(mut rec) = self.rec.take() {
                let _ = rec.ffmpeg.kill();
                let _ = rec.ffmpeg.wait();
                rec.mic.cancel();
                let _ = std::fs::remove_dir_all(&rec.dir);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_is_raw_i420_in_and_h264_mp4_out() {
        let a = video_args((640, 360), 15, Path::new("/tmp/v.mp4"));
        let s: Vec<String> = a.iter().map(|x| x.to_string_lossy().into_owned()).collect();
        let at = |k: &str| s.iter().position(|x| x == k).unwrap();
        assert_eq!(s[at("-f") + 1], "rawvideo");
        assert_eq!(s[at("-video_size") + 1], "640x360");
        assert_eq!(s[at("-framerate") + 1], "15");
        // Input options come before the input.
        assert!(at("-use_wallclock_as_timestamps") < at("-i"));
        assert_eq!(s[at("-i") + 1], "pipe:0");
        assert_eq!(s[at("-c:v") + 1], "h264");
        assert_eq!(s[at("-t") + 1], "60");
        assert_eq!(s.last().unwrap(), "/tmp/v.mp4");
    }

    #[test]
    fn the_sound_is_added_as_aac_and_the_picture_copied() {
        let a = mux_args(
            Path::new("/t/video.mp4"),
            Path::new("/t/audio.f32"),
            48_000,
            Path::new("/t/clip.mp4"),
        );
        let s: Vec<String> = a.iter().map(|x| x.to_string_lossy().into_owned()).collect();
        let joined = s.join(" ");
        assert!(joined.contains("-i /t/video.mp4 -f f32le -ar 48000 -ac 1 -i /t/audio.f32"));
        assert!(joined.contains("-c:v copy -c:a aac"));
        assert!(joined.contains("-movflags +faststart"));
        assert_eq!(s.last().unwrap(), "/t/clip.mp4");
    }
}
