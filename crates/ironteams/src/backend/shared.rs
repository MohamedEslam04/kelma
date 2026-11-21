//! The Shared tab (a chat's files and links, a channel's Files tab) and the chat details pane
//! (plan chat 7.5, 7.6). Chats read the Substrate `AllShared` list and add what the cached
//! messages carry; channels also list their SharePoint folder.

use super::Engine;
use crate::find_data::{SharedData, matches_all, terms_of};
use chrono::{Local, TimeZone};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use store::{ConvKind, MessageRow};
use teams_proto::views::{SharedItem, SharedKind};

mod files;

/// Messages of the cache scanned for files and links.
const CACHE_SCAN: usize = 3000;
/// Items the details pane lists.
const RECENT: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Item {
    kind: SharedKind,
    name: String,
    ext: String,
    shared_by: String,
    shared_ms: i64,
    url: String,
    /// The SharePoint drive item, for the files of a channel's folder (row actions need it).
    drive: Option<onedrive::DriveItem>,
    /// What is being done to the row while the server answers ("Deleting…"), or "".
    busy: &'static str,
}

impl From<SharedItem> for Item {
    fn from(i: SharedItem) -> Item {
        Item {
            kind: i.kind,
            name: i.name,
            ext: i.extension,
            shared_by: i.shared_by,
            shared_ms: i.shared_ms,
            url: i.url,
            drive: None,
            busy: "",
        }
    }
}

impl Item {
    /// A file or folder of a channel's SharePoint folder.
    fn from_drive(i: onedrive::DriveItem) -> Item {
        Item {
            kind: SharedKind::File,
            ext: if i.is_folder {
                "DIR".into()
            } else {
                ext_of(&i.name)
            },
            name: i.name.clone(),
            shared_by: i.modified_by.clone(),
            shared_ms: i.modified_ms,
            url: i.web_url.clone(),
            drive: Some(i),
            busy: "",
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    /// The conversation `items` belong to.
    conv: String,
    items: Vec<Item>,
    /// `recent`, `files` or `links`.
    filter: String,
    keyword: String,
    loading: bool,
    details_open: bool,
    /// The Shared tab is showing.
    tab_open: bool,
    seq: u64,
    /// Team thread id to AAD group id, for the channel folders.
    groups: HashMap<String, String>,
    /// The open channel's SharePoint files folder, once read: where New folder and New document
    /// create things.
    pub(super) folder: Option<onedrive::ItemRef>,
    /// What the last file action did or why it failed, shown over the list.
    note: String,
    /// `note` says an action failed (drawn in red).
    note_error: bool,
    /// The row just created, highlighted for a moment.
    highlight: String,
}

/// How long a created row stays highlighted.
const HIGHLIGHT_MS: u64 = 2500;

impl State {
    fn item_mut(&mut self, url: &str) -> Option<&mut Item> {
        self.items.iter_mut().find(|i| i.url == url)
    }

    /// An action on a row started: the row dims with `label` until the server answers.
    fn set_busy(&mut self, url: &str, label: &'static str) {
        if let Some(i) = self.item_mut(url) {
            i.busy = label;
        }
    }

    /// The server answered a delete: the row goes, or comes back as it was.
    fn delete_done(&mut self, url: &str, ok: bool) {
        if ok {
            self.items.retain(|i| i.url != url);
        } else {
            self.set_busy(url, "");
        }
    }

    /// Rename: the row shows the new name at once. Returns the old one.
    fn rename_now(&mut self, url: &str, name: &str) -> Option<String> {
        let i = self.item_mut(url)?;
        let old = std::mem::replace(&mut i.name, name.to_string());
        if !i.ext.is_empty() && i.ext != "DIR" {
            i.ext = ext_of(name);
        }
        Some(old)
    }

    /// The server answered a rename: the row takes the renamed item (its address can change
    /// with the name), or its old name back.
    fn rename_done(&mut self, url: &str, result: Result<onedrive::DriveItem, String>) {
        let Some(pos) = self.items.iter().position(|i| i.url == url) else {
            return;
        };
        match result {
            Ok(made) => self.items[pos] = Item::from_drive(made),
            Err(old) => {
                let i = &mut self.items[pos];
                i.ext = if i.ext == "DIR" {
                    i.ext.clone()
                } else {
                    ext_of(&old)
                };
                i.name = old;
            }
        }
    }

    /// A folder or document was created: it shows first, under the Files filter (the list may
    /// be long and on another filter), and is highlighted. Returns its address.
    fn add_created(&mut self, made: onedrive::DriveItem) -> String {
        let mut item = Item::from_drive(made);
        item.shared_ms = item.shared_ms.max(teams_proto::time::now_ms());
        let url = item.url.clone();
        self.items.retain(|i| i.url != url);
        self.items.insert(0, item);
        self.filter = "files".into();
        self.keyword.clear();
        self.highlight = url.clone();
        url
    }
}

/// `Nov 18, 2026`.
fn date_label(ms: i64) -> String {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .filter(|_| ms > 0)
        .map(|t| t.format("%b %-d, %Y").to_string())
        .unwrap_or_default()
}

/// The links of a message (`<a href>` with their text), without media and mention markup.
fn links_of(html: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut rest = html;
    while let Some(start) = rest.find("<a ") {
        rest = &rest[start..];
        let Some(tag_end) = rest.find('>') else { break };
        let tag = &rest[..tag_end];
        let href = tag
            .split("href=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or_default()
            .replace("&amp;", "&");
        let after = &rest[tag_end + 1..];
        let (text, next) = match after.find("</a>") {
            Some(e) => (&after[..e], &after[e + 4..]),
            None => ("", after),
        };
        if href.starts_with("http://") || href.starts_with("https://") {
            let label = ::render::plain_text(&::render::parse_teams_html(text));
            out.push((href, label.trim().to_string()));
        }
        rest = next;
    }
    out
}

/// The extension of a file name without the dot, upper case.
fn ext_of(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_uppercase())
        .filter(|e| !e.is_empty() && e.len() <= 5)
        .unwrap_or_else(|| "FILE".into())
}

/// Files and links the cached messages of a conversation carry, newest first.
fn items_from_cache(rows: &[MessageRow]) -> Vec<Item> {
    let mut out: Vec<Item> = vec![];
    let mut seen: HashSet<String> = HashSet::new();
    for m in rows {
        let props: serde_json::Value = serde_json::from_str(&m.reactions_json).unwrap_or_default();
        for f in props
            .get("f")
            .and_then(|f| f.as_array())
            .into_iter()
            .flatten()
        {
            let (Some(name), Some(url)) = (
                f.get("name").and_then(|v| v.as_str()),
                f.get("url").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            if seen.insert(url.to_string()) {
                out.push(Item {
                    kind: SharedKind::File,
                    ext: f
                        .get("type")
                        .and_then(|v| v.as_str())
                        .filter(|t| !t.is_empty())
                        .map(str::to_uppercase)
                        .unwrap_or_else(|| ext_of(name)),
                    name: name.to_string(),
                    shared_by: m.sender_name.clone(),
                    shared_ms: m.compose_ms,
                    url: url.to_string(),
                    drive: None,
                    busy: "",
                });
            }
        }
        for (url, label) in links_of(&m.content_html) {
            if seen.insert(url.clone()) {
                out.push(Item {
                    kind: SharedKind::Link,
                    name: if label.is_empty() { url.clone() } else { label },
                    ext: String::new(),
                    shared_by: m.sender_name.clone(),
                    shared_ms: m.compose_ms,
                    url,
                    drive: None,
                    busy: "",
                });
            }
        }
    }
    out.sort_by_key(|i| std::cmp::Reverse(i.shared_ms));
    out
}

/// Adds `extra` items whose address is not there yet; newest first.
fn merge_items(items: &mut Vec<Item>, extra: Vec<Item>) {
    for e in extra {
        if !items.iter().any(|i| i.url == e.url) {
            items.push(e);
        }
    }
    items.sort_by_key(|i| std::cmp::Reverse(i.shared_ms));
}

/// The rows the Shared tab shows for a filter (`recent`, `files`, `links`) and a keyword.
fn visible(items: &[Item], filter: &str, keyword: &str) -> Vec<Item> {
    let terms = terms_of(keyword);
    items
        .iter()
        .filter(|i| match filter {
            "files" => i.kind == SharedKind::File,
            "links" => i.kind == SharedKind::Link,
            _ => true,
        })
        .filter(|i| terms.is_empty() || matches_all(&format!("{} {}", i.name, i.shared_by), &terms))
        .cloned()
        .collect()
}

/// The icon of a row: `folder`, a document kind (`word`, `excel`, `powerpoint`, `pdf`, `image`,
/// `archive`, ...; see `drive::state::FileKind`) or `link`.
fn icon_of(i: &Item) -> &'static str {
    if i.kind == SharedKind::Link {
        return "link";
    }
    let item = match &i.drive {
        Some(d) => d.clone(),
        None => onedrive::DriveItem {
            name: format!("f.{}", i.ext.to_lowercase()),
            is_folder: i.ext == "DIR",
            ..Default::default()
        },
    };
    super::drive::state::FileKind::of(&item).as_str()
}

fn data(i: &Item, highlight: &str) -> SharedData {
    SharedData {
        icon: icon_of(i),
        busy: i.busy.to_string(),
        highlight: !highlight.is_empty() && i.url == highlight,
        kind: match i.kind {
            SharedKind::File => "file",
            SharedKind::Link => "link",
        },
        name: i.name.clone(),
        ext: i.ext.clone(),
        shared_by: i.shared_by.clone(),
        date: date_label(i.shared_ms),
        url: i.url.clone(),
        editable: i.drive.is_some(),
        sharepoint: i.kind == SharedKind::File && files::is_sharepoint(&i.url),
    }
}

impl Engine {
    /// A chat was opened: forget the old list, and load the new one when the pane is showing.
    pub(super) fn shared_chat_opened(self: &Arc<Self>, conv: &str) {
        let show = {
            let mut f = self.find.lock().unwrap();
            let s = &mut f.shared;
            s.conv = conv.to_string();
            s.items.clear();
            s.keyword.clear();
            s.filter = "recent".into();
            s.folder = None;
            s.note.clear();
            s.note_error = false;
            s.highlight.clear();
            s.tab_open = false;
            s.seq += 1;
            s.details_open
        };
        self.push_shared();
        if show {
            let e = self.clone();
            tokio::spawn(async move { e.load_shared().await });
        }
    }

    pub(super) async fn shared_tab(self: &Arc<Self>, tab: &str) {
        let open = tab == "shared";
        self.find.lock().unwrap().shared.tab_open = open;
        if open {
            self.load_shared().await;
        }
    }

    pub(super) async fn details(self: &Arc<Self>, open: bool) {
        self.find.lock().unwrap().shared.details_open = open;
        if open {
            self.load_shared().await;
        }
    }

    pub(super) fn shared_filter(self: &Arc<Self>, f: &str) {
        self.find.lock().unwrap().shared.filter = f.to_string();
        self.push_shared();
    }

    pub(super) fn shared_keyword(self: &Arc<Self>, k: String) {
        self.find.lock().unwrap().shared.keyword = k;
        self.push_shared();
    }

    /// Upload: the file picker adds the file to the composer like the paperclip does, so the
    /// chat shows again with the chip to send.
    pub(super) async fn shared_upload(self: &Arc<Self>) {
        self.ui.chat_tab("chat");
        self.find.lock().unwrap().shared.tab_open = false;
        self.clone().handle(super::Cmd::AttachFile).await;
    }

    /// Upload from OneDrive: the OneDrive file picker (`DriveFilePicker`) adds the chosen file
    /// to the composer like Attach cloud files does, so the chat shows again with the chip.
    pub(super) async fn shared_onedrive(self: &Arc<Self>) {
        self.ui.chat_tab("chat");
        self.find.lock().unwrap().shared.tab_open = false;
        self.clone()
            .handle(super::Cmd::Drive(super::DriveCmd::PickBegin(
                "attach".into(),
            )))
            .await;
    }

    fn push_shared(&self) {
        let (rows, recent, loading, can_create, note, error) = {
            let f = self.find.lock().unwrap();
            let s = &f.shared;
            (
                visible(&s.items, &s.filter, &s.keyword)
                    .iter()
                    .map(|i| data(i, &s.highlight))
                    .collect(),
                s.items
                    .iter()
                    .take(RECENT)
                    .map(|i| data(i, &s.highlight))
                    .collect(),
                s.loading,
                s.folder.is_some(),
                s.note.clone(),
                s.note_error,
            )
        };
        self.ui.shared(rows, recent, loading);
        self.ui.shared_tools(can_create, note, error);
    }

    /// Fills the list: the cache at once, then the server.
    async fn load_shared(self: &Arc<Self>) {
        let Some(conv) = self.view.lock().unwrap().current.clone() else {
            return;
        };
        let (seq, is_channel) = {
            let mut f = self.find.lock().unwrap();
            f.shared.seq += 1;
            f.shared.conv = conv.clone();
            f.shared.loading = !self.offline.load(Ordering::Relaxed);
            let kind = self
                .store
                .lock()
                .unwrap()
                .conversation(&conv)
                .ok()
                .flatten();
            (
                f.shared.seq,
                kind.is_some_and(|c| c.kind == ConvKind::Channel)
                    || conv.ends_with("@thread.tacv2"),
            )
        };
        let rows = self
            .store
            .lock()
            .unwrap()
            .messages_before(&conv, None, CACHE_SCAN)
            .unwrap_or_default();
        // A reload (after New folder, Rename, ...) keeps the rows it shows until the server
        // answers: they used to drop to the cached ones, so a folder just made vanished for the
        // whole round trip.
        merge_items(
            &mut self.find.lock().unwrap().shared.items,
            items_from_cache(&rows),
        );
        self.push_shared();
        if self.offline.load(Ordering::Relaxed) {
            return;
        }
        // The shared items and a channel's folder (team lookup, `filesFolder`, its children) do
        // not depend on each other: asked one after the other, the channel tab waited for both.
        let (shared, folder) = tokio::join!(self.client.shared_items(&self.me, &conv), async {
            if is_channel {
                self.channel_folder_items(&conv, seq).await
            } else {
                vec![]
            }
        });
        let mut remote: Vec<Item> = vec![];
        match shared {
            Ok(list) => remote.extend(list.into_iter().map(Item::from)),
            Err(err) => tracing::warn!(%err, "shared items failed"),
        }
        remote.extend(folder);
        {
            let mut f = self.find.lock().unwrap();
            if f.shared.seq != seq {
                return;
            }
            // The server list leads, the cache fills in what it does not know. A row of the folder
            // the server no longer lists is gone, except the one just made (a listing can lag).
            let mut cached = std::mem::take(&mut f.shared.items);
            let made = f.shared.highlight.clone();
            cached.retain(|i| i.drive.is_none() || i.url == made);
            f.shared.items = remote;
            merge_items(&mut f.shared.items, cached);
            f.shared.loading = false;
        }
        self.push_shared();
    }

    /// The Graph group id of the team a channel belongs to (looked up once per team).
    pub(super) async fn channel_group(self: &Arc<Self>, channel: &str) -> Option<String> {
        let team = self
            .store
            .lock()
            .unwrap()
            .conversation(channel)
            .ok()
            .flatten()
            .and_then(|c| c.team_id)?;
        let looked_up = self.find.lock().unwrap().shared.groups.get(&team).cloned();
        // The aggregator's team list (read at start) knows it too: no Graph team list then.
        let known = looked_up.or_else(|| self.lists_channel_group(channel));
        if known.is_some() {
            return known;
        }
        let teams = self.client.teams_and_channels().await.ok()?;
        let mut f = self.find.lock().unwrap();
        for t in &teams {
            f.shared.groups.insert(t.id.clone(), t.group_id.clone());
        }
        f.shared.groups.get(&team).cloned()
    }

    /// The files of a channel's SharePoint folder (Graph `filesFolder`, then its children). Once
    /// the folder is known the toolbar (New folder, New document) shows, before the listing.
    async fn channel_folder_items(self: &Arc<Self>, channel: &str, seq: u64) -> Vec<Item> {
        let Some(group) = self.channel_group(channel).await else {
            return vec![];
        };
        let folder = match self.client.channel_files_folder(&group, channel).await {
            Ok(f) => f,
            Err(err) => {
                tracing::warn!(%err, "channel files folder failed");
                return vec![];
            }
        };
        {
            let mut f = self.find.lock().unwrap();
            if f.shared.seq != seq {
                return vec![];
            }
            f.shared.folder = Some(onedrive::ItemRef::new(&folder.drive_id, &folder.item_id));
        }
        self.push_shared();
        match onedrive::children(&self.client, &folder.drive_id, &folder.item_id, None).await {
            Ok(page) => page.items.into_iter().map(Item::from_drive).collect(),
            Err(err) => {
                tracing::warn!(%err, "channel files failed");
                vec![]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, ms: i64, html: &str, files: &str) -> MessageRow {
        MessageRow {
            id: id.into(),
            conversation_id: "c".into(),
            parent_id: None,
            sender_mri: "8:orgid:a".into(),
            sender_name: "Sara".into(),
            compose_ms: ms,
            edited_ms: None,
            deleted: false,
            message_type: "RichText/Html".into(),
            content_html: html.into(),
            client_message_id: None,
            reactions_json: format!("{{\"f\": {files}}}"),
        }
    }

    #[test]
    fn cached_messages_give_files_and_links_newest_first() {
        let rows = vec![
            msg(
                "1",
                100,
                "<p>see <a href=\"https://example.com/a?x=1&amp;y=2\">the doc</a></p>",
                "[]",
            ),
            msg(
                "2",
                200,
                "<p>file</p>",
                r#"[{"name":"plan.pdf","url":"https://x/plan.pdf","type":"pdf"}]"#,
            ),
            msg(
                "3",
                300,
                "<p>again <a href=\"https://example.com/a?x=1&amp;y=2\">dup</a></p>",
                "[]",
            ),
        ];
        let items = items_from_cache(&rows);
        assert_eq!(items.len(), 2, "the repeated link counts once");
        assert_eq!(items[0].kind, SharedKind::File);
        assert_eq!(
            (items[0].name.as_str(), items[0].ext.as_str()),
            ("plan.pdf", "PDF")
        );
        assert_eq!(items[1].kind, SharedKind::Link);
        assert_eq!(items[1].url, "https://example.com/a?x=1&y=2");
        assert_eq!(items[1].name, "the doc");
        assert_eq!(items[1].shared_by, "Sara");
    }

    #[test]
    fn only_web_links_count() {
        assert!(
            links_of("<a href=\"mailto:a@b.c\">mail</a><a href=\"kelma-media:1\">x</a>").is_empty()
        );
        assert_eq!(
            links_of("<a href=\"https://e.com\"></a>"),
            [("https://e.com".to_string(), String::new())]
        );
    }

    #[test]
    fn filters_and_keyword_narrow_the_list() {
        let items = vec![
            Item {
                kind: SharedKind::File,
                name: "Budget.xlsx".into(),
                ext: "XLSX".into(),
                shared_by: "Sara".into(),
                shared_ms: 3,
                url: "u1".into(),
                drive: None,
                busy: "",
            },
            Item {
                kind: SharedKind::Link,
                name: "Wiki".into(),
                ext: String::new(),
                shared_by: "Omar".into(),
                shared_ms: 2,
                url: "u2".into(),
                drive: None,
                busy: "",
            },
        ];
        assert_eq!(visible(&items, "recent", "").len(), 2);
        assert_eq!(visible(&items, "files", "")[0].name, "Budget.xlsx");
        assert_eq!(visible(&items, "links", "")[0].name, "Wiki");
        assert_eq!(
            visible(&items, "recent", "omar").len(),
            1,
            "the keyword also matches who shared"
        );
        assert!(visible(&items, "files", "wiki").is_empty());
    }

    #[test]
    fn merge_keeps_one_item_per_address() {
        let a = |url: &str, ms| Item {
            kind: SharedKind::Link,
            name: url.into(),
            ext: String::new(),
            shared_by: String::new(),
            shared_ms: ms,
            url: url.into(),
            drive: None,
            busy: "",
        };
        let mut items = vec![a("u1", 1)];
        merge_items(&mut items, vec![a("u1", 9), a("u2", 5)]);
        assert_eq!(
            items.iter().map(|i| i.url.as_str()).collect::<Vec<_>>(),
            ["u2", "u1"]
        );
    }

    #[test]
    fn extensions_come_from_the_name() {
        assert_eq!(ext_of("report.final.docx"), "DOCX");
        assert_eq!(ext_of("README"), "FILE");
        assert_eq!(ext_of("a.verylongext"), "FILE");
    }

    fn drive(name: &str, url: &str, folder: bool) -> onedrive::DriveItem {
        onedrive::DriveItem {
            name: name.into(),
            web_url: url.into(),
            is_folder: folder,
            modified_ms: 5,
            ..Default::default()
        }
    }

    fn state(items: Vec<onedrive::DriveItem>) -> State {
        State {
            items: items.into_iter().map(Item::from_drive).collect(),
            filter: "recent".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_delete_dims_the_row_at_once_then_drops_it_or_brings_it_back() {
        let mut s = state(vec![drive("a.docx", "u1", false), drive("b", "u2", true)]);
        s.set_busy("u1", "Deleting…");
        assert_eq!(s.items[0].busy, "Deleting…");
        assert_eq!(data(&s.items[0], "").busy, "Deleting…");
        s.delete_done("u1", false);
        assert_eq!(s.items.len(), 2, "a failed delete keeps the row");
        assert_eq!(s.items[0].busy, "");
        s.set_busy("u1", "Deleting…");
        s.delete_done("u1", true);
        assert_eq!(
            s.items.iter().map(|i| i.url.as_str()).collect::<Vec<_>>(),
            ["u2"]
        );
    }

    #[test]
    fn a_rename_shows_the_new_name_at_once_and_the_old_one_if_it_fails() {
        let mut s = state(vec![drive("Plan.docx", "u1", false)]);
        assert_eq!(
            s.rename_now("u1", "Plan.xlsx").as_deref(),
            Some("Plan.docx")
        );
        assert_eq!(
            (s.items[0].name.as_str(), s.items[0].ext.as_str()),
            ("Plan.xlsx", "XLSX")
        );
        s.rename_done("u1", Err("Plan.docx".into()));
        assert_eq!(
            (s.items[0].name.as_str(), s.items[0].ext.as_str()),
            ("Plan.docx", "DOCX")
        );
        // The server's item replaces the row (its address follows the name).
        s.rename_now("u1", "Plan v2.docx");
        s.rename_done("u1", Ok(drive("Plan v2.docx", "u1-v2", false)));
        assert_eq!(s.items[0].url, "u1-v2");
        assert_eq!(s.items[0].name, "Plan v2.docx");
        assert!(s.items[0].drive.is_some());
    }

    #[test]
    fn a_created_item_comes_first_under_files_and_is_highlighted() {
        let mut s = state(vec![drive("old.docx", "u1", false)]);
        // A file shared later than the folder is made still comes after it.
        s.items.insert(
            0,
            Item::from(SharedItem {
                kind: SharedKind::File,
                name: "Wiki.docx".into(),
                url: "u9".into(),
                extension: "docx".into(),
                shared_by: "Sara".into(),
                shared_by_mri: String::new(),
                shared_ms: i64::MAX,
                message_id: String::new(),
            }),
        );
        s.keyword = "zzz".into();
        let url = s.add_created(drive("Plans", "u2", true));
        assert_eq!(url, "u2");
        assert_eq!(s.filter, "files");
        assert_eq!(s.highlight, "u2");
        let rows = visible(&s.items, &s.filter, &s.keyword);
        assert_eq!(rows[0].name, "Plans", "first, and not hidden by a keyword");
        assert!(data(&rows[0], &s.highlight).highlight);
        assert!(!data(&rows[1], &s.highlight).highlight);
    }

    #[test]
    fn rows_get_a_folder_icon_or_a_document_kind() {
        let s = state(vec![
            drive("Plans", "u0", true),
            drive("a.docx", "u1", false),
            drive("b.XLSX", "u2", false),
            drive("c.pptx", "u3", false),
            drive("d.pdf", "u4", false),
            drive("e.png", "u5", false),
            drive("f.zip", "u6", false),
            drive("g.weird", "u7", false),
        ]);
        let icons: Vec<_> = s.items.iter().map(icon_of).collect();
        assert_eq!(
            icons,
            [
                "folder",
                "word",
                "excel",
                "powerpoint",
                "pdf",
                "image",
                "archive",
                "other"
            ]
        );
        assert_eq!(s.items[0].ext, "DIR");
        // A file shared in a message has no drive item: its extension decides.
        let shared = items_from_cache(&[msg(
            "1",
            1,
            "<p>x</p>",
            r#"[{"name":"notes.pptx","url":"https://x/n.pptx","type":"pptx"}]"#,
        )]);
        assert_eq!(icon_of(&shared[0]), "powerpoint");
    }
}
