//! Acting on files: create, upload, rename, move, copy, delete, download, favorites,
//! shortcuts, versions and preview.

use std::path::Path;
use std::time::Duration;

use reqwest::Method;
use serde_json::{Value, json};
use teams_proto::client::RawBody;
use teams_proto::files::{SIMPLE_UPLOAD_MAX, encode_path_segment};
use teams_proto::time::parse_iso_ms;
use teams_proto::{Client, Error, Result};

use crate::api::{ItemRef, graph, graph_raw};
use crate::browse::{children_of, my_drive};
use crate::model::{DriveItem, parse_item};
use crate::package::{DocKind, zip_bytes};

/// Never overwrite: a clash gets a numbered name.
const RENAME_ON_CONFLICT: &str = "@microsoft.graph.conflictBehavior";

/// Body of `POST …/children` that creates a folder.
pub fn folder_body(name: &str) -> Value {
    json!({"name": name, "folder": {}, RENAME_ON_CONFLICT: "rename"})
}

/// Body of `PATCH` that renames an item.
pub fn rename_body(name: &str) -> Value {
    json!({"name": name})
}

/// Body of `PATCH` that moves an item (same drive), optionally renaming it.
pub fn move_body(parent_id: &str, name: Option<&str>) -> Value {
    let mut b = json!({"parentReference": {"id": parent_id}});
    if let Some(n) = name {
        b["name"] = json!(n);
    }
    b
}

/// Body of `POST …/copy`.
pub fn copy_body(drive_id: &str, parent_id: &str, name: Option<&str>) -> Value {
    let mut b = json!({"parentReference": {"driveId": drive_id, "id": parent_id}});
    if let Some(n) = name {
        b["name"] = json!(n);
    }
    b
}

/// Body of `POST …/children` that adds a shortcut to `target` (a OneDrive "Add shortcut").
pub fn shortcut_body(name: &str, target_drive_id: &str, target_id: &str) -> Value {
    json!({
        "name": name,
        "remoteItem": {"id": target_id, "parentReference": {"driveId": target_drive_id}},
        RENAME_ON_CONFLICT: "rename",
    })
}

/// URL that uploads up to [`SIMPLE_UPLOAD_MAX`] bytes in one `PUT`.
pub fn upload_url(parent: &ItemRef, name: &str) -> String {
    format!(
        "{}/content?{RENAME_ON_CONFLICT}=rename",
        parent.child_url(name)
    )
}

/// URL that opens an upload session for a larger file.
pub fn upload_session_url(parent: &ItemRef, name: &str) -> String {
    format!("{}/createUploadSession", parent.child_url(name))
}

/// State of an asynchronous copy.
#[derive(Debug, Clone, PartialEq)]
pub enum Monitor {
    Pending,
    /// Finished: the id of the new item.
    Done(String),
    Failed(String),
}

/// Reads a copy monitor answer. A finished job is either a JSON `status: completed` with
/// `resourceId`, or a 303 whose `Location` ends in `/items/<id>`.
pub fn parse_monitor(status: u16, location: Option<&str>, body: &str) -> Monitor {
    if status == 303 || status == 201 {
        let id = location
            .and_then(|l| l.split('?').next())
            .and_then(|l| l.rsplit('/').next())
            .filter(|s| !s.is_empty());
        return match id {
            Some(id) => Monitor::Done(id.to_owned()),
            None => Monitor::Failed("copy finished without an item id".into()),
        };
    }
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    match v["status"].as_str() {
        Some("completed") => match v["resourceId"].as_str() {
            Some(id) => Monitor::Done(id.to_owned()),
            None => Monitor::Failed("copy finished without an item id".into()),
        },
        Some("failed") => Monitor::Failed(
            v["error"]["message"]
                .as_str()
                .unwrap_or("the copy failed")
                .to_owned(),
        ),
        _ if status >= 400 => Monitor::Failed(format!("monitor answered HTTP {status}")),
        _ => Monitor::Pending,
    }
}

/// One saved version of a file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Version {
    pub id: String,
    pub modified_ms: i64,
    pub modified_by: String,
    pub size: u64,
}

/// Parses `GET …/versions`, newest first as Graph returns them.
pub fn parse_versions(v: &Value) -> Vec<Version> {
    v["value"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| {
            Some(Version {
                id: x["id"].as_str()?.to_owned(),
                modified_ms: x["lastModifiedDateTime"]
                    .as_str()
                    .and_then(parse_iso_ms)
                    .unwrap_or(0),
                modified_by: x["lastModifiedBy"]["user"]["displayName"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                size: x["size"].as_u64().unwrap_or(0),
            })
        })
        .collect()
}

/// The embeddable preview URL from `POST …/preview` (`getUrl`, else `postUrl`).
pub fn parse_preview(v: &Value) -> Option<String> {
    v["getUrl"]
        .as_str()
        .or(v["postUrl"].as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn one(v: &Value, what: &str) -> Result<DriveItem> {
    parse_item(v).ok_or_else(|| Error::Protocol(format!("{what} returned no drive item")))
}

/// Drive id and item id of `r`, looking them up when it is a root or the user's own drive.
async fn resolve(client: &Client, r: &ItemRef) -> Result<(String, String)> {
    let drive_id = if r.drive_id.is_empty() {
        my_drive(client).await?.id
    } else {
        r.drive_id.clone()
    };
    let id = if r.item_id == "root" {
        let v = graph(
            client,
            "root_id",
            Method::GET,
            &format!("{}?$select=id", r.url()),
            None,
        )
        .await?;
        v["id"]
            .as_str()
            .ok_or_else(|| Error::Protocol("drive root has no id".into()))?
            .to_owned()
    } else {
        r.item_id.clone()
    };
    Ok((drive_id, id))
}

/// Creates a folder (a clash gets a numbered name).
pub async fn create_folder(client: &Client, parent: &ItemRef, name: &str) -> Result<DriveItem> {
    let v = graph(
        client,
        "create_folder",
        Method::POST,
        &format!("{}/children", parent.url()),
        Some(&folder_body(name)),
    )
    .await?;
    one(&v, "create_folder")
}

/// Uploads a file: one `PUT` up to 4 MB, an upload session above that.
pub async fn upload_file(
    client: &Client,
    parent: &ItemRef,
    name: &str,
    data: Vec<u8>,
) -> Result<DriveItem> {
    let stored = if data.len() <= SIMPLE_UPLOAD_MAX {
        client
            .put_bytes(&upload_url(parent, name), data, None)
            .await?
    } else {
        let body = json!({"item": {RENAME_ON_CONFLICT: "rename"}});
        let s = graph(
            client,
            "upload_session",
            Method::POST,
            &upload_session_url(parent, name),
            Some(&body),
        )
        .await?;
        let url = s["uploadUrl"]
            .as_str()
            .ok_or_else(|| Error::Protocol("upload session has no uploadUrl".into()))?;
        client.upload_chunks(url, &data).await?
    };
    one(&stored, "upload")
}

/// What [`upload_folder`] created.
#[derive(Debug, Clone, PartialEq)]
pub struct FolderUpload {
    pub folder: DriveItem,
    pub folders: usize,
    pub files: usize,
}

fn io_err(path: &Path, e: std::io::Error) -> Error {
    Error::Protocol(format!("{}: {e}", path.display()))
}

/// Uploads a local folder with everything in it. Symbolic links are skipped.
pub async fn upload_folder(
    client: &Client,
    parent: &ItemRef,
    local: &Path,
) -> Result<FolderUpload> {
    let name = local
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Protocol(format!("{} has no folder name", local.display())))?;
    let root = create_folder(client, parent, name).await?;
    let mut report = FolderUpload {
        folder: root.clone(),
        folders: 1,
        files: 0,
    };
    // Depth-first with an explicit stack: async recursion would need boxing.
    let mut stack = vec![(local.to_path_buf(), ItemRef::from(&root))];
    while let Some((dir, remote)) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .map_err(|e| io_err(&dir, e))?
            .collect::<std::io::Result<_>>()
            .map_err(|e| io_err(&dir, e))?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let path = e.path();
            let kind = e.file_type().map_err(|e| io_err(&path, e))?;
            let n = e.file_name().to_string_lossy().into_owned();
            if kind.is_dir() {
                let sub = create_folder(client, &remote, &n).await?;
                report.folders += 1;
                stack.push((path, ItemRef::from(&sub)));
            } else if kind.is_file() {
                let data = std::fs::read(&path).map_err(|e| io_err(&path, e))?;
                upload_file(client, &remote, &n, data).await?;
                report.files += 1;
            }
        }
    }
    Ok(report)
}

/// Creates a blank document in `parent`. `name` is the file name (the kind's extension is added
/// when missing); `None` uses the kind's default name.
pub async fn create_office_doc(
    client: &Client,
    parent: &ItemRef,
    kind: DocKind,
    name: Option<&str>,
) -> Result<DriveItem> {
    let file_name = match name {
        Some(n) => kind.file_name(n),
        None => kind.default_name().to_owned(),
    };
    upload_file(client, parent, &file_name, kind.blank()).await
}

/// Renames an item.
pub async fn rename(client: &Client, item: &ItemRef, new_name: &str) -> Result<DriveItem> {
    let v = graph(
        client,
        "rename",
        Method::PATCH,
        &item.url(),
        Some(&rename_body(new_name)),
    )
    .await?;
    one(&v, "rename")
}

/// Moves an item into another folder of the same drive.
pub async fn move_to(
    client: &Client,
    item: &ItemRef,
    new_parent: &ItemRef,
    new_name: Option<&str>,
) -> Result<DriveItem> {
    let (_, parent_id) = resolve(client, new_parent).await?;
    let v = graph(
        client,
        "move",
        Method::PATCH,
        &item.url(),
        Some(&move_body(&parent_id, new_name)),
    )
    .await?;
    one(&v, "move")
}

/// How often the copy monitor is polled, and for how long.
const COPY_POLL: Duration = Duration::from_secs(1);
const COPY_POLL_MAX: u32 = 120;

/// Copies an item into a folder (any drive) and waits for the asynchronous job to finish.
pub async fn copy_to(
    client: &Client,
    item: &ItemRef,
    dest: &ItemRef,
    new_name: Option<&str>,
) -> Result<DriveItem> {
    let (dest_drive, dest_id) = resolve(client, dest).await?;
    let r = graph_raw(
        client,
        "copy",
        Method::POST,
        &format!("{}/copy", item.url()),
        RawBody::Json(&copy_body(&dest_drive, &dest_id, new_name)),
    )
    .await?;
    let monitor = r
        .headers
        .get("location")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| Error::Protocol("copy returned no monitor URL".into()))?
        .to_owned();
    // The monitor URL is pre-authenticated (no token) and a finished job answers with a
    // redirect to the item, which must not be followed without credentials.
    teams_proto::client::install_crypto_provider();
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    for _ in 0..COPY_POLL_MAX {
        // The monitor URL may carry a temporary token: keep it out of error text.
        let resp = http
            .get(&monitor)
            .send()
            .await
            .map_err(|e| Error::Http(e.without_url()))?;
        let status = resp.status().as_u16();
        let location = resp
            .headers()
            .get("location")
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned);
        let text = resp.text().await.unwrap_or_default();
        match parse_monitor(status, location.as_deref(), &text) {
            Monitor::Done(id) => {
                let new = ItemRef::new(&dest_drive, &id);
                let v = graph(client, "copied_item", Method::GET, &new.url(), None).await?;
                return one(&v, "copy");
            }
            Monitor::Failed(m) => return Err(Error::Protocol(format!("copy failed: {m}"))),
            Monitor::Pending => tokio::time::sleep(COPY_POLL).await,
        }
    }
    Err(Error::Protocol("copy did not finish in time".into()))
}

/// Deletes an item (it goes to the recycle bin).
pub async fn delete(client: &Client, item: &ItemRef) -> Result<()> {
    graph(client, "delete", Method::DELETE, &item.url(), None)
        .await
        .map(|_| ())
}

/// Downloads a file, or a folder as a zip built on the client.
pub async fn download(client: &Client, item: &DriveItem) -> Result<Vec<u8>> {
    if !item.is_folder {
        return download_file(client, &ItemRef::from(item)).await;
    }
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    // (folder, path prefix inside the zip)
    let mut stack = vec![(ItemRef::from(item), String::new())];
    while let Some((folder, prefix)) = stack.pop() {
        let mut next: Option<String> = None;
        let mut empty = true;
        loop {
            let page = children_of(client, &folder, next.as_deref()).await?;
            for c in &page.items {
                empty = false;
                let path = format!("{prefix}{}", c.name);
                if c.is_folder {
                    stack.push((ItemRef::from(c), format!("{path}/")));
                } else {
                    let data = download_file(client, &ItemRef::from(c)).await?;
                    entries.push((path, data));
                }
            }
            next = page.next_link;
            if next.is_none() {
                break;
            }
        }
        if empty && !prefix.is_empty() {
            entries.push((prefix, Vec::new()));
        }
    }
    zip_bytes(&entries)
}

async fn download_file(client: &Client, item: &ItemRef) -> Result<Vec<u8>> {
    Ok(graph_raw(
        client,
        "download",
        Method::GET,
        &format!("{}/content", item.url()),
        RawBody::None,
    )
    .await?
    .body)
}

/// Adds an item to Favorites.
pub async fn follow(client: &Client, item: &ItemRef) -> Result<()> {
    graph(
        client,
        "follow",
        Method::POST,
        &format!("{}/follow", item.url()),
        None,
    )
    .await
    .map(|_| ())
}

/// Removes an item from Favorites.
pub async fn unfollow(client: &Client, item: &ItemRef) -> Result<()> {
    graph(
        client,
        "unfollow",
        Method::POST,
        &format!("{}/unfollow", item.url()),
        None,
    )
    .await
    .map(|_| ())
}

/// Adds a shortcut to `target` (an item from another drive or library) inside `parent`.
pub async fn add_shortcut(
    client: &Client,
    target: &DriveItem,
    parent: &ItemRef,
    name: Option<&str>,
) -> Result<DriveItem> {
    let body = shortcut_body(name.unwrap_or(&target.name), &target.drive_id, &target.id);
    let v = graph(
        client,
        "add_shortcut",
        Method::POST,
        &format!("{}/children", parent.url()),
        Some(&body),
    )
    .await?;
    one(&v, "add_shortcut")
}

/// Version history of a file.
pub async fn versions(client: &Client, item: &ItemRef) -> Result<Vec<Version>> {
    let v = graph(
        client,
        "versions",
        Method::GET,
        &format!("{}/versions", item.url()),
        None,
    )
    .await?;
    Ok(parse_versions(&v))
}

/// Makes an earlier version the current one.
pub async fn restore_version(client: &Client, item: &ItemRef, version_id: &str) -> Result<()> {
    graph(
        client,
        "restore_version",
        Method::POST,
        &format!(
            "{}/versions/{}/restoreVersion",
            item.url(),
            encode_path_segment(version_id)
        ),
        None,
    )
    .await
    .map(|_| ())
}

/// A short-lived URL that renders the file for embedding (Office files, PDFs, images).
pub async fn preview_url(client: &Client, item: &ItemRef) -> Result<String> {
    let v = graph(
        client,
        "preview",
        Method::POST,
        &format!("{}/preview", item.url()),
        Some(&json!({})),
    )
    .await?;
    parse_preview(&v).ok_or_else(|| Error::Protocol("preview returned no URL".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_and_rename_bodies() {
        assert_eq!(
            folder_body("Reports"),
            json!({"name": "Reports", "folder": {}, "@microsoft.graph.conflictBehavior": "rename"})
        );
        assert_eq!(rename_body("b.txt"), json!({"name": "b.txt"}));
    }

    #[test]
    fn move_and_copy_bodies() {
        assert_eq!(
            move_body("01P", None),
            json!({"parentReference": {"id": "01P"}})
        );
        assert_eq!(
            move_body("01P", Some("new.txt")),
            json!({"parentReference": {"id": "01P"}, "name": "new.txt"})
        );
        assert_eq!(
            copy_body("b!d", "01P", Some("copy.txt")),
            json!({"parentReference": {"driveId": "b!d", "id": "01P"}, "name": "copy.txt"})
        );
        assert!(copy_body("d", "p", None).get("name").is_none());
    }

    #[test]
    fn shortcut_request_body() {
        assert_eq!(
            shortcut_body("Plan", "b!theirs", "01T"),
            json!({
                "name": "Plan",
                "remoteItem": {"id": "01T", "parentReference": {"driveId": "b!theirs"}},
                "@microsoft.graph.conflictBehavior": "rename"
            })
        );
    }

    #[test]
    fn upload_urls_address_by_path() {
        let p = ItemRef::new("b!d", "01P");
        assert_eq!(
            upload_url(&p, "تقرير 1.pdf"),
            "https://graph.microsoft.com/v1.0/drives/b%21d/items/01P:/%D8%AA%D9%82%D8%B1%D9%8A%D8%B1%201.pdf:/content?@microsoft.graph.conflictBehavior=rename"
        );
        assert_eq!(
            upload_session_url(&ItemRef::my_root(), "a#b.bin"),
            "https://graph.microsoft.com/v1.0/me/drive/root:/a%23b.bin:/createUploadSession"
        );
    }

    #[test]
    fn copy_monitor_states() {
        assert_eq!(
            parse_monitor(
                202,
                None,
                r#"{"status":"inProgress","percentageComplete":10}"#
            ),
            Monitor::Pending
        );
        assert_eq!(
            parse_monitor(200, None, r#"{"status":"completed","resourceId":"01NEW"}"#),
            Monitor::Done("01NEW".into())
        );
        assert_eq!(
            parse_monitor(
                303,
                Some("https://c.sharepoint.com/_api/v2.0/drives/b!d/items/01NEW?x=1"),
                ""
            ),
            Monitor::Done("01NEW".into())
        );
        assert_eq!(
            parse_monitor(
                200,
                None,
                r#"{"status":"failed","error":{"message":"no space"}}"#
            ),
            Monitor::Failed("no space".into())
        );
        assert!(matches!(parse_monitor(404, None, ""), Monitor::Failed(_)));
        assert_eq!(parse_monitor(200, None, "not json"), Monitor::Pending);
    }

    #[test]
    fn versions_and_preview() {
        let v = parse_versions(&json!({"value": [
            {"id": "2.0", "lastModifiedDateTime": "2026-11-10T08:30:00Z",
             "lastModifiedBy": {"user": {"displayName": "Ada"}}, "size": 42},
            {"id": "1.0"}, {"size": 1}
        ]}));
        assert_eq!(v.len(), 2);
        assert_eq!(
            (v[0].id.as_str(), v[0].modified_by.as_str(), v[0].size),
            ("2.0", "Ada", 42)
        );
        assert_eq!(v[1].modified_ms, 0);
        assert_eq!(
            parse_preview(&json!({"getUrl": "https://g", "postUrl": "https://p"})),
            Some("https://g".into())
        );
        assert_eq!(
            parse_preview(&json!({"postUrl": "https://p"})),
            Some("https://p".into())
        );
        assert_eq!(parse_preview(&json!({"getUrl": ""})), None);
    }
}
