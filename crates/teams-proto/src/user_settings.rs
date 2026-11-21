//! User settings for notifications (plans/chat.md 9.1) and privacy (plans/settings-and-platform.md
//! X1.5).
//!
//! Where each thing lives, as read from the Teams web client (facts only) and rusty-teams (MIT,
//! `chatsvc/src/settings.rs`, whose property-bag access is adapted here):
//!
//! * Notification types, sounds and meeting-chat notifications are one JSON string,
//!   `userPersonalSettings`, in the `GET /v1/users/ME/properties` bag. `toastSettings` holds the
//!   banner flag of each type and `activityGenerationSettings` the Activity-feed flag, so
//!   banner and feed is both on, feed only is the feed flag alone, and off is neither.
//!   `emailSettings` is the mail version of the same keys. `selectedPreset` is 1 all activity,
//!   2 my @mentions, 3 custom. `toastSettings.temporarilyPauseUntil` is "pause notifications".
//!   Writing is `PUT /v1/users/ME/properties?name=userPersonalSettings` with the whole object as
//!   a JSON string, so every write here keeps the keys it does not know.
//! * Read receipts, "block calls from people outside my contacts" and "block adding me to group
//!   chats" are plain bag properties (`readReceiptsEnabled`, `disableNonContactCall`,
//!   `disableNonContactAddToGroup` as strings, `userEnableSurveysNotification` as a bool).
//! * The blocked and the priority-access people are middle-tier lists:
//!   `GET {mt}/beta/userSettings/blocklist` and `.../breakthroughlist` answer `{"value": [..]}`;
//!   `POST .../manage` takes `{"add": [mri]}` or `{"remove": [mri]}`.
//! * Quiet hours are not a server setting that the web client knows, so [`QuietHours`] is a local
//!   schedule the app keeps itself.
//!
//! Every write here is a [`PlannedRequest`]: it can be printed and is never sent by this module.

use crate::client::Client;
use crate::error::{Error, Result};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// A request this module would make, kept as data so callers can print it instead of sending it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRequest {
    pub method: Method,
    pub url: String,
    pub body: Option<Value>,
}

impl Client {
    /// Sends a planned settings write (the user's own Teams settings).
    pub async fn send_planned(&self, r: &PlannedRequest) -> Result<Value> {
        self.json("settings write", r.method.clone(), &r.url, r.body.as_ref())
            .await
    }
}

impl PlannedRequest {
    /// `METHOD url`, then the body on the next line when there is one.
    pub fn dry_run(&self) -> String {
        match &self.body {
            Some(b) => format!("{} {}\n{}", self.method, self.url, b),
            None => format!("{} {}", self.method, self.url),
        }
    }
}

/// How a notification type reaches the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    BannerAndFeed,
    FeedOnly,
    Off,
}

impl Delivery {
    pub fn from_slug(s: &str) -> Option<Delivery> {
        match s {
            "banner" | "banner-and-feed" => Some(Delivery::BannerAndFeed),
            "feed" | "feed-only" => Some(Delivery::FeedOnly),
            "off" => Some(Delivery::Off),
            _ => None,
        }
    }
}

/// A notification type with its own switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    ChatMessage,
    ChatMention,
    ChatEveryoneMention,
    PersonMention,
    ChannelMention,
    TeamMention,
    TagMention,
    Reply,
    ReplyToReply,
    Like,
    LikeInChat,
    FollowedChannelPost,
    FollowedChannelReply,
    ChannelPost,
    ChannelReply,
    VisibleChannelMention,
    MeetingStarted,
    Priority,
    Vip,
}

impl Kind {
    pub const ALL: [Kind; 19] = [
        Kind::ChatMessage,
        Kind::ChatMention,
        Kind::ChatEveryoneMention,
        Kind::PersonMention,
        Kind::ChannelMention,
        Kind::TeamMention,
        Kind::TagMention,
        Kind::Reply,
        Kind::ReplyToReply,
        Kind::Like,
        Kind::LikeInChat,
        Kind::FollowedChannelPost,
        Kind::FollowedChannelReply,
        Kind::ChannelPost,
        Kind::ChannelReply,
        Kind::VisibleChannelMention,
        Kind::MeetingStarted,
        Kind::Priority,
        Kind::Vip,
    ];

    /// The key in `toastSettings` and `activityGenerationSettings`.
    pub fn key(self) -> &'static str {
        match self {
            Kind::ChatMessage => "Chat",
            Kind::ChatMention => "MentionInChat_Person_v2",
            Kind::ChatEveryoneMention => "MentionInChat_Everyone",
            Kind::PersonMention => "Mention_Person",
            Kind::ChannelMention => "Mention_Channel",
            Kind::TeamMention => "Mention_Team",
            Kind::TagMention => "Mention_Tag",
            Kind::Reply => "Reply",
            Kind::ReplyToReply => "ReplyToReply",
            Kind::Like => "Like",
            Kind::LikeInChat => "LikeInChat_v2",
            Kind::FollowedChannelPost => "Follow_ChannelNewMessage",
            Kind::FollowedChannelReply => "Follow_ChannelReplyMessage",
            Kind::ChannelPost => "Visible_ChannelNewMessage_v2",
            Kind::ChannelReply => "Visible_ChannelReplyMessage_v2",
            Kind::VisibleChannelMention => "Visible_ChannelMention_v2",
            Kind::MeetingStarted => "MeetingWentLiveNotification",
            Kind::Priority => "Priority",
            Kind::Vip => "VIP",
        }
    }

    /// Short name for command lines.
    pub fn slug(self) -> &'static str {
        match self {
            Kind::ChatMessage => "chat",
            Kind::ChatMention => "chat-mention",
            Kind::ChatEveryoneMention => "chat-everyone",
            Kind::PersonMention => "mention",
            Kind::ChannelMention => "channel-mention",
            Kind::TeamMention => "team-mention",
            Kind::TagMention => "tag-mention",
            Kind::Reply => "reply",
            Kind::ReplyToReply => "reply-to-reply",
            Kind::Like => "like",
            Kind::LikeInChat => "like-in-chat",
            Kind::FollowedChannelPost => "followed-post",
            Kind::FollowedChannelReply => "followed-reply",
            Kind::ChannelPost => "channel-post",
            Kind::ChannelReply => "channel-reply",
            Kind::VisibleChannelMention => "channel-visible-mention",
            Kind::MeetingStarted => "meeting-started",
            Kind::Priority => "priority",
            Kind::Vip => "vip",
        }
    }

    pub fn from_slug(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.slug() == s)
    }
}

/// `selectedPreset`: the starting point Teams offers before the per-type switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    AllActivity,
    MyAtMentions,
    Custom,
}

/// Sound and preview settings. They sit in `soundsSettings` or `toastSettings`; the values are
/// Teams option names such as `On`, `Off`, `Default`, `Ring`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundField {
    AlertSound,
    CallAndMeetingStart,
    ShowDuringCallsAndMeetings,
    AlertSoundType,
    UrgentSoundType,
    WhenBusyOrInMeetings,
    UrgentAndPriority,
    GlobalSoundAndPreview,
    ShowPreviewInToasts,
}

impl SoundField {
    pub const ALL: [SoundField; 9] = [
        SoundField::AlertSound,
        SoundField::CallAndMeetingStart,
        SoundField::ShowDuringCallsAndMeetings,
        SoundField::AlertSoundType,
        SoundField::UrgentSoundType,
        SoundField::WhenBusyOrInMeetings,
        SoundField::UrgentAndPriority,
        SoundField::GlobalSoundAndPreview,
        SoundField::ShowPreviewInToasts,
    ];

    fn location(self) -> (&'static str, &'static str) {
        match self {
            SoundField::AlertSound => ("soundsSettings", "alertSound"),
            SoundField::CallAndMeetingStart => {
                ("soundsSettings", "incomingCallsAndMeetingStartSound")
            }
            SoundField::ShowDuringCallsAndMeetings => {
                ("soundsSettings", "showNotificationDuringCallsAndMeetings")
            }
            SoundField::AlertSoundType => ("toastSettings", "alertSoundType"),
            SoundField::UrgentSoundType => ("toastSettings", "alertUrgentSoundType"),
            SoundField::WhenBusyOrInMeetings => (
                "toastSettings",
                "PlayNotificationSoundsWhenBusyOrInMeetings",
            ),
            SoundField::UrgentAndPriority => (
                "toastSettings",
                "PlayNotificationSoundsForUrgentAndPriority",
            ),
            SoundField::GlobalSoundAndPreview => ("toastSettings", "GlobalSoundAndPreview"),
            SoundField::ShowPreviewInToasts => ("toastSettings", "ShowPreviewInToasts"),
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            SoundField::AlertSound => "alert-sound",
            SoundField::CallAndMeetingStart => "call-sound",
            SoundField::ShowDuringCallsAndMeetings => "show-in-calls",
            SoundField::AlertSoundType => "alert-sound-type",
            SoundField::UrgentSoundType => "urgent-sound-type",
            SoundField::WhenBusyOrInMeetings => "sound-when-busy",
            SoundField::UrgentAndPriority => "sound-urgent",
            SoundField::GlobalSoundAndPreview => "sound-and-preview",
            SoundField::ShowPreviewInToasts => "preview",
        }
    }

    pub fn from_slug(s: &str) -> Option<SoundField> {
        SoundField::ALL.into_iter().find(|f| f.slug() == s)
    }
}

/// Which meeting chats notify (`chatSettings.meetingChatNotification`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeetingChatMode {
    /// Meetings the user takes part in.
    Participated,
    /// Every meeting the user is invited to.
    Invited,
    /// None, unless unmuted by hand.
    Unmuted,
}

impl MeetingChatMode {
    fn value(self) -> &'static str {
        match self {
            MeetingChatMode::Participated => "MeetingsUserParticipatesIn",
            MeetingChatMode::Invited => "MeetingsUserIsInvitedTo",
            MeetingChatMode::Unmuted => "MeetingsManuallyUnmuted",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            MeetingChatMode::Participated => "participated",
            MeetingChatMode::Invited => "all",
            MeetingChatMode::Unmuted => "none",
        }
    }

    pub fn from_slug(s: &str) -> Option<MeetingChatMode> {
        [
            MeetingChatMode::Participated,
            MeetingChatMode::Invited,
            MeetingChatMode::Unmuted,
        ]
        .into_iter()
        .find(|m| m.slug() == s)
    }
}

/// The RSVP answers a meeting chat can notify for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rsvp {
    Accepted,
    Tentative,
    Follow,
    NoResponse,
    Declined,
}

impl Rsvp {
    pub const ALL: [Rsvp; 5] = [
        Rsvp::Accepted,
        Rsvp::Tentative,
        Rsvp::Follow,
        Rsvp::NoResponse,
        Rsvp::Declined,
    ];

    fn key(self) -> &'static str {
        match self {
            Rsvp::Accepted => "meetingChatNotificationRSVPv2Accepted",
            Rsvp::Tentative => "meetingChatNotificationRSVPv2Tentative",
            Rsvp::Follow => "meetingChatNotificationRSVPv2Follow",
            Rsvp::NoResponse => "meetingChatNotificationRSVPv2NoResponse",
            Rsvp::Declined => "meetingChatNotificationRSVPv2Declined",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Rsvp::Accepted => "accepted",
            Rsvp::Tentative => "tentative",
            Rsvp::Follow => "follow",
            Rsvp::NoResponse => "no-response",
            Rsvp::Declined => "declined",
        }
    }

    pub fn from_slug(s: &str) -> Option<Rsvp> {
        Rsvp::ALL.into_iter().find(|r| r.slug() == s)
    }
}

/// The notification half of `userPersonalSettings`, kept whole so a write loses nothing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NotificationSettings {
    personal: Map<String, Value>,
}

impl NotificationSettings {
    /// Reads the settings out of the `/v1/users/ME/properties` bag. Teams stores them as a JSON
    /// string; an object is accepted too. Missing or broken settings read as empty.
    pub fn from_properties(props: &Value) -> Self {
        let parsed = match props.get("userPersonalSettings") {
            Some(Value::String(s)) => serde_json::from_str::<Value>(s).ok(),
            Some(v) => Some(v.clone()),
            None => None,
        };
        NotificationSettings {
            personal: parsed
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default(),
        }
    }

    pub fn to_value(&self) -> Value {
        Value::Object(self.personal.clone())
    }

    fn get(&self, section: &str, key: &str) -> Option<&Value> {
        self.personal.get(section)?.get(key)
    }

    fn put(&mut self, section: &str, key: &str, value: Value) {
        let s = self
            .personal
            .entry(section.to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        if !s.is_object() {
            *s = Value::Object(Map::new());
        }
        if let Some(o) = s.as_object_mut() {
            o.insert(key.to_owned(), value);
        }
    }

    pub fn preset(&self) -> Option<Preset> {
        match self.personal.get("selectedPreset")?.as_i64()? {
            1 => Some(Preset::AllActivity),
            2 => Some(Preset::MyAtMentions),
            3 => Some(Preset::Custom),
            _ => None,
        }
    }

    /// How `kind` is delivered, or `None` when Teams has no value for it yet.
    pub fn delivery(&self, kind: Kind) -> Option<Delivery> {
        let feed = self
            .get("activityGenerationSettings", kind.key())
            .and_then(Value::as_bool);
        let banner = self
            .get("toastSettings", kind.key())
            .and_then(Value::as_bool);
        if feed.is_none() && banner.is_none() {
            return None;
        }
        Some(match (feed.unwrap_or(false), banner.unwrap_or(false)) {
            (true, true) => Delivery::BannerAndFeed,
            (true, false) => Delivery::FeedOnly,
            _ => Delivery::Off,
        })
    }

    /// Sets how `kind` is delivered. Like Teams, this moves the preset to custom.
    pub fn set_delivery(&mut self, kind: Kind, delivery: Delivery) {
        let (feed, banner) = match delivery {
            Delivery::BannerAndFeed => (true, true),
            Delivery::FeedOnly => (true, false),
            Delivery::Off => (false, false),
        };
        self.put("activityGenerationSettings", kind.key(), json!(feed));
        self.put("toastSettings", kind.key(), json!(banner));
        self.personal.insert("selectedPreset".to_owned(), json!(3));
    }

    /// When "pause notifications" ends (an ISO time), if it is set.
    pub fn paused_until(&self) -> Option<&str> {
        self.get("toastSettings", "temporarilyPauseUntil")?
            .as_str()
            .filter(|s| !s.is_empty())
    }

    pub fn set_paused_until(&mut self, until: Option<String>) {
        match until {
            Some(t) => self.put("toastSettings", "temporarilyPauseUntil", json!(t)),
            None => {
                if let Some(o) = self
                    .personal
                    .get_mut("toastSettings")
                    .and_then(Value::as_object_mut)
                {
                    o.remove("temporarilyPauseUntil");
                }
            }
        }
    }

    pub fn sound(&self, field: SoundField) -> Option<&str> {
        let (section, key) = field.location();
        self.get(section, key)?.as_str()
    }

    pub fn set_sound(&mut self, field: SoundField, value: &str) {
        let (section, key) = field.location();
        self.put(section, key, json!(value));
    }

    pub fn meeting_chat_mode(&self) -> Option<MeetingChatMode> {
        match self
            .get("chatSettings", "meetingChatNotification")?
            .as_str()?
        {
            "MeetingsUserParticipatesIn" => Some(MeetingChatMode::Participated),
            "MeetingsUserIsInvitedTo" => Some(MeetingChatMode::Invited),
            "MeetingsManuallyUnmuted" => Some(MeetingChatMode::Unmuted),
            _ => None,
        }
    }

    pub fn set_meeting_chat_mode(&mut self, mode: MeetingChatMode) {
        self.put(
            "chatSettings",
            "meetingChatNotification",
            json!(mode.value()),
        );
    }

    pub fn meeting_chat_rsvp(&self, rsvp: Rsvp) -> Option<bool> {
        self.get("chatSettings", rsvp.key())?.as_bool()
    }

    pub fn set_meeting_chat_rsvp(&mut self, rsvp: Rsvp, on: bool) {
        self.put("chatSettings", rsvp.key(), json!(on));
    }

    /// The write that saves these settings. Not sent by this module.
    pub fn write_request(&self, chat_service: &str) -> PlannedRequest {
        property_request(
            chat_service,
            "userPersonalSettings",
            Value::String(self.to_value().to_string()),
        )
    }
}

fn property_request(chat_service: &str, name: &str, value: Value) -> PlannedRequest {
    PlannedRequest {
        method: Method::PUT,
        url: format!("{chat_service}/v1/users/ME/properties?name={name}"),
        body: Some(json!({ name: value })),
    }
}

/// The privacy switches of the property bag plus the blocked people.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrivacySettings {
    /// MRIs of the blocked people.
    pub blocked: Vec<String>,
    /// Whether people see when we have read their messages.
    pub read_receipts: bool,
    /// Calls from people outside our contacts are blocked.
    pub block_non_contact_calls: bool,
    /// People outside our contacts cannot add us to group chats.
    pub block_non_contact_group_adds: bool,
    /// Whether Microsoft may ask us to take surveys.
    pub surveys: bool,
}

/// A privacy switch in the property bag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyFlag {
    ReadReceipts,
    BlockNonContactCalls,
    BlockNonContactGroupAdds,
    Surveys,
}

impl PrivacyFlag {
    pub const ALL: [PrivacyFlag; 4] = [
        PrivacyFlag::ReadReceipts,
        PrivacyFlag::BlockNonContactCalls,
        PrivacyFlag::BlockNonContactGroupAdds,
        PrivacyFlag::Surveys,
    ];

    fn property(self) -> &'static str {
        match self {
            PrivacyFlag::ReadReceipts => "readReceiptsEnabled",
            PrivacyFlag::BlockNonContactCalls => "disableNonContactCall",
            PrivacyFlag::BlockNonContactGroupAdds => "disableNonContactAddToGroup",
            PrivacyFlag::Surveys => "userEnableSurveysNotification",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            PrivacyFlag::ReadReceipts => "read-receipts",
            PrivacyFlag::BlockNonContactCalls => "block-calls",
            PrivacyFlag::BlockNonContactGroupAdds => "block-group-adds",
            PrivacyFlag::Surveys => "surveys",
        }
    }

    pub fn from_slug(s: &str) -> Option<PrivacyFlag> {
        PrivacyFlag::ALL.into_iter().find(|f| f.slug() == s)
    }
}

/// Reads the privacy switches. `blocked` stays empty: it comes from [`contacts`].
pub fn parse_privacy(props: &Value) -> PrivacySettings {
    let flag = |name: &str| match props.get(name) {
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::String(s)) => Some(s.eq_ignore_ascii_case("true")),
        _ => None,
    };
    PrivacySettings {
        blocked: Vec::new(),
        read_receipts: crate::receipts::parse_read_receipts_enabled(props),
        block_non_contact_calls: flag("disableNonContactCall").unwrap_or(false),
        block_non_contact_group_adds: flag("disableNonContactAddToGroup").unwrap_or(false),
        surveys: flag("userEnableSurveysNotification").unwrap_or(true),
    }
}

/// The write that sets a privacy switch. Teams keeps three of them as the strings `"true"` and
/// `"false"`, and the surveys one as a bool. Not sent by this module.
pub fn privacy_flag_request(chat_service: &str, flag: PrivacyFlag, on: bool) -> PlannedRequest {
    let value = match flag {
        PrivacyFlag::Surveys => json!(on),
        _ => json!(on.to_string()),
    };
    property_request(chat_service, flag.property(), value)
}

/// A list of people kept by the middle tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactList {
    /// People we blocked.
    Blocked,
    /// People who reach us in Do not disturb ("priority access", Teams calls the list
    /// "breakthrough").
    Priority,
}

impl ContactList {
    fn path(self) -> &'static str {
        match self {
            ContactList::Blocked => "beta/userSettings/blocklist",
            ContactList::Priority => "beta/userSettings/breakthroughlist",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Add,
    Remove,
}

/// People who may notify us in Do not disturb.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PriorityAccess {
    pub contacts: Vec<String>,
}

/// Reads a `{"value": [..]}` list whose entries are MRIs or objects with an `mri`.
pub fn parse_contact_list(v: &Value) -> Vec<String> {
    v.get("value")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| match e {
                    Value::String(s) => Some(s.as_str()),
                    other => other.get("mri").and_then(Value::as_str),
                })
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub fn list_request(middle_tier: &str, list: ContactList) -> PlannedRequest {
    PlannedRequest {
        method: Method::GET,
        url: format!("{middle_tier}/{}", list.path()),
        body: None,
    }
}

/// The write that adds or removes one person. Not sent by this module.
pub fn manage_request(
    middle_tier: &str,
    list: ContactList,
    change: Change,
    mri: &str,
) -> PlannedRequest {
    let field = match change {
        Change::Add => "add",
        Change::Remove => "remove",
    };
    PlannedRequest {
        method: Method::POST,
        url: format!("{middle_tier}/{}/manage", list.path()),
        body: Some(json!({ field: [mri] })),
    }
}

/// Checks the answer of a manage call: `{"value": {"status": ..}}` with a status that is not
/// `Error`.
pub fn check_manage_result(v: &Value) -> Result<()> {
    match v.pointer("/value/status").and_then(Value::as_str) {
        Some(s) if s != "Error" => Ok(()),
        _ => Err(Error::Protocol(
            "contact list change was not accepted".into(),
        )),
    }
}

/// The `/v1/users/ME/properties` bag.
pub async fn properties(client: &Client) -> Result<Value> {
    let url = format!("{}/v1/users/ME/properties", client.chat_service().await);
    client
        .json("user properties", Method::GET, &url, None)
        .await
}

pub async fn notifications(client: &Client) -> Result<NotificationSettings> {
    Ok(NotificationSettings::from_properties(
        &properties(client).await?,
    ))
}

/// The MRIs on one middle-tier list.
pub async fn contacts(client: &Client, list: ContactList) -> Result<Vec<String>> {
    let req = list_request(&client.middle_tier().await, list);
    let v = client
        .json("contact list", req.method, &req.url, None)
        .await?;
    Ok(parse_contact_list(&v))
}

pub async fn priority_access(client: &Client) -> Result<PriorityAccess> {
    Ok(PriorityAccess {
        contacts: contacts(client, ContactList::Priority).await?,
    })
}

pub async fn privacy(client: &Client) -> Result<PrivacySettings> {
    let mut p = parse_privacy(&properties(client).await?);
    p.blocked = contacts(client, ContactList::Blocked).await?;
    Ok(p)
}

/// A day of the week, Monday first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Day {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Day {
    const ALL: [Day; 7] = [
        Day::Mon,
        Day::Tue,
        Day::Wed,
        Day::Thu,
        Day::Fri,
        Day::Sat,
        Day::Sun,
    ];

    /// `mon`, `Monday` and so on.
    pub fn from_name(s: &str) -> Option<Day> {
        let s = s.to_ascii_lowercase();
        (s.len() >= 3).then_some(())?;
        Day::ALL.into_iter().find(|d| {
            let name = format!("{d:?}").to_ascii_lowercase();
            let full = match d {
                Day::Mon => "monday",
                Day::Tue => "tuesday",
                Day::Wed => "wednesday",
                Day::Thu => "thursday",
                Day::Fri => "friday",
                Day::Sat => "saturday",
                Day::Sun => "sunday",
            };
            s == name || s == full
        })
    }

    fn previous(self) -> Day {
        let i = Day::ALL.iter().position(|d| *d == self).unwrap_or(0);
        Day::ALL[(i + 6) % 7]
    }
}

/// A local do-not-disturb schedule. Teams has no server-side quiet hours that desktop clients
/// read, so the app stores this itself. `days` are the days a window starts on, so a Friday
/// 22:00 to 07:00 window also covers early Saturday. `mobile` records that the user also wants
/// the phone quiet; IronTeams cannot push that to the phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuietHours {
    pub enabled: bool,
    pub days: Vec<Day>,
    /// Start, in minutes after midnight.
    pub start_min: u16,
    /// End, in minutes after midnight. At or before the start it means the next day.
    pub end_min: u16,
    pub mobile: bool,
}

impl QuietHours {
    /// A common starting point: weeknights 22:00 to 07:00.
    pub fn weeknights() -> QuietHours {
        QuietHours {
            enabled: true,
            days: vec![Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri],
            start_min: 22 * 60,
            end_min: 7 * 60,
            mobile: true,
        }
    }

    /// True when `minute` (after midnight) of `day` falls inside a quiet window.
    pub fn is_quiet(&self, day: Day, minute: u16) -> bool {
        if !self.enabled || self.start_min == self.end_min {
            return false;
        }
        if self.start_min < self.end_min {
            return self.days.contains(&day) && (self.start_min..self.end_min).contains(&minute);
        }
        (self.days.contains(&day) && minute >= self.start_min)
            || (self.days.contains(&day.previous()) && minute < self.end_min)
    }
}

/// `22:30` to minutes after midnight.
pub fn parse_clock(s: &str) -> Option<u16> {
    let (h, m) = s.split_once(':')?;
    let (h, m): (u16, u16) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

pub fn format_clock(minutes: u16) -> String {
    format!("{:02}:{:02}", minutes / 60 % 24, minutes % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CS: &str = "https://teams.microsoft.com/api/chatsvc/emea";
    const MT: &str = "https://teams.microsoft.com/api/mt/emea";

    fn bag() -> Value {
        let personal = json!({
            "selectedPreset": 3,
            "toastSettings": {
                "Chat": true, "Reply": true, "Like": false, "Mention_Person": true,
                "Mention_Team": false, "temporarilyPauseUntil": "2026-11-19T12:00:00.000Z",
                "alertSoundType": "Default", "PlayNotificationSoundsWhenBusyOrInMeetings": "Off",
            },
            "activityGenerationSettings": {
                "Chat": true, "Reply": true, "Like": true, "Mention_Person": true,
                "Mention_Team": false, "FutureThing": 7,
            },
            "emailSettings": {"Chat": false, "EmailNotificationDelay": "Hourly"},
            "soundsSettings": {"alertSound": "On", "incomingCallsAndMeetingStartSound": "Ring"},
            "chatSettings": {
                "meetingChatNotification": "MeetingsUserIsInvitedTo",
                "meetingChatNotificationRSVPv2Accepted": true,
                "meetingChatNotificationRSVPv2Declined": false,
                "mruChatHeaderCallOption": "VOICE_CALL",
            },
            "unknownSection": {"keep": [1, 2]},
        });
        json!({
            "skypeName": "orgid:synthetic",
            "userPersonalSettings": personal.to_string(),
            "readReceiptsEnabled": "false",
            "disableNonContactCall": "true",
            "userEnableSurveysNotification": true,
        })
    }

    #[test]
    fn delivery_combines_the_banner_and_feed_flags() {
        let n = NotificationSettings::from_properties(&bag());
        assert_eq!(n.delivery(Kind::ChatMessage), Some(Delivery::BannerAndFeed));
        assert_eq!(n.delivery(Kind::Like), Some(Delivery::FeedOnly));
        assert_eq!(n.delivery(Kind::TeamMention), Some(Delivery::Off));
        assert_eq!(n.delivery(Kind::Priority), None);
        assert_eq!(n.preset(), Some(Preset::Custom));
    }

    #[test]
    fn properties_may_carry_the_settings_as_an_object() {
        let v = json!({"userPersonalSettings": {"selectedPreset": 1}});
        assert_eq!(
            NotificationSettings::from_properties(&v).preset(),
            Some(Preset::AllActivity)
        );
        let empty = NotificationSettings::from_properties(&json!({}));
        assert_eq!(empty.preset(), None);
        assert_eq!(empty.delivery(Kind::Reply), None);
    }

    #[test]
    fn set_delivery_edits_both_flags_and_keeps_everything_else() {
        let mut n = NotificationSettings::from_properties(&bag());
        n.set_delivery(Kind::TeamMention, Delivery::BannerAndFeed);
        n.set_delivery(Kind::Reply, Delivery::Off);
        n.set_delivery(Kind::Priority, Delivery::FeedOnly);
        assert_eq!(n.delivery(Kind::TeamMention), Some(Delivery::BannerAndFeed));
        assert_eq!(n.delivery(Kind::Reply), Some(Delivery::Off));
        assert_eq!(n.delivery(Kind::Priority), Some(Delivery::FeedOnly));
        let raw = n.to_value();
        assert_eq!(raw["selectedPreset"], json!(3));
        assert_eq!(raw["activityGenerationSettings"]["FutureThing"], json!(7));
        assert_eq!(raw["emailSettings"]["Chat"], json!(false));
        assert_eq!(raw["unknownSection"], json!({"keep": [1, 2]}));
        assert_eq!(raw["toastSettings"]["Reply"], json!(false));
        assert_eq!(raw["activityGenerationSettings"]["Reply"], json!(false));
        assert_eq!(raw["toastSettings"]["Priority"], json!(false));
        assert_eq!(raw["activityGenerationSettings"]["Priority"], json!(true));
    }

    #[test]
    fn changing_a_type_moves_the_preset_to_custom() {
        let v = json!({"userPersonalSettings": {"selectedPreset": 2}});
        let mut n = NotificationSettings::from_properties(&v);
        n.set_delivery(Kind::Like, Delivery::Off);
        assert_eq!(n.preset(), Some(Preset::Custom));
        let mut fresh = NotificationSettings::from_properties(&json!({}));
        fresh.set_delivery(Kind::Like, Delivery::Off);
        assert_eq!(fresh.to_value()["selectedPreset"], json!(3));
    }

    #[test]
    fn the_write_is_one_put_of_the_settings_as_a_json_string() {
        let mut n = NotificationSettings::from_properties(&bag());
        n.set_delivery(Kind::Like, Delivery::Off);
        let r = n.write_request(CS);
        assert_eq!(r.method, Method::PUT);
        assert_eq!(
            r.url,
            format!("{CS}/v1/users/ME/properties?name=userPersonalSettings")
        );
        let sent = r.body.unwrap()["userPersonalSettings"]
            .as_str()
            .unwrap()
            .to_owned();
        let back: Value = serde_json::from_str(&sent).unwrap();
        assert_eq!(back["toastSettings"]["Like"], json!(false));
        assert_eq!(back["unknownSection"], json!({"keep": [1, 2]}));
    }

    #[test]
    fn pause_is_read_and_written_in_the_toast_settings() {
        let mut n = NotificationSettings::from_properties(&bag());
        assert_eq!(n.paused_until(), Some("2026-11-19T12:00:00.000Z"));
        n.set_paused_until(None);
        assert_eq!(n.paused_until(), None);
        n.set_paused_until(Some("2026-11-20T08:00:00.000Z".into()));
        assert_eq!(
            n.to_value()["toastSettings"]["temporarilyPauseUntil"],
            json!("2026-11-20T08:00:00.000Z")
        );
        let blank =
            json!({"userPersonalSettings": {"toastSettings": {"temporarilyPauseUntil": ""}}});
        assert_eq!(
            NotificationSettings::from_properties(&blank).paused_until(),
            None
        );
    }

    #[test]
    fn sounds_come_from_the_toast_and_sound_sections() {
        let mut n = NotificationSettings::from_properties(&bag());
        assert_eq!(n.sound(SoundField::AlertSound), Some("On"));
        assert_eq!(n.sound(SoundField::CallAndMeetingStart), Some("Ring"));
        assert_eq!(n.sound(SoundField::AlertSoundType), Some("Default"));
        assert_eq!(n.sound(SoundField::WhenBusyOrInMeetings), Some("Off"));
        assert_eq!(n.sound(SoundField::UrgentAndPriority), None);
        n.set_sound(SoundField::UrgentAndPriority, "On");
        n.set_sound(SoundField::AlertSound, "Off");
        let raw = n.to_value();
        assert_eq!(
            raw["toastSettings"]["PlayNotificationSoundsForUrgentAndPriority"],
            json!("On")
        );
        assert_eq!(raw["soundsSettings"]["alertSound"], json!("Off"));
        // A sound is not a notification type: the preset stays.
        assert_eq!(raw["selectedPreset"], json!(3));
    }

    #[test]
    fn meeting_chat_notifications_read_and_write() {
        let mut n = NotificationSettings::from_properties(&bag());
        assert_eq!(n.meeting_chat_mode(), Some(MeetingChatMode::Invited));
        assert_eq!(n.meeting_chat_rsvp(Rsvp::Accepted), Some(true));
        assert_eq!(n.meeting_chat_rsvp(Rsvp::Declined), Some(false));
        assert_eq!(n.meeting_chat_rsvp(Rsvp::Follow), None);
        n.set_meeting_chat_mode(MeetingChatMode::Participated);
        n.set_meeting_chat_rsvp(Rsvp::Follow, true);
        let raw = n.to_value();
        assert_eq!(
            raw["chatSettings"]["meetingChatNotification"],
            json!("MeetingsUserParticipatesIn")
        );
        assert_eq!(
            raw["chatSettings"]["meetingChatNotificationRSVPv2Follow"],
            json!(true)
        );
        assert_eq!(
            raw["chatSettings"]["mruChatHeaderCallOption"],
            json!("VOICE_CALL")
        );
        assert_eq!(
            MeetingChatMode::from_slug("none"),
            Some(MeetingChatMode::Unmuted)
        );
    }

    #[test]
    fn kinds_have_unique_keys_and_slugs() {
        let mut keys: Vec<_> = Kind::ALL.iter().map(|k| k.key()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), Kind::ALL.len());
        for k in Kind::ALL {
            assert_eq!(Kind::from_slug(k.slug()), Some(k));
        }
        assert_eq!(Kind::from_slug("nope"), None);
        assert_eq!(Kind::TeamMention.key(), "Mention_Team");
    }

    #[test]
    fn privacy_flags_are_read_from_the_bag() {
        let p = parse_privacy(&bag());
        assert!(!p.read_receipts);
        assert!(p.block_non_contact_calls);
        assert!(!p.block_non_contact_group_adds);
        assert!(p.surveys);
        let d = parse_privacy(&json!({}));
        assert!(d.read_receipts);
        assert!(!d.block_non_contact_calls);
        assert!(d.surveys);
    }

    #[test]
    fn a_privacy_flag_is_put_as_the_string_teams_uses() {
        let r = privacy_flag_request(CS, PrivacyFlag::ReadReceipts, true);
        assert_eq!(r.method, Method::PUT);
        assert_eq!(
            r.url,
            format!("{CS}/v1/users/ME/properties?name=readReceiptsEnabled")
        );
        assert_eq!(r.body, Some(json!({"readReceiptsEnabled": "true"})));
        let r = privacy_flag_request(CS, PrivacyFlag::BlockNonContactCalls, false);
        assert_eq!(r.body, Some(json!({"disableNonContactCall": "false"})));
        let r = privacy_flag_request(CS, PrivacyFlag::Surveys, false);
        assert_eq!(
            r.body,
            Some(json!({"userEnableSurveysNotification": false}))
        );
        assert_eq!(
            PrivacyFlag::from_slug("read-receipts"),
            Some(PrivacyFlag::ReadReceipts)
        );
    }

    #[test]
    fn contact_lists_accept_strings_or_objects() {
        let v = json!({"value": ["8:orgid:a", {"mri": "8:orgid:b"}, {"x": 1}, 5, ""]});
        assert_eq!(parse_contact_list(&v), vec!["8:orgid:a", "8:orgid:b"]);
        assert!(parse_contact_list(&json!({})).is_empty());
        assert!(parse_contact_list(&json!({"value": null})).is_empty());
    }

    #[test]
    fn block_and_priority_writes_are_posts_to_the_manage_routes() {
        let r = manage_request(MT, ContactList::Blocked, Change::Add, "8:orgid:a");
        assert_eq!(r.method, Method::POST);
        assert_eq!(r.url, format!("{MT}/beta/userSettings/blocklist/manage"));
        assert_eq!(r.body, Some(json!({"add": ["8:orgid:a"]})));
        let r = manage_request(MT, ContactList::Priority, Change::Remove, "8:orgid:a");
        assert_eq!(
            r.url,
            format!("{MT}/beta/userSettings/breakthroughlist/manage")
        );
        assert_eq!(r.body, Some(json!({"remove": ["8:orgid:a"]})));
        assert_eq!(
            list_request(MT, ContactList::Priority).url,
            format!("{MT}/beta/userSettings/breakthroughlist")
        );
    }

    #[test]
    fn a_manage_answer_must_report_a_status_other_than_error() {
        assert!(check_manage_result(&json!({"value": {"status": "Success"}})).is_ok());
        assert!(check_manage_result(&json!({"value": {"status": "Error"}})).is_err());
        assert!(check_manage_result(&json!({})).is_err());
    }

    #[test]
    fn dry_run_prints_method_url_and_body() {
        let r = manage_request(MT, ContactList::Blocked, Change::Remove, "8:orgid:a");
        let text = r.dry_run();
        assert!(text.starts_with(&format!("POST {MT}/beta/userSettings/blocklist/manage")));
        assert!(text.contains(r#"{"remove":["8:orgid:a"]}"#));
        assert_eq!(
            list_request(MT, ContactList::Blocked)
                .dry_run()
                .lines()
                .count(),
            1
        );
    }

    fn weekdays(d: &[Day]) -> Vec<Day> {
        d.to_vec()
    }

    #[test]
    fn quiet_hours_inside_one_day() {
        let q = QuietHours {
            enabled: true,
            days: weekdays(&[Day::Sat, Day::Sun]),
            start_min: 9 * 60,
            end_min: 17 * 60,
            mobile: false,
        };
        assert!(q.is_quiet(Day::Sat, 9 * 60));
        assert!(q.is_quiet(Day::Sun, 16 * 60 + 59));
        assert!(!q.is_quiet(Day::Sat, 17 * 60));
        assert!(!q.is_quiet(Day::Mon, 12 * 60));
    }

    #[test]
    fn quiet_hours_may_run_past_midnight() {
        let q = QuietHours {
            enabled: true,
            days: weekdays(&[Day::Fri]),
            start_min: 22 * 60,
            end_min: 7 * 60,
            mobile: true,
        };
        assert!(q.is_quiet(Day::Fri, 23 * 60));
        assert!(q.is_quiet(Day::Sat, 6 * 60 + 59));
        assert!(!q.is_quiet(Day::Sat, 7 * 60));
        assert!(!q.is_quiet(Day::Fri, 6 * 60));
        assert!(!q.is_quiet(Day::Sat, 23 * 60));
        let wrap = QuietHours {
            days: weekdays(&[Day::Sun]),
            ..q.clone()
        };
        assert!(wrap.is_quiet(Day::Mon, 3 * 60));
    }

    #[test]
    fn quiet_hours_off_or_empty_never_fire() {
        let mut q = QuietHours::weeknights();
        assert!(q.is_quiet(Day::Tue, 23 * 60));
        q.enabled = false;
        assert!(!q.is_quiet(Day::Tue, 23 * 60));
        q.enabled = true;
        q.end_min = q.start_min;
        assert!(!q.is_quiet(Day::Tue, 23 * 60));
    }

    #[test]
    fn quiet_hours_parse_times_and_round_trip_json() {
        assert_eq!(parse_clock("22:30"), Some(22 * 60 + 30));
        assert_eq!(parse_clock("7:05"), Some(7 * 60 + 5));
        assert_eq!(parse_clock("24:00"), None);
        assert_eq!(parse_clock("12:60"), None);
        assert_eq!(parse_clock("noon"), None);
        assert_eq!(format_clock(22 * 60 + 30), "22:30");
        assert_eq!(Day::from_name("mon"), Some(Day::Mon));
        assert_eq!(Day::from_name("Sunday"), Some(Day::Sun));
        assert_eq!(Day::from_name("x"), None);
        let q = QuietHours::weeknights();
        let text = serde_json::to_string(&q).unwrap();
        assert_eq!(serde_json::from_str::<QuietHours>(&text).unwrap(), q);
    }
}
