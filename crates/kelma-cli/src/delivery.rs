//! `kelma-cli deliver`: send a message with a delivery option, and show what messages carry.

use anyhow::{Context, Result, bail};
use teams_proto::Client;
use teams_proto::delivery::{Announcement, Delivery};

const USAGE: &str = "usage: deliver send <conversation id> <standard|important|urgent> <text> | \
     deliver announce <channel id> <headline> <text> [theme 0-11] | \
     deliver show <conversation id> [n]";

/// Parses the delivery word of `deliver send`.
fn parse_option(word: &str) -> Result<Delivery> {
    match word {
        "standard" => Ok(Delivery::Standard),
        "important" => Ok(Delivery::Important),
        "urgent" => Ok(Delivery::Urgent),
        other => bail!("unknown delivery option `{other}` (standard, important or urgent)"),
    }
}

/// Announcements are a channel feature; refuse any other conversation up front.
fn require_channel(conversation_id: &str) -> Result<()> {
    if !conversation_id.contains("@thread.tacv2") {
        bail!("announcements can only be posted in a channel");
    }
    Ok(())
}

/// `deliver send|announce|show`. Sending posts a real message, so aim it carefully.
pub async fn run(client: &Client, display_name: &str, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("send") => {
            let a = |i: usize| args.get(i).context(USAGE);
            let (conv, option, text) = (a(1)?, parse_option(a(2)?)?, a(3)?);
            let html = render::text_to_teams_html(text);
            let cmid = client
                .send_message_delivery(conv, &html, display_name, None, &option)
                .await?;
            println!("sent (clientmessageid {cmid})");
        }
        Some("announce") => {
            let a = |i: usize| args.get(i).context(USAGE);
            let (conv, headline, text) = (a(1)?, a(2)?, a(3)?);
            require_channel(conv)?;
            let theme: u8 = match args.get(4) {
                Some(t) => t.parse().context("theme is a number from 0 to 11")?,
                None => 0,
            };
            let announcement = Announcement::with_theme(headline, theme)?;
            let html = render::text_to_teams_html(text);
            let cmid = client
                .send_message_delivery(
                    conv,
                    &html,
                    display_name,
                    None,
                    &Delivery::Announcement(announcement),
                )
                .await?;
            println!("posted (clientmessageid {cmid})");
        }
        Some("show") => {
            let conv = args.get(1).context(USAGE)?;
            let n: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
            let page = client.messages(conv, None, n).await?;
            for m in page.messages.iter().filter(|m| m.is_chat_content()) {
                let ann = match &m.announcement {
                    Some(a) => format!(
                        " announcement(title={:?}, theme={}, image={})",
                        a.title,
                        a.color_theme.as_deref().unwrap_or("-"),
                        a.image.is_some()
                    ),
                    None => String::new(),
                };
                println!(
                    "{} importance={:?}{ann}  {}",
                    m.id,
                    m.importance,
                    render::plain_text(&render::parse_teams_html(&m.content))
                );
            }
        }
        _ => bail!(USAGE),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_delivery_words() {
        assert_eq!(parse_option("standard").unwrap(), Delivery::Standard);
        assert_eq!(parse_option("important").unwrap(), Delivery::Important);
        assert_eq!(parse_option("urgent").unwrap(), Delivery::Urgent);
        assert!(parse_option("loud").is_err());
    }

    #[test]
    fn announcements_are_refused_outside_channels() {
        assert!(require_channel("48:notes").is_err());
        assert!(require_channel("19:abc@thread.v2").is_err());
        assert!(require_channel("19:abc@thread.tacv2").is_ok());
    }
}
