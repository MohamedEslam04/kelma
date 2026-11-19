//! `kelma-cli web <url> [--shim] [--theme T] [--browser PATH] [--profile DIR]`: opens a page in
//! the web host window (crate `webhost`) and prints what the page asks for until the window is
//! closed or Ctrl+C. Nothing is signed in here: the host profile keeps whatever the user signs
//! into inside the window. App SSO tokens are not minted yet (requests are printed and refused).

use anyhow::{Result, bail};
use session::Paths;
use std::path::PathBuf;

#[derive(Debug, Default, PartialEq)]
struct WebArgs {
    url: String,
    shim: bool,
    theme: Option<String>,
    browser: Option<PathBuf>,
    profile: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<WebArgs> {
    let mut a = WebArgs::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || -> Result<String> {
            match it.next() {
                Some(v) => Ok(v.clone()),
                None => bail!("{arg} needs a value"),
            }
        };
        match arg.as_str() {
            "--shim" => a.shim = true,
            "--theme" => a.theme = Some(value()?),
            "--browser" => a.browser = Some(value()?.into()),
            "--profile" => a.profile = Some(value()?.into()),
            flag if flag.starts_with("--") => bail!("unknown flag {flag}"),
            url if a.url.is_empty() => a.url = url.to_string(),
            extra => bail!("only one url, got another: {extra}"),
        }
    }
    if a.url.is_empty() {
        bail!(
            "usage: kelma-cli web <url> [--shim] [--theme default|dark|contrast] [--browser PATH] [--profile DIR]"
        );
    }
    Ok(a)
}

pub async fn run(paths: &Paths, args: &[String]) -> Result<()> {
    let a = parse(args)?;
    let profile = a
        .profile
        .unwrap_or_else(|| webhost::default_profile_dir(&paths.data_dir));
    let mut opts = webhost::WebOptions::new(&profile);
    opts.title = Some("kelma".into());
    opts.size = Some((1200, 800));
    opts.browser = a.browser;
    let mut url = a.url.clone();
    if a.shim {
        let cfg = paths.load_config().ok();
        let mut context = webhost::HostContext::default();
        if let Some(t) = &a.theme {
            context.theme = t.clone();
        }
        if let Some(cfg) = &cfg {
            context.user_display_name = cfg.display_name.clone().unwrap_or_default();
            if let Some(mri) = &cfg.self_mri {
                context.user_object_id = mri.trim_start_matches("8:orgid:").to_string();
            }
            if cfg.tenant != "organizations" {
                context.tenant_id = cfg.tenant.clone();
            }
        }
        url = webhost::shim::expand_url(&url, &context);
        opts.shim = Some(webhost::ShimConfig {
            context,
            allowed_origins: Vec::new(),
        });
        opts.token = Some(std::sync::Arc::new(|req: webhost::TokenRequest| {
            Box::pin(async move {
                println!(
                    "token request from {}: resources {:?}, silent {} (refused: no app SSO broker yet)",
                    req.origin, req.resources, req.silent
                );
                Err("kelma-cli web does not mint app tokens".to_string())
            })
        }));
    }
    println!("profile: {}", profile.display());
    let win = webhost::open(&url, opts).await?;
    win.on_message(std::sync::Arc::new(|call: webhost::BridgeCall| {
        Box::pin(async move {
            println!("window.kelma.invoke from {}: {}", call.origin, call.payload);
            Ok(serde_json::json!({ "ok": true }))
        })
    }));
    let mut events = win.events();
    println!("opened {url}; close the window or press Ctrl+C");
    loop {
        tokio::select! {
            _ = win.closed() => break,
            _ = tokio::signal::ctrl_c() => {
                win.close().await;
                break;
            }
            Ok(ev) = events.recv() => println!("app: {ev:?}"),
        }
    }
    println!("window closed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_the_url_and_flags_in_any_order() {
        let a = parse(&args(&[
            "--shim",
            "https://app.example.com/tab?l={locale}",
            "--theme",
            "dark",
            "--browser",
            "/usr/bin/vivaldi",
            "--profile",
            "/tmp/p",
        ]))
        .unwrap();
        assert_eq!(
            a,
            WebArgs {
                url: "https://app.example.com/tab?l={locale}".into(),
                shim: true,
                theme: Some("dark".into()),
                browser: Some("/usr/bin/vivaldi".into()),
                profile: Some("/tmp/p".into()),
            }
        );
    }

    #[test]
    fn a_url_is_required_and_unknown_flags_are_rejected() {
        assert!(parse(&args(&[])).is_err());
        assert!(parse(&args(&["--shim"])).is_err());
        assert!(parse(&args(&["https://a.test", "--nope"])).is_err());
        assert!(
            parse(&args(&["https://a.test", "--theme"])).is_err(),
            "value missing"
        );
        assert!(parse(&args(&["https://a.test", "https://b.test"])).is_err());
    }
}
