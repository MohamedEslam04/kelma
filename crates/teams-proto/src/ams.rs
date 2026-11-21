//! AMS (Azure Media Services for Skype): inline image download and upload.

use crate::client::Client;
use crate::error::{Error, Result};
use reqwest::Method;
use serde_json::{Value, json};
use url::Url;

/// Host of the media service URLs messages point at (`…/v1/objects/<id>/views/<view>`).
pub const AMS_VIEW_HOST: &str = "as-api.asm.skype.com";
/// Host objects are created and uploaded on.
const AMS_UPLOAD: &str = "https://as-prod.asyncgw.teams.microsoft.com/v1/objects/";
/// AMS object type of a voice message recording. NOT YET CONFIRMED: the metadata of a voice
/// message Teams mobile uploaded must be read (`kelma-cli ams-meta`) before this is trusted.
pub const AUDIO_OBJECT_TYPE: &str = "sharing/audio";
/// Content slot an audio object's bytes are written to (its view is `audio`).
const AUDIO_CONTENT: &str = "audio";

/// Largest image we download.
pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// True if `url` is an AMS image URL we may send the skypetoken to.
pub fn is_ams_url(url: &str) -> bool {
    let Ok(u) = Url::parse(url) else { return false };
    let host = u.host_str().unwrap_or_default();
    u.scheme() == "https"
        && (host == "api.asm.skype.com"
            || host.ends_with("-api.asm.skype.com")
            || host.ends_with(".asyncgw.teams.microsoft.com"))
}

/// Rewrites a thumbnail view URL to the full-size view.
pub fn full_size_url(url: &str) -> String {
    for view in ["/views/imgt1", "/views/imgo"] {
        if let Some(base) = url.strip_suffix(view) {
            return format!("{base}/views/imgpsh_fullsize");
        }
    }
    url.to_string()
}

/// HTML for an inline image message referencing an uploaded AMS object.
pub fn image_html(object_id: &str, width: u32, height: u32) -> String {
    format!(
        r#"<p><img itemscope="image" style="vertical-align:bottom" src="https://as-api.asm.skype.com/v1/objects/{object_id}/views/imgo" alt="image" itemtype="http://schema.skype.com/AMSImage" width="{width}" height="{height}" id="{object_id}"></p>"#
    )
}

impl Client {
    /// Downloads an AMS image. Refuses non-AMS hosts so the token never leaves Microsoft.
    pub async fn download_image(&self, url: &str) -> Result<Vec<u8>> {
        if !is_ams_url(url) {
            return Err(Error::Protocol(
                "refusing to send credentials to a non-AMS image host".into(),
            ));
        }
        let skype = self.tokens().await.skype.unwrap_or_default();
        self.bytes(
            url,
            &[
                ("Cookie", format!("skypetoken_asm={skype}")),
                ("Accept", "image/*".into()),
            ],
            MAX_IMAGE_BYTES,
        )
        .await
    }

    /// Downloads any AMS object (voice messages, videos). Refuses non-AMS hosts.
    pub async fn download_media(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>> {
        if !is_ams_url(url) {
            return Err(Error::Protocol(
                "refusing to send credentials to a non-AMS host".into(),
            ));
        }
        let skype = self.tokens().await.skype.unwrap_or_default();
        self.bytes(
            url,
            &[("Cookie", format!("skypetoken_asm={skype}"))],
            max_bytes,
        )
        .await
    }

    /// Uploads an image visible to `conversation_id` and returns the AMS object id.
    pub async fn upload_image(
        &self,
        conversation_id: &str,
        filename: &str,
        data: Vec<u8>,
    ) -> Result<String> {
        self.upload_object(conversation_id, filename, "pish/image", "imgpsh", data)
            .await
    }

    /// Uploads a voice message recording (AAC in MP4) visible to `conversation_id` and returns
    /// the AMS object id.
    pub async fn upload_voice(
        &self,
        conversation_id: &str,
        filename: &str,
        data: Vec<u8>,
    ) -> Result<String> {
        self.upload_object(
            conversation_id,
            filename,
            AUDIO_OBJECT_TYPE,
            AUDIO_CONTENT,
            data,
        )
        .await
    }

    /// Creates an AMS object of `kind` that `conversation_id` may read, writes `data` to its
    /// `content` slot and returns its id.
    pub(crate) async fn upload_object(
        &self,
        conversation_id: &str,
        filename: &str,
        kind: &str,
        content: &str,
        data: Vec<u8>,
    ) -> Result<String> {
        let mut perms = serde_json::Map::new();
        perms.insert(conversation_id.to_string(), json!(["read"]));
        let body = json!({"type": kind, "permissions": perms, "filename": filename, "sharingMode": "Inline"});
        let v = self
            .json("ams_create", Method::POST, AMS_UPLOAD, Some(&body))
            .await?;
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol("AMS create returned no id".into()))?
            .to_string();
        let url = format!("{AMS_UPLOAD}{id}/content/{content}");
        self.ensure_fresh().await?;
        let skype = self.tokens().await.skype.unwrap_or_default();
        let resp = self
            .http()
            .put(&url)
            .header("Authorization", format!("skype_token {skype}"))
            .header("Content-Type", "application/octet-stream")
            .body(data)
            .send()
            .await?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(Error::status(
                "ams_upload",
                status,
                &resp.text().await.unwrap_or_default(),
            ));
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ams_host_allowlist() {
        assert!(is_ams_url(
            "https://eu-api.asm.skype.com/v1/objects/0-x/views/imgo"
        ));
        assert!(is_ams_url(
            "https://api.asm.skype.com/v1/objects/0-x/views/imgo"
        ));
        assert!(is_ams_url(
            "https://as-prod.asyncgw.teams.microsoft.com/v1/objects/0-x"
        ));
        assert!(!is_ams_url("http://eu-api.asm.skype.com/v1/objects/0-x"));
        assert!(!is_ams_url("https://evil.com/api.asm.skype.com"));
        assert!(!is_ams_url("https://asm.skype.com.evil.com/x"));
    }

    #[test]
    fn full_size_rewrite() {
        assert_eq!(
            full_size_url("https://h/v1/objects/1/views/imgo"),
            "https://h/v1/objects/1/views/imgpsh_fullsize"
        );
        assert_eq!(
            full_size_url("https://h/v1/objects/1/views/imgt1"),
            "https://h/v1/objects/1/views/imgpsh_fullsize"
        );
        assert_eq!(full_size_url("https://h/x.png"), "https://h/x.png");
    }

    #[test]
    fn image_html_shape() {
        let h = image_html("0-weu-d1-abc", 250, 141);
        assert!(h.contains("itemtype=\"http://schema.skype.com/AMSImage\""));
        assert!(h.contains("/v1/objects/0-weu-d1-abc/views/imgo"));
    }
}
