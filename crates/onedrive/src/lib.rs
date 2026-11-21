//! OneDrive and SharePoint files for ironteams: browse, act, share and the recycle bin.
//!
//! Everything is an async function on `&teams_proto::Client`. Graph calls carry the Graph
//! audience's bearer and the recycle bin carries the user's `-my.sharepoint.com` bearer, both
//! through the auth broker, so a token can only reach the host it was minted for. The request
//! builders and response parsers are pure functions with their own tests.

pub mod api;
pub mod around;
pub mod bin;
pub mod browse;
pub mod create;
pub mod insights;
pub mod model;
pub mod ops;
pub mod package;
pub mod share;

pub use api::ItemRef;
pub use around::{Recording, channel_recordings, meeting_recordings, recordings, sync_help};
pub use bin::{BinItem, delete_forever, recycle_bin, restore};
pub use browse::{
    children, followed_sites, following, group_drive, is_media, item_at, item_from_share, media,
    my_drive, recent, root_children, search, search_from, shared_with_me, site_drives, thumbnails,
};
pub use create::{Created, FORMS_NEW_URL, NewItem, Notebook, Planned, create, plan};
pub use insights::{
    ForYouCard, Insight, InsightKind, Person, PersonFiles, for_you, people, shared_by_me,
};
pub use model::{
    Drive, DriveItem, Group, Page, SearchPage, Site, SortKey, Thumbnails, group_items, sort_items,
};
pub use ops::{
    FolderUpload, Version, add_shortcut, copy_to, create_folder, create_office_doc, delete,
    download, follow, move_to, preview_url, rename, restore_version, unfollow, upload_file,
    upload_folder, versions,
};
pub use package::DocKind;
pub use share::{
    LinkScope, LinkType, Permission, Recipient, Role, create_link, invite, permissions,
    remove_permission, update_permission,
};
