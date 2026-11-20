//! Teams and channels management: the "Your teams and channels" page, discover and join, the
//! channel list control, new and edited channels and teams, members, tags, the channel email
//! address, notifications and the read-only Communities feed (plan `plans/teams-and-channels.md`).
//!
//! The UI sends `TeamsStore.command(name, argument)`; `TeamsCmd::parse` turns that into a
//! command, `Engine::manage_cmd` runs it, and the result is pushed back as one `TeamsData`.
//!
//! Safety: when `KELMA_SANDBOX_FILE` names the sandbox ids file, every write is refused unless it
//! targets the sandbox team, so a test run can never change a real team.

mod data;
#[cfg(test)]
mod live;
mod view;

use super::Engine;
use crate::backend::util::chat_time;
pub use data::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use teams_proto::engage::{Community, EngagePost};
use teams_proto::tags::Tag;
use teams_proto::teams::{
    Access, ChannelKind, ChannelNotify, ChannelRef, DiscoverTeam, EmailSenders, Moderation,
    NotifyLevel, Role, TeamEntry, TeamRef,
};
pub use view::{Page, SEP, role_name};

/// Every command name the UI may send.
const NAMES: &[&str] = &[
    "open",
    "refresh",
    "filter",
    "sort",
    "query",
    "expand",
    "team-show",
    "chan-show",
    "chan-pin",
    "chan-default",
    "leave-team",
    "leave-channel",
    "dialog",
    "dialog-close",
    "channel-create",
    "channel-edit",
    "channel-delete",
    "channel-restore",
    "channel-archive",
    "mod-set",
    "roster",
    "pick",
    "member-search",
    "member-add",
    "member-role",
    "member-remove",
    "tag-create",
    "tag-rename",
    "tag-delete",
    "tag-select",
    "tag-add",
    "tag-remove",
    "email-create",
    "email-remove",
    "email-senders",
    "email-copy",
    "notify-set",
    "team-create",
    "team-edit",
    "team-setting",
    "team-code",
    "team-archive",
    "team-delete",
    "team-link",
    "chan-link",
    "discover",
    "join",
    "join-cancel",
    "join-code",
    "communities",
    "community-open",
    "community-more",
    "open-post",
];

/// A command from the Teams UI: a name and its arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct TeamsCmd {
    pub name: String,
    pub args: Vec<String>,
}

impl TeamsCmd {
    /// The commands the UI sends as `(name, argument)`; several arguments are joined by [`SEP`].
    pub fn parse(name: &str, arg: &str) -> Option<TeamsCmd> {
        NAMES.contains(&name).then(|| TeamsCmd {
            name: name.into(),
            args: if arg.is_empty() {
                vec![]
            } else {
                arg.split(SEP).map(str::to_owned).collect()
            },
        })
    }

    fn arg(&self, i: usize) -> &str {
        self.args.get(i).map_or("", String::as_str)
    }

    fn flag(&self, i: usize) -> bool {
        self.arg(i) == "1"
    }
}

/// The only team a test run may write to.
#[derive(Debug, Clone, PartialEq)]
struct Guard {
    team: String,
    group: String,
    channels: Vec<String>,
}

impl Guard {
    fn from_env() -> Option<Guard> {
        let path = std::env::var("KELMA_SANDBOX_FILE").ok()?;
        let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        let text = |p: &str| v.pointer(p).and_then(Value::as_str).map(str::to_owned);
        Some(Guard {
            team: text("/team/thread_id")?,
            group: text("/team/group_id")?,
            channels: v
                .get("channels")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .filter_map(|(_, c)| c.as_str().map(str::to_owned))
                .collect(),
        })
    }
}

/// Settings a member or guest may be given, as stored by the service. Only keys that the team
/// already stores are listed, so a key the service does not know is never written.
const SETTINGS: &[(&str, &str, &str)] = &[
    (
        "userSettings",
        "editEnabled",
        "Members can edit their messages",
    ),
    (
        "userSettings",
        "deleteEnabled",
        "Members can delete their messages",
    ),
    (
        "userSettings",
        "giphyEnabled",
        "Allow emoji, memes, GIFs and stickers",
    ),
    ("userSettings", "createTopic", "Members can create channels"),
    ("userSettings", "updateTopic", "Members can edit channels"),
    (
        "userSettings",
        "deleteTopic",
        "Members can delete and restore channels",
    ),
    (
        "userSettings",
        "createPrivateSpace",
        "Members can create private channels",
    ),
    (
        "userSettings",
        "installApp",
        "Members can add and remove apps",
    ),
    ("userSettings", "teamMention", "Show team @mentions"),
    ("userSettings", "channelMention", "Show channel @mentions"),
    ("guestSettings", "createTopic", "Guests can create channels"),
    ("guestSettings", "updateTopic", "Guests can edit channels"),
    ("guestSettings", "deleteTopic", "Guests can delete channels"),
];

/// The settings rows for a stored `{userSettings, guestSettings}`.
fn settings_rows(stored: &Value) -> Vec<SettingData> {
    SETTINGS
        .iter()
        .filter_map(|(group, key, label)| {
            let v = stored.get(*group)?.get(*key)?;
            let on = match v {
                Value::Bool(b) => *b,
                Value::String(s) => s.eq_ignore_ascii_case("true"),
                _ => return None,
            };
            Some(SettingData {
                key: (*key).into(),
                group: if *group == "userSettings" {
                    "user"
                } else {
                    "guest"
                },
                label,
                on,
            })
        })
        .collect()
}

pub(crate) struct State {
    pub(crate) page: Page,
    loaded: bool,
    loading: bool,
    error: String,
    notice: String,
    discover: Vec<DiscoverTeam>,
    requested: Vec<DiscoverTeam>,
    join_info: String,
    communities: Vec<Community>,
    community: Option<u64>,
    posts: Vec<EngagePost>,
    posts_more: bool,
    dialog: DialogData,
    /// People ticked for a new private channel.
    picks: Vec<String>,
    /// The tag open in the tags dialog.
    tag: Option<String>,
    tag_list: Vec<Tag>,
    names: HashMap<String, String>,
    guard: Option<Guard>,
    /// Bumped per Discover and per member search, so a slower answer to an older query is
    /// dropped.
    discover_seq: u64,
    member_seq: u64,
}

impl Default for State {
    fn default() -> Self {
        State {
            page: Page::default(),
            loaded: false,
            loading: false,
            error: String::new(),
            notice: String::new(),
            discover: vec![],
            requested: vec![],
            join_info: String::new(),
            communities: vec![],
            community: None,
            posts: vec![],
            posts_more: false,
            dialog: DialogData::default(),
            picks: vec![],
            tag: None,
            tag_list: vec![],
            names: HashMap::new(),
            guard: Guard::from_env(),
            discover_seq: 0,
            member_seq: 0,
        }
    }
}

fn discover_row(t: &DiscoverTeam, requested: bool) -> DiscoverData {
    DiscoverData {
        group: t.group_id.clone(),
        name: t.name.clone(),
        description: t.description.clone(),
        people: t.members,
        access: t.access.clone(),
        requested,
    }
}

fn level_name(l: NotifyLevel) -> &'static str {
    match l {
        NotifyLevel::BannerAndFeed => "banner",
        NotifyLevel::Feed => "feed",
        NotifyLevel::Off => "off",
    }
}

fn senders_name(s: EmailSenders) -> &'static str {
    match s {
        EmailSenders::Anyone => "Anyone",
        EmailSenders::Members => "Members",
        EmailSenders::Domains => "Domains",
    }
}

impl State {
    fn person(&self, mri: &str, role: Role, me: &str, picked: bool) -> PersonData {
        PersonData {
            mri: mri.into(),
            name: self
                .names
                .get(mri)
                .cloned()
                .unwrap_or_else(|| "Unknown person".into()),
            detail: String::new(),
            role: role_name(role),
            me: mri == me,
            picked,
        }
    }

    fn data(&self) -> TeamsData {
        TeamsData {
            teams: self.page.rows(),
            stats: self.page.stats(),
            loading: self.loading,
            error: self.error.clone(),
            notice: self.notice.clone(),
            discover: self
                .discover
                .iter()
                .map(|t| discover_row(t, self.requested.iter().any(|r| r.group_id == t.group_id)))
                .collect(),
            requests: self
                .requested
                .iter()
                .map(|t| discover_row(t, true))
                .collect(),
            join_info: self.join_info.clone(),
            communities: self
                .communities
                .iter()
                .map(|c| CommunityData {
                    id: c.id.to_string(),
                    name: c.name.clone(),
                    description: c.description.clone(),
                    people: c.members,
                    company: c.is_company_wide,
                })
                .collect(),
            community_name: self
                .communities
                .iter()
                .find(|c| Some(c.id) == self.community)
                .map(|c| c.name.clone())
                .unwrap_or_default(),
            posts: self
                .posts
                .iter()
                .map(|p| PostData {
                    id: p.id.to_string(),
                    sender: p.sender_name.clone(),
                    time: chat_time(p.created_ms),
                    text: p.text.clone(),
                    likes: p.likes,
                    reply: p.replied_to_id.is_some(),
                    url: p.web_url.clone(),
                })
                .collect(),
            posts_more: self.posts_more,
            dialog: self.dialog.clone(),
        }
    }

    /// Refuses a write that does not target the sandbox team (when a sandbox is configured).
    fn allow_team(&self, team_thread_id: &str) -> anyhow::Result<()> {
        match &self.guard {
            Some(g) if g.team != team_thread_id => {
                anyhow::bail!("refused: only the sandbox team may be changed in this run")
            }
            _ => Ok(()),
        }
    }

    fn allow_channel(&self, channel_id: &str) -> anyhow::Result<()> {
        let Some(g) = &self.guard else { return Ok(()) };
        let in_sandbox = self
            .page
            .channel(channel_id)
            .is_some_and(|(t, _)| t.id == g.team)
            || g.channels.iter().any(|c| c == channel_id);
        if in_sandbox {
            Ok(())
        } else {
            anyhow::bail!("refused: only the sandbox team may be changed in this run")
        }
    }

    fn team_ref(&self, thread_id: &str) -> anyhow::Result<TeamRef> {
        self.page
            .team(thread_id)
            .map(TeamEntry::team_ref)
            .ok_or_else(|| anyhow::anyhow!("that team is not in your list any more"))
    }

    fn channel_ref(&self, channel_id: &str) -> anyhow::Result<ChannelRef> {
        self.page
            .channel(channel_id)
            .map(|(_, c)| ChannelRef::from_entry(c))
            .ok_or_else(|| anyhow::anyhow!("that channel is not in your list any more"))
    }
}

fn is_owner(t: &TeamEntry) -> bool {
    t.role == Role::Owner
}

impl Engine {
    fn manage_state<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.manage.lock().unwrap())
    }

    /// Pushes the whole Teams UI state.
    pub(crate) fn manage_push(&self) {
        let d = self.manage_state(|s| s.data());
        self.ui.teams_store(d);
    }

    /// Hidden teams and channels, for the sidebar list.
    pub(super) fn hidden_ids(&self) -> std::collections::HashSet<String> {
        self.manage_state(|s| s.page.hidden_ids())
    }

    /// Pinned channel ids, for the sidebar list (they come first).
    pub(super) fn pinned_ids(&self) -> Vec<String> {
        self.manage_state(|s| s.page.pinned.clone())
    }

    #[cfg(test)]
    pub(crate) fn manage_set_entries(&self, entries: Vec<TeamEntry>) {
        self.manage_state(|s| {
            s.page.entries = entries;
            s.loaded = true;
        });
        self.mark(|d| d.teams = true);
        self.manage_push();
    }

    /// Reloads the list (teams, channels and pinned channels) from the service.
    async fn manage_reload(self: &Arc<Self>) -> anyhow::Result<()> {
        let entries = self.client.teams_and_channels().await?;
        let pinned = self
            .client
            .pinned_channels()
            .await
            .map(|p| p.channel_ids)
            .unwrap_or_default();
        self.manage_state(|s| {
            s.page.entries = entries;
            s.page.pinned = pinned;
            s.loaded = true;
        });
        self.mark(|d| d.teams = true);
        Ok(())
    }

    /// Loads the list in the background at start, so the sidebar can honor hidden channels.
    pub(super) fn start_manage(self: &Arc<Self>) {
        let e = self.clone();
        tokio::spawn(async move {
            if e.manage_reload().await.is_ok() {
                e.manage_push();
            }
        });
    }

    /// Runs one command; failures are shown in the page or the open dialog, not as a banner.
    pub(super) async fn manage_cmd(self: &Arc<Self>, c: TeamsCmd) -> anyhow::Result<()> {
        if let Err(err) = self.manage_run(&c).await {
            tracing::warn!(%err, cmd = %c.name, "teams command failed");
            let text = err.to_string();
            self.manage_state(|s| {
                s.loading = false;
                s.dialog.busy = false;
                if s.dialog.kind.is_empty() {
                    s.error = text;
                } else {
                    s.dialog.error = text;
                }
            });
        }
        self.manage_push();
        Ok(())
    }

    /// Display names for people we do not know yet.
    async fn manage_names(&self, mris: &[String]) {
        let missing: Vec<String> = self.manage_state(|s| {
            mris.iter()
                .filter(|m| m.starts_with("8:") && !s.names.contains_key(*m))
                .cloned()
                .collect()
        });
        if missing.is_empty() {
            return;
        }
        if let Ok(profiles) = self.client.profiles(&missing).await {
            self.manage_state(|s| {
                for p in profiles {
                    s.names.insert(p.mri, p.display_name);
                }
            });
        }
    }

    /// Loads the roster of a team or channel into the dialog.
    async fn manage_roster(&self, thread_id: &str) -> anyhow::Result<()> {
        let roster: Vec<_> = self
            .client
            .roster(thread_id)
            .await?
            .into_iter()
            .filter(|m| !m.is_bot)
            .collect();
        let mris: Vec<String> = roster.iter().map(|m| m.mri.clone()).collect();
        self.manage_names(&mris).await;
        let me = self.me.clone();
        self.manage_state(|s| {
            let picks = s.picks.clone();
            s.dialog.people = roster
                .iter()
                .map(|m| s.person(&m.mri, m.role, &me, picks.contains(&m.mri)))
                .collect();
        });
        Ok(())
    }

    async fn manage_run(self: &Arc<Self>, c: &TeamsCmd) -> anyhow::Result<()> {
        let st = |f: &dyn Fn(&mut State)| self.manage_state(|s| f(s));
        match c.name.as_str() {
            "open" => {
                st(&|s| {
                    s.error.clear();
                    s.loading = true;
                });
                self.manage_push();
                let r = self.manage_reload().await;
                st(&|s| s.loading = false);
                r?;
            }
            "refresh" => {
                self.manage_reload().await?;
                self.manage_state(|s| s.notice.clear());
            }
            "filter" => st(&|s| s.page.filter = c.arg(0).into()),
            "sort" => st(&|s| s.page.sort = c.arg(0).into()),
            "query" => st(&|s| s.page.query = c.arg(0).into()),
            "expand" => st(&|s| {
                let id = c.arg(0).to_string();
                if !s.page.expanded.remove(&id) {
                    s.page.expanded.insert(id);
                }
            }),
            // ---- T1.3 show, hide, pin ----
            "team-show" => {
                let (id, on) = (c.arg(0).to_string(), c.flag(1));
                self.manage_state(|s| s.allow_team(&id))?;
                st(&|s| {
                    if let Some(t) = s.page.entries.iter_mut().find(|t| t.id == id) {
                        t.is_shown = on;
                    }
                });
                self.mark(|d| d.teams = true);
                self.manage_push();
                self.client.set_team_shown(&id, on).await?;
                self.manage_reload().await?;
            }
            "chan-show" => {
                let (id, on) = (c.arg(0).to_string(), c.flag(1));
                let (r, general) = self.manage_state(|s| {
                    s.allow_channel(&id)?;
                    let general = s.page.channel(&id).is_some_and(|(_, c)| c.is_general);
                    Ok::<_, anyhow::Error>((s.channel_ref(&id)?, general))
                })?;
                st(&|s| {
                    for t in &mut s.page.entries {
                        for ch in t.channels.iter_mut().filter(|ch| ch.id == id) {
                            ch.is_shown = on;
                        }
                    }
                });
                self.mark(|d| d.teams = true);
                self.manage_push();
                self.client.set_channel_shown(&r, general, on).await?;
                self.manage_reload().await?;
            }
            "chan-pin" => {
                let (id, on) = (c.arg(0).to_string(), c.flag(1));
                self.manage_state(|s| s.allow_channel(&id))?;
                st(&|s| {
                    if on {
                        s.page.pinned.push(id.clone());
                    } else {
                        s.page.pinned.retain(|p| p != &id);
                    }
                });
                self.mark(|d| d.teams = true);
                self.manage_push();
                let p = self.client.set_channel_pinned(&id, on).await?;
                self.manage_state(|s| s.page.pinned = p.channel_ids);
                self.manage_reload().await?;
            }
            "chan-default" => {
                let id = c.arg(0).to_string();
                self.manage_state(|s| s.allow_channel(&id))?;
                self.client
                    .set_channel_default_shown(&id, c.flag(1))
                    .await?;
                self.manage_notice("Updated");
            }
            "leave-team" => {
                let id = c.arg(0).to_string();
                let t = self.manage_state(|s| {
                    s.allow_team(&id)?;
                    s.team_ref(&id)
                })?;
                self.client.remove_team_member(&t, &self.me).await?;
                self.manage_reload().await?;
                self.manage_notice("You left the team");
            }
            "leave-channel" => {
                let id = c.arg(0).to_string();
                let (t, r) = self.manage_state(|s| {
                    s.allow_channel(&id)?;
                    let t = s
                        .page
                        .channel(&id)
                        .map(|(t, _)| t.team_ref())
                        .ok_or_else(|| anyhow::anyhow!("channel not found"))?;
                    Ok::<_, anyhow::Error>((t, s.channel_ref(&id)?))
                })?;
                self.client
                    .remove_private_channel_member(&t, &r, &self.me)
                    .await?;
                self.manage_reload().await?;
                self.manage_notice("You left the channel");
            }
            // ---- dialogs ----
            "dialog" => self.manage_open_dialog(c).await?,
            "dialog-close" => st(&|s| {
                s.dialog = DialogData::default();
                s.picks.clear();
                s.tag = None;
            }),
            "channel-create" => self.manage_channel_create(c).await?,
            "channel-edit" => {
                let (team, ch) = (c.arg(0).to_string(), c.arg(1).to_string());
                let (t, r) = self.manage_state(|s| {
                    s.allow_channel(&ch)?;
                    Ok::<_, anyhow::Error>((s.team_ref(&team)?, s.channel_ref(&ch)?))
                })?;
                let (name, desc) = (c.arg(2).trim(), c.arg(3).trim());
                if name.is_empty() {
                    anyhow::bail!("Give the channel a name");
                }
                self.client
                    .edit_channel(&t, &r, Some(name), Some(desc).filter(|d| !d.is_empty()))
                    .await?;
                self.manage_reload().await?;
                self.manage_close_dialog("Channel updated");
            }
            "channel-delete" => {
                let (team, ch) = (c.arg(0).to_string(), c.arg(1).to_string());
                let (t, r) = self.manage_state(|s| {
                    s.allow_channel(&ch)?;
                    Ok::<_, anyhow::Error>((s.team_ref(&team)?, s.channel_ref(&ch)?))
                })?;
                self.client.delete_channel(&t, &r).await?;
                self.manage_reload().await?;
                self.manage_notice("Channel deleted. You can restore it for 21 days.");
            }
            "channel-restore" => {
                let (team, ch) = (c.arg(0).to_string(), c.arg(1).to_string());
                self.manage_state(|s| s.allow_team(&team))?;
                self.client.restore_channel(&team, &ch).await?;
                self.manage_reload().await?;
                let list = self.client.channels_in_state(&team, "deleted").await?;
                self.manage_state(|s| s.dialog.deleted = deleted_rows(list));
                self.manage_notice("Channel restored");
            }
            "channel-archive" => {
                let (ch, on) = (c.arg(0).to_string(), c.flag(1));
                self.manage_state(|s| s.allow_channel(&ch))?;
                self.client.set_archival(&ch, false, on).await?;
                self.manage_reload().await?;
                self.manage_notice(if on {
                    "Channel archived"
                } else {
                    "Channel restored from the archive"
                });
            }
            "mod-set" => self.manage_moderation(c).await?,
            "roster" => {
                let team = c.arg(0).to_string();
                self.manage_roster(&team).await?;
            }
            "pick" => {
                let mri = c.arg(0).to_string();
                st(&|s| {
                    if !s.picks.contains(&mri) {
                        s.picks.push(mri.clone());
                    } else {
                        s.picks.retain(|p| p != &mri);
                    }
                    let picks = s.picks.clone();
                    for p in &mut s.dialog.people {
                        p.picked = picks.contains(&p.mri);
                    }
                });
            }
            "member-search" => self.manage_member_search(c.arg(0)).await?,
            "member-add" | "member-role" | "member-remove" => self.manage_member_change(c).await?,
            "tag-create" | "tag-rename" | "tag-delete" | "tag-select" | "tag-add"
            | "tag-remove" => self.manage_tag(c).await?,
            "email-create" | "email-remove" | "email-senders" | "email-copy" => {
                self.manage_email(c).await?
            }
            "notify-set" => {
                let ch = self.manage_state(|s| s.dialog.channel.clone());
                let level = match c.arg(0) {
                    "banner" => NotifyLevel::BannerAndFeed,
                    "off" => NotifyLevel::Off,
                    _ => NotifyLevel::Feed,
                };
                let n = ChannelNotify {
                    level,
                    include_replies: c.flag(1),
                };
                self.client.set_channel_notifications(&ch, n).await?;
                self.manage_state(|s| {
                    s.dialog.notify_level = level_name(level).into();
                    s.dialog.notify_replies = n.include_replies && level != NotifyLevel::Off;
                });
            }
            "team-create" => self.manage_team_create(c).await?,
            "team-edit" => {
                let team = self.manage_state(|s| s.dialog.team.clone());
                self.manage_state(|s| s.allow_team(&team))?;
                let (name, desc) = (c.arg(0).trim(), c.arg(1).trim());
                self.client
                    .edit_team(
                        &team,
                        Some(name).filter(|n| !n.is_empty()),
                        Some(desc).filter(|d| !d.is_empty()),
                        None,
                    )
                    .await?;
                self.manage_reload().await?;
                self.manage_notice("Team updated");
            }
            "team-setting" => {
                let team = self.manage_state(|s| s.dialog.team.clone());
                self.manage_state(|s| s.allow_team(&team))?;
                let (group, key, on) = (c.arg(0).to_string(), c.arg(1).to_string(), c.flag(2));
                let one = json!({ key.as_str(): on });
                let (user, guest) = if group == "guest" {
                    (None, Some(&one))
                } else {
                    (Some(&one), None)
                };
                self.client.set_team_settings(&team, user, guest).await?;
                let stored = self.client.team_settings(&team).await?;
                self.manage_state(|s| s.dialog.settings = settings_rows(&stored));
            }
            "team-code" => {
                let team = self.manage_state(|s| s.dialog.team.clone());
                self.manage_state(|s| s.allow_team(&team))?;
                let code = if c.arg(0) == "remove" {
                    self.client.remove_team_join_code(&team).await?;
                    String::new()
                } else {
                    self.client.reset_team_join_code(&team).await?
                };
                self.manage_state(|s| s.dialog.team_code = code);
            }
            "team-archive" => {
                let team = c.arg(0).to_string();
                self.manage_state(|s| s.allow_team(&team))?;
                if c.flag(1) {
                    self.client.archive_team(&team).await?;
                } else {
                    self.client.unarchive_team(&team).await?;
                }
                self.manage_reload().await?;
                self.manage_notice(if c.flag(1) {
                    "Team archived"
                } else {
                    "Team restored"
                });
            }
            "team-delete" => {
                let team = c.arg(0).to_string();
                self.manage_state(|s| s.allow_team(&team))?;
                self.client.delete_team(&team).await?;
                self.manage_reload().await?;
                self.manage_close_dialog("Team deleted");
            }
            "team-link" => self.manage_copy_team_link(c.arg(0)).await?,
            "chan-link" => self.manage_copy_channel_link(c.arg(0)).await?,
            // ---- T1.2 discover and join ----
            "discover" => {
                let q = c.arg(0).trim().to_string();
                let seq = self.manage_state(|s| {
                    s.discover_seq += 1;
                    s.discover_seq
                });
                let found = if q.is_empty() {
                    self.client.suggested_teams(Access::Public).await?
                } else {
                    self.client.find_teams(&q, Access::Public, "").await?
                };
                self.manage_state(|s| {
                    if s.discover_seq == seq {
                        s.discover = found;
                    }
                });
            }
            "join" => {
                let group = c.arg(0).to_string();
                if self.manage_state(|s| s.guard.is_some()) {
                    anyhow::bail!("refused: joining teams is off in this run");
                }
                self.client.join_team(&group).await?;
                let found =
                    self.manage_state(|s| s.discover.iter().find(|t| t.group_id == group).cloned());
                self.manage_state(|s| {
                    if let Some(t) = found.filter(|t| t.access != "Public") {
                        s.requested.push(t);
                    }
                });
                self.manage_reload().await?;
                self.manage_notice("Joined, or the request was sent to the team owners");
            }
            "join-cancel" => {
                let group = c.arg(0).to_string();
                self.client.cancel_join_request(&group).await?;
                self.manage_state(|s| s.requested.retain(|t| t.group_id != group));
            }
            "join-code" => {
                let code = c.arg(0).trim().to_string();
                if code.is_empty() {
                    anyhow::bail!("Enter the team code");
                }
                // In a sandbox run only the sandbox team's own code may be tried.
                if let Some(g) = self.manage_state(|s| s.guard.clone()) {
                    let own = self.client.team_join_code(&g.team).await?;
                    if own.as_deref() != Some(code.as_str()) {
                        anyhow::bail!(
                            "refused: only the sandbox team's own code may be used in this run"
                        );
                    }
                }
                let info = self.client.join_team_with_code(&code, true).await?;
                self.manage_reload().await?;
                let text = match info {
                    Some(i) if i.needs_approval => {
                        format!("Request sent to the owners of {}", i.name)
                    }
                    Some(i) => format!("You joined {}", i.name),
                    None => "Done".into(),
                };
                self.manage_state(|s| s.join_info = text);
            }
            // ---- T3.1 communities ----
            "communities" => {
                let list = self.client.communities().await?;
                self.manage_state(|s| s.communities = list);
            }
            "community-open" => {
                let id: u64 = c.arg(0).parse()?;
                self.manage_state(|s| {
                    s.community = Some(id);
                    s.posts.clear();
                    s.posts_more = false;
                });
                self.manage_feed(None).await?;
            }
            "open-post" => {
                let url = c.arg(0);
                if url.starts_with("https://") {
                    let _ = crate::external::open(url);
                }
            }
            "community-more" => {
                let older = self.manage_state(|s| s.posts.last().map(|p| p.id));
                self.manage_feed(older).await?;
            }
            _ => {}
        }
        Ok(())
    }

    fn manage_notice(&self, text: &str) {
        self.manage_state(|s| s.notice = text.to_string());
    }

    fn manage_close_dialog(&self, notice: &str) {
        self.manage_state(|s| {
            s.dialog = DialogData::default();
            s.picks.clear();
            s.notice = notice.into();
        });
        self.ui.teams_dialog(String::new());
    }

    async fn manage_feed(&self, older_than: Option<u64>) -> anyhow::Result<()> {
        let Some(id) = self.manage_state(|s| s.community) else {
            return Ok(());
        };
        const PAGE: u32 = 20;
        let page = self.client.community_feed(id, PAGE, older_than).await?;
        self.manage_state(|s| {
            // Two quick "More" ask for the same page: only the first answer extends the feed
            // (the second finds the last post moved), and a feed switched meanwhile keeps its own.
            if s.community != Some(id) || s.posts.last().map(|p| p.id) != older_than {
                return;
            }
            s.posts_more = page.len() as u32 >= PAGE;
            s.posts.extend(page);
        });
        Ok(())
    }

    async fn manage_open_dialog(self: &Arc<Self>, c: &TeamsCmd) -> anyhow::Result<()> {
        let (kind, team, ch) = (
            c.arg(0).to_string(),
            c.arg(1).to_string(),
            c.arg(2).to_string(),
        );
        let (team_name, ch_name, ch_kind, owner) = self.manage_state(|s| {
            let t = s.page.team(&team);
            let ch = s.page.channel(&ch).map(|(_, c)| c);
            (
                t.map(|t| t.name.clone()).unwrap_or_default(),
                ch.map(|c| c.name.clone()).unwrap_or_default(),
                ch.map(|c| match c.kind {
                    ChannelKind::Standard => "standard",
                    ChannelKind::Private => "private",
                    ChannelKind::Shared => "shared",
                })
                .unwrap_or("standard")
                .to_string(),
                t.is_some_and(is_owner),
            )
        });
        self.manage_state(|s| {
            let access = s
                .page
                .team(&team)
                .map(|t| {
                    if t.access == Access::Public {
                        "Public"
                    } else {
                        "Private"
                    }
                })
                .unwrap_or("Private");
            let (name, description) = match kind.as_str() {
                "edit-channel" => (ch_name.clone(), String::new()),
                "manage-team" => (
                    team_name.clone(),
                    s.page
                        .team(&team)
                        .map(|t| t.description.clone())
                        .unwrap_or_default(),
                ),
                _ => (String::new(), String::new()),
            };
            s.dialog = DialogData {
                kind: kind.clone(),
                team: team.clone(),
                team_name,
                channel: ch.clone(),
                channel_name: ch_name,
                channel_kind: ch_kind,
                name,
                description,
                access: access.into(),
                owner,
                // What a channel has before anything was changed; the load below replaces them.
                post_perm: "Everyone".into(),
                reply_perm: "Everyone".into(),
                pin_perm: "Everyone".into(),
                allow_bots: true,
                allow_connectors: true,
                notify_level: "feed".into(),
                email_senders: "Anyone".into(),
                ..DialogData::default()
            };
            s.picks.clear();
            s.tag = None;
        });
        self.ui.teams_dialog(kind.clone());
        self.manage_push();
        match kind.as_str() {
            "moderation" => {
                let m = self.client.moderation(&ch).await?;
                self.manage_state(|s| {
                    s.dialog.post_perm = m.post_permissions;
                    s.dialog.reply_perm = m.reply_permissions;
                    s.dialog.pin_perm = m.pin_permissions;
                    s.dialog.allow_bots = m.allow_bots;
                    s.dialog.allow_connectors = m.allow_connectors;
                });
                Ok(())
            }
            "members" => {
                let thread = if ch.is_empty() {
                    team.clone()
                } else {
                    ch.clone()
                };
                self.manage_roster(&thread).await
            }
            "tags" => self.manage_load_tags(&team).await,
            "email" => {
                let e = self.client.channel_email(&ch).await?;
                self.manage_state(|s| {
                    s.dialog.email = e.as_ref().map(|e| e.address.clone()).unwrap_or_default();
                    s.dialog.email_senders = e
                        .as_ref()
                        .map_or("Anyone", |e| senders_name(e.senders))
                        .into();
                });
                Ok(())
            }
            "notify" => {
                let n = self.client.channel_notifications(&ch).await?;
                self.manage_state(|s| {
                    s.dialog.notify_level = level_name(n.level).into();
                    s.dialog.notify_replies = n.include_replies;
                });
                Ok(())
            }
            "manage-team" => {
                let stored = self.client.team_settings(&team).await;
                let code = self.client.team_join_code(&team).await.unwrap_or_default();
                self.manage_state(|s| {
                    if let Ok(v) = &stored {
                        s.dialog.settings = settings_rows(v);
                    }
                    s.dialog.team_code = code.unwrap_or_default();
                });
                Ok(())
            }
            "deleted" => {
                let list = self.client.channels_in_state(&team, "deleted").await?;
                self.manage_state(|s| s.dialog.deleted = deleted_rows(list));
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn manage_channel_create(self: &Arc<Self>, c: &TeamsCmd) -> anyhow::Result<()> {
        let team = c.arg(0).to_string();
        let name = c.arg(1).trim().to_string();
        let desc = c.arg(2).trim().to_string();
        let kind = match c.arg(3) {
            "private" => ChannelKind::Private,
            "shared" => ChannelKind::Shared,
            _ => ChannelKind::Standard,
        };
        if name.is_empty() {
            anyhow::bail!("Give the channel a name");
        }
        let t = self.manage_state(|s| {
            s.allow_team(&team)?;
            s.team_ref(&team)
        })?;
        self.manage_state(|s| s.dialog.busy = true);
        self.manage_push();
        let id = if kind == ChannelKind::Standard {
            let id = self
                .client
                .create_standard_channel(&t, &name, &desc, c.flag(4))
                .await?;
            if c.flag(5) {
                self.client.set_channel_default_shown(&id, true).await?;
            }
            id
        } else {
            let id = self
                .client
                .create_private_channel(&t, &name, &desc, kind)
                .await?;
            let picks: Vec<(String, Role)> = self
                .manage_state(|s| s.picks.clone())
                .into_iter()
                .filter(|m| m != &self.me)
                .map(|m| (m, Role::Member))
                .collect();
            if !picks.is_empty() {
                let r = ChannelRef {
                    id: id.clone(),
                    kind,
                };
                self.client
                    .set_private_channel_members(&t, &r, &picks)
                    .await?;
            }
            id
        };
        tracing::info!(channel = %id, "channel created");
        self.manage_reload().await?;
        self.manage_close_dialog("Channel created");
        Ok(())
    }

    async fn manage_moderation(&self, c: &TeamsCmd) -> anyhow::Result<()> {
        let ch = self.manage_state(|s| s.dialog.channel.clone());
        self.manage_state(|s| s.allow_channel(&ch))?;
        let mut m: Moderation = self.manage_state(|s| Moderation {
            post_permissions: s.dialog.post_perm.clone(),
            reply_permissions: s.dialog.reply_perm.clone(),
            pin_permissions: s.dialog.pin_perm.clone(),
            allow_bots: s.dialog.allow_bots,
            allow_connectors: s.dialog.allow_connectors,
        });
        let v = c.arg(1);
        match c.arg(0) {
            "post" => m.post_permissions = v.into(),
            "reply" => m.reply_permissions = v.into(),
            "pin" => m.pin_permissions = v.into(),
            "bots" => m.allow_bots = c.flag(1),
            "connectors" => m.allow_connectors = c.flag(1),
            _ => return Ok(()),
        }
        self.client.set_moderation(&ch, &m).await?;
        self.manage_state(|s| {
            s.dialog.post_perm = m.post_permissions;
            s.dialog.reply_perm = m.reply_permissions;
            s.dialog.pin_perm = m.pin_permissions;
            s.dialog.allow_bots = m.allow_bots;
            s.dialog.allow_connectors = m.allow_connectors;
        });
        Ok(())
    }

    async fn manage_member_search(&self, q: &str) -> anyhow::Result<()> {
        let q = q.trim();
        let seq = self.manage_state(|s| {
            s.member_seq += 1;
            s.member_seq
        });
        if q.chars().count() < 2 {
            self.manage_state(|s| s.dialog.candidates.clear());
            return Ok(());
        }
        let found = self.client.search_people(&self.me, q).await?;
        self.manage_state(|s| {
            if s.member_seq != seq {
                return;
            }
            let have: Vec<String> = s.dialog.people.iter().map(|p| p.mri.clone()).collect();
            s.dialog.candidates = found
                .into_iter()
                .filter(|p| p.mri.starts_with("8:") && !have.contains(&p.mri))
                .map(|p| {
                    s.names.insert(p.mri.clone(), p.display_name.clone());
                    PersonData {
                        detail: p.email.or(p.job_title).unwrap_or_default(),
                        name: p.display_name,
                        mri: p.mri,
                        ..PersonData::default()
                    }
                })
                .collect();
        });
        Ok(())
    }

    async fn manage_member_change(self: &Arc<Self>, c: &TeamsCmd) -> anyhow::Result<()> {
        let (team, ch, ckind) = self.manage_state(|s| {
            (
                s.dialog.team.clone(),
                s.dialog.channel.clone(),
                s.dialog.channel_kind.clone(),
            )
        });
        let private = !ch.is_empty() && ckind != "standard";
        let (t, r) = self.manage_state(|s| {
            s.allow_team(&team)?;
            if !ch.is_empty() {
                s.allow_channel(&ch)?;
            }
            Ok::<_, anyhow::Error>((s.team_ref(&team)?, s.channel_ref(&ch).ok()))
        })?;
        let mri = c.arg(0).to_string();
        let role = match c.arg(1) {
            "owner" => Role::Owner,
            "guest" => Role::Guest,
            _ => Role::Member,
        };
        match (c.name.as_str(), private, r) {
            ("member-remove", true, Some(r)) => {
                self.client
                    .remove_private_channel_member(&t, &r, &mri)
                    .await?
            }
            ("member-remove", _, _) => self.client.remove_team_member(&t, &mri).await?,
            (_, true, Some(r)) => {
                self.client
                    .set_private_channel_members(&t, &r, &[(mri, role)])
                    .await?
            }
            _ => self.client.set_team_members(&t, &[(mri, role)]).await?,
        }
        let thread = if ch.is_empty() { team } else { ch };
        self.manage_roster(&thread).await?;
        self.manage_state(|s| s.dialog.candidates.clear());
        Ok(())
    }

    async fn manage_load_tags(&self, team: &str) -> anyhow::Result<()> {
        let tags = self.client.tags(team).await?;
        let selected = self.manage_state(|s| s.tag.clone());
        self.manage_state(|s| {
            s.dialog.tags = tags
                .iter()
                .map(|t| TagData {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    description: t.description.clone(),
                    count: t.member_count,
                    mine: t.is_member,
                    selected: selected.as_deref() == Some(t.id.as_str()),
                })
                .collect();
            s.tag_list = tags;
        });
        Ok(())
    }

    async fn manage_load_tag_people(&self, team: &str, tag: &str) -> anyhow::Result<()> {
        let mris = self.client.tag_members(team, tag).await?;
        self.manage_names(&mris).await;
        let me = self.me.clone();
        self.manage_state(|s| {
            s.dialog.tag_people = mris
                .iter()
                .map(|m| s.person(m, Role::Member, &me, false))
                .collect();
        });
        Ok(())
    }

    async fn manage_tag(self: &Arc<Self>, c: &TeamsCmd) -> anyhow::Result<()> {
        let team = self.manage_state(|s| s.dialog.team.clone());
        self.manage_state(|s| s.allow_team(&team))?;
        let selected = self.manage_state(|s| s.tag.clone());
        let tag_of =
            |id: &str| self.manage_state(|s| s.tag_list.iter().find(|t| t.id == id).cloned());
        match c.name.as_str() {
            "tag-create" => {
                let name = c.arg(0).trim();
                if name.is_empty() {
                    anyhow::bail!("Give the tag a name");
                }
                // A tag needs at least one member; we are the first.
                let t = self
                    .client
                    .create_tag(&team, name, c.arg(1).trim(), std::slice::from_ref(&self.me))
                    .await?;
                self.manage_state(|s| s.tag = Some(t.id.clone()));
                self.manage_load_tags(&team).await?;
                self.manage_load_tag_people(&team, &t.id).await?;
            }
            "tag-rename" => {
                // The version a change must quote moves with every change: read it fresh.
                self.manage_load_tags(&team).await?;
                let Some(t) = tag_of(c.arg(0)) else {
                    return Ok(());
                };
                self.client
                    .edit_tag(&team, &t, Some(c.arg(1).trim()), None)
                    .await?;
                self.manage_load_tags(&team).await?;
            }
            "tag-delete" => {
                self.client.delete_tag(&team, c.arg(0)).await?;
                self.manage_state(|s| {
                    s.tag = None;
                    s.dialog.tag_people.clear();
                });
                self.manage_load_tags(&team).await?;
            }
            "tag-select" => {
                let id = c.arg(0).to_string();
                self.manage_state(|s| s.tag = Some(id.clone()));
                self.manage_load_tags(&team).await?;
                self.manage_load_tag_people(&team, &id).await?;
            }
            "tag-add" => {
                let Some(t) = selected.as_deref().and_then(tag_of) else {
                    anyhow::bail!("Pick a tag first");
                };
                self.client
                    .add_to_tags(
                        &team,
                        &[c.arg(0).to_string()],
                        std::slice::from_ref(&t.name),
                    )
                    .await?;
                self.manage_load_tags(&team).await?;
                self.manage_load_tag_people(&team, &t.id).await?;
                self.manage_state(|s| s.dialog.candidates.clear());
            }
            _ => {
                // tag-remove: removing the last member deletes the tag.
                let Some(id) = selected else { return Ok(()) };
                self.client
                    .remove_from_tags(&team, &[c.arg(0).to_string()], std::slice::from_ref(&id))
                    .await?;
                self.manage_load_tags(&team).await?;
                let still = self.manage_state(|s| s.dialog.tags.iter().any(|t| t.id == id));
                if still {
                    self.manage_load_tag_people(&team, &id).await?;
                } else {
                    self.manage_state(|s| {
                        s.tag = None;
                        s.dialog.tag_people.clear();
                    });
                }
            }
        }
        Ok(())
    }

    async fn manage_email(&self, c: &TeamsCmd) -> anyhow::Result<()> {
        let ch = self.manage_state(|s| s.dialog.channel.clone());
        match c.name.as_str() {
            "email-copy" => {
                let a = self.manage_state(|s| s.dialog.email.clone());
                arboard::Clipboard::new()?.set_text(a)?;
                self.manage_notice("Address copied");
                return Ok(());
            }
            "email-remove" => {
                self.manage_state(|s| s.allow_channel(&ch))?;
                self.client.remove_channel_email(&ch).await?;
                self.manage_state(|s| s.dialog.email.clear());
            }
            _ => {
                self.manage_state(|s| s.allow_channel(&ch))?;
                let exists = self.manage_state(|s| !s.dialog.email.is_empty());
                let (senders, domains) = if c.name == "email-senders" {
                    let s = match c.arg(0) {
                        "Members" => EmailSenders::Members,
                        "Domains" => EmailSenders::Domains,
                        _ => EmailSenders::Anyone,
                    };
                    let domains: Vec<String> = c
                        .arg(1)
                        .split(',')
                        .map(|d| d.trim().to_string())
                        .filter(|d| !d.is_empty())
                        .collect();
                    if s == EmailSenders::Domains && domains.is_empty() {
                        anyhow::bail!("Enter at least one domain");
                    }
                    (s, domains)
                } else {
                    (EmailSenders::Anyone, vec![])
                };
                let made = self
                    .client
                    .set_channel_email(&ch, senders, &domains, exists || c.name == "email-senders")
                    .await?;
                if let Some(e) = made {
                    self.manage_state(|s| {
                        s.dialog.email = e.address;
                        s.dialog.email_senders = senders_name(e.senders).into();
                    });
                } else {
                    self.manage_state(|s| s.dialog.email_senders = senders_name(senders).into());
                }
            }
        }
        Ok(())
    }

    async fn manage_team_create(self: &Arc<Self>, c: &TeamsCmd) -> anyhow::Result<()> {
        let name = c.arg(0).trim().to_string();
        if name.is_empty() {
            anyhow::bail!("Give the team a name");
        }
        if self.manage_state(|s| s.guard.is_some()) {
            anyhow::bail!("refused: creating teams is off in this run");
        }
        let access = match c.arg(2) {
            "public" => Access::Public,
            "org" => Access::OrgWide,
            _ => Access::Private,
        };
        self.manage_state(|s| s.dialog.busy = true);
        self.manage_push();
        self.client.validate_team_name(&name).await?;
        self.client
            .create_team(&name, c.arg(1).trim(), access)
            .await?;
        self.manage_reload().await?;
        self.manage_close_dialog("Team created");
        Ok(())
    }

    async fn manage_copy_team_link(&self, team: &str) -> anyhow::Result<()> {
        let t = self.manage_state(|s| s.team_ref(team))?;
        let tenant = self.client.tenant_id().await?;
        let link = format!(
            "https://teams.microsoft.com/l/team/{}/conversations?groupId={}&tenantId={tenant}",
            urlenc(&t.thread_id),
            t.group_id
        );
        arboard::Clipboard::new()?.set_text(link)?;
        self.manage_notice("Link to the team copied");
        Ok(())
    }

    async fn manage_copy_channel_link(&self, channel: &str) -> anyhow::Result<()> {
        let (t, name) = self.manage_state(|s| {
            s.page
                .channel(channel)
                .map(|(t, c)| (t.team_ref(), c.name.clone()))
                .ok_or_else(|| anyhow::anyhow!("channel not found"))
        })?;
        let tenant = self.client.tenant_id().await?;
        let link = format!(
            "https://teams.microsoft.com/l/channel/{}/{}?groupId={}&tenantId={tenant}",
            urlenc(channel),
            urlenc(&name),
            t.group_id
        );
        arboard::Clipboard::new()?.set_text(link)?;
        self.manage_notice("Link to the channel copied");
        Ok(())
    }
}

fn urlenc(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn deleted_rows(list: Vec<teams_proto::teams::DeletedChannel>) -> Vec<DeletedData> {
    list.into_iter()
        .map(|d| DeletedData {
            when: if d.deleted_ms > 0 {
                format!("Deleted {}", chat_time(d.deleted_ms))
            } else {
                String::new()
            },
            id: d.id,
            name: d.name,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_commands_and_splits_arguments() {
        let c = TeamsCmd::parse("channel-create", &format!("t{SEP}Name{SEP}{SEP}private")).unwrap();
        assert_eq!(c.args, ["t", "Name", "", "private"]);
        assert!(TeamsCmd::parse("open", "").unwrap().args.is_empty());
        assert!(TeamsCmd::parse("nope", "").is_none());
    }

    #[test]
    fn settings_rows_list_only_stored_keys() {
        let stored = json!({
            "userSettings": {"editEnabled": true, "createTopic": "False", "unknown": true},
            "guestSettings": {"createTopic": false}
        });
        let rows = settings_rows(&stored);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            (rows[0].key.as_str(), rows[0].group, rows[0].on),
            ("editEnabled", "user", true)
        );
        assert!(!rows[1].on, "string values are read too");
        assert_eq!(rows[2].group, "guest");
    }

    #[test]
    fn the_sandbox_guard_refuses_other_teams() {
        let mut s = State {
            guard: Some(Guard {
                team: "t-sandbox".into(),
                group: "g".into(),
                channels: vec!["c-tests".into()],
            }),
            ..State::default()
        };
        assert!(s.allow_team("t-sandbox").is_ok());
        assert!(s.allow_team("t-real").is_err());
        assert!(s.allow_channel("c-tests").is_ok());
        assert!(s.allow_channel("c-real").is_err());
        s.guard = None;
        assert!(s.allow_team("anything").is_ok());
    }
}
