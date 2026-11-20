//! The actions of the Files tab: the per-row "…" menu (Download, Copy link, Share, Rename, Delete)
//! and the toolbar's New folder and New document. They reuse the OneDrive crate's Graph calls on
//! the drive items of a channel's SharePoint folder (`ops::download`, `rename`, `delete`,
//! `create_folder`, `create_office_doc`, `share::create_link`). Rows that come from a chat's
//! messages have no drive item, so only Open, Copy link and Open in SharePoint apply to them.

use super::super::Engine;
use onedrive::{DocKind, DriveItem, ItemRef, LinkScope, LinkType, ops};
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// Characters SharePoint does not allow in a file or folder name.
const BAD_NAME_CHARS: [char; 9] = ['"', '*', ':', '<', '>', '?', '/', '\\', '|'];

/// The name to use for a new or renamed item: trimmed, not empty, not starting or ending with a
/// dot, and without the characters SharePoint refuses.
pub(super) fn clean_name(name: &str) -> Option<String> {
    let n = name.trim();
    (!n.is_empty()
        && !n.starts_with('.')
        && !n.ends_with('.')
        && !n.contains(BAD_NAME_CHARS)
        && n.chars().count() <= 255)
        .then(|| n.to_string())
}

/// True for an address on SharePoint (including `-my.sharepoint.com` OneDrive for Business).
pub(super) fn is_sharepoint(url: &str) -> bool {
    url.strip_prefix("https://")
        .and_then(|r| r.split(['/', '?', '#']).next())
        .is_some_and(|host| host.to_ascii_lowercase().ends_with(".sharepoint.com"))
}

/// The document kind a toolbar entry names.
pub(super) fn doc_kind(key: &str) -> Option<DocKind> {
    match key {
        "word" => Some(DocKind::Word),
        "excel" => Some(DocKind::Excel),
        "powerpoint" => Some(DocKind::PowerPoint),
        _ => None,
    }
}

impl Engine {
    fn shared_drive_item(&self, url: &str) -> Option<DriveItem> {
        self.find
            .lock()
            .unwrap()
            .shared
            .items
            .iter()
            .find(|i| i.url == url)
            .and_then(|i| i.drive.clone())
    }

    fn shared_note(&self, note: impl Into<String>) {
        self.find.lock().unwrap().shared.note = note.into();
        self.push_shared();
    }

    /// The item behind a row, or a note saying why there is none.
    fn editable_row(&self, url: &str) -> Option<DriveItem> {
        let item = self.shared_drive_item(url);
        if item.is_none() {
            self.shared_note("This file was shared in a message; change it from its location.");
        }
        item
    }

    fn shared_online(&self) -> bool {
        let online = !self.offline.load(Ordering::Relaxed);
        if !online {
            self.shared_note("You're offline.");
        }
        online
    }

    /// Copy link: the item's own address, which works for everyone who already has access.
    pub(in crate::backend) fn shared_copy_link(&self, url: &str) {
        match arboard::Clipboard::new().and_then(|mut c| c.set_text(url.to_string())) {
            Ok(()) => self.shared_note("Link copied"),
            Err(err) => self.shared_note(format!("Couldn't copy the link: {err}")),
        }
    }

    /// Download: asks where to save, then writes the file (a folder as a zip).
    pub(in crate::backend) async fn shared_download(self: &Arc<Self>, url: &str) {
        let Some(item) = self.editable_row(url) else {
            return;
        };
        if !self.shared_online() {
            return;
        }
        let file_name = if item.is_folder {
            format!("{}.zip", item.name)
        } else {
            item.name.clone()
        };
        let Some(target) = rfd::AsyncFileDialog::new()
            .set_file_name(super::super::drive::sanitize_file_name(&file_name))
            .save_file()
            .await
        else {
            return;
        };
        self.shared_note(format!("Downloading {}…", item.name));
        let result = async {
            let bytes = ops::download(&self.client, &item).await?;
            let path = target.path().to_path_buf();
            tokio::task::spawn_blocking(move || std::fs::write(path, bytes)).await??;
            anyhow::Ok(())
        }
        .await;
        match result {
            Ok(()) => self.shared_note(format!("Downloaded {}", item.name)),
            Err(err) => self.shared_note(format!("Couldn't download {}: {err}", item.name)),
        }
    }

    /// Share: a view link for people in the organization, copied to the clipboard.
    pub(in crate::backend) async fn shared_share(self: &Arc<Self>, url: &str) {
        let Some(item) = self.editable_row(url) else {
            return;
        };
        if !self.shared_online() {
            return;
        }
        let made = onedrive::create_link(
            &self.client,
            &ItemRef::from(&item),
            LinkScope::Organization,
            LinkType::View,
            None,
            false,
        )
        .await;
        match made.map(|p| p.link_url) {
            Ok(Some(link)) => match arboard::Clipboard::new().and_then(|mut c| c.set_text(link)) {
                Ok(()) => {
                    self.shared_note("Link for people in your organization copied");
                }
                Err(err) => self.shared_note(format!("Couldn't copy the link: {err}")),
            },
            Ok(None) => self.shared_note("Teams made no link for this file."),
            Err(err) => self.shared_note(format!("Couldn't create a link: {err}")),
        }
    }

    /// Rename: `url` U+001F `new name`.
    pub(in crate::backend) async fn shared_rename(self: &Arc<Self>, arg: &str) {
        let Some((url, name)) = arg.split_once('\u{1f}') else {
            return;
        };
        let Some(item) = self.editable_row(url) else {
            return;
        };
        let Some(name) = clean_name(name) else {
            self.shared_note("A name can't be empty or contain \" * : < > ? / \\ |");
            return;
        };
        if name == item.name || !self.shared_online() {
            return;
        }
        match ops::rename(&self.client, &ItemRef::own(&item), &name).await {
            Ok(_) => {
                self.shared_note(format!("Renamed to {name}"));
                self.load_shared().await;
            }
            Err(err) => self.shared_note(format!("Couldn't rename {}: {err}", item.name)),
        }
    }

    /// Delete: the item goes to the site's recycle bin.
    pub(in crate::backend) async fn shared_delete(self: &Arc<Self>, url: &str) {
        let Some(item) = self.editable_row(url) else {
            return;
        };
        if !self.shared_online() {
            return;
        }
        match ops::delete(&self.client, &ItemRef::own(&item)).await {
            Ok(()) => {
                self.find
                    .lock()
                    .unwrap()
                    .shared
                    .items
                    .retain(|i| i.url != item.web_url);
                self.shared_note(format!("Deleted {} (in the recycle bin)", item.name));
                self.load_shared().await;
            }
            Err(err) => self.shared_note(format!("Couldn't delete {}: {err}", item.name)),
        }
    }

    /// The channel's files folder, or a note when it is not known (a chat, or not loaded yet).
    fn shared_folder(&self) -> Option<ItemRef> {
        let folder = self.find.lock().unwrap().shared.folder.clone();
        if folder.is_none() {
            self.shared_note("This list has no folder to add to.");
        }
        folder
    }

    /// New folder in the channel's files folder.
    pub(in crate::backend) async fn shared_new_folder(self: &Arc<Self>, name: &str) {
        let Some(name) = clean_name(name) else {
            self.shared_note("A name can't be empty or contain \" * : < > ? / \\ |");
            return;
        };
        let Some(parent) = self.shared_folder() else {
            return;
        };
        if !self.shared_online() {
            return;
        }
        match ops::create_folder(&self.client, &parent, &name).await {
            Ok(_) => {
                self.shared_note(format!("Created the folder {name}"));
                self.load_shared().await;
            }
            Err(err) => self.shared_note(format!("Couldn't create the folder: {err}")),
        }
    }

    /// New document (`word`, `excel` or `powerpoint`) in the channel's files folder.
    pub(in crate::backend) async fn shared_new_doc(self: &Arc<Self>, kind: &str) {
        let Some(kind) = doc_kind(kind) else {
            return;
        };
        let Some(parent) = self.shared_folder() else {
            return;
        };
        if !self.shared_online() {
            return;
        }
        match ops::create_office_doc(&self.client, &parent, kind, None).await {
            Ok(made) => {
                self.shared_note(format!("Created {}", made.name));
                self.load_shared().await;
            }
            Err(err) => self.shared_note(format!("Couldn't create the document: {err}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_must_be_ones_sharepoint_accepts() {
        assert_eq!(clean_name("  Plan v2.docx "), Some("Plan v2.docx".into()));
        assert_eq!(clean_name("Q4 / Q1"), None);
        assert_eq!(clean_name("what?"), None);
        assert_eq!(clean_name(""), None);
        assert_eq!(clean_name("   "), None);
        assert_eq!(clean_name(".hidden"), None);
        assert_eq!(clean_name("name."), None);
        assert_eq!(clean_name(&"a".repeat(256)), None);
    }

    #[test]
    fn sharepoint_addresses_are_told_apart() {
        assert!(is_sharepoint(
            "https://contoso.sharepoint.com/sites/team/Shared%20Documents/a.docx"
        ));
        assert!(is_sharepoint("https://contoso-my.sharepoint.com/:w:/p/x"));
        assert!(!is_sharepoint("https://example.com/sharepoint.com/a"));
        assert!(!is_sharepoint("https://sharepoint.com.example.org/a"));
        assert!(!is_sharepoint("http://contoso.sharepoint.com/a"));
    }

    #[test]
    fn the_toolbar_names_three_document_kinds() {
        assert_eq!(doc_kind("word"), Some(DocKind::Word));
        assert_eq!(doc_kind("excel"), Some(DocKind::Excel));
        assert_eq!(doc_kind("powerpoint"), Some(DocKind::PowerPoint));
        assert_eq!(doc_kind("visio"), None);
    }
}
