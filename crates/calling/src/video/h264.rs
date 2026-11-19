//! H.264 as kelma sends it: Annex-B access units (start codes between NAL units), constrained
//! baseline, `packetization-mode=1`. webrtc-rs's H.264 payloader reads Annex-B and writes the RTP
//! (single NAL units, STAP-A for SPS/PPS, FU-A for large slices).

/// The NAL units of an Annex-B byte stream, without their start codes.
pub fn split_annexb(stream: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= stream.len() {
        if stream[i] == 0 && stream[i + 1] == 0 && stream[i + 2] == 1 {
            starts.push((i, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut units = Vec::with_capacity(starts.len());
    for (n, &(_, body)) in starts.iter().enumerate() {
        let mut end = starts.get(n + 1).map_or(stream.len(), |&(at, _)| at);
        // A four-byte start code's leading zero belongs to it, not to the unit before.
        while end > body && stream[end - 1] == 0 && n + 1 < starts.len() {
            end -= 1;
        }
        if end > body {
            units.push(&stream[body..end]);
        }
    }
    units
}

/// `nal_unit_type` of one NAL unit (its first byte's low five bits).
pub fn nal_type(unit: &[u8]) -> Option<u8> {
    unit.first().map(|b| b & 0x1f)
}

/// Whether an access unit starts a decodable picture: it carries an IDR slice (type 5).
pub fn is_keyframe(stream: &[u8]) -> bool {
    split_annexb(stream).iter().any(|u| nal_type(u) == Some(5))
}

/// The H.264 payload types and `profile-level-id`s kelma registers, in order of preference.
/// webrtc-rs pairs H.264 codecs by `packetization-mode` and the first two bytes of
/// `profile-level-id` only, so both constrained-baseline spellings are needed: the media servers
/// offer and answer `42C0..` (payload type 107, kelma's live re-offer and rusty-teams'
/// `teams_answer.sdp`), browsers `42e0..`. Level 3.1 (`1f`) covers 1080p at 15 frames a second
/// of screen content.
pub const PAYLOADS: [(u8, &str); 2] = [(107, "42c01f"), (108, "42e01f")];

/// The `a=fmtp` of a kelma H.264 line with `profile_level_id`.
pub fn fmtp(profile_level_id: &str) -> String {
    format!("level-asymmetry-allowed=1;packetization-mode=1;profile-level-id={profile_level_id}")
}

/// What a source asks of the encoder.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EncoderSettings {
    pub fps: f32,
    pub bitrate_bps: u32,
    /// Screen content (sharp text, few changes) rather than a camera.
    pub screen: bool,
    /// Frames between keyframes when nobody asks for one.
    pub keyframe_interval: u32,
}

impl EncoderSettings {
    /// A screen share: 15 frames a second at 1.5 Mbit/s (the web client seeds 1.5 Mbit/s as its
    /// send bandwidth, mautrix-teams `calls.go`), a keyframe every 2 s so a viewer who joins or
    /// loses a packet gets a picture soon without asking.
    pub fn screen() -> Self {
        EncoderSettings {
            fps: 15.0,
            bitrate_bps: 1_500_000,
            screen: true,
            keyframe_interval: 30,
        }
    }
}

/// One encoded picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedFrame {
    /// Annex-B.
    pub data: Vec<u8>,
    pub keyframe: bool,
}

#[cfg(feature = "screen-share")]
pub use encoder::H264Encoder;

#[cfg(feature = "screen-share")]
mod encoder {
    use super::{EncodedFrame, EncoderSettings, is_keyframe};
    use crate::video::I420Frame;
    use crate::{Error, Result};
    use openh264::encoder::{
        BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Profile, RateControlMode,
        SpsPpsStrategy, UsageType,
    };
    use openh264::formats::YUVSlices;
    use openh264::{OpenH264API, Timestamp};

    /// OpenH264 (Cisco's encoder, built from source by the `openh264` crate).
    pub struct H264Encoder {
        inner: Encoder,
        keyframe: bool,
        started: std::time::Instant,
    }

    impl H264Encoder {
        pub fn new(settings: EncoderSettings) -> Result<Self> {
            let config = EncoderConfig::new()
                .bitrate(BitRate::from_bps(settings.bitrate_bps))
                .max_frame_rate(FrameRate::from_hz(settings.fps))
                .usage_type(if settings.screen {
                    UsageType::ScreenContentRealTime
                } else {
                    UsageType::CameraVideoRealTime
                })
                .rate_control_mode(RateControlMode::Bitrate)
                .profile(Profile::Baseline)
                // SPS and PPS before every IDR, so a receiver can join at any keyframe.
                .sps_pps_strategy(SpsPpsStrategy::ConstantId)
                .intra_frame_period(IntraFramePeriod::from_num_frames(
                    settings.keyframe_interval,
                ))
                .skip_frames(true);
            let inner = Encoder::with_api_config(OpenH264API::from_source(), config)
                .map_err(|e| Error::Media(format!("H.264 encoder: {e}")))?;
            Ok(H264Encoder {
                inner,
                keyframe: true,
                started: std::time::Instant::now(),
            })
        }

        /// The next frame is a keyframe.
        pub fn request_keyframe(&mut self) {
            self.keyframe = true;
        }

        /// One frame in, its access unit out; `None` when the encoder skipped it (rate control).
        pub fn encode(&mut self, frame: &I420Frame) -> Result<Option<EncodedFrame>> {
            if std::mem::take(&mut self.keyframe) {
                self.inner.force_intra_frame();
            }
            let (y, u, v) = frame.planes();
            let w = frame.width;
            let yuv = YUVSlices::new((y, u, v), (w, frame.height), (w, w / 2, w / 2));
            let ms = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
            let bits = self
                .inner
                .encode_at(&yuv, Timestamp::from_millis(ms))
                .map_err(|e| Error::Media(format!("H.264 encode: {e}")))?;
            let data = bits.to_vec();
            if data.is_empty() {
                return Ok(None);
            }
            Ok(Some(EncodedFrame {
                keyframe: is_keyframe(&data),
                data,
            }))
        }
    }
}
