//! A channel's tabs beyond Posts and Files (Teams' "+" Add a tab): listed in the channel header,
//! opened in the browser, a website or an installed app's tab can be added, and a tab can be
//! renamed or removed.

use super::Engine;
use std::sync::Arc;

/// Splits an argument of two parts joined by U+001F, both trimmed and not empty.
pub fn split_pair(arg: &str) -> Option<(String, String)> {
    let (a, b) = arg.split_once('\u{1f}')?;
    let (a, b) = (a.trim(), b.trim());
    (!a.is_empty() && !b.is_empty()).then(|| (a.to_string(), b.to_string()))
}

/// Splits the "Add a tab" argument: `name` and `url` joined by U+001F.
pub fn split_add(arg: &str) -> Option<(String, String)> {
    let (name, url) = arg.split_once('\u{1f}')?;
    let (name, url) = (name.trim(), url.trim());
    let url = if url.contains("://") {
        url.to_string()
    } else {
        format!("https://{url}")
    };
    (!name.is_empty() && url.starts_with("https://") && url.len() > "https://".len())
        .then(|| (name.to_string(), url))
}

/// `(app id, name)` of the installed apps that have a tab for a team, by name.
pub fn tab_apps(installed: &app_store::InstalledApps) -> Vec<(String, String)> {
    let mut apps: Vec<(String, String)> = installed
        .entitlements
        .iter()
        .filter(|e| e.state.is_installed())
        .filter_map(|e| installed.definitions.get(&e.id))
        .filter(|d| d.works_in.team && d.has_tabs && !d.name.is_empty())
        .map(|d| (d.id.clone(), d.name.clone()))
        .collect();
    apps.sort_by_key(|(_, n)| n.to_lowercase());
    apps.dedup_by(|a, b| a.0 == b.0);
    apps
}

impl Engine {
    /// The open channel and its team's group id.
    async fn open_channel_group(self: &Arc<Self>) -> anyhow::Result<(String, String)> {
        let conv = self
            .view
            .lock()
            .unwrap()
            .current
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Open a channel first."))?;
        let group = self
            .channel_group(&conv)
            .await
            .filter(|g| !g.is_empty())
            .ok_or_else(|| anyhow::anyhow!("This channel's team is not known."))?;
        Ok((conv, group))
    }

    /// Reads the open channel's tabs for its header (nothing for chats).
    pub(super) async fn load_channel_tabs(self: &Arc<Self>, conv: &str) {
        self.ui.channel_tabs(vec![]);
        if !conv.contains("@thread.tacv2")
            || self.offline.load(std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        let Some(group) = self.channel_group(conv).await.filter(|g| !g.is_empty()) else {
            return;
        };
        match self.client.channel_tabs(&group, conv).await {
            Ok(tabs) => {
                let still_open = self.view.lock().unwrap().current.as_deref() == Some(conv);
                if still_open {
                    self.ui.channel_tabs(
                        tabs.into_iter()
                            .map(|t| (t.name, t.open_url, t.id))
                            .collect(),
                    );
                }
            }
            Err(err) => tracing::debug!(%err, "channel tabs"),
        }
    }

    /// "Add a tab" with a website: adds it to the open channel, then reads the tabs again.
    pub(super) async fn add_channel_tab(self: &Arc<Self>, arg: &str) -> anyhow::Result<()> {
        let Some((name, url)) = split_add(arg) else {
            anyhow::bail!("A tab needs a name and an https address.");
        };
        let (conv, group) = self.open_channel_group().await?;
        self.client
            .add_website_tab(&group, &conv, &name, &url)
            .await?;
        self.load_channel_tabs(&conv).await;
        Ok(())
    }

    /// "Add a tab" with an installed app (`app id` U+001F `name`). UNVERIFIED live: apps whose
    /// tab needs setup are added unconfigured.
    pub(super) async fn add_channel_app_tab(self: &Arc<Self>, arg: &str) -> anyhow::Result<()> {
        let Some((app, name)) = split_pair(arg) else {
            anyhow::bail!("A tab needs an app and a name.");
        };
        let (conv, group) = self.open_channel_group().await?;
        self.client.add_app_tab(&group, &conv, &name, &app).await?;
        self.load_channel_tabs(&conv).await;
        Ok(())
    }

    /// "Add a tab" in the open chat: a website (`name` U+001F `url`) or, with `app`, an installed
    /// app (`app id` U+001F `name`). UNVERIFIED: Graph's chat tabs were never tried from kelma,
    /// and kelma does not list a chat's tabs, so it says where the tab shows.
    pub(super) async fn add_chat_tab(self: &Arc<Self>, arg: &str, app: bool) -> anyhow::Result<()> {
        let parts = if app { split_pair(arg) } else { split_add(arg) };
        let Some((a, b)) = parts else {
            anyhow::bail!("A tab needs a name and an https address or an app.");
        };
        let chat = self
            .view
            .lock()
            .unwrap()
            .current
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Open a chat first."))?;
        if self.offline.load(std::sync::atomic::Ordering::Relaxed) {
            anyhow::bail!("kelma is offline: the tab was not added.");
        }
        if app {
            self.client.add_chat_app_tab(&chat, &b, &a).await?;
        } else {
            self.client.add_chat_website_tab(&chat, &a, &b).await?;
        }
        self.ui
            .banner("Tab added. kelma doesn't show chat tabs yet: open the chat in Teams to use it.".into());
        Ok(())
    }

    /// Renames a tab (`tab id` U+001F `new name`), then reads the tabs again.
    pub(super) async fn rename_channel_tab(self: &Arc<Self>, arg: &str) -> anyhow::Result<()> {
        let Some((id, name)) = split_pair(arg) else {
            anyhow::bail!("A tab needs a name.");
        };
        let (conv, group) = self.open_channel_group().await?;
        self.client
            .rename_channel_tab(&group, &conv, &id, &name)
            .await?;
        self.load_channel_tabs(&conv).await;
        Ok(())
    }

    /// Removes a tab from the open channel, then reads the tabs again.
    pub(super) async fn remove_channel_tab(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        if id.is_empty() {
            return Ok(());
        }
        let (conv, group) = self.open_channel_group().await?;
        self.client.remove_channel_tab(&group, &conv, id).await?;
        self.load_channel_tabs(&conv).await;
        Ok(())
    }

    /// The installed apps that can have a tab in a team, for the "Add a tab" dialog.
    pub(super) async fn load_tab_apps(self: &Arc<Self>) {
        self.ui.tab_apps(vec![]);
        if self.offline.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        match app_store::installed_apps(&self.client).await {
            Ok(installed) => self.ui.tab_apps(tab_apps(&installed)),
            Err(err) => tracing::debug!(%err, "installed apps for a tab"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_add_argument_needs_a_name_and_a_web_address() {
        assert_eq!(
            split_add("Docs\u{1f}example.com/docs"),
            Some(("Docs".into(), "https://example.com/docs".into()))
        );
        assert_eq!(
            split_add(" Board \u{1f} https://tasks.office.com/b "),
            Some(("Board".into(), "https://tasks.office.com/b".into()))
        );
        assert_eq!(split_add("\u{1f}https://x.com"), None);
        assert_eq!(split_add("Docs\u{1f}"), None);
        assert_eq!(split_add("Docs\u{1f}http://plain.example"), None);
        assert_eq!(split_add("no separator"), None);
    }

    #[test]
    fn rename_and_app_arguments_need_both_parts() {
        assert_eq!(
            split_pair(" t1 \u{1f} New name "),
            Some(("t1".into(), "New name".into()))
        );
        assert_eq!(split_pair("t1\u{1f}  "), None);
        assert_eq!(split_pair("\u{1f}name"), None);
        assert_eq!(split_pair("t1"), None);
    }

    #[test]
    fn only_installed_team_apps_with_tabs_are_offered_by_name() {
        let app = |id: &str, name: &str, scope: &str| {
            serde_json::json!({
                "id": id,
                "name": name,
                "metaCapabilities": {"configurableTabs": {"scopes": [scope]}},
                "galleryTabs": [{"context": ["channelTab"]}],
            })
        };
        let v = serde_json::json!({"value": {
            "userEntitlements": {"tenant": [
                {"id": "a", "state": "Installed"},
                {"id": "b", "state": "InstalledAndPermanent"},
                {"id": "c", "state": "Installed"},
                {"id": "d", "state": "Featured"},
            ]},
            "definitions": {
                "a": app("a", "Zebra", "Team"),
                "b": app("b", "apple", "Team"),
                "c": app("c", "Personal only", "Personal"),
                "d": app("d", "Not installed", "Team"),
            },
        }});
        let installed = app_store::parse_installed(&v);
        assert_eq!(
            tab_apps(&installed),
            [("b".into(), "apple".into()), ("a".into(), "Zebra".into())]
        );
    }
}
