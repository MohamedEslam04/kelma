//! Shared helpers for the audio tests.
#![allow(dead_code)]

/// Power of the `freq` component in `samples` (Goertzel), normalised so a full-scale sine of
/// that frequency gives about `0.25`.
pub fn goertzel_power(samples: &[i16], freq: f64, rate: f64) -> f64 {
    let n = samples.len() as f64;
    let w = 2.0 * std::f64::consts::PI * freq / rate;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in samples {
        let s = f64::from(x) / 32768.0 + coeff * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2) / (n * n)
}

pub fn rms(samples: &[i16]) -> f64 {
    let sum: f64 = samples
        .iter()
        .map(|&x| (f64::from(x) / 32768.0).powi(2))
        .sum();
    (sum / samples.len() as f64).sqrt()
}

/// Frequency in `candidates` with the most power.
pub fn dominant(samples: &[i16], candidates: &[f64], rate: f64) -> f64 {
    candidates
        .iter()
        .copied()
        .max_by(|a, b| {
            goertzel_power(samples, *a, rate).total_cmp(&goertzel_power(samples, *b, rate))
        })
        .unwrap()
}

pub fn sine(freq: f64, rate: f64, amp: f64, samples: usize) -> Vec<i16> {
    (0..samples)
        .map(|i| {
            let t = i as f64 / rate;
            (amp * 32767.0 * (2.0 * std::f64::consts::PI * freq * t).sin()) as i16
        })
        .collect()
}
