//! File chips in chat messages (X1.6): a Word, Excel or PowerPoint attachment opens where
//! Settings > Files and links says, like the same file in OneDrive. The chip sends
//! `kelma-file:<TYPE>:<url>`; `chip_open_plan` is the pure decision, `open_file_chip` runs it.

use super::{DriveCmd, Engine, FileKind, drive_open_web_url};
use crate::backend::settings::{OpenFilesIn, files_prefs};
use onedrive::{DriveItem, browse};
use std::sync::Arc;

/// Where a chip's file opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipOpen {
    /// The web address in the browser.
    Browser,
    /// Download and hand to the desktop's default app.
    Desktop,
    /// IronTeams's own preview of the file.
    IronTeams,
}

/// The chip's `<TYPE>:<url>` (what follows `kelma-file:`) as `(type, url)`.
pub fn parse_chip(arg: &str) -> Option<(&str, &str)> {
    let (ext, url) = arg.split_once(':')?;
    (url.starts_with("https://") || url.starts_with("http://")).then_some((ext, url))
}

/// The file type of a chip: the posted type, or (for the generic `FILE`) the extension of the
/// address's last path segment. Lower case, no dot.
fn chip_ext(ext: &str, url: &str) -> String {
    let ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if !ext.is_empty() && ext != "file" {
        return ext;
    }
    let path = url.split(['?', '#']).next().unwrap_or_default();
    path.rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

fn is_office(ext: &str) -> bool {
    let probe = DriveItem {
        name: format!("file.{ext}"),
        ..DriveItem::default()
    };
    matches!(
        FileKind::of(&probe),
        FileKind::Word | FileKind::Excel | FileKind::PowerPoint
    )
}

/// The choice for one chip. Only Office files on a SharePoint/OneDrive address follow the
/// Settings choice (the others cannot be resolved to an item, or keep their behavior): the
/// browser stays the default for everything else.
pub fn chip_open_plan(url: &str, ext: &str, pref: OpenFilesIn) -> ChipOpen {
    if !teams_proto::images::is_sharepoint_url(url) || !is_office(&chip_ext(ext, url)) {
        return ChipOpen::Browser;
    }
    match pref {
        OpenFilesIn::Browser => ChipOpen::Browser,
        OpenFilesIn::Desktop => ChipOpen::Desktop,
        OpenFilesIn::IronTeams => ChipOpen::IronTeams,
    }
}

/// The file card's "Open in …" place and its second line: `OneDrive` for a personal site,
/// `SharePoint` for a team site, `""` elsewhere; the line says where a click opens the file.
pub fn file_card_labels(url: &str, ext: &str, pref: OpenFilesIn) -> (&'static str, &'static str) {
    let place = if !teams_proto::images::is_sharepoint_url(url) {
        ""
    } else if url
        .split('/')
        .nth(2)
        .is_some_and(|h| h.to_ascii_lowercase().contains("-my.sharepoint."))
    {
        "OneDrive"
    } else {
        "SharePoint"
    };
    let detail = match chip_open_plan(url, ext, pref) {
        ChipOpen::Browser => "Opens in the browser",
        ChipOpen::Desktop => "Opens in the desktop app",
        ChipOpen::IronTeams => "Opens in IronTeams",
    };
    (place, detail)
}

/// A free name for `name` in `dir`: `name`, then `name (1).ext`, `name (2).ext`, ...
fn free_path(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    (1..1000)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

impl Engine {
    /// The file card's "…" menu: `download` (to the Downloads folder), `copy` (the link),
    /// `share` (the OneDrive share dialog) or `web` (open in OneDrive or SharePoint). `url` is
    /// the file's address as posted.
    pub(in crate::backend) async fn file_card_action(
        self: &Arc<Self>,
        action: &str,
        url: &str,
    ) -> anyhow::Result<()> {
        match action {
            "copy" => {
                arboard::Clipboard::new()?.set_text(url.to_string())?;
                self.ui.banner("Link copied".into());
            }
            "web" => drive_open_web_url(url),
            "download" | "share" if self.drive_offline() => {
                self.ui
                    .banner("This needs a connection to OneDrive or SharePoint".into());
            }
            "download" => {
                let item = browse::item_from_share(&self.client, url).await?;
                if item.is_folder {
                    anyhow::bail!("That link is a folder; open it in the browser instead");
                }
                self.ui.banner(format!("Downloading {}...", item.name));
                let bytes = super::ops::download(&self.client, &item).await?;
                let dir = diag::default_output_dir();
                let path = free_path(&dir, &super::sanitize_file_name(&item.name));
                let shown = path.clone();
                tokio::task::spawn_blocking(move || std::fs::write(path, bytes)).await??;
                self.ui.banner(format!("Saved to {}", shown.display()));
            }
            "share" => {
                let item = browse::item_from_share(&self.client, url).await?;
                // The share dialog belongs to the Files view: bring it up first.
                self.drive_cmd(DriveCmd::Enter).await?;
                self.ui.show_view("onedrive");
                self.drive_share_open(item).await?;
            }
            _ => {}
        }
        Ok(())
    }

    /// A file chip was clicked (`arg` is `<TYPE>:<url>`).
    pub(in crate::backend) async fn open_file_chip(self: &Arc<Self>, arg: &str) {
        let Some((ext, url)) = parse_chip(arg) else {
            return;
        };
        let plan = chip_open_plan(url, ext, files_prefs().open_office_in);
        // Demo mode has no Graph: the browser path only logs there.
        if plan == ChipOpen::Browser || self.drive_offline() {
            drive_open_web_url(url);
            return;
        }
        if let Err(err) = self.open_chip_item(url, plan).await {
            tracing::info!(%err, "file chip: no desktop or preview open, using the browser");
            drive_open_web_url(url);
        }
    }

    /// Resolves the item behind the link (Graph `shares`) and opens it the OneDrive way.
    async fn open_chip_item(self: &Arc<Self>, url: &str, plan: ChipOpen) -> anyhow::Result<()> {
        let item = browse::item_from_share(&self.client, url).await?;
        if item.is_folder {
            anyhow::bail!("the link is a folder");
        }
        match plan {
            ChipOpen::Desktop => self.drive_open_external(item).await,
            ChipOpen::IronTeams => {
                // The preview dialog belongs to the Files view: bring it up first.
                self.drive_cmd(DriveCmd::Enter).await?;
                self.ui.show_view("onedrive");
                self.drive_open_item(item).await
            }
            ChipOpen::Browser => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use OpenFilesIn::*;

    const SP: &str = "https://contoso-my.sharepoint.com/:w:/p/ann/EabcDEF?e=x1";
    const SP_NAMED: &str = "https://contoso.sharepoint.com/sites/t/Shared%20Documents/plan.xlsx";

    #[test]
    fn file_card_names_the_place_and_where_a_click_opens() {
        assert_eq!(
            file_card_labels(SP, "DOCX", Desktop),
            ("OneDrive", "Opens in the desktop app")
        );
        assert_eq!(
            file_card_labels(SP_NAMED, "XLSX", IronTeams),
            ("SharePoint", "Opens in IronTeams")
        );
        assert_eq!(
            file_card_labels(SP_NAMED, "PDF", IronTeams),
            ("SharePoint", "Opens in the browser")
        );
        assert_eq!(
            file_card_labels("https://example.com/a.zip", "ZIP", Desktop),
            ("", "Opens in the browser")
        );
    }

    #[test]
    fn downloads_never_overwrite_a_file() {
        let dir = std::env::temp_dir().join(format!("ironteams-free-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(free_path(&dir, "plan.docx"), dir.join("plan.docx"));
        std::fs::write(dir.join("plan.docx"), b"x").unwrap();
        assert_eq!(free_path(&dir, "plan.docx"), dir.join("plan (1).docx"));
        std::fs::write(dir.join("notes"), b"x").unwrap();
        assert_eq!(free_path(&dir, "notes"), dir.join("notes (1)"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn chip_text_is_split_into_type_and_address() {
        assert_eq!(
            parse_chip("DOCX:https://a.sharepoint.com/x?y=1:2"),
            Some(("DOCX", "https://a.sharepoint.com/x?y=1:2"))
        );
        assert_eq!(parse_chip("DOCX:javascript:alert(1)"), None);
        assert_eq!(parse_chip("no-colon"), None);
    }

    #[test]
    fn browser_choice_always_opens_the_web_address() {
        for ext in ["DOCX", "XLSX", "PPTX", "PDF", "FILE"] {
            assert_eq!(chip_open_plan(SP, ext, Browser), ChipOpen::Browser, "{ext}");
        }
    }

    #[test]
    fn office_chips_follow_the_desktop_and_ironteams_choices() {
        for ext in ["DOCX", "xlsx", "PPTX", "doc", "XLS", "ppt"] {
            assert_eq!(chip_open_plan(SP, ext, Desktop), ChipOpen::Desktop, "{ext}");
            assert_eq!(
                chip_open_plan(SP, ext, IronTeams),
                ChipOpen::IronTeams,
                "{ext}"
            );
        }
    }

    #[test]
    fn a_generic_type_falls_back_to_the_extension_in_the_address() {
        assert_eq!(chip_open_plan(SP_NAMED, "FILE", Desktop), ChipOpen::Desktop);
        assert_eq!(chip_open_plan(SP_NAMED, "", IronTeams), ChipOpen::IronTeams);
        // A sharing link without a file name cannot tell: stays in the browser.
        assert_eq!(chip_open_plan(SP, "FILE", Desktop), ChipOpen::Browser);
    }

    #[test]
    fn other_files_and_other_hosts_keep_the_browser() {
        assert_eq!(chip_open_plan(SP, "PDF", Desktop), ChipOpen::Browser);
        assert_eq!(chip_open_plan(SP, "PNG", IronTeams), ChipOpen::Browser);
        assert_eq!(
            chip_open_plan("https://example.com/a.docx", "DOCX", Desktop),
            ChipOpen::Browser
        );
        assert_eq!(
            chip_open_plan("http://contoso.sharepoint.com/a.docx", "DOCX", Desktop),
            ChipOpen::Browser
        );
    }
}
