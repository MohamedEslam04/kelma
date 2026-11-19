//! OneDrive: the Files view of the rail. The pure view state lives in `state`, the commands in
//! `cmd`; this file runs them against `crates/onedrive` (or an in-memory demo tree offline) and
//! pushes the result to the UI.

mod cmd;
pub(crate) mod state;

pub use cmd::DriveCmd;
use cmd::LinkSettings;
use state::{Card, Crumb, FileKind, Library, Preview, Section, State};
pub use state::{DriveData, RowData};

use super::Engine;
use crate::data::Pixels;
use onedrive::{
    DriveItem, ItemRef, NewItem, Permission, PersonFiles, Recipient, Role, Version, bin, browse,
    insights, ops, share,
};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use teams_proto::time::now_ms;

/// Pictures and text bigger than this open in the browser instead of kelma's viewer.
const MAX_PREVIEW_BYTES: u64 = 15 * 1024 * 1024;
/// How much of a text file the viewer shows.
const MAX_TEXT_CHARS: usize = 200_000;
/// Files bigger than this are not downloaded to be opened by a desktop app.
const MAX_OPEN_BYTES: u64 = 100 * 1024 * 1024;
/// Uploads are read into memory; refuse what would not fit comfortably.
const MAX_UPLOAD_BYTES: u64 = 250 * 1024 * 1024;
/// Thumbnails are fetched for this many items of a listing.
const MAX_THUMBS: usize = 48;
/// Cards in Home's "For you" row.
const MAX_CARDS: usize = 8;
/// Photos and videos per page of Browse files by Media.
const MEDIA_PAGE: u32 = 50;
const SHARE_HELP: &str = "Choose people and what they can do, or create a link.";

/// A person in the share dialog's picker.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonData {
    /// The MRI (`8:orgid:<oid>`).
    pub id: String,
    pub name: String,
    pub detail: String,
    pub picked: bool,
    email: Option<String>,
}

/// Who or what has access, as the "Manage access" list shows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PermData {
    pub id: String,
    pub who: String,
    /// `read`, `write` or `owner`.
    pub role: String,
    pub link: bool,
    pub detail: String,
    /// The role can be changed and the access removed.
    pub editable: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ShareData {
    pub name: String,
    pub results: Vec<PersonData>,
    pub picked: Vec<PersonData>,
    pub perms: Vec<PermData>,
    /// `read` or `write`.
    pub role: &'static str,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VersionData {
    pub id: String,
    pub when: String,
    pub by: String,
    pub size: String,
    pub current: bool,
}

#[derive(Clone, Default)]
pub struct PickerData {
    pub purpose: String,
    pub title: String,
    /// Folders only (the destination of Move to and Copy to); else files can be picked too.
    pub folders_only: bool,
    pub rows: Vec<RowData>,
    pub crumbs: Vec<String>,
    pub loading: bool,
    pub can_choose: bool,
}

#[derive(Clone, Default)]
pub struct PreviewData {
    pub title: String,
    /// `image`, `text`, `markdown`.
    pub kind: &'static str,
    pub image: Option<Pixels>,
    pub text: String,
    /// Pages previews only: the page shown (from 1) and how many there are.
    pub page: usize,
    pub pages: usize,
}

struct Picker {
    purpose: String,
    folders_only: bool,
    trail: Vec<Crumb>,
    items: Vec<DriveItem>,
    selected: Option<String>,
    loading: bool,
    generation: u64,
}

struct Share {
    item: DriveItem,
    role: Role,
    results: Vec<PersonData>,
    picked: Vec<PersonData>,
    perms: Vec<Permission>,
    status: String,
}

/// A document open page by page in the preview dialog.
struct OpenPages {
    doc: Arc<preview::Document>,
    name: String,
    /// The page shown, from 0.
    page: usize,
}

/// How wide a page is rendered, in pixels.
const PAGE_WIDTH: u32 = 1000;

/// The OneDrive part of the engine's state.
#[derive(Default)]
pub struct Drive {
    s: State,
    entered: bool,
    /// Libraries, favorites and the "For you" cards load once, from whichever feature needs
    /// them first (the OneDrive view or the cloud-file picker).
    background_started: bool,
    /// Who shared files with the user, from the last People listing (a person's files open
    /// from it).
    people: Vec<PersonFiles>,
    /// The PDF (or converted Office file) open in the preview dialog.
    pages: Option<OpenPages>,
    my_drive_id: Option<String>,
    picker: Option<Picker>,
    share: Option<Share>,
    versions_of: Option<DriveItem>,
}

// -- pure helpers --------------------------------------------------------------------------

/// Local time minus UTC at `ms`, in milliseconds.
fn local_offset(ms: i64) -> i64 {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map_or(0, |t| i64::from(t.offset().local_minus_utc()) * 1000)
}

/// Folders of a drive path, outermost first: `/Docs/Reports` gives `Docs`, `Docs/Reports`.
fn path_prefixes(parent_path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut acc = String::new();
    for seg in parent_path.split('/').filter(|s| !s.is_empty()) {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(seg);
        out.push(acc.clone());
    }
    out
}

/// The lines of the Details dialog.
fn details_text(i: &DriveItem, favorite: bool) -> String {
    let when = |ms: i64| {
        if ms <= 0 {
            "unknown".to_owned()
        } else {
            chrono::DateTime::from_timestamp_millis(ms + local_offset(ms))
                .map_or_else(String::new, |t| t.format("%Y-%m-%d %H:%M").to_string())
        }
    };
    let kind = FileKind::of(i).as_str();
    let mut lines = vec![
        format!("Name: {}", i.name),
        format!("Type: {}", if i.is_folder { "Folder" } else { kind }),
    ];
    if i.is_folder {
        lines.push(format!("Items: {}", i.child_count));
    } else {
        lines.push(format!("Size: {}", state::size_label(i.size)));
    }
    lines.push(format!(
        "Location: {}",
        if i.parent_path.is_empty() {
            "/"
        } else {
            &i.parent_path
        }
    ));
    lines.push(format!(
        "Modified: {} by {}",
        when(i.modified_ms),
        non_empty(&i.modified_by)
    ));
    if !i.created_by.is_empty() {
        lines.push(format!("Created by: {}", i.created_by));
    }
    if i.is_remote_shortcut {
        lines.push("Shortcut or shared item".into());
    }
    lines.push(format!("Favorite: {}", if favorite { "yes" } else { "no" }));
    if !i.web_url.is_empty() {
        lines.push(format!("Web address: {}", i.web_url));
    }
    lines.join("\n")
}

fn non_empty(s: &str) -> &str {
    if s.is_empty() { "unknown" } else { s }
}

/// Settings > Files and links: where Word, Excel and PowerPoint files open. The browser and the
/// desktop app replace kelma's own preview; every other file keeps its preview.
fn with_files_pref(
    pref: super::settings::OpenFilesIn,
    preview: Preview,
    kind: FileKind,
) -> Preview {
    use super::settings::OpenFilesIn;
    if !matches!(
        kind,
        FileKind::Word | FileKind::Excel | FileKind::PowerPoint
    ) {
        return preview;
    }
    match pref {
        OpenFilesIn::Kelma => preview,
        OpenFilesIn::Browser => Preview::Browser,
        // Downloads the file and hands it to the desktop's default app.
        OpenFilesIn::Desktop => Preview::Player,
    }
}

/// `expirationDateTime` for a link that lasts `days` days.
fn link_expiry(days: u32, now: i64) -> String {
    calendar::format_iso_ms(now + i64::from(days) * 86_400_000)
}

fn role_str(r: Role) -> &'static str {
    match r {
        Role::Read => "read",
        Role::Write => "write",
    }
}

fn parse_role(s: &str) -> Role {
    if s == "read" { Role::Read } else { Role::Write }
}

/// A directory person as the invite call wants them: by e-mail, else by object id.
fn recipient_of(p: &PersonData) -> Option<Recipient> {
    match (&p.email, teams_proto::people::oid_of_mri(&p.id)) {
        (Some(e), _) if !e.is_empty() => Some(Recipient::Email(e.clone())),
        (_, Some(oid)) => Some(Recipient::ObjectId(oid.to_owned())),
        _ => None,
    }
}

fn perm_data(p: &Permission, now: i64) -> PermData {
    let owner = p.roles.iter().any(|r| r == "owner");
    let role = if owner {
        "owner"
    } else if p.roles.iter().any(|r| r == "write") {
        "write"
    } else {
        "read"
    };
    let link = p.link_scope.is_some();
    let who = if link {
        match p.link_scope.as_deref() {
            Some("anonymous") => "Anyone with the link".to_owned(),
            Some("organization") => "People in your organization with the link".to_owned(),
            _ => "People you chose, with the link".to_owned(),
        }
    } else if p.granted_to.is_empty() {
        "Unknown".to_owned()
    } else {
        p.granted_to.join(", ")
    };
    let mut detail = Vec::new();
    if p.inherited {
        detail.push("inherited".to_owned());
    }
    if p.link_type.as_deref() == Some("blocksDownload") {
        detail.push("download blocked".into());
    }
    if let Some(e) = p.expires_ms {
        detail.push(format!(
            "expires {}",
            state::when_label(e, now, local_offset(e))
        ));
    }
    PermData {
        id: p.id.clone(),
        who,
        role: role.into(),
        link,
        detail: detail.join(", "),
        editable: !owner && !p.inherited,
    }
}

fn version_data(versions: &[Version], now: i64) -> Vec<VersionData> {
    versions
        .iter()
        .enumerate()
        .map(|(n, v)| VersionData {
            id: v.id.clone(),
            when: state::when_label(v.modified_ms, now, local_offset(v.modified_ms)),
            by: v.modified_by.clone(),
            size: state::size_label(v.size),
            current: n == 0,
        })
        .collect()
}

fn bin_item(b: bin::BinItem) -> DriveItem {
    DriveItem {
        id: b.id,
        name: b.name,
        size: b.size,
        is_folder: b.is_folder,
        modified_ms: b.deleted_ms,
        modified_by: b.deleted_by,
        parent_path: format!("/{}", b.dir.trim_matches('/')),
        ..Default::default()
    }
}

fn library_item(l: &Library) -> DriveItem {
    DriveItem {
        id: "root".into(),
        drive_id: l.drive_id.clone(),
        name: l.name.clone(),
        is_folder: true,
        ..Default::default()
    }
}

fn my_files_item() -> DriveItem {
    DriveItem {
        id: "root".into(),
        name: "My files".into(),
        is_folder: true,
        ..Default::default()
    }
}

/// Where a thumbnail comes from: a listing that carries its address, or the item itself.
#[derive(Clone)]
enum ThumbSource {
    Url(String),
    Item(ItemRef),
}

/// What the People list calls a person.
fn person_label(p: &onedrive::insights::Person) -> String {
    [&p.name, &p.address]
        .into_iter()
        .find(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| "Unknown".into())
}

/// The People list as rows: one per person, opening to their files.
fn people_items(people: &[PersonFiles]) -> Vec<DriveItem> {
    people
        .iter()
        .enumerate()
        .map(|(n, p)| DriveItem {
            id: format!("person:{n}"),
            name: person_label(&p.person),
            is_folder: true,
            child_count: u32::try_from(p.files.len()).unwrap_or(u32::MAX),
            modified_ms: p.latest_ms,
            ..Default::default()
        })
        .collect()
}

/// "Shared by you" as rows. Only drive items can be acted on, so shares of other things (a
/// page, a mail attachment) are left out; the sharer is the user, so no name is shown.
fn shared_by_you_items(shared: Vec<insights::Insight>) -> Vec<DriveItem> {
    shared
        .iter()
        .map(insights::Insight::to_drive_item)
        .filter(|i| !i.id.is_empty() && !i.drive_id.is_empty())
        .map(|i| DriveItem {
            modified_by: String::new(),
            ..i
        })
        .collect()
}

/// Meeting recordings as rows, with the place (`My files` or `<team> › <channel>`) as their
/// location. Recordings in My files keep their folder path so Open location still works.
fn recording_items(recs: Vec<onedrive::Recording>) -> Vec<DriveItem> {
    recs.into_iter()
        .map(|r| {
            if r.place == onedrive::around::MY_FILES {
                r.item
            } else {
                DriveItem {
                    parent_path: r.place,
                    ..r.item
                }
            }
        })
        .collect()
}

/// `drive_id` of the picker's "Recent" and "Shared with me" entries: lists, not folders.
const VIRTUAL: &str = "@virtual";

/// The extra top-level entries of the picker when attaching files: Teams offers Recent and
/// Shared with me next to My files and the team libraries.
fn virtual_items() -> Vec<DriveItem> {
    [("Recent", "recent"), ("Shared with me", "shared")]
        .into_iter()
        .map(|(name, id)| DriveItem {
            id: id.into(),
            drive_id: VIRTUAL.into(),
            name: name.into(),
            is_folder: true,
            ..Default::default()
        })
        .collect()
}

fn picker_data(p: &Picker, libs: &[Library]) -> PickerData {
    let s = State {
        items: p.items.clone(),
        selected: p.selected.iter().cloned().collect(),
        libraries: libs.to_vec(),
        ..State::default()
    };
    PickerData {
        purpose: p.purpose.clone(),
        title: match p.purpose.as_str() {
            "move" => "Move to",
            "copy" => "Copy to",
            _ => "Choose a file",
        }
        .into(),
        folders_only: p.folders_only,
        // The top level is a fixed list (My files first); inside a folder, folders come first.
        rows: if p.trail.is_empty() {
            s.plain_rows(now_ms(), &local_offset)
        } else {
            s.rows(now_ms(), &local_offset)
        },
        crumbs: std::iter::once("OneDrive".to_owned())
            .chain(p.trail.iter().map(|c| c.name.clone()))
            .collect(),
        loading: p.loading,
        can_choose: if p.folders_only {
            !p.trail.is_empty()
        } else {
            p.selected.is_some()
        },
    }
}

// -- the engine -----------------------------------------------------------------------------

impl Engine {
    fn drive_offline(&self) -> bool {
        self.offline.load(Ordering::Relaxed)
    }

    fn drive_push(&self) {
        let data = {
            let d = self.drive.lock().unwrap();
            d.s.data(now_ms(), &local_offset)
        };
        self.ui.drive(data);
    }

    fn drive_status(&self, text: impl Into<String>) {
        self.drive.lock().unwrap().s.status = text.into();
        self.drive_push();
    }

    fn drive_selection(&self) -> Vec<DriveItem> {
        let d = self.drive.lock().unwrap();
        d.s.selected_items().into_iter().cloned().collect()
    }

    /// The one selected item, or a status line saying what is needed.
    fn drive_one(&self) -> Option<DriveItem> {
        let sel = self.drive_selection();
        if let [one] = sel.as_slice() {
            return Some(one.clone());
        }
        self.drive_status("Select one item first");
        None
    }

    pub(super) async fn drive_cmd(self: &Arc<Self>, cmd: DriveCmd) -> anyhow::Result<()> {
        use DriveCmd::*;
        match cmd {
            Enter => {
                let first = !std::mem::replace(&mut self.drive.lock().unwrap().entered, true);
                if first {
                    self.drive_start_background();
                    return self.drive_go(Section::Home).await;
                }
                self.drive_push();
            }
            Nav(key) => {
                if let Some(section) = Section::from_key(&key) {
                    self.drive_go(section).await?;
                }
            }
            OpenRow(id) => self.drive_open_row(&id).await?,
            // The first crumb of a person's files is the People list.
            Crumb(0) if matches!(self.drive.lock().unwrap().s.section, Section::Person(_)) => {
                self.drive_go(Section::People).await?;
            }
            Crumb(i) => {
                let g = self.drive.lock().unwrap().s.go_to_crumb(i);
                if let Some(g) = g {
                    self.drive_push();
                    self.drive_load(g).await?;
                }
            }
            Refresh => {
                let g = {
                    let mut d = self.drive.lock().unwrap();
                    d.s.loading = true;
                    d.s.generation += 1;
                    d.s.generation
                };
                self.drive_push();
                if self.drive.lock().unwrap().s.section == Section::Home {
                    let e = self.clone();
                    tokio::spawn(async move { e.drive_load_cards().await });
                }
                self.drive_load(g).await?;
            }
            LoadMore => self.drive_load_more().await?,
            Select(_) | Toggle(_) | SelectAll | ClearSelection | View(_) | Sort(_) | Group(_)
            | Column(_) | Type(_) | Filter(_) => self.drive_instant(cmd),
            Search(q) => {
                let q = q.trim().to_owned();
                let section = if q.is_empty() {
                    Section::Home
                } else {
                    Section::Search(q)
                };
                self.drive_go(section).await?;
            }
            NewFolder(name) => self.drive_new_folder(name.trim()).await?,
            NewDoc(kind) => self.drive_new_doc(kind).await?,
            NewLink(arg) => self.drive_new_link(&arg).await?,
            NewNotebook(name) => self.drive_new_notebook(name.trim()).await?,
            NewForms => self.drive_new_forms(),
            CardOpen(id) => self.drive_card_open(&id).await?,
            UploadFiles => self.drive_upload_pick(false).await?,
            UploadFolder => self.drive_upload_pick(true).await?,
            Drop(path) => self.drive_upload_paths(vec![path]).await?,
            OpenKelma => {
                if let Some(i) = self.drive_one() {
                    self.drive_open_item(i).await?;
                }
            }
            OpenBrowser => {
                if let Some(i) = self.drive_one() {
                    self.drive_open_web(&i);
                }
            }
            OpenDesktop => {
                if let Some(i) = self.drive_one() {
                    self.drive_open_external(i).await?;
                }
            }
            OpenLocation => {
                if let Some(i) = self.drive_one() {
                    self.drive_open_location(i).await?;
                }
            }
            Favorite(id) => self.drive_favorite(id).await?,
            Shortcut => self.drive_shortcut().await?,
            Rename(name) => self.drive_rename(name.trim()).await?,
            MoveTo => self.drive_pick_begin("move".into()).await?,
            CopyTo => self.drive_pick_begin("copy".into()).await?,
            Download => self.drive_download().await?,
            Delete => self.drive_delete().await?,
            Restore => self.drive_bin(true).await?,
            DeleteForever => self.drive_bin(false).await?,
            CopyLink => self.drive_copy_link(),
            Versions => self.drive_versions().await?,
            VersionRestore(id) => self.drive_version_restore(&id).await?,
            Details => {
                if let Some(i) = self.drive_one() {
                    let fav = self.drive.lock().unwrap().s.favorites.contains(&i.id);
                    self.ui.drive_info("Details".into(), details_text(&i, fav));
                }
            }
            PreviewPage(which) => self.drive_preview_page(&which).await?,
            Sync => self.ui.drive_info("Sync".into(), onedrive::sync_help()),
            Share => self.drive_share_begin().await?,
            ShareSearch(q) => self.drive_share_search(q.trim()).await?,
            SharePick(id) => self.drive_share_pick(&id),
            ShareRole(r) => {
                if let Some(s) = self.drive.lock().unwrap().share.as_mut() {
                    s.role = parse_role(&r);
                }
                self.drive_share_push();
            }
            ShareSend(msg) => self.drive_share_send(&msg).await?,
            ShareLink(arg) => self.drive_share_link(&arg).await?,
            SharePermRole(id, role) => self.drive_perm_role(&id, parse_role(&role)).await?,
            SharePermRemove(id) => self.drive_perm_remove(&id).await?,
            PickBegin(purpose) => self.drive_pick_begin(purpose).await?,
            PickOpen(id) => self.drive_pick_open(&id).await?,
            PickCrumb(i) => self.drive_pick_crumb(i).await?,
            PickSelect(id) => self.drive_pick_select(&id),
            PickChoose => self.drive_pick_choose().await?,
            PickCancel | DialogClose => {
                let mut d = self.drive.lock().unwrap();
                d.picker = None;
                d.share = None;
                d.versions_of = None;
                d.pages = None;
            }
        }
        Ok(())
    }

    /// The commands that only change the view state (see `DriveCmd::is_instant`).
    pub(super) fn drive_instant(self: &Arc<Self>, cmd: DriveCmd) {
        match cmd {
            DriveCmd::Select(id) => self.drive_edit(|s| s.select_only(&id)),
            DriveCmd::Toggle(id) => self.drive_edit(|s| s.toggle(&id)),
            DriveCmd::SelectAll => self.drive_edit(State::select_all),
            DriveCmd::ClearSelection => self.drive_edit(State::clear_selection),
            DriveCmd::View(v) => {
                let grid = v == "grid";
                self.drive_edit(|s| s.grid = grid);
                if grid {
                    self.drive_thumbs();
                }
            }
            DriveCmd::Sort(k) => {
                if let Some(k) = state::parse_sort_key(&k) {
                    self.drive_edit(|s| s.set_sort(k));
                }
            }
            DriveCmd::Group(k) => {
                let key = state::parse_sort_key(&k);
                self.drive_edit(|s| s.group = key);
            }
            DriveCmd::Column(c) => self.drive_edit(|s| s.columns.toggle(&c)),
            DriveCmd::Type(t) => {
                self.drive_edit(|s| s.type_filter = state::TypeFilter::parse(&t));
            }
            DriveCmd::Filter(q) => self.drive_edit(|s| s.filter = q),
            _ => {}
        }
    }

    /// Changes the view state and pushes it; for the commands that need no network.
    fn drive_edit(&self, f: impl FnOnce(&mut State)) {
        f(&mut self.drive.lock().unwrap().s);
        self.drive_push();
    }

    /// Starts the background load (libraries, favorites, cards) unless it already started.
    fn drive_start_background(self: &Arc<Self>) {
        if !std::mem::replace(&mut self.drive.lock().unwrap().background_started, true) {
            let e = self.clone();
            tokio::spawn(async move { e.drive_background_load().await });
        }
    }

    /// Libraries (Quick access), favorites and the "For you" cards, which the list needs but
    /// should not wait for.
    async fn drive_background_load(self: Arc<Self>) {
        if self.drive_offline() {
            let libs = crate::demo::drive_libraries()
                .into_iter()
                .map(|(drive_id, name)| Library { drive_id, name })
                .collect();
            let favs = crate::demo::drive_favorites();
            {
                let mut d = self.drive.lock().unwrap();
                d.s.libraries = libs;
                d.s.favorites = favs;
            }
            self.drive_push();
            self.drive_load_cards().await;
            return;
        }
        let cards = self.clone();
        tokio::spawn(async move { cards.drive_load_cards().await });
        if let Ok(p) = browse::following(&self.client).await {
            self.drive.lock().unwrap().s.favorites = p.items.into_iter().map(|i| i.id).collect();
            self.drive_push();
        }
        let mut libs = Vec::new();
        if let Ok(sites) = browse::followed_sites(&self.client).await {
            for site in sites.into_iter().take(12) {
                if let Ok(drives) = browse::site_drives(&self.client, &site.id).await
                    && let Some(d) = drives.into_iter().next()
                {
                    libs.push(Library {
                        drive_id: d.id,
                        name: site.name,
                    });
                }
            }
        }
        self.drive.lock().unwrap().s.libraries = libs;
        self.drive_push();
    }

    async fn drive_go(self: &Arc<Self>, section: Section) -> anyhow::Result<()> {
        // Libraries come from the background load; wait for it on first use.
        let g = {
            let mut d = self.drive.lock().unwrap();
            // Photos and videos are for looking at: start them as tiles.
            if section == Section::Media {
                d.s.grid = true;
            }
            d.s.go(section)
        };
        self.drive_push();
        self.drive_load(g).await
    }

    /// Lists the current section and folder, and shows the result unless the user moved on.
    async fn drive_load(self: &Arc<Self>, generation: u64) -> anyhow::Result<()> {
        let (section, trail, libs) = {
            let d = self.drive.lock().unwrap();
            (
                d.s.section.clone(),
                d.s.trail.clone(),
                d.s.libraries.clone(),
            )
        };
        let result = self.drive_fetch(&section, &trail, &libs, None).await;
        let used = {
            let mut d = self.drive.lock().unwrap();
            match result {
                Ok((items, next)) => d.s.set_listing(generation, items, next),
                Err(err) => {
                    if generation == d.s.generation {
                        d.s.loading = false;
                        d.s.status = format!("Couldn't load this list: {err}");
                    }
                    false
                }
            }
        };
        self.drive_push();
        if used {
            self.drive_thumbs();
            if section == Section::Meetings {
                let e = self.clone();
                tokio::spawn(async move { e.drive_meetings_everywhere(generation).await });
            }
        }
        Ok(())
    }

    /// Meetings first lists the `Recordings` folder of My files (one request); this adds the
    /// channel libraries' recordings, which take a request per channel, and swaps the list.
    async fn drive_meetings_everywhere(self: Arc<Self>, generation: u64) {
        let items: Vec<DriveItem> = if self.drive_offline() {
            let mut v = crate::demo::drive_listing("meetings", "", now_ms());
            v.extend(crate::demo::drive_channel_recordings(now_ms()));
            v.sort_by_key(|i| std::cmp::Reverse(i.modified_ms));
            v
        } else {
            match onedrive::meeting_recordings(&self.client).await {
                Ok(r) => recording_items(r),
                Err(err) => {
                    tracing::warn!(%err, "meeting recordings failed");
                    return;
                }
            }
        };
        let used = self
            .drive
            .lock()
            .unwrap()
            .s
            .set_listing(generation, items, None);
        if used {
            self.drive_push();
        }
    }

    async fn drive_load_more(self: &Arc<Self>) -> anyhow::Result<()> {
        let (generation, trail, next, section) = {
            let d = self.drive.lock().unwrap();
            (
                d.s.generation,
                d.s.trail.clone(),
                d.s.next_link.clone(),
                d.s.section.clone(),
            )
        };
        if section == Section::Media {
            // The "next link" of Media is the offset of the next page.
            let Some(from) = next.and_then(|n| n.parse::<u32>().ok()) else {
                return Ok(());
            };
            let page = browse::media(&self.client, from, MEDIA_PAGE).await?;
            let next = page.more.then(|| (from + MEDIA_PAGE).to_string());
            self.drive
                .lock()
                .unwrap()
                .s
                .append_listing(generation, page.items, next);
            self.drive_push();
            self.drive_thumbs();
            return Ok(());
        }
        let (Some(next), Some(folder)) = (next, trail.last()) else {
            return Ok(());
        };
        let page =
            browse::children(&self.client, &folder.drive_id, &folder.item_id, Some(&next)).await?;
        self.drive
            .lock()
            .unwrap()
            .s
            .append_listing(generation, page.items, page.next_link);
        self.drive_push();
        self.drive_thumbs();
        Ok(())
    }

    /// Who shared files with the user: fetched by the People list, then kept for the person
    /// lists that open from it.
    async fn drive_people(&self) -> anyhow::Result<Vec<PersonFiles>> {
        let (listing, cached) = {
            let d = self.drive.lock().unwrap();
            (d.s.section == Section::People, d.people.clone())
        };
        let people = if self.drive_offline() {
            crate::demo::drive_people_files(now_ms())
        } else if listing || cached.is_empty() {
            insights::people(&self.client).await?
        } else {
            cached
        };
        self.drive.lock().unwrap().people = people.clone();
        Ok(people)
    }

    async fn drive_fetch(
        &self,
        section: &Section,
        trail: &[Crumb],
        libs: &[Library],
        next: Option<&str>,
    ) -> anyhow::Result<(Vec<DriveItem>, Option<String>)> {
        // The People lists work the same online and in the demo (`drive_people` knows which).
        match section {
            Section::People => return Ok((people_items(&self.drive_people().await?), None)),
            Section::Person(name) => {
                let files = self
                    .drive_people()
                    .await?
                    .into_iter()
                    .find(|p| &person_label(&p.person) == name)
                    .map(|p| p.files);
                return Ok((files.unwrap_or_default(), None));
            }
            _ => {}
        }
        if self.drive_offline() {
            let folder = trail.last().map_or("", |c| c.item_id.as_str());
            return Ok((
                crate::demo::drive_listing(&section.key(), folder, now_ms()),
                None,
            ));
        }
        let c = &self.client;
        Ok(match section {
            Section::Home => (browse::recent(c).await?, None),
            Section::Shared => (browse::shared_with_me(c).await?, None),
            Section::Favorites => (browse::following(c).await?.items, None),
            Section::Meetings => (onedrive::recordings(c).await?, None),
            Section::SharedByYou => (shared_by_you_items(insights::shared_by_me(c).await?), None),
            Section::Media => {
                let page = browse::media(c, 0, MEDIA_PAGE).await?;
                (page.items, page.more.then(|| MEDIA_PAGE.to_string()))
            }
            // Answered above, before the demo check.
            Section::People | Section::Person(_) => (Vec::new(), None),
            Section::Search(q) => (browse::search(c, q).await?, None),
            Section::Libraries => (libs.iter().map(library_item).collect(), None),
            Section::Bin => (
                bin::recycle_bin(c)
                    .await?
                    .into_iter()
                    .map(bin_item)
                    .collect(),
                None,
            ),
            Section::MyFiles | Section::Library(_) => {
                let f = trail
                    .last()
                    .ok_or_else(|| anyhow::anyhow!("no folder to list"))?;
                let page = browse::children(c, &f.drive_id, &f.item_id, next).await?;
                (page.items, page.next_link)
            }
        })
    }

    /// Fetches thumbnails in the background: the "For you" cards on Home, and the tiles of the
    /// grid view. Tiles whose listing has no thumbnail address (Search hits, so Media) ask for
    /// it per item.
    fn drive_thumbs(self: &Arc<Self>) {
        if self.drive_offline() {
            return;
        }
        let (generation, wanted): (u64, Vec<(String, ThumbSource)>) = {
            let d = self.drive.lock().unwrap();
            let mut wanted = Vec::new();
            if d.s.section == Section::Home {
                wanted.extend(
                    d.s.cards
                        .iter()
                        .filter(|c| !d.s.thumbs.contains_key(&c.id))
                        .map(|c| (c.id.clone(), ThumbSource::Item(ItemRef::from(&c.item)))),
                );
            }
            if d.s.grid {
                let per_item = d.s.section == Section::Media;
                wanted.extend(
                    d.s.items
                        .iter()
                        .filter(|i| !d.s.thumbs.contains_key(&i.id))
                        .filter_map(|i| {
                            let src = match &i.thumbnail_url {
                                Some(u) => ThumbSource::Url(u.clone()),
                                None if per_item && !i.drive_id.is_empty() => {
                                    ThumbSource::Item(ItemRef::from(i))
                                }
                                None => return None,
                            };
                            Some((i.id.clone(), src))
                        })
                        .take(MAX_THUMBS),
                );
            }
            (d.s.generation, wanted)
        };
        if wanted.is_empty() {
            return;
        }
        let e = self.clone();
        tokio::spawn(async move {
            for batch in wanted.chunks(4) {
                let mut set = tokio::task::JoinSet::new();
                for (id, src) in batch.iter().cloned() {
                    let client = e.client.clone();
                    set.spawn(async move {
                        let url = match src {
                            ThumbSource::Url(u) => u,
                            ThumbSource::Item(r) => {
                                let t = browse::thumbnails(&client, &r).await.ok()?;
                                t.medium.or(t.small).or(t.large)?
                            }
                        };
                        let bytes = client.bytes(&url, &[], 3 * 1024 * 1024).await.ok()?;
                        let px = tokio::task::spawn_blocking(move || {
                            super::util::decode_image(&bytes, 240, 160)
                        })
                        .await
                        .ok()??;
                        Some((id, px))
                    });
                }
                while let Some(r) = set.join_next().await {
                    if let Ok(Some((id, px))) = r {
                        let mut d = e.drive.lock().unwrap();
                        if d.s.generation != generation {
                            return;
                        }
                        d.s.thumbs.insert(id, px);
                    }
                }
                e.drive_push();
            }
        });
    }

    /// Loads Home's "For you" cards and fetches their thumbnails.
    async fn drive_load_cards(self: Arc<Self>) {
        let found = if self.drive_offline() {
            crate::demo::drive_for_you(now_ms())
        } else {
            match insights::for_you(&self.client, MAX_CARDS).await {
                Ok(c) => c,
                Err(err) => {
                    tracing::warn!(%err, "for you failed");
                    return;
                }
            }
        };
        let cards: Vec<Card> = found
            .into_iter()
            .map(|c| {
                let item = c.insight.to_drive_item();
                Card {
                    id: format!("card:{}/{}", item.drive_id, item.id),
                    item,
                    reason: c.reason,
                }
            })
            .collect();
        self.drive.lock().unwrap().s.cards = cards;
        self.drive_push();
        self.drive_thumbs();
    }

    /// A "For you" card was clicked: open its file like a row of the list.
    async fn drive_card_open(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let item = {
            let d = self.drive.lock().unwrap();
            d.s.cards
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.item.clone())
        };
        match item {
            Some(item) => self.drive_open_item(item).await,
            None => Ok(()),
        }
    }

    async fn drive_open_row(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let item = self.drive.lock().unwrap().s.item(id).cloned();
        let Some(item) = item else {
            return Ok(());
        };
        let section = self.drive.lock().unwrap().s.section.clone();
        match section {
            // Deleted items cannot be opened; Restore is the way back.
            Section::Bin => return Ok(()),
            Section::People => return self.drive_go(Section::Person(item.name)).await,
            _ => {}
        }
        self.drive_open_item(item).await
    }

    /// A folder opens in the list, a file in kelma's viewer or the browser.
    async fn drive_open_item(self: &Arc<Self>, item: DriveItem) -> anyhow::Result<()> {
        if item.is_folder {
            let g = {
                let mut d = self.drive.lock().unwrap();
                if d.s.section.is_browsable() {
                    d.s.open_folder(&item)
                } else {
                    // A folder from Home, Shared, Libraries ...: browse it as its own root.
                    d.s.open_foreign_folder(&item)
                }
            };
            self.drive_push();
            return self.drive_load(g).await;
        }
        let preview = with_files_pref(
            super::settings::files_prefs().open_office_in,
            Preview::of(&item),
            FileKind::of(&item),
        );
        match preview {
            Preview::Image | Preview::Text | Preview::Markdown
                if item.size <= MAX_PREVIEW_BYTES && !self.drive_offline() =>
            {
                self.drive_status(format!("Opening {}...", item.name));
                let bytes = ops::download(&self.client, &item).await?;
                let data = tokio::task::spawn_blocking(move || preview_of(&item, &bytes)).await?;
                self.drive_status("");
                self.ui.drive_preview(data);
            }
            Preview::Image | Preview::Text | Preview::Markdown if self.drive_offline() => {
                let text = crate::demo::drive_file_text(&item.name);
                self.ui.drive_preview(PreviewData {
                    title: item.name.clone(),
                    kind: if Preview::of(&item) == Preview::Markdown {
                        "markdown"
                    } else {
                        "text"
                    },
                    image: None,
                    text,
                    page: 0,
                    pages: 0,
                });
            }
            Preview::Pages if item.size <= MAX_OPEN_BYTES => {
                if !self.drive_open_pages(&item).await {
                    self.drive_open_browser_preview(&item).await?;
                }
            }
            Preview::Player => self.drive_open_external(item).await?,
            _ => self.drive_open_browser_preview(&item).await?,
        }
        Ok(())
    }

    /// Shows a PDF, or an Office file converted to PDF by Graph (a read-only request), page by
    /// page inside kelma. False when that is not possible (password, a type Graph cannot convert,
    /// a damaged file): the caller falls back to the browser.
    async fn drive_open_pages(self: &Arc<Self>, item: &DriveItem) -> bool {
        self.drive_status(format!("Opening {}...", item.name));
        let bytes: Vec<u8> = if self.drive_offline() {
            crate::demo::drive_pdf(&[(612, 792), (612, 792), (612, 792)])
        } else {
            let fetched = if FileKind::of(item) == FileKind::Pdf {
                ops::download(&self.client, item)
                    .await
                    .map_err(|e| e.to_string())
            } else {
                preview::office_to_pdf(&self.client, &item.drive_id, &item.id, &item.name)
                    .await
                    .map_err(|e| e.to_string())
            };
            match fetched {
                Ok(b) => b,
                Err(err) => {
                    tracing::info!(%err, "no page preview, using the browser");
                    self.drive_status("");
                    return false;
                }
            }
        };
        let doc = match tokio::task::spawn_blocking(move || preview::Document::open(bytes)).await {
            Ok(Ok(doc)) if doc.page_count() > 0 => Arc::new(doc),
            _ => {
                self.drive_status("");
                return false;
            }
        };
        self.drive.lock().unwrap().pages = Some(OpenPages {
            doc,
            name: item.name.clone(),
            page: 0,
        });
        self.drive_status("");
        self.drive_show_page().await
    }

    /// Renders the current page of the open document and shows it.
    async fn drive_show_page(self: &Arc<Self>) -> bool {
        let Some((doc, name, page)) = self
            .drive
            .lock()
            .unwrap()
            .pages
            .as_ref()
            .map(|p| (p.doc.clone(), p.name.clone(), p.page))
        else {
            return false;
        };
        let pages = doc.page_count();
        let image = tokio::task::spawn_blocking(move || {
            let r = doc.render_page(page, PAGE_WIDTH).ok()?;
            Some(Pixels::clone_from_slice(&r.pixels, r.width, r.height))
        })
        .await
        .ok()
        .flatten();
        let Some(image) = image else {
            self.drive_status("Couldn't draw this page");
            return false;
        };
        self.ui.drive_preview(PreviewData {
            title: name,
            kind: "pages",
            image: Some(image),
            text: String::new(),
            page: page + 1,
            pages,
        });
        true
    }

    /// Previous or next page of the open document.
    async fn drive_preview_page(self: &Arc<Self>, which: &str) -> anyhow::Result<()> {
        {
            let mut d = self.drive.lock().unwrap();
            let Some(p) = d.pages.as_mut() else {
                return Ok(());
            };
            let last = p.doc.page_count().saturating_sub(1);
            p.page = match which {
                "next" => (p.page + 1).min(last),
                "prev" => p.page.saturating_sub(1),
                _ => p.page,
            };
        }
        self.drive_show_page().await;
        Ok(())
    }

    /// Office files and PDFs: the preview URL in the browser, else the web address.
    async fn drive_open_browser_preview(&self, item: &DriveItem) -> anyhow::Result<()> {
        if self.drive_offline() {
            self.drive_open_web(item);
            return Ok(());
        }
        match ops::preview_url(&self.client, &ItemRef::from(item)).await {
            Ok(url) if url.starts_with("https://") => {
                let _ = crate::external::open(&url);
                Ok(())
            }
            _ => {
                self.drive_open_web(item);
                Ok(())
            }
        }
    }

    fn drive_open_web(&self, item: &DriveItem) {
        if item.web_url.starts_with("https://") {
            let _ = crate::external::open(&item.web_url);
        } else {
            self.drive_status("This item has no web address");
        }
    }

    /// Downloads to the cache and hands the file to the desktop (`xdg-open`).
    async fn drive_open_external(self: &Arc<Self>, item: DriveItem) -> anyhow::Result<()> {
        if item.is_folder || self.drive_offline() {
            self.drive_open_web(&item);
            return Ok(());
        }
        if item.size > MAX_OPEN_BYTES {
            self.drive_open_web(&item);
            return Ok(());
        }
        self.drive_status(format!("Downloading {}...", item.name));
        let bytes = ops::download(&self.client, &item).await?;
        let dir = session::Paths::new()?.cache_dir.join("drive-open");
        let name = sanitize_file_name(&item.name);
        let path = tokio::task::spawn_blocking(move || -> anyhow::Result<std::path::PathBuf> {
            std::fs::create_dir_all(&dir)?;
            let path = dir.join(name);
            std::fs::write(&path, bytes)?;
            Ok(path)
        })
        .await??;
        self.drive_status("");
        let _ = crate::external::open(path);
        Ok(())
    }

    /// Goes to the folder that holds the item. Own files are browsed natively; items of other
    /// drives open the folder in the browser.
    async fn drive_open_location(self: &Arc<Self>, item: DriveItem) -> anyhow::Result<()> {
        if self.drive_offline() {
            return self.drive_go(Section::MyFiles).await;
        }
        let mine = self.drive_my_drive_id().await;
        if item.parent_path.is_empty() || mine.as_deref() != Some(item.drive_id.as_str()) {
            if let Some((parent, _)) = item.web_url.rsplit_once('/') {
                let _ = crate::external::open(parent);
            }
            return Ok(());
        }
        let g = self.drive.lock().unwrap().s.go(Section::MyFiles);
        for path in path_prefixes(&item.parent_path) {
            let folder = browse::item_at(&self.client, &path).await?;
            self.drive.lock().unwrap().s.trail.push(Crumb {
                name: folder.name,
                drive_id: folder.drive_id,
                item_id: folder.id,
            });
        }
        self.drive_push();
        self.drive_load(g).await
    }

    async fn drive_my_drive_id(&self) -> Option<String> {
        if let Some(id) = self.drive.lock().unwrap().my_drive_id.clone() {
            return Some(id);
        }
        let id = browse::my_drive(&self.client).await.ok()?.id;
        self.drive.lock().unwrap().my_drive_id = Some(id.clone());
        Some(id)
    }

    /// Where new files and folders go: the open folder, else the root of My files.
    fn drive_target(&self) -> ItemRef {
        let d = self.drive.lock().unwrap();
        match d.s.folder() {
            Some(c) => ItemRef::new(&c.drive_id, &c.item_id),
            None => ItemRef::my_root(),
        }
    }

    /// Reloads the open list after a change and selects `select` when it is there.
    async fn drive_refresh_select(self: &Arc<Self>, select: Option<String>) -> anyhow::Result<()> {
        let g = {
            let mut d = self.drive.lock().unwrap();
            d.s.generation += 1;
            d.s.generation
        };
        self.drive_load(g).await?;
        if let Some(id) = select {
            self.drive_edit(|s| s.select_only(&id));
        }
        Ok(())
    }

    fn demo_note(&self) -> bool {
        if self.drive_offline() {
            self.drive_status("Demo mode: nothing is saved");
            return true;
        }
        false
    }

    async fn drive_new_folder(self: &Arc<Self>, name: &str) -> anyhow::Result<()> {
        if name.is_empty() {
            return Ok(());
        }
        if self.drive_offline() {
            let item = crate::demo::drive_new_folder(name);
            let id = item.id.clone();
            self.drive.lock().unwrap().s.items.push(item);
            self.drive_edit(|s| s.select_only(&id));
            return Ok(());
        }
        let parent = self.drive_target();
        let made = ops::create_folder(&self.client, &parent, name).await?;
        self.drive_refresh_select(Some(made.id)).await
    }

    async fn drive_new_doc(self: &Arc<Self>, kind: onedrive::DocKind) -> anyhow::Result<()> {
        if self.demo_note() {
            return Ok(());
        }
        let parent = self.drive_target();
        self.drive_status(format!("Creating {}...", kind.default_name()));
        let made = ops::create_office_doc(&self.client, &parent, kind, None).await?;
        self.drive_refresh_select(Some(made.id)).await?;
        self.drive_status("");
        Ok(())
    }

    /// New > Link: a `.url` file that opens `address`. The argument is `name, a newline, then the address`.
    async fn drive_new_link(self: &Arc<Self>, arg: &str) -> anyhow::Result<()> {
        let (name, url) = arg.split_once('\n').unwrap_or((arg, ""));
        let (name, url) = (name.trim(), url.trim());
        if name.is_empty() || !(url.starts_with("https://") || url.starts_with("http://")) {
            self.drive_status("A link needs a name and a web address that starts with https://");
            return Ok(());
        }
        if self.demo_note() {
            return Ok(());
        }
        let parent = self.drive_target();
        let what = NewItem::Link {
            name: name.to_owned(),
            url: url.to_owned(),
        };
        match onedrive::create::create(&self.client, &parent, &what).await? {
            onedrive::Created::Item(made) => self.drive_refresh_select(Some(made.id)).await,
            onedrive::Created::Notebook(_) => Ok(()),
        }
    }

    /// New > OneNote notebook: OneNote keeps notebooks in its own place, not in the open folder,
    /// so the notebook opens in the browser once it exists.
    async fn drive_new_notebook(self: &Arc<Self>, name: &str) -> anyhow::Result<()> {
        if name.is_empty() || self.demo_note() {
            return Ok(());
        }
        let what = NewItem::Notebook {
            name: name.to_owned(),
        };
        self.drive_status(format!("Creating notebook {name}..."));
        let made = onedrive::create::create(&self.client, &self.drive_target(), &what).await?;
        if let onedrive::Created::Notebook(nb) = made {
            if nb.web_url.starts_with("https://") {
                let _ = crate::external::open(&nb.web_url);
            }
            self.drive_status(format!("Created notebook {}", nb.name));
        }
        Ok(())
    }

    /// New > Forms survey: Forms has no API for this client, so it opens in the browser.
    fn drive_new_forms(&self) {
        if self.drive_offline() {
            self.drive_status("Demo mode: Microsoft Forms would open in the browser");
            return;
        }
        let _ = crate::external::open(onedrive::FORMS_NEW_URL);
    }

    async fn drive_upload_pick(self: &Arc<Self>, folder: bool) -> anyhow::Result<()> {
        // Demo mode never shows a file dialog (tests and screenshots have nobody to answer it).
        if self.demo_note() {
            return Ok(());
        }
        let paths: Vec<std::path::PathBuf> = if folder {
            rfd::AsyncFileDialog::new()
                .pick_folder()
                .await
                .into_iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        } else {
            rfd::AsyncFileDialog::new()
                .pick_files()
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        };
        self.drive_upload_paths(paths).await
    }

    /// Uploads files and folders (the picker, or dropped on the window).
    async fn drive_upload_paths(
        self: &Arc<Self>,
        paths: Vec<std::path::PathBuf>,
    ) -> anyhow::Result<()> {
        if paths.is_empty() || self.demo_note() {
            return Ok(());
        }
        let parent = self.drive_target();
        let (mut files, mut failed) = (0usize, Vec::new());
        for path in paths {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.drive_status(format!("Uploading {name}..."));
            let r: anyhow::Result<usize> = async {
                if path.is_dir() {
                    let report = ops::upload_folder(&self.client, &parent, &path).await?;
                    Ok(report.files)
                } else {
                    let p = path.clone();
                    let data = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
                        let meta = std::fs::metadata(&p)?;
                        anyhow::ensure!(
                            meta.len() <= MAX_UPLOAD_BYTES,
                            "{} is too big to upload here (250 MB limit)",
                            p.display()
                        );
                        Ok(std::fs::read(&p)?)
                    })
                    .await??;
                    ops::upload_file(&self.client, &parent, &name, data).await?;
                    Ok(1)
                }
            }
            .await;
            match r {
                Ok(n) => files += n,
                Err(err) => failed.push(format!("{name}: {err}")),
            }
        }
        self.drive_refresh_select(None).await?;
        let mut status = format!("Uploaded {files} file{}", if files == 1 { "" } else { "s" });
        if !failed.is_empty() {
            status.push_str(&format!(". Failed: {}", failed.join("; ")));
        }
        self.drive_status(status);
        Ok(())
    }

    async fn drive_favorite(self: &Arc<Self>, id: String) -> anyhow::Result<()> {
        let items: Vec<DriveItem> = if id.is_empty() {
            self.drive_selection()
        } else {
            self.drive
                .lock()
                .unwrap()
                .s
                .item(&id)
                .cloned()
                .into_iter()
                .collect()
        };
        if items.is_empty() {
            return Ok(());
        }
        let make_favorite = {
            let d = self.drive.lock().unwrap();
            !items.iter().all(|i| d.s.favorites.contains(&i.id))
        };
        for i in &items {
            {
                let mut d = self.drive.lock().unwrap();
                if make_favorite {
                    d.s.favorites.insert(i.id.clone());
                } else {
                    d.s.favorites.remove(&i.id);
                }
            }
            if self.drive_offline() {
                continue;
            }
            let r = ItemRef::from(i);
            let done = if make_favorite {
                ops::follow(&self.client, &r).await
            } else {
                ops::unfollow(&self.client, &r).await
            };
            if let Err(err) = done {
                // Undo the optimistic star.
                let mut d = self.drive.lock().unwrap();
                if make_favorite {
                    d.s.favorites.remove(&i.id);
                } else {
                    d.s.favorites.insert(i.id.clone());
                }
                drop(d);
                self.drive_status(format!("Couldn't change the favorite: {err}"));
                return Ok(());
            }
        }
        {
            // Un-favoriting inside Favorites removes the row.
            let mut d = self.drive.lock().unwrap();
            if !make_favorite && d.s.section == Section::Favorites {
                let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
                d.s.items.retain(|i| !ids.contains(&i.id));
                d.s.selected.retain(|s| !ids.contains(s));
            }
        }
        self.drive_push();
        Ok(())
    }

    /// Add shortcut: Graph refuses every body we tried (docs/protocol.md), so the failure is
    /// expected and shown as a plain sentence.
    async fn drive_shortcut(self: &Arc<Self>) -> anyhow::Result<()> {
        let sel = self.drive_selection();
        let Some(target) = sel.first() else {
            self.drive_status("Select a folder to add as a shortcut");
            return Ok(());
        };
        if self.drive_offline() {
            self.drive_status("Demo mode: nothing is saved");
            return Ok(());
        }
        match ops::add_shortcut(&self.client, target, &ItemRef::my_root(), None).await {
            Ok(_) => {
                self.drive_status(format!("Added a shortcut to {} in My files", target.name));
            }
            Err(err) => {
                tracing::warn!(%err, "add shortcut failed");
                self.drive_status(
                    "OneDrive did not accept the shortcut, so none was added. \
                     Open the item in the browser to add it there.",
                );
            }
        }
        Ok(())
    }

    async fn drive_rename(self: &Arc<Self>, name: &str) -> anyhow::Result<()> {
        let Some(item) = self.drive_one() else {
            return Ok(());
        };
        if name.is_empty() || name == item.name {
            return Ok(());
        }
        if self.drive_offline() {
            self.drive_edit(|s| {
                if let Some(i) = s.items.iter_mut().find(|i| i.id == item.id) {
                    i.name = name.to_owned();
                }
            });
            return Ok(());
        }
        let renamed = ops::rename(&self.client, &ItemRef::own(&item), name).await?;
        self.drive_edit(|s| {
            if let Some(i) = s.items.iter_mut().find(|i| i.id == item.id) {
                i.name = renamed.name.clone();
            }
        });
        Ok(())
    }

    async fn drive_delete(self: &Arc<Self>) -> anyhow::Result<()> {
        let sel = self.drive_selection();
        if sel.is_empty() {
            return Ok(());
        }
        let mut gone = Vec::new();
        for i in &sel {
            if self.drive_offline() || ops::delete(&self.client, &ItemRef::own(i)).await.is_ok() {
                gone.push(i.id.clone());
            }
        }
        let failed = sel.len() - gone.len();
        {
            let mut d = self.drive.lock().unwrap();
            d.s.items.retain(|i| !gone.contains(&i.id));
            d.s.selected.retain(|s| !gone.contains(s));
            d.s.status = match (gone.len(), failed) {
                (n, 0) => format!("Deleted {n} item{} (in the recycle bin)", plural(n)),
                (n, f) => format!("Deleted {n}, couldn't delete {f}"),
            };
        }
        self.drive_push();
        Ok(())
    }

    /// Recycle bin: Restore (`true`) or Delete forever (`false`) for the selection.
    async fn drive_bin(self: &Arc<Self>, restore: bool) -> anyhow::Result<()> {
        let sel = self.drive_selection();
        if sel.is_empty() {
            return Ok(());
        }
        let mut gone = Vec::new();
        for i in &sel {
            let ok = self.drive_offline()
                || if restore {
                    bin::restore(&self.client, &i.id).await.is_ok()
                } else {
                    bin::delete_forever(&self.client, &i.id).await.is_ok()
                };
            if ok {
                gone.push(i.id.clone());
            }
        }
        {
            let mut d = self.drive.lock().unwrap();
            d.s.items.retain(|i| !gone.contains(&i.id));
            d.s.selected.retain(|s| !gone.contains(s));
            let n = gone.len();
            d.s.status = if n == sel.len() {
                format!(
                    "{} {n} item{}",
                    if restore {
                        "Restored"
                    } else {
                        "Deleted forever"
                    },
                    plural(n)
                )
            } else {
                format!("Only {n} of {} items could be changed", sel.len())
            };
        }
        self.drive_push();
        Ok(())
    }

    fn drive_copy_link(&self) {
        let sel = self.drive_selection();
        let Some(i) = sel.first().filter(|i| !i.web_url.is_empty()) else {
            self.drive_status("Select an item with a web address first");
            return;
        };
        // The item's own address works for everyone who already has access, which is what
        // Teams' "Copy link" gives. A new sharing link is a decision for the Share dialog.
        match arboard::Clipboard::new().and_then(|mut c| c.set_text(i.web_url.clone())) {
            Ok(()) => self.drive_status("Link copied"),
            Err(err) => self.drive_status(format!("Couldn't copy the link: {err}")),
        }
    }

    async fn drive_download(self: &Arc<Self>) -> anyhow::Result<()> {
        let sel = self.drive_selection();
        if sel.is_empty() {
            return Ok(());
        }
        if self.demo_note() {
            return Ok(());
        }
        let file_name = |i: &DriveItem| {
            if i.is_folder {
                format!("{}.zip", i.name)
            } else {
                i.name.clone()
            }
        };
        let targets: Vec<(DriveItem, std::path::PathBuf)> = if let [one] = sel.as_slice() {
            let Some(f) = rfd::AsyncFileDialog::new()
                .set_file_name(sanitize_file_name(&file_name(one)))
                .save_file()
                .await
            else {
                return Ok(());
            };
            vec![(one.clone(), f.path().to_path_buf())]
        } else {
            let Some(dir) = rfd::AsyncFileDialog::new().pick_folder().await else {
                return Ok(());
            };
            sel.iter()
                .map(|i| {
                    (
                        i.clone(),
                        dir.path().join(sanitize_file_name(&file_name(i))),
                    )
                })
                .collect()
        };
        let mut done = 0;
        for (item, path) in targets {
            self.drive_status(format!("Downloading {}...", item.name));
            let bytes = ops::download(&self.client, &item).await?;
            tokio::task::spawn_blocking(move || std::fs::write(path, bytes)).await??;
            done += 1;
        }
        self.drive_status(format!("Downloaded {done} item{}", plural(done)));
        Ok(())
    }

    async fn drive_versions(self: &Arc<Self>) -> anyhow::Result<()> {
        let Some(item) = self.drive_one() else {
            return Ok(());
        };
        let list = if self.drive_offline() {
            crate::demo::drive_versions(now_ms())
        } else {
            ops::versions(&self.client, &ItemRef::from(&item)).await?
        };
        self.drive.lock().unwrap().versions_of = Some(item.clone());
        self.ui
            .drive_versions(item.name, version_data(&list, now_ms()));
        Ok(())
    }

    async fn drive_version_restore(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let item = self.drive.lock().unwrap().versions_of.clone();
        let Some(item) = item else {
            return Ok(());
        };
        if self.demo_note() {
            return Ok(());
        }
        ops::restore_version(&self.client, &ItemRef::from(&item), id).await?;
        self.drive_status(format!("Restored an earlier version of {}", item.name));
        Ok(())
    }

    // -- share --------------------------------------------------------------------------------

    async fn drive_share_begin(self: &Arc<Self>) -> anyhow::Result<()> {
        let Some(item) = self.drive_one() else {
            return Ok(());
        };
        self.drive.lock().unwrap().share = Some(Share {
            item: item.clone(),
            role: Role::Write,
            results: Vec::new(),
            picked: Vec::new(),
            perms: Vec::new(),
            status: SHARE_HELP.into(),
        });
        self.drive_share_push();
        self.drive_share_reload_perms().await;
        Ok(())
    }

    fn drive_share_push(&self) {
        let now = now_ms();
        let data = {
            let d = self.drive.lock().unwrap();
            let Some(s) = d.share.as_ref() else {
                return;
            };
            ShareData {
                name: s.item.name.clone(),
                results: s
                    .results
                    .iter()
                    .map(|p| PersonData {
                        picked: s.picked.iter().any(|x| x.id == p.id),
                        ..p.clone()
                    })
                    .collect(),
                picked: s.picked.clone(),
                perms: s.perms.iter().map(|p| perm_data(p, now)).collect(),
                role: role_str(s.role),
                status: s.status.clone(),
            }
        };
        self.ui.drive_share(data);
    }

    fn drive_share_status(&self, text: impl Into<String>) {
        if let Some(s) = self.drive.lock().unwrap().share.as_mut() {
            s.status = text.into();
        }
        self.drive_share_push();
    }

    async fn drive_share_reload_perms(&self) {
        let item = {
            let d = self.drive.lock().unwrap();
            d.share.as_ref().map(|s| s.item.clone())
        };
        let Some(item) = item else {
            return;
        };
        let perms = if self.drive_offline() {
            crate::demo::drive_permissions()
        } else {
            match share::permissions(&self.client, &ItemRef::from(&item)).await {
                Ok(p) => p,
                Err(err) => {
                    self.drive_share_status(format!("Couldn't list who has access: {err}"));
                    return;
                }
            }
        };
        if let Some(s) = self.drive.lock().unwrap().share.as_mut() {
            s.perms = perms;
        }
        self.drive_share_push();
    }

    async fn drive_share_search(self: &Arc<Self>, q: &str) -> anyhow::Result<()> {
        if q.chars().count() < 2 {
            if let Some(s) = self.drive.lock().unwrap().share.as_mut() {
                s.results.clear();
            }
            self.drive_share_push();
            return Ok(());
        }
        let people = if self.drive_offline() {
            crate::demo::drive_people(q)
        } else {
            self.client.search_people(&self.me, q).await?
        };
        let results = people
            .into_iter()
            .map(|p| PersonData {
                id: p.mri,
                name: p.display_name,
                detail: p.job_title.or(p.email.clone()).unwrap_or_default(),
                picked: false,
                email: p.email,
            })
            .collect();
        if let Some(s) = self.drive.lock().unwrap().share.as_mut() {
            s.results = results;
        }
        self.drive_share_push();
        Ok(())
    }

    fn drive_share_pick(&self, id: &str) {
        if let Some(s) = self.drive.lock().unwrap().share.as_mut() {
            match s.picked.iter().position(|p| p.id == id) {
                Some(i) => {
                    s.picked.remove(i);
                }
                None => {
                    if let Some(p) = s.results.iter().find(|p| p.id == id) {
                        s.picked.push(p.clone());
                    }
                }
            }
        }
        self.drive_share_push();
    }

    async fn drive_share_send(self: &Arc<Self>, message: &str) -> anyhow::Result<()> {
        let (item, role, people) = {
            let d = self.drive.lock().unwrap();
            let Some(s) = d.share.as_ref() else {
                return Ok(());
            };
            (s.item.clone(), s.role, s.picked.clone())
        };
        let recipients: Vec<Recipient> = people.iter().filter_map(recipient_of).collect();
        if recipients.is_empty() {
            self.drive_share_status("Pick at least one person first");
            return Ok(());
        }
        if self.drive_offline() {
            self.drive_share_status("Demo mode: nothing is shared");
            return Ok(());
        }
        self.drive_share_status("Sharing...");
        match share::invite(
            &self.client,
            &ItemRef::from(&item),
            &recipients,
            role,
            message,
            true,
        )
        .await
        {
            Ok(_) => {
                if let Some(s) = self.drive.lock().unwrap().share.as_mut() {
                    s.picked.clear();
                    s.results.clear();
                    s.status = format!(
                        "Shared with {} {}",
                        recipients.len(),
                        if recipients.len() == 1 {
                            "person"
                        } else {
                            "people"
                        }
                    );
                }
                self.drive_share_push();
                self.drive_share_reload_perms().await;
            }
            Err(err) => self.drive_share_status(format!("Couldn't share: {err}")),
        }
        Ok(())
    }

    async fn drive_share_link(self: &Arc<Self>, arg: &str) -> anyhow::Result<()> {
        let Some(settings) = LinkSettings::parse(arg) else {
            return Ok(());
        };
        let item = self
            .drive
            .lock()
            .unwrap()
            .share
            .as_ref()
            .map(|s| s.item.clone());
        let Some(item) = item else {
            return Ok(());
        };
        if self.drive_offline() {
            self.drive_share_status("Demo mode: no link was created");
            return Ok(());
        }
        let expiry = settings.expiry_days.map(|d| link_expiry(d, now_ms()));
        let made = share::create_link(
            &self.client,
            &ItemRef::from(&item),
            settings.scope,
            settings.kind,
            expiry.as_deref(),
            settings.block_download,
        )
        .await;
        match made {
            Ok(p) => {
                let copied = p
                    .link_url
                    .as_deref()
                    .map(|u| {
                        arboard::Clipboard::new()
                            .and_then(|mut c| c.set_text(u))
                            .is_ok()
                    })
                    .unwrap_or(false);
                self.drive_share_status(if copied {
                    "Link created and copied"
                } else {
                    "Link created"
                });
                self.drive_share_reload_perms().await;
            }
            Err(err) => self.drive_share_status(format!("Couldn't create the link: {err}")),
        }
        Ok(())
    }

    async fn drive_perm_role(self: &Arc<Self>, id: &str, role: Role) -> anyhow::Result<()> {
        let item = self
            .drive
            .lock()
            .unwrap()
            .share
            .as_ref()
            .map(|s| s.item.clone());
        let Some(item) = item else {
            return Ok(());
        };
        if self.drive_offline() {
            self.drive_share_status("Demo mode: nothing is changed");
            return Ok(());
        }
        match share::update_permission(&self.client, &ItemRef::from(&item), id, role).await {
            Ok(()) => self.drive_share_reload_perms().await,
            Err(err) => self.drive_share_status(format!("Couldn't change access: {err}")),
        }
        Ok(())
    }

    async fn drive_perm_remove(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let item = self
            .drive
            .lock()
            .unwrap()
            .share
            .as_ref()
            .map(|s| s.item.clone());
        let Some(item) = item else {
            return Ok(());
        };
        if self.drive_offline() {
            self.drive_share_status("Demo mode: nothing is changed");
            return Ok(());
        }
        match share::remove_permission(&self.client, &ItemRef::from(&item), id).await {
            Ok(()) => self.drive_share_reload_perms().await,
            Err(err) => self.drive_share_status(format!("Couldn't remove access: {err}")),
        }
        Ok(())
    }

    // -- the reusable file picker -------------------------------------------------------------

    /// Opens the picker for `purpose`. `move` and `copy` act on the selection; other purposes
    /// (for example `attach`) are answered through [`Engine::drive_picked`].
    async fn drive_pick_begin(self: &Arc<Self>, purpose: String) -> anyhow::Result<()> {
        let folders_only = matches!(purpose.as_str(), "move" | "copy");
        if folders_only && self.drive_selection().is_empty() {
            self.drive_status("Select something first");
            return Ok(());
        }
        // The chat can open the picker before the OneDrive view ever did.
        self.drive_start_background();
        {
            let mut d = self.drive.lock().unwrap();
            let libs = d.s.libraries.clone();
            let mut items = vec![my_files_item()];
            if purpose == "attach" {
                items.extend(virtual_items());
            }
            items.extend(libs.iter().map(library_item));
            d.picker = Some(Picker {
                purpose,
                folders_only,
                trail: Vec::new(),
                items,
                selected: None,
                loading: false,
                generation: 0,
            });
        }
        self.drive_picker_push();
        Ok(())
    }

    fn drive_picker_push(&self) {
        let data = {
            let d = self.drive.lock().unwrap();
            d.picker.as_ref().map(|p| picker_data(p, &d.s.libraries))
        };
        if let Some(data) = data {
            self.ui.drive_picker(data);
        }
    }

    async fn drive_pick_open(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let (folder, generation, folders_only) = {
            let mut d = self.drive.lock().unwrap();
            let Some(p) = d.picker.as_mut() else {
                return Ok(());
            };
            let Some(item) = p
                .items
                .iter()
                .find(|i| i.id == id && !i.id.is_empty())
                .cloned()
            else {
                return Ok(());
            };
            if !item.is_folder {
                p.selected = Some(item.id);
                drop(d);
                self.drive_picker_push();
                return Ok(());
            }
            p.trail.push(Crumb {
                name: item.name.clone(),
                drive_id: item.drive_id.clone(),
                item_id: item.id.clone(),
            });
            p.items.clear();
            p.selected = None;
            p.loading = true;
            p.generation += 1;
            (
                p.trail.last().cloned().unwrap(),
                p.generation,
                p.folders_only,
            )
        };
        self.drive_picker_push();
        self.drive_pick_list(folder, generation, folders_only).await
    }

    async fn drive_pick_crumb(self: &Arc<Self>, i: usize) -> anyhow::Result<()> {
        // Crumb 0 is the OneDrive root; crumb n is trail[n - 1].
        let (folder, generation, folders_only) = {
            let mut d = self.drive.lock().unwrap();
            let libs = d.s.libraries.clone();
            let Some(p) = d.picker.as_mut() else {
                return Ok(());
            };
            p.trail.truncate(i);
            p.selected = None;
            p.generation += 1;
            match p.trail.last().cloned() {
                None => {
                    let mut items = vec![my_files_item()];
                    items.extend(libs.iter().map(library_item));
                    p.items = items;
                    p.loading = false;
                    drop(d);
                    self.drive_picker_push();
                    return Ok(());
                }
                Some(f) => {
                    p.loading = true;
                    (f, p.generation, p.folders_only)
                }
            }
        };
        self.drive_picker_push();
        self.drive_pick_list(folder, generation, folders_only).await
    }

    async fn drive_pick_list(
        self: &Arc<Self>,
        folder: Crumb,
        generation: u64,
        folders_only: bool,
    ) -> anyhow::Result<()> {
        let items = if folder.drive_id == VIRTUAL {
            let list = match folder.item_id.as_str() {
                "recent" => browse::recent(&self.client).await,
                _ => browse::shared_with_me(&self.client).await,
            };
            match list {
                Ok(items) => items,
                Err(err) => {
                    self.drive_status(format!("Couldn't list {}: {err}", folder.name));
                    Vec::new()
                }
            }
        } else if self.drive_offline() {
            crate::demo::drive_listing("files", &folder.item_id, now_ms())
        } else {
            match browse::children(&self.client, &folder.drive_id, &folder.item_id, None).await {
                Ok(p) => p.items,
                Err(err) => {
                    self.drive_status(format!("Couldn't list that folder: {err}"));
                    Vec::new()
                }
            }
        };
        {
            let mut d = self.drive.lock().unwrap();
            let Some(p) = d.picker.as_mut().filter(|p| p.generation == generation) else {
                return Ok(());
            };
            p.items = items
                .into_iter()
                .filter(|i| i.is_folder || !folders_only)
                .collect();
            p.loading = false;
        }
        self.drive_picker_push();
        Ok(())
    }

    fn drive_pick_select(&self, id: &str) {
        if let Some(p) = self.drive.lock().unwrap().picker.as_mut()
            && p.items.iter().any(|i| i.id == id && !i.is_folder)
        {
            p.selected = Some(id.to_owned());
        }
        self.drive_picker_push();
    }

    async fn drive_pick_choose(self: &Arc<Self>) -> anyhow::Result<()> {
        let (purpose, dest, file) = {
            let mut d = self.drive.lock().unwrap();
            let Some(p) = d.picker.take() else {
                return Ok(());
            };
            let file = p
                .selected
                .as_ref()
                .and_then(|id| p.items.iter().find(|i| &i.id == id).cloned());
            (p.purpose, p.trail.last().cloned(), file)
        };
        self.ui.drive_dialog("");
        self.drive_picked(&purpose, dest, file).await
    }

    /// What happens when the user confirmed the picker. Features that attach cloud files add
    /// their purpose here (`file` is the chosen file, `dest` the folder it was in).
    pub(super) async fn drive_picked(
        self: &Arc<Self>,
        purpose: &str,
        dest: Option<Crumb>,
        file: Option<DriveItem>,
    ) -> anyhow::Result<()> {
        match (purpose, dest) {
            ("move", Some(dest)) => self.drive_move_copy(true, dest).await,
            ("copy", Some(dest)) => self.drive_move_copy(false, dest).await,
            ("attach", _) => {
                let Some(f) = file else {
                    return Ok(());
                };
                let drive_id = if f.drive_id.is_empty() {
                    self.drive_my_drive_id().await.unwrap_or_default()
                } else {
                    f.drive_id.clone()
                };
                self.attach_cloud_file(&drive_id, &f.id, &f.name).await
            }
            (other, _) => {
                tracing::info!(purpose = other, picked = ?file.map(|f| f.name), "file picked with no handler");
                Ok(())
            }
        }
    }

    async fn drive_move_copy(self: &Arc<Self>, moving: bool, dest: Crumb) -> anyhow::Result<()> {
        let sel = self.drive_selection();
        if sel.is_empty() {
            return Ok(());
        }
        if self.demo_note() {
            return Ok(());
        }
        let mine = self.drive_my_drive_id().await.unwrap_or_default();
        let norm = |d: &str| {
            if d.is_empty() {
                mine.clone()
            } else {
                d.to_owned()
            }
        };
        let target = ItemRef::new(&dest.drive_id, &dest.item_id);
        let (mut done, mut failed) = (0usize, Vec::new());
        for i in &sel {
            let r = if moving {
                if norm(&i.drive_id) != norm(&dest.drive_id) {
                    failed.push(format!(
                        "{}: moving between libraries is not supported, use Copy to",
                        i.name
                    ));
                    continue;
                }
                ops::move_to(&self.client, &ItemRef::own(i), &target, None).await
            } else {
                self.drive_status(format!("Copying {}...", i.name));
                ops::copy_to(&self.client, &ItemRef::from(i), &target, None).await
            };
            match r {
                Ok(_) => done += 1,
                Err(err) => failed.push(format!("{}: {err}", i.name)),
            }
        }
        if moving && done > 0 {
            // Moved items are no longer in this folder.
            self.drive_refresh_select(None).await?;
        }
        let verb = if moving { "Moved" } else { "Copied" };
        let mut status = format!("{verb} {done} item{} to {}", plural(done), dest.name);
        if !failed.is_empty() {
            status.push_str(&format!(". Failed: {}", failed.join("; ")));
        }
        self.drive_status(status);
        Ok(())
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// A file name that is safe to create in a directory of ours.
fn sanitize_file_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let s = s.trim_matches('.').trim().to_owned();
    if s.is_empty() { "download".into() } else { s }
}

/// What the viewer shows for a downloaded file.
fn preview_of(item: &DriveItem, bytes: &[u8]) -> PreviewData {
    match Preview::of(item) {
        Preview::Image => PreviewData {
            title: item.name.clone(),
            kind: "image",
            image: super::util::decode_image(bytes, 1600, 1100),
            text: String::new(),
            page: 0,
            pages: 0,
        },
        p => PreviewData {
            title: item.name.clone(),
            kind: if p == Preview::Markdown {
                "markdown"
            } else {
                "text"
            },
            image: None,
            text: String::from_utf8_lossy(bytes)
                .chars()
                .take(MAX_TEXT_CHARS)
                .collect(),
            page: 0,
            pages: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_files_setting_decides_where_office_files_open() {
        use super::super::settings::OpenFilesIn::*;
        for kind in [FileKind::Word, FileKind::Excel, FileKind::PowerPoint] {
            assert_eq!(with_files_pref(Kelma, Preview::Pages, kind), Preview::Pages);
            assert_eq!(
                with_files_pref(Browser, Preview::Pages, kind),
                Preview::Browser
            );
            assert_eq!(
                with_files_pref(Desktop, Preview::Pages, kind),
                Preview::Player
            );
        }
        // Pictures, text and PDFs are not Office files: the setting leaves them alone.
        for kind in [FileKind::Image, FileKind::Pdf, FileKind::Text] {
            assert_eq!(
                with_files_pref(Desktop, Preview::Image, kind),
                Preview::Image
            );
            assert_eq!(
                with_files_pref(Browser, Preview::Pages, kind),
                Preview::Pages
            );
        }
    }

    #[test]
    fn path_prefixes_walk_down_a_drive_path() {
        assert_eq!(
            path_prefixes("/Docs/Reports/2026"),
            ["Docs", "Docs/Reports", "Docs/Reports/2026"]
        );
        assert!(path_prefixes("").is_empty());
        assert!(path_prefixes("/").is_empty());
    }

    #[test]
    fn people_are_invited_by_email_else_by_object_id() {
        let p = |mri: &str, email: Option<&str>| PersonData {
            id: mri.into(),
            name: "x".into(),
            detail: String::new(),
            picked: true,
            email: email.map(str::to_owned),
        };
        assert_eq!(
            recipient_of(&p("8:orgid:aaa", Some("a@x.com"))),
            Some(Recipient::Email("a@x.com".into()))
        );
        assert_eq!(
            recipient_of(&p("8:orgid:aaa", None)),
            Some(Recipient::ObjectId("aaa".into()))
        );
        assert_eq!(recipient_of(&p("28:bot", None)), None);
    }

    #[test]
    fn permissions_read_as_people_and_links() {
        let now = 1_795_000_000_000;
        let person = Permission {
            id: "p1".into(),
            roles: vec!["write".into()],
            granted_to: vec!["Ada".into(), "Grace".into()],
            ..Default::default()
        };
        let d = perm_data(&person, now);
        assert_eq!(
            (d.who.as_str(), d.role.as_str(), d.link, d.editable),
            ("Ada, Grace", "write", false, true)
        );
        let link = Permission {
            id: "p2".into(),
            roles: vec!["read".into()],
            link_scope: Some("organization".into()),
            link_type: Some("blocksDownload".into()),
            expires_ms: Some(now + 86_400_000),
            ..Default::default()
        };
        let d = perm_data(&link, now);
        assert!(d.link && d.who.starts_with("People in your organization"));
        assert!(d.detail.contains("download blocked") && d.detail.contains("expires"));
        let owner = Permission {
            id: "p3".into(),
            roles: vec!["owner".into()],
            granted_to: vec!["Me".into()],
            ..Default::default()
        };
        assert!(
            !perm_data(&owner, now).editable,
            "the owner cannot be removed"
        );
        let inherited = Permission {
            inherited: true,
            ..person
        };
        assert!(!perm_data(&inherited, now).editable);
    }

    #[test]
    fn versions_mark_the_newest_as_current() {
        let v = |id: &str, ms| Version {
            id: id.into(),
            modified_ms: ms,
            modified_by: "Ada".into(),
            size: 2048,
        };
        let d = version_data(
            &[v("3.0", 3_000_000_000_000), v("2.0", 2_000_000_000_000)],
            3_000_000_000_000,
        );
        assert!(d[0].current && !d[1].current);
        assert_eq!(d[1].size, "2 KB");
    }

    #[test]
    fn link_expiry_is_an_iso_time_days_ahead() {
        assert_eq!(link_expiry(1, 0), "1970-01-02T00:00:00.000Z");
    }

    #[test]
    fn file_names_cannot_escape_the_target_folder() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize_file_name("a/b.txt"), "a_b.txt");
        assert_eq!(sanitize_file_name(".."), "download");
        assert_eq!(sanitize_file_name("Q4 plan.xlsx"), "Q4 plan.xlsx");
    }

    #[test]
    fn details_list_the_facts() {
        let i = DriveItem {
            name: "Plan.docx".into(),
            size: 20_480,
            modified_by: "Ada".into(),
            modified_ms: 1_795_000_000_000,
            parent_path: "/Docs".into(),
            web_url: "https://x/Plan.docx".into(),
            ..Default::default()
        };
        let t = details_text(&i, true);
        for want in [
            "Name: Plan.docx",
            "Type: word",
            "Size: 20 KB",
            "Location: /Docs",
            "by Ada",
            "Favorite: yes",
            "https://x/Plan.docx",
        ] {
            assert!(t.contains(want), "{want} in {t}");
        }
    }

    #[test]
    fn a_text_file_is_cut_and_a_picture_is_decoded() {
        let big = "a".repeat(MAX_TEXT_CHARS + 50);
        let item = DriveItem {
            name: "x.txt".into(),
            ..Default::default()
        };
        assert_eq!(
            preview_of(&item, big.as_bytes()).text.chars().count(),
            MAX_TEXT_CHARS
        );
        let md = DriveItem {
            name: "x.md".into(),
            ..Default::default()
        };
        assert_eq!(preview_of(&md, b"# hi").kind, "markdown");
        let img = DriveItem {
            name: "x.png".into(),
            ..Default::default()
        };
        let mut png = Vec::new();
        image::RgbaImage::new(2, 2)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let p = preview_of(&img, &png);
        assert_eq!(p.kind, "image");
        assert!(p.image.is_some());
    }
    #[test]
    fn people_become_rows_that_count_their_files() {
        let people = crate::demo::drive_people_files(1_795_000_000_000);
        let rows = people_items(&people);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "person:0");
        assert_eq!(rows[0].name, "Sara Hassan");
        assert_eq!(rows[0].child_count, 2);
        assert!(rows[0].is_folder);
        assert_eq!(rows[0].modified_ms, people[0].latest_ms);
        // A person with no name is shown by address, then as Unknown.
        let nameless = onedrive::Person {
            address: "a@example.test".into(),
            ..Default::default()
        };
        assert_eq!(person_label(&nameless), "a@example.test");
        assert_eq!(person_label(&onedrive::Person::default()), "Unknown");
    }

    #[test]
    fn shared_by_you_keeps_drive_items_and_drops_the_sharer_name() {
        let me = onedrive::Person {
            name: "Me".into(),
            ..Default::default()
        };
        let with_item = insights::Insight {
            title: "Plan.docx".into(),
            item: Some(ItemRef::new("d1", "i1")),
            shared_by: Some(me.clone()),
            ..Default::default()
        };
        let page = insights::Insight {
            title: "A page".into(),
            shared_by: Some(me),
            ..Default::default()
        };
        let rows = shared_by_you_items(vec![with_item, page]);
        assert_eq!(rows.len(), 1, "only drive items can be acted on");
        assert_eq!(
            (rows[0].name.as_str(), rows[0].id.as_str()),
            ("Plan.docx", "i1")
        );
        assert_eq!(rows[0].modified_by, "");
    }

    #[test]
    fn recordings_show_their_place_and_my_files_keeps_its_path() {
        let item = |name: &str, path: &str| DriveItem {
            id: name.into(),
            name: name.into(),
            parent_path: path.into(),
            ..Default::default()
        };
        let rows = recording_items(vec![
            onedrive::Recording {
                item: item("mine.mp4", "/Recordings"),
                place: "My files".into(),
            },
            onedrive::Recording {
                item: item("team.mp4", "/General/Recordings"),
                place: "Sales \u{203a} General".into(),
            },
        ]);
        assert_eq!(rows[0].parent_path, "/Recordings");
        assert_eq!(rows[1].parent_path, "Sales \u{203a} General");
    }
}
