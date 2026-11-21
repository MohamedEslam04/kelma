//! Video clips: "Record video clip" in the composer, and the clips other people send.
//!
//! Sending, **UNVERIFIED** (no capture of a Teams video clip being sent is in `docs/` or the
//! reference clones; nothing has been sent live): the MP4 (H.264/AAC) is uploaded to the media
//! service (AMS) as an object of [`VIDEO_OBJECT_TYPE`] whose bytes go to the [`VIDEO_CONTENT`]
//! slot, then a `RichText/Html` message carries one
//! `<video src=".../views/video" itemtype="http://schema.skype.com/AMSVideo" …>` tag and names the
//! object in `amsreferences`. That is what the mautrix-teams bridge sends for a video, and the
//! tag is what its parser reads back from Teams; the native Teams "video clip" may use another
//! shape (a `card.video` card, a `VideoMsg` preview image), which is why this stays UNVERIFIED
//! until `ironteams-cli ams-meta` is run on a clip Teams itself sent.
//!
//! Receiving: a clip is either such a `<video>` tag or a `card.video` card, which
//! [`crate::cards::attachments_html`] writes as a "🎬 Video" label and a `kelma-media:` link.
//! [`split_video`] takes both out of a message's HTML so the chat draws them as clip cards.

use crate::ams::AMS_VIEW_HOST;
use crate::chatsvc::encode_conversation;
use crate::client::Client;
use crate::error::Result;
use crate::mentions::outgoing_body;
use reqwest::Method;
use serde_json::{Value, json};

/// AMS object type of a video clip. UNVERIFIED: taken from the mautrix-teams bridge
/// (`sharing/videototranscode` makes AMS prepare the `video` and `thumbnail` views); not yet
/// confirmed against the metadata of a clip Teams sent (`ironteams-cli ams-meta`).
pub const VIDEO_OBJECT_TYPE: &str = "sharing/videototranscode";
/// Content slot a video object's bytes are written to. UNVERIFIED, as [`VIDEO_OBJECT_TYPE`].
pub const VIDEO_CONTENT: &str = "original";
/// `itemtype` of a video tag in a message.
pub const AMS_VIDEO: &str = "http://schema.skype.com/AMSVideo";
/// The longest clip the recorder makes, as in Teams.
pub const MAX_CLIP_MS: u64 = 60_000;

/// The URL a message points at for an uploaded video object.
pub fn video_url(object_id: &str) -> String {
    format!("https://{AMS_VIEW_HOST}/v1/objects/{object_id}/views/video")
}

/// The thumbnail view of a clip at `url` (`…/views/video` → `…/views/thumbnail`), as the
/// rusty-teams client shows Teams clips. `None` for anything that is not an AMS video view.
pub fn thumbnail_url(url: &str) -> Option<String> {
    let object = url.strip_suffix("/views/video")?;
    crate::ams::is_ams_url(url).then(|| format!("{object}/views/thumbnail"))
}

/// `PT12.40S`: a clip's length as the tag's `data-duration` states it.
pub fn iso_duration(duration_ms: u64) -> String {
    format!("PT{}.{:02}S", duration_ms / 1000, (duration_ms % 1000) / 10)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The message HTML of a clip at `url` (a video view URL).
pub fn video_html(url: &str, width: u32, height: u32, duration_ms: u64) -> String {
    format!(
        r#"<video src="{}" itemscope="" itemtype="{AMS_VIDEO}" width="{width}" height="{height}" data-duration="{}">Video clip</video>"#,
        esc(url),
        iso_duration(duration_ms)
    )
}

/// The request body of a clip message pointing at the video object `object_id`. UNVERIFIED (see
/// the module notes).
pub fn video_clip_message_body(
    object_id: &str,
    width: u32,
    height: u32,
    duration_ms: u64,
    display_name: &str,
    client_message_id: &str,
) -> Value {
    let html = video_html(&video_url(object_id), width, height, duration_ms);
    let mut b = outgoing_body(&html, display_name, client_message_id, None, &[], None);
    b["amsreferences"] = json!([object_id]);
    b
}

/// A video clip found in a message's HTML by [`split_video`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoClip {
    /// The AMS `views/video` URL.
    pub url: String,
    /// The length the message states (0 when it states none).
    pub duration_ms: u64,
    /// The picture size the tag states (0 when it states none).
    pub width: u32,
    pub height: u32,
}

/// `PT12.4S`, `PT1M5S`, `PT12S` in milliseconds (0 when it cannot be read).
fn parse_iso(d: &str) -> u64 {
    let Some(rest) = d.strip_prefix("PT") else {
        return 0;
    };
    let (mut ms, mut num) = (0f64, String::new());
    for c in rest.chars() {
        match c {
            'H' => ms += num.parse::<f64>().unwrap_or(0.0) * 3_600_000.0,
            'M' => ms += num.parse::<f64>().unwrap_or(0.0) * 60_000.0,
            'S' => ms += num.parse::<f64>().unwrap_or(0.0) * 1000.0,
            _ => {
                num.push(c);
                continue;
            }
        }
        num.clear();
    }
    ms.round() as u64
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let len = tag[at..].find('"')?;
    Some(&tag[at..at + len])
}

fn unesc(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

const CARD_LABEL: &str = "<p><b>🎬 Video";
const CARD_LINK: &str = "<p><a href=\"kelma-media:";
const CARD_END: &str = "\">▶ Play</a></p>";

/// Takes the clips out of `html`: AMS `<video>` tags and the "🎬 Video" lines that
/// [`crate::cards::attachments_html`] writes for `card.video` cards. Returns the HTML without them
/// and the clips in order; `None` when there is none, so other messages pay nothing.
pub fn split_video(html: &str) -> Option<(String, Vec<VideoClip>)> {
    if !html.contains("<video") && !html.contains(CARD_LABEL) {
        return None;
    }
    let (mut rest, mut clips) = (String::new(), Vec::new());
    let mut tail = html;
    loop {
        let tag = tail.find("<video");
        let card = tail.find(CARD_LABEL);
        let at = match (tag, card) {
            (Some(t), Some(c)) => t.min(c),
            (Some(t), None) => t,
            (None, Some(c)) => c,
            (None, None) => break,
        };
        if Some(at) == tag {
            let Some(open_end) = tail[at..].find('>') else {
                break;
            };
            let open = &tail[at..at + open_end];
            let close = tail[at..]
                .find("</video>")
                .map_or(at + open_end + 1, |c| at + c + "</video>".len());
            match attr(open, "src").filter(|_| open.contains(AMS_VIDEO)) {
                Some(src) => {
                    clips.push(VideoClip {
                        url: unesc(src),
                        duration_ms: attr(open, "data-duration").map_or(0, parse_iso),
                        width: attr(open, "width")
                            .and_then(|w| w.parse().ok())
                            .unwrap_or(0),
                        height: attr(open, "height")
                            .and_then(|h| h.parse().ok())
                            .unwrap_or(0),
                    });
                    rest.push_str(&tail[..at]);
                }
                // Some other video: left as it is.
                None => rest.push_str(&tail[..close]),
            }
            tail = &tail[close..];
            continue;
        }
        let label = &tail[at + CARD_LABEL.len()..];
        let Some(label_end) = label.find("</b></p>") else {
            break;
        };
        let after = &label[label_end + "</b></p>".len()..];
        let Some(link) = after.strip_prefix(CARD_LINK) else {
            rest.push_str(&tail[..at + CARD_LABEL.len()]);
            tail = label;
            continue;
        };
        let Some(link_end) = link.find(CARD_END) else {
            break;
        };
        let seconds = label[..label_end]
            .strip_prefix(" · ")
            .and_then(|d| d.split_once(':'))
            .and_then(|(m, s)| Some(m.parse::<u64>().ok()? * 60 + s.parse::<u64>().ok()?))
            .unwrap_or(0);
        clips.push(VideoClip {
            url: unesc(&link[..link_end]),
            duration_ms: seconds * 1000,
            width: 0,
            height: 0,
        });
        rest.push_str(&tail[..at]);
        tail = &link[link_end + CARD_END.len()..];
    }
    rest.push_str(tail);
    (!clips.is_empty()).then_some((rest, clips))
}

impl Client {
    /// Uploads a recorded clip (H.264/AAC in MP4) visible to `conversation_id` and returns the AMS
    /// object id. UNVERIFIED object type and slot (see [`VIDEO_OBJECT_TYPE`]).
    pub async fn upload_video_clip(
        &self,
        conversation_id: &str,
        filename: &str,
        data: Vec<u8>,
    ) -> Result<String> {
        self.upload_object(
            conversation_id,
            filename,
            VIDEO_OBJECT_TYPE,
            VIDEO_CONTENT,
            data,
        )
        .await
    }

    /// Sends a clip message for the uploaded video object `object_id`. UNVERIFIED body (see the
    /// module notes). A retry with the same `client_message_id` does not post a second message.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_video_clip_as(
        &self,
        conversation_id: &str,
        object_id: &str,
        size: (u32, u32),
        duration_ms: u64,
        display_name: &str,
        client_message_id: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/v1/users/ME/conversations/{}/messages",
            self.chat_service().await,
            encode_conversation(conversation_id)
        );
        let body = video_clip_message_body(
            object_id,
            size.0,
            size.1,
            duration_ms,
            display_name,
            client_message_id,
        );
        self.json("send_video_clip", Method::POST, &url, Some(&body))
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::attachments_html;

    #[test]
    fn the_body_has_one_ams_video_tag_and_names_the_object() {
        let b = video_clip_message_body("0-x-y", 640, 360, 12_400, "Me", "1795092143150");
        assert_eq!(b["messagetype"], "RichText/Html");
        assert_eq!(b["clientmessageid"], "1795092143150");
        assert_eq!(b["amsreferences"], json!(["0-x-y"]));
        let content = b["content"].as_str().unwrap();
        assert!(content.contains(&format!("src=\"{}\"", video_url("0-x-y"))));
        assert!(content.contains("itemtype=\"http://schema.skype.com/AMSVideo\""));
        assert!(content.contains("width=\"640\" height=\"360\""));
        assert!(content.contains("data-duration=\"PT12.40S\""));
    }

    #[test]
    fn our_own_tag_reads_back_as_a_clip() {
        let html = format!(
            "<p>hi</p>{}",
            video_html(&video_url("0-x-y"), 640, 360, 9_000)
        );
        let (rest, clips) = split_video(&html).unwrap();
        assert_eq!(rest, "<p>hi</p>");
        assert_eq!(
            clips,
            [VideoClip {
                url: video_url("0-x-y"),
                duration_ms: 9_000,
                width: 640,
                height: 360,
            }]
        );
    }

    #[test]
    fn a_video_card_reads_back_as_a_clip() {
        let v = json!([{"contentType": "application/vnd.microsoft.card.video", "content": {"duration": "PT1M5S", "media": [{"url": "https://h/v1/objects/0-d/views/video"}]}}]);
        let html = attachments_html(&v).unwrap();
        let (rest, clips) = split_video(&html).unwrap();
        assert_eq!(rest, "");
        assert_eq!(clips[0].url, "https://h/v1/objects/0-d/views/video");
        assert_eq!(clips[0].duration_ms, 65_000);
    }

    #[test]
    fn other_messages_and_other_videos_are_left_alone() {
        assert!(split_video("<p>no clip</p>").is_none());
        assert!(split_video("<video src=\"https://x/a.mp4\">a</video>").is_none());
        // A voice message is not a clip.
        assert!(split_video("<p><b>🎤 Voice message · 0:05</b></p>").is_none());
    }

    #[test]
    fn durations_and_thumbnails() {
        assert_eq!(parse_iso("PT2.99S"), 2_990);
        assert_eq!(parse_iso("PT1M5S"), 65_000);
        assert_eq!(parse_iso("nonsense"), 0);
        assert_eq!(iso_duration(60_000), "PT60.00S");
        assert_eq!(
            thumbnail_url("https://eu-api.asm.skype.com/v1/objects/0-a/views/video").as_deref(),
            Some("https://eu-api.asm.skype.com/v1/objects/0-a/views/thumbnail")
        );
        assert_eq!(
            thumbnail_url("https://evil.example/v1/objects/a/views/video"),
            None
        );
    }
}
