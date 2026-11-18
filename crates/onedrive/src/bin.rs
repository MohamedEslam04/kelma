//! Recycle bin through SharePoint REST on the user's own `-my.sharepoint.com` host. Graph has
//! no recycle bin for OneDrive for Business, so the SharePoint token is used here.

use reqwest::Method;
use serde_json::Value;
use teams_proto::broker::Audience;
use teams_proto::time::parse_iso_ms;
use teams_proto::{Client, Error, Result};

use crate::api::GRAPH;

/// The user's OneDrive site.
#[derive(Debug, Clone, PartialEq)]
pub struct MySite {
    /// For example `contoso-my.sharepoint.com`.
    pub host: String,
    /// For example `https://contoso-my.sharepoint.com/personal/ada_contoso_com` (no trailing slash).
    pub site_url: String,
}

/// An item in the recycle bin.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BinItem {
    /// Recycle bin id (a GUID), the argument of [`restore`] and [`delete_forever`].
    pub id: String,
    pub name: String,
    /// Original folder, relative to the site (`personal/ada_contoso_com/Documents/Reports`).
    pub dir: String,
    pub size: u64,
    pub deleted_ms: i64,
    pub deleted_by: String,
    pub is_folder: bool,
}

/// Host and site URL from the `webUrl`/`sharepointIds` of the drive root.
pub fn parse_my_site(v: &Value) -> Option<MySite> {
    let site_url = v
        .pointer("/sharepointIds/siteUrl")
        .and_then(Value::as_str)?
        .trim_end_matches('/')
        .to_owned();
    let host = url::Url::parse(&site_url).ok()?.host_str()?.to_owned();
    Audience::sharepoint(&host)?;
    Some(MySite { host, site_url })
}

/// Parses `GET _api/web/RecycleBin` (`odata=nometadata`). Newest deletion first.
pub fn parse_bin(v: &Value) -> Vec<BinItem> {
    let mut out: Vec<BinItem> = v["value"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| {
            let size = match &i["Size"] {
                Value::String(s) => s.parse().unwrap_or(0),
                n => n.as_u64().unwrap_or(0),
            };
            // RecycleBinItemType: 5 Folder, 6 FolderWithLists.
            let kind = i["ItemType"].as_u64().unwrap_or(1);
            Some(BinItem {
                id: i["Id"].as_str()?.to_owned(),
                name: i["LeafName"].as_str()?.to_owned(),
                dir: i["DirName"].as_str().unwrap_or_default().to_owned(),
                size,
                deleted_ms: i["DeletedDate"]
                    .as_str()
                    .and_then(parse_iso_ms)
                    .unwrap_or(0),
                deleted_by: i["DeletedByName"].as_str().unwrap_or_default().to_owned(),
                is_folder: kind == 5 || kind == 6,
            })
        })
        .collect();
    out.sort_by_key(|i| std::cmp::Reverse(i.deleted_ms));
    out
}

/// URL of a recycle bin entry action (`restore`, `deleteObject`).
pub fn bin_action_url(site_url: &str, id: &str, action: &str) -> String {
    format!("{site_url}/_api/web/RecycleBin('{id}')/{action}")
}

/// The user's OneDrive site, from the drive root.
pub async fn my_site(client: &Client) -> Result<MySite> {
    let v = crate::api::graph(
        client,
        "my_site",
        Method::GET,
        &format!("{GRAPH}/me/drive/root?$select=sharepointIds"),
        None,
    )
    .await?;
    parse_my_site(&v).ok_or_else(|| Error::Protocol("drive root has no SharePoint site".into()))
}

async fn rest(client: &Client, site: &MySite, method: Method, url: &str) -> Result<Value> {
    let aud = Audience::sharepoint(&site.host)
        .ok_or_else(|| Error::Protocol(format!("{} is not a SharePoint host", site.host)))?;
    let (status, text) = client
        .request_as(
            &aud,
            method,
            url,
            None,
            &[("Accept", "application/json;odata=nometadata")],
        )
        .await?;
    if !(200..300).contains(&status) {
        return Err(Error::status("recycle_bin", status, &text));
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// Items in the first-stage recycle bin, newest deletion first.
pub async fn recycle_bin(client: &Client) -> Result<Vec<BinItem>> {
    let site = my_site(client).await?;
    let url = format!("{}/_api/web/RecycleBin?$top=500", site.site_url);
    Ok(parse_bin(&rest(client, &site, Method::GET, &url).await?))
}

/// Restores an item to its original location.
pub async fn restore(client: &Client, id: &str) -> Result<()> {
    let site = my_site(client).await?;
    let url = bin_action_url(&site.site_url, id, "restore");
    rest(client, &site, Method::POST, &url).await.map(|_| ())
}

/// Deletes an item from the recycle bin for good.
pub async fn delete_forever(client: &Client, id: &str) -> Result<()> {
    let site = my_site(client).await?;
    let url = bin_action_url(&site.site_url, id, "deleteObject");
    rest(client, &site, Method::POST, &url).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn site_from_drive_root() {
        let s = parse_my_site(&json!({"sharepointIds": {
            "siteUrl": "https://contoso-my.sharepoint.com/personal/ada_contoso_com/"}}))
        .unwrap();
        assert_eq!(s.host, "contoso-my.sharepoint.com");
        assert_eq!(
            s.site_url,
            "https://contoso-my.sharepoint.com/personal/ada_contoso_com"
        );
        assert!(parse_my_site(&json!({})).is_none());
        assert!(
            parse_my_site(&json!({"sharepointIds": {"siteUrl": "https://evil.example.com/x"}}))
                .is_none()
        );
    }

    #[test]
    fn bin_items_newest_first() {
        let v = json!({"value": [
            {"Id": "g1", "LeafName": "old.txt", "DirName": "personal/ada/Documents", "Size": "12",
             "DeletedDate": "2026-11-01T10:00:00Z", "DeletedByName": "Ada", "ItemType": 1},
            {"Id": "g2", "LeafName": "Folder", "DirName": "personal/ada/Documents", "Size": 300,
             "DeletedDate": "2026-11-05T10:00:00Z", "DeletedByName": "Ada", "ItemType": 5},
            {"LeafName": "no id"}
        ]});
        let b = parse_bin(&v);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].name, "Folder");
        assert!(b[0].is_folder);
        assert_eq!(b[0].size, 300);
        assert_eq!(b[1].size, 12);
        assert!(b[0].deleted_ms > b[1].deleted_ms);
    }

    #[test]
    fn action_urls() {
        assert_eq!(
            bin_action_url("https://c-my.sharepoint.com/personal/a", "g-1", "restore"),
            "https://c-my.sharepoint.com/personal/a/_api/web/RecycleBin('g-1')/restore"
        );
    }
}
