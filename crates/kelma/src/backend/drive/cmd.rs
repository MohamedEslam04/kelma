//! The commands the OneDrive view sends, as `(name, argument)` strings from the `DriveStore`
//! global.

use onedrive::DocKind;
use std::path::PathBuf;

/// Everything the OneDrive view asks the engine to do. Commands that act on "the selection" carry
/// no id; row buttons select their row first.
#[derive(Debug, Clone, PartialEq)]
pub enum DriveCmd {
    /// The OneDrive view was opened (loads Home the first time).
    Enter,
    Nav(String),
    /// Double click or Enter on a row: a folder opens, a file previews.
    OpenRow(String),
    Crumb(usize),
    Refresh,
    LoadMore,
    Select(String),
    Toggle(String),
    SelectAll,
    ClearSelection,
    View(String),
    Sort(String),
    Group(String),
    Column(String),
    Type(String),
    Filter(String),
    Search(String),
    NewFolder(String),
    NewDoc(DocKind),
    /// `name, a newline, then the address`: an Internet shortcut file in the open folder.
    NewLink(String),
    NewNotebook(String),
    /// Opens Microsoft Forms in the browser (it has no API for this client).
    NewForms,
    /// A "For you" card on Home was clicked.
    CardOpen(String),
    UploadFiles,
    UploadFolder,
    Drop(PathBuf),
    OpenKelma,
    OpenBrowser,
    OpenDesktop,
    OpenLocation,
    /// Favorite toggle for one item, or for the selection when the id is empty.
    Favorite(String),
    Shortcut,
    Rename(String),
    /// Opens the folder picker for moving the selection.
    MoveTo,
    CopyTo,
    Download,
    Delete,
    Restore,
    DeleteForever,
    CopyLink,
    Versions,
    VersionRestore(String),
    Details,
    Sync,
    Share,
    ShareSearch(String),
    SharePick(String),
    ShareRole(String),
    /// Send the invitation with this message.
    ShareSend(String),
    /// `scope,type,block-download,expiry-days`, for example `organization,view,0,7`.
    ShareLink(String),
    SharePermRole(String, String),
    SharePermRemove(String),
    /// Opens the file picker for a purpose (`move`, `copy`, or one another feature names).
    PickBegin(String),
    PickOpen(String),
    PickCrumb(usize),
    PickSelect(String),
    PickChoose,
    PickCancel,
    DialogClose,
}

impl DriveCmd {
    /// Commands that only change the view state: no network, run inline and in order.
    pub fn is_instant(&self) -> bool {
        matches!(
            self,
            DriveCmd::Select(_)
                | DriveCmd::Toggle(_)
                | DriveCmd::SelectAll
                | DriveCmd::ClearSelection
                | DriveCmd::View(_)
                | DriveCmd::Sort(_)
                | DriveCmd::Group(_)
                | DriveCmd::Column(_)
                | DriveCmd::Type(_)
                | DriveCmd::Filter(_)
        )
    }

    pub fn parse(name: &str, arg: &str) -> Option<DriveCmd> {
        let s = || arg.to_owned();
        Some(match name {
            "enter" => DriveCmd::Enter,
            "nav" => DriveCmd::Nav(s()),
            "open-row" => DriveCmd::OpenRow(s()),
            "crumb" => DriveCmd::Crumb(arg.parse().ok()?),
            "refresh" => DriveCmd::Refresh,
            "load-more" => DriveCmd::LoadMore,
            "select" => DriveCmd::Select(s()),
            "toggle" => DriveCmd::Toggle(s()),
            "select-all" => DriveCmd::SelectAll,
            "clear-selection" => DriveCmd::ClearSelection,
            "view" => DriveCmd::View(s()),
            "sort" => DriveCmd::Sort(s()),
            "group" => DriveCmd::Group(s()),
            "column" => DriveCmd::Column(s()),
            "type" => DriveCmd::Type(s()),
            "filter" => DriveCmd::Filter(s()),
            "search" => DriveCmd::Search(s()),
            "new-folder" => DriveCmd::NewFolder(s()),
            "new-doc" => DriveCmd::NewDoc(parse_doc_kind(arg)?),
            "new-link" => DriveCmd::NewLink(s()),
            "new-notebook" => DriveCmd::NewNotebook(s()),
            "new-forms" => DriveCmd::NewForms,
            "card-open" => DriveCmd::CardOpen(s()),
            "upload-files" => DriveCmd::UploadFiles,
            "upload-folder" => DriveCmd::UploadFolder,
            "drop" => DriveCmd::Drop(PathBuf::from(arg)),
            "open-kelma" => DriveCmd::OpenKelma,
            "open-browser" => DriveCmd::OpenBrowser,
            "open-desktop" => DriveCmd::OpenDesktop,
            "open-location" => DriveCmd::OpenLocation,
            "favorite" => DriveCmd::Favorite(s()),
            "shortcut" => DriveCmd::Shortcut,
            "rename" => DriveCmd::Rename(s()),
            "move-to" => DriveCmd::MoveTo,
            "copy-to" => DriveCmd::CopyTo,
            "download" => DriveCmd::Download,
            "delete" => DriveCmd::Delete,
            "restore" => DriveCmd::Restore,
            "delete-forever" => DriveCmd::DeleteForever,
            "copy-link" => DriveCmd::CopyLink,
            "versions" => DriveCmd::Versions,
            "version-restore" => DriveCmd::VersionRestore(s()),
            "details" => DriveCmd::Details,
            "sync" => DriveCmd::Sync,
            "share" => DriveCmd::Share,
            "share-search" => DriveCmd::ShareSearch(s()),
            "share-pick" => DriveCmd::SharePick(s()),
            "share-role" => DriveCmd::ShareRole(s()),
            "share-send" => DriveCmd::ShareSend(s()),
            "share-link" => DriveCmd::ShareLink(s()),
            "share-perm-role" => {
                let (id, role) = arg.split_once('|')?;
                DriveCmd::SharePermRole(id.into(), role.into())
            }
            "share-perm-remove" => DriveCmd::SharePermRemove(s()),
            "pick-begin" => DriveCmd::PickBegin(s()),
            "pick-open" => DriveCmd::PickOpen(s()),
            "pick-crumb" => DriveCmd::PickCrumb(arg.parse().ok()?),
            "pick-select" => DriveCmd::PickSelect(s()),
            "pick-choose" => DriveCmd::PickChoose,
            "pick-cancel" => DriveCmd::PickCancel,
            "dialog-close" => DriveCmd::DialogClose,
            _ => return None,
        })
    }
}

fn parse_doc_kind(s: &str) -> Option<DocKind> {
    Some(match s {
        "word" => DocKind::Word,
        "excel" => DocKind::Excel,
        "powerpoint" => DocKind::PowerPoint,
        "visio" => DocKind::Visio,
        "markdown" => DocKind::Markdown,
        "text" => DocKind::Text,
        _ => return None,
    })
}

/// A parsed `share-link` argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSettings {
    pub scope: onedrive::LinkScope,
    pub kind: onedrive::LinkType,
    pub block_download: bool,
    pub expiry_days: Option<u32>,
}

impl LinkSettings {
    /// `scope,type,block,days` with scope `anyone|organization|users`, type `view|edit`, block
    /// `0|1` and days `0` for no expiry.
    pub fn parse(s: &str) -> Option<LinkSettings> {
        let mut p = s.split(',');
        let scope = match p.next()? {
            "anyone" => onedrive::LinkScope::Anonymous,
            "organization" => onedrive::LinkScope::Organization,
            "users" => onedrive::LinkScope::Users,
            _ => return None,
        };
        let kind = match p.next()? {
            "view" => onedrive::LinkType::View,
            "edit" => onedrive::LinkType::Edit,
            _ => return None,
        };
        let block_download = p.next()? == "1";
        let days: u32 = p.next()?.parse().ok()?;
        Some(LinkSettings {
            scope,
            kind,
            block_download,
            expiry_days: (days > 0).then_some(days),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_parse_with_their_arguments() {
        assert_eq!(
            DriveCmd::parse("nav", "shared"),
            Some(DriveCmd::Nav("shared".into()))
        );
        assert_eq!(DriveCmd::parse("crumb", "2"), Some(DriveCmd::Crumb(2)));
        assert_eq!(DriveCmd::parse("crumb", "x"), None);
        assert_eq!(
            DriveCmd::parse("new-doc", "excel"),
            Some(DriveCmd::NewDoc(DocKind::Excel))
        );
        assert_eq!(
            DriveCmd::parse("new-doc", "visio"),
            Some(DriveCmd::NewDoc(DocKind::Visio))
        );
        assert_eq!(DriveCmd::parse("new-doc", "forms"), None);
        assert_eq!(DriveCmd::parse("new-forms", ""), Some(DriveCmd::NewForms));
        assert_eq!(
            DriveCmd::parse("new-link", "Docs\nhttps://x.example/"),
            Some(DriveCmd::NewLink("Docs\nhttps://x.example/".into()))
        );
        assert_eq!(
            DriveCmd::parse("card-open", "c1"),
            Some(DriveCmd::CardOpen("c1".into()))
        );
        assert_eq!(
            DriveCmd::parse("drop", "/tmp/a b.txt"),
            Some(DriveCmd::Drop("/tmp/a b.txt".into()))
        );
        assert_eq!(
            DriveCmd::parse("share-perm-role", "perm1|write"),
            Some(DriveCmd::SharePermRole("perm1".into(), "write".into()))
        );
        assert_eq!(DriveCmd::parse("share-perm-role", "nope"), None);
        assert_eq!(
            DriveCmd::parse("favorite", ""),
            Some(DriveCmd::Favorite(String::new()))
        );
        assert_eq!(DriveCmd::parse("bogus", ""), None);
    }

    #[test]
    fn link_settings_parse_or_refuse() {
        let s = LinkSettings::parse("organization,view,1,7").unwrap();
        assert_eq!(s.scope, onedrive::LinkScope::Organization);
        assert_eq!(s.kind, onedrive::LinkType::View);
        assert!(s.block_download);
        assert_eq!(s.expiry_days, Some(7));
        assert_eq!(
            LinkSettings::parse("anyone,edit,0,0").unwrap().expiry_days,
            None
        );
        assert_eq!(LinkSettings::parse("everyone,edit,0,0"), None);
        assert_eq!(LinkSettings::parse("anyone,edit"), None);
    }
}
