//! Plain view data of the Teams management UI, built by the backend and turned into the
//! `TeamsStore` global on the UI thread.

pub use super::view::TeamRowData;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiscoverData {
    pub group: String,
    pub name: String,
    pub description: String,
    pub people: u32,
    pub access: String,
    pub requested: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PersonData {
    pub mri: String,
    pub name: String,
    pub detail: String,
    pub role: &'static str,
    pub me: bool,
    pub picked: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TagData {
    pub id: String,
    pub name: String,
    pub description: String,
    pub count: u32,
    pub mine: bool,
    pub selected: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeletedData {
    pub id: String,
    pub name: String,
    pub when: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingData {
    pub key: String,
    pub group: &'static str,
    pub label: &'static str,
    pub on: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommunityData {
    pub id: String,
    pub name: String,
    pub description: String,
    pub people: u32,
    pub company: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PostData {
    pub id: String,
    pub sender: String,
    pub time: String,
    pub text: String,
    pub likes: u32,
    pub reply: bool,
    pub url: String,
}

/// What the dialogs show. `kind` is empty while no dialog is open.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DialogData {
    pub kind: String,
    pub busy: bool,
    pub error: String,
    pub team: String,
    pub team_name: String,
    pub channel: String,
    pub channel_name: String,
    pub channel_kind: String,
    pub name: String,
    pub description: String,
    pub access: String,
    pub owner: bool,
    pub people: Vec<PersonData>,
    pub candidates: Vec<PersonData>,
    pub tags: Vec<TagData>,
    pub tag_people: Vec<PersonData>,
    pub deleted: Vec<DeletedData>,
    pub settings: Vec<SettingData>,
    pub email: String,
    pub email_senders: String,
    pub post_perm: String,
    pub reply_perm: String,
    pub pin_perm: String,
    pub allow_bots: bool,
    pub allow_connectors: bool,
    pub notify_level: String,
    pub notify_replies: bool,
    pub team_code: String,
}

#[derive(Clone, Debug, Default)]
pub struct TeamsData {
    pub teams: Vec<TeamRowData>,
    pub stats: Vec<(String, String)>,
    pub loading: bool,
    pub error: String,
    pub notice: String,
    pub discover: Vec<DiscoverData>,
    pub requests: Vec<DiscoverData>,
    pub join_info: String,
    pub communities: Vec<CommunityData>,
    pub community_name: String,
    pub posts: Vec<PostData>,
    pub posts_more: bool,
    pub dialog: DialogData,
}
