//! Reading a voice message back: AAC in MP4 (Teams voice messages) or MP3 (Teams voicemail), to
//! mono f32 with symphonia, in pure Rust, so playing needs no system library.

use crate::{Clip, Error, Result};
use std::io::Cursor;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::TrackType;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Decodes a whole recording to mono (channels are averaged) at its own sample rate. A packet
/// that does not decode is skipped, as players do; a file with no audio track is an error.
pub fn decode(bytes: &[u8]) -> Result<Clip> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(
            &Hint::new(),
            source,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| Error::Decode(e.to_string()))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| Error::Decode("no audio track".into()))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| Error::Decode("the audio track has no codec parameters".into()))?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| Error::Decode(e.to_string()))?;

    let (mut mono, mut rate) = (Vec::new(), 0u32);
    let mut interleaved: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            // A truncated file still plays what arrived.
            Err(SymphoniaError::IoError(_)) => break,
            Err(e) => return Err(Error::Decode(e.to_string())),
        };
        if packet.track_id != track_id {
            continue;
        }
        let buffer = match decoder.decode(&packet) {
            Ok(buffer) => buffer,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(Error::Decode(e.to_string())),
        };
        let channels = buffer.spec().channels().count().max(1);
        rate = buffer.spec().rate();
        interleaved.resize(buffer.samples_interleaved(), 0.0);
        buffer.copy_to_slice_interleaved(&mut interleaved);
        mix_down(&interleaved, channels, &mut mono);
    }
    if rate == 0 {
        return Err(Error::Decode("no audio decoded".into()));
    }
    Ok(Clip::new(mono, rate))
}

/// Appends the average of each frame's `channels` samples to `out`.
fn mix_down(interleaved: &[f32], channels: usize, out: &mut Vec<f32>) {
    if channels == 1 {
        out.extend_from_slice(interleaved);
        return;
    }
    let scale = 1.0 / channels as f32;
    out.extend(
        interleaved
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() * scale),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{encode_m4a, test_frequency, test_sine};

    #[test]
    fn stereo_frames_are_averaged() {
        let mut out = Vec::new();
        mix_down(&[1.0, 0.0, 0.5, 0.5, -1.0, 1.0], 2, &mut out);
        assert_eq!(out, [0.5, 0.5, 0.0]);
        mix_down(&[0.25], 1, &mut out);
        assert_eq!(out.last(), Some(&0.25));
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(matches!(decode(b"not audio at all"), Err(Error::Decode(_))));
        assert!(matches!(decode(&[]), Err(Error::Decode(_))));
    }

    /// Encode with ffmpeg, decode with symphonia: the length and the tone survive.
    #[test]
    fn an_encoded_clip_decodes_to_the_same_length_and_tone() {
        let clip = Clip::new(test_sine(440.0, 2.0, 48_000), 48_000);
        let m4a = match encode_m4a(&clip) {
            Ok(m4a) => m4a,
            Err(Error::EncoderMissing) => {
                eprintln!("ffmpeg is not installed: round trip skipped");
                return;
            }
            Err(e) => panic!("{e}"),
        };
        let back = decode(&m4a).unwrap();
        assert_eq!(back.sample_rate, 44_100);
        let difference = back.duration().as_secs_f64() - clip.duration().as_secs_f64();
        // AAC adds a little priming and padding at the edges.
        assert!(difference.abs() < 0.05, "length differs by {difference} s");
        let body = &back.samples[4410..back.samples.len() - 4410];
        let hz = test_frequency(body, back.sample_rate);
        assert!((hz - 440.0).abs() < 3.0, "{hz} Hz");
    }
}
