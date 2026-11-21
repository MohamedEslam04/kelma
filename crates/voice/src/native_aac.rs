//! AAC-LC mono in MP4 with the system's own encoder, so Windows and macOS need no ffmpeg: Media
//! Foundation's AAC encoder and MPEG-4 sink on Windows, AudioToolbox's `ExtAudioFile` on macOS.
//! Both write the file at `output` (MP4 needs a seekable output); AudioToolbox brands it `mp42`
//! as ffmpeg does, Media Foundation with its own default brand.

#[cfg(any(windows, target_os = "macos"))]
use crate::{Clip, Error, Result};
#[cfg(any(windows, target_os = "macos"))]
use std::path::Path;

/// Sample rate of the encoded stream (what Teams voice messages use).
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
const OUTPUT_RATE: u32 = 44_100;

/// `samples` at `from` Hz, linearly resampled to `to` Hz. Speech only needs this much: the band
/// that matters is far below either Nyquist frequency.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() || from == 0 || to == 0 {
        return samples.to_vec();
    }
    let len = (samples.len() as u64 * u64::from(to) / u64::from(from)).max(1) as usize;
    let step = f64::from(from) / f64::from(to);
    (0..len)
        .map(|i| {
            let pos = i as f64 * step;
            let at = pos as usize;
            let frac = (pos - at as f64) as f32;
            let a = samples[at.min(samples.len() - 1)];
            let b = samples[(at + 1).min(samples.len() - 1)];
            a + (b - a) * frac
        })
        .collect()
}

#[cfg(windows)]
pub(crate) use win::encode;

/// Whether Media Foundation (and so the AAC encoder) is installed.
#[cfg(windows)]
pub(crate) fn available() -> bool {
    win::load().is_ok()
}

#[cfg(target_os = "macos")]
pub(crate) use mac::encode;

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use std::ffi::c_void;

    type OsStatus = i32;
    type ExtAudioFileRef = *mut c_void;

    #[repr(C)]
    struct StreamDescription {
        sample_rate: f64,
        format_id: u32,
        format_flags: u32,
        bytes_per_packet: u32,
        frames_per_packet: u32,
        bytes_per_frame: u32,
        channels_per_frame: u32,
        bits_per_channel: u32,
        reserved: u32,
    }

    #[repr(C)]
    struct AudioBuffer {
        channels: u32,
        byte_size: u32,
        data: *mut c_void,
    }

    #[repr(C)]
    struct AudioBufferList {
        count: u32,
        buffers: [AudioBuffer; 1],
    }

    const fn fourcc(c: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*c)
    }
    const MPEG4_FILE: u32 = fourcc(b"mp4f");
    const AAC: u32 = fourcc(b"aac ");
    const LINEAR_PCM: u32 = fourcc(b"lpcm");
    const CLIENT_DATA_FORMAT: u32 = fourcc(b"cfmt");
    const FLAG_FLOAT: u32 = 1;
    const FLAG_PACKED: u32 = 1 << 3;
    const ERASE_FILE: u32 = 1;

    #[link(name = "AudioToolbox", kind = "framework")]
    unsafe extern "C" {
        fn ExtAudioFileCreateWithURL(
            url: *const c_void,
            file_type: u32,
            format: *const StreamDescription,
            layout: *const c_void,
            flags: u32,
            out: *mut ExtAudioFileRef,
        ) -> OsStatus;
        fn ExtAudioFileSetProperty(
            file: ExtAudioFileRef,
            id: u32,
            size: u32,
            data: *const c_void,
        ) -> OsStatus;
        fn ExtAudioFileWrite(
            file: ExtAudioFileRef,
            frames: u32,
            data: *const AudioBufferList,
        ) -> OsStatus;
        fn ExtAudioFileDispose(file: ExtAudioFileRef) -> OsStatus;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(
            allocator: *const c_void,
            path: *const u8,
            len: isize,
            is_directory: u8,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
    }

    fn check(status: OsStatus, what: &str) -> Result<()> {
        if status == 0 {
            Ok(())
        } else {
            Err(Error::Encode(format!(
                "AudioToolbox {what}: OSStatus {status}"
            )))
        }
    }

    /// Closes the file (which writes the MP4 index) when dropped.
    struct File(ExtAudioFileRef);

    impl Drop for File {
        fn drop(&mut self) {
            // SAFETY: the reference came from ExtAudioFileCreateWithURL and is disposed once.
            unsafe { ExtAudioFileDispose(self.0) };
        }
    }

    pub(crate) fn encode(clip: &Clip, output: &Path) -> Result<()> {
        use std::os::unix::ffi::OsStrExt;
        let path = output.as_os_str().as_bytes();
        // SAFETY: the path bytes outlive the call; a null allocator means the default one.
        let url = unsafe {
            CFURLCreateFromFileSystemRepresentation(
                std::ptr::null(),
                path.as_ptr(),
                path.len() as isize,
                0,
            )
        };
        if url.is_null() {
            return Err(Error::Encode("AudioToolbox: bad output path".into()));
        }
        let aac = StreamDescription {
            sample_rate: f64::from(OUTPUT_RATE),
            format_id: AAC,
            format_flags: 0,
            bytes_per_packet: 0,
            frames_per_packet: 1024,
            bytes_per_frame: 0,
            channels_per_frame: 1,
            bits_per_channel: 0,
            reserved: 0,
        };
        let mut file: ExtAudioFileRef = std::ptr::null_mut();
        // SAFETY: `url` is a live CFURL, `aac` a complete description, `file` receives the ref.
        let status = unsafe {
            ExtAudioFileCreateWithURL(
                url,
                MPEG4_FILE,
                &aac,
                std::ptr::null(),
                ERASE_FILE,
                &mut file,
            )
        };
        // SAFETY: created above, released once.
        unsafe { CFRelease(url) };
        check(status, "create file")?;
        let file = File(file);
        // What we hand over: float mono at the recording's rate (AudioToolbox converts the rate).
        let pcm = StreamDescription {
            sample_rate: f64::from(clip.sample_rate),
            format_id: LINEAR_PCM,
            format_flags: FLAG_FLOAT | FLAG_PACKED,
            bytes_per_packet: 4,
            frames_per_packet: 1,
            bytes_per_frame: 4,
            channels_per_frame: 1,
            bits_per_channel: 32,
            reserved: 0,
        };
        // SAFETY: the property is an AudioStreamBasicDescription of the given size.
        check(
            unsafe {
                ExtAudioFileSetProperty(
                    file.0,
                    CLIENT_DATA_FORMAT,
                    std::mem::size_of::<StreamDescription>() as u32,
                    (&pcm as *const StreamDescription).cast(),
                )
            },
            "set the input format",
        )?;
        for chunk in clip.samples.chunks(4096) {
            let list = AudioBufferList {
                count: 1,
                buffers: [AudioBuffer {
                    channels: 1,
                    byte_size: (chunk.len() * 4) as u32,
                    data: chunk.as_ptr() as *mut c_void,
                }],
            };
            // SAFETY: the buffer list points at `chunk`, alive for the call; it is only read.
            check(
                unsafe { ExtAudioFileWrite(file.0, chunk.len() as u32, &list) },
                "write",
            )?;
        }
        drop(file);
        Ok(())
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::ffi::c_void;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Media::MediaFoundation::{
        IMFAttributes, IMFMediaBuffer, IMFMediaType, IMFSample, IMFSinkWriter,
        MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT,
        MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
        MF_TRANSCODE_CONTAINERTYPE, MFAudioFormat_AAC, MFAudioFormat_PCM, MFMediaType_Audio,
        MFTranscodeContainerType_MPEG4,
    };
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::core::{GUID, HRESULT, HSTRING, Interface, PCSTR, PCWSTR, s, w};

    /// `MF_VERSION`: SDK 2, API 0x70.
    const MF_VERSION: u32 = 0x0002_0070;
    /// 96 kbit/s, the lowest rate the Media Foundation AAC encoder accepts.
    const AAC_BYTES_PER_SECOND: u32 = 12_000;

    type Startup = unsafe extern "system" fn(u32, u32) -> HRESULT;
    type Shutdown = unsafe extern "system" fn() -> HRESULT;
    type CreateOut = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
    type CreateAttributes = unsafe extern "system" fn(*mut *mut c_void, u32) -> HRESULT;
    type CreateBuffer = unsafe extern "system" fn(u32, *mut *mut c_void) -> HRESULT;
    type CreateWriter =
        unsafe extern "system" fn(PCWSTR, *mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT;

    /// Media Foundation's entry points, loaded at run time: Windows N editions without the Media
    /// Feature Pack have no Media Foundation, and kelma must still start there.
    pub(super) struct Mf {
        startup: Startup,
        shutdown: Shutdown,
        create_attributes: CreateAttributes,
        create_media_type: CreateOut,
        create_sample: CreateOut,
        create_buffer: CreateBuffer,
        create_writer: CreateWriter,
    }

    fn proc<T>(module: HMODULE, name: PCSTR) -> Result<T> {
        // SAFETY: GetProcAddress only looks the name up.
        let f = unsafe { GetProcAddress(module, name) }
            .ok_or_else(|| Error::Encode("Media Foundation is incomplete on this system".into()))?;
        // SAFETY: `T` is the documented signature of the named function (a plain fn pointer).
        Ok(unsafe { std::mem::transmute_copy(&f) })
    }

    pub(super) fn load() -> Result<Mf> {
        let missing = |_| {
            Error::Encode(
                "Media Foundation is not installed (Windows N: install the Media Feature Pack)"
                    .into(),
            )
        };
        // SAFETY: loading system libraries by name; they stay loaded for the process.
        let plat = unsafe { LoadLibraryW(w!("mfplat.dll")) }.map_err(missing)?;
        // SAFETY: as above.
        let rw = unsafe { LoadLibraryW(w!("mfreadwrite.dll")) }.map_err(missing)?;
        Ok(Mf {
            startup: proc(plat, s!("MFStartup"))?,
            shutdown: proc(plat, s!("MFShutdown"))?,
            create_attributes: proc(plat, s!("MFCreateAttributes"))?,
            create_media_type: proc(plat, s!("MFCreateMediaType"))?,
            create_sample: proc(plat, s!("MFCreateSample"))?,
            create_buffer: proc(plat, s!("MFCreateMemoryBuffer"))?,
            create_writer: proc(rw, s!("MFCreateSinkWriterFromURL"))?,
        })
    }

    fn mf(what: &str) -> impl Fn(windows::core::Error) -> Error + '_ {
        move |e| Error::Encode(format!("Media Foundation {what}: {e}"))
    }

    /// An interface from a creator that hands back a raw pointer.
    fn create<T: Interface>(f: impl FnOnce(*mut *mut c_void) -> HRESULT, what: &str) -> Result<T> {
        let mut raw = std::ptr::null_mut();
        f(&mut raw).ok().map_err(mf(what))?;
        // SAFETY: on success the creator returned an owned reference of interface `T`.
        Ok(unsafe { T::from_raw(raw) })
    }

    fn audio_type(mf_: &Mf, subtype: &GUID, bytes_per_second: u32) -> Result<IMFMediaType> {
        // SAFETY: the creator is MFCreateMediaType.
        let t: IMFMediaType = create(|p| unsafe { (mf_.create_media_type)(p) }, "media type")?;
        // SAFETY: plain attribute setters on a live media type.
        unsafe {
            t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
                .map_err(mf("media type"))?;
            t.SetGUID(&MF_MT_SUBTYPE, subtype)
                .map_err(mf("media type"))?;
            t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)
                .map_err(mf("media type"))?;
            t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, OUTPUT_RATE)
                .map_err(mf("media type"))?;
            t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 1)
                .map_err(mf("media type"))?;
            t.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, bytes_per_second)
                .map_err(mf("media type"))?;
        }
        Ok(t)
    }

    pub(crate) fn encode(clip: &Clip, output: &Path) -> Result<()> {
        let mf_ = load()?;
        // SAFETY: COM for this (blocking) thread; balanced below when it was initialized here.
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        // SAFETY: balanced by the shutdown below.
        let started = unsafe { (mf_.startup)(MF_VERSION, 0) }
            .ok()
            .map_err(mf("start"));
        let result = started.and_then(|()| write(&mf_, clip, output));
        // SAFETY: matched with the successful startup / COM initialization above.
        unsafe {
            let _ = (mf_.shutdown)();
            if com {
                CoUninitialize();
            }
        }
        result
    }

    fn write(mf_: &Mf, clip: &Clip, output: &Path) -> Result<()> {
        // SAFETY: the creator is MFCreateAttributes.
        let attrs: IMFAttributes =
            create(|p| unsafe { (mf_.create_attributes)(p, 2) }, "attributes")?;
        // SAFETY: attribute setters on a live store.
        unsafe {
            attrs
                .SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)
                .map_err(mf("attributes"))?;
        }
        let url = HSTRING::from(output.as_os_str());
        // SAFETY: the creator is MFCreateSinkWriterFromURL; `url` and `attrs` outlive the call.
        let writer: IMFSinkWriter = create(
            |p| unsafe {
                (mf_.create_writer)(
                    PCWSTR(url.as_ptr()),
                    std::ptr::null_mut(),
                    attrs.as_raw(),
                    p,
                )
            },
            "sink writer",
        )?;
        let out = audio_type(mf_, &MFAudioFormat_AAC, AAC_BYTES_PER_SECOND)?;
        let input = audio_type(mf_, &MFAudioFormat_PCM, OUTPUT_RATE * 2)?;
        // SAFETY: setter on a live media type.
        unsafe { input.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 2) }.map_err(mf("media type"))?;
        // SAFETY: the writer and both types are live; no encoding parameters.
        let stream = unsafe {
            let stream = writer.AddStream(&out).map_err(mf("add stream"))?;
            writer
                .SetInputMediaType(stream, &input, None::<&IMFAttributes>)
                .map_err(mf("input type"))?;
            writer.BeginWriting().map_err(mf("begin"))?;
            stream
        };
        let pcm: Vec<i16> = resample_linear(&clip.samples, clip.sample_rate, OUTPUT_RATE)
            .into_iter()
            .map(|s| (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)
            .collect();
        let mut written: i64 = 0;
        for chunk in pcm.chunks(4410) {
            let bytes = (chunk.len() * 2) as u32;
            // SAFETY: the creator is MFCreateMemoryBuffer.
            let buffer: IMFMediaBuffer =
                create(|p| unsafe { (mf_.create_buffer)(bytes, p) }, "buffer")?;
            // SAFETY: the locked region holds `bytes` bytes; exactly that many are copied in.
            unsafe {
                let mut data = std::ptr::null_mut();
                buffer.Lock(&mut data, None, None).map_err(mf("lock"))?;
                std::ptr::copy_nonoverlapping(chunk.as_ptr().cast::<u8>(), data, bytes as usize);
                buffer.Unlock().map_err(mf("unlock"))?;
                buffer.SetCurrentLength(bytes).map_err(mf("buffer"))?;
            }
            // SAFETY: the creator is MFCreateSample.
            let sample: IMFSample = create(|p| unsafe { (mf_.create_sample)(p) }, "sample")?;
            // Times in 100 ns units.
            let at = written * 10_000_000 / i64::from(OUTPUT_RATE);
            let len = chunk.len() as i64 * 10_000_000 / i64::from(OUTPUT_RATE);
            // SAFETY: live sample, buffer and writer.
            unsafe {
                sample.AddBuffer(&buffer).map_err(mf("sample"))?;
                sample.SetSampleTime(at).map_err(mf("sample"))?;
                sample.SetSampleDuration(len).map_err(mf("sample"))?;
                writer.WriteSample(stream, &sample).map_err(mf("write"))?;
            }
            written += chunk.len() as i64;
        }
        // SAFETY: live writer; Finalize writes the MP4 index and closes the file.
        unsafe { writer.Finalize() }.map_err(mf("finalize"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_the_duration_and_the_shape() {
        let up: Vec<f32> = (0..480).map(|i| i as f32 / 480.0).collect();
        let down = resample_linear(&up, 48_000, 44_100);
        assert_eq!(down.len(), 441);
        assert_eq!(down[0], 0.0);
        // A ramp stays a ramp.
        assert!(down.windows(2).all(|w| w[1] > w[0]));
        assert!((down[440] - 440.0 * 48_000.0 / 44_100.0 / 480.0).abs() < 1e-3);
        assert_eq!(resample_linear(&up, 48_000, 48_000), up);
        assert!(resample_linear(&[], 48_000, 44_100).is_empty());
    }
}
