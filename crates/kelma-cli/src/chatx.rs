//! Commands for message translation and link previews: `translate` and `preview`.

use anyhow::{Context, Result, bail};
use teams_proto::Client;
use teams_proto::translate::{self, MessageRef, Scope, Status, Trigger};
use teams_proto::urlpreview;

fn show_status(s: Status) -> &'static str {
    match s {
        Status::Done => "done",
        Status::MessageNotFound => "message not found",
        Status::VersionNotFound => "version not found",
        Status::LanguageDetectionFailed => "language not detected",
        Status::NotNeeded => "no translation needed",
        Status::Failed => "failed",
    }
}

async fn print_translations(client: &Client, scope: Scope, to: &str, ids: &[String]) -> Result<()> {
    if ids.is_empty() {
        bail!("give at least one message id");
    }
    let refs: Vec<MessageRef> = ids.iter().map(|i| MessageRef::new(i)).collect();
    let to = translate::language_code(to);
    for t in translate::translate_messages(client, &scope, &to, &refs, &Trigger::OnDemand).await? {
        println!("{}: {}", t.message_id, show_status(t.status));
        for (label, v) in [
            ("subject", &t.subject),
            ("title", &t.title),
            ("content", &t.content_html),
        ] {
            if let Some(v) = v {
                println!("  {label}: {v}");
            }
        }
    }
    Ok(())
}

/// `kelma-cli translate languages [ui locale] | settings | message <chat> <to> <id>.. |
/// channel <team> <channel> <root> <to> <id>..`. Translating sends the message to Microsoft's
/// translator, so only run it on messages you may share.
pub async fn translate(client: &Client, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("languages") => {
            let locale = args.get(1).map_or("en-US", String::as_str);
            for l in translate::languages(client, locale).await? {
                println!("{:<10} {}", l.code, l.name);
            }
        }
        Some("settings") => {
            let s = translate::settings(client).await?;
            println!(
                "translate to  {} (chosen: {})",
                s.target(),
                s.target_language.is_some()
            );
            println!(
                "mode          {:?} (chosen: {})",
                s.effective_mode(),
                s.mode.is_some()
            );
            println!(
                "display lang  {}",
                s.display_language.as_deref().unwrap_or("-")
            );
            println!("known         {}", s.known_languages.join(", "));
        }
        Some("message") => {
            let (Some(chat), Some(to)) = (args.get(1), args.get(2)) else {
                bail!("usage: translate message <chat id> <to> <message id>..")
            };
            let scope = Scope::Chat {
                chat_id: chat.clone(),
            };
            print_translations(client, scope, to, &args[3..]).await?;
        }
        Some("channel") => {
            let usage =
                "usage: translate channel <team id> <channel id> <root id> <to> <message id>..";
            let a = |i: usize| args.get(i).context(usage);
            let scope = Scope::Channel {
                team_id: a(1)?.clone(),
                channel_id: a(2)?.clone(),
                root_id: a(3)?.clone(),
            };
            print_translations(client, scope, a(4)?, &args[5..]).await?;
        }
        _ => bail!(
            "usage: translate languages [ui locale] | settings | message <chat> <to> <id>.. | \
             channel <team> <channel> <root> <to> <id>.."
        ),
    }
    Ok(())
}

/// `kelma-cli preview <url> [--image]`: what Teams shows under that link. `--image` also
/// downloads the thumbnail and prints its size.
pub async fn preview(client: &Client, args: &[String]) -> Result<()> {
    let url = args.first().context("usage: preview <url> [--image]")?;
    let p = urlpreview::preview(client, url).await?;
    let show = |label: &str, v: Option<&str>| println!("{label:<12}{}", v.unwrap_or("-"));
    show("url", Some(&p.url));
    show("domain", Some(&p.domain()));
    show("site", p.site_name.as_deref());
    show("title", p.title.as_deref());
    show("description", p.description.as_deref());
    show("image", p.image_url.as_deref());
    show("large image", p.large_image_url.as_deref());
    show("favicon", p.favicon_url.as_deref());
    println!(
        "{:<12}{}x{}",
        "image size",
        p.image_width.unwrap_or(0),
        p.image_height.unwrap_or(0)
    );
    println!("{:<12}{:?}", "status", p.status);
    println!("{:<12}{}", "showable", p.is_showable());
    if args.iter().any(|a| a == "--image") {
        let image = p.image_url.as_deref().context("the page has no image")?;
        let bytes = urlpreview::fetch_image(client, image).await?;
        println!("{:<12}{} bytes", "downloaded", bytes.len());
    }
    Ok(())
}
