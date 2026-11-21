//! Voice messages for IronTeams, the audio engine (no UI, no network).
//!
//! * [`Recorder`] captures the microphone as mono 48 kHz audio with a live waveform.
//! * [`encode_m4a`] turns a [`Clip`] into the AAC-LC mono MP4 that Teams clients play, by
//!   running the system `ffmpeg` on Linux and with the system's own encoder on Windows (Media
//!   Foundation) and macOS (AudioToolbox). IronTeams is GPL-3.0-or-later, so no GPL-incompatible
//!   encoder such as fdk-aac is linked.
//! * [`decode`] reads AAC-in-MP4 (and MP3, which voicemail uses) back with symphonia, pure Rust.
//! * [`peaks`] reduces a whole clip to waveform bars for the message bubble.
//! * [`Player`] plays one clip at a time at 1x, 1.5x or 2x without raising the pitch.

mod decode;
mod encode;
#[cfg(any(windows, target_os = "macos", test))]
mod native_aac;
mod peaks;
mod play;
mod record;
mod stretch;

pub use decode::decode;
pub use encode::{encode_m4a, encoder_available, ffmpeg_program, ffmpeg_runs, no_console};
pub use peaks::peaks;
pub use play::{Player, PlayerState};
pub use record::{MAX_DURATION, Recorder};

use std::time::Duration;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `ffmpeg` was not found (looked at `IRONTEAMS_FFMPEG`, then `PATH`). The UI should say
    /// "Install ffmpeg to send voice messages".
    #[error("ffmpeg is not installed, it is needed to encode voice messages")]
    EncoderMissing,
    #[error("encode: {0}")]
    Encode(String),
    #[error("decode: {0}")]
    Decode(String),
    #[error("audio device: {0}")]
    Audio(String),
}

impl From<devices::Error> for Error {
    fn from(e: devices::Error) -> Self {
        Error::Audio(e.to_string())
    }
}

/// Mono audio: f32 samples in -1.0..=1.0 at `sample_rate` frames per second.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Clip {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl Clip {
    pub fn new(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            samples,
            sample_rate,
        }
    }

    /// Length of the audio.
    pub fn duration(&self) -> Duration {
        if self.sample_rate == 0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(self.samples.len() as f64 / f64::from(self.sample_rate))
    }
}

/// A sine wave at `amplitude`, for tests across modules.
#[cfg(test)]
pub(crate) fn test_sine(freq_hz: f32, seconds: f32, sample_rate: u32) -> Vec<f32> {
    let count = (seconds * sample_rate as f32) as usize;
    (0..count)
        .map(|n| 0.5 * (std::f32::consts::TAU * freq_hz * n as f32 / sample_rate as f32).sin())
        .collect()
}

/// Rising zero crossings per second: the frequency of a clean tone.
#[cfg(test)]
pub(crate) fn test_frequency(samples: &[f32], sample_rate: u32) -> f32 {
    let crossings = samples
        .windows(2)
        .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
        .count();
    crossings as f32 * sample_rate as f32 / samples.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_is_samples_over_rate() {
        let clip = Clip::new(vec![0.0; 24_000], 48_000);
        assert_eq!(clip.duration(), Duration::from_millis(500));
        assert_eq!(Clip::default().duration(), Duration::ZERO);
    }
}
