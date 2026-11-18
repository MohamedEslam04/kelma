use std::collections::VecDeque;

/// Playout buffer between the decoder and an output device callback.
///
/// Playback starts only once `prefill` frames are queued, so small arrival jitter does not
/// cause stutter. When the device asks for more than is queued, the shortfall is filled with
/// silence and the buffer re-primes (an *underrun*). Above `max` frames the oldest audio is
/// dropped, which bounds latency when the network delivers a burst.
///
/// Units are interleaved samples of the *device* format; `frame_len` is one 20 ms frame of it.
pub struct JitterBuffer {
    queue: VecDeque<i16>,
    frame_len: usize,
    prefill: usize,
    max: usize,
    playing: bool,
    underruns: usize,
    dropped: usize,
}

impl JitterBuffer {
    pub fn new(frame_len: usize, prefill_frames: usize, max_frames: usize) -> Self {
        Self {
            queue: VecDeque::new(),
            frame_len,
            prefill: prefill_frames * frame_len,
            max: max_frames.max(prefill_frames) * frame_len,
            playing: false,
            underruns: 0,
            dropped: 0,
        }
    }

    pub fn push(&mut self, samples: &[i16]) {
        self.queue.extend(samples);
        if self.queue.len() > self.max {
            let excess = self.queue.len() - self.max;
            self.queue.drain(..excess);
            self.dropped += excess;
        }
    }

    /// Fill `out` from the queue, padding with silence while priming or on underrun.
    pub fn pop_into(&mut self, out: &mut [i16]) {
        if !self.playing && self.queue.len() >= self.prefill.max(1) {
            self.playing = true;
        }
        if !self.playing {
            out.fill(0);
            return;
        }
        let n = out.len().min(self.queue.len());
        for (o, s) in out.iter_mut().zip(self.queue.drain(..n)) {
            *o = s;
        }
        if n < out.len() {
            out[n..].fill(0);
            self.playing = false;
            self.underruns += 1;
        }
    }

    pub fn buffered(&self) -> usize {
        self.queue.len()
    }

    pub fn underruns(&self) -> usize {
        self.underruns
    }

    pub fn dropped_samples(&self) -> usize {
        self.dropped
    }

    #[allow(dead_code)]
    pub(crate) fn frame_len(&self) -> usize {
        self.frame_len
    }
}
