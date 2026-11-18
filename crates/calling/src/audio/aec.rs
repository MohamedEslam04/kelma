//! Echo cancellation, noise suppression and automatic gain control for the microphone path,
//! through [sonora](https://crates.io/crates/sonora), a pure Rust port of WebRTC's audio
//! processing module (AEC3, NS, AGC2).
//!
//! The canceller needs to hear what the speaker plays (the *render* or far-end signal) to
//! subtract its echo from what the microphone picks up (the *capture* or near-end signal).
//! [`echo_cancelled`] wires that up: it wraps the microphone [`FrameSource`] and returns an
//! [`EchoReference`] for the playback side to report every frame it hands the speaker.

use super::{Frame, FrameSource};
use crate::{Error, FRAME_SAMPLES, Result, SAMPLE_RATE};
use sonora::config::{
    AdaptiveDigital, EchoCanceller, GainController2, HighPassFilter, NoiseSuppression,
};
use sonora::{AudioProcessing, Config, StreamConfig};
use tokio::sync::mpsc;

/// WebRTC processes 10 ms at a time.
const CHUNK: usize = FRAME_SAMPLES / 2;
/// Render frames queued between the playback callback and the capture side: one second.
const REFERENCE_DEPTH: usize = 50;
/// Starting guess for the render-to-capture delay (device output + input latency). AEC3 finds
/// the actual delay itself; this is only its starting point.
pub const DEFAULT_STREAM_DELAY_MS: u32 = 40;

/// AEC + noise suppression + AGC on 48 kHz mono frames of 10 ms or 20 ms.
///
/// Feed every frame that is played with [`process_render`](Self::process_render) and every
/// microphone frame through [`process_capture`](Self::process_capture), in the order they
/// happen. Both sides take `&mut self`: share it behind a lock, or use [`echo_cancelled`].
pub struct AudioProcessor {
    apm: AudioProcessing,
    stream_delay_ms: u32,
}

impl AudioProcessor {
    pub fn new() -> Self {
        let config = Config {
            high_pass_filter: Some(HighPassFilter::default()),
            echo_canceller: Some(EchoCanceller::default()),
            noise_suppression: Some(NoiseSuppression::default()),
            gain_controller2: Some(GainController2 {
                adaptive_digital: Some(AdaptiveDigital::default()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let stream = StreamConfig::new(SAMPLE_RATE, 1);
        let mut processor = Self {
            apm: AudioProcessing::builder()
                .config(config)
                .capture_config(stream)
                .render_config(stream)
                .build(),
            stream_delay_ms: 0,
        };
        processor.set_stream_delay_ms(DEFAULT_STREAM_DELAY_MS);
        processor
    }

    /// The estimated delay between handing a frame to the speaker and its echo reaching
    /// [`process_capture`](Self::process_capture). Clamped to 0..=500 ms.
    pub fn set_stream_delay_ms(&mut self, ms: u32) {
        self.stream_delay_ms = ms.min(500);
        // Only fails when the value had to be clamped, which it cannot be here.
        let _ = self.apm.set_stream_delay_ms(self.stream_delay_ms as i32);
    }

    pub fn stream_delay_ms(&self) -> u32 {
        self.stream_delay_ms
    }

    /// Analyse a frame that is being played (the far end).
    pub fn process_render(&mut self, frame: &[i16]) -> Result<()> {
        check_len(frame)?;
        let mut scratch = [0i16; CHUNK];
        for chunk in frame.as_chunks::<CHUNK>().0 {
            self.apm
                .process_render_i16(chunk, &mut scratch)
                .map_err(|e| Error::Audio(format!("echo canceller render: {e:?}")))?;
        }
        Ok(())
    }

    /// Clean a microphone frame: remove the far end's echo and noise, level the gain.
    pub fn process_capture(&mut self, frame: &[i16]) -> Result<Frame> {
        check_len(frame)?;
        let mut out = vec![0i16; frame.len()];
        let dsts = out.as_chunks_mut::<CHUNK>().0;
        for (src, dst) in frame.as_chunks::<CHUNK>().0.iter().zip(dsts) {
            // The delay must be set before every capture chunk.
            let _ = self.apm.set_stream_delay_ms(self.stream_delay_ms as i32);
            self.apm
                .process_capture_i16(src, dst)
                .map_err(|e| Error::Audio(format!("echo canceller capture: {e:?}")))?;
        }
        Ok(out)
    }
}

impl Default for AudioProcessor {
    fn default() -> Self {
        Self::new()
    }
}

fn check_len(frame: &[i16]) -> Result<()> {
    if frame.len() == CHUNK || frame.len() == FRAME_SAMPLES {
        Ok(())
    } else {
        Err(Error::Audio(format!(
            "audio processing takes 10 or 20 ms of 48 kHz mono ({CHUNK} or {FRAME_SAMPLES} \
             samples), got {}",
            frame.len()
        )))
    }
}

/// Wrap the microphone `source` so every frame it yields goes through `processor`, and get the
/// [`EchoReference`] the playback side reports played frames to.
///
/// Render frames travel over a bounded queue and are analysed on the capture side, just before
/// the next microphone frame, so the playback callback never waits on the processor.
pub fn echo_cancelled<S: FrameSource>(
    source: S,
    processor: AudioProcessor,
) -> (EchoCancelled<S>, EchoReference) {
    let (tx, rx) = mpsc::channel(REFERENCE_DEPTH);
    (
        EchoCancelled {
            inner: source,
            processor,
            reference: rx,
        },
        EchoReference { tx },
    )
}

/// A microphone [`FrameSource`] whose frames have been through an [`AudioProcessor`].
pub struct EchoCancelled<S> {
    inner: S,
    processor: AudioProcessor,
    reference: mpsc::Receiver<Frame>,
}

impl<S: FrameSource> FrameSource for EchoCancelled<S> {
    async fn next_frame(&mut self) -> Option<Frame> {
        let frame = self.inner.next_frame().await?;
        while let Ok(played) = self.reference.try_recv() {
            if let Err(e) = self.processor.process_render(&played) {
                tracing::warn!("audio processing: {e}");
            }
        }
        match self.processor.process_capture(&frame) {
            Ok(clean) => Some(clean),
            Err(e) => {
                tracing::warn!("audio processing: passing the frame through: {e}");
                Some(frame)
            }
        }
    }
}

/// The playback side's handle: report each 48 kHz mono frame as it is handed to the speaker.
/// Never blocks; if the capture side stops draining, frames are dropped.
#[derive(Clone)]
pub struct EchoReference {
    tx: mpsc::Sender<Frame>,
}

impl EchoReference {
    pub fn push(&self, frame: &[i16]) {
        let _ = self.tx.try_send(frame.to_vec());
    }
}
