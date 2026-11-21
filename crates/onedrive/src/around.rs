//! Around files: meeting recordings and the "Sync" explanation.

use futures_util::future::join;
use futures_util::stream::{self, StreamExt};
use reqwest::Method;
use serde_json::Value;
use teams_proto::files::encode_path_segment;
use teams_proto::{Client, Result};

use crate::api::{GRAPH, ItemRef, graph, is_status};
use crate::model::{DriveItem, SortKey, parse_item, parse_page, sort_items};

/// Folder of My files where Teams saves meeting recordings.
pub const RECORDINGS_FOLDER: &str = "Recordings";

/// Meeting recordings in the `Recordings` folder of My files, newest first. A user who never
/// recorded a meeting has no such folder: that is an empty list, not an error.
pub async fn recordings(client: &Client) -> Result<Vec<DriveItem>> {
    let url =
        format!("{GRAPH}/me/drive/root:/{RECORDINGS_FOLDER}:/children?$expand=thumbnails&$top=200");
    match graph(client, "recordings", Method::GET, &url, None).await {
        Ok(v) => {
            let mut items = parse_page(&v).items;
            sort_items(&mut items, SortKey::Modified, false);
            Ok(items)
        }
        Err(e) if is_status(&e, 404) => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// A meeting recording and where it is kept.
#[derive(Debug, Clone, PartialEq)]
pub struct Recording {
    pub item: DriveItem,
    /// `My files`, or `<team> › <channel>` for a channel meeting.
    pub place: String,
}

/// Place label of the user's own recordings.
pub const MY_FILES: &str = "My files";

/// `(id, displayName)` pairs of a Graph collection (joined teams, channels).
pub fn parse_named(v: &Value) -> Vec<(String, String)> {
    v["value"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            Some((
                e["id"].as_str()?.to_owned(),
                e["displayName"].as_str().unwrap_or_default().to_owned(),
            ))
        })
        .collect()
}

/// URL of a channel's files folder (the channel's folder in the team library).
pub fn files_folder_url(team_id: &str, channel_id: &str) -> String {
    format!(
        "{GRAPH}/teams/{}/channels/{}/filesFolder",
        encode_path_segment(team_id),
        encode_path_segment(channel_id)
    )
}

/// The channel folder from a `filesFolder` answer: its drive comes from `parentReference`.
pub fn parse_files_folder(v: &Value) -> Option<ItemRef> {
    let id = v["id"].as_str()?;
    let drive = v["parentReference"]["driveId"].as_str()?;
    Some(ItemRef::new(drive, id))
}

/// URL that lists the `Recordings` folder inside `folder`.
pub fn recordings_url(folder: &ItemRef) -> String {
    format!(
        "{}/children?$expand=thumbnails&$top=200",
        folder.child_url(RECORDINGS_FOLDER)
    )
}

/// Files of a `Recordings` listing, labelled with `place`.
pub fn parse_recordings(v: &Value, place: &str) -> Vec<Recording> {
    v["value"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(parse_item)
        .filter(|i| !i.is_folder)
        .map(|item| Recording {
            item,
            place: place.to_owned(),
        })
        .collect()
}

/// Sorts recordings newest first (ties by name).
pub fn sort_recordings(v: &mut [Recording]) {
    v.sort_by(|a, b| {
        b.item
            .modified_ms
            .cmp(&a.item.modified_ms)
            .then_with(|| a.item.name.cmp(&b.item.name))
    });
}

async fn list_recordings(client: &Client, folder: &ItemRef, place: &str) -> Result<Vec<Recording>> {
    match graph(
        client,
        "recordings",
        Method::GET,
        &recordings_url(folder),
        None,
    )
    .await
    {
        Ok(v) => Ok(parse_recordings(&v, place)),
        Err(e) if is_status(&e, 404) => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// How many Graph calls the channel crawl keeps in flight at once.
const CRAWL_CONCURRENCY: usize = 8;

/// Recordings of channel meetings: the `Recordings` folder in each channel's files folder of
/// every joined team. A channel without that folder, or one that cannot be read, is skipped.
/// Costs two Graph calls per channel; the channels are read side by side (one after the other,
/// a user in many teams waited minutes).
pub async fn channel_recordings(client: &Client) -> Result<Vec<Recording>> {
    let teams = graph(
        client,
        "joined_teams",
        Method::GET,
        &format!("{GRAPH}/me/joinedTeams?$select=id,displayName"),
        None,
    )
    .await?;
    // (team, channel, place) of every channel of every team.
    let channels: Vec<(String, String, String)> = stream::iter(parse_named(&teams))
        .map(|(team_id, team_name)| async move {
            let url = format!(
                "{GRAPH}/teams/{}/channels?$select=id,displayName",
                encode_path_segment(&team_id)
            );
            let Ok(channels) = graph(client, "channels", Method::GET, &url, None).await else {
                return Vec::new();
            };
            parse_named(&channels)
                .into_iter()
                .map(|(channel_id, channel_name)| {
                    let place = format!("{team_name} › {channel_name}");
                    (team_id.clone(), channel_id, place)
                })
                .collect()
        })
        .buffer_unordered(CRAWL_CONCURRENCY)
        .concat()
        .await;
    // A channel's two calls depend on each other; different channels do not.
    let mut out: Vec<Recording> = stream::iter(channels)
        .map(|(team_id, channel_id, place)| async move {
            let folder = graph(
                client,
                "files_folder",
                Method::GET,
                &files_folder_url(&team_id, &channel_id),
                None,
            )
            .await
            .ok()
            .and_then(|v| parse_files_folder(&v));
            let Some(folder) = folder else {
                return Vec::new();
            };
            list_recordings(client, &folder, &place)
                .await
                .unwrap_or_default()
        })
        .buffer_unordered(CRAWL_CONCURRENCY)
        .concat()
        .await;
    sort_recordings(&mut out);
    Ok(out)
}

/// The Meetings view: recordings in My files and in the channel libraries, newest first. The
/// two are read at the same time.
pub async fn meeting_recordings(client: &Client) -> Result<Vec<Recording>> {
    let (mine, channels) = join(
        list_recordings(client, &ItemRef::my_root(), MY_FILES),
        channel_recordings(client),
    )
    .await;
    let mut out = mine?;
    out.extend(channels?);
    sort_recordings(&mut out);
    Ok(out)
}

/// What the "Sync" button explains: IronTeams does not sync files, a Linux client does.
pub fn sync_help() -> String {
    "IronTeams does not sync OneDrive to a folder on this computer. On Linux, use one of these \
     instead.\n\
     \n\
     1. The `onedrive` client (abraunegg/onedrive), which syncs a local folder two ways:\n\
     \n\
     \x20  sudo dnf install onedrive        # or: sudo apt install onedrive\n\
     \x20  onedrive                         # opens a sign-in link, then syncs ~/OneDrive\n\
     \x20  systemctl --user enable --now onedrive\n\
     \n\
     2. rclone, which mounts or copies on demand:\n\
     \n\
     \x20  rclone config                    # new remote, type: onedrive, then sign in\n\
     \x20  # ~/.config/rclone/rclone.conf\n\
     \x20  [onedrive]\n\
     \x20  type = onedrive\n\
     \x20  drive_type = business\n\
     \x20  # mount it like a disk:\n\
     \x20  rclone mount onedrive: ~/OneDrive --vfs-cache-mode full\n\
     \x20  # or copy a folder once:\n\
     \x20  rclone copy onedrive:Documents ~/Documents-backup\n"
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_help_points_at_linux_clients_with_a_config() {
        let t = sync_help();
        assert!(t.contains("abraunegg/onedrive"));
        assert!(t.contains("rclone mount"));
        assert!(t.contains("[onedrive]") && t.contains("type = onedrive"));
        assert!(t.contains("does not sync"));
    }

    use serde_json::json;

    #[test]
    fn channel_recordings_requests() {
        assert_eq!(
            files_folder_url("t-1", "19:abc@thread.tacv2"),
            "https://graph.microsoft.com/v1.0/teams/t-1/channels/19%3Aabc%40thread.tacv2/filesFolder"
        );
        assert_eq!(
            recordings_url(&ItemRef::new("b!d", "01F")),
            "https://graph.microsoft.com/v1.0/drives/b%21d/items/01F:/Recordings:/children?$expand=thumbnails&$top=200"
        );
        assert_eq!(
            recordings_url(&ItemRef::my_root()),
            "https://graph.microsoft.com/v1.0/me/drive/root:/Recordings:/children?$expand=thumbnails&$top=200"
        );
    }

    #[test]
    fn parses_teams_channels_and_files_folder() {
        let named = parse_named(&json!({"value": [
            {"id": "t1", "displayName": "Design"}, {"displayName": "no id"}, {"id": "t2"}
        ]}));
        assert_eq!(
            named,
            [("t1".into(), "Design".into()), ("t2".into(), String::new())]
        );
        let f = parse_files_folder(&json!({
            "id": "01CH", "name": "General", "folder": {"childCount": 3},
            "parentReference": {"driveId": "b!lib", "driveType": "documentLibrary"}
        }));
        assert_eq!(f, Some(ItemRef::new("b!lib", "01CH")));
        assert_eq!(parse_files_folder(&json!({"id": "01CH"})), None);
    }

    #[test]
    fn recordings_are_labelled_files_newest_first() {
        let listing = |names: &[(&str, &str)]| {
            json!({"value": names.iter().map(|(n, t)| json!({
                "id": format!("01{n}"), "name": n, "lastModifiedDateTime": t,
                "parentReference": {"driveId": "b!d"}, "file": {"mimeType": "video/mp4"}
            })).chain([json!({"id": "01DIR", "name": "Sub", "folder": {"childCount": 0}})]).collect::<Vec<_>>()})
        };
        let mut all = parse_recordings(
            &listing(&[
                ("a.mp4", "2026-11-01T00:00:00Z"),
                ("c.mp4", "2026-11-09T00:00:00Z"),
            ]),
            MY_FILES,
        );
        all.extend(parse_recordings(
            &listing(&[("b.mp4", "2026-11-05T00:00:00Z")]),
            "Design › General",
        ));
        sort_recordings(&mut all);
        let got: Vec<(&str, &str)> = all
            .iter()
            .map(|r| (r.item.name.as_str(), r.place.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("c.mp4", "My files"),
                ("b.mp4", "Design › General"),
                ("a.mp4", "My files")
            ]
        );
    }

    #[test]
    fn recordings_folder_name() {
        assert_eq!(RECORDINGS_FOLDER, "Recordings");
    }
}
