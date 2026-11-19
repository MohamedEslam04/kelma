//! Capture spike: pick a screen or window in the desktop's ScreenCast dialog, encode it as H.264
//! the way a call sends it, and write the Annex-B stream to a file.
//!
//! ```text
//! cargo run -p calling --example screencast -- [seconds] [out.h264] [screen|window|any]
//! ffprobe out.h264        # or: ffplay out.h264
//! ```

use std::io::Write;
use std::time::{Duration, Instant};

use calling::video::h264::{EncoderSettings, H264Encoder};
use calling::video::screencast::{CaptureSettings, ShareKind, pick};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5);
    let out = args.next().unwrap_or_else(|| "screencast.h264".into());
    let kind = match args.next().as_deref() {
        Some("screen") => ShareKind::Screen,
        Some("window") => ShareKind::Window,
        _ => ShareKind::Any,
    };
    let settings = CaptureSettings::default();
    println!("choose what to share in the desktop's dialog…");
    let Some(mut capture) = pick(kind, settings).await? else {
        println!("cancelled");
        return Ok(());
    };
    println!("capturing {:?} for {secs} s", capture.size);
    let mut encoder = H264Encoder::new(EncoderSettings::screen())?;
    let mut file = std::fs::File::create(&out)?;
    let (mut frames, mut keyframes, mut bytes) = (0u32, 0u32, 0usize);
    let started = Instant::now();
    let deadline = tokio::time::sleep(Duration::from_secs(secs));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            f = capture.frames.recv() => {
                let Some(frame) = f else { break };
                frames += 1;
                if let Some(e) = encoder.encode(&frame)? {
                    keyframes += u32::from(e.keyframe);
                    bytes += e.data.len();
                    file.write_all(&e.data)?;
                }
            }
        }
    }
    capture.stop().await;
    let secs = started.elapsed().as_secs_f64();
    println!(
        "{frames} frames ({:.1}/s), {keyframes} keyframes, {:.0} kbit/s -> {out}",
        f64::from(frames) / secs,
        bytes as f64 * 8.0 / secs / 1000.0
    );
    Ok(())
}
