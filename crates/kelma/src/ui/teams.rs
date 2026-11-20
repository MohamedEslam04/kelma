//! UI-thread side of the Teams management UI: fills the `TeamsStore` global.

use super::{UiHandle, model, rgb};
use crate::backend::{
    CommunityData, DeletedData, DialogData, DiscoverData, PostData, SettingData, TagData,
    TeamRowData, TeamsData, TeamsPersonData as PersonData,
};
use crate::{
    AppWindow, TChannel, TCommunity, TDeleted, TDiscover, TPerson, TPost, TSetting, TStat, TTag,
    TTeam, TeamsStore,
};
use slint::{ComponentHandle, ModelRc};

fn int(n: impl TryInto<i32>) -> i32 {
    n.try_into().unwrap_or(i32::MAX)
}

fn team(t: TeamRowData) -> TTeam {
    TTeam {
        id: t.id.into(),
        group: t.group.into(),
        name: t.name.into(),
        desc: t.description.into(),
        access: t.access.into(),
        role: t.role.into(),
        people: int(t.people),
        channel_count: int(t.channel_count),
        shown_count: int(t.shown_count),
        shown: t.shown,
        archived: t.archived,
        expanded: t.expanded,
        owner: t.owner,
        initials: t.initials.into(),
        color: rgb(t.color),
        channels: model(
            t.channels
                .into_iter()
                .map(|c| TChannel {
                    id: c.id.into(),
                    name: c.name.into(),
                    kind: c.kind.into(),
                    shown: c.shown,
                    pinned: c.pinned,
                    muted: c.muted,
                    archived: c.archived,
                    general: c.general,
                    member: c.member,
                    owner: c.owner,
                })
                .collect(),
        ),
    }
}

fn discover(d: DiscoverData) -> TDiscover {
    TDiscover {
        group: d.group.into(),
        name: d.name.into(),
        desc: d.description.into(),
        people: int(d.people),
        access: d.access.into(),
        requested: d.requested,
    }
}

fn person(p: PersonData) -> TPerson {
    TPerson {
        mri: p.mri.into(),
        name: p.name.into(),
        detail: p.detail.into(),
        role: p.role.into(),
        me: p.me,
        picked: p.picked,
    }
}

fn people(v: Vec<PersonData>) -> ModelRc<TPerson> {
    model(v.into_iter().map(person).collect())
}

fn set_dialog(s: &TeamsStore<'_>, d: DialogData) {
    s.set_busy(d.busy);
    s.set_d_error(d.error.into());
    s.set_d_team(d.team.into());
    s.set_d_team_name(d.team_name.into());
    s.set_d_channel(d.channel.into());
    s.set_d_channel_name(d.channel_name.into());
    s.set_d_kind(d.channel_kind.into());
    s.set_d_name(d.name.into());
    s.set_d_desc(d.description.into());
    s.set_d_access(d.access.into());
    s.set_d_owner(d.owner);
    s.set_people(people(d.people));
    s.set_candidates(people(d.candidates));
    s.set_tag_people(people(d.tag_people));
    s.set_tags(model(
        d.tags
            .into_iter()
            .map(|t: TagData| TTag {
                id: t.id.into(),
                name: t.name.into(),
                desc: t.description.into(),
                count: int(t.count),
                mine: t.mine,
                selected: t.selected,
            })
            .collect(),
    ));
    s.set_deleted(model(
        d.deleted
            .into_iter()
            .map(|x: DeletedData| TDeleted {
                id: x.id.into(),
                name: x.name.into(),
                when: x.when.into(),
            })
            .collect(),
    ));
    let setting = |x: SettingData| TSetting {
        key: x.key.into(),
        group: x.group.into(),
        label: x.label.into(),
        on: x.on,
    };
    let (user, guest): (Vec<_>, Vec<_>) = d.settings.into_iter().partition(|x| x.group == "user");
    s.set_user_settings(model(user.into_iter().map(setting).collect()));
    s.set_guest_settings(model(guest.into_iter().map(setting).collect()));
    s.set_email(d.email.into());
    s.set_email_senders(d.email_senders.into());
    s.set_post_perm(d.post_perm.into());
    s.set_reply_perm(d.reply_perm.into());
    s.set_pin_perm(d.pin_perm.into());
    s.set_allow_bots(d.allow_bots);
    s.set_allow_connectors(d.allow_connectors);
    s.set_notify_level(d.notify_level.into());
    s.set_notify_replies(d.notify_replies);
    s.set_team_code(d.team_code.into());
}

fn set(w: &AppWindow, d: TeamsData) {
    let s = w.global::<TeamsStore>();
    s.set_teams(model(d.teams.into_iter().map(team).collect()));
    s.set_all_teams(model(d.all_teams.into_iter().map(team).collect()));
    s.set_stats(model(
        d.stats
            .into_iter()
            .map(|(label, value)| TStat {
                label: label.into(),
                value: value.into(),
            })
            .collect(),
    ));
    s.set_loading(d.loading);
    s.set_error(d.error.into());
    s.set_notice(d.notice.into());
    s.set_discover(model(d.discover.into_iter().map(discover).collect()));
    s.set_requests(model(d.requests.into_iter().map(discover).collect()));
    s.set_join_info(d.join_info.into());
    s.set_communities(model(
        d.communities
            .into_iter()
            .map(|c: CommunityData| TCommunity {
                id: c.id.into(),
                name: c.name.into(),
                desc: c.description.into(),
                people: int(c.people),
                company: c.company,
            })
            .collect(),
    ));
    s.set_community_name(d.community_name.into());
    s.set_posts(model(
        d.posts
            .into_iter()
            .map(|p: PostData| TPost {
                id: p.id.into(),
                sender: p.sender.into(),
                time: p.time.into(),
                text: p.text.into(),
                likes: int(p.likes),
                reply: p.reply,
                url: p.url.into(),
            })
            .collect(),
    ));
    s.set_posts_more(d.posts_more);
    set_dialog(&s, d.dialog);
}

impl UiHandle {
    /// Replaces everything the Teams UI shows (the dialog kind is set by [`Self::teams_dialog`]).
    pub fn teams_store(&self, d: TeamsData) {
        self.run(move |w| set(&w, d));
    }

    /// Opens a dialog (`""` closes it).
    pub fn teams_dialog(&self, kind: String) {
        self.run(move |w| w.global::<TeamsStore>().set_dialog(kind.into()));
    }
}
