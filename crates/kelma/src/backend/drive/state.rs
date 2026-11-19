//! The OneDrive view as plain data: where the user is, what is listed, how it is sorted, grouped
//! and filtered, what is selected, and the rows the UI shows. No network and no UI types, so all of
//! it is unit tested.

use crate::data::Pixels;
use onedrive::{DriveItem, SortKey, group_items, sort_items};
use std::collections::{HashMap, HashSet};

/// A part of OneDrive the left navigation opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Section {
    Home,
    MyFiles,
    Shared,
    Favorites,
    Libraries,
    Bin,
    Meetings,
    /// "Shared by you".
    SharedByYou,
    /// "Browse files by People": who shared files with the user.
    People,
    /// The files one person shared with the user, by display name.
    Person(String),
    /// "Browse files by Media": photos and videos in My files.
    Media,
    /// A document library (a team's Documents, a site library), by drive id.
    Library(String),
    /// Search results for a query.
    Search(String),
}

impl Section {
    /// The key the UI uses for the navigation entry.
    pub fn key(&self) -> String {
        match self {
            Section::Home => "home".into(),
            Section::MyFiles => "files".into(),
            Section::Shared => "shared".into(),
            Section::Favorites => "favorites".into(),
            Section::Libraries => "libraries".into(),
            Section::Bin => "bin".into(),
            Section::Meetings => "meetings".into(),
            Section::SharedByYou => "shared-by".into(),
            Section::People => "people".into(),
            Section::Person(name) => format!("person:{name}"),
            Section::Media => "media".into(),
            Section::Library(id) => format!("drive:{id}"),
            Section::Search(_) => "search".into(),
        }
    }

    pub fn from_key(key: &str) -> Option<Section> {
        Some(match key {
            "home" => Section::Home,
            "files" => Section::MyFiles,
            "shared" => Section::Shared,
            "favorites" => Section::Favorites,
            "libraries" => Section::Libraries,
            "bin" => Section::Bin,
            "meetings" => Section::Meetings,
            "shared-by" => Section::SharedByYou,
            "people" => Section::People,
            "media" => Section::Media,
            k if k.strip_prefix("person:").is_some_and(|n| !n.is_empty()) => {
                Section::Person(k["person:".len()..].into())
            }
            k => Section::Library(k.strip_prefix("drive:").filter(|d| !d.is_empty())?.into()),
        })
    }

    /// Heading of the list.
    pub fn title(&self, libraries: &[Library]) -> String {
        match self {
            Section::Home => "Home".into(),
            Section::MyFiles => "My files".into(),
            Section::Shared => "Shared".into(),
            Section::Favorites => "Favorites".into(),
            Section::Libraries => "Libraries".into(),
            Section::Bin => "Recycle bin".into(),
            Section::Meetings => "Meetings".into(),
            Section::SharedByYou => "Shared by you".into(),
            Section::People => "People".into(),
            Section::Person(name) => name.clone(),
            Section::Media => "Media".into(),
            Section::Library(id) => libraries
                .iter()
                .find(|l| &l.drive_id == id)
                .map_or_else(|| "Library".into(), |l| l.name.clone()),
            Section::Search(q) => format!("Search: {q}"),
        }
    }

    /// Lists the data layer returns newest first, which the default view keeps.
    fn newest_first(&self) -> bool {
        matches!(
            self,
            Section::Bin
                | Section::People
                | Section::Person(_)
                | Section::Media
                | Section::Meetings
                | Section::SharedByYou
        )
    }

    /// Sections that list folders the user can open and write into.
    pub fn is_browsable(&self) -> bool {
        matches!(self, Section::MyFiles | Section::Library(_))
    }
}

/// A document library listed under Libraries and Quick access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    pub drive_id: String,
    pub name: String,
}

/// One folder of the breadcrumb trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub name: String,
    pub drive_id: String,
    pub item_id: String,
}

/// What kind of file an item is; drives the icon, the type pills and how it opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Folder,
    Word,
    Excel,
    PowerPoint,
    Pdf,
    Image,
    Video,
    Audio,
    Markdown,
    Text,
    Code,
    Archive,
    Other,
}

impl FileKind {
    pub fn of(item: &DriveItem) -> FileKind {
        if item.is_folder {
            return FileKind::Folder;
        }
        let ext = extension(&item.name);
        match ext.as_str() {
            "doc" | "docx" | "dot" | "dotx" | "odt" | "rtf" => FileKind::Word,
            "xls" | "xlsx" | "xlsm" | "xlsb" | "ods" => FileKind::Excel,
            "ppt" | "pptx" | "pps" | "ppsx" | "odp" => FileKind::PowerPoint,
            "pdf" => FileKind::Pdf,
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "heic" => {
                FileKind::Image
            }
            "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" | "wmv" => FileKind::Video,
            "mp3" | "wav" | "m4a" | "ogg" | "flac" | "aac" => FileKind::Audio,
            "md" | "markdown" => FileKind::Markdown,
            "txt" | "log" | "csv" | "tsv" => FileKind::Text,
            "rs" | "py" | "js" | "ts" | "json" | "yaml" | "yml" | "toml" | "xml" | "html"
            | "css" | "sh" | "c" | "cpp" | "h" | "java" | "go" | "rb" | "php" | "sql" | "svg"
            | "ini" | "conf" => FileKind::Code,
            "zip" | "7z" | "tar" | "gz" | "rar" => FileKind::Archive,
            _ => match item.mime.split('/').next() {
                Some("image") => FileKind::Image,
                Some("video") => FileKind::Video,
                Some("audio") => FileKind::Audio,
                Some("text") => FileKind::Text,
                _ => FileKind::Other,
            },
        }
    }

    /// The word the UI keys its icon colour on.
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::Folder => "folder",
            FileKind::Word => "word",
            FileKind::Excel => "excel",
            FileKind::PowerPoint => "powerpoint",
            FileKind::Pdf => "pdf",
            FileKind::Image => "image",
            FileKind::Video => "video",
            FileKind::Audio => "audio",
            FileKind::Markdown => "markdown",
            FileKind::Text => "text",
            FileKind::Code => "code",
            FileKind::Archive => "archive",
            FileKind::Other => "other",
        }
    }
}

/// How a file opens inside kelma.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preview {
    /// Shown by kelma: a decoded picture.
    Image,
    /// Shown by kelma as text (plain, code) or rendered markdown.
    Text,
    Markdown,
    /// A PDF, or an Office file Graph converts to PDF: shown by kelma page by page.
    Pages,
    /// The rest: the browser, through the preview or web URL.
    Browser,
    /// Video and audio: the system player through `xdg-open`.
    Player,
}

/// Image types the `image` crate decodes here.
const DECODABLE: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

impl Preview {
    pub fn of(item: &DriveItem) -> Preview {
        match FileKind::of(item) {
            FileKind::Image if DECODABLE.contains(&extension(&item.name).as_str()) => {
                Preview::Image
            }
            FileKind::Pdf => Preview::Pages,
            FileKind::Word | FileKind::Excel | FileKind::PowerPoint
                if preview::can_convert(&item.name) =>
            {
                Preview::Pages
            }
            FileKind::Markdown => Preview::Markdown,
            FileKind::Text | FileKind::Code => Preview::Text,
            FileKind::Video | FileKind::Audio => Preview::Player,
            _ => Preview::Browser,
        }
    }
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// The type pills of Home.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TypeFilter {
    #[default]
    All,
    Word,
    Excel,
    PowerPoint,
    Pdf,
    More,
}

impl TypeFilter {
    pub fn parse(s: &str) -> TypeFilter {
        match s {
            "word" => TypeFilter::Word,
            "excel" => TypeFilter::Excel,
            "powerpoint" => TypeFilter::PowerPoint,
            "pdf" => TypeFilter::Pdf,
            "more" => TypeFilter::More,
            _ => TypeFilter::All,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TypeFilter::All => "all",
            TypeFilter::Word => "word",
            TypeFilter::Excel => "excel",
            TypeFilter::PowerPoint => "powerpoint",
            TypeFilter::Pdf => "pdf",
            TypeFilter::More => "more",
        }
    }

    fn matches(self, kind: FileKind) -> bool {
        match self {
            TypeFilter::All => true,
            TypeFilter::Word => kind == FileKind::Word,
            TypeFilter::Excel => kind == FileKind::Excel,
            TypeFilter::PowerPoint => kind == FileKind::PowerPoint,
            TypeFilter::Pdf => kind == FileKind::Pdf,
            TypeFilter::More => !matches!(
                kind,
                FileKind::Folder
                    | FileKind::Word
                    | FileKind::Excel
                    | FileKind::PowerPoint
                    | FileKind::Pdf
            ),
        }
    }
}

/// Which optional columns the list shows (Name is always there).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Columns {
    pub modified: bool,
    pub modified_by: bool,
    pub size: bool,
    pub location: bool,
    pub activity: bool,
}

impl Default for Columns {
    fn default() -> Self {
        Columns {
            modified: true,
            modified_by: true,
            size: true,
            location: false,
            activity: false,
        }
    }
}

impl Columns {
    /// Flips one column by name; unknown names change nothing.
    pub fn toggle(&mut self, name: &str) {
        let flag = match name {
            "modified" => &mut self.modified,
            "modified-by" => &mut self.modified_by,
            "size" => &mut self.size,
            "location" => &mut self.location,
            "activity" => &mut self.activity,
            _ => return,
        };
        *flag = !*flag;
    }
}

pub fn parse_sort_key(s: &str) -> Option<SortKey> {
    Some(match s {
        "name" => SortKey::Name,
        "modified" => SortKey::Modified,
        "modified-by" => SortKey::ModifiedBy,
        "size" => SortKey::Size,
        _ => return None,
    })
}

pub fn sort_key_name(k: SortKey) -> &'static str {
    match k {
        SortKey::Name => "name",
        SortKey::Modified => "modified",
        SortKey::ModifiedBy => "modified-by",
        SortKey::Size => "size",
    }
}

/// "5 B", "412 KB", "1.2 MB", "3.4 GB".
pub fn size_label(bytes: u64) -> String {
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{} KB", bytes.div_ceil(1024)),
        1_048_576..=1_073_741_823 => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
        _ => format!("{:.1} GB", bytes as f64 / 1_073_741_824.0),
    }
}

/// "10:30" today, "Yesterday", "Nov 10" this year, "Nov 10, 2025" before. `off_ms` is local time
/// minus UTC at `ms` (in the time zone chosen in Settings), so "today" is the day there.
pub fn when_label(ms: i64, now_ms: i64, off_ms: i64) -> String {
    use chrono::{Datelike, TimeZone, Utc};
    if ms <= 0 {
        return String::new();
    }
    let local = |t: i64| Utc.timestamp_millis_opt(t + off_ms).single();
    let (Some(t), Some(now)) = (local(ms), local(now_ms)) else {
        return String::new();
    };
    let days = (now.date_naive() - t.date_naive()).num_days();
    match days {
        0 => t.format(appearance::formats::time_pattern()).to_string(),
        1 => "Yesterday".into(),
        _ if t.year() == now.year() => t.format("%b %-d").to_string(),
        _ => t.format("%b %-d, %Y").to_string(),
    }
}

/// One row of the list: an item, a group header, or (in the recycle bin) a deleted item.
#[derive(Clone, Default)]
pub struct RowData {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub modified: String,
    pub modified_by: String,
    pub size: String,
    pub location: String,
    pub activity: String,
    pub selected: bool,
    pub favorite: bool,
    pub is_folder: bool,
    pub shortcut: bool,
    /// A group header: only `name` (the label) is meaningful.
    pub header: bool,
    /// Position among the item rows (headers excluded), for the grid.
    pub index: i32,
    pub thumb: Option<Pixels>,
}

/// A "For you" card on Home: a file and why it is shown.
#[derive(Clone, Default)]
pub struct CardData {
    /// Key of the card (also the key of its thumbnail in `State::thumbs`).
    pub id: String,
    pub title: String,
    pub reason: String,
    pub kind: String,
    pub thumb: Option<Pixels>,
}

/// What the selection allows, for the toolbar.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct SelectionInfo {
    pub count: usize,
    /// Name of the item when exactly one is selected.
    pub name: String,
    pub has_folder: bool,
    /// Every selected item is a favorite.
    pub all_favorite: bool,
}

/// All that the main list shows, ready to push to the UI.
#[derive(Clone, Default)]
pub struct DriveData {
    pub section: String,
    pub title: String,
    pub rows: Vec<RowData>,
    /// The "For you" cards, on Home only.
    pub cards: Vec<CardData>,
    /// Breadcrumb labels, root first; only folders of a browsable section have more than one.
    pub crumbs: Vec<String>,
    pub quick: Vec<(String, String)>,
    pub grid: bool,
    pub sort: &'static str,
    pub ascending: bool,
    /// `none` or a sort key name.
    pub group: &'static str,
    pub columns: Columns,
    pub type_filter: &'static str,
    pub filter: String,
    pub status: String,
    pub loading: bool,
    pub more: bool,
    pub selection: SelectionInfo,
}

/// Everything about the main view that is not a dialog.
pub struct State {
    pub section: Section,
    /// Folders opened below the section root (My files or a library), root first.
    pub trail: Vec<Crumb>,
    pub items: Vec<DriveItem>,
    pub next_link: Option<String>,
    pub libraries: Vec<Library>,
    pub favorites: HashSet<String>,
    pub selected: Vec<String>,
    pub grid: bool,
    pub sort: SortKey,
    pub ascending: bool,
    pub group: Option<SortKey>,
    pub columns: Columns,
    pub type_filter: TypeFilter,
    pub filter: String,
    pub status: String,
    pub loading: bool,
    /// Bumped on every navigation; a listing that comes back with an older number is dropped.
    pub generation: u64,
    pub thumbs: HashMap<String, Pixels>,
    /// Home's "For you" cards, newest load first; each card's thumbnail is `thumbs[card.id]`.
    pub cards: Vec<Card>,
}

/// A "For you" card with the item it opens.
#[derive(Clone)]
pub struct Card {
    pub id: String,
    pub item: DriveItem,
    pub reason: String,
}

impl Default for State {
    fn default() -> Self {
        State {
            section: Section::Home,
            trail: Vec::new(),
            items: Vec::new(),
            next_link: None,
            libraries: Vec::new(),
            favorites: HashSet::new(),
            selected: Vec::new(),
            grid: false,
            sort: SortKey::Name,
            ascending: true,
            group: None,
            columns: Columns::default(),
            type_filter: TypeFilter::All,
            filter: String::new(),
            status: String::new(),
            loading: false,
            generation: 0,
            thumbs: HashMap::new(),
            cards: Vec::new(),
        }
    }
}

impl State {
    /// Opens a section: the trail starts at its root and the listing is cleared.
    pub fn go(&mut self, section: Section) -> u64 {
        self.trail = match &section {
            Section::MyFiles => vec![Crumb {
                name: "My files".into(),
                drive_id: String::new(),
                item_id: "root".into(),
            }],
            Section::Library(id) => vec![Crumb {
                name: section.title(&self.libraries),
                drive_id: id.clone(),
                item_id: "root".into(),
            }],
            _ => Vec::new(),
        };
        self.section = section;
        self.reset_listing()
    }

    /// Opens a folder below the current section root.
    pub fn open_folder(&mut self, folder: &DriveItem) -> u64 {
        self.trail.push(Crumb {
            name: folder.name.clone(),
            drive_id: folder.drive_id.clone(),
            item_id: folder.id.clone(),
        });
        self.reset_listing()
    }

    /// Opens a folder reached from a list that is not a folder view (Home, Shared, Libraries,
    /// Favorites): it becomes the root of a library-style view.
    pub fn open_foreign_folder(&mut self, folder: &DriveItem) -> u64 {
        self.section = Section::Library(folder.drive_id.clone());
        self.trail = vec![Crumb {
            name: folder.name.clone(),
            drive_id: folder.drive_id.clone(),
            item_id: folder.id.clone(),
        }];
        self.reset_listing()
    }

    /// Goes back to the folder at breadcrumb `index` (0 is the section root). Out of range does
    /// nothing and returns `None`.
    pub fn go_to_crumb(&mut self, index: usize) -> Option<u64> {
        if index >= self.trail.len() {
            return None;
        }
        self.trail.truncate(index + 1);
        Some(self.reset_listing())
    }

    /// The folder being listed, for a browsable section.
    pub fn folder(&self) -> Option<&Crumb> {
        self.trail.last().filter(|_| self.section.is_browsable())
    }

    fn reset_listing(&mut self) -> u64 {
        self.items.clear();
        self.next_link = None;
        self.selected.clear();
        self.filter.clear();
        self.status.clear();
        self.loading = true;
        self.generation += 1;
        self.generation
    }

    /// Stores a listing unless a newer navigation started meanwhile. Returns whether it was used.
    pub fn set_listing(
        &mut self,
        generation: u64,
        items: Vec<DriveItem>,
        next: Option<String>,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        self.items = items;
        self.next_link = next;
        self.loading = false;
        let ids: HashSet<&str> = self.items.iter().map(|i| i.id.as_str()).collect();
        self.selected.retain(|s| ids.contains(s.as_str()));
        true
    }

    /// Adds the next page to the listing.
    pub fn append_listing(
        &mut self,
        generation: u64,
        items: Vec<DriveItem>,
        next: Option<String>,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        self.items.extend(items);
        self.next_link = next;
        self.loading = false;
        true
    }

    pub fn item(&self, id: &str) -> Option<&DriveItem> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn selected_items(&self) -> Vec<&DriveItem> {
        self.selected
            .iter()
            .filter_map(|id| self.item(id))
            .collect()
    }

    /// Click on a row: it becomes the only selected one.
    pub fn select_only(&mut self, id: &str) {
        if self.item(id).is_some() && self.selectable() {
            self.selected = vec![id.to_owned()];
        }
    }

    /// Checkbox: adds or removes one row.
    pub fn toggle(&mut self, id: &str) {
        if self.item(id).is_none() || !self.selectable() {
            return;
        }
        match self.selected.iter().position(|s| s == id) {
            Some(i) => {
                self.selected.remove(i);
            }
            None => self.selected.push(id.to_owned()),
        }
    }

    /// The People list holds people, not files: nothing there can be acted on.
    fn selectable(&self) -> bool {
        self.section != Section::People
    }

    /// Selects every row the filters show.
    pub fn select_all(&mut self) {
        if !self.selectable() {
            return;
        }
        self.selected = self.visible().into_iter().map(|i| i.id).collect();
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    /// Chooses the sort key; choosing the current one flips the direction.
    pub fn set_sort(&mut self, key: SortKey) {
        if self.sort == key {
            self.ascending = !self.ascending;
        } else {
            self.sort = key;
            // Newest and biggest first is what people want when they first pick these.
            self.ascending = matches!(key, SortKey::Name | SortKey::ModifiedBy);
        }
    }

    /// The listing after the type pill and the name filter, sorted. While grouped, the group key
    /// decides the order (groups must be contiguous); the direction follows `ascending`.
    pub fn visible(&self) -> Vec<DriveItem> {
        let q = self.filter.trim().to_lowercase();
        let mut v: Vec<DriveItem> = self
            .items
            .iter()
            .filter(|i| self.type_filter.matches(FileKind::of(i)))
            .filter(|i| {
                q.is_empty()
                    || i.name.to_lowercase().contains(&q)
                    || i.modified_by.to_lowercase().contains(&q)
            })
            .cloned()
            .collect();
        // These lists have no meaningful name order of their own: newest first (the bin by
        // deletion date, People by latest share) until the user picks a sort.
        if self.section.newest_first() && self.group.is_none() && self.sort == SortKey::Name {
            v.sort_by_key(|i| std::cmp::Reverse(i.modified_ms));
            return v;
        }
        match self.group {
            // Groups must be contiguous, so folders do not float to the top, except for Size
            // where they are a group of their own.
            Some(key) if key != SortKey::Size => sort_flat(&mut v, key, self.ascending),
            key => sort_items(&mut v, key.unwrap_or(self.sort), self.ascending),
        }
        v
    }

    fn row(&self, i: &DriveItem, index: i32, now_ms: i64, off: &dyn Fn(i64) -> i64) -> RowData {
        let bin = self.section == Section::Bin;
        RowData {
            id: i.id.clone(),
            name: i.name.clone(),
            kind: FileKind::of(i).as_str().into(),
            modified: when_label(i.modified_ms, now_ms, off(i.modified_ms)),
            modified_by: i.modified_by.clone(),
            size: if i.is_folder {
                match i.child_count {
                    0 => String::new(),
                    1 => "1 item".into(),
                    n => format!("{n} items"),
                }
            } else {
                size_label(i.size)
            },
            location: if i.parent_path.is_empty() {
                String::new()
            } else {
                i.parent_path.trim_start_matches('/').to_owned()
            },
            activity: match (bin, i.modified_by.is_empty()) {
                (true, _) => format!("Deleted by {}", or_unknown(&i.modified_by)),
                (false, true) => String::new(),
                (false, false) => format!("{} modified this", i.modified_by),
            },
            selected: self.selected.contains(&i.id),
            favorite: self.favorites.contains(&i.id),
            is_folder: i.is_folder,
            shortcut: i.is_remote_shortcut,
            header: false,
            index,
            thumb: self.thumbs.get(&i.id).cloned(),
        }
    }

    /// The rows to show: the visible items, with a header row before each group when grouped.
    pub fn rows(&self, now_ms: i64, off: &dyn Fn(i64) -> i64) -> Vec<RowData> {
        let visible = self.visible();
        let mut out = Vec::with_capacity(visible.len());
        let mut n = 0;
        match self.group {
            Some(key) => {
                // Group on local wall-clock days, so "Today" starts at local midnight, not UTC.
                let local: Vec<DriveItem> = visible
                    .iter()
                    .map(|i| DriveItem {
                        modified_ms: i.modified_ms + off(i.modified_ms),
                        ..(*i).clone()
                    })
                    .collect();
                for g in group_items(&local, key, now_ms + off(now_ms)) {
                    out.push(RowData {
                        id: format!("group:{}", g.label),
                        name: g.label,
                        header: true,
                        ..Default::default()
                    });
                    for shifted in &g.items {
                        let Some(i) = visible.iter().find(|v| v.id == shifted.id) else {
                            continue;
                        };
                        out.push(self.row(i, n, now_ms, off));
                        n += 1;
                    }
                }
            }
            None => {
                for i in &visible {
                    out.push(self.row(i, n, now_ms, off));
                    n += 1;
                }
            }
        }
        out
    }

    /// The items in the order they were listed, with no sorting, grouping or filtering.
    pub fn plain_rows(&self, now_ms: i64, off: &dyn Fn(i64) -> i64) -> Vec<RowData> {
        self.items
            .iter()
            .enumerate()
            .map(|(n, i)| self.row(i, n as i32, now_ms, off))
            .collect()
    }

    pub fn selection_info(&self) -> SelectionInfo {
        let sel = self.selected_items();
        SelectionInfo {
            count: sel.len(),
            name: if let [one] = sel.as_slice() {
                one.name.clone()
            } else {
                String::new()
            },
            has_folder: sel.iter().any(|i| i.is_folder),
            all_favorite: !sel.is_empty() && sel.iter().all(|i| self.favorites.contains(&i.id)),
        }
    }

    /// Breadcrumb labels: the section title, then the folders below it.
    pub fn crumbs(&self) -> Vec<String> {
        if self.section.is_browsable() {
            self.trail.iter().map(|c| c.name.clone()).collect()
        } else if let Section::Person(name) = &self.section {
            vec!["People".into(), name.clone()]
        } else {
            vec![self.section.title(&self.libraries)]
        }
    }

    /// The cards Home shows above the list (none while a name filter is typed).
    fn card_data(&self) -> Vec<CardData> {
        if self.section != Section::Home || !self.filter.trim().is_empty() {
            return Vec::new();
        }
        self.cards
            .iter()
            .map(|c| CardData {
                id: c.id.clone(),
                title: c.item.name.clone(),
                reason: c.reason.clone(),
                kind: FileKind::of(&c.item).as_str().into(),
                thumb: self.thumbs.get(&c.id).cloned(),
            })
            .collect()
    }

    pub fn data(&self, now_ms: i64, off: &dyn Fn(i64) -> i64) -> DriveData {
        DriveData {
            section: self.section.key(),
            title: self.section.title(&self.libraries),
            rows: self.rows(now_ms, off),
            cards: self.card_data(),
            crumbs: self.crumbs(),
            quick: self
                .libraries
                .iter()
                .map(|l| (format!("drive:{}", l.drive_id), l.name.clone()))
                .collect(),
            grid: self.grid,
            sort: sort_key_name(self.sort),
            ascending: self.ascending,
            group: self.group.map_or("none", sort_key_name),
            // The recycle bin and Meetings are about where a file is: always show its location.
            columns: Columns {
                location: self.columns.location
                    || matches!(self.section, Section::Bin | Section::Meetings),
                ..self.columns
            },
            type_filter: self.type_filter.as_str(),
            filter: self.filter.clone(),
            status: self.status.clone(),
            loading: self.loading,
            more: self.next_link.is_some(),
            selection: self.selection_info(),
        }
    }
}

/// Like `onedrive::sort_items` but folders are sorted with the files.
fn sort_flat(items: &mut [DriveItem], key: SortKey, ascending: bool) {
    items.sort_by(|a, b| {
        let by_key = match key {
            SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortKey::Modified => a.modified_ms.cmp(&b.modified_ms),
            SortKey::ModifiedBy => a
                .modified_by
                .to_lowercase()
                .cmp(&b.modified_by.to_lowercase()),
            SortKey::Size => a.size.cmp(&b.size),
        }
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        if ascending { by_key } else { by_key.reverse() }
    });
}

fn or_unknown(s: &str) -> &str {
    if s.is_empty() { "someone" } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_795_000_000_000;
    const DAY_MS: i64 = 86_400_000;

    fn file(id: &str, name: &str, size: u64, ago_days: i64, by: &str) -> DriveItem {
        DriveItem {
            id: id.into(),
            drive_id: "d".into(),
            name: name.into(),
            size,
            modified_ms: NOW - ago_days * DAY_MS,
            modified_by: by.into(),
            ..Default::default()
        }
    }

    fn folder(id: &str, name: &str) -> DriveItem {
        DriveItem {
            id: id.into(),
            drive_id: "d".into(),
            name: name.into(),
            is_folder: true,
            child_count: 3,
            modified_ms: NOW - 3 * DAY_MS,
            ..Default::default()
        }
    }

    fn loaded() -> State {
        let mut s = State::default();
        let g = s.go(Section::MyFiles);
        s.set_listing(
            g,
            vec![
                file("1", "report.docx", 20_000, 0, "Ada"),
                file("2", "budget.xlsx", 3_000_000, 1, "Grace"),
                folder("3", "Projects"),
                file("4", "deck.pptx", 200_000, 9, "Ada"),
                file("5", "scan.pdf", 40_000_000, 40, "Linus"),
                file("6", "notes.md", 500, 3, "Grace"),
            ],
            None,
        );
        s
    }

    fn names(s: &State) -> Vec<String> {
        s.rows(NOW, &|_| 0).into_iter().map(|r| r.name).collect()
    }

    #[test]
    fn sections_round_trip_through_their_keys() {
        for k in [
            "home",
            "files",
            "shared",
            "favorites",
            "libraries",
            "bin",
            "meetings",
            "shared-by",
            "people",
            "media",
            "person:Ada Lovelace",
            "drive:b!x",
        ] {
            assert_eq!(Section::from_key(k).unwrap().key(), k);
        }
        assert_eq!(Section::from_key("drive:"), None);
        assert_eq!(Section::from_key("person:"), None);
        assert_eq!(Section::from_key("nope"), None);
    }

    #[test]
    fn file_kinds_come_from_the_extension_then_the_mime() {
        let k = |n: &str| FileKind::of(&file("x", n, 1, 0, ""));
        assert_eq!(k("A.DOCX"), FileKind::Word);
        assert_eq!(k("b.xlsx"), FileKind::Excel);
        assert_eq!(k("c.pptx"), FileKind::PowerPoint);
        assert_eq!(k("d.pdf"), FileKind::Pdf);
        assert_eq!(k("e.png"), FileKind::Image);
        assert_eq!(k("f.mp4"), FileKind::Video);
        assert_eq!(k("g.rs"), FileKind::Code);
        assert_eq!(k("h"), FileKind::Other);
        let mut m = file("x", "noext", 1, 0, "");
        m.mime = "image/heic".into();
        assert_eq!(FileKind::of(&m), FileKind::Image);
        assert_eq!(FileKind::of(&folder("f", "a.docx")), FileKind::Folder);
    }

    #[test]
    fn previews_pick_native_browser_or_player() {
        let p = |n: &str| Preview::of(&file("x", n, 1, 0, ""));
        assert_eq!(p("a.png"), Preview::Image);
        assert_eq!(p("a.heic"), Preview::Browser, "not decodable here");
        assert_eq!(p("a.md"), Preview::Markdown);
        assert_eq!(p("a.txt"), Preview::Text);
        assert_eq!(p("a.json"), Preview::Text);
        assert_eq!(p("a.docx"), Preview::Pages, "converted to PDF by Graph");
        assert_eq!(p("a.xlsx"), Preview::Pages);
        assert_eq!(p("a.pdf"), Preview::Pages);
        assert_eq!(p("a.zip"), Preview::Browser);
        assert_eq!(p("a.mp3"), Preview::Player);
    }

    #[test]
    fn sizes_and_dates_read_like_a_file_manager() {
        assert_eq!(size_label(0), "0 B");
        assert_eq!(size_label(2048), "2 KB");
        assert_eq!(size_label(1_572_864), "1.5 MB");
        assert_eq!(size_label(3 * 1_073_741_824), "3.0 GB");
        assert_eq!(when_label(NOW, NOW, 0).len(), 5, "HH:MM today");
        assert_eq!(when_label(NOW - DAY_MS, NOW, 0), "Yesterday");
        assert_eq!(when_label(NOW - 9 * DAY_MS, NOW, 0), "Nov 9");
        assert_eq!(when_label(NOW - 400 * DAY_MS, NOW, 0), "Oct 14, 2025");
        assert_eq!(when_label(0, NOW, 0), "");
    }

    #[test]
    fn when_labels_count_the_day_in_the_offset_zone() {
        use chrono::{TimeZone, Utc};
        let hour = 3_600_000;
        let at = Utc
            .with_ymd_and_hms(2026, 11, 18, 23, 0, 0)
            .unwrap()
            .timestamp_millis();
        let now = Utc
            .with_ymd_and_hms(2026, 11, 19, 12, 0, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(when_label(at, now, 0), "Yesterday");
        // At UTC+2 the same instant is 01:00 on the 19th: a time, not "Yesterday".
        assert!(when_label(at, now, 2 * hour).contains(":00"));
        assert_ne!(when_label(at, now, 2 * hour), "Yesterday");
    }

    #[test]
    fn my_files_starts_the_trail_and_folders_extend_it() {
        let mut s = State::default();
        s.go(Section::MyFiles);
        assert_eq!(s.crumbs(), ["My files"]);
        s.open_folder(&folder("3", "Projects"));
        s.open_folder(&folder("4", "2026"));
        assert_eq!(s.crumbs(), ["My files", "Projects", "2026"]);
        assert_eq!(s.folder().unwrap().item_id, "4");
        assert!(s.go_to_crumb(1).is_some());
        assert_eq!(s.crumbs(), ["My files", "Projects"]);
        assert!(s.go_to_crumb(0).is_some());
        assert_eq!(s.crumbs(), ["My files"]);
        assert!(s.go_to_crumb(5).is_none(), "out of range does nothing");
    }

    #[test]
    fn a_folder_opened_from_shared_becomes_its_own_root() {
        let mut s = State::default();
        s.go(Section::Shared);
        s.open_foreign_folder(&folder("9", "Team plans"));
        assert_eq!(s.crumbs(), ["Team plans"]);
        assert_eq!(s.folder().unwrap().item_id, "9");
        s.open_folder(&folder("10", "2026"));
        assert_eq!(s.crumbs(), ["Team plans", "2026"]);
    }

    #[test]
    fn other_sections_show_only_their_title_as_the_crumb() {
        let mut s = State::default();
        s.go(Section::Shared);
        assert_eq!(s.crumbs(), ["Shared"]);
        assert!(s.folder().is_none());
        s.libraries = vec![Library {
            drive_id: "b!1".into(),
            name: "Sales".into(),
        }];
        s.go(Section::Library("b!1".into()));
        assert_eq!(s.crumbs(), ["Sales"]);
        assert_eq!(s.folder().unwrap().drive_id, "b!1");
    }

    #[test]
    fn a_listing_for_an_older_navigation_is_dropped() {
        let mut s = State::default();
        let old = s.go(Section::MyFiles);
        let new = s.go(Section::Shared);
        assert!(!s.set_listing(old, vec![file("1", "a.txt", 1, 0, "")], None));
        assert!(s.items.is_empty() && s.loading);
        assert!(s.set_listing(new, vec![file("2", "b.txt", 1, 0, "")], Some("next".into())));
        assert_eq!(s.items.len(), 1);
        assert!(!s.loading);
        assert!(s.append_listing(new, vec![file("3", "c.txt", 1, 0, "")], None));
        assert_eq!(s.items.len(), 2);
        assert!(s.next_link.is_none());
    }

    #[test]
    fn folders_come_first_and_sorting_a_key_twice_flips_it() {
        let mut s = loaded();
        assert_eq!(
            names(&s),
            [
                "Projects",
                "budget.xlsx",
                "deck.pptx",
                "notes.md",
                "report.docx",
                "scan.pdf"
            ]
        );
        s.set_sort(SortKey::Size);
        assert!(!s.ascending, "size starts biggest first");
        assert_eq!(names(&s)[..3], ["Projects", "scan.pdf", "budget.xlsx"]);
        s.set_sort(SortKey::Size);
        assert!(s.ascending);
        assert_eq!(names(&s)[..3], ["Projects", "notes.md", "report.docx"]);
        s.set_sort(SortKey::Modified);
        assert!(!s.ascending, "modified starts newest first");
        assert_eq!(names(&s)[1], "report.docx");
    }

    #[test]
    fn grouping_adds_headers_and_follows_the_group_key() {
        let mut s = loaded();
        s.group = Some(SortKey::Modified);
        s.ascending = false;
        let rows = s.rows(NOW, &|_| 0);
        let heads: Vec<_> = rows
            .iter()
            .filter(|r| r.header)
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(
            heads,
            [
                "Today",
                "Yesterday",
                "Earlier this week",
                "Last week",
                "Older"
            ],
            "folders sort with files while grouped, so no group repeats"
        );
        // Item rows are numbered without the headers.
        let idx: Vec<_> = rows.iter().filter(|r| !r.header).map(|r| r.index).collect();
        assert_eq!(idx, (0..6).collect::<Vec<_>>());
    }

    #[test]
    fn grouping_uses_local_days_not_utc_days() {
        // 02:30 local at UTC+2 is 00:30 UTC. A file from 45 minutes earlier is from today in
        // local time, but from "yesterday" by UTC days.
        let utc_midnight = NOW.div_euclid(DAY_MS) * DAY_MS;
        let now = utc_midnight + 30 * 60_000;
        let mut f = file("1", "fresh.txt", 1, 0, "me");
        f.modified_ms = now - 45 * 60_000;
        let mut s = State::default();
        let g = s.go(Section::MyFiles);
        s.set_listing(g, vec![f], None);
        s.group = Some(SortKey::Modified);
        let plus2 = |_: i64| 2 * 3_600_000;
        let rows = s.rows(now, &plus2);
        assert_eq!(rows[0].name, "Today");
        assert_eq!(rows[1].name, "fresh.txt");
    }

    #[test]
    fn group_by_name_uses_the_first_letter() {
        let mut s = loaded();
        s.group = Some(SortKey::Name);
        let rows = s.rows(NOW, &|_| 0);
        let heads: Vec<_> = rows
            .iter()
            .filter(|r| r.header)
            .map(|r| r.name.clone())
            .collect();
        assert_eq!(heads, ["B", "D", "N", "P", "R", "S"]);
    }

    #[test]
    fn type_pills_and_the_name_box_filter_the_list() {
        let mut s = loaded();
        s.type_filter = TypeFilter::Word;
        assert_eq!(names(&s), ["report.docx"]);
        s.type_filter = TypeFilter::More;
        assert_eq!(
            names(&s),
            ["notes.md"],
            "More is everything but the named types and folders"
        );
        s.type_filter = TypeFilter::All;
        s.filter = "gra".into();
        assert_eq!(
            names(&s),
            ["budget.xlsx", "notes.md"],
            "matches the person too"
        );
        s.filter = "PROJ".into();
        assert_eq!(names(&s), ["Projects"]);
        s.filter = "zzz".into();
        assert!(names(&s).is_empty());
    }

    #[test]
    fn selection_click_toggle_all_and_prune() {
        let mut s = loaded();
        s.select_only("1");
        s.select_only("2");
        assert_eq!(s.selected, ["2"]);
        s.toggle("4");
        s.toggle("2");
        assert_eq!(s.selected, ["4"]);
        s.toggle("nope");
        assert_eq!(s.selected, ["4"]);
        s.select_all();
        assert_eq!(s.selected.len(), 6);
        s.filter = "report".into();
        s.select_all();
        assert_eq!(s.selected, ["1"], "select all respects the filter");
        let rows = s.rows(NOW, &|_| 0);
        assert!(rows[0].selected);
        s.clear_selection();
        assert_eq!(s.selection_info(), SelectionInfo::default());
        // Navigation clears the selection.
        s.select_only("1");
        s.open_folder(&folder("3", "Projects"));
        assert!(s.selected.is_empty());
    }

    #[test]
    fn selection_info_describes_one_or_many() {
        let mut s = loaded();
        s.select_only("3");
        let one = s.selection_info();
        assert_eq!(
            (one.count, one.name.as_str(), one.has_folder),
            (1, "Projects", true)
        );
        s.favorites.insert("3".into());
        assert!(s.selection_info().all_favorite);
        s.toggle("1");
        let two = s.selection_info();
        assert_eq!(two.count, 2);
        assert!(two.name.is_empty() && !two.all_favorite);
    }

    #[test]
    fn rows_carry_the_columns_the_list_shows() {
        let mut s = loaded();
        s.favorites.insert("1".into());
        let rows = s.rows(NOW, &|_| 0);
        let r = rows.iter().find(|r| r.id == "1").unwrap();
        assert_eq!(r.kind, "word");
        assert_eq!(r.size, "20 KB");
        assert_eq!(r.modified_by, "Ada");
        assert_eq!(r.activity, "Ada modified this");
        assert!(r.favorite);
        let f = rows.iter().find(|r| r.id == "3").unwrap();
        assert_eq!((f.kind.as_str(), f.size.as_str()), ("folder", "3 items"));
    }

    #[test]
    fn column_toggles_flip_one_column() {
        let mut c = Columns::default();
        assert!(c.modified && !c.location);
        c.toggle("location");
        c.toggle("modified");
        c.toggle("bogus");
        assert!(c.location && !c.modified && c.size && c.modified_by && !c.activity);
    }

    #[test]
    fn the_recycle_bin_lists_the_newest_deletion_first() {
        let mut s = State::default();
        let g = s.go(Section::Bin);
        s.set_listing(
            g,
            vec![
                file("a", "old.txt", 1, 5, "Ada"),
                file("b", "new.txt", 1, 1, "Ada"),
            ],
            None,
        );
        assert_eq!(names(&s), ["new.txt", "old.txt"]);
        assert_eq!(s.rows(NOW, &|_| 0)[0].activity, "Deleted by Ada");
    }

    #[test]
    fn the_recycle_bin_always_shows_the_original_location() {
        let mut s = State::default();
        s.go(Section::Bin);
        assert!(s.data(NOW, &|_| 0).columns.location);
        s.go(Section::Shared);
        assert!(!s.data(NOW, &|_| 0).columns.location);
    }

    #[test]
    fn data_summarizes_the_view() {
        let mut s = loaded();
        s.libraries = vec![Library {
            drive_id: "b!9".into(),
            name: "Design".into(),
        }];
        s.grid = true;
        s.group = Some(SortKey::Size);
        let d = s.data(NOW, &|_| 0);
        assert_eq!(
            (d.section.as_str(), d.title.as_str()),
            ("files", "My files")
        );
        assert!(d.grid && d.group == "size" && d.sort == "name");
        assert_eq!(d.quick, [("drive:b!9".to_string(), "Design".to_string())]);
        assert_eq!(d.rows.iter().filter(|r| !r.header).count(), 6);
    }
    #[test]
    fn nothing_in_the_people_list_can_be_selected() {
        let mut s = State::default();
        let g = s.go(Section::People);
        let person = DriveItem {
            id: "person:0".into(),
            name: "Ada".into(),
            is_folder: true,
            ..Default::default()
        };
        s.set_listing(g, vec![person], None);
        s.select_only("person:0");
        s.toggle("person:0");
        s.select_all();
        assert!(s.selected.is_empty());
        // The same list in a file section selects normally.
        let g = s.go(Section::Shared);
        s.set_listing(g, vec![file("1", "a.docx", 1, 0, "Ada")], None);
        s.select_all();
        assert_eq!(s.selected, ["1"]);
    }

    #[test]
    fn cards_show_on_home_only_and_not_while_filtering() {
        let mut s = State {
            cards: vec![Card {
                id: "card:d/1".into(),
                item: file("1", "plan.xlsx", 1, 0, "Ada"),
                reason: "You frequently open this".into(),
            }],
            ..State::default()
        };
        assert_eq!(s.data(NOW, &|_| 0).cards.len(), 1);
        assert_eq!(s.data(NOW, &|_| 0).cards[0].kind, "excel");
        s.filter = "pl".into();
        assert!(s.data(NOW, &|_| 0).cards.is_empty());
        s.filter.clear();
        s.go(Section::MyFiles);
        assert!(s.data(NOW, &|_| 0).cards.is_empty());
    }

    #[test]
    fn meetings_and_the_bin_always_show_the_location_column() {
        let mut s = State::default();
        assert!(!s.data(NOW, &|_| 0).columns.location);
        s.go(Section::Meetings);
        assert!(s.data(NOW, &|_| 0).columns.location);
        s.go(Section::Bin);
        assert!(s.data(NOW, &|_| 0).columns.location);
    }

    #[test]
    fn a_person_shows_people_then_their_name_in_the_breadcrumbs() {
        let mut s = State::default();
        s.go(Section::Person("Ada".into()));
        assert_eq!(s.crumbs(), ["People", "Ada"]);
        assert_eq!(s.section.title(&[]), "Ada");
    }
}
