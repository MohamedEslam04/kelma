//! Sending a message that carries app cards, as the composer's apps picker does (plan S3.6).
//!
//! A message-extension result is a Bot Framework attachment (`contentType` + `content`). The
//! message is a normal `RichText/Html` message whose `properties.cards` is a JSON *string* of
//! the card list, the same property an announcement banner uses (see
//! [`crate::delivery::apply_delivery`]) and the one incoming bot cards are read from
//! (`model::message_card`). Each entry names the app and gives the card an id of its own:
//! `{appId, cardClientId, appName, contentType, content}`. The typed text is the message body.
//!
//! What the evidence supports: the property name, the JSON-string form, the entry keys `appId`,
//! `cardClientId`, `contentType` and `content` (announcement banner; the app unfurl card of
//! teams-lite adds `appName`), and that `content` is the card as the app returned it. Assumed:
//! that the chat service takes a card with the extension result's own `contentType` (adaptive,
//! hero, thumbnail) in this form, and that no `messagingExtensionId` is needed.

use crate::chatsvc::encode_conversation;
use crate::client::Client;
use crate::delivery::{Delivery, apply_delivery};
use crate::error::Result;
use crate::mentions::{Mention, outgoing_body};
use reqwest::Method;
use serde_json::{Value, json};

/// A card a message carries.
#[derive(Debug, Clone, PartialEq)]
pub struct OutCard {
    /// The app the extension belongs to (the app id of the manifest).
    pub app_id: String,
    pub app_name: String,
    /// `application/vnd.microsoft.card.adaptive`, `.hero`, `.thumbnail` and so on.
    pub content_type: String,
    /// The card as the app returned it.
    pub content: Value,
}

/// What the outbox stores for a list of cards: a JSON array of
/// `{appId, appName, contentType, content}`.
pub fn cards_to_json(cards: &[OutCard]) -> String {
    Value::Array(
        cards
            .iter()
            .map(|c| {
                json!({
                    "appId": c.app_id,
                    "appName": c.app_name,
                    "contentType": c.content_type,
                    "content": c.content,
                })
            })
            .collect(),
    )
    .to_string()
}

/// The cards of [`cards_to_json`]. Entries without an app id or a content type are dropped, and
/// text that is not a JSON array gives no cards.
pub fn cards_from_json(s: &str) -> Vec<OutCard> {
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(s) else {
        return Vec::new();
    };
    let text = |v: &Value, key: &str| v[key].as_str().unwrap_or_default().to_owned();
    list.iter()
        .filter_map(|v| {
            let card = OutCard {
                app_id: text(v, "appId"),
                app_name: text(v, "appName"),
                content_type: text(v, "contentType"),
                content: v.get("content").cloned().unwrap_or(Value::Null),
            };
            (!card.app_id.is_empty() && !card.content_type.is_empty()).then_some(card)
        })
        .collect()
}

/// The `properties.cards` list of a send: [`cards_to_json`] plus a `cardClientId` for each card,
/// built from the client message id so a retry sends the same ids.
pub fn cards_property(cards: &[OutCard], client_message_id: &str) -> Value {
    Value::Array(
        cards
            .iter()
            .enumerate()
            .map(|(i, c)| {
                json!({
                    "appId": c.app_id,
                    "cardClientId": format!("card-{client_message_id}-{i}"),
                    "appName": c.app_name,
                    "contentType": c.content_type,
                    "content": c.content,
                })
            })
            .collect(),
    )
}

/// Request body for a message with cards. `html` is the typed text (may be empty: the message
/// then holds a blank paragraph), and mentions, the `qtdMsgs` of a quote reply and a delivery
/// option travel along. With no
/// cards it is the same body [`outgoing_body`] makes.
#[allow(clippy::too_many_arguments)]
pub fn card_message_body(
    html: &str,
    cards: &[OutCard],
    display_name: &str,
    client_message_id: &str,
    subject: Option<&str>,
    mentions: &[Mention],
    qtd_msgs: Option<&str>,
    delivery: &Delivery,
) -> Value {
    let html = if html.trim().is_empty() && !cards.is_empty() {
        "<p>&nbsp;</p>"
    } else {
        html
    };
    let mut b = outgoing_body(
        html,
        display_name,
        client_message_id,
        subject,
        mentions,
        qtd_msgs,
    );
    apply_delivery(&mut b, delivery, client_message_id);
    if !cards.is_empty() {
        if !b["properties"].is_object() {
            b["properties"] = json!({});
        }
        // A JSON *string*, like `mentions` and an announcement's `cards`.
        b["properties"]["cards"] =
            Value::String(cards_property(cards, client_message_id).to_string());
    }
    b
}

impl Client {
    /// Sends a message with cards under a caller-chosen client message id (the outbox retries
    /// with the same id).
    #[allow(clippy::too_many_arguments)]
    pub async fn send_card_message_as(
        &self,
        conversation_id: &str,
        html: &str,
        cards: &[OutCard],
        display_name: &str,
        subject: Option<&str>,
        mentions: &[Mention],
        qtd_msgs: Option<&str>,
        delivery: &Delivery,
        client_message_id: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/v1/users/ME/conversations/{}/messages",
            self.chat_service().await,
            encode_conversation(conversation_id)
        );
        let body = card_message_body(
            html,
            cards,
            display_name,
            client_message_id,
            subject,
            mentions,
            qtd_msgs,
            delivery,
        );
        self.json("send", Method::POST, &url, Some(&body))
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mentions::MentionKind;

    fn adaptive() -> OutCard {
        OutCard {
            app_id: "app-1".into(),
            app_name: "Contoso Tickets".into(),
            content_type: "application/vnd.microsoft.card.adaptive".into(),
            content: json!({"type": "AdaptiveCard", "body": [{"type": "TextBlock", "text": "TCK-1"}]}),
        }
    }

    fn hero() -> OutCard {
        OutCard {
            app_id: "app-2".into(),
            app_name: "GIFs".into(),
            content_type: "application/vnd.microsoft.card.hero".into(),
            content: json!({"title": "Cat", "images": [{"url": "https://x.example/c.gif"}]}),
        }
    }

    fn cards_of(body: &Value) -> Vec<Value> {
        serde_json::from_str(body["properties"]["cards"].as_str().expect("a JSON string")).unwrap()
    }

    #[test]
    fn the_cards_are_a_json_string_with_the_app_and_a_card_id_each() {
        let b = card_message_body(
            "<p>look</p>",
            &[adaptive(), hero()],
            "Me",
            "1731",
            None,
            &[],
            None,
            &Delivery::Standard,
        );
        assert_eq!(b["content"], "<p>look</p>");
        assert_eq!(b["messagetype"], "RichText/Html");
        assert_eq!(b["clientmessageid"], "1731");
        let cards = cards_of(&b);
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0]["appId"], "app-1");
        assert_eq!(cards[0]["appName"], "Contoso Tickets");
        assert_eq!(cards[0]["cardClientId"], "card-1731-0");
        assert_eq!(
            cards[0]["contentType"],
            "application/vnd.microsoft.card.adaptive"
        );
        assert_eq!(cards[0]["content"]["type"], "AdaptiveCard");
        assert_eq!(cards[1]["cardClientId"], "card-1731-1");
        assert_eq!(cards[1]["content"]["title"], "Cat");
    }

    #[test]
    fn a_retry_builds_the_same_body() {
        let build = || {
            card_message_body(
                "<p>x</p>",
                &[adaptive()],
                "Me",
                "9",
                None,
                &[],
                None,
                &Delivery::Standard,
            )
        };
        assert_eq!(build(), build());
    }

    #[test]
    fn no_text_gives_a_blank_paragraph_and_no_cards_gives_a_plain_message() {
        let b = card_message_body(
            "  ",
            &[hero()],
            "Me",
            "1",
            None,
            &[],
            None,
            &Delivery::Standard,
        );
        assert_eq!(b["content"], "<p>&nbsp;</p>");
        let plain = card_message_body("", &[], "Me", "1", None, &[], None, &Delivery::Standard);
        assert_eq!(plain, outgoing_body("", "Me", "1", None, &[], None));
        assert!(plain["properties"].is_null(), "no cards property at all");
    }

    #[test]
    fn mentions_importance_and_subject_travel_with_the_cards() {
        let mention = Mention {
            item_id: 0,
            mri: "8:orgid:abc".into(),
            display_name: "Sam".into(),
            kind: MentionKind::Person,
        };
        let b = card_message_body(
            "<p>hi</p>",
            &[adaptive()],
            "Me",
            "5",
            Some("Topic"),
            &[mention],
            None,
            &Delivery::Important,
        );
        assert_eq!(b["properties"]["subject"], "Topic");
        assert_eq!(b["properties"]["importance"], "high");
        assert!(
            b["properties"]["mentions"]
                .as_str()
                .unwrap()
                .contains("8:orgid:abc")
        );
        assert_eq!(cards_of(&b).len(), 1);
    }

    #[test]
    fn a_quote_reply_keeps_its_qtd_msgs_next_to_the_cards() {
        let qtd = crate::chatsvc::quoted_messages_property("1731", "8:orgid:abc", 1731);
        let b = card_message_body(
            "<p>x</p>",
            &[hero()],
            "Me",
            "6",
            None,
            &[],
            Some(&qtd),
            &Delivery::Standard,
        );
        assert_eq!(b["properties"]["qtdMsgs"], qtd);
        assert_eq!(cards_of(&b).len(), 1);
    }

    #[test]
    fn the_stored_form_round_trips_and_drops_broken_entries() {
        let cards = vec![adaptive(), hero()];
        assert_eq!(cards_from_json(&cards_to_json(&cards)), cards);
        let broken = json!([
            {"appId": "", "contentType": "x", "content": {}},
            {"appId": "a", "contentType": "", "content": {}},
            {"appId": "a", "contentType": "t"},
        ])
        .to_string();
        let kept = cards_from_json(&broken);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].content, Value::Null);
        assert!(cards_from_json("not json").is_empty());
        assert!(cards_from_json("{}").is_empty());
    }

    #[test]
    fn the_cards_read_back_as_the_rows_an_incoming_card_gets() {
        // What the receiving side reads must round-trip what is sent.
        let b = card_message_body(
            "<p>x</p>",
            &[adaptive()],
            "Me",
            "3",
            None,
            &[],
            None,
            &Delivery::Standard,
        );
        let attachments: Value =
            serde_json::from_str(b["properties"]["cards"].as_str().unwrap()).unwrap();
        let rows = crate::chat_card::rows_for_attachments(&attachments, "");
        assert!(!rows.is_empty(), "the card is drawn");
    }
}
