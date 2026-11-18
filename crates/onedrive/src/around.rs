//! Around files: meeting recordings and the "Sync" explanation.

use reqwest::Method;
use teams_proto::{Client, Result};

use crate::api::{GRAPH, graph, is_status};
use crate::model::{DriveItem, SortKey, parse_page, sort_items};

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

/// What the "Sync" button explains: kelma does not sync files, a Linux client does.
pub fn sync_help() -> String {
    "kelma does not sync OneDrive to a folder on this computer. On Linux, use one of these \
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

    #[test]
    fn recordings_folder_name() {
        assert_eq!(RECORDINGS_FOLDER, "Recordings");
    }
}
