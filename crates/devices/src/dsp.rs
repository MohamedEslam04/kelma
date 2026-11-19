//! Sample-level helpers with no device in sight: the test tone, the level meter and the
//! channel/rate conversion the loopback uses. Everything here is tested offline.

/// One reading for a level meter, both on a 0.0..=1.0 scale of full scale.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MicLevel {
    pub rms: f32,
    pub peak: f32,
}

impl MicLevel {
    /// RMS in dB relative to full scale (floored at -100 dB for silence).
    pub fn rms_dbfs(&self) -> f32 {
        if self.rms <= 1e-5 {
            -100.0
        } else {
            20.0 * self.rms.log10()
        }
    }
}

/// A short sine tone, interleaved over `channels`, with a 10 ms fade in and out so it does
/// not click. `amplitude` is 0.0..=1.0.
pub fn sine_tone(
    freq_hz: f32,
    amplitude: f32,
    millis: u32,
    sample_rate: u32,
    channels: u16,
) -> Vec<f32> {
    let frames = (u64::from(sample_rate) * u64::from(millis) / 1000) as usize;
    let fade = (sample_rate as usize / 100).min(frames / 2).max(1);
    let mut out = Vec::with_capacity(frames * usize::from(channels));
    for n in 0..frames {
        let t = n as f32 / sample_rate as f32;
        let env = (n.min(frames - 1 - n) as f32 / fade as f32).min(1.0);
        let s = (std::f32::consts::TAU * freq_hz * t).sin() * amplitude * env;
        out.extend(std::iter::repeat_n(s, usize::from(channels)));
    }
    out
}

/// Turns a stream of interleaved samples into one [`MicLevel`] per window of audio.
pub struct LevelMeter {
    channels: usize,
    window_frames: usize,
    frames: usize,
    sum_sq: f64,
    peak: f32,
}

impl LevelMeter {
    /// A reading is emitted every `window_millis` of audio (50 ms is smooth for a meter).
    pub fn new(sample_rate: u32, channels: u16, window_millis: u32) -> Self {
        Self {
            channels: usize::from(channels.max(1)),
            window_frames: (sample_rate as usize * window_millis as usize / 1000).max(1),
            frames: 0,
            sum_sq: 0.0,
            peak: 0.0,
        }
    }

    /// Feed interleaved samples; returns a reading for every window completed.
    /// Callbacks hand over whole frames; a trailing partial frame is ignored.
    pub fn push(&mut self, samples: &[f32]) -> Vec<MicLevel> {
        let mut out = Vec::new();
        for frame in samples.chunks_exact(self.channels) {
            // Level of the frame is the mean of its channels' power; peak is the loudest.
            for &s in frame {
                self.sum_sq += f64::from(s) * f64::from(s) / self.channels as f64;
                self.peak = self.peak.max(s.abs());
            }
            self.frames += 1;
            if self.frames == self.window_frames {
                out.push(MicLevel {
                    rms: (self.sum_sq / self.frames as f64).sqrt() as f32,
                    peak: self.peak.min(1.0),
                });
                self.frames = 0;
                self.sum_sq = 0.0;
                self.peak = 0.0;
            }
        }
        out
    }
}

/// Converts interleaved audio from one channel count and rate to another (mono mix, then
/// fan out; linear resampling) for the mic-to-speaker loopback. Keeps state across calls.
pub struct Remix {
    in_channels: usize,
    out_channels: usize,
    /// Input frames advanced per output frame.
    step: f64,
    /// Position of the next output frame between the previous input frame (0.0) and the
    /// current one (1.0).
    pos: f64,
    last: f32,
}

impl Remix {
    pub fn new(in_rate: u32, in_channels: u16, out_rate: u32, out_channels: u16) -> Self {
        Self {
            in_channels: usize::from(in_channels.max(1)),
            out_channels: usize::from(out_channels.max(1)),
            step: f64::from(in_rate) / f64::from(out_rate),
            // The first output frame is the first input frame: no added delay at equal rates.
            pos: 1.0,
            last: 0.0,
        }
    }

    pub fn convert(&mut self, input: &[f32]) -> Vec<f32> {
        let mut out = Vec::new();
        for frame in input.chunks_exact(self.in_channels) {
            let cur = frame.iter().sum::<f32>() / self.in_channels as f32;
            while self.pos <= 1.0 {
                let s = self.last + (cur - self.last) * self.pos as f32;
                out.extend(std::iter::repeat_n(s, self.out_channels));
                self.pos += self.step;
            }
            self.pos -= 1.0;
            self.last = cur;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_has_the_requested_length_and_channels() {
        let t = sine_tone(440.0, 0.5, 100, 48_000, 2);
        assert_eq!(t.len(), 4800 * 2);
        // Both channels carry the same sample.
        assert!(t.as_chunks::<2>().0.iter().all(|f| f[0] == f[1]));
    }

    #[test]
    fn tone_is_bounded_by_amplitude_and_fades_at_the_edges() {
        let t = sine_tone(440.0, 0.5, 200, 48_000, 1);
        assert!(t.iter().all(|s| s.abs() <= 0.5 + 1e-6));
        assert!(t.iter().any(|s| s.abs() > 0.45), "reaches near amplitude");
        assert_eq!(t[0], 0.0);
        assert!(t[1].abs() < 0.01, "starts faded in, not at full level");
        assert!(t[t.len() - 1].abs() < 0.01, "ends faded out");
    }

    #[test]
    fn tone_of_zero_length_is_empty() {
        assert!(sine_tone(440.0, 0.5, 0, 48_000, 2).is_empty());
    }

    #[test]
    fn meter_reports_silence() {
        let mut m = LevelMeter::new(1000, 1, 50);
        let r = m.push(&[0.0; 50]);
        assert_eq!(
            r,
            vec![MicLevel {
                rms: 0.0,
                peak: 0.0
            }]
        );
        assert_eq!(r[0].rms_dbfs(), -100.0);
    }

    #[test]
    fn meter_rms_and_peak_of_a_square_wave() {
        let mut m = LevelMeter::new(1000, 1, 50);
        let samples: Vec<f32> = (0..50)
            .map(|i| if i % 2 == 0 { 0.5 } else { -0.5 })
            .collect();
        let r = m.push(&samples);
        assert_eq!(r.len(), 1);
        assert!((r[0].rms - 0.5).abs() < 1e-6);
        assert_eq!(r[0].peak, 0.5);
        assert!((r[0].rms_dbfs() - -6.0206).abs() < 1e-3);
    }

    #[test]
    fn meter_emits_one_reading_per_window_across_calls() {
        let mut m = LevelMeter::new(1000, 2, 50);
        // 50 frames per window, 2 channels; feed 3 windows in uneven whole-frame pieces.
        let all = vec![0.25f32; 50 * 2 * 3];
        let mut got = 0;
        for piece in all.chunks(78) {
            got += m.push(piece).len();
        }
        assert_eq!(got, 3);
    }

    #[test]
    fn meter_peak_resets_each_window() {
        let mut m = LevelMeter::new(1000, 1, 10);
        let mut s = vec![0.0f32; 20];
        s[3] = 0.9;
        let r = m.push(&s);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].peak, 0.9);
        assert_eq!(r[1].peak, 0.0);
    }

    #[test]
    fn remix_mono_to_stereo_same_rate_duplicates() {
        let mut r = Remix::new(48_000, 1, 48_000, 2);
        let out = r.convert(&[0.1, 0.2, 0.3]);
        assert_eq!(out, vec![0.1, 0.1, 0.2, 0.2, 0.3, 0.3]);
    }

    #[test]
    fn remix_stereo_to_mono_averages() {
        let mut r = Remix::new(48_000, 2, 48_000, 1);
        assert_eq!(r.convert(&[0.2, 0.4, 1.0, 0.0]), vec![0.3, 0.5]);
    }

    #[test]
    fn remix_resamples_to_the_output_rate() {
        // 24 kHz -> 48 kHz doubles the frames; across calls the total stays proportional.
        let mut r = Remix::new(24_000, 1, 48_000, 1);
        let mut n = r.convert(&[0.0; 100]).len();
        n += r.convert(&[0.0; 100]).len();
        assert!((n as i64 - 400).abs() <= 2, "got {n}");
        // 48 kHz -> 24 kHz halves them.
        let mut r = Remix::new(48_000, 1, 24_000, 1);
        let n = r.convert(&[0.0; 200]).len();
        assert!((n as i64 - 100).abs() <= 1, "got {n}");
    }

    #[test]
    fn remix_interpolates_a_ramp() {
        let mut r = Remix::new(1000, 1, 2000, 1);
        let out = r.convert(&[0.0, 1.0, 2.0]);
        // Output lies on the input ramp (delayed by one input frame).
        for w in out.windows(2) {
            assert!(w[1] >= w[0], "monotonic: {out:?}");
        }
        assert!(out.iter().all(|s| (0.0..=2.0).contains(s)));
    }
}
