//! Pitch-keeping speed change (WSOLA, waveform-similarity overlap-add) for faster playback of
//! speech. Plain resampling would raise the pitch at 1.5x or 2x; this keeps it.

use std::collections::VecDeque;

/// Length of one analysis frame. 25 ms holds a few pitch periods of a voice.
const FRAME_SECONDS: f64 = 0.025;
/// How far from its nominal place a frame may be taken to line up with the audio before it.
const SEARCH_SECONDS: f64 = 0.010;
/// Shortest frame, so a zero or tiny sample rate still works.
const MIN_FRAME: usize = 64;

/// Changes the speed of a clip as it is played, without changing its pitch.
///
/// Frames of [`FRAME_SECONDS`] are taken from the input every `hop × speed` samples and laid
/// down every `hop` samples (half a frame) with a Hann window, which sums to one at that
/// overlap. Each frame is taken within [`SEARCH_SECONDS`] of its nominal place where it best
/// continues the previous frame, so the overlapping waveforms line up and the pitch stays. At
/// 1× the frames continue each other exactly, which passes the audio through unchanged.
///
/// The input is the whole clip, passed again on every call; the stretcher keeps only its place.
pub struct Stretcher {
    speed: f32,
    hop: usize,
    tolerance: isize,
    window: Vec<f32>,
    /// Input position of the next output sample: it advances by `speed` per sample handed out.
    position: f64,
    /// Nominal input position of the next frame.
    anchor: f64,
    /// Start of the last frame taken; `None` at the start or after a seek.
    previous: Option<isize>,
    /// Second half of the last frame, windowed, waiting for the next frame's first half.
    tail: Vec<f32>,
    /// Output made and not handed out yet.
    ready: VecDeque<f32>,
}

impl Stretcher {
    /// A stretcher at the start of a clip of `sample_rate`, playing at `speed`.
    pub fn new(sample_rate: u32, speed: f32) -> Self {
        let frame = (((f64::from(sample_rate) * FRAME_SECONDS) as usize) & !1).max(MIN_FRAME);
        let window = (0..frame)
            .map(|n| {
                (0.5 - 0.5 * (std::f64::consts::TAU * n as f64 / frame as f64).cos()) as f32
            })
            .collect();
        Self {
            speed,
            hop: frame / 2,
            tolerance: (f64::from(sample_rate) * SEARCH_SECONDS) as isize,
            window,
            position: 0.0,
            anchor: 0.0,
            previous: None,
            tail: Vec::new(),
            ready: VecDeque::new(),
        }
    }

    /// Plays on at `speed`; the position does not jump.
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed;
    }

    /// Continues from input sample `position`.
    pub fn seek(&mut self, position: usize) {
        self.position = position as f64;
        self.anchor = self.position;
        self.previous = None;
        self.tail.clear();
        self.ready.clear();
    }

    /// Fills `out` from `input` (the whole clip) and returns how many samples it wrote: fewer
    /// than `out.len()` only once the clip is over, then 0 on every later call.
    pub fn render(&mut self, input: &[f32], out: &mut [f32]) -> usize {
        let end = input.len() as f64;
        let mut written = 0;
        while written < out.len() && self.position < end {
            if self.ready.is_empty() {
                self.step(input);
            }
            out[written] = self.ready.pop_front().unwrap_or(0.0);
            self.position += f64::from(self.speed);
            written += 1;
        }
        written
    }

    /// Input position of the next output sample.
    pub fn position(&self) -> f64 {
        self.position
    }

    /// Lays down one frame: `hop` more samples of output.
    fn step(&mut self, input: &[f32]) {
        let hop = self.hop;
        let at = |i: isize| {
            usize::try_from(i)
                .ok()
                .and_then(|i| input.get(i))
                .copied()
                .unwrap_or(0.0)
        };
        let start = match self.previous {
            // The first frame starts at full volume: no fade in after a start or a seek.
            None => {
                let start = self.anchor.round() as isize;
                self.ready.extend((0..hop).map(|j| at(start + j as isize)));
                start
            }
            Some(previous) => {
                let natural = previous + hop as isize;
                let start = if (self.speed - 1.0).abs() < 1e-6 {
                    // At 1× the natural continuation is the audio itself.
                    self.anchor = natural as f64;
                    natural
                } else {
                    self.best_start(&at, self.anchor.round() as isize, natural)
                };
                for j in 0..hop {
                    let sample = self.tail[j] + at(start + j as isize) * self.window[j];
                    self.ready.push_back(sample);
                }
                start
            }
        };
        self.tail.clear();
        self.tail.extend(
            (0..hop).map(|j| at(start + (hop + j) as isize) * self.window[hop + j]),
        );
        self.previous = Some(start);
        self.anchor += hop as f64 * f64::from(self.speed);
    }

    /// The frame start within the tolerance of `nominal` whose first half looks most like the
    /// audio that would naturally follow the previous frame (from `natural`). Every second
    /// sample is compared, which is plenty for speech and halves the work.
    fn best_start(&self, at: &impl Fn(isize) -> f32, nominal: isize, natural: isize) -> isize {
        let (mut best, mut best_score) = (nominal, f32::NEG_INFINITY);
        for candidate in nominal - self.tolerance..=nominal + self.tolerance {
            let (mut dot, mut energy) = (0.0f32, 1e-9f32);
            for j in (0..self.hop as isize).step_by(2) {
                let x = at(candidate + j);
                dot += x * at(natural + j);
                energy += x * x;
            }
            let score = dot / energy.sqrt();
            if score > best_score {
                best = candidate;
                best_score = score;
            }
        }
        best
    }
}

/// `input` played at `speed`, all at once (the pieces [`Stretcher::render`] gives, joined).
pub fn stretch(input: &[f32], sample_rate: u32, speed: f32) -> Vec<f32> {
    let mut stretcher = Stretcher::new(sample_rate, speed);
    let mut out = Vec::with_capacity((input.len() as f64 / f64::from(speed.max(0.01))) as usize);
    let mut buffer = vec![0.0f32; 4096];
    loop {
        let n = stretcher.render(input, &mut buffer);
        out.extend_from_slice(&buffer[..n]);
        if n < buffer.len() {
            return out;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{test_frequency, test_sine};

    const RATE: u32 = 48_000;

    #[test]
    fn faster_playback_is_shorter_and_keeps_the_pitch() {
        let input = test_sine(440.0, 3.0, RATE);
        for speed in [1.5, 2.0] {
            let out = stretch(&input, RATE, speed);
            let expected = input.len() as f32 / speed;
            assert!(
                (out.len() as f32 - expected).abs() < expected * 0.03,
                "speed {speed}: {} samples, expected about {expected}",
                out.len()
            );
            let body = &out[RATE as usize / 4..out.len() - RATE as usize / 4];
            let hz = test_frequency(body, RATE);
            assert!((hz - 440.0).abs() < 3.0, "speed {speed}: {hz} Hz");
        }
    }

    #[test]
    fn slower_playback_is_longer_and_keeps_the_pitch() {
        let input = test_sine(440.0, 2.0, RATE);
        let out = stretch(&input, RATE, 0.75);
        let expected = input.len() as f32 / 0.75;
        assert!((out.len() as f32 - expected).abs() < expected * 0.03);
        let hz = test_frequency(&out[RATE as usize / 4..out.len() - RATE as usize / 4], RATE);
        assert!((hz - 440.0).abs() < 3.0, "{hz} Hz");
    }

    #[test]
    fn normal_speed_passes_the_audio_through_unchanged() {
        let input = test_sine(330.0, 1.0, RATE);
        let out = stretch(&input, RATE, 1.0);
        assert!(out.len().abs_diff(input.len()) < 8, "{}", out.len());
        let worst = input
            .iter()
            .zip(&out)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-3, "worst difference {worst}");
    }

    #[test]
    fn output_is_the_same_whatever_the_buffer_size() {
        let input = test_sine(440.0, 1.0, RATE);
        let whole = stretch(&input, RATE, 1.5);
        let mut stretcher = Stretcher::new(RATE, 1.5);
        let mut pieced = Vec::new();
        let mut buffer = [0.0f32; 333];
        loop {
            let n = stretcher.render(&input, &mut buffer);
            pieced.extend_from_slice(&buffer[..n]);
            if n < buffer.len() {
                break;
            }
        }
        assert_eq!(whole, pieced);
    }

    #[test]
    fn position_advances_by_speed_per_output_sample() {
        let input = test_sine(440.0, 5.0, RATE);
        let mut stretcher = Stretcher::new(RATE, 1.5);
        let mut buffer = vec![0.0; 2 * RATE as usize / 10];
        stretcher.render(&input, &mut buffer);
        stretcher.render(&input, &mut buffer);
        let expected = 2.0 * buffer.len() as f64 * 1.5;
        assert!((stretcher.position() - expected).abs() < 1.0);
    }

    #[test]
    fn changing_speed_keeps_position_continuous() {
        let input = test_sine(440.0, 5.0, RATE);
        let mut stretcher = Stretcher::new(RATE, 1.0);
        let mut buffer = vec![0.0; RATE as usize / 5];
        stretcher.render(&input, &mut buffer);
        let before = stretcher.position();
        stretcher.set_speed(2.0);
        assert!((stretcher.position() - before).abs() < 1.0);
        stretcher.render(&input, &mut buffer);
        assert!((stretcher.position() - (before + buffer.len() as f64 * 2.0)).abs() < 1.0);
    }

    #[test]
    fn seek_continues_from_the_new_position() {
        let input: Vec<f32> = (0..48_000).map(|n| n as f32 / 48_000.0).collect();
        let mut stretcher = Stretcher::new(RATE, 1.0);
        stretcher.seek(20_000);
        assert_eq!(stretcher.position(), 20_000.0);
        let mut buffer = [0.0f32; 100];
        assert_eq!(stretcher.render(&input, &mut buffer), 100);
        for (n, sample) in buffer.iter().enumerate() {
            assert!((sample - input[20_000 + n]).abs() < 1e-4, "sample {n}");
        }
    }

    #[test]
    fn running_out_of_audio_returns_a_short_count() {
        let input = test_sine(440.0, 0.5, RATE);
        let mut stretcher = Stretcher::new(RATE, 2.0);
        let mut buffer = vec![0.0; RATE as usize];
        let produced = stretcher.render(&input, &mut buffer);
        assert!(produced < buffer.len());
        assert_eq!(stretcher.render(&input, &mut buffer), 0);
    }

    #[test]
    fn empty_input_produces_nothing() {
        assert!(stretch(&[], RATE, 1.5).is_empty());
    }
}
