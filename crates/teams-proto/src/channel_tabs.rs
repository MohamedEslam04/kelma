//! A channel's tabs beyond Posts and Files (websites, Planner, OneNote, apps), through Graph
//! `teams/{group}/channels/{channel}/tabs`.
//!
//! Live (2026-11-19, one work tenant, read only): the list answers with `displayName`, `webUrl`
//! (the tab in Teams on the web), `configuration` (`contentUrl`, `websiteUrl`, `entityId`) and,
//! expanded, `teamsApp` (`id`, `displayName`). The middle tier's `beta/teams/…/tabs` answers 405.

use crate::client::Client;
use crate::error::Result;
use crate::graph::graph_url;
use reqwest::Method;
use serde_json::{Value, json};

/// The Teams app behind a website tab.
pub const WEBSITE_APP: &str = "com.microsoft.teamspace.tab.web";
/// Apps whose tab kelma already shows as its own header tab.
const BUILT_IN: &[&str] = &[
    "com.microsoft.teamspace.tab.files.sharepoint",
    "com.microsoft.teamspace.tab.wiki",
];

/// One tab of a channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelTab {
    pub id: String,
    pub name: String,
    pub app_id: String,
    pub app_name: String,
    /// What to open: the tab's own website, else the tab in Teams on the web.
    pub open_url: String,
    /// The page the tab shows inside Teams.
    pub content_url: String,
}

fn s(v: &Value, p: &str) -> String {
    v.pointer(p)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Parses the tab list, leaving out the tabs kelma shows itself (Files, the old wiki).
pub fn parse_tabs(v: &Value) -> Vec<ChannelTab> {
    v.get("value")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|t| {
                    let website = s(t, "/configuration/websiteUrl");
                    let web = s(t, "/webUrl");
                    ChannelTab {
                        id: s(t, "/id"),
                        name: s(t, "/displayName"),
                        app_id: s(t, "/teamsApp/id"),
                        app_name: s(t, "/teamsApp/displayName"),
                        open_url: if website.starts_with("https://") {
                            website
                        } else {
                            web
                        },
                        content_url: s(t, "/configuration/contentUrl"),
                    }
                })
                .filter(|t| {
                    // The Files tab's app also comes with a tenant-specific id (live), so its
                    // name decides too.
                    !t.name.is_empty()
                        && !BUILT_IN.contains(&t.app_id.as_str())
                        && t.app_name != "Files"
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The body that adds a website tab.
pub fn website_tab_body(name: &str, url: &str) -> Value {
    json!({
        "displayName": name,
        "teamsApp@odata.bind":
            format!("https://graph.microsoft.com/v1.0/appCatalogs/teamsApps/{WEBSITE_APP}"),
        "configuration": {
            "entityId": null,
            "contentUrl": url,
            "websiteUrl": url,
            "removeUrl": null,
        },
    })
}

/// The body that renames a tab (`PATCH …/tabs/{id}`).
pub fn rename_tab_body(name: &str) -> Value {
    json!({ "displayName": name })
}

/// The body that adds a tab of an installed app, without a configuration. UNVERIFIED: Graph
/// documents that apps whose tab needs setup are added unconfigured and set up in Teams, but
/// kelma has not sent this to a tenant.
pub fn app_tab_body(name: &str, app_id: &str) -> Value {
    json!({
        "displayName": name,
        "teamsApp@odata.bind":
            format!("https://graph.microsoft.com/v1.0/appCatalogs/teamsApps/{app_id}"),
    })
}

impl Client {
    /// The channel's tabs (without Files).
    pub async fn channel_tabs(&self, group_id: &str, channel_id: &str) -> Result<Vec<ChannelTab>> {
        let url = graph_url(
            &["teams", group_id, "channels", channel_id, "tabs"],
            "$expand=teamsApp",
        );
        let v = self.json("channel tabs", Method::GET, &url, None).await?;
        Ok(parse_tabs(&v))
    }

    /// Adds a website tab to the channel (Teams posts "… added a tab" in the channel).
    pub async fn add_website_tab(
        &self,
        group_id: &str,
        channel_id: &str,
        name: &str,
        url: &str,
    ) -> Result<()> {
        let u = graph_url(&["teams", group_id, "channels", channel_id, "tabs"], "");
        let body = website_tab_body(name, url);
        self.json("add tab", Method::POST, &u, Some(&body)).await?;
        Ok(())
    }

    /// Adds a tab of an installed app (UNVERIFIED, see [`app_tab_body`]).
    pub async fn add_app_tab(
        &self,
        group_id: &str,
        channel_id: &str,
        name: &str,
        app_id: &str,
    ) -> Result<()> {
        let u = graph_url(&["teams", group_id, "channels", channel_id, "tabs"], "");
        let body = app_tab_body(name, app_id);
        self.json("add app tab", Method::POST, &u, Some(&body))
            .await?;
        Ok(())
    }

    /// Adds a website tab to a chat: Graph `POST /chats/{chat}/tabs` with the channel body.
    /// UNVERIFIED: never sent from kelma yet (docs/protocol.md).
    pub async fn add_chat_website_tab(&self, chat_id: &str, name: &str, url: &str) -> Result<()> {
        let body = website_tab_body(name, url);
        self.json(
            "add chat tab",
            Method::POST,
            &chat_tabs_url(chat_id),
            Some(&body),
        )
        .await?;
        Ok(())
    }

    /// Adds an installed app's tab to a chat (UNVERIFIED, as [`Client::add_chat_website_tab`]).
    pub async fn add_chat_app_tab(&self, chat_id: &str, name: &str, app_id: &str) -> Result<()> {
        let body = app_tab_body(name, app_id);
        self.json(
            "add chat app tab",
            Method::POST,
            &chat_tabs_url(chat_id),
            Some(&body),
        )
        .await?;
        Ok(())
    }

    /// Renames a tab (Graph "Update tab").
    pub async fn rename_channel_tab(
        &self,
        group_id: &str,
        channel_id: &str,
        tab_id: &str,
        name: &str,
    ) -> Result<()> {
        let u = graph_url(
            &["teams", group_id, "channels", channel_id, "tabs", tab_id],
            "",
        );
        let body = rename_tab_body(name);
        self.json("rename tab", Method::PATCH, &u, Some(&body))
            .await?;
        Ok(())
    }

    /// Removes a tab from the channel (Graph "Delete tab").
    pub async fn remove_channel_tab(
        &self,
        group_id: &str,
        channel_id: &str,
        tab_id: &str,
    ) -> Result<()> {
        let u = graph_url(
            &["teams", group_id, "channels", channel_id, "tabs", tab_id],
            "",
        );
        self.json("remove tab", Method::DELETE, &u, None).await?;
        Ok(())
    }
}

/// A chat's tabs on Graph: `/chats/{chat}/tabs`.
pub fn chat_tabs_url(chat_id: &str) -> String {
    graph_url(&["chats", chat_id, "tabs"], "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_tab_goes_to_the_chats_tabs_with_the_channel_body() {
        assert_eq!(
            chat_tabs_url("19:abc@thread.v2"),
            "https://graph.microsoft.com/v1.0/chats/19:abc@thread.v2/tabs"
        );
    }

    #[test]
    fn tabs_parse_without_the_files_tab_and_open_their_website() {
        let v = json!({"value": [
            {"id": "t1", "displayName": "Files", "webUrl": "https://teams.microsoft.com/l/x",
             "configuration": {"contentUrl": "https://sp/x", "websiteUrl": null},
             "teamsApp": {"id": "com.microsoft.teamspace.tab.files.sharepoint", "displayName": "Files"}},
            {"id": "t2", "displayName": "Board", "webUrl": "https://teams.microsoft.com/l/entity/t2",
             "configuration": {"contentUrl": "https://tasks.office.com/b", "websiteUrl": "https://tasks.office.com/b"},
             "teamsApp": {"id": "com.microsoft.teamspace.tab.planner", "displayName": "Planner"}},
            {"id": "t3", "displayName": "Notes", "webUrl": "https://teams.microsoft.com/l/entity/t3",
             "configuration": {"contentUrl": "https://x/notes", "websiteUrl": ""},
             "teamsApp": {"id": "0d820ecd-def2-4297-adad-78056cde7c78", "displayName": "OneNote"}}
        ]});
        let tabs = parse_tabs(&v);
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].name, "Board");
        assert_eq!(tabs[0].open_url, "https://tasks.office.com/b");
        assert_eq!(tabs[0].app_name, "Planner");
        // No website: the tab in Teams on the web.
        assert_eq!(tabs[1].open_url, "https://teams.microsoft.com/l/entity/t3");
    }

    #[test]
    fn the_files_tab_is_left_out_whatever_its_app_id() {
        let v = json!({"value": [
            {"id": "t1", "displayName": "Files", "webUrl": "https://teams.microsoft.com/l/x",
             "teamsApp": {"id": "2a527703-1f6f-4559-a332-d8a7d288cd88", "displayName": "Files"}}
        ]});
        assert!(parse_tabs(&v).is_empty());
    }

    #[test]
    fn a_rename_sends_only_the_name_and_an_app_tab_binds_its_app() {
        assert_eq!(rename_tab_body("Plan"), json!({"displayName": "Plan"}));
        let b = app_tab_body("Board", "abc-123");
        assert_eq!(b["displayName"], "Board");
        assert_eq!(
            b["teamsApp@odata.bind"],
            "https://graph.microsoft.com/v1.0/appCatalogs/teamsApps/abc-123"
        );
        assert!(b.get("configuration").is_none());
    }

    #[test]
    fn a_website_tab_binds_the_website_app() {
        let b = website_tab_body("Docs", "https://example.com/docs");
        assert_eq!(b["displayName"], "Docs");
        assert_eq!(
            b["teamsApp@odata.bind"],
            "https://graph.microsoft.com/v1.0/appCatalogs/teamsApps/com.microsoft.teamspace.tab.web"
        );
        assert_eq!(b["configuration"]["websiteUrl"], "https://example.com/docs");
    }
}
