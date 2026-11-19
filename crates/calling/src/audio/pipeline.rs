use super::{FrameSink, FrameSource, OpusDecoder, OpusEncoder};
use crate::{OpusPacket, Result};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Channel depth between stages: one second of audio.
const DEPTH: usize = 50;

/// Pull frames from `source`, Opus-encode them, and hand the packets out. The task ends (and
/// the channel closes) when the source ends or the receiver is dropped. Needs a tokio runtime.
///
/// A source that is not tied to a clock (tone, file) should be wrapped in
/// [`Paced`](super::Paced) when the packets go out live.
pub fn spawn_capture_encoder<S>(mut source: S) -> mpsc::Receiver<OpusPacket>
where
    S: FrameSource + 'static,
{
    let (tx, rx) = mpsc::channel(DEPTH);
    tokio::spawn(async move {
        let mut encoder = match OpusEncoder::new() {
            Ok(e) => e,
            Err(e) => {
                tracing::error!("audio capture: {e}");
                return;
            }
        };
        while let Some(frame) = source.next_frame().await {
            match encoder.encode(&frame) {
                Ok(packet) => {
                    if tx.send(packet).await.is_err() {
                        break;
                    }
                }
                Err(e) => tracing::warn!("audio capture: dropping frame: {e}"),
            }
        }
    });
    rx
}

/// Decode packets from `packets` into `sink`. An empty packet means "lost" and is concealed;
/// so is a packet that fails to decode, so the sink always gets one frame per packet. When
/// `packets` closes the sink is finished and the result is returned through the handle.
pub fn spawn_decoder_playback<K>(
    mut packets: mpsc::Receiver<OpusPacket>,
    mut sink: K,
) -> JoinHandle<Result<()>>
where
    K: FrameSink + 'static,
{
    tokio::spawn(async move {
        let mut decoder = OpusDecoder::new()?;
        while let Some(packet) = packets.recv().await {
            let frame = match decoder.decode(&packet) {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!("audio playback: bad packet, concealing: {e}");
                    decoder.conceal()?
                }
            };
            sink.write_frame(&frame).await?;
        }
        sink.finish().await
    })
}
