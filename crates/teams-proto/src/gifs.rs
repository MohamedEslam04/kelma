//! GIF search (the composer's GIF button) through Teams' Giphy input extension, and the message
//! HTML Teams sends for a picked GIF.
//!
//! Request from rusty-teams `crates/chatsvc/src/gifs.rs` (MIT): `POST MT/beta/commands/
//! com.microsoft.teamspace.inputextension.giphy/execute?version=1.0` with a list of
//! `{name, value}` parameters; an empty query gives the trending GIFs. A 403 means GIFs are off
//! for the user. The message shape was read from GIFs received in Teams: an `<img>` with
//! `itemtype="http://schema.skype.com/Giphy"`, its title as `alt` and its size.

use crate::client::Client;
use crate::error::{Error, Result};
use reqwest::Method;
use serde_json::{Value, json};

const PATH: &str =
    "/beta/commands/com.microsoft.teamspace.inputextension.giphy/execute?version=1.0";

/// One search result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gif {
    /// The animated GIF that is sent.
    pub url: String,
    /// A smaller still or animated image for the picker.
    pub preview_url: String,
    pub title: String,
    pub width: u32,
    pub height: u32,
}

/// The search parameters (G rated, 27 results).
pub fn search_body(query: &str, lang: &str) -> Value {
    json!([
        {"name": "query", "value": query},
        {"name": "rating", "value": "g"},
        {"name": "limit", "value": "27"},
        {"name": "lang", "value": lang},
    ])
}

/// The results of a search answer.
pub fn parse_gifs(body: &Value) -> Vec<Gif> {
    body.pointer("/result/0/result")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let field = |name: &str| {
                entry
                    .get("fieldValues")?
                    .as_array()?
                    .iter()
                    .find(|f| f.get("fieldName").and_then(Value::as_str) == Some(name))?
                    .get("fieldValue")?
                    .as_str()
            };
            let number = |name: &str| field(name)?.parse::<u32>().ok().filter(|n| *n > 0);
            let url = entry.get("composeValue")?.as_str()?.to_string();
            if !url.starts_with("https://") {
                return None;
            }
            Some(Gif {
                preview_url: field("previewImageFW")
                    .or_else(|| field("previewImage"))
                    .map_or_else(|| url.clone(), str::to_string),
                title: field("title").unwrap_or_default().to_string(),
                width: number("width")?,
                height: number("height")?,
                url,
            })
        })
        .collect()
}

fn attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The message HTML for a picked GIF, as Teams sends it.
pub fn gif_html(g: &Gif) -> String {
    format!(
        r#"<p><img alt="{}" src="{}" itemtype="http://schema.skype.com/Giphy" width="{}" height="{}"></p>"#,
        attr(&g.title),
        attr(&g.url),
        g.width,
        g.height
    )
}

impl Client {
    /// Searches GIFs; an empty query gives the trending ones.
    pub async fn search_gifs(&self, query: &str, lang: &str) -> Result<Vec<Gif>> {
        let url = format!("{}{PATH}", self.middle_tier().await);
        match self
            .json(
                "gif search",
                Method::POST,
                &url,
                Some(&search_body(query, lang)),
            )
            .await
        {
            Ok(v) => Ok(parse_gifs(&v)),
            Err(e) if e.to_string().contains("HTTP 403") => Err(Error::Protocol(
                "GIFs are turned off for your account".into(),
            )),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_parse_with_their_preview_and_size() {
        let v = json!({"result": [{"result": [
            {"composeValue": "https://media1.giphy.com/media/a/giphy.gif",
             "fieldValues": [
                {"fieldName": "title", "fieldValue": "Happy Dance GIF"},
                {"fieldName": "width", "fieldValue": "480"},
                {"fieldName": "height", "fieldValue": "270"},
                {"fieldName": "previewImageFW", "fieldValue": "https://media1.giphy.com/media/a/200w.gif"}
             ]},
            {"composeValue": "javascript:alert(1)", "fieldValues": [
                {"fieldName": "width", "fieldValue": "1"}, {"fieldName": "height", "fieldValue": "1"}]},
            {"composeValue": "https://media1.giphy.com/media/b/giphy.gif", "fieldValues": []}
        ]}]});
        let g = parse_gifs(&v);
        assert_eq!(g.len(), 1, "only https results with a size");
        assert_eq!(g[0].title, "Happy Dance GIF");
        assert_eq!(
            g[0].preview_url,
            "https://media1.giphy.com/media/a/200w.gif"
        );
        assert_eq!((g[0].width, g[0].height), (480, 270));
    }

    #[test]
    fn a_gif_is_sent_as_a_giphy_image() {
        let g = Gif {
            url: "https://media1.giphy.com/media/a/giphy.gif".into(),
            preview_url: String::new(),
            title: "Say \"hi\" <3".into(),
            width: 480,
            height: 270,
        };
        assert_eq!(
            gif_html(&g),
            r#"<p><img alt="Say &quot;hi&quot; &lt;3" src="https://media1.giphy.com/media/a/giphy.gif" itemtype="http://schema.skype.com/Giphy" width="480" height="270"></p>"#
        );
    }
}
