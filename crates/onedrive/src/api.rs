//! Shared plumbing: item addressing and Graph calls with the Graph audience.

use reqwest::Method;
use serde_json::Value;
use teams_proto::broker::Audience;
use teams_proto::client::{RawBody, RawResponse};
use teams_proto::files::encode_path_segment;
use teams_proto::{Client, Error, Result};

use crate::model::DriveItem;

/// Microsoft Graph v1.0 base URL.
pub const GRAPH: &str = "https://graph.microsoft.com/v1.0";

/// Where an item lives. An empty `drive_id` means the signed-in user's own drive; an
/// `item_id` of `root` is the drive's root folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRef {
    pub drive_id: String,
    pub item_id: String,
}

impl ItemRef {
    /// The root of My files.
    pub fn my_root() -> Self {
        ItemRef {
            drive_id: String::new(),
            item_id: "root".into(),
        }
    }

    pub fn new(drive_id: &str, item_id: &str) -> Self {
        ItemRef {
            drive_id: drive_id.into(),
            item_id: item_id.into(),
        }
    }

    /// The item to delete or rename for `item`: its own stub for a shortcut or a shared entry
    /// (so the target is never touched), else the item itself.
    pub fn own(item: &DriveItem) -> Self {
        match &item.stub {
            Some((drive, id)) => ItemRef::new(drive, id),
            None => ItemRef::from(item),
        }
    }

    fn drive_base(&self) -> String {
        if self.drive_id.is_empty() {
            format!("{GRAPH}/me/drive")
        } else {
            format!("{GRAPH}/drives/{}", encode_path_segment(&self.drive_id))
        }
    }

    /// URL of the item itself (`/root` for the drive root).
    pub fn url(&self) -> String {
        if self.item_id == "root" {
            format!("{}/root", self.drive_base())
        } else {
            format!(
                "{}/items/{}",
                self.drive_base(),
                encode_path_segment(&self.item_id)
            )
        }
    }

    /// URL of the child called `name` (`<item>:/<name>:`), for path addressing.
    pub fn child_url(&self, name: &str) -> String {
        format!("{}:/{}:", self.url(), encode_path_segment(name))
    }
}

impl From<&DriveItem> for ItemRef {
    fn from(i: &DriveItem) -> Self {
        ItemRef::new(&i.drive_id, &i.id)
    }
}

/// Calls Graph and parses the JSON answer (`Null` for an empty body).
pub async fn graph(
    client: &Client,
    context: &'static str,
    method: Method,
    url: &str,
    body: Option<&Value>,
) -> Result<Value> {
    client
        .json_as(context, &Audience::Graph, method, url, body)
        .await
}

/// Calls Graph and returns the raw response, failing on a non-2xx status.
pub async fn graph_raw(
    client: &Client,
    context: &'static str,
    method: Method,
    url: &str,
    body: RawBody<'_>,
) -> Result<RawResponse> {
    let r = client
        .request_raw_as(&Audience::Graph, method, url, body, &[])
        .await?;
    if !(200..300).contains(&r.status) {
        return Err(Error::status(
            context,
            r.status,
            &String::from_utf8_lossy(&r.body),
        ));
    }
    Ok(r)
}

/// True when `e` is an HTTP error with `status`.
pub fn is_status(e: &Error, status: u16) -> bool {
    matches!(e, Error::Status { status: s, .. } if *s == status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_urls() {
        assert_eq!(
            ItemRef::my_root().url(),
            "https://graph.microsoft.com/v1.0/me/drive/root"
        );
        assert_eq!(
            ItemRef::new("", "01AB").url(),
            "https://graph.microsoft.com/v1.0/me/drive/items/01AB"
        );
        assert_eq!(
            ItemRef::new("b!x-y", "01AB").url(),
            "https://graph.microsoft.com/v1.0/drives/b%21x-y/items/01AB"
        );
        assert_eq!(
            ItemRef::new("b!x", "root").child_url("a b#.txt"),
            "https://graph.microsoft.com/v1.0/drives/b%21x/root:/a%20b%23.txt:"
        );
    }

    #[test]
    fn item_ref_from_drive_item() {
        let it = DriveItem {
            id: "01X".into(),
            drive_id: "b!d".into(),
            ..Default::default()
        };
        assert_eq!(ItemRef::from(&it), ItemRef::new("b!d", "01X"));
        assert_eq!(ItemRef::own(&it), ItemRef::new("b!d", "01X"));
        let shortcut = DriveItem {
            id: "01TARGET".into(),
            drive_id: "b!theirs".into(),
            stub: Some(("b!mine".into(), "01STUB".into())),
            ..Default::default()
        };
        assert_eq!(
            ItemRef::from(&shortcut),
            ItemRef::new("b!theirs", "01TARGET")
        );
        assert_eq!(ItemRef::own(&shortcut), ItemRef::new("b!mine", "01STUB"));
    }
}
