//! Link previews (plans/chat.md 9.7).
//!
//! Teams unfurls a link through the URL preview service (`urlp`): `GET v1/url/info?url=<url>`
//! answers the title, description, site name and thumbnail, and the thumbnail itself is served by
//! the same service. The service takes the skypetoken, which `http::headers_for` attaches to its
//! hosts only. A sent link keeps its preview in the message property `links`; the property shape
//! and the showable rule are adapted from rusty-teams (MIT, `core/src/links.rs`). Every URL we
//! fetch with credentials is checked against the service hosts first, so a link inside a
//! message can never receive a token.

use crate::chatsvc::encode_conversation;
use crate::client::Client;
use crate::error::{Error, Result};
use reqwest::Method;
use serde_json::{Value, json};
use std::net::Ipv4Addr;

/// The preview service. Teams reads the host from `regionGtms.urlp`; this one answers for every
/// region.
pub const URLP_BASE: &str = "https://urlp.asm.skype.com";

/// Largest preview image we download.
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

const HYPERLINK_TYPE: &str = "http://schema.skype.com/HyperLink";

/// Hosts whose links are tenant or Microsoft content, not web pages worth a preview card.
const INTERNAL_HOSTS: [&str; 8] = [
    "sharepoint.com",
    "onedrive.com",
    "1drv.ms",
    "teams.microsoft.com",
    "teams.cloud.microsoft",
    "teams.live.com",
    "office.com",
    "microsoft365.com",
];

/// What the service knows about a link.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkPreview {
    /// Where the link ends up after redirects (what a click opens).
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    /// The site's own name, usually its domain.
    pub site_name: Option<String>,
    /// Thumbnail on the preview service (240 px).
    pub image_url: Option<String>,
    pub image_width: Option<u32>,
    pub image_height: Option<u32>,
    /// The 480 px thumbnail.
    pub large_image_url: Option<String>,
    pub favicon_url: Option<String>,
    /// Status the service got from the page, such as 200 or 502.
    pub status: Option<u16>,
}

/// Which picture of a page the service serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Thumbnail,
    LargeThumbnail,
    Favicon,
}

impl LinkPreview {
    /// The domain to show on the card, without `www.`.
    pub fn domain(&self) -> String {
        let host = link_host(&self.url).unwrap_or_default();
        host.strip_prefix("www.").unwrap_or(&host).to_owned()
    }

    /// True when the card has at least two of image, title and description, as Teams requires.
    pub fn is_showable(&self) -> bool {
        [
            self.image_url.is_some(),
            self.title.is_some(),
            self.description.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count()
            >= 2
    }
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// `GET` URL of the info call for `link`.
pub fn info_url(base: &str, link: &str) -> String {
    format!(
        "{}/v1/url/info?url={}",
        base.trim_end_matches('/'),
        enc(link)
    )
}

/// Headers of a service call (besides auth). Teams sends one id three times.
pub fn request_headers(id: &str) -> Vec<(String, String)> {
    ["RequestId", "CorrelationId", "TransactionId"]
        .into_iter()
        .map(|k| (k.to_owned(), id.to_owned()))
        .collect()
}

/// A string that is present and not blank, with runs of whitespace folded to one space.
fn clean(v: &Value, key: &str) -> Option<String> {
    let folded = v
        .get(key)?
        .as_str()?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!folded.is_empty()).then_some(folded)
}

fn size(meta: Option<&Value>, key: &str) -> Option<u32> {
    meta?
        .get(key)?
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0)
}

/// Reads an `info` answer. Blank strings are no value, and whitespace in the title is folded.
pub fn parse_info(v: &Value) -> LinkPreview {
    let meta = v.get("thumbnail_meta");
    LinkPreview {
        url: clean(v, "target_url")
            .or_else(|| clean(v, "url"))
            .unwrap_or_default(),
        title: clean(v, "title"),
        description: clean(v, "description"),
        site_name: clean(v, "site"),
        image_url: clean(v, "thumbnail"),
        image_width: size(meta, "width"),
        image_height: size(meta, "height"),
        large_image_url: clean(v, "largethumbnail"),
        favicon_url: clean(v, "favicon"),
        status: match v.get("status_code") {
            Some(Value::String(s)) => s.parse().ok(),
            Some(Value::Number(n)) => n.as_u64().and_then(|n| u16::try_from(n).ok()),
            _ => None,
        },
    }
}

/// The service URL that serves `remote_image` (an image on a web page) as `kind`.
pub fn image_proxy_url(base: &str, kind: ImageKind, remote_image: &str) -> String {
    let kind = match kind {
        ImageKind::Thumbnail => "Thumbnail",
        ImageKind::LargeThumbnail => "LargeThumbnail",
        ImageKind::Favicon => "Favicon",
    };
    format!(
        "{}/v1/url/image/{kind}?url={}",
        base.trim_end_matches('/'),
        enc(remote_image)
    )
}

/// True when `url` is https on a host of the preview service, the only URLs that get the token.
pub fn is_service_url(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    let host = u.host_str().unwrap_or_default();
    u.scheme() == "https"
        && ((host == "urlp.asm.skype.com" && u.path().starts_with("/v1/url/"))
            || (host.ends_with(".asyncgw.teams.microsoft.com")
                && u.path().starts_with("/urlp/v1/url/")))
}

/// True for an http(s) link to the public web: not tenant content, an internal name or a
/// private address.
pub fn is_public_link(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    if !matches!(u.scheme(), "http" | "https") {
        return false;
    }
    match u.host() {
        Some(url::Host::Domain(host)) => {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            host.contains('.')
                && !INTERNAL_HOSTS
                    .iter()
                    .any(|i| host == *i || host.ends_with(&format!(".{i}")))
        }
        Some(url::Host::Ipv4(a)) => !is_private(a),
        _ => false,
    }
}

fn is_private(a: Ipv4Addr) -> bool {
    a.is_private() || a.is_loopback() || a.is_link_local() || a.is_unspecified()
}

fn link_host(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
}

/// The `links` message property that makes Teams show `preview` under a sent message.
pub fn links_property_json(requested_url: &str, preview: &LinkPreview) -> String {
    let mut p = serde_json::Map::new();
    if let Some(u) = &preview.image_url {
        p.insert("previewurl".into(), json!(u));
    }
    if let Some((width, height)) = preview.image_width.zip(preview.image_height) {
        p.insert(
            "previewmeta".into(),
            json!({"height": height, "width": width}),
        );
    }
    if let Some(t) = &preview.title {
        p.insert("title".into(), json!(t));
    }
    if let Some(d) = &preview.description {
        p.insert("description".into(), json!(d));
    }
    json!([{
        "@type": HYPERLINK_TYPE,
        "itemid": "0",
        "url": requested_url,
        "preview": p,
        "previewenabled": true,
    }])
    .to_string()
}

/// Reads the previews of a message from its `properties.links` (a JSON string or the list). Entries
/// with the preview switched off, outside the public web or with too little to show are left out.
pub fn parse_links_property(links: &Value) -> Vec<LinkPreview> {
    let list: Vec<Value> = match links {
        Value::String(s) => serde_json::from_str(s).unwrap_or_default(),
        Value::Array(a) => a.clone(),
        _ => Vec::new(),
    };
    list.iter().filter_map(entry_preview).collect()
}

fn entry_preview(entry: &Value) -> Option<LinkPreview> {
    if entry.get("previewenabled").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let url = entry.get("url")?.as_str()?;
    if !is_public_link(url) {
        return None;
    }
    let p = entry.get("preview").filter(|p| p.is_object())?;
    let meta = ["previewmeta", "previewMetadata"]
        .into_iter()
        .find_map(|k| p.get(k).filter(|m| m.is_object()));
    let preview = LinkPreview {
        url: url.to_owned(),
        title: clean(p, "title"),
        description: clean(p, "description"),
        image_url: clean(p, "previewurl").filter(|u| is_service_url(u)),
        image_width: size(meta, "width"),
        image_height: size(meta, "height"),
        ..LinkPreview::default()
    };
    preview.is_showable().then_some(preview)
}

/// URL and body of the call that sets the `links` property of a message.
pub fn set_links_request(
    chat_service: &str,
    conversation_id: &str,
    message_id: &str,
    requested_url: &str,
    preview: &LinkPreview,
) -> (String, Value) {
    let url = format!(
        "{}/v1/users/ME/conversations/{}/messages/{message_id}/properties?name=links",
        chat_service.trim_end_matches('/'),
        encode_conversation(conversation_id)
    );
    (
        url,
        json!({"links": links_property_json(requested_url, preview)}),
    )
}

/// What the Teams service knows about `link`. A page it cannot reach still answers, with
/// nothing to show: check [`LinkPreview::is_showable`]. The service fetches the page, so
/// callers preview only links the user may share with Microsoft.
pub async fn preview(client: &Client, link: &str) -> Result<LinkPreview> {
    if !matches!(
        url::Url::parse(link)
            .map(|u| u.scheme().to_owned())
            .as_deref(),
        Ok("http" | "https")
    ) {
        return Err(Error::Protocol(format!("not a web link: {link}")));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let all = request_headers(&id);
    let extra: Vec<(&str, &str)> = all.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let (status, text) = client
        .request(Method::GET, &info_url(URLP_BASE, link), None, &extra)
        .await?;
    if !(200..300).contains(&status) {
        return Err(Error::status("url_preview", status, &text));
    }
    Ok(parse_info(&serde_json::from_str(&text)?))
}

/// Downloads a preview image from the service (`image_url` or `large_image_url` of a preview).
pub async fn fetch_image(client: &Client, image_url: &str) -> Result<Vec<u8>> {
    if !is_service_url(image_url) {
        return Err(Error::Protocol(
            "refusing to send credentials to a host that is not the preview service".into(),
        ));
    }
    client.bytes(image_url, &[], MAX_IMAGE_BYTES).await
}

/// Sets the preview of a sent message (property `links`).
pub async fn set_message_preview(
    client: &Client,
    conversation_id: &str,
    message_id: &str,
    requested_url: &str,
    preview: &LinkPreview,
) -> Result<()> {
    let (url, body) = set_links_request(
        &client.chat_service().await,
        conversation_id,
        message_id,
        requested_url,
        preview,
    );
    client
        .json("set_links", Method::PUT, &url, Some(&body))
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = r#"{"url":"https://www.rust-lang.org","target_url":"https://rust-lang.org/","size":"18595","status_code":"200","content_type":"text/html","site":"rust-lang.org","category":"generic","title":"\n            Rust Programming Language\n        ","description":"A language empowering everyone to build reliable and efficient software.","favicon":"https://urlp.asm.skype.com/v1/url/image/Favicon?url=https%3a%2f%2frust-lang.org%2ff.png","favicon_meta":{"width":16,"height":16},"thumbnail":"https://urlp.asm.skype.com/v1/url/image/Thumbnail?url=https%3a%2f%2fwww.rust-lang.org%2fs.jpg","thumbnail_meta":{"width":240,"height":200},"largethumbnail":"https://urlp.asm.skype.com/v1/url/image/LargeThumbnail?url=https%3a%2f%2fwww.rust-lang.org%2fs.jpg","largethumbnail_meta":{"width":480,"height":480},"user_pic":""}"#;

    fn info() -> Value {
        serde_json::from_str(INFO).unwrap()
    }

    #[test]
    fn info_url_encodes_the_link() {
        assert_eq!(
            info_url(URLP_BASE, "https://a.example/p?q=1&r=2"),
            "https://urlp.asm.skype.com/v1/url/info?url=https%3A%2F%2Fa.example%2Fp%3Fq%3D1%26r%3D2"
        );
        assert_eq!(
            info_url("https://urlp.asm.skype.com/", "https://a.example"),
            "https://urlp.asm.skype.com/v1/url/info?url=https%3A%2F%2Fa.example"
        );
    }

    #[test]
    fn headers_repeat_the_request_id() {
        let h = request_headers("rid");
        let get = |k: &str| h.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("RequestId"), Some("rid"));
        assert_eq!(get("CorrelationId"), Some("rid"));
        assert_eq!(get("TransactionId"), Some("rid"));
    }

    #[test]
    fn parses_an_info_answer() {
        let p = parse_info(&info());
        assert_eq!(p.url, "https://rust-lang.org/");
        assert_eq!(p.title.as_deref(), Some("Rust Programming Language"));
        assert_eq!(
            p.description.as_deref(),
            Some("A language empowering everyone to build reliable and efficient software.")
        );
        assert_eq!(p.site_name.as_deref(), Some("rust-lang.org"));
        assert!(p.image_url.as_deref().unwrap().contains("/Thumbnail?"));
        assert_eq!((p.image_width, p.image_height), (Some(240), Some(200)));
        assert!(
            p.large_image_url
                .as_deref()
                .unwrap()
                .contains("/LargeThumbnail?")
        );
        assert!(p.favicon_url.is_some());
        assert_eq!(p.status, Some(200));
        assert!(p.is_showable());
        assert_eq!(p.domain(), "rust-lang.org");
    }

    #[test]
    fn an_unreachable_page_has_nothing_to_show() {
        let p = parse_info(
            &json!({"url": "https://x.invalid/a", "target_url": "https://x.invalid/a",
            "status_code": "502", "site": "x.invalid", "favicon": "", "thumbnail": "", "largethumbnail": ""}),
        );
        assert_eq!(p.status, Some(502));
        assert_eq!(
            (&p.title, &p.image_url, &p.favicon_url),
            (&None, &None, &None)
        );
        assert!(!p.is_showable());
        // The requested url stands in when the service gives no target.
        assert_eq!(
            parse_info(&json!({"url": "https://a.example"})).url,
            "https://a.example"
        );
        assert_eq!(parse_info(&json!({})), LinkPreview::default());
    }

    #[test]
    fn a_card_needs_two_parts() {
        let mut p = LinkPreview {
            title: Some("T".into()),
            ..LinkPreview::default()
        };
        assert!(!p.is_showable());
        p.description = Some("D".into());
        assert!(p.is_showable());
    }

    #[test]
    fn the_proxy_serves_a_remote_image_in_three_sizes() {
        assert_eq!(
            image_proxy_url(
                URLP_BASE,
                ImageKind::Thumbnail,
                "https://a.example/i.png?x=1"
            ),
            "https://urlp.asm.skype.com/v1/url/image/Thumbnail?url=https%3A%2F%2Fa.example%2Fi.png%3Fx%3D1"
        );
        assert!(
            image_proxy_url(URLP_BASE, ImageKind::LargeThumbnail, "https://a/i")
                .contains("/image/LargeThumbnail?")
        );
        assert!(
            image_proxy_url(URLP_BASE, ImageKind::Favicon, "https://a/i")
                .contains("/image/Favicon?")
        );
    }

    #[test]
    fn only_the_service_hosts_get_the_token() {
        assert!(is_service_url(
            "https://urlp.asm.skype.com/v1/url/image/Thumbnail?url=x"
        ));
        assert!(is_service_url(
            "https://us-prod.asyncgw.teams.microsoft.com/urlp/v1/url/image/Thumbnail?url=x"
        ));
        assert!(!is_service_url(
            "http://urlp.asm.skype.com/v1/url/image/Thumbnail?url=x"
        ));
        assert!(!is_service_url(
            "https://urlp.asm.skype.com.evil.com/v1/url/image/Thumbnail"
        ));
        assert!(!is_service_url(
            "https://evil.com/urlp.asm.skype.com/v1/url/image/Thumbnail"
        ));
        assert!(!is_service_url(
            "https://urlp.asm.skype.com@evil.com/v1/url/image/Thumbnail"
        ));
        // Other asyncgw paths are AMS objects, not previews.
        assert!(!is_service_url(
            "https://us-prod.asyncgw.teams.microsoft.com/v1/objects/0-x/views/imgo"
        ));
        assert!(!is_service_url("not a url"));
    }

    #[test]
    fn public_links_exclude_tenant_content_and_private_hosts() {
        for ok in [
            "https://www.rust-lang.org/",
            "http://a.example/x?y=1",
            "https://user@b.example:8443/",
        ] {
            assert!(is_public_link(ok), "{ok}");
        }
        for bad in [
            "ftp://a.example",
            "javascript:alert(1)",
            "https://contoso.sharepoint.com/f",
            "https://teams.microsoft.com/l/message/1",
            "https://localhost/x",
            "https://intranet/x",
            "https://10.0.0.5/x",
            "https://192.168.1.1/x",
            "https://127.0.0.1/x",
            "https://[::1]/x",
        ] {
            assert!(!is_public_link(bad), "{bad}");
        }
    }

    #[test]
    fn the_links_property_round_trips() {
        let p = parse_info(&info());
        let json = links_property_json("https://www.rust-lang.org", &p);
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v[0]["@type"], HYPERLINK_TYPE);
        assert_eq!(v[0]["url"], "https://www.rust-lang.org");
        assert_eq!(v[0]["previewenabled"], true);
        assert_eq!(v[0]["preview"]["title"], "Rust Programming Language");
        assert_eq!(
            v[0]["preview"]["previewmeta"],
            json!({"width": 240, "height": 200})
        );
        let back = parse_links_property(&Value::String(json)).remove(0);
        assert_eq!(back.url, "https://www.rust-lang.org");
        assert_eq!(back.title, p.title);
        assert_eq!(back.image_url, p.image_url);
        assert_eq!(
            (back.image_width, back.image_height),
            (Some(240), Some(200))
        );
    }

    #[test]
    fn the_property_skips_disabled_internal_and_thin_entries() {
        let list = json!([
            {"url": "https://a.example", "preview": {"title": "A", "description": "a"}},
            {"url": "https://contoso.sharepoint.com/f", "preview": {"title": "F", "description": "f"}, "previewenabled": true},
            {"url": "https://t.example", "preview": {"title": "Only"}, "previewenabled": true},
            {"url": "https://b.example", "preview": {"title": " B ", "description": "about", "previewMetadata": {"width": 20, "height": 10}}, "previewenabled": true},
            "junk",
        ]);
        for v in [list.clone(), Value::String(list.to_string())] {
            let p = parse_links_property(&v);
            assert_eq!(p.len(), 1);
            assert_eq!(p[0].url, "https://b.example");
            assert_eq!(p[0].title.as_deref(), Some("B"));
            assert_eq!((p[0].image_width, p[0].image_height), (Some(20), Some(10)));
        }
        assert!(parse_links_property(&json!("not json")).is_empty());
        assert!(parse_links_property(&json!("[]")).is_empty());
        assert!(parse_links_property(&Value::Null).is_empty());
    }

    #[test]
    fn a_foreign_image_in_the_property_is_dropped() {
        let v = json!([{"url": "https://a.example", "preview": {"title": "A", "description": "d",
            "previewurl": "https://evil.example/i.png"}, "previewenabled": true}]);
        let p = parse_links_property(&v);
        assert_eq!(p[0].image_url, None);
    }

    #[test]
    fn the_set_request_targets_the_message_property() {
        let p = parse_info(&info());
        let (url, body) = set_links_request(
            "https://amer.ng.msg.teams.microsoft.com",
            "19:a@thread.v2",
            "1700",
            "https://www.rust-lang.org",
            &p,
        );
        assert_eq!(
            url,
            "https://amer.ng.msg.teams.microsoft.com/v1/users/ME/conversations/19%3Aa%40thread.v2/messages/1700/properties?name=links"
        );
        let json = body["links"].as_str().unwrap();
        assert!(json.contains("previewenabled"));
    }
}
