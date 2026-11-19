//! Delivery options of a message: Standard, Important, Urgent and the channel Announcement.
//!
//! Teams web ("Set delivery options" in the composer) sends these as message properties:
//!
//! - **Importance** is `properties.importance`: `""` (Standard), `"high"` (Important, shown with
//!   a red "IMPORTANT!" label) or `"urgent"` (Urgent; the receiving client repeats the
//!   notification until the message is acknowledged). Incoming messages may also carry the
//!   numeric forms 2 and 3.
//! - **Announcement** (channel posts only) is a headline in `properties.title` plus one card in
//!   `properties.cards` with `contentType` `application/vnd.microsoft.teams.messaging-announcementBanner`.
//!   The card content holds either a `colorTheme` (index `"0"`..`"11"` of the banner palette) or
//!   `imageData` (`origImage` and `croppedImage`, each `{src, id?, imageWidth, imageHeight,
//!   imageAltText}`) for a background picture.
//!
//! Sources: Teams web client JS bundle (message serializer, announcement composer and renderer).

use crate::chatsvc::{encode_conversation, send_body};
use crate::client::Client;
use crate::error::{Error, Result};
use crate::time::now_ms;
use reqwest::Method;
use serde_json::{Value, json};

/// `properties.importance` of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Importance {
    #[default]
    Standard,
    Important,
    Urgent,
}

impl Importance {
    /// The value Teams web writes to `properties.importance`.
    pub fn wire(self) -> &'static str {
        match self {
            Importance::Standard => "",
            Importance::Important => "high",
            Importance::Urgent => "urgent",
        }
    }

    /// Reads a `properties.importance` value (string or number). Anything unknown is Standard.
    pub fn from_wire(v: Option<&Value>) -> Self {
        match v {
            Some(Value::String(s)) => match s.as_str() {
                "high" | "2" => Importance::Important,
                "urgent" | "3" => Importance::Urgent,
                _ => Importance::Standard,
            },
            Some(Value::Number(n)) => match n.as_i64() {
                Some(2) => Importance::Important,
                Some(3) => Importance::Urgent,
                _ => Importance::Standard,
            },
            _ => Importance::Standard,
        }
    }

    /// Reads the importance of a message from its `properties` object.
    pub fn from_properties(props: Option<&Value>) -> Self {
        Self::from_wire(props.and_then(|p| p.get("importance")))
    }

    /// True for a message the user should be reminded of until they react to it.
    pub fn is_urgent(self) -> bool {
        self == Importance::Urgent
    }
}

/// `contentType` of the announcement card.
pub const ANNOUNCEMENT_CONTENT_TYPE: &str =
    "application/vnd.microsoft.teams.messaging-announcementBanner";
/// `appId` of the announcement card.
pub const ANNOUNCEMENT_APP_ID: &str = "announcement-card";
/// How many banner colours the composer offers (`colorTheme` `"0"` to `"11"`).
pub const THEME_COUNT: u8 = 12;
/// Size Teams web crops a banner image to.
pub const BANNER_WIDTH: u32 = 1317;
pub const BANNER_HEIGHT: u32 = 366;

/// A background picture of an announcement. The picture must already be uploaded to the media
/// service (the URLs are its `imgo`/`imgpsh_fullsize` URLs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BannerImage {
    /// The full picture.
    pub original_url: String,
    /// The picture cropped to the banner.
    pub cropped_url: String,
    pub cropped_width: Option<u32>,
    pub cropped_height: Option<u32>,
    pub alt_text: Option<String>,
    /// The media service object id; sent in `amsreferences` so the picture stays attached.
    pub ams_id: Option<String>,
    /// A picture from Teams' own illustration set.
    pub is_teams_illustration: bool,
}

impl BannerImage {
    /// An uploaded picture used for both the original and the cropped banner, at the standard
    /// banner size.
    pub fn new(url: &str, alt_text: &str, ams_id: Option<&str>) -> Self {
        BannerImage {
            original_url: url.to_string(),
            cropped_url: url.to_string(),
            cropped_width: Some(BANNER_WIDTH),
            cropped_height: Some(BANNER_HEIGHT),
            alt_text: Some(alt_text.to_string()),
            ams_id: ams_id.map(str::to_string),
            is_teams_illustration: false,
        }
    }
}

/// The headline and background of a channel announcement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announcement {
    /// The headline shown on the banner.
    pub title: String,
    /// Banner colour index (`"0"`..`"11"`), when the banner has no picture.
    pub color_theme: Option<String>,
    /// Background picture, when the banner has one.
    pub image: Option<BannerImage>,
}

impl Announcement {
    /// A headline on a coloured banner. A theme outside the palette falls back to the first
    /// colour, as the Teams composer does.
    pub fn with_theme(title: &str, theme: u8) -> Result<Self> {
        let theme = if theme < THEME_COUNT { theme } else { 0 };
        Ok(Announcement {
            title: required_title(title)?,
            color_theme: Some(theme.to_string()),
            image: None,
        })
    }

    /// A headline on a picture. The headline may be empty when the picture says it all.
    pub fn with_image(title: &str, image: BannerImage) -> Self {
        Announcement {
            title: title.trim().to_string(),
            color_theme: None,
            image: Some(image),
        }
    }

    /// The card `content` object.
    fn card_content(&self) -> Value {
        if let Some(img) = &self.image {
            let size = |w: Option<u32>, h: Option<u32>, alt: &Option<String>, src: &str| {
                let mut o = json!({ "src": src });
                if let Some(w) = w {
                    o["imageWidth"] = json!(w);
                }
                if let Some(h) = h {
                    o["imageHeight"] = json!(h);
                }
                if let Some(a) = alt {
                    o["imageAltText"] = json!(a);
                }
                if let Some(id) = &img.ams_id {
                    o["id"] = json!(id);
                }
                o
            };
            let mut data = json!({
                "origImage": size(None, None, &None, &img.original_url),
                "croppedImage": size(
                    img.cropped_width,
                    img.cropped_height,
                    &img.alt_text,
                    &img.cropped_url,
                ),
            });
            if img.is_teams_illustration {
                data["isTeamsIllustration"] = json!(true);
            }
            json!({ "imageData": data })
        } else {
            json!({ "colorTheme": self.color_theme.clone().unwrap_or_else(|| "0".to_string()) })
        }
    }
}

fn required_title(title: &str) -> Result<String> {
    let t = title.trim();
    if t.is_empty() {
        return Err(Error::Protocol("an announcement needs a headline".into()));
    }
    Ok(t.to_string())
}

/// What a message is delivered as.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Delivery {
    #[default]
    Standard,
    Important,
    Urgent,
    /// A channel announcement (only meaningful for a new channel post).
    Announcement(Announcement),
}

impl Delivery {
    /// The importance this option sends.
    pub fn importance(&self) -> Importance {
        match self {
            Delivery::Important => Importance::Important,
            Delivery::Urgent => Importance::Urgent,
            _ => Importance::Standard,
        }
    }
}

/// Adds the delivery option to a send body (from [`send_body`] or any other builder). Standard
/// adds nothing, so a plain message stays exactly what it was.
pub fn apply_delivery(body: &mut Value, delivery: &Delivery, client_message_id: &str) {
    let ensure_props = |b: &mut Value| {
        if !b["properties"].is_object() {
            b["properties"] = json!({});
        }
    };
    match delivery {
        Delivery::Standard => {}
        Delivery::Important | Delivery::Urgent => {
            ensure_props(body);
            body["properties"]["importance"] = json!(delivery.importance().wire());
        }
        Delivery::Announcement(a) => {
            ensure_props(body);
            body["properties"]["title"] = json!(a.title);
            let card = json!([{
                "appId": ANNOUNCEMENT_APP_ID,
                "cardClientId": format!("{ANNOUNCEMENT_APP_ID}-{client_message_id}"),
                "contentType": ANNOUNCEMENT_CONTENT_TYPE,
                "content": a.card_content(),
            }]);
            // `properties.cards` is a JSON *string*, like `mentions` and `links`.
            body["properties"]["cards"] = Value::String(card.to_string());
            if let Some(id) = a.image.as_ref().and_then(|i| i.ams_id.as_ref()) {
                body["amsreferences"] = json!([id]);
            }
        }
    }
}

/// Request body for sending a message with a delivery option.
pub fn delivery_body(
    html: &str,
    display_name: &str,
    client_message_id: &str,
    subject: Option<&str>,
    delivery: &Delivery,
) -> Value {
    let mut b = send_body(html, display_name, client_message_id, subject);
    apply_delivery(&mut b, delivery, client_message_id);
    b
}

/// [`delivery_body`] for a queued message that may also carry mentions and the `qtdMsgs` of a
/// quote reply (see [`crate::mentions::outgoing_body`]).
pub fn outgoing_delivery_body(
    html: &str,
    display_name: &str,
    client_message_id: &str,
    subject: Option<&str>,
    mentions: &[crate::mentions::Mention],
    qtd_msgs: Option<&str>,
    delivery: &Delivery,
) -> Value {
    let mut b = crate::mentions::outgoing_body(
        html,
        display_name,
        client_message_id,
        subject,
        mentions,
        qtd_msgs,
    );
    apply_delivery(&mut b, delivery, client_message_id);
    b
}

/// A list value that may arrive as JSON or as a JSON-encoded string.
fn json_or_string(v: Option<&Value>) -> Option<Value> {
    match v? {
        Value::String(s) if !s.is_empty() => serde_json::from_str(s).ok(),
        Value::String(_) | Value::Null => None,
        other => Some(other.clone()),
    }
}

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for p in path {
        cur = cur.get(p)?;
    }
    cur.as_str().filter(|s| !s.is_empty())
}

fn u32_at(v: &Value, path: &[&str]) -> Option<u32> {
    let mut cur = v;
    for p in path {
        cur = cur.get(p)?;
    }
    cur.as_u64().and_then(|n| u32::try_from(n).ok())
}

/// Reads the announcement of a message from its `properties` object: the headline from `title`
/// and the banner from the announcement card in `cards`. None when the message has no
/// announcement card.
pub fn parse_announcement(props: Option<&Value>) -> Option<Announcement> {
    let props = props?;
    let cards = json_or_string(props.get("cards"))?;
    let card = cards.as_array()?.iter().find(|c| {
        c.get("contentType").and_then(Value::as_str) == Some(ANNOUNCEMENT_CONTENT_TYPE)
    })?;
    // The card content is an object on the wire; tolerate a JSON string too (draft form).
    let content = match card.get("content")? {
        Value::String(s) => serde_json::from_str::<Value>(s).ok()?,
        other => other.clone(),
    };
    let image = content.get("imageData").and_then(|data| {
        let cropped_url = str_at(data, &["croppedImage", "src"]).map(str::to_string);
        let original_url = str_at(data, &["origImage", "src"]).map(str::to_string);
        let (cropped_url, original_url) = match (cropped_url, original_url) {
            (Some(c), Some(o)) => (c, o),
            (Some(c), None) => (c.clone(), c),
            (None, Some(o)) => (o.clone(), o),
            (None, None) => return None,
        };
        Some(BannerImage {
            original_url,
            cropped_url,
            cropped_width: u32_at(data, &["croppedImage", "imageWidth"]),
            cropped_height: u32_at(data, &["croppedImage", "imageHeight"]),
            alt_text: str_at(data, &["croppedImage", "imageAltText"]).map(str::to_string),
            ams_id: str_at(data, &["croppedImage", "id"]).map(str::to_string),
            is_teams_illustration: data
                .get("isTeamsIllustration")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    });
    // The theme is read from `imageData.colorTheme` first, then from the content itself.
    let color_theme = ["imageData", ""]
        .iter()
        .find_map(|k| {
            let v = if k.is_empty() {
                content.get("colorTheme")
            } else {
                content.get(k).and_then(|d| d.get("colorTheme"))
            };
            v.and_then(Value::as_str).filter(|s| !s.is_empty())
        })
        .map(str::to_string);
    Some(Announcement {
        title: props
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        color_theme,
        image,
    })
}

impl Client {
    /// Sends a queued message with a delivery option, a caller-chosen client message id (the
    /// outbox retries with the same id), and the mentions and `qtdMsgs` it may carry.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_outgoing_delivery_as(
        &self,
        conversation_id: &str,
        html: &str,
        display_name: &str,
        subject: Option<&str>,
        mentions: &[crate::mentions::Mention],
        qtd_msgs: Option<&str>,
        delivery: &Delivery,
        client_message_id: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/v1/users/ME/conversations/{}/messages",
            self.chat_service().await,
            encode_conversation(conversation_id)
        );
        let body = outgoing_delivery_body(
            html,
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

    /// Sends an HTML message with a delivery option. Returns the client message id used.
    /// Announcements only make sense as a new post in a channel.
    pub async fn send_message_delivery(
        &self,
        conversation_id: &str,
        html: &str,
        display_name: &str,
        subject: Option<&str>,
        delivery: &Delivery,
    ) -> Result<String> {
        let cmid = now_ms().to_string();
        let url = format!(
            "{}/v1/users/ME/conversations/{}/messages",
            self.chat_service().await,
            encode_conversation(conversation_id)
        );
        let body = delivery_body(html, display_name, &cmid, subject, delivery);
        self.json("send", Method::POST, &url, Some(&body)).await?;
        Ok(cmid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_important_queued_message_keeps_its_id_mentions_and_quote() {
        use crate::mentions::{Mention, MentionKind};
        let m = Mention {
            item_id: 0,
            mri: "8:orgid:a".into(),
            display_name: "A".into(),
            kind: MentionKind::Person,
        };
        let b = outgoing_delivery_body(
            "<p>x</p>",
            "Me",
            "1795",
            None,
            &[m],
            Some("[{\"messageId\":1}]"),
            &Delivery::Important,
        );
        assert_eq!(b["clientmessageid"], "1795");
        assert_eq!(b["properties"]["importance"], "high");
        assert!(b["properties"]["mentions"].is_string());
        assert!(b["properties"]["qtdMsgs"].is_string());
        // Standard adds nothing.
        let plain =
            outgoing_delivery_body("<p>x</p>", "Me", "1", None, &[], None, &Delivery::Standard);
        assert!(
            plain
                .get("properties")
                .is_none_or(|p| p.get("importance").is_none())
        );
    }

    #[test]
    fn importance_wire_values() {
        assert_eq!(Importance::Standard.wire(), "");
        assert_eq!(Importance::Important.wire(), "high");
        assert_eq!(Importance::Urgent.wire(), "urgent");
    }

    #[test]
    fn importance_reads_strings_and_numbers() {
        let read = |v: Value| Importance::from_wire(Some(&v));
        assert_eq!(read(json!("high")), Importance::Important);
        assert_eq!(read(json!("urgent")), Importance::Urgent);
        assert_eq!(read(json!(2)), Importance::Important);
        assert_eq!(read(json!(3)), Importance::Urgent);
        assert_eq!(read(json!("")), Importance::Standard);
        assert_eq!(read(json!("normal")), Importance::Standard);
        assert_eq!(read(json!("shouting")), Importance::Standard);
        assert_eq!(read(json!(null)), Importance::Standard);
        assert_eq!(Importance::from_wire(None), Importance::Standard);
        assert_eq!(Importance::from_properties(None), Importance::Standard);
        assert_eq!(
            Importance::from_properties(Some(&json!({"importance": "high"}))),
            Importance::Important
        );
    }

    #[test]
    fn only_urgent_is_urgent() {
        assert!(Importance::Urgent.is_urgent());
        assert!(!Importance::Important.is_urgent());
        assert!(!Importance::Standard.is_urgent());
    }

    #[test]
    fn standard_body_is_the_plain_send_body() {
        let plain = send_body("<p>hi</p>", "Me", "1", None);
        let b = delivery_body("<p>hi</p>", "Me", "1", None, &Delivery::Standard);
        assert_eq!(b, plain);
        assert!(b.get("properties").is_none());
    }

    #[test]
    fn important_and_urgent_set_the_importance_property() {
        let b = delivery_body("<p>hi</p>", "Me", "1", None, &Delivery::Important);
        assert_eq!(b["properties"]["importance"], "high");
        assert_eq!(b["content"], "<p>hi</p>");
        assert_eq!(b["messagetype"], "RichText/Html");
        let b = delivery_body("<p>hi</p>", "Me", "1", None, &Delivery::Urgent);
        assert_eq!(b["properties"]["importance"], "urgent");
        assert_eq!(b["clientmessageid"], "1");
    }

    #[test]
    fn importance_keeps_the_subject() {
        let b = delivery_body("x", "Me", "1", Some("Release"), &Delivery::Urgent);
        assert_eq!(b["properties"]["subject"], "Release");
        assert_eq!(b["properties"]["importance"], "urgent");
    }

    #[test]
    fn apply_delivery_works_on_a_body_without_properties() {
        let mut b = json!({"content": "x"});
        apply_delivery(&mut b, &Delivery::Important, "1");
        assert_eq!(b["properties"]["importance"], "high");
        let mut b = json!({"content": "x"});
        apply_delivery(&mut b, &Delivery::Standard, "1");
        assert!(b.get("properties").is_none());
    }

    #[test]
    fn theme_announcement_body() {
        let a = Announcement::with_theme("  Town hall  ", 3).unwrap();
        let b = delivery_body(
            "<p>Join us</p>",
            "Me",
            "42",
            None,
            &Delivery::Announcement(a),
        );
        assert_eq!(b["properties"]["title"], "Town hall");
        assert!(b.get("amsreferences").is_none());
        // cards is a JSON string holding one announcement card
        let cards: Value =
            serde_json::from_str(b["properties"]["cards"].as_str().unwrap()).unwrap();
        let card = &cards[0];
        assert_eq!(card["appId"], "announcement-card");
        assert_eq!(card["cardClientId"], "announcement-card-42");
        assert_eq!(
            card["contentType"],
            "application/vnd.microsoft.teams.messaging-announcementBanner"
        );
        assert_eq!(card["content"], json!({"colorTheme": "3"}));
    }

    #[test]
    fn theme_out_of_range_falls_back_to_the_first() {
        let a = Announcement::with_theme("x", 99).unwrap();
        assert_eq!(a.color_theme.as_deref(), Some("0"));
        let a = Announcement::with_theme("x", 11).unwrap();
        assert_eq!(a.color_theme.as_deref(), Some("11"));
    }

    #[test]
    fn theme_announcement_needs_a_headline() {
        assert!(Announcement::with_theme("", 1).is_err());
        assert!(Announcement::with_theme("   ", 1).is_err());
    }

    #[test]
    fn image_announcement_body() {
        let img = BannerImage::new(
            "https://as/img/0-x/imgpsh_fullsize",
            "A sunset",
            Some("0-x"),
        );
        let a = Announcement::with_image("Offsite", img);
        let b = delivery_body("<p>x</p>", "Me", "7", None, &Delivery::Announcement(a));
        assert_eq!(b["properties"]["title"], "Offsite");
        assert_eq!(b["amsreferences"], json!(["0-x"]));
        let cards: Value =
            serde_json::from_str(b["properties"]["cards"].as_str().unwrap()).unwrap();
        let data = &cards[0]["content"]["imageData"];
        assert_eq!(
            data["origImage"]["src"],
            "https://as/img/0-x/imgpsh_fullsize"
        );
        assert_eq!(data["origImage"]["id"], "0-x");
        assert_eq!(
            data["croppedImage"]["src"],
            "https://as/img/0-x/imgpsh_fullsize"
        );
        assert_eq!(data["croppedImage"]["imageWidth"], 1317);
        assert_eq!(data["croppedImage"]["imageHeight"], 366);
        assert_eq!(data["croppedImage"]["imageAltText"], "A sunset");
        assert!(data.get("isTeamsIllustration").is_none());
        assert!(cards[0]["content"].get("colorTheme").is_none());
    }

    #[test]
    fn image_announcement_may_have_no_headline() {
        let a = Announcement::with_image("  ", BannerImage::new("u", "", None));
        assert_eq!(a.title, "");
        let b = delivery_body("x", "Me", "1", None, &Delivery::Announcement(a));
        assert_eq!(b["properties"]["title"], "");
        assert!(b.get("amsreferences").is_none());
    }

    #[test]
    fn announcement_keeps_the_subject() {
        let a = Announcement::with_theme("Hi", 0).unwrap();
        let b = delivery_body("x", "Me", "1", Some("S"), &Delivery::Announcement(a));
        assert_eq!(b["properties"]["subject"], "S");
        assert_eq!(b["properties"]["title"], "Hi");
    }

    #[test]
    fn delivery_importance() {
        assert_eq!(Delivery::Standard.importance(), Importance::Standard);
        assert_eq!(Delivery::Important.importance(), Importance::Important);
        assert_eq!(Delivery::Urgent.importance(), Importance::Urgent);
        let a = Announcement::with_theme("x", 0).unwrap();
        assert_eq!(Delivery::Announcement(a).importance(), Importance::Standard);
    }

    #[test]
    fn announcement_round_trips_through_the_parser() {
        for a in [
            Announcement::with_theme("Town hall", 5).unwrap(),
            Announcement::with_image(
                "Offsite",
                BannerImage::new("https://u/i", "alt", Some("0-a")),
            ),
        ] {
            let b = delivery_body("x", "Me", "9", None, &Delivery::Announcement(a.clone()));
            assert_eq!(parse_announcement(b.get("properties")), Some(a));
        }
    }

    #[test]
    fn parses_a_card_whose_content_is_an_object_or_a_string() {
        let card = |content: Value| {
            json!({"title": "T", "cards": json!([{
                "appId": "announcement-card",
                "contentType": ANNOUNCEMENT_CONTENT_TYPE,
                "content": content,
            }]).to_string()})
        };
        let obj = card(json!({"colorTheme": "2"}));
        let s = card(json!(r#"{"colorTheme":"2"}"#));
        let want = Announcement {
            title: "T".into(),
            color_theme: Some("2".into()),
            image: None,
        };
        assert_eq!(parse_announcement(Some(&obj)), Some(want.clone()));
        assert_eq!(parse_announcement(Some(&s)), Some(want));
    }

    #[test]
    fn parses_the_theme_inside_image_data_first() {
        let p = json!({"title": "T", "cards": [{
            "contentType": ANNOUNCEMENT_CONTENT_TYPE,
            "content": {"colorTheme": "1", "imageData": {"colorTheme": "4"}},
        }]});
        let a = parse_announcement(Some(&p)).unwrap();
        assert_eq!(a.color_theme.as_deref(), Some("4"));
        assert!(a.image.is_none());
    }

    #[test]
    fn parses_an_illustration_and_a_cropped_only_image() {
        let p = json!({"title": "", "cards": [{
            "contentType": ANNOUNCEMENT_CONTENT_TYPE,
            "content": {"imageData": {
                "croppedImage": {"src": "https://u/c", "imageWidth": 800, "imageHeight": 200},
                "isTeamsIllustration": true,
            }},
        }]});
        let a = parse_announcement(Some(&p)).unwrap();
        let img = a.image.unwrap();
        assert_eq!(img.cropped_url, "https://u/c");
        assert_eq!(img.original_url, "https://u/c");
        assert_eq!(img.cropped_width, Some(800));
        assert_eq!(img.cropped_height, Some(200));
        assert!(img.is_teams_illustration);
        assert_eq!(a.title, "");
    }

    #[test]
    fn no_announcement_without_the_card() {
        assert_eq!(parse_announcement(None), None);
        assert_eq!(parse_announcement(Some(&json!({}))), None);
        assert_eq!(parse_announcement(Some(&json!({"title": "T"}))), None);
        // a title alone is not an announcement; only the card makes one
        let other = json!({"cards": [{"contentType": "application/vnd.microsoft.card.adaptive", "content": {}}]});
        assert_eq!(parse_announcement(Some(&other)), None);
        assert_eq!(
            parse_announcement(Some(&json!({"cards": "not json"}))),
            None
        );
        assert_eq!(parse_announcement(Some(&json!({"cards": ""}))), None);
    }
}
