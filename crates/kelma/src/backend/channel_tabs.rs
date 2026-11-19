//! A channel's tabs beyond Posts and Files (Teams' "+" Add a tab): listed in the channel header,
//! opened in the browser, and a website tab can be added.

use super::Engine;
use std::sync::Arc;

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

impl Engine {
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
                    self.ui
                        .channel_tabs(tabs.into_iter().map(|t| (t.name, t.open_url)).collect());
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
        let Some(conv) = self.view.lock().unwrap().current.clone() else {
            return Ok(());
        };
        let group = self
            .channel_group(&conv)
            .await
            .filter(|g| !g.is_empty())
            .ok_or_else(|| anyhow::anyhow!("This channel's team is not known."))?;
        self.client
            .add_website_tab(&group, &conv, &name, &url)
            .await?;
        self.load_channel_tabs(&conv).await;
        Ok(())
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
}
