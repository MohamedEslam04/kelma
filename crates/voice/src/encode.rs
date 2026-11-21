//! Encoding a recording to the format Teams voice messages use: AAC-LC, mono, 44.1 kHz in an
//! MP4 container (`.m4a`, brand mp42).
//!
//! IronTeams is GPL-3.0-or-later, so it must not link fdk-aac (GPL-incompatible license). On Linux
//! the system `ffmpeg` does the encoding as a child process instead: raw samples go in on
//! stdin, the file comes out of a temporary path because MP4 needs a seekable output. Windows and
//! macOS use the system's own AAC encoder (`native_aac`: Media Foundation, AudioToolbox), so they
//! need no ffmpeg; `IRONTEAMS_FFMPEG` still picks ffmpeg there, and ffmpeg is the fallback when the
//! system encoder fails.

use crate::{Clip, Error, Result};
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Environment variable that names the `ffmpeg` program to run instead of the one on `PATH`.
const FFMPEG_ENV: &str = "IRONTEAMS_FFMPEG";

/// Bit rate of the AAC stream; plenty for speech, small to upload.
const BIT_RATE: &str = "64k";
/// Sample rate Teams voice messages use.
const OUTPUT_RATE: &str = "44100";

/// Encode `clip` as AAC-LC mono in an MP4 container. Linux needs `ffmpeg`:
/// [`Error::EncoderMissing`] when it cannot be found (set `IRONTEAMS_FFMPEG` to point at it).
pub fn encode_m4a(clip: &Clip) -> Result<Vec<u8>> {
    #[cfg(any(windows, target_os = "macos"))]
    if compat::var_os(FFMPEG_ENV).is_none() {
        match encode_native(clip) {
            Ok(bytes) => return Ok(bytes),
            Err(e) if ffmpeg_runs() => {
                eprintln!("ironteams voice: the system AAC encoder failed ({e}); trying ffmpeg");
            }
            Err(e) => return Err(e),
        }
    }
    encode_with(&ffmpeg_program(), clip)
}

/// The system encoder (Windows, macOS) into a temporary file, read back.
#[cfg(any(windows, target_os = "macos"))]
fn encode_native(clip: &Clip) -> Result<Vec<u8>> {
    if clip.samples.is_empty() {
        return Err(Error::Encode("nothing to encode".into()));
    }
    let output = TempFile::create()?;
    crate::native_aac::encode(clip, output.path())?;
    std::fs::read(output.path()).map_err(|e| Error::Encode(format!("read encoded file: {e}")))
}

/// Whether a voice message can be encoded here: always on macOS (AudioToolbox); on Windows when
/// Media Foundation or ffmpeg is there; on Linux when ffmpeg runs.
pub fn encoder_available() -> bool {
    if cfg!(target_os = "macos") {
        return true;
    }
    #[cfg(windows)]
    if crate::native_aac::available() {
        return true;
    }
    ffmpeg_runs()
}

/// The ffmpeg to run: `IRONTEAMS_FFMPEG`, else one shipped next to the executable (`ffmpeg.exe` beside
/// `ironteams.exe`, or in the macOS app's `Contents/MacOS`), else `ffmpeg` from `PATH`.
pub fn ffmpeg_program() -> std::ffi::OsString {
    if let Some(p) = compat::var_os(FFMPEG_ENV) {
        return p;
    }
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(name)))
        .filter(|p| p.is_file())
        .map_or_else(|| "ffmpeg".into(), PathBuf::into_os_string)
}

/// Whether [`ffmpeg_program`] runs.
pub fn ffmpeg_runs() -> bool {
    let mut command = Command::new(ffmpeg_program());
    no_console(&mut command);
    command
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// No console window flashing up for a child process on Windows.
pub fn no_console(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

/// [`encode_m4a`] with the encoder program chosen by the caller.
fn encode_with(program: &OsStr, clip: &Clip) -> Result<Vec<u8>> {
    if clip.samples.is_empty() {
        return Err(Error::Encode("nothing to encode".into()));
    }
    let output = TempFile::create()?;
    let mut command = Command::new(program);
    no_console(&mut command);
    let mut child = command
        .args(ffmpeg_arguments(clip.sample_rate, output.path()))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            ErrorKind::NotFound => Error::EncoderMissing,
            _ => Error::Encode(format!("start ffmpeg: {e}")),
        })?;

    // Feed the samples from another thread: ffmpeg may write diagnostics while it reads, and a
    // single thread doing both could block with both pipes full.
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::Encode("ffmpeg has no stdin".into()))?;
    let pcm = f32_little_endian(&clip.samples);
    let feeder = std::thread::spawn(move || stdin.write_all(&pcm));

    let result = child
        .wait_with_output()
        .map_err(|e| Error::Encode(format!("wait for ffmpeg: {e}")))?;
    // A write error just means ffmpeg quit early; its exit status and messages say why.
    let fed = feeder.join();
    if !result.status.success() {
        let message = String::from_utf8_lossy(&result.stderr);
        return Err(Error::Encode(format!(
            "ffmpeg failed ({}): {}",
            result.status,
            message.trim()
        )));
    }
    if !matches!(fed, Ok(Ok(()))) {
        return Err(Error::Encode("could not send the audio to ffmpeg".into()));
    }
    std::fs::read(output.path()).map_err(|e| Error::Encode(format!("read encoded file: {e}")))
}

/// ffmpeg's command line: raw mono f32 at `input_rate` on stdin, an MP4 at `output`.
fn ffmpeg_arguments(input_rate: u32, output: &Path) -> Vec<std::ffi::OsString> {
    let mut arguments: Vec<std::ffi::OsString> = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "f32le",
        "-ar",
    ]
    .map(Into::into)
    .to_vec();
    arguments.push(input_rate.to_string().into());
    arguments.extend(
        [
            "-ac",
            "1",
            "-i",
            "pipe:0",
            "-vn",
            "-c:a",
            "aac",
            "-b:a",
            BIT_RATE,
            "-ar",
            OUTPUT_RATE,
            "-ac",
            "1",
            "-brand",
            "mp42",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ]
        .map(std::ffi::OsString::from),
    );
    arguments.push(output.into());
    arguments
}

fn f32_little_endian(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

/// A file in the temporary directory, removed when dropped. Created exclusively so another
/// process cannot have put something at the name first.
struct TempFile(PathBuf);

impl TempFile {
    fn create() -> Result<Self> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let name = format!(
            "ironteams-voice-{}-{}-{nanos}.m4a",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| Error::Encode(format!("create temporary file: {e}")))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_sine;

    fn ffmpeg_present() -> bool {
        Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    #[test]
    fn samples_become_little_endian_bytes() {
        assert_eq!(f32_little_endian(&[1.0]), 1.0f32.to_le_bytes());
        assert_eq!(f32_little_endian(&[0.0, 0.5]).len(), 8);
    }

    #[test]
    fn command_line_declares_input_and_teams_output_format() {
        let args: Vec<String> = ffmpeg_arguments(48_000, Path::new("/tmp/out.m4a"))
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let text = args.join(" ");
        assert!(
            text.contains("-f f32le -ar 48000 -ac 1 -i pipe:0"),
            "{text}"
        );
        assert!(text.contains("-c:a aac"), "{text}");
        assert!(text.contains("-ar 44100 -ac 1"), "{text}");
        assert!(text.ends_with("-f mp4 /tmp/out.m4a"), "{text}");
    }

    #[test]
    fn empty_clip_is_refused() {
        assert!(matches!(
            encode_m4a(&Clip::new(Vec::new(), 48_000)),
            Err(Error::Encode(_))
        ));
    }

    #[test]
    fn a_missing_encoder_is_reported_as_such() {
        let clip = Clip::new(vec![0.1; 4800], 48_000);
        let result = encode_with(OsStr::new("ironteams-no-such-ffmpeg"), &clip);
        assert!(matches!(result, Err(Error::EncoderMissing)), "{result:?}");
    }

    #[test]
    fn output_is_an_mp4_with_the_mp42_brand() {
        if !ffmpeg_present() {
            eprintln!("skipped: ffmpeg is not installed");
            return;
        }
        let clip = Clip::new(test_sine(440.0, 0.5, 48_000), 48_000);
        let bytes = encode_with(&ffmpeg_program(), &clip).expect("encode");
        assert_eq!(&bytes[4..8], b"ftyp");
        assert_eq!(&bytes[8..12], b"mp42");
    }

    /// The system encoder (Windows, macOS) writes an MP4 that decodes back to about as long a
    /// clip, with no ffmpeg involved.
    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn the_system_encoder_writes_an_mp4_that_decodes_back() {
        #[cfg(windows)]
        if !crate::native_aac::available() {
            eprintln!("skipped: no Media Foundation on this system");
            return;
        }
        let clip = Clip::new(test_sine(440.0, 1.0, 48_000), 48_000);
        let bytes = encode_native(&clip).expect("encode");
        assert_eq!(&bytes[4..8], b"ftyp");
        #[cfg(target_os = "macos")]
        assert_eq!(&bytes[8..12], b"mp42", "brand");
        eprintln!("brand {:?}", String::from_utf8_lossy(&bytes[8..12]));
        let back = crate::decode(&bytes).expect("decode");
        let secs = back.duration().as_secs_f32();
        assert!((0.9..1.2).contains(&secs), "decoded {secs} s");
    }

    #[test]
    fn a_long_clip_does_not_deadlock() {
        if !ffmpeg_present() {
            eprintln!("skipped: ffmpeg is not installed");
            return;
        }
        // Several megabytes of PCM, far more than a pipe buffer holds.
        let clip = Clip::new(test_sine(300.0, 60.0, 48_000), 48_000);
        assert!(!encode_m4a(&clip).expect("encode").is_empty());
    }
}
