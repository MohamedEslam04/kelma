//! The echo canceller's stream delay follows the measured device latency (feature `aec`).
#![cfg(feature = "aec")]

use calling::FRAME_SAMPLES;
use calling::audio::{
    AudioProcessor, DEFAULT_STREAM_DELAY_MS, DelayEstimator, DelaySample, Frame, FrameSource,
    echo_cancelled,
};
use std::collections::VecDeque;
use std::time::Duration;

fn ms(n: u64) -> Option<Duration> {
    Some(Duration::from_millis(n))
}

fn sample(output: Option<Duration>, input: Option<Duration>) -> DelaySample {
    DelaySample {
        output,
        input,
        mic_backlog: Duration::ZERO,
        render_wait: Duration::ZERO,
    }
}

#[test]
fn starts_at_the_default_and_stays_there_without_timestamps() {
    let mut e = DelayEstimator::new();
    assert_eq!(e.delay_ms(), DEFAULT_STREAM_DELAY_MS);
    assert_eq!(e.observe(sample(None, None)), None);
    assert_eq!(e.observe(sample(ms(70), None)), None);
    assert_eq!(e.observe(sample(None, ms(50))), None);
    assert_eq!(e.delay_ms(), DEFAULT_STREAM_DELAY_MS);
}

#[test]
fn nonsensical_latencies_fall_back_to_the_default() {
    let mut e = DelayEstimator::new();
    // A device that claims more than 400 ms either way is not believable.
    assert_eq!(e.observe(sample(ms(4_000), ms(20))), None);
    assert_eq!(e.observe(sample(ms(20), ms(4_000))), None);
    assert_eq!(e.delay_ms(), DEFAULT_STREAM_DELAY_MS);
}

#[test]
fn first_valid_sample_is_adopted_at_once_and_reported() {
    let mut e = DelayEstimator::new();
    assert_eq!(e.observe(sample(ms(70), ms(50))), Some(120));
    assert_eq!(e.delay_ms(), 120);
}

#[test]
fn queueing_adds_on_the_mic_side_and_subtracts_on_the_render_side() {
    let mut e = DelayEstimator::new();
    let s = DelaySample {
        output: ms(70),
        input: ms(50),
        mic_backlog: Duration::from_millis(40),
        render_wait: Duration::from_millis(10),
    };
    assert_eq!(e.observe(s), Some(150));
}

#[test]
fn estimate_is_clamped_to_a_sane_range() {
    let mut low = DelayEstimator::new();
    low.observe(sample(ms(0), ms(0)));
    assert_eq!(low.delay_ms(), 10);
    let mut high = DelayEstimator::new();
    assert_eq!(high.observe(sample(ms(300), ms(300))), Some(400));
}

#[test]
fn small_wander_is_not_reported_but_a_lasting_move_is() {
    let mut e = DelayEstimator::new();
    assert_eq!(e.observe(sample(ms(70), ms(50))), Some(120));
    // Jitter of +-8 ms around 120 never crosses the 10 ms hysteresis.
    for i in 0..500 {
        let wobble = if i % 2 == 0 { 78 } else { 62 };
        assert_eq!(e.observe(sample(ms(wobble), ms(50))), None, "step {i}");
    }
    assert_eq!(e.delay_ms(), 120);
    // A single spike is smoothed away.
    assert_eq!(e.observe(sample(ms(150), ms(50))), None);
    // The latency really moving to 200 ms is followed, and reported once it is clear of the old.
    let mut reports = Vec::new();
    for _ in 0..500 {
        if let Some(d) = e.observe(sample(ms(150), ms(50))) {
            reports.push(d);
        }
    }
    assert!(!reports.is_empty() && reports.len() <= 8, "{reports:?}");
    assert!(reports.windows(2).all(|w| w[1] > w[0]), "{reports:?}");
    assert!((190..=205).contains(&e.delay_ms()), "{}", e.delay_ms());
}

// ---- the canceller with a long echo path ----

const fn secs(s: usize) -> usize {
    s * 50
}

/// Deterministic speech-like bursts (as in `audio_aec.rs`).
struct Talker {
    seed: u64,
    lp: f64,
    left: usize,
    on: bool,
}

impl Talker {
    fn new(seed: u64) -> Self {
        Self {
            seed,
            lp: 0.0,
            left: 0,
            on: false,
        }
    }

    fn rand(&mut self) -> f64 {
        self.seed = self
            .seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.seed >> 11) as f64 / (1u64 << 53) as f64
    }

    fn frame(&mut self) -> Frame {
        (0..FRAME_SAMPLES)
            .map(|_| {
                if self.left == 0 {
                    self.on = !self.on;
                    let ms = if self.on {
                        150.0 + 250.0 * self.rand()
                    } else {
                        50.0 + 150.0 * self.rand()
                    };
                    self.left = (ms * 48.0) as usize;
                }
                self.left -= 1;
                let white = self.rand() * 2.0 - 1.0;
                self.lp += 0.3 * (white - self.lp);
                let v = if self.on { self.lp * 0.5 } else { 0.0 };
                (v * 3.0 * 32767.0).clamp(-32768.0, 32767.0) as i16
            })
            .collect()
    }
}

/// Loudspeaker-to-microphone path: `delay` samples late, attenuated, with a decaying tail.
struct EchoPath {
    history: VecDeque<f64>,
    taps: Vec<(usize, f64)>,
}

impl EchoPath {
    fn new(delay: usize, gain: f64) -> Self {
        let taps = vec![
            (delay, gain),
            (delay + 37, gain * 0.4),
            (delay + 101, gain * -0.25),
            (delay + 240, gain * 0.1),
        ];
        let len = taps.iter().map(|t| t.0).max().unwrap() + 1;
        Self {
            history: std::iter::repeat_n(0.0, len).collect(),
            taps,
        }
    }

    fn frame(&mut self, played: &[i16]) -> Frame {
        played
            .iter()
            .map(|&s| {
                self.history.pop_back();
                self.history.push_front(f64::from(s));
                let y: f64 = self.taps.iter().map(|&(d, g)| self.history[d] * g).sum();
                y.clamp(-32768.0, 32767.0) as i16
            })
            .collect()
    }
}

fn erle_db(mic: &[Frame], out: &[Frame]) -> f64 {
    let energy = |f: &[Frame]| {
        f.iter()
            .flatten()
            .map(|&s| f64::from(s).powi(2))
            .sum::<f64>()
            .max(1.0)
    };
    10.0 * (energy(mic) / energy(out)).log10()
}

/// ERLE over the first 2 s (while it converges) and the last 4 s of 10 s of far-end talk through
/// a 120 ms echo path, with the stream delay set to `delay_ms`.
fn erle_with_delay(delay_ms: u32) -> (f64, f64) {
    let mut p = AudioProcessor::new();
    p.set_stream_delay_ms(delay_ms);
    let mut far = Talker::new(11);
    let mut room = EchoPath::new(120 * 48, 0.5);
    let (mut mic_head, mut out_head) = (Vec::new(), Vec::new());
    let (mut mic_tail, mut out_tail) = (Vec::new(), Vec::new());
    for i in 0..secs(10) {
        let played = far.frame();
        p.process_render(&played).unwrap();
        let mic = room.frame(&played);
        let out = p.process_capture(&mic).unwrap();
        if i < secs(2) {
            mic_head.push(mic);
            out_head.push(out);
        } else if i >= secs(6) {
            mic_tail.push(mic);
            out_tail.push(out);
        }
    }
    (erle_db(&mic_head, &out_head), erle_db(&mic_tail, &out_tail))
}

#[test]
fn measured_delay_cancels_a_120_ms_echo_at_least_as_well_as_the_fixed_40_ms() {
    // Synthetic device timestamps for that path: 70 ms to the speaker, 50 ms back from the mic.
    let mut e = DelayEstimator::new();
    e.observe(sample(ms(70), ms(50)));
    let measured = e.delay_ms();
    assert_eq!(measured, 120);
    let (head_measured, tail_measured) = erle_with_delay(measured);
    let (head_default, tail_default) = erle_with_delay(DEFAULT_STREAM_DELAY_MS);
    println!(
        "ERLE, 120 ms echo, first 2 s / last 4 s: measured delay {head_measured:.1} / \
         {tail_measured:.1} dB, fixed 40 ms {head_default:.1} / {tail_default:.1} dB"
    );
    assert!(tail_measured >= 10.0, "only {tail_measured:.1} dB");
    assert!(
        tail_measured >= tail_default - 1.0,
        "measured {tail_measured:.1} dB vs fixed {tail_default:.1} dB"
    );
    assert!(
        head_measured >= head_default - 1.0,
        "while converging: measured {head_measured:.1} dB vs fixed {head_default:.1} dB"
    );
}

struct Frames(std::sync::mpsc::Receiver<Frame>);

impl FrameSource for Frames {
    async fn next_frame(&mut self) -> Option<Frame> {
        self.0.try_recv().ok()
    }
}

#[tokio::test]
async fn wired_source_keeps_the_default_delay_without_device_timestamps() {
    let (tx, rx) = std::sync::mpsc::channel();
    let (mut source, reference) = echo_cancelled(Frames(rx), AudioProcessor::new());
    for _ in 0..secs(1) {
        reference.push(&vec![0; FRAME_SAMPLES]);
        tx.send(vec![0; FRAME_SAMPLES]).unwrap();
        assert!(source.next_frame().await.is_some());
    }
    assert_eq!(source.stream_delay_ms(), DEFAULT_STREAM_DELAY_MS);
}
