//! Chat list sections ("conversation folders"): Favorites, the sections the user made, and the
//! system ones (Recent chats, Teams and channels, Meeting chats, Muted chats).
//!
//! Read: `GET csa/api/v1/teams/users/me/conversationFolders?supportsAdditionalSystemGeneratedFolders=true&supportsSliceItems=true`
//! (checked live 2026-11-19). The write calls (create, rename, move an item, remove, delete,
//! reorder) use the same URL with `POST`, `PUT /{id}`, `DELETE` and `PATCH`; the Teams web client
//! defines them (`teams/users/me/conversationFolders`) but their bodies are built server side
//! of its GraphQL layer, so IronTeams does not send them: sections it creates stay local. See
//! `docs/protocol.md`, "Chat list sections".

use crate::client::Client;
use crate::error::Result;
use reqwest::Method;
use serde_json::Value;

const URL: &str = "https://teams.microsoft.com/api/csa/api/v1/teams/users/me/conversationFolders?supportsAdditionalSystemGeneratedFolders=true&supportsSliceItems=true";

/// What kind of section a folder is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderKind {
    /// The Favorites section.
    Favorites,
    /// A section the user made.
    User,
    /// Recent chats, Teams and channels, Meeting chats, Muted chats, Quick views, Communities.
    System,
}

/// One item of a section: a chat, a meeting chat or a channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderItem {
    pub conversation_id: String,
    /// `chat`, `meeting`, `space` (a channel's team), `topic` (a channel), `streamofnotes`.
    pub thread_type: String,
}

/// A section of the chat list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub kind: FolderKind,
    /// The wire `folderType` (`Favorites`, `UserCreated`, `RecentChats`, ...).
    pub folder_type: String,
    pub expanded: bool,
    pub items: Vec<FolderItem>,
}

/// All sections, in the order Teams shows them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folders {
    pub folders: Vec<Folder>,
}

impl Folders {
    /// Favorites first, then the user's sections, in Teams' order.
    pub fn user_sections(&self) -> impl Iterator<Item = &Folder> {
        self.folders.iter().filter(|f| f.kind == FolderKind::User)
    }

    pub fn favorites(&self) -> Option<&Folder> {
        self.folders
            .iter()
            .find(|f| f.kind == FolderKind::Favorites)
    }
}

/// Parses the `conversationFolders` answer. Deleted folders are dropped and the order follows
/// `conversationFolderOrder` (folders missing from it keep the response order, after it).
pub fn parse_folders(v: &Value) -> Folders {
    let v = v.get("conversationFolderResponse").unwrap_or(v);
    let order: Vec<&str> = v
        .get("conversationFolderOrder")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut folders: Vec<Folder> = v
        .get("conversationFolders")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|f| f.get("isDeleted").and_then(Value::as_bool) != Some(true))
        .filter_map(|f| {
            let folder_type = f.get("folderType")?.as_str()?.to_string();
            let kind = match folder_type.as_str() {
                "Favorites" => FolderKind::Favorites,
                "UserCreated" => FolderKind::User,
                _ => FolderKind::System,
            };
            Some(Folder {
                id: f.get("id")?.as_str()?.to_string(),
                name: f
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                kind,
                folder_type,
                expanded: f.get("isExpanded").and_then(Value::as_bool).unwrap_or(true),
                items: f
                    .get("conversationFolderItems")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|i| {
                        Some(FolderItem {
                            conversation_id: i.get("conversationId")?.as_str()?.to_string(),
                            thread_type: i
                                .get("threadType")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                        })
                    })
                    .collect(),
            })
        })
        .collect();
    // Stable: ids in `order` first (by position), the rest after them in response order.
    folders.sort_by_key(|f| order.iter().position(|o| *o == f.id).unwrap_or(usize::MAX));
    Folders { folders }
}

impl Client {
    /// The chat list sections of this account.
    pub async fn conversation_folders(&self) -> Result<Folders> {
        let v = self
            .json("conversation folders", Method::GET, URL, None)
            .await?;
        Ok(parse_folders(&v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "conversationFolderOrder": ["b", "fav", "gone", "a"],
            "conversationFolders": [
                {"id": "a", "name": "Ops", "folderType": "UserCreated", "isExpanded": false,
                 "conversationFolderItems": [
                    {"conversationId": "19:x@thread.v2", "threadType": "chat"},
                    {"conversationId": "19:m@thread.v2", "threadType": "meeting"},
                    {"threadType": "chat"}
                 ]},
                {"id": "fav", "name": "Favorites", "folderType": "Favorites", "isExpanded": true,
                 "conversationFolderItems": [
                    {"conversationId": "48:notes", "threadType": "streamofnotes"},
                    {"conversationId": "19:c@thread.tacv2", "threadType": "topic"}
                 ]},
                {"id": "b", "name": "Dev", "folderType": "UserCreated"},
                {"id": "gone", "name": "Old", "folderType": "UserCreated", "isDeleted": true},
                {"id": "rc", "name": "Recent chats", "folderType": "RecentChats"}
            ]
        })
    }

    #[test]
    fn parses_folders_in_teams_order() {
        let f = parse_folders(&sample());
        let ids: Vec<&str> = f.folders.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(
            ids,
            ["b", "fav", "a", "rc"],
            "deleted dropped, order applied"
        );
        let ops = f.user_sections().nth(1).unwrap();
        assert_eq!(ops.name, "Ops");
        assert!(!ops.expanded);
        assert_eq!(
            ops.items.len(),
            2,
            "an item without a conversation id is skipped"
        );
        assert_eq!(ops.items[1].thread_type, "meeting");
        assert_eq!(f.favorites().unwrap().items[0].conversation_id, "48:notes");
        assert_eq!(f.folders[3].kind, FolderKind::System);
    }

    #[test]
    fn reads_the_v2_envelope_and_tolerates_empty() {
        let f = parse_folders(&json!({"conversationFolderResponse": sample()}));
        assert_eq!(f.folders.len(), 4);
        assert!(parse_folders(&json!({})).folders.is_empty());
    }
}
