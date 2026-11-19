//! The "Your teams and channels" page: filter, sort and search over the teams list, and the plain
//! view data the UI thread turns into Slint models. Pure functions, no network.

use crate::data::{color_for, initials};
use std::collections::HashSet;
use teams_proto::teams::{Access, ChannelEntry, ChannelKind, Role, TeamEntry};

/// Separates the fields of a command argument that carries several values (a unit separator,
/// which no name or description contains).
pub const SEP: char = '\u{1f}';

pub fn role_name(r: Role) -> &'static str {
    match r {
        Role::Owner => "Owner",
        Role::Member => "Member",
        Role::Guest => "Guest",
    }
}

fn access_name(a: Access, org_wide: bool) -> &'static str {
    if org_wide {
        "Org-wide"
    } else if a == Access::Public {
        "Public"
    } else {
        "Private"
    }
}

fn kind_name(k: ChannelKind) -> &'static str {
    match k {
        ChannelKind::Standard => "standard",
        ChannelKind::Private => "private",
        ChannelKind::Shared => "shared",
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChannelRowData {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
    pub shown: bool,
    pub pinned: bool,
    pub muted: bool,
    pub archived: bool,
    pub general: bool,
    pub member: bool,
    pub owner: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TeamRowData {
    pub id: String,
    pub group: String,
    pub name: String,
    pub description: String,
    pub access: &'static str,
    pub role: &'static str,
    pub people: u32,
    pub channel_count: usize,
    pub shown_count: usize,
    pub shown: bool,
    pub archived: bool,
    pub expanded: bool,
    pub owner: bool,
    pub initials: String,
    pub color: u32,
    pub channels: Vec<ChannelRowData>,
}

/// What the page shows: filter, sort, search and which teams are open.
#[derive(Debug, Clone)]
pub struct Page {
    pub entries: Vec<TeamEntry>,
    pub pinned: Vec<String>,
    /// `all`, `owned`, `member`, `guest`, `hidden` or `archived`.
    pub filter: String,
    /// `az`, `za`, `people` or `channels`.
    pub sort: String,
    pub query: String,
    pub expanded: HashSet<String>,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            entries: vec![],
            pinned: vec![],
            filter: "all".into(),
            sort: "az".into(),
            query: String::new(),
            expanded: HashSet::new(),
        }
    }
}

fn live_channels(t: &TeamEntry) -> impl Iterator<Item = &ChannelEntry> {
    t.channels.iter().filter(|c| !c.is_deleted)
}

fn passes_filter(t: &TeamEntry, filter: &str) -> bool {
    match filter {
        "owned" => t.role == Role::Owner,
        "member" => t.role == Role::Member,
        "guest" => t.role == Role::Guest,
        "hidden" => !t.is_shown || live_channels(t).any(|c| !c.is_shown),
        "archived" => t.is_archived,
        _ => true,
    }
}

impl Page {
    fn channel_row(&self, c: &ChannelEntry) -> ChannelRowData {
        ChannelRowData {
            id: c.id.clone(),
            name: c.name.clone(),
            kind: kind_name(c.kind),
            shown: c.is_shown,
            pinned: c.is_pinned || self.pinned.contains(&c.id),
            muted: c.is_muted,
            archived: c.is_archived,
            general: c.is_general,
            member: c.is_member || c.kind == ChannelKind::Standard,
            owner: c.role == Role::Owner,
        }
    }

    /// The teams the page lists, after filter, search and sort.
    pub fn rows(&self) -> Vec<TeamRowData> {
        let q = self.query.trim().to_lowercase();
        let mut teams: Vec<&TeamEntry> = self
            .entries
            .iter()
            .filter(|t| !t.is_deleted && passes_filter(t, &self.filter))
            .filter(|t| {
                q.is_empty()
                    || t.name.to_lowercase().contains(&q)
                    || live_channels(t).any(|c| c.name.to_lowercase().contains(&q))
            })
            .collect();
        match self.sort.as_str() {
            "za" => teams.sort_by_key(|t| std::cmp::Reverse(t.name.to_lowercase())),
            "people" => {
                teams.sort_by_key(|t| (std::cmp::Reverse(t.member_count), t.name.to_lowercase()))
            }
            "channels" => teams.sort_by_key(|t| {
                (
                    std::cmp::Reverse(live_channels(t).count()),
                    t.name.to_lowercase(),
                )
            }),
            _ => teams.sort_by_key(|t| t.name.to_lowercase()),
        }
        teams
            .into_iter()
            .map(|t| {
                let name_hit = q.is_empty() || t.name.to_lowercase().contains(&q);
                let mut channels: Vec<ChannelRowData> = live_channels(t)
                    .filter(|c| name_hit || c.name.to_lowercase().contains(&q))
                    .map(|c| self.channel_row(c))
                    .collect();
                // General first, then pinned, then by name.
                channels.sort_by_key(|c| (!c.general, !c.pinned, c.name.to_lowercase()));
                let total = live_channels(t).count();
                TeamRowData {
                    id: t.id.clone(),
                    group: t.group_id.clone(),
                    name: t.name.clone(),
                    description: t.description.clone(),
                    access: access_name(t.access, t.is_org_wide),
                    role: role_name(t.role),
                    people: t.member_count,
                    channel_count: total,
                    shown_count: live_channels(t).filter(|c| c.is_shown).count(),
                    shown: t.is_shown,
                    archived: t.is_archived,
                    // A search that matched only channels opens the team.
                    expanded: self.expanded.contains(&t.id) || (!q.is_empty() && !name_hit),
                    owner: t.role == Role::Owner,
                    initials: initials(&t.name),
                    color: color_for(&t.id),
                    channels,
                }
            })
            .collect()
    }

    /// The Analytics tab: totals computed from the list.
    pub fn stats(&self) -> Vec<(String, String)> {
        let teams: Vec<&TeamEntry> = self.entries.iter().filter(|t| !t.is_deleted).collect();
        let channels: Vec<&ChannelEntry> = teams.iter().flat_map(|t| live_channels(t)).collect();
        let n = |c: usize| c.to_string();
        let count = |f: &dyn Fn(&TeamEntry) -> bool| teams.iter().filter(|t| f(t)).count();
        vec![
            ("Teams".into(), n(teams.len())),
            ("Teams you own".into(), n(count(&|t| t.role == Role::Owner))),
            (
                "Public teams".into(),
                n(count(&|t| t.access == Access::Public)),
            ),
            ("Archived teams".into(), n(count(&|t| t.is_archived))),
            ("Hidden teams".into(), n(count(&|t| !t.is_shown))),
            ("Channels".into(), n(channels.len())),
            (
                "Private and shared channels".into(),
                n(channels
                    .iter()
                    .filter(|c| c.kind != ChannelKind::Standard)
                    .count()),
            ),
            (
                "Hidden channels".into(),
                n(channels.iter().filter(|c| !c.is_shown).count()),
            ),
            (
                "Pinned channels".into(),
                n(channels
                    .iter()
                    .filter(|c| c.is_pinned || self.pinned.contains(&c.id))
                    .count()),
            ),
            (
                "Muted channels".into(),
                n(channels.iter().filter(|c| c.is_muted).count()),
            ),
        ]
    }

    /// Ids the sidebar should not list: hidden teams and hidden channels.
    pub fn hidden_ids(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for t in self.entries.iter().filter(|t| !t.is_deleted) {
            if !t.is_shown {
                out.insert(t.id.clone());
            }
            for c in live_channels(t).filter(|c| !c.is_shown) {
                out.insert(c.id.clone());
            }
        }
        out
    }

    pub fn team(&self, thread_id: &str) -> Option<&TeamEntry> {
        self.entries.iter().find(|t| t.id == thread_id)
    }

    /// The team a channel belongs to, and the channel.
    pub fn channel(&self, channel_id: &str) -> Option<(&TeamEntry, &ChannelEntry)> {
        self.entries.iter().find_map(|t| {
            t.channels
                .iter()
                .find(|c| c.id == channel_id)
                .map(|c| (t, c))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(id: &str, name: &str, shown: bool) -> ChannelEntry {
        ChannelEntry {
            id: id.into(),
            name: name.into(),
            kind: ChannelKind::Standard,
            is_general: name == "General",
            is_shown: shown,
            is_pinned: false,
            is_muted: false,
            is_archived: false,
            is_deleted: false,
            is_member: true,
            is_moderator: false,
            role: Role::Member,
            created_ms: 0,
        }
    }

    fn team(
        id: &str,
        name: &str,
        role: Role,
        people: u32,
        channels: Vec<ChannelEntry>,
    ) -> TeamEntry {
        TeamEntry {
            id: id.into(),
            group_id: format!("g-{id}"),
            name: name.into(),
            description: String::new(),
            access: Access::Private,
            is_org_wide: false,
            is_shown: true,
            is_collapsed: false,
            is_archived: false,
            is_deleted: false,
            role,
            member_count: people,
            owner_count: 1,
            email: String::new(),
            channels,
        }
    }

    fn page() -> Page {
        let mut archived = team(
            "t3",
            "Old project",
            Role::Guest,
            2,
            vec![ch("t3", "General", true)],
        );
        archived.is_archived = true;
        let mut deleted = team("t4", "Gone", Role::Owner, 1, vec![]);
        deleted.is_deleted = true;
        Page {
            entries: vec![
                team(
                    "t1",
                    "Zebra",
                    Role::Owner,
                    9,
                    vec![ch("t1", "General", true), ch("c2", "Roadmap", false)],
                ),
                team(
                    "t2",
                    "alpha",
                    Role::Member,
                    30,
                    vec![ch("t2", "General", true)],
                ),
                archived,
                deleted,
            ],
            ..Page::default()
        }
    }

    fn names(p: &Page) -> Vec<String> {
        p.rows().into_iter().map(|r| r.name).collect()
    }

    #[test]
    fn lists_teams_a_to_z_ignoring_case_and_skipping_deleted() {
        assert_eq!(names(&page()), ["alpha", "Old project", "Zebra"]);
    }

    #[test]
    fn sorts_z_to_a_by_people_and_by_channels() {
        let mut p = page();
        p.sort = "za".into();
        assert_eq!(names(&p), ["Zebra", "Old project", "alpha"]);
        p.sort = "people".into();
        assert_eq!(names(&p), ["alpha", "Zebra", "Old project"]);
        p.sort = "channels".into();
        assert_eq!(names(&p), ["Zebra", "alpha", "Old project"]);
    }

    #[test]
    fn filters_by_role_hidden_and_archived() {
        let mut p = page();
        p.filter = "owned".into();
        assert_eq!(names(&p), ["Zebra"]);
        p.filter = "member".into();
        assert_eq!(names(&p), ["alpha"]);
        p.filter = "guest".into();
        assert_eq!(names(&p), ["Old project"]);
        p.filter = "hidden".into();
        assert_eq!(names(&p), ["Zebra"], "a team with a hidden channel counts");
        p.filter = "archived".into();
        assert_eq!(names(&p), ["Old project"]);
    }

    #[test]
    fn search_matches_team_names_and_opens_a_team_for_a_channel_hit() {
        let mut p = page();
        p.query = "ALP".into();
        assert_eq!(names(&p), ["alpha"]);
        p.query = "road".into();
        let rows = p.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Zebra");
        assert!(rows[0].expanded, "a channel hit opens the team");
        assert_eq!(
            rows[0]
                .channels
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["Roadmap"]
        );
        assert_eq!(rows[0].channel_count, 2, "the count is the whole team");
    }

    #[test]
    fn channels_list_general_first_then_pinned_and_keep_hidden_ones() {
        let mut p = page();
        p.pinned = vec!["c2".into()];
        p.entries[0].channels.push(ch("c3", "Alpha plan", true));
        let t = &p.rows().into_iter().find(|r| r.name == "Zebra").unwrap();
        assert_eq!(
            t.channels
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["General", "Roadmap", "Alpha plan"]
        );
        assert!(!t.channels[1].shown && t.channels[1].pinned);
        assert_eq!((t.channel_count, t.shown_count), (3, 2));
    }

    #[test]
    fn hidden_ids_list_hidden_teams_and_channels() {
        let mut p = page();
        p.entries[1].is_shown = false;
        let h = p.hidden_ids();
        assert!(h.contains("t2") && h.contains("c2") && h.len() == 2);
    }

    #[test]
    fn stats_count_the_list() {
        let s: std::collections::HashMap<_, _> = page().stats().into_iter().collect();
        assert_eq!(s["Teams"], "3");
        assert_eq!(s["Teams you own"], "1");
        assert_eq!(s["Channels"], "4");
        assert_eq!(s["Hidden channels"], "1");
        assert_eq!(s["Archived teams"], "1");
    }
}
