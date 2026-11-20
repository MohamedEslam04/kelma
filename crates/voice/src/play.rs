//! Playing a voice message: one output stream at a time, opened when playback starts and
//! closed when it stops or has been paused (or finished) for a few seconds, so the sound
//! device is not held for nothing.
//!
//! [`Playback`] is the position, speed and end-of-clip bookkeeping and [`DeviceFeeder`]
//! converts its output to the device's rate and channels; neither touches a device, so both
//! are tested with fed buffers. The cpal stream is not `Send`, so it lives on a thread of its
//! own, which also closes it when it has been idle long enough.

use crate::stretch::Stretcher;
use crate::{Clip, Error, Result};
use devices::Remix;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long playback may sit paused or finished before the sound device is released.
const IDLE_CLOSE: Duration = Duration::from_secs(3);
/// How often the stream thread checks for being idle.
const IDLE_CHECK: Duration = Duration::from_millis(250);
const MIN_SPEED: f32 = 0.5;
const MAX_SPEED: f32 = 3.0;

/// What the UI shows, polled about every 50 ms.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayerState {
    /// The message loaded, as given to [`Player::play`]; `None` after [`Player::stop`].
    pub key: Option<String>,
    /// Sound is coming out. False when paused and once the end has been reached.
    pub playing: bool,
    /// How far into the message (at the end: the whole duration).
    pub position: Duration,
    pub duration: Duration,
}

/// The loaded message and where playback is in it.
struct Playback {
    key: Option<String>,
    clip: Arc<Clip>,
    stretcher: Stretcher,
    playing: bool,
    ended: bool,
    /// Bumped whenever the audio jumps (new message, seek, replay), so the feeder drops what
    /// it had already converted.
    generation: u64,
    /// Since when nothing has been playing, if so.
    idle_since: Option<Instant>,
}

impl Playback {
    fn empty() -> Self {
        Self {
            key: None,
            clip: Arc::new(Clip::default()),
            stretcher: Stretcher::new(0, 1.0),
            playing: false,
            ended: false,
            generation: 0,
            idle_since: Some(Instant::now()),
        }
    }

    fn samples_at(&self, position: Duration) -> usize {
        let at = (position.as_secs_f64() * f64::from(self.clip.sample_rate)) as usize;
        at.min(self.clip.samples.len())
    }

    fn load(&mut self, key: &str, clip: Arc<Clip>, from: Duration, speed: f32) {
        self.stretcher = Stretcher::new(clip.sample_rate, speed.clamp(MIN_SPEED, MAX_SPEED));
        self.key = Some(key.to_owned());
        self.clip = clip;
        self.stretcher.seek(self.samples_at(from));
        self.playing = true;
        self.ended = false;
        self.generation += 1;
        self.idle_since = None;
    }

    fn clear(&mut self) {
        *self = Self {
            generation: self.generation + 1,
            ..Self::empty()
        };
    }

    /// Mono samples at the clip's rate for the next stretch of output. Silence while paused;
    /// running out of clip ends playback.
    fn render(&mut self, out: &mut [f32]) {
        if !self.playing {
            out.fill(0.0);
            return;
        }
        let produced = self.stretcher.render(&self.clip.samples, out);
        out[produced..].fill(0.0);
        if produced < out.len() {
            self.playing = false;
            self.ended = true;
            self.idle_since = Some(Instant::now());
        }
    }

    fn pause(&mut self) {
        if self.playing {
            self.playing = false;
            self.idle_since = Some(Instant::now());
        }
    }

    /// Continue, or start again from the beginning when the end had been reached.
    fn resume(&mut self) {
        if self.key.is_none() {
            return;
        }
        if self.ended {
            self.stretcher.seek(0);
            self.ended = false;
            self.generation += 1;
        }
        self.playing = true;
        self.idle_since = None;
    }

    fn seek(&mut self, to: Duration) {
        self.stretcher.seek(self.samples_at(to));
        self.ended = false;
        self.generation += 1;
    }

    fn set_speed(&mut self, speed: f32) {
        self.stretcher.set_speed(speed.clamp(MIN_SPEED, MAX_SPEED));
    }

    fn state(&self) -> PlayerState {
        let duration = self.clip.duration();
        let position = if self.ended {
            duration
        } else if self.clip.sample_rate == 0 {
            Duration::ZERO
        } else {
            let seconds = self.stretcher.position() / f64::from(self.clip.sample_rate);
            Duration::from_secs_f64(seconds).min(duration)
        };
        PlayerState {
            key: self.key.clone(),
            playing: self.playing,
            position,
            duration,
        }
    }

    fn idle_for(&self) -> Option<Duration> {
        self.idle_since.map(|since| since.elapsed())
    }
}

fn locked(playback: &Mutex<Playback>) -> MutexGuard<'_, Playback> {
    playback.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Turns [`Playback`]'s mono output into what the device wants (its rate and channel count),
/// buffer by buffer. Lives in the audio callback.
struct DeviceFeeder {
    device_rate: u32,
    channels: usize,
    remix: Remix,
    /// Converted audio not yet handed to the device.
    queue: Vec<f32>,
    queue_read: usize,
    scratch: Vec<f32>,
    generation: u64,
}

impl DeviceFeeder {
    fn new(device_rate: u32, channels: u16) -> Self {
        Self {
            device_rate,
            channels: usize::from(channels.max(1)),
            remix: Remix::new(device_rate, 1, device_rate, channels),
            queue: Vec::new(),
            queue_read: 0,
            scratch: Vec::new(),
            // Differs from any real generation, so the first call sets up for its clip.
            generation: u64::MAX,
        }
    }

    /// Fill `out` (interleaved, whole frames) from `shared`, in silence when nothing plays.
    fn fill(&mut self, shared: &Mutex<Playback>, out: &mut [f32]) {
        let mut playback = locked(shared);
        let clip_rate = playback.clip.sample_rate.max(1);
        if playback.generation != self.generation {
            self.generation = playback.generation;
            self.remix = Remix::new(clip_rate, 1, self.device_rate, self.channels as u16);
            self.queue.clear();
            self.queue_read = 0;
        }
        let mut filled = 0;
        while filled < out.len() {
            let take = (self.queue.len() - self.queue_read).min(out.len() - filled);
            out[filled..filled + take]
                .copy_from_slice(&self.queue[self.queue_read..self.queue_read + take]);
            self.queue_read += take;
            filled += take;
            if filled == out.len() {
                break;
            }
            if !playback.playing {
                out[filled..].fill(0.0);
                break;
            }
            let frames = (out.len() - filled) / self.channels;
            let source_frames = ((frames as f64 * f64::from(clip_rate)
                / f64::from(self.device_rate))
            .ceil() as usize)
                .max(1);
            self.scratch.resize(source_frames, 0.0);
            playback.render(&mut self.scratch);
            self.queue = self.remix.convert(&self.scratch);
            self.queue_read = 0;
        }
    }
}

/// The thread that owns the output stream.
struct StreamThread {
    stop: Sender<()>,
    thread: JoinHandle<()>,
    open: Arc<AtomicBool>,
}

impl StreamThread {
    fn start(device: Option<String>, shared: Arc<Mutex<Playback>>) -> Result<Self> {
        let open = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let still_open = open.clone();
        let thread = std::thread::spawn(move || {
            let feed_from = shared.clone();
            let opened = devices::open_output_stream(device.as_deref(), move |rate, channels| {
                let mut feeder = DeviceFeeder::new(rate, channels);
                move |out: &mut [f32]| feeder.fill(&feed_from, out)
            });
            let stream = match opened {
                Ok(stream) => stream,
                Err(e) => {
                    let _ = ready_tx.send(Err(e.into()));
                    return;
                }
            };
            still_open.store(true, Ordering::Release);
            let _ = ready_tx.send(Ok(()));
            loop {
                match stop_rx.recv_timeout(IDLE_CHECK) {
                    Err(RecvTimeoutError::Timeout) => {
                        // Decide and mark closed under the lock, so a resume that arrives
                        // meanwhile either sees the stream gone and reopens it, or keeps it.
                        let playback = locked(&shared);
                        if playback.idle_for().is_some_and(|idle| idle >= IDLE_CLOSE) {
                            still_open.store(false, Ordering::Release);
                            break;
                        }
                    }
                    _ => break,
                }
            }
            still_open.store(false, Ordering::Release);
            drop(stream);
        });
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                stop: stop_tx,
                thread,
                open,
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(Error::Audio("the playback thread ended".into())),
        }
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    fn close(self) {
        let _ = self.stop.send(());
        let _ = self.thread.join();
    }
}

/// Plays one voice message at a time.
pub struct Player {
    output_device: Option<String>,
    shared: Arc<Mutex<Playback>>,
    stream: Option<StreamThread>,
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

impl Player {
    /// A player with nothing loaded. No sound device is opened until [`Player::play`].
    pub fn new() -> Self {
        Self {
            output_device: None,
            shared: Arc::new(Mutex::new(Playback::empty())),
            stream: None,
        }
    }

    /// Where sound goes from the next [`Player::play`] on: a device id from
    /// `devices::audio_outputs`, or `None` for the default speakers.
    pub fn set_output_device(&mut self, device: Option<&str>) {
        self.output_device = device.map(str::to_owned);
        // Let the next playback open the new device.
        if let Some(stream) = self.stream.take() {
            stream.close();
        }
    }

    /// Play `clip` from `from` at `speed` (1.0, 1.5 or 2.0 without changing the pitch).
    /// Whatever played before stops. `key` names the message in [`PlayerState`].
    pub fn play(&mut self, key: &str, clip: Arc<Clip>, from: Duration, speed: f32) -> Result<()> {
        locked(&self.shared).load(key, clip, from, speed);
        self.ensure_stream().inspect_err(|_| self.stop())
    }

    pub fn pause(&mut self) {
        locked(&self.shared).pause();
    }

    /// Continue, or play again from the start when the message had ended. Reopens the sound
    /// device if it was released while paused.
    pub fn resume(&mut self) -> Result<()> {
        locked(&self.shared).resume();
        self.ensure_stream().inspect_err(|_| self.pause())
    }

    /// Jump to `position` (past the end counts as the end), keeping playing or paused as it was.
    pub fn seek(&mut self, position: Duration) {
        locked(&self.shared).seek(position);
    }

    /// Change the speed from now on: 1.0, 1.5 or 2.0.
    pub fn set_speed(&mut self, speed: f32) {
        locked(&self.shared).set_speed(speed);
    }

    /// Stop, forget the message and release the sound device.
    pub fn stop(&mut self) {
        locked(&self.shared).clear();
        if let Some(stream) = self.stream.take() {
            stream.close();
        }
    }

    pub fn state(&self) -> PlayerState {
        locked(&self.shared).state()
    }

    /// Open the output stream unless one is running.
    fn ensure_stream(&mut self) -> Result<()> {
        if self.stream.as_ref().is_some_and(StreamThread::is_open) {
            return Ok(());
        }
        if let Some(old) = self.stream.take() {
            old.close();
        }
        self.stream = Some(StreamThread::start(
            self.output_device.clone(),
            self.shared.clone(),
        )?);
        Ok(())
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        if let Some(stream) = self.stream.take() {
            stream.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 8_000;

    fn clip_of_seconds(seconds: usize) -> Arc<Clip> {
        Arc::new(Clip::new(vec![0.5; seconds * RATE as usize], RATE))
    }

    fn loaded(speed: f32) -> Playback {
        let mut playback = Playback::empty();
        playback.load("message", clip_of_seconds(2), Duration::ZERO, speed);
        playback
    }

    fn near(a: Duration, b: Duration) -> bool {
        a.abs_diff(b) < Duration::from_millis(30)
    }

    #[test]
    fn a_fresh_player_has_nothing_loaded() {
        assert_eq!(Player::new().state(), PlayerState::default());
    }

    #[test]
    fn loading_starts_playing_at_the_requested_place() {
        let mut playback = Playback::empty();
        playback.load("a", clip_of_seconds(2), Duration::from_millis(500), 1.0);
        let state = playback.state();
        assert_eq!(state.key.as_deref(), Some("a"));
        assert!(state.playing);
        assert_eq!(state.duration, Duration::from_secs(2));
        assert!(near(state.position, Duration::from_millis(500)));
    }

    #[test]
    fn position_follows_what_was_played_at_normal_speed() {
        let mut playback = loaded(1.0);
        playback.render(&mut vec![0.0; RATE as usize / 2]);
        assert!(near(playback.state().position, Duration::from_millis(500)));
    }

    #[test]
    fn faster_playback_covers_the_message_faster() {
        let mut playback = loaded(2.0);
        playback.render(&mut vec![0.0; RATE as usize / 2]);
        assert!(near(playback.state().position, Duration::from_secs(1)));
        playback.set_speed(1.5);
        playback.render(&mut vec![0.0; RATE as usize / 2]);
        assert!(near(playback.state().position, Duration::from_millis(1750)));
    }

    #[test]
    fn paused_playback_is_silent_and_stands_still() {
        let mut playback = loaded(1.0);
        playback.render(&mut vec![0.0; RATE as usize / 4]);
        playback.pause();
        let at = playback.state().position;
        let mut out = vec![1.0; 1000];
        playback.render(&mut out);
        assert!(out.iter().all(|&s| s == 0.0));
        let state = playback.state();
        assert!(!state.playing);
        assert_eq!(state.position, at);
        playback.resume();
        assert!(playback.state().playing);
        assert_eq!(playback.state().position, at);
    }

    #[test]
    fn reaching_the_end_stops_playing_with_the_position_at_the_end() {
        let mut playback = loaded(1.0);
        let mut out = vec![0.0; 3 * RATE as usize];
        playback.render(&mut out);
        let state = playback.state();
        assert!(!state.playing);
        assert_eq!(state.position, state.duration);
        assert!(out[2 * RATE as usize + 100..].iter().all(|&s| s == 0.0));
        assert!(playback.idle_for().is_some());
    }

    #[test]
    fn resuming_after_the_end_plays_again_from_the_start() {
        let mut playback = loaded(1.0);
        playback.render(&mut vec![0.0; 3 * RATE as usize]);
        playback.resume();
        let state = playback.state();
        assert!(state.playing);
        assert_eq!(state.position, Duration::ZERO);
    }

    #[test]
    fn seeking_moves_the_position_and_clamps_to_the_end() {
        let mut playback = loaded(1.0);
        playback.seek(Duration::from_millis(1200));
        assert!(near(playback.state().position, Duration::from_millis(1200)));
        playback.seek(Duration::from_secs(60));
        assert_eq!(playback.state().position, Duration::from_secs(2));
    }

    #[test]
    fn seeking_while_paused_keeps_it_paused() {
        let mut playback = loaded(1.0);
        playback.pause();
        playback.seek(Duration::from_secs(1));
        assert!(!playback.state().playing);
        assert!(near(playback.state().position, Duration::from_secs(1)));
    }

    #[test]
    fn loading_another_message_replaces_the_first() {
        let mut playback = loaded(1.0);
        playback.render(&mut vec![0.0; 4000]);
        playback.load("b", clip_of_seconds(1), Duration::ZERO, 1.0);
        let state = playback.state();
        assert_eq!(state.key.as_deref(), Some("b"));
        assert_eq!(state.duration, Duration::from_secs(1));
        assert_eq!(state.position, Duration::ZERO);
    }

    #[test]
    fn clearing_forgets_the_message() {
        let mut playback = loaded(1.0);
        playback.clear();
        assert_eq!(playback.state(), PlayerState::default());
        playback.resume();
        assert!(!playback.state().playing, "nothing to resume");
    }

    #[test]
    fn idle_time_runs_only_while_nothing_plays() {
        let mut playback = loaded(1.0);
        assert!(playback.idle_for().is_none());
        playback.pause();
        assert!(playback.idle_for().is_some());
        playback.resume();
        assert!(playback.idle_for().is_none());
    }

    #[test]
    fn feeder_converts_to_the_device_rate_and_channels() {
        let shared = Mutex::new(loaded(1.0));
        // 16 kHz stereo: two output frames per clip sample.
        let mut feeder = DeviceFeeder::new(2 * RATE, 2);
        let mut out = vec![0.0; 2 * 1600];
        feeder.fill(&shared, &mut out);
        assert!(
            out.iter().all(|&s| (s - 0.5).abs() < 1e-3),
            "{:?}",
            &out[..8]
        );
        // 1600 device frames are 800 clip samples = 100 ms.
        let position = locked(&shared).state().position;
        assert!(near(position, Duration::from_millis(100)), "{position:?}");
    }

    #[test]
    fn feeder_handles_odd_buffer_sizes_without_gaps() {
        let shared = Mutex::new(loaded(1.0));
        let mut feeder = DeviceFeeder::new(44_100, 2);
        for _ in 0..50 {
            let mut out = vec![0.0; 2 * 441];
            feeder.fill(&shared, &mut out);
            assert!(out.iter().all(|&s| (s - 0.5).abs() < 1e-3));
        }
    }

    #[test]
    fn feeder_is_silent_when_paused_and_at_the_end() {
        let shared = Mutex::new(loaded(1.0));
        let mut feeder = DeviceFeeder::new(RATE, 1);
        locked(&shared).pause();
        let mut out = vec![1.0; 500];
        feeder.fill(&shared, &mut out);
        assert!(out.iter().all(|&s| s == 0.0));
        locked(&shared).resume();
        let mut out = vec![1.0; 3 * RATE as usize];
        feeder.fill(&shared, &mut out);
        assert!(!locked(&shared).state().playing, "ended");
        assert!(out[2 * RATE as usize + 100..].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn feeder_drops_converted_audio_after_a_seek() {
        let clip = Arc::new(Clip::new(
            (0..2 * RATE as usize)
                .map(|n| if n < RATE as usize { 0.25 } else { 0.75 })
                .collect(),
            RATE,
        ));
        let mut playback = Playback::empty();
        playback.load("x", clip, Duration::ZERO, 1.0);
        let shared = Mutex::new(playback);
        let mut feeder = DeviceFeeder::new(RATE, 1);
        let mut out = vec![0.0; 100];
        feeder.fill(&shared, &mut out);
        assert!((out[99] - 0.25).abs() < 1e-3);
        locked(&shared).seek(Duration::from_millis(1500));
        feeder.fill(&shared, &mut out);
        assert!(out.iter().all(|&s| (s - 0.75).abs() < 1e-3));
    }

    #[test]
    #[ignore = "plays through the real speakers"]
    fn plays_a_tone_on_the_default_output() {
        let tone = Clip::new(crate::test_sine(440.0, 0.5, 48_000), 48_000);
        let mut player = Player::new();
        player
            .play("tone", Arc::new(tone), Duration::ZERO, 1.0)
            .expect("open output");
        std::thread::sleep(Duration::from_millis(800));
        let state = player.state();
        assert!(!state.playing);
        assert_eq!(state.position, state.duration);
    }
}
