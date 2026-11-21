//! Sending files the way Teams does for chats: upload to the sender's OneDrive folder
//! `Microsoft Teams Chat Files`, give the chat members access, then post a message whose
//! `properties.files` points at the item.

use reqwest::Method;
use serde_json::{Value, json};

use crate::client::Client;
use crate::error::{Error, Result};
use crate::http::headers_for;

/// Folder Teams uses for files sent in chats.
pub const CHAT_FILES_FOLDER: &str = "Microsoft Teams Chat Files";
/// Files up to this size go in one PUT; larger ones use an upload session.
pub const SIMPLE_UPLOAD_MAX: usize = 4 * 1024 * 1024;
/// Upload session chunk size; must be a multiple of 320 KiB.
pub const CHUNK: usize = 16 * 320 * 1024;

/// A file stored in OneDrive and ready to be referenced from a message.
#[derive(Debug, Clone, PartialEq)]
pub struct UploadedFile {
    /// Graph drive item id (used for sharing calls).
    pub drive_item_id: String,
    /// SharePoint list item unique id (what Teams puts in `properties.files[].id`).
    pub unique_id: String,
    pub name: String,
    /// Full URL of the file (`webUrl`).
    pub object_url: String,
    /// The OneDrive site URL with a trailing slash.
    pub site_url: String,
}

/// File extension Teams shows on the card (`pdf`, `docx`, ...).
pub fn file_type(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Escapes a file name for a Graph path segment (`root:/folder/<name>:`).
pub fn encode_path_segment(name: &str) -> String {
    let mut out = String::new();
    for b in name.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Graph URL that uploads `name` into the chat files folder (renaming on conflict).
pub fn simple_upload_url(name: &str) -> String {
    format!(
        "https://graph.microsoft.com/v1.0/me/drive/root:/{}/{}:/content?@microsoft.graph.conflictBehavior=rename",
        encode_path_segment(CHAT_FILES_FOLDER),
        encode_path_segment(name)
    )
}

/// Graph URL that opens an upload session for `name` in the chat files folder.
pub fn session_url(name: &str) -> String {
    format!(
        "https://graph.microsoft.com/v1.0/me/drive/root:/{}/{}:/createUploadSession",
        encode_path_segment(CHAT_FILES_FOLDER),
        encode_path_segment(name)
    )
}

/// The Entra object id inside a user MRI (`8:orgid:<guid>`).
pub fn member_object_id(mri: &str) -> Option<&str> {
    mri.strip_prefix("8:orgid:").filter(|g| !g.is_empty())
}

/// Builds an [`UploadedFile`] from a Graph drive item (`$select=id,name,webUrl,sharepointIds`).
pub fn uploaded_from_item(item: &Value) -> Result<UploadedFile> {
    let s = |p: &str| item.pointer(p).and_then(Value::as_str).map(str::to_owned);
    let missing = |what: &str| Error::Protocol(format!("drive item has no {what}"));
    let mut site_url = s("/sharepointIds/siteUrl").ok_or_else(|| missing("siteUrl"))?;
    if !site_url.ends_with('/') {
        site_url.push('/');
    }
    Ok(UploadedFile {
        drive_item_id: s("/id").ok_or_else(|| missing("id"))?,
        unique_id: s("/sharepointIds/listItemUniqueId")
            .ok_or_else(|| missing("listItemUniqueId"))?,
        name: s("/name").ok_or_else(|| missing("name"))?,
        object_url: s("/webUrl").ok_or_else(|| missing("webUrl"))?,
        site_url,
    })
}

/// The `properties.files` value (a JSON string of an array) for a message carrying `f`.
pub fn files_property(f: &UploadedFile) -> String {
    let ft = file_type(&f.name);
    json!([{
        "@type": "http://schema.skype.com/File",
        "version": 2,
        "id": f.unique_id,
        "baseUrl": f.site_url,
        "type": ft,
        "title": f.name,
        "state": "active",
        "objectUrl": f.object_url,
        "providerData": "",
        "itemid": f.unique_id,
        "fileName": f.name,
        "fileType": ft,
        "fileInfo": {
            "itemId": null,
            "fileUrl": f.object_url,
            "siteUrl": f.site_url,
            "serverRelativeUrl": "",
            "shareUrl": f.object_url,
            "shareId": null
        },
        "botFileProperties": {},
        "permissionScope": "users",
        "filePreview": {},
        "fileChicletState": {"serviceName": "p2p", "state": "active"}
    }])
    .to_string()
}

impl Client {
    /// Sends raw bytes with Graph credentials (none for pre-authenticated session URLs).
    pub async fn put_bytes(
        &self,
        url: &str,
        data: Vec<u8>,
        range: Option<String>,
    ) -> Result<Value> {
        self.ensure_fresh().await?;
        let parsed =
            url::Url::parse(url).map_err(|e| Error::Protocol(format!("bad url {url}: {e}")))?;
        let tokens = self.tokens().await;
        let mut req = self.http().put(parsed.clone());
        for (k, v) in headers_for(&parsed, &tokens, self.epid()) {
            req = req.header(k, v);
        }
        if let Some(r) = range {
            req = req.header("Content-Range", r);
        }
        let resp = req
            .header("Content-Type", "application/octet-stream")
            .body(data)
            .send()
            .await
            // Upload session URLs carry a temporary access token: keep them out of error text.
            .map_err(|e| Error::Http(e.without_url()))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        if !(200..300).contains(&status) {
            return Err(Error::status("file_upload", status, &text));
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// Sends `data` to a pre-authenticated upload session URL in [`CHUNK`]-sized pieces and
    /// returns the stored item (the response to the last chunk).
    pub async fn upload_chunks(&self, upload_url: &str, data: &[u8]) -> Result<Value> {
        let total = data.len();
        let mut last = Value::Null;
        for (i, chunk) in data.chunks(CHUNK).enumerate() {
            let start = i * CHUNK;
            let range = format!("bytes {start}-{}/{total}", start + chunk.len() - 1);
            last = self
                .put_bytes(upload_url, chunk.to_vec(), Some(range))
                .await?;
        }
        Ok(last)
    }

    /// Uploads `data` as `name` to the chat files folder and returns the stored item.
    pub async fn upload_file(&self, name: &str, data: Vec<u8>) -> Result<UploadedFile> {
        if self.tokens().await.graph.is_none() {
            return Err(Error::Auth(
                "this account has no Microsoft Graph access, so files cannot be uploaded".into(),
            ));
        }
        let item = if data.len() <= SIMPLE_UPLOAD_MAX {
            self.put_bytes(&simple_upload_url(name), data, None).await?
        } else {
            let body = json!({"item": {"@microsoft.graph.conflictBehavior": "rename"}});
            let s = self
                .json(
                    "upload_session",
                    Method::POST,
                    &session_url(name),
                    Some(&body),
                )
                .await?;
            let upload_url = s
                .get("uploadUrl")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Protocol("upload session has no uploadUrl".into()))?
                .to_string();
            self.upload_chunks(&upload_url, &data).await?
        };
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol("upload returned no item id".into()))?;
        let url = format!(
            "https://graph.microsoft.com/v1.0/me/drive/items/{id}?$select=id,name,webUrl,sharepointIds"
        );
        let full = self.json("drive_item", Method::GET, &url, None).await?;
        uploaded_from_item(&full)
    }

    /// Gives chat members (user MRIs) edit access to an uploaded file, as Teams does.
    pub async fn share_file_with(&self, f: &UploadedFile, member_mris: &[String]) -> Result<()> {
        let recipients: Vec<Value> = member_mris
            .iter()
            .filter_map(|m| member_object_id(m))
            .map(|id| json!({"objectId": id}))
            .collect();
        if recipients.is_empty() {
            return Ok(());
        }
        let url = format!(
            "https://graph.microsoft.com/v1.0/me/drive/items/{}/invite",
            f.drive_item_id
        );
        let body = json!({
            "recipients": recipients,
            "requireSignIn": true,
            "sendInvitation": false,
            "roles": ["write"],
        });
        self.json("file_share", Method::POST, &url, Some(&body))
            .await
            .map(|_| ())
    }
}

impl Client {
    /// Uploads a file, shares it with the other members of `conversation_id` and posts it there.
    /// Channels keep their files in the team's SharePoint site, see [`crate::channel_files`].
    pub async fn send_file(
        &self,
        conversation_id: &str,
        name: &str,
        data: Vec<u8>,
        display_name: &str,
    ) -> Result<UploadedFile> {
        let cmid = crate::time::now_ms().to_string();
        self.send_file_as(conversation_id, name, data, display_name, &cmid)
            .await
    }

    /// Like [`Client::send_file`] with a caller-chosen client message id.
    pub async fn send_file_as(
        &self,
        conversation_id: &str,
        name: &str,
        data: Vec<u8>,
        display_name: &str,
        client_message_id: &str,
    ) -> Result<UploadedFile> {
        let conv = conversation_id
            .split(";messageid=")
            .next()
            .unwrap_or(conversation_id);
        if crate::channel_files::is_channel(conv) {
            return self
                .send_channel_file_as(conversation_id, name, data, display_name, client_message_id)
                .await;
        }
        let file = self.upload_file(name, data).await?;
        if conv != "48:notes" {
            let me = self.self_mri().await.unwrap_or_default();
            let (members, _) = self.thread_members(conv).await?;
            let others: Vec<String> = members.into_iter().filter(|m| *m != me).collect();
            self.share_file_with(&file, &others).await?;
        }
        self.send_file_message_as(
            conversation_id,
            "",
            display_name,
            &files_property(&file),
            client_message_id,
        )
        .await?;
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> Value {
        json!({
            "id": "01ABC",
            "name": "Report Q3.pdf",
            "webUrl": "https://contoso-my.sharepoint.com/personal/me_contoso_com/Documents/Microsoft%20Teams%20Chat%20Files/Report%20Q3.pdf",
            "sharepointIds": {
                "listItemUniqueId": "5f0c-uuid",
                "siteUrl": "https://contoso-my.sharepoint.com/personal/me_contoso_com"
            }
        })
    }

    #[test]
    fn upload_urls_are_encoded() {
        assert_eq!(
            simple_upload_url("تقرير 1.pdf"),
            "https://graph.microsoft.com/v1.0/me/drive/root:/Microsoft%20Teams%20Chat%20Files/%D8%AA%D9%82%D8%B1%D9%8A%D8%B1%201.pdf:/content?@microsoft.graph.conflictBehavior=rename"
        );
        assert!(session_url("a#b.txt").ends_with("/a%23b.txt:/createUploadSession"));
    }

    #[test]
    fn object_ids_from_mris() {
        assert_eq!(member_object_id("8:orgid:abc-1"), Some("abc-1"));
        assert_eq!(member_object_id("28:bot"), None);
        assert_eq!(member_object_id("8:orgid:"), None);
    }

    #[test]
    fn item_and_files_property() {
        let f = uploaded_from_item(&item()).unwrap();
        assert_eq!(f.unique_id, "5f0c-uuid");
        assert!(f.site_url.ends_with("me_contoso_com/"));
        let prop = files_property(&f);
        // Round-trips through the parser IronTeams uses for incoming messages.
        let parsed = crate::model::parse_files(Some(&json!({ "files": prop })));
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "Report Q3.pdf");
        assert_eq!(parsed[0].file_type, "pdf");
        assert_eq!(parsed[0].url, f.object_url);
    }

    #[test]
    fn missing_fields_are_errors() {
        assert!(uploaded_from_item(&json!({"id": "1"})).is_err());
        assert_eq!(file_type("archive.TAR.GZ"), "gz");
        assert_eq!(file_type("README"), "");
    }
}
