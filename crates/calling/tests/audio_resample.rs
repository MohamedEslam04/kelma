mod audio_common;
use audio_common::*;
use calling::FRAME_SAMPLES;
use calling::audio::InputConverter;

#[test]
fn passthrough_48k_mono_is_exact() {
    let mut conv = InputConverter::new(48_000, 1);
    let pcm = sine(440.0, 48_000.0, 0.5, FRAME_SAMPLES * 2 + 100);
    let frames = conv.push(&pcm);
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0], &pcm[..FRAME_SAMPLES]);
    assert_eq!(frames[1], &pcm[FRAME_SAMPLES..2 * FRAME_SAMPLES]);
}

#[test]
fn resamples_44100_to_960_sample_frames() {
    let mut conv = InputConverter::new(44_100, 1);
    // 441 samples are exactly 10 ms; 20 ms of input is one frame.
    let pcm = sine(1000.0, 44_100.0, 0.5, 441 * 100); // 1 s
    let mut frames = Vec::new();
    for chunk in pcm.chunks(441) {
        frames.extend(conv.push(chunk));
    }
    // 1 s of audio gives 50 frames, give or take the resampler's one-sample latency.
    assert!((49..=50).contains(&frames.len()), "{} frames", frames.len());
    assert!(frames.iter().all(|f| f.len() == FRAME_SAMPLES));
    let flat: Vec<i16> = frames.concat();
    assert_eq!(
        dominant(&flat, &[800.0, 1000.0, 1087.0, 1200.0], 48_000.0),
        1000.0
    );
    let level = rms(&flat);
    assert!((0.3..0.4).contains(&level), "rms {level}");
}

#[test]
fn downsample_96k_keeps_tone() {
    let mut conv = InputConverter::new(96_000, 1);
    let pcm = sine(1000.0, 96_000.0, 0.5, 96_000);
    let flat: Vec<i16> = pcm
        .chunks(960)
        .flat_map(|c| conv.push(c))
        .flatten()
        .collect();
    assert!(flat.len() >= 47 * FRAME_SAMPLES);
    assert_eq!(dominant(&flat, &[800.0, 1000.0, 1200.0], 48_000.0), 1000.0);
}

#[test]
fn stereo_is_downmixed_to_mono() {
    let mut conv = InputConverter::new(48_000, 2);
    // Left = +1000, right = +3000 -> mono 2000.
    let interleaved: Vec<i16> = (0..FRAME_SAMPLES).flat_map(|_| [1000i16, 3000]).collect();
    let frames = conv.push(&interleaved);
    assert_eq!(frames.len(), 1);
    assert!(frames[0].iter().all(|&s| s == 2000));
}

#[test]
fn odd_chunk_sizes_do_not_lose_samples() {
    let mut conv = InputConverter::new(48_000, 1);
    let pcm = sine(300.0, 48_000.0, 0.5, FRAME_SAMPLES * 4);
    let frames: Vec<Vec<i16>> = pcm.chunks(333).flat_map(|c| conv.push(c)).collect();
    assert_eq!(frames.concat(), pcm);
}
