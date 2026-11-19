//! `kelma-cli web <url> [--shim] [--sso-resource R] [--valid-domain D]... [--theme T]
//! [--browser PATH] [--profile DIR]`: opens a page in the web host window (crate `webhost`) and
//! prints what the page asks for until the window is closed or Ctrl+C. The host profile keeps
//! whatever the user signs into inside the window. With `--shim` and a kelma session, app SSO
//! tokens (`getAuthToken`) are minted by the session for the app's own `api://<domain>/<app id>`
//! resource (`--sso-resource`, the manifest's `webApplicationInfo.resource`) after the web host
//! has checked that it belongs to the page's domain; without a session they are refused.

use anyhow::{Context, Result, bail};
use session::Paths;
use std::path::PathBuf;
use teams_proto::Client;

#[derive(Debug, Default, PartialEq)]
struct WebArgs {
    url: String,
    shim: bool,
    theme: Option<String>,
    browser: Option<PathBuf>,
    profile: Option<PathBuf>,
    sso_resource: Option<String>,
    valid_domains: Vec<String>,
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
            "--sso-resource" => a.sso_resource = Some(value()?),
            "--valid-domain" => a.valid_domains.push(value()?),
            flag if flag.starts_with("--") => bail!("unknown flag {flag}"),
            url if a.url.is_empty() => a.url = url.to_string(),
            extra => bail!("only one url, got another: {extra}"),
        }
    }
    if a.url.is_empty() {
        bail!(
            "usage: kelma-cli web <url> [--shim] [--sso-resource api://<domain>/<app id>] [--valid-domain D]... [--theme default|dark|contrast] [--browser PATH] [--profile DIR]"
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
            sso_resource: a.sso_resource.clone(),
            valid_domains: a.valid_domains.clone(),
            // A dev window: no device permissions, like an app the user granted nothing.
            granted: Vec::new(),
        });
        let client = match signed_in(paths).await {
            Ok(c) => Some(c),
            Err(e) => {
                println!("app SSO tokens will be refused: {e:#}");
                None
            }
        };
        opts.token = Some(std::sync::Arc::new(move |req: webhost::TokenRequest| {
            let client = client.clone();
            Box::pin(async move {
                let (origin, resources) = (req.origin.clone(), req.resources.clone());
                let result = app_token(client.as_ref(), req).await;
                match &result {
                    Ok(_) => println!("token for {resources:?} served to {origin}"),
                    Err(reason) => {
                        println!("token for {resources:?} refused to {origin}: {reason}")
                    }
                }
                result
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

/// The signed-in kelma session, if there is one.
async fn signed_in(paths: &Paths) -> Result<Client> {
    let cfg = paths.load_config()?;
    let sess = session::stored_session(&cfg)?.context("not signed in: run `kelma-cli login`")?;
    let client = Client::resume(sess).await?;
    session::persist(&cfg, &client).await?;
    Ok(client)
}

/// Answers a `getAuthToken` request the web host has already checked (one resource, on the
/// page's domain) with a token from the session, or the reason for teams-js.
async fn app_token(
    client: Option<&Client>,
    req: webhost::TokenRequest,
) -> std::result::Result<String, String> {
    let client = client.ok_or("kelma is not signed in, so it cannot get app tokens")?;
    let resource = req.resources.first().ok_or("no resource")?;
    let claims = webhost::shim::merge_claims(&req.claims);
    client
        .app_token(resource, claims.as_deref())
        .await
        .map(|t| t.access_token)
        .map_err(|e| webhost::shim::token_failure_reason(&e.to_string()))
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
                ..WebArgs::default()
            }
        );
    }

    #[test]
    fn parses_the_app_sso_resource_and_valid_domains() {
        let a = parse(&args(&[
            "https://app.example.com/tab",
            "--sso-resource",
            "api://app.example.com/abc",
            "--valid-domain",
            "example.com",
            "--valid-domain",
            "*.example.net",
        ]))
        .unwrap();
        assert_eq!(a.sso_resource.as_deref(), Some("api://app.example.com/abc"));
        assert_eq!(a.valid_domains, vec!["example.com", "*.example.net"]);
    }

    #[tokio::test]
    async fn app_tokens_are_refused_without_a_session() {
        let req = webhost::TokenRequest {
            resources: vec!["api://app.example.com/abc".into()],
            claims: Vec::new(),
            silent: true,
            tenant_id: None,
            origin: "https://app.example.com".into(),
        };
        let err = app_token(None, req).await.unwrap_err();
        assert!(err.contains("not signed in"), "{err}");
    }

    /// The generic app path against the live service with a first-party resource kelma already
    /// uses. Needs `kelma-cli login`; prints only the expiry, never the token.
    #[tokio::test]
    #[ignore = "mints a live token"]
    async fn live_app_token_generic_path() {
        teams_proto::client::install_crypto_provider();
        let paths = Paths::new().unwrap();
        let client = signed_in(&paths).await.expect("signed in");
        let t = client
            .app_token("https://graph.microsoft.com", None)
            .await
            .expect("minted");
        let left = t
            .expires_at
            .duration_since(std::time::SystemTime::now())
            .unwrap_or_default();
        assert!(!t.access_token.is_empty());
        println!("app path token minted, expires in {} s", left.as_secs());
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
