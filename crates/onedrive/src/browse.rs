//! Browsing: My files, folders, favorites, libraries, Home (recent), Shared and search.

use reqwest::Method;
use serde_json::{Value, json};
use teams_proto::files::encode_path_segment;
use teams_proto::{Client, Error, Result};

use crate::api::{GRAPH, ItemRef, graph, is_status};
use crate::model::{
    Drive, DriveItem, Page, Site, Thumbnails, parse_drive, parse_item, parse_page, parse_search,
    parse_site, parse_thumbnails,
};

/// Listing query: thumbnails come along so a grid needs no second call.
const LIST_QUERY: &str = "$expand=thumbnails&$top=200";

/// Request body of `POST /search/query` over files and list items. `newest_first` sorts by
/// last modified, newest first.
pub fn search_body(query: &str, from: u32, size: u32, newest_first: bool) -> Value {
    let mut req = json!({
        "entityTypes": ["driveItem", "listItem"],
        "query": {"queryString": query},
        "from": from,
        "size": size,
    });
    if newest_first {
        req["sortProperties"] = json!([{"name": "lastModifiedDateTime", "isDescending": true}]);
    }
    json!({"requests": [req]})
}

/// Graph only follows next links on its own host; anything else is refused by the audience
/// check, so a link in a response can never carry a token elsewhere.
fn page_url(base: &str, next: Option<&str>) -> String {
    match next {
        Some(n) => n.to_owned(),
        None => base.to_owned(),
    }
}

async fn list(client: &Client, context: &'static str, url: &str) -> Result<Page<DriveItem>> {
    Ok(parse_page(
        &graph(client, context, Method::GET, url, None).await?,
    ))
}

/// First page of My files.
pub async fn root_children(client: &Client) -> Result<Page<DriveItem>> {
    children_of(client, &ItemRef::my_root(), None).await
}

/// A page of the children of a folder. Pass the previous page's `next_link` as `next`.
pub async fn children(
    client: &Client,
    drive_id: &str,
    item_id: &str,
    next: Option<&str>,
) -> Result<Page<DriveItem>> {
    children_of(client, &ItemRef::new(drive_id, item_id), next).await
}

pub(crate) async fn children_of(
    client: &Client,
    folder: &ItemRef,
    next: Option<&str>,
) -> Result<Page<DriveItem>> {
    let base = format!("{}/children?{LIST_QUERY}", folder.url());
    list(client, "children", &page_url(&base, next)).await
}

/// Graph URL of the item at a path inside My files (`Documents/Reports`; empty is the root).
pub fn path_url(path: &str) -> String {
    let segments: Vec<String> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(encode_path_segment)
        .collect();
    if segments.is_empty() {
        format!("{GRAPH}/me/drive/root")
    } else {
        format!("{GRAPH}/me/drive/root:/{}", segments.join("/"))
    }
}

/// The item at a path inside My files.
pub async fn item_at(client: &Client, path: &str) -> Result<DriveItem> {
    let v = graph(client, "item_at", Method::GET, &path_url(path), None).await?;
    parse_item(&v).ok_or_else(|| Error::Protocol(format!("no item at {path}")))
}

/// Items the user follows (Favorites).
pub async fn following(client: &Client) -> Result<Page<DriveItem>> {
    list(client, "following", &format!("{GRAPH}/me/drive/following")).await
}

/// Sites the user follows (Libraries). Some tenants answer 403 for `/me/followedSites`; the
/// sites of the user's joined teams stand in then.
pub async fn followed_sites(client: &Client) -> Result<Vec<Site>> {
    match graph(
        client,
        "followed_sites",
        Method::GET,
        &format!("{GRAPH}/me/followedSites"),
        None,
    )
    .await
    {
        Ok(v) => Ok(v["value"]
            .as_array()
            .map(|a| a.iter().filter_map(parse_site).collect())
            .unwrap_or_default()),
        Err(e) if is_status(&e, 403) => team_sites(client).await,
        Err(e) => Err(e),
    }
}

async fn team_sites(client: &Client) -> Result<Vec<Site>> {
    let teams = graph(
        client,
        "joined_teams",
        Method::GET,
        &format!("{GRAPH}/me/joinedTeams?$select=id"),
        None,
    )
    .await?;
    let mut out = Vec::new();
    for id in teams["value"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["id"].as_str())
    {
        let url = format!("{GRAPH}/groups/{id}/sites/root?$select=id,displayName,webUrl");
        // A team without a site (or one we cannot read) is skipped, not fatal.
        if let Ok(v) = graph(client, "team_site", Method::GET, &url, None).await {
            out.extend(parse_site(&v));
        }
    }
    Ok(out)
}

/// The document libraries of a site.
pub async fn site_drives(client: &Client, site_id: &str) -> Result<Vec<Drive>> {
    let url = format!("{GRAPH}/sites/{site_id}/drives?$select=id,name,webUrl,driveType");
    let v = graph(client, "site_drives", Method::GET, &url, None).await?;
    Ok(v["value"]
        .as_array()
        .map(|a| a.iter().filter_map(parse_drive).collect())
        .unwrap_or_default())
}

/// The Documents library of a Microsoft 365 group (a team).
pub async fn group_drive(client: &Client, group_id: &str) -> Result<Drive> {
    let url = format!("{GRAPH}/groups/{group_id}/drive?$select=id,name,webUrl,driveType");
    let v = graph(client, "group_drive", Method::GET, &url, None).await?;
    parse_drive(&v).ok_or_else(|| Error::Protocol("group drive has no id".into()))
}

/// The signed-in user's own drive.
pub async fn my_drive(client: &Client) -> Result<Drive> {
    let url = format!("{GRAPH}/me/drive?$select=id,name,webUrl,driveType");
    let v = graph(client, "my_drive", Method::GET, &url, None).await?;
    parse_drive(&v).ok_or_else(|| Error::Protocol("drive has no id".into()))
}

async fn search_raw(
    client: &Client,
    query: &str,
    from: u32,
    size: u32,
    newest_first: bool,
) -> Result<crate::model::SearchPage> {
    let body = search_body(query, from, size, newest_first);
    let v = graph(
        client,
        "search",
        Method::POST,
        &format!("{GRAPH}/search/query"),
        Some(&body),
    )
    .await?;
    Ok(parse_search(&v))
}

/// Searches OneDrive and SharePoint files for `query` (first 50 hits).
pub async fn search(client: &Client, query: &str) -> Result<Vec<DriveItem>> {
    Ok(search_from(client, query, 0, 50).await?.items)
}

/// One page of search hits starting at `from`; `more` tells whether a next page exists.
pub async fn search_from(
    client: &Client,
    query: &str,
    from: u32,
    size: u32,
) -> Result<crate::model::SearchPage> {
    search_raw(client, query, from, size, false).await
}

/// Home: files the user opened recently. Graph `/me/drive/recent` answers in tenants where it
/// is still served; when it is empty or fails, Search sorted by last modified stands in.
pub async fn recent(client: &Client) -> Result<Vec<DriveItem>> {
    match list(client, "recent", &format!("{GRAPH}/me/drive/recent")).await {
        Ok(p) if !p.items.is_empty() => Ok(p.items),
        _ => Ok(search_raw(client, "*", 0, 50, true).await?.items),
    }
}

/// Shared with the user: Graph `/me/drive/sharedWithMe`, else Search over everything that is
/// not in the user's own OneDrive.
pub async fn shared_with_me(client: &Client) -> Result<Vec<DriveItem>> {
    match list(
        client,
        "shared_with_me",
        &format!("{GRAPH}/me/drive/sharedWithMe"),
    )
    .await
    {
        Ok(p) if !p.items.is_empty() => Ok(p.items),
        _ => {
            let own = crate::bin::my_site(client).await?;
            let q = format!("* AND NOT path:\"{}\"", own.site_url);
            Ok(search_raw(client, &q, 0, 50, true).await?.items)
        }
    }
}

/// Thumbnail URLs of one item (empty when it has none).
pub async fn thumbnails(client: &Client, item: &ItemRef) -> Result<Thumbnails> {
    let v = graph(
        client,
        "thumbnails",
        Method::GET,
        &format!("{}/thumbnails", item.url()),
        None,
    )
    .await?;
    Ok(parse_thumbnails(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_request_body() {
        let b = search_body("budget", 50, 25, false);
        let r = &b["requests"][0];
        assert_eq!(r["entityTypes"], json!(["driveItem", "listItem"]));
        assert_eq!(r["query"]["queryString"], "budget");
        assert_eq!(
            (r["from"].as_u64(), r["size"].as_u64()),
            (Some(50), Some(25))
        );
        assert!(r.get("sortProperties").is_none());
        let b = search_body("*", 0, 50, true);
        assert_eq!(
            b["requests"][0]["sortProperties"][0],
            json!({"name": "lastModifiedDateTime", "isDescending": true})
        );
    }

    #[test]
    fn path_urls() {
        assert_eq!(
            path_url(""),
            "https://graph.microsoft.com/v1.0/me/drive/root"
        );
        assert_eq!(
            path_url("/"),
            "https://graph.microsoft.com/v1.0/me/drive/root"
        );
        assert_eq!(
            path_url("Documents/تقرير 1/x#y"),
            "https://graph.microsoft.com/v1.0/me/drive/root:/Documents/%D8%AA%D9%82%D8%B1%D9%8A%D8%B1%201/x%23y"
        );
    }

    #[test]
    fn paging_prefers_the_next_link() {
        assert_eq!(page_url("a", None), "a");
        assert_eq!(page_url("a", Some("b")), "b");
    }
}
