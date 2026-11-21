//! `ironteams-cli appearance [show|watch]`: print the saved appearance, general and accessibility
//! preferences, what the desktop portal says, and the theme, accent, language, contrast and motion
//! IronTeams would use. `watch` then prints live portal changes
//! until Ctrl+C. Read-only: nothing is written.

use anyhow::{Result, bail};
use appearance::{
    AccessibilityPrefs, AppearancePrefs, GeneralPrefs, PortalSettings, ResolvedTheme, portal,
    resolve,
};
use futures_util::StreamExt;

fn show(prefs: &AppearancePrefs, portal: Option<&PortalSettings>) {
    println!(
        "file:       {}",
        AppearancePrefs::path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|e| e.to_string())
    );
    println!(
        "prefs:      {}",
        serde_json::to_string(prefs).unwrap_or_default()
    );
    match portal {
        Some(p) => println!(
            "portal:     color-scheme {:?}, accent {}, high contrast {}, animations {}",
            p.color_scheme,
            p.accent.map_or("none".into(), |c| c.to_hex()),
            p.high_contrast,
            p.animations_enabled
                .map_or("not exposed".into(), |a| a.to_string())
        ),
        None => println!("portal:     not available"),
    }
    let r = resolve(prefs, portal);
    println!(
        "resolved:   theme {}, accent {}, density {:?}, font scale {}",
        match r.theme {
            ResolvedTheme::Light => "light",
            ResolvedTheme::Dark => "dark",
        },
        r.accent.map_or("IronTeams default".into(), |c| c.to_hex()),
        r.density,
        r.font_scale
    );
    let general = GeneralPrefs::load();
    let a11y = AccessibilityPrefs::load();
    let lang = appearance::effective_language(&general);
    println!(
        "general:    {}",
        serde_json::to_string(&general).unwrap_or_default()
    );
    println!(
        "a11y:       {}",
        serde_json::to_string(&a11y).unwrap_or_default()
    );
    println!(
        "effective:  language {} ({}), high contrast {}, reduced motion {}, text scale {}",
        lang.tag,
        if lang.rtl { "rtl" } else { "ltr" },
        appearance::resolve_high_contrast(&a11y, portal),
        appearance::resolve_reduced_motion(&a11y, portal),
        appearance::effective_font_scale(prefs, &a11y)
    );
}

pub async fn run(args: &[String]) -> Result<()> {
    let sub = args.first().map(String::as_str).unwrap_or("show");
    if !matches!(sub, "show" | "watch") {
        bail!("usage: ironteams-cli appearance [show|watch]");
    }
    let prefs = AppearancePrefs::load();
    let mut state = portal::read().await?;
    show(&prefs, state.as_ref());
    if sub == "watch" {
        let Some(mut changes) = portal::watch().await? else {
            bail!("no session bus, nothing to watch");
        };
        println!("watching the portal, Ctrl+C to stop");
        let mut current = state.take().unwrap_or_default();
        loop {
            tokio::select! {
                change = changes.next() => {
                    let Some(change) = change else { bail!("portal connection closed") };
                    current.apply(change);
                    println!("{change:?}");
                    show(&prefs, Some(&current));
                }
                _ = tokio::signal::ctrl_c() => break,
            }
        }
    }
    Ok(())
}
