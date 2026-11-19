use super::{Frame, resample::InputConverter};
use crate::{Error, FRAME_SAMPLES, Result, SAMPLE_RATE};
use std::collections::VecDeque;
use std::future::Future;
use std::io::BufWriter;
use std::path::Path;
use std::time::Duration;
use tokio::time::{Interval, MissedTickBehavior};

/// Where 20 ms mono 48 kHz frames come from.
pub trait FrameSource: Send {
    /// The next frame ([`FRAME_SAMPLES`] samples), or `None` when the source has ended.
    fn next_frame(&mut self) -> impl Future<Output = Option<Frame>> + Send;
}

/// Where decoded 20 ms mono 48 kHz frames go.
pub trait FrameSink: Send {
    fn write_frame(&mut self, frame: &[i16]) -> impl Future<Output = Result<()>> + Send;

    /// Called once when the stream has ended; flushes/finalises the sink.
    fn finish(&mut self) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }
}

/// A sine wave, phase-continuous across frames.
pub struct ToneSource {
    freq: f64,
    amplitude: f64,
    remaining: Option<usize>,
    n: u64,
}

impl ToneSource {
    /// `frames` frames of a `freq_hz` sine at half scale.
    pub fn new(freq_hz: f32, frames: usize) -> Self {
        Self {
            freq: freq_hz.into(),
            amplitude: 0.5,
            remaining: Some(frames),
            n: 0,
        }
    }

    /// A tone that never ends.
    pub fn endless(freq_hz: f32) -> Self {
        Self {
            remaining: None,
            ..Self::new(freq_hz, 0)
        }
    }

    /// Peak amplitude as a fraction of full scale (clamped to 0..=1).
    pub fn with_amplitude(mut self, amplitude: f32) -> Self {
        self.amplitude = f64::from(amplitude).clamp(0.0, 1.0);
        self
    }
}

impl FrameSource for ToneSource {
    async fn next_frame(&mut self) -> Option<Frame> {
        match &mut self.remaining {
            Some(0) => return None,
            Some(r) => *r -= 1,
            None => {}
        }
        let frame = (0..FRAME_SAMPLES)
            .map(|i| {
                let t = (self.n + i as u64) as f64 / f64::from(SAMPLE_RATE);
                (self.amplitude * 32767.0 * (2.0 * std::f64::consts::PI * self.freq * t).sin())
                    as i16
            })
            .collect();
        self.n += FRAME_SAMPLES as u64;
        Some(frame)
    }
}

/// Wraps a source that is not tied to a clock (tone, file) so it delivers one frame per 20 ms
/// like a microphone would. The first frame is immediate.
pub struct Paced<S> {
    inner: S,
    tick: Option<Interval>,
}

impl<S> Paced<S> {
    pub fn new(inner: S) -> Self {
        Self { inner, tick: None }
    }
}

impl<S: FrameSource> FrameSource for Paced<S> {
    async fn next_frame(&mut self) -> Option<Frame> {
        let tick = self.tick.get_or_insert_with(|| {
            let mut i = tokio::time::interval(Duration::from_millis(20));
            i.set_missed_tick_behavior(MissedTickBehavior::Delay);
            i
        });
        tick.tick().await;
        self.inner.next_frame().await
    }
}

/// A WAV file (16-bit int, or float, any rate and channel count) as 48 kHz mono frames. The
/// last partial frame is padded with silence. Reads the whole file up front; meant for tests
/// and live-test fixtures, not long recordings.
pub struct WavSource {
    samples: Vec<i16>,
    cursor: usize,
    chunk: usize,
    conv: InputConverter,
    queue: VecDeque<Frame>,
    flushed: bool,
}

impl WavSource {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let wav_err =
            |e: hound::Error| Error::Audio(format!("wav {}: {e}", path.as_ref().display()));
        let mut reader = hound::WavReader::open(path.as_ref()).map_err(wav_err)?;
        let spec = reader.spec();
        let samples: Vec<i16> = match (spec.sample_format, spec.bits_per_sample) {
            (hound::SampleFormat::Float, _) => reader
                .samples::<f32>()
                .map(|s| s.map(|f| (f.clamp(-1.0, 1.0) * 32767.0) as i16))
                .collect::<std::result::Result<_, _>>(),
            (hound::SampleFormat::Int, bits @ 1..=32) => reader
                .samples::<i32>()
                .map(|s| s.map(|v| (v >> bits.saturating_sub(16)) << 16u16.saturating_sub(bits)))
                .map(|s| s.map(|v| v as i16))
                .collect::<std::result::Result<_, _>>(),
            (_, bits) => {
                return Err(Error::Audio(format!(
                    "wav: unsupported {bits}-bit sample size"
                )));
            }
        }
        .map_err(wav_err)?;
        let channels = usize::from(spec.channels.max(1));
        Ok(Self {
            samples,
            cursor: 0,
            // ~10 ms of input per step, whole sample-frames only.
            chunk: (spec.sample_rate as usize / 100).max(1) * channels,
            conv: InputConverter::new(spec.sample_rate, spec.channels),
            queue: VecDeque::new(),
            flushed: false,
        })
    }
}

impl FrameSource for WavSource {
    async fn next_frame(&mut self) -> Option<Frame> {
        loop {
            if let Some(frame) = self.queue.pop_front() {
                return Some(frame);
            }
            if self.cursor < self.samples.len() {
                let end = (self.cursor + self.chunk).min(self.samples.len());
                self.queue
                    .extend(self.conv.push(&self.samples[self.cursor..end]));
                self.cursor = end;
            } else if !self.flushed {
                self.flushed = true;
                self.queue.extend(self.conv.flush());
            } else {
                return None;
            }
        }
    }
}

/// Records frames to a 48 kHz mono 16-bit WAV file.
pub struct WavSink {
    writer: Option<hound::WavWriter<BufWriter<std::fs::File>>>,
}

impl WavSink {
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let writer = hound::WavWriter::create(path.as_ref(), spec)
            .map_err(|e| Error::Audio(format!("wav {}: {e}", path.as_ref().display())))?;
        Ok(Self {
            writer: Some(writer),
        })
    }
}

impl FrameSink for WavSink {
    async fn write_frame(&mut self, frame: &[i16]) -> Result<()> {
        let w = self
            .writer
            .as_mut()
            .ok_or_else(|| Error::Audio("wav sink already finished".into()))?;
        for &s in frame {
            w.write_sample(s)
                .map_err(|e| Error::Audio(format!("wav write: {e}")))?;
        }
        Ok(())
    }

    async fn finish(&mut self) -> Result<()> {
        if let Some(w) = self.writer.take() {
            w.finalize()
                .map_err(|e| Error::Audio(format!("wav finalize: {e}")))?;
        }
        Ok(())
    }
}
