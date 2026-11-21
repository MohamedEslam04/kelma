//! `ironteams-cli tray-demo [--step SECS]` and `ironteams-cli tray-demo autostart <enable|disable|status>`.
//!
//! The demo shows the tray icon and walks the unread badge through 3, 12, 99+ and 0, toggles
//! mute, and emits the matching launcher badge counts, then exits. Clicks and menu picks made
//! while it runs are printed. `autostart` writes or removes the autostart entry; set
//! `XDG_CONFIG_HOME` to try it without touching `~/.config`.

use anyhow::{Result, bail};
use std::time::Duration;
use tray::{Tray, TrayEvent, TrayOptions, autostart};

pub fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("autostart") => autostart_cmd(&args[1..]),
        Some("--step") | None => demo(step_from(args)?),
        Some(other) => {
            bail!("unknown tray-demo argument {other}; try `--step SECS` or `autostart`")
        }
    }
}

fn step_from(args: &[String]) -> Result<Duration> {
    match args {
        [] => Ok(Duration::from_secs(4)),
        [flag, secs] if flag == "--step" => Ok(Duration::from_secs_f32(secs.parse()?)),
        _ => bail!("usage: tray-demo [--step SECS]"),
    }
}

fn autostart_cmd(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("enable") => {
            let exe = std::env::current_exe()?;
            let path = autostart::enable(&exe.to_string_lossy(), Some("--hidden"))?;
            println!("Autostart enabled: {}", path.display());
        }
        Some("disable") => {
            autostart::disable()?;
            println!("Autostart disabled.");
        }
        Some("status") | None => println!(
            "Autostart is {}.",
            if autostart::is_enabled() { "on" } else { "off" }
        ),
        Some(other) => bail!("unknown autostart command {other}; try enable, disable or status"),
    }
    Ok(())
}

/// Wait up to `d`, printing any tray events that arrive meanwhile.
fn hold(tray: &tray::TrayHandle, d: Duration) {
    let events = tray.events();
    let end = std::time::Instant::now() + d;
    while let Some(left) = end.checked_duration_since(std::time::Instant::now()) {
        match events.recv_timeout(left) {
            Ok(TrayEvent::ToggleMute(m)) => {
                println!("  event: ToggleMute({m})");
                tray.set_muted(m);
            }
            Ok(e) => println!("  event: {e:?}"),
            Err(_) => break,
        }
    }
}

fn demo(step: Duration) -> Result<()> {
    let tray = Tray::start(TrayOptions::default());
    match tray.tray_error() {
        None => println!("Tray icon registered."),
        Some(e) => println!("Tray icon NOT registered: {e}"),
    }
    println!(
        "Launcher badge bus: {}",
        if tray.launcher_ready() {
            "connected"
        } else {
            "unavailable"
        }
    );
    hold(&tray, step);
    for count in [3, 12, 120, 0] {
        println!("unread = {count}");
        tray.set_unread(count);
        hold(&tray, step);
    }
    for muted in [true, false] {
        println!("muted = {muted}");
        tray.set_muted(muted);
        hold(&tray, step);
    }
    tray.shutdown();
    println!("Tray removed.");
    Ok(())
}
