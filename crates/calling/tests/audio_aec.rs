//! Echo cancellation, noise suppression and AGC on 48 kHz mono frames (feature `aec`).
#![cfg(feature = "aec")]

use calling::FRAME_SAMPLES;
use calling::audio::{AudioProcessor, Frame, FrameSource, echo_cancelled};
use std::collections::VecDeque;

/// 10 ms at 48 kHz.
const HALF_FRAME: usize = FRAME_SAMPLES / 2;
/// Seconds of audio, in 20 ms frames.
const fn secs(s: usize) -> usize {
    s * 50
}

/// Deterministic speech-like signal: low-passed noise switched on and off in syllable-sized
/// bursts (150..400 ms on, 50..200 ms off), so it is neither stationary nor a pure tone.
struct Talker {
    seed: u64,
    lp: f64,
    left: usize,
    on: bool,
    amp: f64,
}

impl Talker {
    fn new(seed: u64, amp: f64) -> Self {
        Self {
            seed,
            lp: 0.0,
            left: 0,
            on: false,
            amp,
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
                let v = if self.on { self.lp * self.amp } else { 0.0 };
                (v * 3.0 * 32767.0).clamp(-32768.0, 32767.0) as i16
            })
            .collect()
    }
}

/// Loudspeaker-to-microphone path: `delay` samples late, attenuated, with a short decaying tail
/// (a small room).
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

fn energy(frames: &[Frame]) -> f64 {
    frames
        .iter()
        .flatten()
        .map(|&s| f64::from(s).powi(2))
        .sum::<f64>()
        .max(1.0)
}

fn db(ratio: f64) -> f64 {
    10.0 * ratio.log10()
}

/// 40 ms of acoustic delay between what we hand the speaker and what the mic hears.
const ECHO_DELAY: usize = 40 * 48;

#[test]
fn accepts_10_and_20_ms_frames_and_keeps_their_length() {
    let mut p = AudioProcessor::new();
    for len in [HALF_FRAME, FRAME_SAMPLES] {
        p.process_render(&vec![0; len]).unwrap();
        assert_eq!(p.process_capture(&vec![0; len]).unwrap().len(), len);
    }
}

#[test]
fn rejects_frames_that_are_not_10_or_20_ms_at_48k() {
    let mut p = AudioProcessor::new();
    // 10 ms at 44.1 kHz and 16 kHz, an empty frame, and 30 ms.
    for len in [441, 160, 0, FRAME_SAMPLES + HALF_FRAME] {
        assert!(p.process_render(&vec![0; len]).is_err(), "render {len}");
        assert!(p.process_capture(&vec![0; len]).is_err(), "capture {len}");
    }
}

#[test]
fn stream_delay_is_configurable() {
    let mut p = AudioProcessor::new();
    p.set_stream_delay_ms(120);
    assert_eq!(p.stream_delay_ms(), 120);
}

#[test]
fn echo_of_the_far_end_is_cancelled_after_convergence() {
    let mut p = AudioProcessor::new();
    p.set_stream_delay_ms(40);
    let mut far = Talker::new(1, 0.5);
    let mut room = EchoPath::new(ECHO_DELAY, 0.5);
    let (mut mic_tail, mut out_tail) = (Vec::new(), Vec::new());
    for i in 0..secs(10) {
        let played = far.frame();
        p.process_render(&played).unwrap();
        let mic = room.frame(&played);
        let out = p.process_capture(&mic).unwrap();
        // Judge the last 4 s, after the canceller has had 6 s to converge.
        if i >= secs(6) {
            mic_tail.push(mic);
            out_tail.push(out);
        }
    }
    let erle = db(energy(&mic_tail) / energy(&out_tail));
    println!("ERLE over the last 4 s: {erle:.1} dB");
    assert!(erle >= 10.0, "echo only reduced by {erle:.1} dB");
}

#[test]
fn near_end_speech_alone_is_preserved() {
    let mut p = AudioProcessor::new();
    let mut near = Talker::new(7, 0.3);
    let (mut mic_tail, mut out_tail) = (Vec::new(), Vec::new());
    for i in 0..secs(6) {
        p.process_render(&vec![0; FRAME_SAMPLES]).unwrap();
        let mic = near.frame();
        let out = p.process_capture(&mic).unwrap();
        if i >= secs(2) {
            mic_tail.push(mic);
            out_tail.push(out);
        }
    }
    let kept = db(energy(&out_tail) / energy(&mic_tail));
    println!("near-end level after processing: {kept:+.1} dB");
    assert!(kept > -6.0, "near-end speech attenuated by {:.1} dB", -kept);
}

#[test]
fn near_end_speech_survives_after_the_far_end_stops() {
    let mut p = AudioProcessor::new();
    let mut far = Talker::new(3, 0.5);
    let mut near = Talker::new(9, 0.3);
    let mut room = EchoPath::new(ECHO_DELAY, 0.5);
    let (mut mic_tail, mut out_tail) = (Vec::new(), Vec::new());
    // 5 s of far end only (the canceller adapts), then 4 s of near end only.
    for i in 0..secs(9) {
        let played = if i < secs(5) {
            far.frame()
        } else {
            vec![0; FRAME_SAMPLES]
        };
        p.process_render(&played).unwrap();
        let echo = room.frame(&played);
        let mic = if i < secs(5) {
            echo
        } else {
            near.frame()
                .iter()
                .zip(&echo)
                .map(|(&a, &b)| a.saturating_add(b))
                .collect()
        };
        let out = p.process_capture(&mic).unwrap();
        if i >= secs(6) {
            mic_tail.push(mic);
            out_tail.push(out);
        }
    }
    let kept = db(energy(&out_tail) / energy(&mic_tail));
    println!("near-end level after the far end stopped: {kept:+.1} dB");
    assert!(kept > -6.0, "near-end speech attenuated by {:.1} dB", -kept);
}

/// A microphone stand-in: hands out whatever the test queued, and ends when nothing is queued.
struct Frames(std::sync::mpsc::Receiver<Frame>);

impl FrameSource for Frames {
    async fn next_frame(&mut self) -> Option<Frame> {
        self.0.try_recv().ok()
    }
}

fn mic() -> (std::sync::mpsc::Sender<Frame>, Frames) {
    let (tx, rx) = std::sync::mpsc::channel();
    (tx, Frames(rx))
}

#[tokio::test]
async fn wired_source_cancels_echo_of_what_the_reference_was_fed() {
    let mut far = Talker::new(5, 0.5);
    let mut room = EchoPath::new(ECHO_DELAY, 0.5);
    let (mic_tx, mic_rx) = mic();
    let (mut source, reference) = echo_cancelled(mic_rx, AudioProcessor::new());
    let (mut mic_tail, mut out_tail) = (Vec::new(), Vec::new());
    for i in 0..secs(10) {
        // The playback side reports what it hands the speaker; the mic hears it later.
        let played = far.frame();
        reference.push(&played);
        let mic = room.frame(&played);
        mic_tx.send(mic.clone()).unwrap();
        let out = source.next_frame().await.expect("a frame per mic frame");
        assert_eq!(out.len(), FRAME_SAMPLES);
        if i >= secs(6) {
            mic_tail.push(mic);
            out_tail.push(out);
        }
    }
    let erle = db(energy(&mic_tail) / energy(&out_tail));
    println!("ERLE through the wired source: {erle:.1} dB");
    assert!(erle >= 10.0, "echo only reduced by {erle:.1} dB");
    // The source ends when its inner source does.
    assert!(source.next_frame().await.is_none());
}

#[tokio::test]
async fn reference_never_blocks_when_nobody_drains_it() {
    let (_mic_tx, mic_rx) = mic();
    let (_source, reference) = echo_cancelled(mic_rx, AudioProcessor::new());
    // Far more than the queue holds: the playback callback must never stall on it.
    for _ in 0..secs(5) {
        reference.push(&vec![0; FRAME_SAMPLES]);
    }
}
