//! `kelma-cli appcard`: send one app card to the notes chat, and read cards back (plan S3.6).

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use teams_proto::Client;
use teams_proto::card_send::{OutCard, card_message_body};
use teams_proto::delivery::Delivery;

const USAGE: &str =
    "usage: appcard send <48:notes> <hero|adaptive> [text] | appcard show <48:notes> [n]";

/// The only chat this command writes to: the user's notes to self.
const NOTES: &str = "48:notes";

/// A made-up card (no real app, no real data) for the live check.
fn demo_card(kind: &str) -> Result<OutCard> {
    Ok(match kind {
        "hero" => OutCard {
            app_id: "kelma-demo-app".into(),
            app_name: "kelma demo".into(),
            content_type: "application/vnd.microsoft.card.hero".into(),
            content: json!({
                "title": "kelma hero card",
                "subtitle": "composer card check",
                "text": "Made-up demo content.",
            }),
        },
        "adaptive" => OutCard {
            app_id: "kelma-demo-app".into(),
            app_name: "kelma demo".into(),
            content_type: "application/vnd.microsoft.card.adaptive".into(),
            content: json!({
                "type": "AdaptiveCard",
                "version": "1.4",
                "body": [
                    {"type": "TextBlock", "text": "kelma adaptive card", "weight": "Bolder"},
                    {"type": "TextBlock", "text": "Made-up demo content.", "isSubtle": true},
                ],
            }),
        },
        other => bail!("unknown card `{other}` (hero or adaptive)"),
    })
}

fn notes_only(conv: &str) -> Result<()> {
    if conv != NOTES {
        bail!("card writes only to {NOTES} (your notes), not {conv}");
    }
    Ok(())
}

pub async fn run(client: &Client, display_name: &str, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("send") => {
            let a = |i: usize| args.get(i).context(USAGE);
            let (conv, kind) = (a(1)?, a(2)?);
            notes_only(conv)?;
            let card = demo_card(kind)?;
            let text = args
                .get(3)
                .map_or("kelma composer card test", String::as_str);
            let html = render::text_to_teams_html(text);
            let cmid = teams_proto::time::now_ms().to_string();
            let body = card_message_body(
                &html,
                std::slice::from_ref(&card),
                display_name,
                &cmid,
                None,
                &[],
                None,
                &Delivery::Standard,
            );
            println!("body: {}", serde_json::to_string(&body)?);
            client
                .send_card_message_as(
                    conv,
                    &html,
                    std::slice::from_ref(&card),
                    display_name,
                    None,
                    &[],
                    None,
                    &Delivery::Standard,
                    &cmid,
                )
                .await?;
            println!("sent (clientmessageid {cmid})");
        }
        Some("show") => {
            let conv = args.get(1).context(USAGE)?;
            let n: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
            let page = client.messages(conv, None, n).await?;
            let mut shown = 0;
            for m in page.messages.iter().filter(|m| m.card.is_some()) {
                shown += 1;
                let card: Value = serde_json::from_str(m.card.as_deref().unwrap_or("null"))?;
                let types: Vec<&str> = card
                    .as_array()
                    .map(|l| l.iter().filter_map(|c| c["contentType"].as_str()).collect())
                    .unwrap_or_default();
                let rows = teams_proto::chat_card::rows_for_attachments(&card, "");
                println!(
                    "{} cmid={:?} type={} text={:?} card types={types:?} rows={}",
                    m.id,
                    m.client_message_id,
                    m.message_type,
                    render::plain_text(&render::parse_teams_html(&m.content)),
                    rows.len()
                );
                println!("{}", serde_json::to_string(&card)?);
            }
            println!("{shown} message(s) with a card in the last {n}");
        }
        _ => bail!(USAGE),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_notes_chat_is_written_to() {
        assert!(notes_only("48:notes").is_ok());
        for other in [
            "19:abc@thread.v2",
            "48:notes;messageid=1",
            "",
            "19:x@unq.gbl.spaces",
        ] {
            assert!(notes_only(other).is_err(), "{other}");
        }
    }

    #[test]
    fn the_demo_cards_are_made_up_and_unknown_kinds_are_refused() {
        assert!(demo_card("hero").is_ok());
        assert!(demo_card("adaptive").is_ok());
        assert!(demo_card("other").is_err());
    }
}
