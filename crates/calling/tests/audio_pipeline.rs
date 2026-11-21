mod audio_common;
use audio_common::*;
use calling::audio::{
    FrameSink, FrameSource, JitterBuffer, Paced, ToneSource, WavSink, WavSource,
    spawn_capture_encoder, spawn_decoder_playback,
};
use calling::{FRAME_SAMPLES, OpusPacket};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ironteams-audio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn read_wav(path: &std::path::Path) -> (hound::WavSpec, Vec<i16>) {
    let mut r = hound::WavReader::open(path).unwrap();
    let spec = r.spec();
    (spec, r.samples::<i16>().map(Result::unwrap).collect())
}

#[tokio::test]
async fn tone_source_yields_requested_frames_of_the_right_frequency() {
    let mut tone = ToneSource::new(1000.0, 10);
    let mut all = Vec::new();
    let mut n = 0;
    while let Some(f) = tone.next_frame().await {
        assert_eq!(f.len(), FRAME_SAMPLES);
        all.extend(f);
        n += 1;
    }
    assert_eq!(n, 10);
    assert_eq!(dominant(&all, &[500.0, 1000.0, 2000.0], 48_000.0), 1000.0);
}

#[tokio::test]
async fn wav_sink_writes_48k_mono_16bit() {
    let path = tmp("sink.wav");
    let mut sink = WavSink::create(&path).unwrap();
    sink.write_frame(&[7i16; FRAME_SAMPLES]).await.unwrap();
    sink.finish().await.unwrap();
    let (spec, samples) = read_wav(&path);
    assert_eq!(
        (spec.sample_rate, spec.channels, spec.bits_per_sample),
        (48_000, 1, 16)
    );
    assert_eq!(samples, vec![7i16; FRAME_SAMPLES]);
}

#[tokio::test]
async fn wav_source_resamples_and_pads_last_frame() {
    let path = tmp("src44.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44_100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&path, spec).unwrap();
    for s in sine(1000.0, 44_100.0, 0.5, 44_100) {
        w.write_sample(s).unwrap();
    }
    w.finalize().unwrap();
    let mut src = WavSource::open(&path).unwrap();
    let mut frames = Vec::new();
    while let Some(f) = src.next_frame().await {
        assert_eq!(f.len(), FRAME_SAMPLES);
        frames.push(f);
    }
    assert!((50..=51).contains(&frames.len()), "{} frames", frames.len());
    assert_eq!(
        dominant(&frames.concat(), &[800.0, 1000.0, 1200.0], 48_000.0),
        1000.0
    );
}

#[tokio::test]
async fn wav_to_opus_to_wav_end_to_end() {
    let input = tmp("e2e-in.wav");
    let output = tmp("e2e-out.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&input, spec).unwrap();
    for s in sine(1000.0, 48_000.0, 0.5, FRAME_SAMPLES * 50) {
        w.write_sample(s).unwrap();
    }
    w.finalize().unwrap();

    let packets = spawn_capture_encoder(WavSource::open(&input).unwrap());
    let done = spawn_decoder_playback(packets, WavSink::create(&output).unwrap());
    done.await.unwrap().unwrap();

    let (spec, samples) = read_wav(&output);
    assert_eq!((spec.sample_rate, spec.channels), (48_000, 1));
    assert_eq!(samples.len(), FRAME_SAMPLES * 50);
    let steady = &samples[FRAME_SAMPLES * 5..];
    assert_eq!(dominant(steady, &[500.0, 1000.0, 2000.0], 48_000.0), 1000.0);
    assert!(rms(steady) > 0.2);
}

#[tokio::test]
async fn empty_packet_is_concealed_not_dropped() {
    let (tx, rx) = mpsc::channel::<OpusPacket>(8);
    let output = tmp("plc.wav");
    let done = spawn_decoder_playback(rx, WavSink::create(&output).unwrap());
    let mut enc = calling::audio::OpusEncoder::new().unwrap();
    for f in sine(1000.0, 48_000.0, 0.5, FRAME_SAMPLES * 3).chunks(FRAME_SAMPLES) {
        tx.send(enc.encode(f).unwrap()).await.unwrap();
    }
    tx.send(Vec::new()).await.unwrap();
    drop(tx);
    done.await.unwrap().unwrap();
    assert_eq!(read_wav(&output).1.len(), FRAME_SAMPLES * 4);
}

#[tokio::test]
async fn paced_source_emits_one_frame_per_20ms() {
    let mut paced = Paced::new(ToneSource::new(440.0, 5));
    let start = Instant::now();
    let mut n = 0;
    while paced.next_frame().await.is_some() {
        n += 1;
    }
    assert_eq!(n, 5);
    let elapsed = start.elapsed();
    // The first frame is immediate, the other four wait 20 ms each.
    assert!(
        elapsed >= Duration::from_millis(75) && elapsed < Duration::from_millis(400),
        "{elapsed:?}"
    );
}

#[test]
fn jitter_buffer_waits_for_prefill_then_plays() {
    // 1 ch @ 48 kHz, prefill 3 frames, cap 10 frames.
    let mut jb = JitterBuffer::new(FRAME_SAMPLES, 3, 10);
    let mut out = vec![1i16; FRAME_SAMPLES];
    jb.push(&[5i16; FRAME_SAMPLES]);
    jb.push(&[5i16; FRAME_SAMPLES]);
    jb.pop_into(&mut out);
    assert!(out.iter().all(|&s| s == 0), "still prefilling: silence");
    jb.push(&[5i16; FRAME_SAMPLES]);
    jb.pop_into(&mut out);
    assert!(out.iter().all(|&s| s == 5), "prefill reached: audio");
}

#[test]
fn jitter_buffer_underrun_gives_silence_then_reprefills() {
    let mut jb = JitterBuffer::new(FRAME_SAMPLES, 2, 10);
    jb.push(&[5i16; FRAME_SAMPLES * 2]);
    let mut out = vec![0i16; FRAME_SAMPLES * 3];
    jb.pop_into(&mut out);
    assert!(out[..FRAME_SAMPLES * 2].iter().all(|&s| s == 5));
    assert!(
        out[FRAME_SAMPLES * 2..].iter().all(|&s| s == 0),
        "underrun pads with silence"
    );
    assert_eq!(jb.underruns(), 1);
    jb.push(&[9i16; FRAME_SAMPLES]);
    let mut out = vec![1i16; FRAME_SAMPLES];
    jb.pop_into(&mut out);
    assert!(
        out.iter().all(|&s| s == 0),
        "one frame is below prefill after an underrun"
    );
}

#[test]
fn jitter_buffer_drops_oldest_when_over_cap() {
    let mut jb = JitterBuffer::new(FRAME_SAMPLES, 1, 2);
    jb.push(&[1i16; FRAME_SAMPLES]);
    jb.push(&[2i16; FRAME_SAMPLES]);
    jb.push(&[3i16; FRAME_SAMPLES]);
    let mut out = vec![0i16; FRAME_SAMPLES];
    jb.pop_into(&mut out);
    assert!(out.iter().all(|&s| s == 2), "oldest frame dropped");
    assert_eq!(jb.dropped_samples(), FRAME_SAMPLES);
}
