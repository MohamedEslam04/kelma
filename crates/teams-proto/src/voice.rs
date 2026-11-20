//! Sending a voice message the way Teams mobile does.
//!
//! The recording (AAC in MP4, `.m4a`) is uploaded to the media service (AMS) as an audio object
//! that the conversation may read. The message is `RichText/Html` whose body is only an
//! `InputExtension` span; its `properties.cards` (a JSON *string*) holds one audio card,
//! `{cardClientId, contentType: "application/vnd.microsoft.card.audio", content: {duration,
//! media: [{url}]}}`, the `cardClientId` matching the span's `itemid`. `amsreferences` names the
//! object so it stays attached to the message.
//!
//! Evidence: the raw JSON of voice messages Teams mobile sent (body, card, the ISO-8601 whole
//! second `duration` and the `views/audio` URL; recorded in `docs/protocol.md`). Incoming, the
//! same card is read by [`crate::cards::attachments_html`].

use crate::ams::AMS_VIEW_HOST;
use crate::chatsvc::encode_conversation;
use crate::client::Client;
use crate::error::Result;
use crate::mentions::outgoing_body;
use reqwest::Method;
use serde_json::{Value, json};

/// The content type of a voice message card.
pub const AUDIO_CARD: &str = "application/vnd.microsoft.card.audio";

/// `PT15S`: a length in whole seconds, as the card states it (at least one second).
pub fn iso_seconds(duration_ms: u64) -> String {
    format!("PT{}S", (duration_ms.div_ceil(1000)).max(1))
}

/// The URL a voice message card points at for an uploaded audio object.
pub fn audio_url(object_id: &str) -> String {
    format!("https://{AMS_VIEW_HOST}/v1/objects/{object_id}/views/audio")
}

/// The card's id: 32 hex digits made from the client message id, so a retry sends the same one.
fn card_id(client_message_id: &str) -> String {
    let mut h: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    for b in client_message_id.bytes() {
        h = (h ^ u128::from(b)).wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
    }
    format!("{h:032x}")
}

/// The request body of a voice message pointing at the audio object `object_id`.
pub fn voice_message_body(
    object_id: &str,
    duration_ms: u64,
    display_name: &str,
    client_message_id: &str,
) -> Value {
    let id = card_id(client_message_id);
    let html = format!(
        r#"<div><span itemid="{id}" itemscope="" itemtype="http://schema.skype.com/InputExtension"></span></div>"#
    );
    let mut b = outgoing_body(&html, display_name, client_message_id, None, &[], None);
    if !b["properties"].is_object() {
        b["properties"] = json!({});
    }
    let cards = json!([{
        "cardClientId": id,
        "content": {
            "duration": iso_seconds(duration_ms),
            "media": [{ "url": audio_url(object_id) }],
        },
        "contentType": AUDIO_CARD,
    }]);
    b["properties"]["cards"] = Value::String(cards.to_string());
    b["amsreferences"] = json!([object_id]);
    b
}

impl Client {
    /// Sends a voice message whose recording is the uploaded audio object `object_id`. A retry
    /// with the same `client_message_id` does not post a second message.
    pub async fn send_voice_as(
        &self,
        conversation_id: &str,
        object_id: &str,
        duration_ms: u64,
        display_name: &str,
        client_message_id: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/v1/users/ME/conversations/{}/messages",
            self.chat_service().await,
            encode_conversation(conversation_id)
        );
        let body = voice_message_body(object_id, duration_ms, display_name, client_message_id);
        self.json("send_voice", Method::POST, &url, Some(&body))
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::{VoiceClip, attachments_html, split_voice};

    #[test]
    fn lengths_are_whole_seconds_rounded_up() {
        assert_eq!(iso_seconds(0), "PT1S");
        assert_eq!(iso_seconds(14_200), "PT15S");
        assert_eq!(iso_seconds(15_000), "PT15S");
        assert_eq!(iso_seconds(61_000), "PT61S");
    }

    #[test]
    fn the_body_is_what_teams_mobile_sends() {
        let b = voice_message_body("0-x-y", 14_200, "Me", "1795092143150");
        assert_eq!(b["messagetype"], "RichText/Html");
        assert_eq!(b["clientmessageid"], "1795092143150");
        assert_eq!(b["amsreferences"], json!(["0-x-y"]));
        let cards: Value =
            serde_json::from_str(b["properties"]["cards"].as_str().unwrap()).unwrap();
        let card = &cards[0];
        assert_eq!(card["contentType"], AUDIO_CARD);
        assert_eq!(card["content"]["duration"], "PT15S");
        assert_eq!(card["content"]["media"][0]["url"], audio_url("0-x-y"));
        // The span in the body names the card.
        let id = card["cardClientId"].as_str().unwrap();
        assert_eq!(id.len(), 32);
        assert!(
            b["content"]
                .as_str()
                .unwrap()
                .contains(&format!("itemid=\"{id}\""))
        );
        // The same message id gives the same card id (a retry), another one does not.
        let again = voice_message_body("0-x-y", 14_200, "Me", "1795092143150");
        assert_eq!(again["properties"]["cards"], b["properties"]["cards"]);
        let other = voice_message_body("0-x-y", 14_200, "Me", "1795092143151");
        assert_ne!(other["properties"]["cards"], b["properties"]["cards"]);
    }

    #[test]
    fn our_own_card_reads_back_as_a_voice_message() {
        let b = voice_message_body("0-x-y", 9_000, "Me", "1");
        let cards: Value =
            serde_json::from_str(b["properties"]["cards"].as_str().unwrap()).unwrap();
        let (_, clips) = split_voice(&attachments_html(&cards).unwrap()).unwrap();
        assert_eq!(
            clips,
            [VoiceClip {
                url: audio_url("0-x-y"),
                duration_ms: 9_000
            }]
        );
    }
}
