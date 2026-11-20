//! Recording a voice message from the microphone.
//!
//! The audio thread feeds [`Capture`], which holds the pure bookkeeping (pause accounting,
//! the live waveform windows, the length cap) so all of it is tested with fed samples. The
//! cpal stream is not `Send`, so [`Recorder`] keeps it on a small thread of its own and
//! stays usable from any thread.

use crate::peaks::{bar_from_rms, rms};
use crate::{Clip, Error, Result};
use devices::Remix;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

/// Longest voice message. Recording pauses itself when it is reached.
pub const MAX_DURATION: Duration = Duration::from_secs(15 * 60);

/// Rate of everything recorded, whatever the microphone runs at.
const SAMPLE_RATE: u32 = 48_000;
/// Samples behind one live waveform bar: 100 ms.
const BAR_SAMPLES: usize = SAMPLE_RATE as usize / 10;

/// What has been recorded so far: mono samples at [`SAMPLE_RATE`] and whether it is paused.
struct Capture {
    samples: Vec<f32>,
    paused: bool,
    max_samples: usize,
}

impl Capture {
    fn new(max_samples: usize) -> Self {
        Self {
            samples: Vec::new(),
            paused: false,
            max_samples,
        }
    }

    /// Append mono samples. Ignored while paused. At the length cap the audio is cut there
    /// and recording pauses itself for good.
    fn push(&mut self, mono: &[f32]) {
        if self.paused {
            return;
        }
        let room = self.max_samples - self.samples.len();
        self.samples.extend_from_slice(&mono[..mono.len().min(room)]);
        if self.is_full() {
            self.paused = true;
        }
    }

    fn is_full(&self) -> bool {
        self.samples.len() >= self.max_samples
    }

    fn pause(&mut self) {
        self.paused = true;
    }

    /// Continue recording, unless the length cap has been reached.
    fn resume(&mut self) {
        self.paused = self.is_full();
    }

    /// Recorded audio only: the time spent paused is not in the samples.
    fn elapsed(&self) -> Duration {
        Duration::from_secs_f64(self.samples.len() as f64 / f64::from(SAMPLE_RATE))
    }

    /// The last `bars` bars, oldest first, one per 100 ms of audio (the newest may be a
    /// partial one still filling). Before enough audio exists the front is flat.
    fn levels(&self, bars: usize) -> Vec<f32> {
        let total = self.samples.len().div_ceil(BAR_SAMPLES);
        let first = total.saturating_sub(bars);
        let mut out = vec![0.0; bars - (total - first)];
        out.extend((first..total).map(|bar| {
            let end = ((bar + 1) * BAR_SAMPLES).min(self.samples.len());
            bar_from_rms(rms(&self.samples[bar * BAR_SAMPLES..end]))
        }));
        out
    }

    fn clip(&self) -> Clip {
        Clip::new(self.samples.clone(), SAMPLE_RATE)
    }
}

fn locked(capture: &Mutex<Capture>) -> MutexGuard<'_, Capture> {
    capture.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A recording in progress. Dropping it, or [`Recorder::cancel`], stops and discards it.
pub struct Recorder {
    capture: Arc<Mutex<Capture>>,
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Recorder {
    /// Start recording from `input_device` (a device id as listed by `devices::audio_inputs`,
    /// or `None` for the default microphone). Stereo microphones are mixed down to mono.
    pub fn start(input_device: Option<&str>) -> Result<Self> {
        let max_samples = (MAX_DURATION.as_secs() as usize) * SAMPLE_RATE as usize;
        let capture = Arc::new(Mutex::new(Capture::new(max_samples)));
        let device = input_device.map(str::to_owned);
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let shared = capture.clone();
        let thread = std::thread::spawn(move || {
            let opened = devices::open_input_stream(device.as_deref(), move |rate, channels| {
                let mut remix = Remix::new(rate, channels, SAMPLE_RATE, 1);
                move |samples: &[f32]| {
                    let mono = remix.convert(samples);
                    locked(&shared).push(&mono);
                }
            });
            match opened {
                Ok(stream) => {
                    let _ = ready_tx.send(Ok(()));
                    // Returns when the recorder is stopped or dropped.
                    let _ = stop_rx.recv();
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e.into()));
                }
            }
        });
        let ready = ready_rx
            .recv()
            .unwrap_or_else(|_| Err(Error::Audio("the recording thread ended".into())));
        if let Err(e) = ready {
            let _ = thread.join();
            return Err(e);
        }
        Ok(Self {
            capture,
            stop: Some(stop_tx),
            thread: Some(thread),
        })
    }

    /// Stop appending samples. The microphone stays open so [`Recorder::resume`] is instant.
    pub fn pause(&self) {
        locked(&self.capture).pause();
    }

    /// Continue after [`Recorder::pause`]. Does nothing once [`MAX_DURATION`] is reached.
    pub fn resume(&self) {
        locked(&self.capture).resume();
    }

    /// Paused by [`Recorder::pause`] or by reaching [`MAX_DURATION`].
    pub fn is_paused(&self) -> bool {
        locked(&self.capture).paused
    }

    /// Length of the audio recorded so far; pauses are not counted.
    pub fn elapsed(&self) -> Duration {
        locked(&self.capture).elapsed()
    }

    /// The live waveform: the last `bars` bars (0.0..=1.0, oldest first), one per 100 ms of
    /// recorded audio.
    pub fn levels(&self, bars: usize) -> Vec<f32> {
        locked(&self.capture).levels(bars)
    }

    /// What has been recorded so far, to listen back while paused.
    pub fn snapshot(&self) -> Clip {
        locked(&self.capture).clip()
    }

    /// Stop and return everything recorded.
    pub fn finish(mut self) -> Result<Clip> {
        self.close_stream();
        Ok(locked(&self.capture).clip())
    }

    /// Stop and throw the recording away.
    pub fn cancel(self) {}

    fn close_stream(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.close_stream();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_sine;

    const SECOND: usize = SAMPLE_RATE as usize;

    #[test]
    fn elapsed_counts_recorded_audio_only() {
        let mut capture = Capture::new(100 * SECOND);
        capture.push(&vec![0.1; SECOND / 2]);
        capture.pause();
        capture.push(&vec![0.1; 10 * SECOND]);
        assert_eq!(capture.elapsed(), Duration::from_millis(500));
        capture.resume();
        capture.push(&vec![0.1; SECOND / 2]);
        assert_eq!(capture.elapsed(), Duration::from_secs(1));
        assert_eq!(capture.clip().samples.len(), SECOND);
        assert_eq!(capture.clip().sample_rate, SAMPLE_RATE);
    }

    #[test]
    fn levels_give_one_bar_per_hundred_milliseconds() {
        let mut capture = Capture::new(100 * SECOND);
        capture.push(&vec![0.0; SECOND]);
        capture.push(&test_sine(300.0, 1.0, SAMPLE_RATE));
        let bars = capture.levels(30);
        assert_eq!(bars.len(), 30);
        // 2 s of audio is 20 bars: ten flat ones of silence, ten alive, ten of padding up front.
        assert!(bars[..20].iter().all(|&b| b == 0.0));
        assert!(bars[20..].iter().all(|&b| b > 0.3), "{bars:?}");
    }

    #[test]
    fn levels_keep_only_the_newest_bars() {
        let mut capture = Capture::new(100 * SECOND);
        capture.push(&test_sine(300.0, 1.0, SAMPLE_RATE));
        capture.push(&vec![0.0; SECOND]);
        // The last 10 bars cover the silent second only.
        assert_eq!(capture.levels(10), vec![0.0; 10]);
        assert_eq!(capture.levels(0).len(), 0);
    }

    #[test]
    fn levels_include_the_bar_still_filling() {
        let mut capture = Capture::new(100 * SECOND);
        capture.push(&vec![0.0; BAR_SAMPLES]);
        capture.push(&test_sine(300.0, 0.05, SAMPLE_RATE));
        let bars = capture.levels(2);
        assert_eq!(bars[0], 0.0);
        assert!(bars[1] > 0.3, "{bars:?}");
    }

    #[test]
    fn levels_of_nothing_are_flat() {
        assert_eq!(Capture::new(SECOND).levels(4), vec![0.0; 4]);
    }

    #[test]
    fn recording_stops_itself_at_the_length_cap() {
        let mut capture = Capture::new(SECOND);
        capture.push(&vec![0.1; SECOND - 10]);
        assert!(!capture.paused);
        capture.push(&vec![0.1; 100]);
        assert_eq!(capture.samples.len(), SECOND, "cut exactly at the cap");
        assert!(capture.paused, "paused itself");
        capture.resume();
        assert!(capture.paused, "cannot resume past the cap");
        capture.push(&[0.1; 10]);
        assert_eq!(capture.elapsed(), Duration::from_secs(1));
    }

    #[test]
    fn the_cap_is_fifteen_minutes() {
        assert_eq!(MAX_DURATION, Duration::from_secs(900));
    }

    #[test]
    #[ignore = "records from the real microphone"]
    fn records_from_the_default_microphone() {
        let recorder = Recorder::start(None).expect("open microphone");
        std::thread::sleep(Duration::from_millis(400));
        recorder.pause();
        let paused_at = recorder.elapsed();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(recorder.elapsed(), paused_at);
        let clip = recorder.finish().expect("finish");
        assert!(clip.duration() >= Duration::from_millis(300));
    }
}
