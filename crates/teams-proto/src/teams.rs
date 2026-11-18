//! Teams and channels: the list, discovery and joining, channel and team management, members,
//! visibility, pinning, notifications and the channel email address.
//!
//! Endpoints are in `docs/protocol.md` ("Teams and channels"). The shapes come from the public
//! JavaScript of the Teams web client (`services-teams-and-channels`), from the open-source
//! clients in `docs/research/open-source.md` §3.18, and from live calls against a throwaway test
//! team. Everything here is internal and can change without notice.

use crate::broker::Audience;
use crate::client::Client;
use crate::error::{Error, Result};
use crate::people::anchor_mailbox;
use crate::time::{now_ms, parse_iso_ms};
use reqwest::Method;
use serde_json::{Value, json};

/// CSA v3: teams, channels, folders and chats in one call.
pub const CSA_V3_ME: &str = "https://teams.microsoft.com/api/csa/api/v3/teams/users/me?isPrefetch=false&enableMembershipSummary=true&supportsAdditionalSystemGeneratedFolders=true&supportsSliceItems=true&enableEngageCommunities=false";
/// CSA v1 base.
pub const CSA_V1: &str = "https://teams.microsoft.com/api/csa/api/v1";

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn text(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn flag(v: &Value, k: &str) -> bool {
    match v.get(k) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

// ---- model ----------------------------------------------------------------------------------

/// How a channel is shared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Standard,
    Private,
    Shared,
}

impl ChannelKind {
    /// The wire name used by the channel create calls.
    pub fn wire(self) -> &'static str {
        match self {
            ChannelKind::Standard => "Standard",
            ChannelKind::Private => "Private",
            ChannelKind::Shared => "Shared",
        }
    }

    fn from_value(ch: &Value) -> Self {
        if flag(ch, "isShared") {
            return ChannelKind::Shared;
        }
        match ch.get("channelType") {
            Some(Value::String(s)) => match s.to_ascii_lowercase().as_str() {
                "private" => ChannelKind::Private,
                "shared" => ChannelKind::Shared,
                _ => ChannelKind::Standard,
            },
            Some(Value::Number(n)) => match n.as_i64() {
                Some(1) => ChannelKind::Private,
                Some(2) => ChannelKind::Shared,
                _ => ChannelKind::Standard,
            },
            _ => ChannelKind::Standard,
        }
    }
}

/// A member's rank in a team or channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Owner,
    Member,
    Guest,
}

impl Role {
    /// From the chat service role names (`Admin`, `User`, `Guest`).
    pub fn from_name(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "admin" | "owner" => Role::Owner,
            "guest" => Role::Guest,
            _ => Role::Member,
        }
    }

    /// The number `bulkUpdateRoledMembers` takes (member 0, guest 1, owner 2).
    pub fn team_code(self) -> u8 {
        match self {
            Role::Member => 0,
            Role::Guest => 1,
            Role::Owner => 2,
        }
    }

    /// The number the provisioning service takes for private channel members (member 1,
    /// owner 2).
    pub fn channel_code(self) -> u8 {
        if self == Role::Owner { 2 } else { 1 }
    }
}

/// Who may find and join a team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Private,
    Public,
    OrgWide,
}

impl Access {
    /// The middle-tier `accessType` number (None 0, Private 1, Secret 2, Public 3).
    pub fn code(self) -> u8 {
        match self {
            Access::Private | Access::OrgWide => 1,
            Access::Public => 3,
        }
    }

    fn from_value(v: Option<&Value>) -> Self {
        match v {
            Some(Value::Number(n)) if n.as_i64() == Some(3) => Access::Public,
            Some(Value::String(s)) if s.eq_ignore_ascii_case("public") => Access::Public,
            _ => Access::Private,
        }
    }
}

/// A channel in the list.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelEntry {
    pub id: String,
    pub name: String,
    pub kind: ChannelKind,
    pub is_general: bool,
    /// Shown in the channel list. `false` means the user hid it.
    pub is_shown: bool,
    pub is_pinned: bool,
    pub is_muted: bool,
    pub is_archived: bool,
    pub is_deleted: bool,
    pub is_member: bool,
    pub is_moderator: bool,
    pub role: Role,
    pub created_ms: i64,
}

/// A team in the list.
#[derive(Debug, Clone, PartialEq)]
pub struct TeamEntry {
    /// The team thread id (`19:…@thread.tacv2`), also the id of its General channel.
    pub id: String,
    /// The AAD group id the Graph and the provisioning service key the team by.
    pub group_id: String,
    pub name: String,
    pub description: String,
    pub access: Access,
    pub is_org_wide: bool,
    pub is_shown: bool,
    pub is_collapsed: bool,
    pub is_archived: bool,
    pub is_deleted: bool,
    pub role: Role,
    /// People, not counting the bots Teams adds to every team.
    pub member_count: u32,
    pub owner_count: u32,
    pub email: String,
    pub channels: Vec<ChannelEntry>,
}

impl TeamEntry {
    /// The ids the team calls need.
    pub fn team_ref(&self) -> TeamRef {
        TeamRef {
            thread_id: self.id.clone(),
            group_id: self.group_id.clone(),
        }
    }
}

/// The ids a team call needs: the team thread id and its group id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamRef {
    pub thread_id: String,
    pub group_id: String,
}

/// What a channel call needs to pick the right service: standard channels go to the middle
/// tier, private and shared ones to the provisioning service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRef {
    pub id: String,
    pub kind: ChannelKind,
}

impl ChannelRef {
    pub fn from_entry(c: &ChannelEntry) -> Self {
        ChannelRef {
            id: c.id.clone(),
            kind: c.kind,
        }
    }
}

fn parse_channel(ch: &Value) -> Option<ChannelEntry> {
    let id = ch.get("id").and_then(Value::as_str)?.to_string();
    Some(ChannelEntry {
        name: text(ch, "displayName"),
        kind: ChannelKind::from_value(ch),
        is_general: flag(ch, "isGeneral"),
        is_shown: flag(ch, "isFavorite"),
        is_pinned: flag(ch, "isPinned"),
        is_muted: flag(ch, "isMuted"),
        is_archived: flag(ch, "isArchived"),
        is_deleted: flag(ch, "isDeleted"),
        is_member: flag(ch, "isMember"),
        is_moderator: flag(ch, "isModerator"),
        role: Role::from_name(&text(ch, "memberRoleString")),
        created_ms: parse_iso_ms(&text(ch, "creationTime")).unwrap_or_default(),
        id,
    })
}

/// Parses the CSA v3 `teams[]` (archived and deleted teams are kept, flagged).
pub fn parse_teams(v: &Value) -> Vec<TeamEntry> {
    v.get("teams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let id = t.get("id").and_then(Value::as_str)?.to_string();
            let count = |k: &str| {
                t.pointer(&format!("/membershipSummary/{k}"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32
            };
            Some(TeamEntry {
                group_id: t
                    .pointer("/teamSiteInformation/groupId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                name: text(t, "displayName"),
                description: text(t, "description"),
                access: if flag(t, "isTenantWide") {
                    Access::OrgWide
                } else {
                    Access::from_value(t.get("accessType"))
                },
                is_org_wide: flag(t, "isTenantWide"),
                is_shown: flag(t, "isFavorite"),
                is_collapsed: flag(t, "isCollapsed"),
                is_archived: flag(t, "isArchived"),
                is_deleted: flag(t, "isDeleted"),
                role: Role::from_name(&text(t, "memberRoleString")),
                member_count: count("totalMemberCount").saturating_sub(count("botCount")),
                owner_count: count("adminRoleCount"),
                email: text(t, "smtpAddress"),
                channels: t
                    .get("channels")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(parse_channel)
                    .collect(),
                id,
            })
        })
        .collect()
}

/// Finds a channel (and its team) by channel id.
pub fn find_channel<'a>(
    teams: &'a [TeamEntry],
    channel_id: &str,
) -> Option<(&'a TeamEntry, &'a ChannelEntry)> {
    teams.iter().find_map(|t| {
        t.channels
            .iter()
            .find(|c| c.id == channel_id)
            .map(|c| (t, c))
    })
}

// ---- request bodies -------------------------------------------------------------------------

/// Body of `POST beta/teams/create` for a team from scratch.
pub fn create_team_body(name: &str, description: &str, access: Access) -> Value {
    json!({
        "displayName": name,
        "description": description,
        "accessType": access.code(),
        "isTenantWide": access == Access::OrgWide,
        "validationRequired": false,
    })
}

/// Body of `POST beta/teams/validateNaming`.
pub fn validate_naming_body(name: &str) -> Value {
    json!({
        "displayName": name,
        "displayNamePrefix": null,
        "displayNameSuffix": null,
        "mailNicknamePrefix": null,
        "mailNicknameSuffix": null,
        "groupId": null,
    })
}

/// Body of `PATCH beta/teams/<t>`: only the fields that change.
pub fn edit_team_body(
    name: Option<&str>,
    description: Option<&str>,
    discoverable: Option<bool>,
) -> Value {
    let mut b = json!({});
    if let Some(n) = name {
        b["displayName"] = Value::from(n);
    }
    if let Some(d) = description {
        b["description"] = Value::from(d);
    }
    if let Some(d) = discoverable {
        b["isDiscoverable"] = Value::from(d);
    }
    b
}

/// Body of `PATCH beta/teams/<t>/teamSettings`. `user` and `guest` are partial settings objects
/// such as `{"createTopic": false}`; the service merges them into the stored settings, and the
/// whole thing travels as a JSON string.
pub fn team_settings_body(user: Option<&Value>, guest: Option<&Value>) -> Value {
    let mut s = json!({});
    if let Some(u) = user {
        s["userSettings"] = u.clone();
    }
    if let Some(g) = guest {
        s["guestSettings"] = g.clone();
    }
    json!({ "spaceAdminSettings": s.to_string() })
}

/// Body of `POST beta/teams/<t>/channels` for a standard channel. `threaded` picks the
/// "threaded chat" layout instead of posts.
pub fn standard_channel_body(
    group_id: &str,
    name: &str,
    description: &str,
    threaded: bool,
) -> Value {
    let mut b = json!({
        "displayName": name,
        "description": description,
        "groupId": group_id,
        "channelType": "Standard",
    });
    if threaded {
        b["chatModality"] = Value::from("Conversational");
    }
    b
}

/// Body of the provisioning-service call that creates a private or shared channel.
pub fn private_channel_body(
    team_thread_id: &str,
    name: &str,
    description: &str,
    kind: ChannelKind,
) -> Value {
    json!({
        "spaceThreadId": team_thread_id,
        "displayName": name,
        "description": description,
        "channelType": kind.wire(),
        "sharedWithParentTeam": false,
    })
}

/// Body of `PATCH beta/teams/<t>/channels/<c>`: only the fields that change.
pub fn edit_channel_body(name: Option<&str>, description: Option<&str>) -> Value {
    let mut b = json!({});
    if let Some(n) = name {
        b["displayName"] = Value::from(n);
    }
    if let Some(d) = description {
        b["description"] = Value::from(d);
    }
    b
}

/// Who may start posts, reply or pin in a channel, and whether bots and connectors may post.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moderation {
    /// `Everyone` or `Moderators`.
    pub post_permissions: String,
    pub reply_permissions: String,
    pub pin_permissions: String,
    pub allow_bots: bool,
    pub allow_connectors: bool,
}

impl Default for Moderation {
    fn default() -> Self {
        Moderation {
            post_permissions: "Everyone".into(),
            reply_permissions: "Everyone".into(),
            pin_permissions: "Everyone".into(),
            allow_bots: true,
            allow_connectors: true,
        }
    }
}

/// The `channelSettings` thread property value: a JSON string.
pub fn moderation_value(m: &Moderation) -> String {
    json!({
        "channelPostPermissions": m.post_permissions,
        "allowReplies": m.reply_permissions,
        "allowPinPosts": m.pin_permissions,
        "allowBotsPost": m.allow_bots,
        "allowConnectorsPost": m.allow_connectors,
    })
    .to_string()
}

/// Reads the moderation settings from `GET /v1/threads/<channel>`; `None` when the channel has
/// never been moderated (everything is then open to everyone).
pub fn parse_moderation(thread: &Value) -> Option<Moderation> {
    let raw = thread.pointer("/properties/channelSettings")?.as_str()?;
    let s: Value = serde_json::from_str(raw).ok()?;
    let d = Moderation::default();
    let pick = |k: &str, dflt: &str| s.get(k).and_then(Value::as_str).unwrap_or(dflt).to_string();
    Some(Moderation {
        post_permissions: pick("channelPostPermissions", &d.post_permissions),
        reply_permissions: pick("allowReplies", &d.reply_permissions),
        pin_permissions: pick("allowPinPosts", &d.pin_permissions),
        allow_bots: s
            .get("allowBotsPost")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        allow_connectors: s
            .get("allowConnectorsPost")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

/// Body of `PUT beta/teams/<t>/bulkUpdateRoledMembers`.
pub fn bulk_roles_body(group_id: &str, members: &[(String, Role)]) -> Value {
    json!({
        "users": members
            .iter()
            .map(|(mri, role)| json!({"mri": mri, "role": role.team_code()}))
            .collect::<Vec<_>>(),
        "groupId": group_id,
    })
}

/// Body of `PUT beta/teams/<t>/members` that removes (`Left`) a person from a team.
pub fn leave_body(group_id: &str, mri: &str) -> Value {
    json!({"groupId": group_id, "userMri": mri, "updateType": "Left"})
}

/// `OID:<oid>@<tenant>`: how the provisioning service names people and private channels.
pub fn oid_ref(oid: &str, tenant_id: &str) -> String {
    format!("OID:{oid}@{tenant_id}")
}

/// Body of the provisioning call that adds people to a private channel or changes their role.
pub fn private_members_body(tenant_id: &str, members: &[(String, Role)]) -> Value {
    json!({"users": members
        .iter()
        .map(|(mri, role)| json!({
            "id": oid_ref(mri.trim_start_matches("8:orgid:"), tenant_id),
            "role": role.channel_code(),
        }))
        .collect::<Vec<_>>()})
}

/// The `PUT /v1/threads/<channel>/members/<me>` body that hides (`interest` `NotInterested`,
/// for 14 days) or shows a standard channel again.
pub fn interest_body(shown: bool, now_ms: i64) -> Value {
    if shown {
        json!({"expiration": 0, "interest": "Interested"})
    } else {
        json!({"expiration": now_ms / 1000 + 14 * 86_400, "interest": "NotInterested"})
    }
}

/// Body of `POST /teams/users/me/pinnedChannels` (pin) and `DELETE` (unpin).
pub fn pinned_body(channel_id: &str, pin: bool) -> Value {
    if pin {
        json!({"newlyPinnedChannels": [channel_id]})
    } else {
        json!({"channelIdsToUnpin": [channel_id]})
    }
}

// ---- results --------------------------------------------------------------------------------

/// A team from "discover" (not joined yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoverTeam {
    pub group_id: String,
    pub name: String,
    pub description: String,
    pub access: String,
    pub members: u32,
}

/// Parses `beta/teams/suggested` and `beta/teams/find`.
pub fn parse_discover(v: &Value) -> Vec<DiscoverTeam> {
    v.get("value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            Some(DiscoverTeam {
                group_id: t.get("groupId").and_then(Value::as_str)?.to_string(),
                name: text(t, "displayName"),
                description: text(t, "description"),
                access: text(t, "accessType"),
                members: t.get("membersCount").and_then(Value::as_u64).unwrap_or(0) as u32,
            })
        })
        .collect()
}

/// What joining by code says about the team before or after joining.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinInfo {
    pub name: String,
    pub description: String,
    pub access: String,
    /// The owners must approve the request.
    pub needs_approval: bool,
}

/// Parses the `JoinTeamInfo` of `beta/teams/joinByCode`.
pub fn parse_join_info(v: &Value) -> Option<JoinInfo> {
    let t = v.get("value")?;
    Some(JoinInfo {
        name: text(t, "displayName"),
        description: text(t, "description"),
        access: text(t, "accessType"),
        needs_approval: flag(t, "isTeamOwnerApprovalRequired"),
    })
}

/// The ids of a team that was just created. The web client reads `value.siteInfo.groupId` and
/// `value.skypeThreadId`.
pub fn parse_created_team(v: &Value) -> Option<TeamRef> {
    let value = v.get("value").unwrap_or(v);
    let group = value
        .pointer("/siteInfo/groupId")
        .or_else(|| value.get("groupId"))
        .and_then(Value::as_str)?;
    let thread = value
        .get("skypeThreadId")
        .or_else(|| value.get("threadId"))
        .and_then(Value::as_str)?;
    Some(TeamRef {
        thread_id: thread.to_string(),
        group_id: group.to_string(),
    })
}

/// The id of a channel that was just created by the middle tier (`value.objectId`).
pub fn parse_created_channel(v: &Value) -> Option<String> {
    let value = v.get("value").unwrap_or(v);
    ["objectId", "id", "threadId"]
        .iter()
        .find_map(|k| value.get(*k).and_then(Value::as_str))
        .map(str::to_owned)
}

/// The substrate group guid of a channel the provisioning service just created. The answer
/// carries `groupId` as `OID:<guid>@<tenant>`; the thread id is not in it.
pub fn parse_created_private(v: &Value) -> Option<String> {
    let g = v.get("groupId")?.as_str()?;
    let g = g.strip_prefix("OID:").unwrap_or(g);
    Some(g.split('@').next()?.to_string())
}

/// A deleted channel that can still be restored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedChannel {
    pub id: String,
    pub name: String,
    pub description: String,
    pub deleted_ms: i64,
}

/// Parses the answer of `PUT beta/teams/<t>/allchannels` (a bare array).
pub fn parse_channels_in_state(v: &Value) -> Vec<DeletedChannel> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some(DeletedChannel {
                id: c.get("objectId").and_then(Value::as_str)?.to_string(),
                name: text(c, "displayName"),
                description: text(c, "description"),
                deleted_ms: c
                    .get("deleteTime")
                    .and_then(|t| {
                        t.as_str()
                            .and_then(|s| s.parse().ok())
                            .or_else(|| t.as_i64())
                    })
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// A person in a team or channel roster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterMember {
    pub mri: String,
    pub role: Role,
    /// A bot (`28:` mri). Every team has a few.
    pub is_bot: bool,
}

/// Parses `GET /v1/threads/<id>/members`.
pub fn parse_members(v: &Value) -> Vec<RosterMember> {
    v.get("members")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let mri = m.get("id").and_then(Value::as_str)?.to_string();
            Some(RosterMember {
                role: Role::from_name(&text(m, "role")),
                is_bot: mri.starts_with("28:"),
                mri,
            })
        })
        .collect()
}

/// Who may send mail to a channel's address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailSenders {
    Anyone,
    Members,
    Domains,
}

impl EmailSenders {
    fn wire(self) -> &'static str {
        match self {
            EmailSenders::Anyone => "Anyone",
            EmailSenders::Members => "Members",
            EmailSenders::Domains => "Domains",
        }
    }

    fn from_wire(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "members" => EmailSenders::Members,
            "domains" => EmailSenders::Domains,
            _ => EmailSenders::Anyone,
        }
    }
}

/// A channel's email address and who may write to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelEmail {
    pub address: String,
    pub senders: EmailSenders,
    pub domains: Vec<String>,
}

/// Parses `GET beta/channels/<c>/email`.
pub fn parse_channel_email(v: &Value) -> Option<ChannelEmail> {
    let address = v
        .pointer("/emailAddressDetails/emailId")?
        .as_str()?
        .to_string();
    let allowed = v.get("allowedSenders");
    Some(ChannelEmail {
        address,
        senders: EmailSenders::from_wire(
            allowed
                .and_then(|a| a.get("allowedSenderType"))
                .and_then(Value::as_str)
                .unwrap_or("Anyone"),
        ),
        domains: allowed
            .and_then(|a| a.get("allowedDomains"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|d| d.as_str().map(str::to_owned))
            .collect(),
    })
}

/// Body of `POST` and `PUT beta/channels/<c>/email`.
pub fn channel_email_body(senders: EmailSenders, domains: &[String]) -> Value {
    let mut b = json!({"allowedSenderType": senders.wire()});
    if senders == EmailSenders::Domains {
        b["allowedDomains"] = json!(domains);
    }
    b
}

/// How loudly a channel notifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyLevel {
    /// A banner and an Activity item for every new post.
    BannerAndFeed,
    /// An Activity item only (the default for channels that are not followed).
    Feed,
    Off,
}

/// A channel's notification setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelNotify {
    pub level: NotifyLevel,
    pub include_replies: bool,
}

impl Default for ChannelNotify {
    fn default() -> Self {
        ChannelNotify {
            level: NotifyLevel::Feed,
            include_replies: false,
        }
    }
}

/// The `channelNotificationSettings` property value: a JSON string.
pub fn notify_value(n: ChannelNotify) -> String {
    let mut s = match n.level {
        NotifyLevel::BannerAndFeed => json!({"allNewPosts": "On", "dskNotif": "On"}),
        NotifyLevel::Feed => json!({"allNewPosts": "On"}),
        NotifyLevel::Off => json!({"allNewPosts": "Off"}),
    };
    s["includeReplies"] = Value::from(n.include_replies && n.level != NotifyLevel::Off);
    s.to_string()
}

/// Reads the `channelNotificationSettings` property back from the conversation properties.
pub fn parse_notify(conversation: &Value) -> Option<ChannelNotify> {
    let raw = conversation
        .pointer("/properties/channelNotificationSettings")?
        .as_str()?;
    let s: Value = serde_json::from_str(raw).ok()?;
    let level = match (
        s.get("allNewPosts").and_then(Value::as_str)?,
        s.get("dskNotif").and_then(Value::as_str),
    ) {
        ("On", Some("On")) => NotifyLevel::BannerAndFeed,
        ("On", _) => NotifyLevel::Feed,
        _ => NotifyLevel::Off,
    };
    Some(ChannelNotify {
        level,
        include_replies: s
            .get("includeReplies")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// The pinned channels and the version the next change must quote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedChannels {
    pub order_version: String,
    pub channel_ids: Vec<String>,
}

/// Parses `GET /teams/users/me/pinnedChannels`.
pub fn parse_pinned(v: &Value) -> Option<PinnedChannels> {
    let order_version = match v.get("orderVersion")? {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    Some(PinnedChannels {
        order_version,
        channel_ids: v
            .get("pinChannelOrder")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|c| c.as_str().map(str::to_owned))
            .collect(),
    })
}

// ---- client ---------------------------------------------------------------------------------

impl Client {
    async fn mt(&self, path: &str) -> String {
        format!("{}/beta/{path}", self.middle_tier().await)
    }

    /// The region in the middle-tier URL (`…/api/mt/emea` gives `emea`, `…/part/amer-02` gives
    /// `amer`), which the provisioning service keys on.
    pub async fn geo(&self) -> String {
        let mt = self.middle_tier().await;
        let last = mt
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("amer");
        last.split('-').next().unwrap_or("amer").to_string()
    }

    /// One middle-tier call. Returns the body text of a 2xx answer (some answers, such as the
    /// join code, are plain text); anything else is an error.
    async fn mt_text(
        &self,
        context: &'static str,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<String> {
        let url = self.mt(path).await;
        let (status, text) = self.request(method, &url, body, &[]).await?;
        if !(200..300).contains(&status) {
            return Err(Error::status(context, status, &text));
        }
        Ok(text)
    }

    /// [`Client::mt_text`] parsed as JSON (`Null` for an empty answer).
    async fn mt_json(
        &self,
        context: &'static str,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value> {
        let t = self.mt_text(context, method, path, body).await?;
        if t.trim().is_empty() {
            Ok(Value::Null)
        } else {
            Ok(serde_json::from_str(&t)?)
        }
    }

    /// The provisioning-service call (private channels, their members). It needs its own token
    /// audience and an `X-AnchorMailbox` header naming us.
    async fn provisioning(
        &self,
        context: &'static str,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value> {
        let me = self.self_mri().await?;
        let tid = self.tenant_id().await?;
        let anchor = anchor_mailbox(&me, &tid)
            .ok_or_else(|| Error::Protocol("self mri is not an orgid mri".into()))?
            .replacen("Oid:", "OID:", 1);
        let url = format!(
            "https://teams.microsoft.com/fabric/{}/templates/api/{path}",
            self.geo().await
        );
        let (status, body_text) = self
            .request_as(
                &Audience::Templates,
                method,
                &url,
                body,
                &[("X-AnchorMailbox", anchor.as_str())],
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(Error::status(context, status, &body_text));
        }
        if body_text.trim().is_empty() {
            Ok(Value::Null)
        } else {
            Ok(serde_json::from_str(&body_text)?)
        }
    }

    /// The path of a private channel on the provisioning service.
    async fn private_path(&self, team: &TeamRef, ch: &ChannelRef) -> Result<String> {
        let substrate = self.substrate_group_id(&ch.id).await?;
        let tid = self.tenant_id().await?;
        Ok(format!(
            "teams/{}/channels/{}",
            team.group_id,
            enc(&oid_ref(&substrate, &tid))
        ))
    }

    /// The substrate group id of a private or shared channel (a thread property).
    pub async fn substrate_group_id(&self, channel_id: &str) -> Result<String> {
        let url = format!(
            "{}/v1/threads/{}?view=msnp24Equivalent",
            self.chat_service().await,
            enc(channel_id)
        );
        let v = self.json("channel thread", Method::GET, &url, None).await?;
        v.pointer("/properties/substrateGroupId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| Error::Protocol("the channel has no substrate group id".into()))
    }

    /// `PUT` of one conversation property of ours (`favorite`, `channelNotificationSettings`).
    async fn put_my_property(&self, conversation: &str, name: &str, body: &Value) -> Result<()> {
        let url = format!(
            "{}/v1/users/ME/conversations/{}/properties?name={name}",
            self.chat_service().await,
            enc(conversation)
        );
        self.json("conversation property", Method::PUT, &url, Some(body))
            .await
            .map(|_| ())
    }

    /// `PUT` of one thread property (`archivalLevel`, `channelSettings`).
    async fn put_thread_property(&self, thread: &str, name: &str, body: &Value) -> Result<()> {
        let url = format!(
            "{}/v1/threads/{}/properties?name={name}",
            self.chat_service().await,
            enc(thread)
        );
        self.json("thread property", Method::PUT, &url, Some(body))
            .await
            .map(|_| ())
    }

    // ---- T1.1 list ----

    /// Every team and channel we belong to, with visibility, pin and mute state.
    pub async fn teams_and_channels(&self) -> Result<Vec<TeamEntry>> {
        let v = self.json("teams", Method::GET, CSA_V3_ME, None).await?;
        Ok(parse_teams(&v))
    }

    // ---- T1.2 discover and join ----

    /// Teams the service suggests (`Access::Public` for the public ones).
    pub async fn suggested_teams(&self, access: Access) -> Result<Vec<DiscoverTeam>> {
        let path = format!(
            "teams/suggested?accessType={}&enableFallback=true",
            access.code()
        );
        Ok(parse_discover(
            &self
                .mt_json("suggested teams", Method::GET, &path, None)
                .await?,
        ))
    }

    /// Searches teams by keyword.
    pub async fn find_teams(
        &self,
        keywords: &str,
        access: Access,
        skip_token: &str,
    ) -> Result<Vec<DiscoverTeam>> {
        let path = format!(
            "teams/find?keywords={}&accessType={}&enableKeywordSearch=false&top=100&skipToken={}",
            enc(keywords),
            access.code(),
            enc(skip_token)
        );
        Ok(parse_discover(
            &self.mt_json("find teams", Method::GET, &path, None).await?,
        ))
    }

    /// Joins a public team (or asks to join a private one) by its group id.
    pub async fn join_team(&self, group_id: &str) -> Result<Value> {
        self.mt_json(
            "join team",
            Method::POST,
            &format!("teams/joinRequest/{group_id}"),
            None,
        )
        .await
    }

    /// Withdraws a join request.
    pub async fn cancel_join_request(&self, group_id: &str) -> Result<()> {
        self.mt_json(
            "cancel join request",
            Method::POST,
            &format!("teams/cancelJoinRequest/{group_id}"),
            None,
        )
        .await
        .map(|_| ())
    }

    /// Joins by team code. The body is the code as a JSON string. `public_only` makes the
    /// service refuse non-public teams.
    pub async fn join_team_with_code(
        &self,
        code: &str,
        public_only: bool,
    ) -> Result<Option<JoinInfo>> {
        let path = format!("teams/joinByCode?restrictJoinToPublicTeam={public_only}");
        let v = self
            .mt_json(
                "join by code",
                Method::POST,
                &path,
                Some(&Value::from(code)),
            )
            .await?;
        Ok(parse_join_info(&v))
    }

    /// The team code, if one is set.
    pub async fn team_join_code(&self, team_thread_id: &str) -> Result<Option<String>> {
        let t = self
            .mt_text(
                "team code",
                Method::GET,
                &format!("teams/{}/joinCode", enc(team_thread_id)),
                None,
            )
            .await?;
        Ok(Some(t.trim().to_string()).filter(|s| !s.is_empty()))
    }

    /// Makes a new team code (replacing the old one) and returns it.
    pub async fn reset_team_join_code(&self, team_thread_id: &str) -> Result<String> {
        Ok(self
            .mt_text(
                "reset team code",
                Method::PUT,
                &format!("teams/{}/joinCode", enc(team_thread_id)),
                None,
            )
            .await?
            .trim()
            .to_string())
    }

    /// Turns the team code off.
    pub async fn remove_team_join_code(&self, team_thread_id: &str) -> Result<()> {
        self.mt_text(
            "remove team code",
            Method::DELETE,
            &format!("teams/{}/joinCode", enc(team_thread_id)),
            None,
        )
        .await
        .map(|_| ())
    }

    // ---- T1.3 show, hide, pin ----

    /// Shows or hides a channel in our list. A standard channel first needs its member
    /// `interest` set, and the General channel carries its own favorite flag.
    pub async fn set_channel_shown(
        &self,
        ch: &ChannelRef,
        is_general: bool,
        shown: bool,
    ) -> Result<()> {
        let flag_value = if shown { "true" } else { "false" };
        if ch.kind == ChannelKind::Standard {
            let me = self.self_mri().await?;
            let url = format!(
                "{}/v1/threads/{}/members/{}",
                self.chat_service().await,
                enc(&ch.id),
                enc(&me)
            );
            self.json(
                "channel interest",
                Method::PUT,
                &url,
                Some(&interest_body(shown, now_ms())),
            )
            .await?;
        }
        if is_general {
            self.put_my_property(
                &ch.id,
                "isGeneralChannelFavorite",
                &json!({"isGeneralChannelFavorite": flag_value}),
            )
            .await?;
        }
        self.put_my_property(&ch.id, "favorite", &json!({"favorite": flag_value}))
            .await
    }

    /// Makes a standard channel show up for every member automatically (`true`), or stops
    /// doing so. The call names the channel in the `teams` position of the URL.
    pub async fn set_channel_default_shown(&self, channel_id: &str, default: bool) -> Result<()> {
        let method = if default { Method::PUT } else { Method::DELETE };
        self.mt_json(
            "show channel for everyone",
            method,
            &format!("teams/{channel_id}/setDefaultChannel"),
            None,
        )
        .await
        .map(|_| ())
    }

    /// Shows or hides a whole team in our list.
    pub async fn set_team_shown(&self, team_thread_id: &str, shown: bool) -> Result<()> {
        let v = if shown { "true" } else { "false" };
        self.put_my_property(team_thread_id, "favorite", &json!({"favorite": v}))
            .await
    }

    /// The pinned channels and the version the next change must quote.
    pub async fn pinned_channels(&self) -> Result<PinnedChannels> {
        let v = self
            .json(
                "pinned channels",
                Method::GET,
                &format!("{CSA_V1}/teams/users/me/pinnedChannels"),
                None,
            )
            .await?;
        parse_pinned(&v).ok_or_else(|| Error::Protocol("pinned channels without a version".into()))
    }

    /// Pins (`true`) or unpins a channel. A stale version (412) is retried once.
    pub async fn set_channel_pinned(&self, channel_id: &str, pin: bool) -> Result<PinnedChannels> {
        let url = format!("{CSA_V1}/teams/users/me/pinnedChannels");
        let method = if pin { Method::POST } else { Method::DELETE };
        for attempt in 0..2 {
            let state = self.pinned_channels().await?;
            let (status, body) = self
                .request(
                    method.clone(),
                    &url,
                    Some(&pinned_body(channel_id, pin)),
                    &[("if-match", state.order_version.as_str())],
                )
                .await?;
            if (200..300).contains(&status) {
                return self.pinned_channels().await;
            }
            if status == 412 && attempt == 0 {
                continue;
            }
            return Err(Error::status("pin channel", status, &body));
        }
        unreachable!("the loop returns on its second pass")
    }

    // ---- T2.6 / T2.7 team ----

    /// Checks a team name without creating anything.
    pub async fn validate_team_name(&self, name: &str) -> Result<Value> {
        self.mt_json(
            "validate team name",
            Method::POST,
            "teams/validateNaming",
            Some(&validate_naming_body(name)),
        )
        .await
    }

    /// Creates a team from scratch and returns its ids.
    pub async fn create_team(
        &self,
        name: &str,
        description: &str,
        access: Access,
    ) -> Result<TeamRef> {
        let v = self
            .mt_json(
                "create team",
                Method::POST,
                "teams/create",
                Some(&create_team_body(name, description, access)),
            )
            .await?;
        parse_created_team(&v)
            .ok_or_else(|| Error::Protocol("team created but the answer has no ids".into()))
    }

    /// Renames a team and/or changes its description or discoverability.
    pub async fn edit_team(
        &self,
        team_thread_id: &str,
        name: Option<&str>,
        description: Option<&str>,
        discoverable: Option<bool>,
    ) -> Result<()> {
        self.mt_json(
            "edit team",
            Method::PATCH,
            &format!("teams/{}", enc(team_thread_id)),
            Some(&edit_team_body(name, description, discoverable)),
        )
        .await
        .map(|_| ())
    }

    /// Changes member or guest permissions (see [`team_settings_body`]).
    pub async fn set_team_settings(
        &self,
        team_thread_id: &str,
        user: Option<&Value>,
        guest: Option<&Value>,
    ) -> Result<()> {
        self.mt_json(
            "team settings",
            Method::PATCH,
            &format!("teams/{}/teamSettings", enc(team_thread_id)),
            Some(&team_settings_body(user, guest)),
        )
        .await
        .map(|_| ())
    }

    /// The team's stored settings (`{userSettings, guestSettings}`).
    pub async fn team_settings(&self, team_thread_id: &str) -> Result<Value> {
        let url = format!(
            "{}/v1/threads/{}?view=msnp24Equivalent",
            self.chat_service().await,
            enc(team_thread_id)
        );
        let v = self.json("team settings", Method::GET, &url, None).await?;
        v.pointer("/properties/spaceAdminSettings")
            .and_then(Value::as_str)
            .map(serde_json::from_str)
            .transpose()?
            .ok_or_else(|| Error::Protocol("team has no spaceAdminSettings".into()))
    }

    /// Archives a team. The middle tier answered 500 `ArchiveUnarchiveEnqueuePsOperationFailed`
    /// in the test tenant; see [`Client::set_archival`] for the chat-level flag.
    pub async fn archive_team(&self, team_thread_id: &str) -> Result<()> {
        self.mt_json(
            "archive team",
            Method::POST,
            &format!("teams/{}/archive", enc(team_thread_id)),
            None,
        )
        .await
        .map(|_| ())
    }

    /// Unarchives a team.
    pub async fn unarchive_team(&self, team_thread_id: &str) -> Result<()> {
        self.mt_json(
            "unarchive team",
            Method::POST,
            &format!("teams/{}/unarchive", enc(team_thread_id)),
            None,
        )
        .await
        .map(|_| ())
    }

    /// Deletes a team (the group goes to the Microsoft 365 recycle bin for 30 days).
    pub async fn delete_team(&self, team_thread_id: &str) -> Result<()> {
        self.mt_json(
            "delete team",
            Method::DELETE,
            &format!("teams/{}/delete", enc(team_thread_id)),
            None,
        )
        .await
        .map(|_| ())
    }

    // ---- T2.1 / T2.2 channels ----

    /// Creates a standard channel and returns its id.
    pub async fn create_standard_channel(
        &self,
        team: &TeamRef,
        name: &str,
        description: &str,
        threaded: bool,
    ) -> Result<String> {
        let v = self
            .mt_json(
                "create channel",
                Method::POST,
                &format!("teams/{}/channels", enc(&team.thread_id)),
                Some(&standard_channel_body(
                    &team.group_id,
                    name,
                    description,
                    threaded,
                )),
            )
            .await?;
        parse_created_channel(&v)
            .ok_or_else(|| Error::Protocol("channel created but the answer has no id".into()))
    }

    /// Creates a private (or shared) channel through the provisioning service and returns its
    /// thread id. The service does not answer with it, so the new channel is the one that shows
    /// up in the list with this name.
    pub async fn create_private_channel(
        &self,
        team: &TeamRef,
        name: &str,
        description: &str,
        kind: ChannelKind,
    ) -> Result<String> {
        let known: Vec<String> = self
            .teams_and_channels()
            .await?
            .iter()
            .flat_map(|t| t.channels.iter().map(|c| c.id.clone()))
            .collect();
        let v = self
            .provisioning(
                "create private channel",
                Method::POST,
                &format!("teams/{}/channels?forceSync=false", team.group_id),
                Some(&private_channel_body(
                    &team.thread_id,
                    name,
                    description,
                    kind,
                )),
            )
            .await?;
        parse_created_private(&v).ok_or_else(|| {
            Error::Protocol("channel created but the answer has no group id".into())
        })?;
        for _ in 0..15 {
            let teams = self.teams_and_channels().await?;
            if let Some(c) = teams.iter().flat_map(|t| &t.channels).find(|c| {
                c.kind != ChannelKind::Standard && c.name == name && !known.contains(&c.id)
            }) {
                return Ok(c.id.clone());
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        Err(Error::Protocol(
            "the new channel did not show up in the list".into(),
        ))
    }

    /// Renames a channel and/or changes its description.
    pub async fn edit_channel(
        &self,
        team: &TeamRef,
        ch: &ChannelRef,
        name: Option<&str>,
        description: Option<&str>,
    ) -> Result<()> {
        let body = edit_channel_body(name, description);
        if ch.kind == ChannelKind::Standard {
            let path = format!("teams/{}/channels/{}", enc(&team.thread_id), enc(&ch.id));
            self.mt_json("edit channel", Method::PATCH, &path, Some(&body))
                .await
                .map(|_| ())
        } else {
            let path = format!("{}?forceSync=false", self.private_path(team, ch).await?);
            self.provisioning("edit channel", Method::PATCH, &path, Some(&body))
                .await
                .map(|_| ())
        }
    }

    /// Deletes a channel. A standard channel stays restorable for 21 days.
    pub async fn delete_channel(&self, team: &TeamRef, ch: &ChannelRef) -> Result<()> {
        if ch.kind == ChannelKind::Standard {
            let path = format!("teams/{}/channels/{}", enc(&team.thread_id), enc(&ch.id));
            let body = json!({"id": ch.id, "hostTeamId": team.thread_id, "isGeneral": false});
            self.mt_json("delete channel", Method::DELETE, &path, Some(&body))
                .await
                .map(|_| ())
        } else {
            let path = format!("{}?forceSync=false", self.private_path(team, ch).await?);
            self.provisioning("delete channel", Method::DELETE, &path, None)
                .await
                .map(|_| ())
        }
    }

    /// Restores a deleted standard channel.
    pub async fn restore_channel(&self, team_thread_id: &str, channel_id: &str) -> Result<()> {
        self.mt_json(
            "restore channel",
            Method::PATCH,
            &format!(
                "teams/{}/channels/{}/restore",
                enc(team_thread_id),
                enc(channel_id)
            ),
            None,
        )
        .await
        .map(|_| ())
    }

    /// Deleted (`"deleted"`) or archived (`"archived"`) channels of a team.
    pub async fn channels_in_state(
        &self,
        team_thread_id: &str,
        state: &str,
    ) -> Result<Vec<DeletedChannel>> {
        let channels: Vec<Value> = ["standard", "private", "shared"]
            .iter()
            .map(|t| json!({"Type": t, "State": [state]}))
            .collect();
        let v = self
            .mt_json(
                "channels in state",
                Method::PUT,
                &format!("teams/{}/allchannels", enc(team_thread_id)),
                Some(&json!({ "Channels": channels })),
            )
            .await?;
        Ok(parse_channels_in_state(&v))
    }

    /// Archives or unarchives a channel (or a team) by its thread. The middle-tier
    /// `…/channels/<c>/archive` route answers 404, so this writes the `archivalLevel` thread
    /// property the list reads `isArchived` from: `Topic` for a channel, `Space` for a team,
    /// `None` to bring it back.
    pub async fn set_archival(&self, thread_id: &str, is_team: bool, archived: bool) -> Result<()> {
        let level = match (archived, is_team) {
            (false, _) => "None",
            (true, true) => "Space",
            (true, false) => "Topic",
        };
        self.put_thread_property(thread_id, "archivalLevel", &json!({"archivalLevel": level}))
            .await
    }

    /// Reads who may post, reply and pin in a channel.
    pub async fn moderation(&self, channel_id: &str) -> Result<Moderation> {
        let url = format!(
            "{}/v1/threads/{}?view=msnp24Equivalent",
            self.chat_service().await,
            enc(channel_id)
        );
        let v = self.json("moderation", Method::GET, &url, None).await?;
        Ok(parse_moderation(&v).unwrap_or_default())
    }

    /// Sets who may post, reply and pin in a channel.
    pub async fn set_moderation(&self, channel_id: &str, m: &Moderation) -> Result<()> {
        self.put_thread_property(
            channel_id,
            "channelSettings",
            &json!({"channelSettings": moderation_value(m)}),
        )
        .await
    }

    // ---- T2.3 members ----

    /// The roster of a team (use its thread id) or of a channel, bots included.
    pub async fn roster(&self, thread_id: &str) -> Result<Vec<RosterMember>> {
        let url = format!(
            "{}/v1/threads/{}/members?view=msnp24Equivalent&pageSize=100&selectMemberRoles=Admin|User|Guest",
            self.chat_service().await,
            enc(thread_id)
        );
        let v = self.json("roster", Method::GET, &url, None).await?;
        Ok(parse_members(&v))
    }

    /// Adds people to a team, or changes their role (the same call does both).
    pub async fn set_team_members(&self, team: &TeamRef, members: &[(String, Role)]) -> Result<()> {
        self.mt_json(
            "set team members",
            Method::PUT,
            &format!(
                "teams/{}/bulkUpdateRoledMembers?allowAsyncAddition=true",
                enc(&team.thread_id)
            ),
            Some(&bulk_roles_body(&team.group_id, members)),
        )
        .await
        .map(|_| ())
    }

    /// Removes a person from a team (or leaves it, for ourselves).
    pub async fn remove_team_member(&self, team: &TeamRef, mri: &str) -> Result<()> {
        self.mt_json(
            "remove team member",
            Method::PUT,
            &format!("teams/{}/members", enc(&team.thread_id)),
            Some(&leave_body(&team.group_id, mri)),
        )
        .await
        .map(|_| ())
    }

    /// Adds people to a private channel, or changes their role there.
    pub async fn set_private_channel_members(
        &self,
        team: &TeamRef,
        ch: &ChannelRef,
        members: &[(String, Role)],
    ) -> Result<()> {
        let tid = self.tenant_id().await?;
        let path = format!(
            "{}/users?forceSync=false",
            self.private_path(team, ch).await?
        );
        self.provisioning(
            "private channel members",
            Method::POST,
            &path,
            Some(&private_members_body(&tid, members)),
        )
        .await
        .map(|_| ())
    }

    /// Removes a person from a private channel.
    pub async fn remove_private_channel_member(
        &self,
        team: &TeamRef,
        ch: &ChannelRef,
        mri: &str,
    ) -> Result<()> {
        let tid = self.tenant_id().await?;
        let path = format!(
            "{}/users/{}?forceSync=false",
            self.private_path(team, ch).await?,
            enc(&oid_ref(mri.trim_start_matches("8:orgid:"), &tid))
        );
        self.provisioning("remove private channel member", Method::DELETE, &path, None)
            .await
            .map(|_| ())
    }

    // ---- T2.5 email ----

    /// The channel's email address, or `None` when it has none.
    pub async fn channel_email(&self, channel_id: &str) -> Result<Option<ChannelEmail>> {
        let path = format!("channels/{}/email", enc(channel_id));
        let url = self.mt(&path).await;
        let (status, body) = self.request(Method::GET, &url, None, &[]).await?;
        match status {
            404 => Ok(None),
            200..=299 => Ok(parse_channel_email(&serde_json::from_str(&body)?)),
            _ => Err(Error::status("channel email", status, &body)),
        }
    }

    /// Creates the channel's address, or changes who may write to it.
    pub async fn set_channel_email(
        &self,
        channel_id: &str,
        senders: EmailSenders,
        domains: &[String],
        exists: bool,
    ) -> Result<Option<ChannelEmail>> {
        let method = if exists { Method::PUT } else { Method::POST };
        let v = self
            .mt_json(
                "channel email",
                method,
                &format!("channels/{}/email", enc(channel_id)),
                Some(&channel_email_body(senders, domains)),
            )
            .await?;
        Ok(parse_channel_email(&v))
    }

    /// Removes the channel's address.
    pub async fn remove_channel_email(&self, channel_id: &str) -> Result<()> {
        self.mt_json(
            "remove channel email",
            Method::DELETE,
            &format!("channels/{}/email", enc(channel_id)),
            None,
        )
        .await
        .map(|_| ())
    }

    // ---- T2.8 notifications ----

    /// The channel's notification setting (the default when none was set).
    pub async fn channel_notifications(&self, channel_id: &str) -> Result<ChannelNotify> {
        let url = format!(
            "{}/v1/users/ME/conversations/{}?view=msnp24Equivalent",
            self.chat_service().await,
            enc(channel_id)
        );
        let v = self
            .json("channel notifications", Method::GET, &url, None)
            .await?;
        Ok(parse_notify(&v).unwrap_or_default())
    }

    /// Sets the channel's notification level.
    pub async fn set_channel_notifications(
        &self,
        channel_id: &str,
        n: ChannelNotify,
    ) -> Result<()> {
        self.put_my_property(
            channel_id,
            "channelNotificationSettings",
            &json!({"channelNotificationSettings": notify_value(n)}),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_team() -> Value {
        json!({"teams": [{
            "id": "19:t@thread.tacv2", "displayName": "Acme", "description": "d",
            "accessType": 3, "isFavorite": true, "isCollapsed": false,
            "memberRoleString": "Admin",
            "teamSiteInformation": {"groupId": "g-1"},
            "membershipSummary": {"totalMemberCount": 7, "adminRoleCount": 2, "botCount": 3},
            "smtpAddress": "acme@example.com",
            "channels": [
                {"id": "19:t@thread.tacv2", "displayName": "General", "isGeneral": true,
                 "isFavorite": true, "channelType": 0, "memberRoleString": "Admin",
                 "isMember": true},
                {"id": "19:p@thread.tacv2", "displayName": "Secret", "channelType": 1,
                 "isFavorite": false, "isPinned": true, "isMuted": true,
                 "isArchived": true, "creationTime": "2026-01-02T03:04:05.000Z"},
                {"displayName": "no id"}
            ]
        }]})
    }

    #[test]
    fn parses_a_team_with_channels() {
        let teams = parse_teams(&sample_team());
        assert_eq!(teams.len(), 1);
        let t = &teams[0];
        assert_eq!(
            (t.id.as_str(), t.group_id.as_str()),
            ("19:t@thread.tacv2", "g-1")
        );
        assert_eq!(t.access, Access::Public);
        assert_eq!(t.role, Role::Owner);
        // Bots are not people.
        assert_eq!((t.member_count, t.owner_count), (4, 2));
        assert!(t.is_shown && !t.is_collapsed);
        assert_eq!(t.channels.len(), 2);
        assert!(t.channels[0].is_general && t.channels[0].is_shown);
        let p = &t.channels[1];
        assert_eq!(p.kind, ChannelKind::Private);
        assert!(!p.is_shown && p.is_pinned && p.is_muted && p.is_archived);
        assert!(p.created_ms > 0);
        assert_eq!(
            t.team_ref(),
            TeamRef {
                thread_id: "19:t@thread.tacv2".into(),
                group_id: "g-1".into()
            }
        );
        let (team, ch) = find_channel(&teams, "19:p@thread.tacv2").unwrap();
        assert_eq!(team.name, "Acme");
        assert_eq!(ChannelRef::from_entry(ch).kind, ChannelKind::Private);
        assert!(find_channel(&teams, "19:none").is_none());
    }

    #[test]
    fn org_wide_and_private_access() {
        let t = |extra: Value| {
            let mut v = json!({"id": "19:t@thread.tacv2", "accessType": 1});
            v.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            parse_teams(&json!({"teams": [v]}))
        };
        assert_eq!(t(json!({}))[0].access, Access::Private);
        assert_eq!(t(json!({"isTenantWide": true}))[0].access, Access::OrgWide);
        assert!(parse_teams(&json!({})).is_empty());
    }

    #[test]
    fn channel_kind_reads_numbers_names_and_the_shared_flag() {
        let k = |v: Value| ChannelKind::from_value(&v);
        assert_eq!(k(json!({"channelType": 0})), ChannelKind::Standard);
        assert_eq!(k(json!({"channelType": 1})), ChannelKind::Private);
        assert_eq!(k(json!({"channelType": "Private"})), ChannelKind::Private);
        assert_eq!(k(json!({"channelType": "shared"})), ChannelKind::Shared);
        assert_eq!(
            k(json!({"channelType": 0, "isShared": true})),
            ChannelKind::Shared
        );
        assert_eq!(k(json!({})), ChannelKind::Standard);
    }

    #[test]
    fn team_and_channel_bodies() {
        let b = create_team_body("Sandbox", "test", Access::Private);
        assert_eq!(b["displayName"], "Sandbox");
        assert_eq!(b["accessType"], 1);
        assert_eq!(b["isTenantWide"], false);
        assert_eq!(create_team_body("x", "", Access::Public)["accessType"], 3);
        assert_eq!(
            create_team_body("x", "", Access::OrgWide)["isTenantWide"],
            true
        );
        assert!(validate_naming_body("n")["groupId"].is_null());
        assert_eq!(
            edit_team_body(Some("n"), None, Some(true)),
            json!({"displayName": "n", "isDiscoverable": true})
        );
        assert_eq!(
            edit_team_body(None, Some(""), None),
            json!({"description": ""})
        );

        let c = standard_channel_body("g-1", "ch", "about", false);
        assert_eq!(c["channelType"], "Standard");
        assert_eq!(c["groupId"], "g-1");
        assert!(c.get("chatModality").is_none());
        assert_eq!(
            standard_channel_body("g", "n", "", true)["chatModality"],
            "Conversational"
        );
        let p = private_channel_body("19:t@thread.tacv2", "n", "d", ChannelKind::Private);
        assert_eq!(p["spaceThreadId"], "19:t@thread.tacv2");
        assert_eq!(p["channelType"], "Private");
        assert_eq!(p["sharedWithParentTeam"], false);
        assert_eq!(
            edit_channel_body(Some("a"), None),
            json!({"displayName": "a"})
        );
        assert_eq!(
            edit_channel_body(None, Some("")),
            json!({"description": ""})
        );
    }

    #[test]
    fn team_settings_travel_as_a_json_string() {
        let b = team_settings_body(Some(&json!({"createTopic": false})), None);
        let inner: Value = serde_json::from_str(b["spaceAdminSettings"].as_str().unwrap()).unwrap();
        assert_eq!(inner, json!({"userSettings": {"createTopic": false}}));
        let both = team_settings_body(
            Some(&json!({"giphyRating": 1})),
            Some(&json!({"installApp": true})),
        );
        let inner: Value =
            serde_json::from_str(both["spaceAdminSettings"].as_str().unwrap()).unwrap();
        assert_eq!(inner["guestSettings"]["installApp"], true);
    }

    #[test]
    fn moderation_round_trips_through_the_thread_property() {
        let m = Moderation {
            post_permissions: "Moderators".into(),
            allow_bots: false,
            ..Moderation::default()
        };
        let thread = json!({"properties": {"channelSettings": moderation_value(&m)}});
        assert_eq!(parse_moderation(&thread), Some(m));
        assert_eq!(parse_moderation(&json!({"properties": {}})), None);
        assert_eq!(
            parse_moderation(&json!({"properties": {"channelSettings": "{}"}})),
            Some(Moderation::default())
        );
        assert_eq!(
            parse_moderation(&json!({"properties": {"channelSettings": "oops"}})),
            None
        );
    }

    #[test]
    fn member_bodies_and_refs() {
        let r = bulk_roles_body(
            "g-1",
            &[
                ("8:orgid:a".into(), Role::Member),
                ("8:orgid:b".into(), Role::Owner),
            ],
        );
        assert_eq!(r["groupId"], "g-1");
        assert_eq!(r["users"][0], json!({"mri": "8:orgid:a", "role": 0}));
        assert_eq!(r["users"][1]["role"], 2);
        assert_eq!(
            leave_body("g-1", "8:orgid:a"),
            json!({"groupId": "g-1", "userMri": "8:orgid:a", "updateType": "Left"})
        );
        assert_eq!(oid_ref("u", "t"), "OID:u@t");
        let p = private_members_body(
            "ten",
            &[
                ("8:orgid:u1".into(), Role::Owner),
                ("8:orgid:u2".into(), Role::Member),
            ],
        );
        assert_eq!(p["users"][0], json!({"id": "OID:u1@ten", "role": 2}));
        assert_eq!(p["users"][1]["role"], 1);
        assert_eq!(Role::Guest.team_code(), 1);
    }

    #[test]
    fn hide_show_and_pin_bodies() {
        let hide = interest_body(false, 1_000_000);
        assert_eq!(hide["interest"], "NotInterested");
        assert_eq!(hide["expiration"], 1_000 + 14 * 86_400);
        assert_eq!(
            interest_body(true, 5),
            json!({"expiration": 0, "interest": "Interested"})
        );
        assert_eq!(
            pinned_body("c", true),
            json!({"newlyPinnedChannels": ["c"]})
        );
        assert_eq!(pinned_body("c", false), json!({"channelIdsToUnpin": ["c"]}));
        let p =
            parse_pinned(&json!({"orderVersion": 42, "pinChannelOrder": ["c1", "c2"]})).unwrap();
        assert_eq!(p.order_version, "42");
        assert_eq!(p.channel_ids, ["c1", "c2"]);
        assert_eq!(
            parse_pinned(&json!({"orderVersion": "v7"}))
                .unwrap()
                .channel_ids,
            Vec::<String>::new()
        );
        assert!(parse_pinned(&json!({})).is_none());
    }

    #[test]
    fn parses_discover_join_and_created_ids() {
        let d = parse_discover(&json!({"type": "x", "value": [
            {"groupId": "g", "displayName": "Open", "description": "d", "accessType": "Public",
             "membersCount": 3, "relevantMembers": [{"smtpAddress": "a@b"}]},
            {"displayName": "no group"}
        ]}));
        assert_eq!(d.len(), 1);
        assert_eq!((d[0].members, d[0].access.as_str()), (3, "Public"));

        let j = parse_join_info(&json!({"value": {"displayName": "T", "description": "d",
            "accessType": "Private", "isTeamOwnerApprovalRequired": true}}))
        .unwrap();
        assert_eq!((j.access.as_str(), j.needs_approval), ("Private", true));
        assert!(parse_join_info(&json!({})).is_none());

        let t = parse_created_team(&json!({"value": {
            "siteInfo": {"groupId": "g-9"}, "skypeThreadId": "19:n@thread.tacv2"}}))
        .unwrap();
        assert_eq!(t.group_id, "g-9");
        assert_eq!(t.thread_id, "19:n@thread.tacv2");
        assert!(parse_created_team(&json!({"value": {"siteInfo": {"groupId": "g"}}})).is_none());
        assert_eq!(
            parse_created_channel(&json!({"value": {"objectId": "19:c@thread.tacv2"}})).as_deref(),
            Some("19:c@thread.tacv2")
        );
        assert_eq!(parse_created_channel(&json!({})), None);
        assert_eq!(
            parse_created_private(&json!({"groupId": "OID:abc-123@tenant-1", "displayName": "x"}))
                .as_deref(),
            Some("abc-123")
        );
        assert_eq!(parse_created_private(&json!({})), None);
    }

    #[test]
    fn parses_deleted_channels_and_rosters() {
        let d = parse_channels_in_state(&json!([
            {"objectId": "19:c@thread.tacv2", "displayName": "old", "description": "x",
             "deleteTime": "1795034000000", "channelState": "Deleted"},
            {"displayName": "no id"}
        ]));
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].deleted_ms, 1_795_034_000_000);
        assert!(parse_channels_in_state(&json!({})).is_empty());

        let r = parse_members(&json!({"members": [
            {"id": "8:orgid:a", "role": "Admin"},
            {"id": "8:orgid:b", "role": "User"},
            {"id": "8:orgid:c", "role": "Guest"},
            {"id": "28:bot", "role": "User"},
            {"role": "User"}
        ]}));
        assert_eq!(r.len(), 4);
        assert_eq!(r[0].role, Role::Owner);
        assert_eq!(r[1].role, Role::Member);
        assert_eq!(r[2].role, Role::Guest);
        assert!(r[3].is_bot && !r[0].is_bot);
    }

    #[test]
    fn channel_email_and_notifications() {
        let e = parse_channel_email(&json!({
            "allowedSenders": {"allowedSenderType": "members"},
            "emailAddressDetails": {"emailId": "x.example.com@amer.teams.ms"}}))
        .unwrap();
        assert_eq!(e.senders, EmailSenders::Members);
        assert_eq!(e.address, "x.example.com@amer.teams.ms");
        assert!(parse_channel_email(&json!({})).is_none());
        assert_eq!(
            channel_email_body(EmailSenders::Domains, &["a.com".into()]),
            json!({"allowedSenderType": "Domains", "allowedDomains": ["a.com"]})
        );
        assert_eq!(
            channel_email_body(EmailSenders::Anyone, &["a.com".into()]),
            json!({"allowedSenderType": "Anyone"})
        );

        for (level, replies) in [
            (NotifyLevel::BannerAndFeed, true),
            (NotifyLevel::Feed, false),
            (NotifyLevel::Off, false),
        ] {
            let n = ChannelNotify {
                level,
                include_replies: replies,
            };
            let conv = json!({"properties": {"channelNotificationSettings": notify_value(n)}});
            assert_eq!(parse_notify(&conv), Some(n));
        }
        // Replies make no sense when notifications are off.
        let off = notify_value(ChannelNotify {
            level: NotifyLevel::Off,
            include_replies: true,
        });
        assert!(off.contains("\"includeReplies\":false"));
        assert_eq!(parse_notify(&json!({"properties": {}})), None);
    }

    #[test]
    fn access_codes_and_encoding() {
        assert_eq!(Access::Private.code(), 1);
        assert_eq!(Access::Public.code(), 3);
        assert_eq!(enc("19:a@thread.tacv2"), "19%3Aa%40thread.tacv2");
    }
}
