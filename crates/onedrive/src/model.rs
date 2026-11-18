//! Drive item model and the pure parsers for Graph and Search responses.

use serde_json::Value;
use teams_proto::time::parse_iso_ms;

/// A file, folder or shortcut in OneDrive or a SharePoint library.
///
/// For a remote shortcut or an item from "shared with me", `id` and `drive_id` point at the
/// target item (what you browse or download), not at the stub in your own drive.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DriveItem {
    pub id: String,
    pub drive_id: String,
    pub name: String,
    pub size: u64,
    pub is_folder: bool,
    pub child_count: u32,
    pub mime: String,
    pub modified_ms: i64,
    pub modified_by: String,
    pub created_by: String,
    pub web_url: String,
    /// Folder path inside the drive, for example `/Documents/Reports` (empty at the root).
    pub parent_path: String,
    pub thumbnail_url: Option<String>,
    pub is_remote_shortcut: bool,
    /// For a remote item: the stub in the listing's own drive as `(drive id, item id)`. Delete
    /// or rename this, never the target (`id`), to remove a shortcut or a shared entry.
    pub stub: Option<(String, String)>,
}

/// One page of results and the link to the next one.
#[derive(Debug, Clone, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_link: Option<String>,
}

impl<T> Default for Page<T> {
    fn default() -> Self {
        Page {
            items: Vec::new(),
            next_link: None,
        }
    }
}

fn str_at<'a>(v: &'a Value, path: &str) -> Option<&'a str> {
    v.pointer(path).and_then(Value::as_str)
}

/// Display name of the user or application behind an `identitySet`.
fn identity(v: &Value, key: &str) -> String {
    let set = &v[key];
    [
        "/user/displayName",
        "/application/displayName",
        "/user/email",
    ]
    .iter()
    .find_map(|p| str_at(set, p))
    .unwrap_or_default()
    .to_owned()
}

/// `/drive/root:/Documents/Sub` or `/drives/b!x/root:/Sub` to `/Documents/Sub`.
fn parent_path(parent: &Value) -> String {
    let p = parent["path"].as_str().unwrap_or_default();
    match p.split_once("root:") {
        Some((_, rest)) => rest.to_owned(),
        None => String::new(),
    }
}

fn thumbnail(v: &Value) -> Option<String> {
    let set = v["thumbnails"].as_array()?.first()?;
    ["medium", "large", "small"]
        .iter()
        .find_map(|s| str_at(set, &format!("/{s}/url")))
        .map(str::to_owned)
}

/// Parses a Graph `driveItem`. Items without an id or a name are dropped.
pub fn parse_item(v: &Value) -> Option<DriveItem> {
    let remote = v.get("remoteItem").filter(|r| r.is_object());
    // Navigation targets live on the remote item; metadata shown in lists is the stub's own
    // when present, else the target's.
    let target = remote.unwrap_or(v);
    let id = target["id"].as_str().or(v["id"].as_str())?;
    let name = v["name"].as_str().or(target["name"].as_str())?;
    let parent = &target["parentReference"];
    let drive_id = parent["driveId"]
        .as_str()
        .or(v["parentReference"]["driveId"].as_str())
        .unwrap_or_default();
    let folder = target.get("folder").or(v.get("folder"));
    let file = target.get("file").or(v.get("file"));
    let modified = str_at(v, "/lastModifiedDateTime")
        .or(str_at(target, "/lastModifiedDateTime"))
        .and_then(parse_iso_ms)
        .unwrap_or(0);
    let by = |key: &str| {
        let own = identity(v, key);
        if own.is_empty() {
            identity(target, key)
        } else {
            own
        }
    };
    Some(DriveItem {
        id: id.to_owned(),
        drive_id: drive_id.to_owned(),
        name: name.to_owned(),
        size: target["size"].as_u64().or(v["size"].as_u64()).unwrap_or(0),
        is_folder: folder.is_some(),
        child_count: folder
            .and_then(|f| f["childCount"].as_u64())
            .unwrap_or(0)
            .try_into()
            .unwrap_or(u32::MAX),
        mime: file
            .and_then(|f| f["mimeType"].as_str())
            .unwrap_or_default()
            .to_owned(),
        modified_ms: modified,
        modified_by: by("lastModifiedBy"),
        created_by: by("createdBy"),
        web_url: str_at(target, "/webUrl")
            .or(str_at(v, "/webUrl"))
            .unwrap_or_default()
            .to_owned(),
        parent_path: parent_path(parent),
        thumbnail_url: thumbnail(v),
        is_remote_shortcut: remote.is_some(),
        stub: remote.and(v["id"].as_str()).map(|id| {
            (
                v["parentReference"]["driveId"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                id.to_owned(),
            )
        }),
    })
}

/// Parses a Graph collection (`value` plus `@odata.nextLink`).
pub fn parse_page(v: &Value) -> Page<DriveItem> {
    Page {
        items: v["value"]
            .as_array()
            .map(|a| a.iter().filter_map(parse_item).collect())
            .unwrap_or_default(),
        next_link: v["@odata.nextLink"].as_str().map(str::to_owned),
    }
}

/// Result of a Graph Search request: the items of every hit container and whether more exist.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SearchPage {
    pub items: Vec<DriveItem>,
    pub more: bool,
}

/// True for a name like `Notes.md` (not `1_.000`): the extension has a letter in it.
fn has_file_extension(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && (1..=5).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
            && ext.chars().any(|c| c.is_ascii_alphabetic())
    })
}

/// Parses the response of `POST /search/query` for `driveItem` and `listItem` hits.
///
/// A `listItem` hit has no `name`; its file name is `fields.fileLeafRef`, or the last segment of
/// its `webUrl`. Hits that are not files or folders (list rows without a file) are dropped.
pub fn parse_search(v: &Value) -> SearchPage {
    let mut out = SearchPage::default();
    let Some(responses) = v["value"].as_array() else {
        return out;
    };
    for r in responses {
        for c in r["hitsContainers"].as_array().into_iter().flatten() {
            out.more |= c["moreResultsAvailable"].as_bool().unwrap_or(false);
            for h in c["hits"].as_array().into_iter().flatten() {
                let res = &h["resource"];
                let mut item = res.clone();
                if item["name"].is_null() {
                    let leaf = str_at(res, "/fields/fileLeafRef")
                        .map(str::to_owned)
                        .or_else(|| {
                            str_at(res, "/webUrl")
                                .and_then(|u| u.rsplit('/').next())
                                .filter(|s| has_file_extension(s))
                                .map(str::to_owned)
                        });
                    if let (Some(leaf), Some(o)) = (leaf, item.as_object_mut()) {
                        o.insert("name".into(), Value::String(leaf));
                    }
                }
                // List item ids are not drive item ids: use the drive item behind them.
                if let (Some(o), Some(id)) = (item.as_object_mut(), res["driveItem"]["id"].as_str())
                {
                    o.insert("id".into(), Value::String(id.to_owned()));
                }
                if let Some(it) = parse_item(&item) {
                    out.items.push(it);
                }
            }
        }
    }
    out
}

/// A drive (library): a user's OneDrive, a team's Documents library, a site library.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Drive {
    pub id: String,
    pub name: String,
    pub web_url: String,
    pub drive_type: String,
}

/// A SharePoint site.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Site {
    pub id: String,
    pub name: String,
    pub web_url: String,
}

/// Parses one drive object.
pub fn parse_drive(v: &Value) -> Option<Drive> {
    Some(Drive {
        id: v["id"].as_str()?.to_owned(),
        name: v["name"].as_str().unwrap_or_default().to_owned(),
        web_url: v["webUrl"].as_str().unwrap_or_default().to_owned(),
        drive_type: v["driveType"].as_str().unwrap_or_default().to_owned(),
    })
}

/// Parses one site object (`displayName` or `name`).
pub fn parse_site(v: &Value) -> Option<Site> {
    Some(Site {
        id: v["id"].as_str()?.to_owned(),
        name: v["displayName"]
            .as_str()
            .or(v["name"].as_str())
            .unwrap_or_default()
            .to_owned(),
        web_url: v["webUrl"].as_str().unwrap_or_default().to_owned(),
    })
}

/// Thumbnail URLs of one item (the first thumbnail set).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Thumbnails {
    pub small: Option<String>,
    pub medium: Option<String>,
    pub large: Option<String>,
}

/// Parses `GET …/thumbnails`.
pub fn parse_thumbnails(v: &Value) -> Thumbnails {
    let set = &v["value"][0];
    let url = |s: &str| str_at(set, &format!("/{s}/url")).map(str::to_owned);
    Thumbnails {
        small: url("small"),
        medium: url("medium"),
        large: url("large"),
    }
}

/// A column to sort or group a file list by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Modified,
    ModifiedBy,
    Size,
}

/// Sorts in place. Folders always come first, as in OneDrive; `ascending` applies within each
/// kind. Names compare case-insensitively, and ties fall back to the name.
pub fn sort_items(items: &mut [DriveItem], key: SortKey, ascending: bool) {
    items.sort_by(|a, b| {
        let by_key = match key {
            SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortKey::Modified => a.modified_ms.cmp(&b.modified_ms),
            SortKey::ModifiedBy => a
                .modified_by
                .to_lowercase()
                .cmp(&b.modified_by.to_lowercase()),
            SortKey::Size => a.size.cmp(&b.size),
        }
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        b.is_folder
            .cmp(&a.is_folder)
            .then(if ascending { by_key } else { by_key.reverse() })
    });
}

/// A labelled run of items.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub label: String,
    pub items: Vec<DriveItem>,
}

const DAY_MS: i64 = 86_400_000;

fn modified_bucket(modified_ms: i64, now_ms: i64) -> &'static str {
    let days = now_ms.div_euclid(DAY_MS) - modified_ms.div_euclid(DAY_MS);
    match days {
        i64::MIN..=0 => "Today",
        1 => "Yesterday",
        2..=6 => "Earlier this week",
        7..=13 => "Last week",
        14..=30 => "Earlier this month",
        _ => "Older",
    }
}

fn size_bucket(it: &DriveItem) -> &'static str {
    match it.size {
        _ if it.is_folder => "Folders",
        0..=102_399 => "Small (under 100 KB)",
        102_400..=10_485_759 => "Medium (100 KB to 10 MB)",
        _ => "Large (10 MB and over)",
    }
}

/// Groups items by consecutive equal labels, keeping the order of `items` (sort first). The
/// label is the first letter for Name, a relative date for Modified, the person for ModifiedBy
/// and a size class for Size.
pub fn group_items(items: &[DriveItem], key: SortKey, now_ms: i64) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for it in items {
        let label = match key {
            SortKey::Name => it
                .name
                .chars()
                .next()
                .filter(|c| c.is_alphabetic())
                .map(|c| c.to_uppercase().collect::<String>())
                .unwrap_or_else(|| "#".into()),
            SortKey::Modified => modified_bucket(it.modified_ms, now_ms).into(),
            SortKey::ModifiedBy if it.modified_by.is_empty() => "Unknown".into(),
            SortKey::ModifiedBy => it.modified_by.clone(),
            SortKey::Size => size_bucket(it).into(),
        };
        match groups.last_mut() {
            Some(g) if g.label == label => g.items.push(it.clone()),
            _ => groups.push(Group {
                label,
                items: vec![it.clone()],
            }),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn file() -> Value {
        json!({
            "id": "01FILE",
            "name": "Budget 2026.xlsx",
            "size": 20480,
            "webUrl": "https://contoso-my.sharepoint.com/personal/ada_contoso_com/Documents/Budget%202026.xlsx",
            "lastModifiedDateTime": "2026-11-10T08:30:00Z",
            "createdBy": {"user": {"displayName": "Ada Lovelace", "email": "ada@contoso.com"}},
            "lastModifiedBy": {"user": {"displayName": "Grace Hopper"}},
            "parentReference": {"driveId": "b!drive1", "id": "01PARENT", "path": "/drive/root:/Documents/Reports"},
            "file": {"mimeType": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"},
            "thumbnails": [{"id": "0", "small": {"url": "https://t/s"}, "medium": {"url": "https://t/m"}, "large": {"url": "https://t/l"}}]
        })
    }

    #[test]
    fn parses_a_file() {
        let it = parse_item(&file()).unwrap();
        assert_eq!(it.id, "01FILE");
        assert_eq!(it.drive_id, "b!drive1");
        assert_eq!(it.size, 20480);
        assert!(!it.is_folder && !it.is_remote_shortcut && it.stub.is_none());
        assert!(it.mime.ends_with("spreadsheetml.sheet"));
        assert_eq!(it.modified_by, "Grace Hopper");
        assert_eq!(it.created_by, "Ada Lovelace");
        assert_eq!(it.parent_path, "/Documents/Reports");
        assert_eq!(it.thumbnail_url.as_deref(), Some("https://t/m"));
        assert_eq!(it.modified_ms, 1_794_299_400_000);
    }

    #[test]
    fn parses_a_folder_and_root_path() {
        let it = parse_item(&json!({
            "id": "01DIR", "name": "Docs", "folder": {"childCount": 7},
            "parentReference": {"driveId": "d", "path": "/drive/root:"},
            "createdBy": {"application": {"displayName": "Teams"}}
        }))
        .unwrap();
        assert!(it.is_folder);
        assert_eq!(it.child_count, 7);
        assert_eq!(it.parent_path, "");
        assert_eq!(it.created_by, "Teams");
        assert_eq!(it.thumbnail_url, None);
    }

    #[test]
    fn remote_shortcut_points_at_the_target() {
        let it = parse_item(&json!({
            "id": "01STUB", "name": "Shared plan.docx",
            "parentReference": {"driveId": "mine"},
            "lastModifiedDateTime": "2026-11-01T00:00:00Z",
            "remoteItem": {
                "id": "01TARGET", "name": "Shared plan.docx", "size": 99,
                "webUrl": "https://contoso.sharepoint.com/x.docx",
                "parentReference": {"driveId": "b!theirs"},
                "file": {"mimeType": "application/msword"}
            }
        }))
        .unwrap();
        assert_eq!(
            (it.id.as_str(), it.drive_id.as_str()),
            ("01TARGET", "b!theirs")
        );
        assert!(it.is_remote_shortcut);
        assert_eq!(it.stub, Some(("mine".into(), "01STUB".into())));
        assert_eq!(it.size, 99);
        assert_eq!(it.web_url, "https://contoso.sharepoint.com/x.docx");
        assert_eq!(it.mime, "application/msword");
    }

    #[test]
    fn items_without_id_or_name_are_dropped() {
        assert!(parse_item(&json!({"name": "x"})).is_none());
        assert!(parse_item(&json!({"id": "x"})).is_none());
        let page = parse_page(&json!({
            "value": [file(), {"nope": 1}],
            "@odata.nextLink": "https://graph.microsoft.com/v1.0/next"
        }));
        assert_eq!(page.items.len(), 1);
        assert_eq!(
            page.next_link.as_deref(),
            Some("https://graph.microsoft.com/v1.0/next")
        );
        assert_eq!(parse_page(&json!({})), Page::default());
    }

    #[test]
    fn search_hits_for_drive_and_list_items() {
        let v = json!({"value": [{"hitsContainers": [{
        "moreResultsAvailable": true,
        "hits": [
            {"hitId": "1", "resource": {
                "@odata.type": "#microsoft.graph.driveItem", "id": "01A", "name": "Plan.pptx",
                "size": 5, "webUrl": "https://c.sharepoint.com/Plan.pptx",
                "parentReference": {"driveId": "b!d", "siteId": "s"}}},
            {"hitId": "2", "resource": {
                "@odata.type": "#microsoft.graph.listItem", "id": "listrow-9",
                "webUrl": "https://c.sharepoint.com/sites/t/Shared%20Documents/Notes.md",
                "fields": {"fileLeafRef": "Notes.md"},
                "driveItem": {"id": "01B"},
                "parentReference": {"driveId": "b!e"}}},
            {"hitId": "3", "resource": {"@odata.type": "#microsoft.graph.listItem", "id": "row",
                "webUrl": "https://c.sharepoint.com/sites/t/Lists/Tasks/1_.000"}}
        ]}]}]});
        let p = parse_search(&v);
        assert!(p.more);
        assert_eq!(p.items.len(), 2);
        assert_eq!(p.items[0].name, "Plan.pptx");
        assert_eq!(p.items[1].name, "Notes.md");
        assert_eq!(p.items[1].id, "01B");
        assert_eq!(p.items[1].drive_id, "b!e");
        assert_eq!(parse_search(&json!({})), SearchPage::default());
    }

    #[test]
    fn drives_sites_and_thumbnails() {
        let d = parse_drive(&json!({"id": "b!x", "name": "Documents", "webUrl": "u", "driveType": "documentLibrary"})).unwrap();
        assert_eq!(
            (d.name.as_str(), d.drive_type.as_str()),
            ("Documents", "documentLibrary")
        );
        assert!(parse_drive(&json!({"name": "x"})).is_none());
        let s =
            parse_site(&json!({"id": "c,1,2", "displayName": "Team A", "webUrl": "w"})).unwrap();
        assert_eq!(s.name, "Team A");
        let t =
            parse_thumbnails(&json!({"value": [{"small": {"url": "a"}, "large": {"url": "c"}}]}));
        assert_eq!(t.small.as_deref(), Some("a"));
        assert_eq!(t.medium, None);
        assert_eq!(
            parse_thumbnails(&json!({"value": []})),
            Thumbnails::default()
        );
    }

    fn it(name: &str, folder: bool, size: u64, ms: i64, by: &str) -> DriveItem {
        DriveItem {
            name: name.into(),
            is_folder: folder,
            size,
            modified_ms: ms,
            modified_by: by.into(),
            ..Default::default()
        }
    }

    fn names(v: &[DriveItem]) -> Vec<&str> {
        v.iter().map(|i| i.name.as_str()).collect()
    }

    #[test]
    fn sorting_keeps_folders_first() {
        let mut v = vec![
            it("b.txt", false, 5, 30, "Zed"),
            it("Zdir", true, 0, 10, "Amy"),
            it("A.txt", false, 9, 20, "amy"),
            it("adir", true, 0, 50, "Bob"),
        ];
        sort_items(&mut v, SortKey::Name, true);
        assert_eq!(names(&v), ["adir", "Zdir", "A.txt", "b.txt"]);
        sort_items(&mut v, SortKey::Name, false);
        assert_eq!(names(&v), ["Zdir", "adir", "b.txt", "A.txt"]);
        sort_items(&mut v, SortKey::Modified, false);
        assert_eq!(names(&v), ["adir", "Zdir", "b.txt", "A.txt"]);
        sort_items(&mut v, SortKey::Size, true);
        assert_eq!(names(&v), ["adir", "Zdir", "b.txt", "A.txt"]);
        sort_items(&mut v, SortKey::ModifiedBy, true);
        assert_eq!(names(&v), ["Zdir", "adir", "A.txt", "b.txt"]);
    }

    #[test]
    fn grouping_labels() {
        let now = 40 * DAY_MS + 5 * 3_600_000;
        let v = vec![
            it("Alpha", false, 10, now - 1000, "Amy"),
            it("apple", false, 10, now - DAY_MS, "Amy"),
            it("Beta", false, 20_000_000, now - 3 * DAY_MS, "Bob"),
            it("9lives", false, 200_000, now - 20 * DAY_MS, ""),
            it("dir", true, 0, now - 39 * DAY_MS, "Bob"),
        ];
        let g = |k| {
            group_items(&v, k, now)
                .into_iter()
                .map(|g| (g.label, g.items.len()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            g(SortKey::Name),
            [
                ("A".into(), 2),
                ("B".into(), 1),
                ("#".into(), 1),
                ("D".into(), 1)
            ]
        );
        assert_eq!(
            g(SortKey::Modified),
            [
                ("Today".into(), 1),
                ("Yesterday".into(), 1),
                ("Earlier this week".into(), 1),
                ("Earlier this month".into(), 1),
                ("Older".into(), 1)
            ]
        );
        assert_eq!(g(SortKey::ModifiedBy)[0], ("Amy".into(), 2));
        assert_eq!(g(SortKey::ModifiedBy)[2], ("Unknown".into(), 1));
        assert_eq!(g(SortKey::Size)[1], ("Large (10 MB and over)".into(), 1));
        assert_eq!(g(SortKey::Size)[3], ("Folders".into(), 1));
    }
}
