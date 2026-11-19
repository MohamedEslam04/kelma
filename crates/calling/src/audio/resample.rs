use crate::{FRAME_SAMPLES, SAMPLE_RATE};

/// Streaming linear-interpolation resampler for mono audio. Plenty for speech; no anti-alias
/// filter, so inputs far above 48 kHz alias slightly (inaudible for voice content).
struct Resampler {
    in_rate: u64,
    out_rate: u64,
    /// Input not yet fully consumed.
    buf: Vec<i16>,
    /// Position of the next output sample in `buf`, in units of `1 / out_rate` input samples.
    pos: u64,
}

impl Resampler {
    fn new(in_rate: u32, out_rate: u32) -> Self {
        Self {
            in_rate: in_rate.into(),
            out_rate: out_rate.into(),
            buf: Vec::new(),
            pos: 0,
        }
    }

    fn process(&mut self, input: &[i16], out: &mut Vec<i16>) {
        self.buf.extend_from_slice(input);
        loop {
            let idx = (self.pos / self.out_rate) as usize;
            if idx + 1 >= self.buf.len() {
                break;
            }
            let frac = (self.pos % self.out_rate) as i64;
            let (a, b) = (i64::from(self.buf[idx]), i64::from(self.buf[idx + 1]));
            out.push((a + (b - a) * frac / self.out_rate as i64) as i16);
            self.pos += self.in_rate;
        }
        let consumed = (self.pos / self.out_rate) as usize;
        self.buf.drain(..consumed.min(self.buf.len()));
        self.pos -= consumed as u64 * self.out_rate;
    }
}

/// Turns whatever a capture device delivers (any rate, interleaved channels) into 20 ms,
/// 48 kHz mono frames: downmix by averaging, resample, regroup.
pub struct InputConverter {
    channels: usize,
    resampler: Option<Resampler>,
    pending: Vec<i16>,
}

impl InputConverter {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        let resampler =
            (sample_rate != SAMPLE_RATE).then(|| Resampler::new(sample_rate, SAMPLE_RATE));
        Self {
            channels: usize::from(channels.max(1)),
            resampler,
            pending: Vec::new(),
        }
    }

    /// Feed interleaved samples; returns every complete 20 ms frame now available.
    pub fn push(&mut self, interleaved: &[i16]) -> Vec<Vec<i16>> {
        let mono: Vec<i16> = if self.channels == 1 {
            interleaved.to_vec()
        } else {
            interleaved
                .chunks_exact(self.channels)
                .map(|c| {
                    (c.iter().map(|&s| i32::from(s)).sum::<i32>() / self.channels as i32) as i16
                })
                .collect()
        };
        match &mut self.resampler {
            Some(r) => r.process(&mono, &mut self.pending),
            None => self.pending.extend_from_slice(&mono),
        }
        let full = self.pending.len() / FRAME_SAMPLES * FRAME_SAMPLES;
        let ready: Vec<i16> = self.pending.drain(..full).collect();
        ready.chunks(FRAME_SAMPLES).map(<[i16]>::to_vec).collect()
    }

    /// The remaining partial frame padded with silence, if there is one (end of a file).
    pub fn flush(&mut self) -> Option<Vec<i16>> {
        if self.pending.is_empty() {
            return None;
        }
        let mut frame = std::mem::take(&mut self.pending);
        frame.resize(FRAME_SAMPLES, 0);
        Some(frame)
    }
}

/// Interleave a mono 48 kHz frame for an output device running at `rate` with `channels`
/// channels (used by playback; the inverse direction of [`InputConverter`]).
#[cfg_attr(not(feature = "devices"), allow(dead_code))]
pub(crate) struct OutputConverter {
    channels: usize,
    resampler: Option<Resampler>,
}

#[cfg_attr(not(feature = "devices"), allow(dead_code))]
impl OutputConverter {
    pub(crate) fn new(sample_rate: u32, channels: u16) -> Self {
        let resampler =
            (sample_rate != SAMPLE_RATE).then(|| Resampler::new(SAMPLE_RATE, sample_rate));
        Self {
            channels: usize::from(channels.max(1)),
            resampler,
        }
    }

    pub(crate) fn convert(&mut self, mono_48k: &[i16]) -> Vec<i16> {
        let mono: Vec<i16> = match &mut self.resampler {
            Some(r) => {
                let mut out = Vec::with_capacity(mono_48k.len());
                r.process(mono_48k, &mut out);
                out
            }
            None => mono_48k.to_vec(),
        };
        mono.iter()
            .flat_map(|&s| std::iter::repeat_n(s, self.channels))
            .collect()
    }
}
