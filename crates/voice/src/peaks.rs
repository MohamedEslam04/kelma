//! Waveform bars for a whole voice message, and the loudness scale shared with the
//! recorder's live bars so a message looks the same while recording and afterwards.

use crate::Clip;

/// Loudness mapped to 0.0 (at or below this, in dB relative to full scale) ...
const FLOOR_DB: f32 = -55.0;
/// ... and 1.0 (at or above this). Speech sits around -35..-15 dBFS, so it fills most of the
/// range while room noise stays flat.
const CEILING_DB: f32 = -10.0;

/// Map an RMS level (0.0..=1.0 of full scale) to a bar height in 0.0..=1.0 on a dB scale.
pub(crate) fn bar_from_rms(rms: f32) -> f32 {
    if rms <= 0.0 {
        return 0.0;
    }
    let decibels = 20.0 * rms.log10();
    ((decibels - FLOOR_DB) / (CEILING_DB - FLOOR_DB)).clamp(0.0, 1.0)
}

/// Root mean square of `samples` (0.0 for none). Summed in a few independent lanes so the
/// compiler can vectorize it: a 15 minute clip must reduce in well under 100 ms.
pub(crate) fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut lanes = [0.0f32; 8];
    let chunks = samples.chunks_exact(lanes.len());
    let rest = chunks.remainder();
    for chunk in chunks {
        for (lane, &sample) in lanes.iter_mut().zip(chunk) {
            *lane += sample * sample;
        }
    }
    let sum: f64 = lanes.iter().map(|&lane| f64::from(lane)).sum::<f64>()
        + rest.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>();
    (sum / samples.len() as f64).sqrt() as f32
}

/// `bars` bar heights (0.0..=1.0) spread evenly over the whole clip, for drawing the waveform
/// of a message. A clip shorter than `bars` samples gives flat bars where there is no audio.
pub fn peaks(clip: &Clip, bars: usize) -> Vec<f32> {
    let total = clip.samples.len();
    (0..bars)
        .map(|bar| {
            let start = bar * total / bars;
            let end = (bar + 1) * total / bars;
            bar_from_rms(rms(&clip.samples[start..end]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_sine;
    use std::time::Instant;

    #[test]
    fn has_one_bar_per_request() {
        let clip = Clip::new(test_sine(220.0, 2.0, 48_000), 48_000);
        assert_eq!(peaks(&clip, 64).len(), 64);
        assert!(peaks(&clip, 0).is_empty());
    }

    #[test]
    fn bars_stay_between_zero_and_one() {
        let loud = Clip::new(vec![1.0; 48_000], 48_000);
        let quiet = Clip::new(vec![1e-6; 48_000], 48_000);
        for clip in [&loud, &quiet] {
            assert!(peaks(clip, 10).iter().all(|b| (0.0..=1.0).contains(b)));
        }
        assert!(peaks(&loud, 10).iter().all(|&b| b == 1.0));
    }

    #[test]
    fn silence_is_flat() {
        let clip = Clip::new(vec![0.0; 48_000], 48_000);
        assert!(peaks(&clip, 32).iter().all(|&b| b == 0.0));
    }

    #[test]
    fn empty_and_tiny_clips_do_not_panic() {
        assert_eq!(peaks(&Clip::default(), 5), vec![0.0; 5]);
        let tiny = Clip::new(vec![0.5, 0.5], 48_000);
        assert_eq!(peaks(&tiny, 8).len(), 8);
    }

    #[test]
    fn louder_audio_gives_taller_bars_and_speech_level_is_lively() {
        let mut samples = vec![0.0; 48_000];
        // Second half is a tone at -20 dBFS RMS (peak 0.14): normal speech.
        samples.extend(test_sine(300.0, 1.0, 48_000).iter().map(|s| s * 0.28));
        let bars = peaks(&Clip::new(samples, 48_000), 2);
        assert_eq!(bars[0], 0.0);
        assert!(bars[1] > 0.6, "speech level looks lively, got {}", bars[1]);
    }

    #[test]
    fn halving_the_amplitude_lowers_the_bar() {
        assert!(bar_from_rms(0.05) < bar_from_rms(0.1));
        assert!(bar_from_rms(0.1) < bar_from_rms(0.2));
    }

    #[test]
    fn a_fifteen_minute_clip_reduces_quickly() {
        let clip = Clip::new(vec![0.1; 15 * 60 * 48_000], 48_000);
        let started = Instant::now();
        let bars = peaks(&clip, 100);
        let took = started.elapsed();
        assert_eq!(bars.len(), 100);
        // The target is under 100 ms optimized; an unoptimized test build is far slower.
        let limit = if cfg!(debug_assertions) { 3000 } else { 100 };
        assert!(took.as_millis() < limit, "took {took:?}");
    }
}
