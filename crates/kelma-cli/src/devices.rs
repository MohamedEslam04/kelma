//! `kelma-cli devices [audio|cameras|test-sound|mic-level]`: list sound devices and cameras
//! (read-only), play the test sound, or show a live microphone level meter. `test-sound` makes
//! a sound and `mic-level` opens the microphone, so neither runs unless asked for by name.

use anyhow::{Result, bail};
use devices::{AudioDevice, Camera};
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq)]
enum Cmd {
    List { audio: bool, cameras: bool },
    TestSound { output: Option<String> },
    MicLevel { input: Option<String>, seconds: u64 },
}

fn parse(args: &[String]) -> Result<Cmd> {
    let (sub, rest) = match args.split_first() {
        Some((s, r)) => (s.as_str(), r),
        None => {
            return Ok(Cmd::List {
                audio: true,
                cameras: true,
            });
        }
    };
    let mut device = None;
    let mut seconds = 10;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        let value = it.next();
        match (a.as_str(), value) {
            ("--device", Some(v)) => device = Some(v.clone()),
            ("--seconds", Some(v)) => seconds = v.parse()?,
            (flag, None) if flag.starts_with("--") => bail!("{flag} needs a value"),
            (other, _) => bail!("unexpected argument {other}"),
        }
    }
    Ok(match sub {
        "audio" => Cmd::List {
            audio: true,
            cameras: false,
        },
        "cameras" => Cmd::List {
            audio: false,
            cameras: true,
        },
        "test-sound" => Cmd::TestSound { output: device },
        "mic-level" => Cmd::MicLevel {
            input: device,
            seconds,
        },
        other => bail!("unknown devices command {other:?}; use audio|cameras|test-sound|mic-level"),
    })
}

fn format_audio(title: &str, devices: &[AudioDevice]) -> String {
    let mut out = format!("{title} ({})\n", devices.len());
    for d in devices {
        let rates: Vec<String> = d.sample_rates.iter().map(|r| r.to_string()).collect();
        out += &format!(
            "  {}{}\n    id {}  max {} ch  rates {}\n",
            if d.is_default { "* " } else { "  " },
            d.name,
            d.id,
            d.channels,
            rates.join("/"),
        );
    }
    out
}

fn format_cameras(cameras: &[Camera]) -> String {
    let mut out = format!("cameras ({})\n", cameras.len());
    for c in cameras {
        out += &format!(
            "  {}  {}\n    driver {}  bus {}\n",
            c.path, c.name, c.driver, c.bus
        );
        for f in &c.formats {
            let sizes: Vec<String> = f.sizes.iter().map(|(w, h)| format!("{w}x{h}")).collect();
            out += &format!("    {}: {}\n", f.fourcc, sizes.join(" "));
        }
    }
    out
}

pub fn run(args: &[String]) -> Result<()> {
    match parse(args)? {
        Cmd::List { audio, cameras } => {
            if audio {
                print!(
                    "{}",
                    format_audio("audio inputs", &devices::audio_inputs()?)
                );
                print!(
                    "{}",
                    format_audio("audio outputs", &devices::audio_outputs()?)
                );
            }
            if cameras {
                print!("{}", format_cameras(&devices::cameras()?));
            }
        }
        Cmd::TestSound { output } => {
            println!("playing a short tone...");
            devices::test_sound(output.as_deref())?;
        }
        Cmd::MicLevel { input, seconds } => {
            let stream = devices::mic_level(input.as_deref())?;
            let end = Instant::now() + Duration::from_secs(seconds);
            while Instant::now() < end {
                if let Some(l) = stream.recv_timeout(Duration::from_millis(100)) {
                    let bar = "#".repeat((l.rms.sqrt() * 40.0).round() as usize);
                    println!("{:6.1} dBFS  peak {:.2}  {bar}", l.rms_dbfs(), l.peak);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use devices::CameraFormat;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn no_argument_lists_everything_read_only() {
        assert_eq!(
            parse(&[]).unwrap(),
            Cmd::List {
                audio: true,
                cameras: true
            }
        );
        assert_eq!(
            parse(&s(&["audio"])).unwrap(),
            Cmd::List {
                audio: true,
                cameras: false
            }
        );
        assert_eq!(
            parse(&s(&["cameras"])).unwrap(),
            Cmd::List {
                audio: false,
                cameras: true
            }
        );
    }

    #[test]
    fn test_sound_and_mic_level_take_a_device_and_seconds() {
        assert_eq!(
            parse(&s(&["test-sound", "--device", "alsa:x"])).unwrap(),
            Cmd::TestSound {
                output: Some("alsa:x".into())
            }
        );
        assert_eq!(
            parse(&s(&["mic-level", "--seconds", "3"])).unwrap(),
            Cmd::MicLevel {
                input: None,
                seconds: 3
            }
        );
    }

    #[test]
    fn rejects_unknown_commands_and_flags() {
        assert!(parse(&s(&["frobnicate"])).is_err());
        assert!(parse(&s(&["audio", "--bogus"])).is_err());
        assert!(parse(&s(&["mic-level", "--seconds", "x"])).is_err());
    }

    #[test]
    fn audio_listing_marks_the_default() {
        let d = AudioDevice {
            id: "alsa:default".into(),
            name: "Default".into(),
            is_default: true,
            channels: 2,
            sample_rates: vec![44_100, 48_000],
        };
        let out = format_audio("audio outputs", &[d]);
        assert!(out.starts_with("audio outputs (1)\n"));
        assert!(out.contains("* Default"));
        assert!(out.contains("rates 44100/48000"));
    }

    #[test]
    fn camera_listing_shows_formats_and_sizes() {
        let c = Camera {
            path: "/dev/video0".into(),
            name: "Cam".into(),
            driver: "uvcvideo".into(),
            bus: "usb-1".into(),
            formats: vec![CameraFormat {
                fourcc: "MJPG".into(),
                sizes: vec![(1280, 720), (640, 480)],
            }],
        };
        let out = format_cameras(&[c]);
        assert!(out.contains("/dev/video0  Cam"));
        assert!(out.contains("MJPG: 1280x720 640x480"));
    }
}
