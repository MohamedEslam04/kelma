//! Real microphone/speaker I/O through cpal (ALSA on Linux; PipeWire serves ALSA clients).

use super::resample::OutputConverter;
use super::{Frame, FrameSink, FrameSource, InputConverter, JitterBuffer};
use crate::{Error, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// Frames queued between the capture callback and the consumer: one second.
const CAPTURE_DEPTH: usize = 50;
/// Playback starts once this many 20 ms frames are queued.
const PLAYOUT_PREFILL_FRAMES: usize = 3;
/// Above this many queued frames the oldest audio is dropped.
const PLAYOUT_MAX_FRAMES: usize = 10;

fn dev_err(what: &str, e: impl std::fmt::Display) -> Error {
    Error::Audio(format!("{what}: {e}"))
}

/// An audio device as listed to the user. `id` is stable and accepted wherever a device name
/// is, as is `name` or any case-insensitive part of either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

fn device_id(d: &Device) -> String {
    d.id().map(|id| id.to_string()).unwrap_or_default()
}

fn device_name(d: &Device) -> String {
    d.description()
        .map(|desc| desc.name().to_string())
        .unwrap_or_else(|_| device_id(d))
}

fn list(input: bool) -> Result<Vec<DeviceInfo>> {
    let host = cpal::default_host();
    let default = if input {
        host.default_input_device()
    } else {
        host.default_output_device()
    };
    let default_id = default.as_ref().map(device_id);
    let devices = host.devices().map_err(|e| dev_err("list devices", e))?;
    Ok(devices
        .filter(|d| {
            if input {
                d.supports_input()
            } else {
                d.supports_output()
            }
        })
        .map(|d| {
            let id = device_id(&d);
            DeviceInfo {
                is_default: default_id.as_deref() == Some(id.as_str()),
                name: device_name(&d),
                id,
            }
        })
        .collect())
}

pub fn list_input_devices() -> Result<Vec<DeviceInfo>> {
    list(true)
}

pub fn list_output_devices() -> Result<Vec<DeviceInfo>> {
    list(false)
}

/// The default device, or the one whose id/name equals `wanted` (exact match first, then a
/// case-insensitive substring).
fn pick_device(wanted: Option<&str>, input: bool) -> Result<Device> {
    let host = cpal::default_host();
    let kind = if input { "input" } else { "output" };
    let Some(wanted) = wanted else {
        let d = if input {
            host.default_input_device()
        } else {
            host.default_output_device()
        };
        return d.ok_or_else(|| Error::Audio(format!("no default {kind} device")));
    };
    let candidates: Vec<(String, String, Device)> = host
        .devices()
        .map_err(|e| dev_err("list devices", e))?
        .filter(|d| {
            if input {
                d.supports_input()
            } else {
                d.supports_output()
            }
        })
        .map(|d| (device_id(&d), device_name(&d), d))
        .collect();
    let lower = wanted.to_lowercase();
    let pos = candidates
        .iter()
        .position(|(id, name, _)| id == wanted || name == wanted)
        .or_else(|| {
            candidates.iter().position(|(id, name, _)| {
                id.to_lowercase().contains(&lower) || name.to_lowercase().contains(&lower)
            })
        });
    match pos {
        Some(i) => Ok(candidates
            .into_iter()
            .nth(i)
            .expect("index from position")
            .2),
        None => Err(Error::Audio(format!(
            "no {kind} device matching {wanted:?}"
        ))),
    }
}

fn log_stream_error(e: cpal::Error) {
    tracing::warn!("audio stream error: {e}");
}

/// Microphone: 20 ms 48 kHz mono frames, converted from whatever the device runs at. Frames
/// are dropped (not queued without bound) if the consumer falls more than a second behind.
pub struct Capture {
    _stream: Stream,
    rx: mpsc::Receiver<Frame>,
}

impl Capture {
    /// Open the default input device, or the one matching `device` (see [`DeviceInfo`]).
    pub fn start(device: Option<&str>) -> Result<Self> {
        let device = pick_device(device, true)?;
        let supported = device
            .default_input_config()
            .map_err(|e| dev_err("input config", e))?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        tracing::info!(
            "audio capture: {} @ {} Hz, {} ch, {format:?}",
            device_name(&device),
            config.sample_rate,
            config.channels
        );
        let (tx, rx) = mpsc::channel(CAPTURE_DEPTH);
        let stream = match format {
            SampleFormat::I16 => build_input::<i16>(&device, config, tx),
            SampleFormat::U16 => build_input::<u16>(&device, config, tx),
            SampleFormat::I32 => build_input::<i32>(&device, config, tx),
            SampleFormat::F32 => build_input::<f32>(&device, config, tx),
            SampleFormat::F64 => build_input::<f64>(&device, config, tx),
            other => Err(Error::Audio(format!(
                "unsupported input sample format {other}"
            ))),
        }?;
        stream
            .play()
            .map_err(|e| dev_err("start input stream", e))?;
        Ok(Self {
            _stream: stream,
            rx,
        })
    }
}

fn build_input<T>(device: &Device, config: StreamConfig, tx: mpsc::Sender<Frame>) -> Result<Stream>
where
    T: SizedSample,
    i16: FromSample<T>,
{
    let mut conv = InputConverter::new(config.sample_rate, config.channels);
    let mut scratch: Vec<i16> = Vec::new();
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                scratch.clear();
                scratch.extend(data.iter().map(|&s| i16::from_sample(s)));
                for frame in conv.push(&scratch) {
                    // Full channel: the consumer is behind, drop rather than block the callback.
                    let _ = tx.try_send(frame);
                }
            },
            log_stream_error,
            None,
        )
        .map_err(|e| dev_err("build input stream", e))
}

impl FrameSource for Capture {
    async fn next_frame(&mut self) -> Option<Frame> {
        self.rx.recv().await
    }
}

/// Speaker: takes 20 ms 48 kHz mono frames, resamples/upmixes to the device format and plays
/// them through a [`JitterBuffer`] (silence while priming or when the supply runs dry).
pub struct Playback {
    _stream: Stream,
    buffer: Arc<Mutex<JitterBuffer>>,
    conv: OutputConverter,
}

impl Playback {
    /// Open the default output device, or the one matching `device` (see [`DeviceInfo`]).
    pub fn start(device: Option<&str>) -> Result<Self> {
        let device = pick_device(device, false)?;
        let supported = device
            .default_output_config()
            .map_err(|e| dev_err("output config", e))?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        tracing::info!(
            "audio playback: {} @ {} Hz, {} ch, {format:?}",
            device_name(&device),
            config.sample_rate,
            config.channels
        );
        let frame_len = config.sample_rate as usize / 50 * usize::from(config.channels);
        let buffer = Arc::new(Mutex::new(JitterBuffer::new(
            frame_len,
            PLAYOUT_PREFILL_FRAMES,
            PLAYOUT_MAX_FRAMES,
        )));
        let conv = OutputConverter::new(config.sample_rate, config.channels);
        let stream = match format {
            SampleFormat::I16 => build_output::<i16>(&device, config, buffer.clone()),
            SampleFormat::U16 => build_output::<u16>(&device, config, buffer.clone()),
            SampleFormat::I32 => build_output::<i32>(&device, config, buffer.clone()),
            SampleFormat::F32 => build_output::<f32>(&device, config, buffer.clone()),
            SampleFormat::F64 => build_output::<f64>(&device, config, buffer.clone()),
            other => Err(Error::Audio(format!(
                "unsupported output sample format {other}"
            ))),
        }?;
        stream
            .play()
            .map_err(|e| dev_err("start output stream", e))?;
        Ok(Self {
            _stream: stream,
            buffer,
            conv,
        })
    }

    /// Underruns heard so far (a rough health indicator for the call).
    pub fn underruns(&self) -> usize {
        self.buffer.lock().map(|b| b.underruns()).unwrap_or(0)
    }
}

fn build_output<T>(
    device: &Device,
    config: StreamConfig,
    buffer: Arc<Mutex<JitterBuffer>>,
) -> Result<Stream>
where
    T: SizedSample + FromSample<i16>,
{
    let mut scratch: Vec<i16> = Vec::new();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                scratch.resize(data.len(), 0);
                match buffer.lock() {
                    Ok(mut b) => b.pop_into(&mut scratch),
                    Err(_) => scratch.fill(0),
                }
                for (o, &s) in data.iter_mut().zip(&scratch) {
                    *o = T::from_sample(s);
                }
            },
            log_stream_error,
            None,
        )
        .map_err(|e| dev_err("build output stream", e))
}

impl FrameSink for Playback {
    async fn write_frame(&mut self, frame: &[i16]) -> Result<()> {
        let samples = self.conv.convert(frame);
        self.buffer
            .lock()
            .map_err(|_| Error::Audio("playback buffer poisoned".into()))?
            .push(&samples);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a sound server; run by hand with `cargo test -p calling -- --ignored`.
    #[test]
    #[ignore = "needs real audio devices"]
    fn lists_devices() {
        let outs = list_output_devices().unwrap();
        println!("{outs:#?}");
        println!("{:#?}", list_input_devices().unwrap());
    }
}
