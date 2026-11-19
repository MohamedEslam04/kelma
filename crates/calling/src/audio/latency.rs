//! Device latency, as the audio backend reports it, and the echo-canceller stream delay derived
//! from it.
//!
//! The canceller wants the time between a far-end frame being handed to its render side and the
//! echo of that frame reaching its capture side. That is the output latency (output callback to
//! speaker) plus the input latency (mic to input callback), plus any queueing in between.
//! [`Capture`](super::Capture) and [`Playback`](super::Playback) publish the first two from
//! cpal's callback timestamps into [`device_latency`] (lock-free atomics, written on the audio
//! threads); the echo-cancelled source reads them and feeds a [`DelayEstimator`].

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

/// Starting guess for the render-to-capture delay, and the value used whenever the device
/// reports no usable latency.
pub const DEFAULT_STREAM_DELAY_MS: u32 = 40;
/// The estimate is kept within this range.
const MIN_DELAY_MS: f32 = 10.0;
const MAX_DELAY_MS: f32 = 400.0;
/// A single device latency above this is not believable (clock jump, bogus timestamps).
const MAX_DEVICE_LATENCY_MS: f32 = 400.0;
/// Weight of each new sample in the smoothed estimate (one sample per 20 ms mic frame, so a
/// time constant of about 0.4 s).
const SMOOTHING: f32 = 0.05;
/// The reported delay only moves when the smoothed estimate is further than this from it.
const HYSTERESIS_MS: f32 = 10.0;

const UNKNOWN: u32 = u32::MAX;

/// The most recent latencies the audio callbacks measured. Written on the audio threads, read
/// anywhere; every value is one atomic.
pub struct DeviceLatency {
    output_us: AtomicU32,
    input_us: AtomicU32,
    mic_backlog_us: AtomicU32,
}

impl DeviceLatency {
    const fn new() -> Self {
        Self {
            output_us: AtomicU32::new(UNKNOWN),
            input_us: AtomicU32::new(UNKNOWN),
            mic_backlog_us: AtomicU32::new(0),
        }
    }

    /// From the output callback: the time from now until the samples it writes are played.
    pub fn set_output(&self, latency: Option<Duration>) {
        store(&self.output_us, latency);
    }

    /// From the input callback: the time since the samples it receives were captured.
    pub fn set_input(&self, latency: Option<Duration>) {
        store(&self.input_us, latency);
    }

    /// Audio sitting in the queue between the input callback and its consumer.
    pub fn set_mic_backlog(&self, backlog: Duration) {
        store(&self.mic_backlog_us, Some(backlog));
    }

    pub fn output(&self) -> Option<Duration> {
        load(&self.output_us)
    }

    pub fn input(&self) -> Option<Duration> {
        load(&self.input_us)
    }

    pub fn mic_backlog(&self) -> Duration {
        load(&self.mic_backlog_us).unwrap_or_default()
    }
}

fn store(cell: &AtomicU32, value: Option<Duration>) {
    let us = value.map_or(UNKNOWN, |d| {
        u32::try_from(d.as_micros())
            .unwrap_or(UNKNOWN - 1)
            .min(UNKNOWN - 1)
    });
    cell.store(us, Ordering::Relaxed);
}

fn load(cell: &AtomicU32) -> Option<Duration> {
    match cell.load(Ordering::Relaxed) {
        UNKNOWN => None,
        us => Some(Duration::from_micros(u64::from(us))),
    }
}

static DEVICE_LATENCY: DeviceLatency = DeviceLatency::new();

/// The latencies of the microphone and speaker streams the process has open (a call has one of
/// each).
pub fn device_latency() -> &'static DeviceLatency {
    &DEVICE_LATENCY
}

/// One look at the latencies: what the devices report plus the queueing around the canceller.
#[derive(Debug, Clone, Copy, Default)]
pub struct DelaySample {
    /// Output callback to speaker, if the backend reports it.
    pub output: Option<Duration>,
    /// Mic to input callback, if the backend reports it.
    pub input: Option<Duration>,
    /// Captured audio waiting between the input callback and the canceller. The echo is that
    /// much later when it arrives.
    pub mic_backlog: Duration,
    /// How long the render frames being handed to the canceller waited in the reference queue
    /// since the speaker callback. The canceller sees them that much later than the speaker
    /// did, so the echo is that much closer.
    pub render_wait: Duration,
}

/// Turns [`DelaySample`]s into the stream delay to give the canceller: smoothed, clamped to
/// 10..=400 ms, and moving only when the estimate has moved by more than 10 ms, so the canceller
/// is not retuned on jitter. Without usable device latencies it stays at
/// [`DEFAULT_STREAM_DELAY_MS`]; if they vanish after being seen, the last estimate stands.
#[derive(Debug, Clone)]
pub struct DelayEstimator {
    smoothed_ms: Option<f32>,
    reported_ms: u32,
}

impl DelayEstimator {
    pub fn new() -> Self {
        Self {
            smoothed_ms: None,
            reported_ms: DEFAULT_STREAM_DELAY_MS,
        }
    }

    /// The delay to use now.
    pub fn delay_ms(&self) -> u32 {
        self.reported_ms
    }

    /// Fold in a sample. Returns the new delay when it changed.
    pub fn observe(&mut self, s: DelaySample) -> Option<u32> {
        let (Some(out), Some(inp)) = (s.output, s.input) else {
            return None;
        };
        let (out, inp) = (millis(out), millis(inp));
        if out > MAX_DEVICE_LATENCY_MS || inp > MAX_DEVICE_LATENCY_MS {
            return None;
        }
        let raw = (out + inp + millis(s.mic_backlog) - millis(s.render_wait))
            .clamp(MIN_DELAY_MS, MAX_DELAY_MS);
        let smoothed = match self.smoothed_ms {
            Some(prev) => prev + SMOOTHING * (raw - prev),
            // Nothing to smooth against yet: believe the first measurement.
            None => raw,
        };
        self.smoothed_ms = Some(smoothed);
        if (smoothed - self.reported_ms as f32).abs() > HYSTERESIS_MS {
            self.reported_ms = smoothed.round() as u32;
            Some(self.reported_ms)
        } else {
            None
        }
    }
}

impl Default for DelayEstimator {
    fn default() -> Self {
        Self::new()
    }
}

fn millis(d: Duration) -> f32 {
    d.as_secs_f32() * 1000.0
}
