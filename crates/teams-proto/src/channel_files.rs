//! Files in channels (plan 8.6) and cloud files attached to a message (plan 8.7).
//!
//! A channel keeps its files in a folder of the team's SharePoint site. Graph gives its drive item
//! as `GET /teams/{groupId}/channels/{channelId}/filesFolder`. A file is uploaded with
//! `PUT /drives/{driveId}/items/{folderId}:/{name}:/content` and posted with `properties.files`
//! exactly like a chat file. Channel members already have access to the folder, so no invite is
//! sent. A OneDrive file picked with "Attach cloud files" is looked up in its own drive and, for
//! a chat, shared with the other members first.

use crate::client::Client;
use crate::error::{Error, Result};
use crate::files::{
    SIMPLE_UPLOAD_MAX, UploadedFile, encode_path_segment, files_property, member_object_id,
    uploaded_from_item,
};
use crate::graph::graph_url;
use crate::teams::find_channel;
use reqwest::Method;
use serde_json::{Value, json};

/// The SharePoint folder of a channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesFolder {
    pub drive_id: String,
    pub item_id: String,
    pub name: String,
}

/// Parses the `filesFolder` drive item.
pub fn parse_files_folder(v: &Value) -> Option<FilesFolder> {
    let text = |p: &str| {
        v.pointer(p)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    };
    Some(FilesFolder {
        drive_id: text("/parentReference/driveId")?.to_string(),
        item_id: text("/id")?.to_string(),
        name: text("/name").unwrap_or_default().to_string(),
    })
}

/// URL that uploads `name` into `folder` in one request (renaming on a name clash).
pub fn upload_url(folder: &FilesFolder, name: &str) -> String {
    format!(
        "https://graph.microsoft.com/v1.0/drives/{}/items/{}:/{}:/content?@microsoft.graph.conflictBehavior=rename",
        encode_path_segment(&folder.drive_id),
        encode_path_segment(&folder.item_id),
        encode_path_segment(name)
    )
}

/// URL that opens an upload session for `name` in `folder`.
pub fn session_url(folder: &FilesFolder, name: &str) -> String {
    format!(
        "https://graph.microsoft.com/v1.0/drives/{}/items/{}:/{}:/createUploadSession",
        encode_path_segment(&folder.drive_id),
        encode_path_segment(&folder.item_id),
        encode_path_segment(name)
    )
}

/// The conversation a message goes to, without a thread suffix (`…;messageid=<root>`).
fn base_conversation(id: &str) -> &str {
    id.split(";messageid=").next().unwrap_or(id)
}

/// True for a channel thread id.
pub fn is_channel(conversation_id: &str) -> bool {
    let c = base_conversation(conversation_id);
    c.ends_with("@thread.tacv2") || c.ends_with("@thread.skype")
}

impl Client {
    /// The files folder of a channel (a team's General channel included).
    pub async fn channel_files_folder(
        &self,
        group_id: &str,
        channel_id: &str,
    ) -> Result<FilesFolder> {
        let url = graph_url(
            &["teams", group_id, "channels", channel_id, "filesFolder"],
            "",
        );
        let v = self.json("files folder", Method::GET, &url, None).await?;
        parse_files_folder(&v)
            .ok_or_else(|| Error::Protocol("the channel's files folder has no drive".into()))
    }

    /// Looks a channel up in the team list and returns its files folder.
    async fn files_folder_of(&self, channel_id: &str) -> Result<FilesFolder> {
        let teams = self.teams_and_channels().await?;
        let (team, _) = find_channel(&teams, channel_id)
            .ok_or_else(|| Error::Protocol("this channel is not in your team list".into()))?;
        if team.group_id.is_empty() {
            return Err(Error::Protocol(
                "this team has no files site, so files cannot be sent to its channels".into(),
            ));
        }
        self.channel_files_folder(&team.group_id, channel_id).await
    }

    /// The stored drive item as an [`UploadedFile`].
    async fn drive_item_file(&self, drive_id: &str, item_id: &str) -> Result<UploadedFile> {
        let url = graph_url(
            &["drives", drive_id, "items", item_id],
            "$select=id,name,webUrl,sharepointIds",
        );
        let full = self.json("drive item", Method::GET, &url, None).await?;
        uploaded_from_item(&full)
    }

    /// Uploads `data` as `name` into `folder` and returns the stored item.
    pub async fn upload_to_folder(
        &self,
        folder: &FilesFolder,
        name: &str,
        data: Vec<u8>,
    ) -> Result<UploadedFile> {
        let item = if data.len() <= SIMPLE_UPLOAD_MAX {
            self.put_bytes(&upload_url(folder, name), data, None)
                .await?
        } else {
            let body = json!({"item": {"@microsoft.graph.conflictBehavior": "rename"}});
            let s = self
                .json(
                    "upload_session",
                    Method::POST,
                    &session_url(folder, name),
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
        self.drive_item_file(&folder.drive_id, id).await
    }

    /// Uploads a file to the channel's folder and posts it (in the thread when `conversation_id`
    /// has a `;messageid=` suffix).
    pub async fn send_channel_file_as(
        &self,
        conversation_id: &str,
        name: &str,
        data: Vec<u8>,
        display_name: &str,
        client_message_id: &str,
    ) -> Result<UploadedFile> {
        let folder = self
            .files_folder_of(base_conversation(conversation_id))
            .await?;
        let file = self.upload_to_folder(&folder, name, data).await?;
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

    /// Deletes a drive item (used to remove a test upload).
    pub async fn delete_drive_item(&self, drive_id: &str, item_id: &str) -> Result<()> {
        let url = graph_url(&["drives", drive_id, "items", item_id], "");
        self.json("delete drive item", Method::DELETE, &url, None)
            .await
            .map(|_| ())
    }

    /// Posts a file that already lives in OneDrive or SharePoint ("Attach cloud files"). In a
    /// chat the other members are given edit access first; a channel's members already see its
    /// files, and a file in someone else's library keeps whatever access it has (sharing it can
    /// be refused, which does not stop the message).
    pub async fn send_cloud_file_as(
        &self,
        conversation_id: &str,
        drive_id: &str,
        item_id: &str,
        display_name: &str,
        client_message_id: &str,
    ) -> Result<UploadedFile> {
        let file = self.drive_item_file(drive_id, item_id).await?;
        let conv = base_conversation(conversation_id);
        if conv != "48:notes" && !is_channel(conv) {
            let me = self.self_mri().await.unwrap_or_default();
            let (members, _) = self.thread_members(conv).await?;
            let others: Vec<String> = members.into_iter().filter(|m| *m != me).collect();
            if let Err(e) = self.share_in_drive(drive_id, &file, &others).await {
                tracing::warn!(%e, "could not share the attached file with the chat members");
            }
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

    /// Gives `member_mris` edit access to an item of any drive.
    async fn share_in_drive(
        &self,
        drive_id: &str,
        f: &UploadedFile,
        member_mris: &[String],
    ) -> Result<()> {
        let recipients: Vec<Value> = member_mris
            .iter()
            .filter_map(|m| member_object_id(m))
            .map(|id| json!({"objectId": id}))
            .collect();
        if recipients.is_empty() {
            return Ok(());
        }
        let url = graph_url(
            &["drives", drive_id, "items", &f.drive_item_id, "invite"],
            "",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_folder_item() {
        let v = json!({"id": "01F", "name": "tests", "parentReference": {"driveId": "b!abc"}});
        assert_eq!(
            parse_files_folder(&v),
            Some(FilesFolder {
                drive_id: "b!abc".into(),
                item_id: "01F".into(),
                name: "tests".into()
            })
        );
        assert_eq!(parse_files_folder(&json!({"id": "01F"})), None);
    }

    #[test]
    fn upload_urls_name_the_drive_folder_and_file() {
        let f = FilesFolder {
            drive_id: "b!a-b".into(),
            item_id: "01F".into(),
            name: String::new(),
        };
        assert_eq!(
            upload_url(&f, "تقرير 1.txt"),
            "https://graph.microsoft.com/v1.0/drives/b%21a-b/items/01F:/%D8%AA%D9%82%D8%B1%D9%8A%D8%B1%201.txt:/content?@microsoft.graph.conflictBehavior=rename"
        );
        assert!(session_url(&f, "a#b.txt").ends_with("/items/01F:/a%23b.txt:/createUploadSession"));
    }

    #[test]
    fn channel_ids_are_recognized_with_or_without_a_thread() {
        assert!(is_channel("19:abc@thread.tacv2"));
        assert!(is_channel("19:abc@thread.tacv2;messageid=1700"));
        assert!(!is_channel("19:abc@thread.v2"));
        assert!(!is_channel("48:notes"));
    }
}
