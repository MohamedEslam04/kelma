//! UI-thread side of finding things: fills the `FindStore` global (Search page, quick views, Find
//! in chat, Shared tab and details pane).

use super::{UiHandle, img, model, rgb, styled};
use crate::find_data::{HitData, PickData, SharedData};
use crate::{AppWindow, ChannelTabApp, ChannelTabItem, FindStore, HitRow, PickRow, SharedRow};
use slint::{ComponentHandle, ModelRc};

fn hit(d: HitData) -> HitRow {
    let (avatar, has_avatar) = img(&d.avatar);
    HitRow {
        id: d.id.into(),
        kind: d.kind.into(),
        section: d.section.into(),
        title: d.title.into(),
        subtitle: d.subtitle.into(),
        snippet: styled(&d.snippet),
        snippet_plain: d.snippet.replace("**", "").into(),
        time: d.time.into(),
        initials: d.initials.into(),
        color: rgb(d.color),
        avatar,
        has_avatar,
        rtl: d.rtl,
    }
}

fn hits(v: Vec<HitData>) -> ModelRc<HitRow> {
    model(v.into_iter().map(hit).collect())
}

fn shared_rows(v: Vec<SharedData>) -> ModelRc<SharedRow> {
    model(
        v.into_iter()
            .map(|s| SharedRow {
                kind: s.kind.into(),
                name: s.name.into(),
                ext: s.ext.into(),
                shared_by: s.shared_by.into(),
                date: s.date.into(),
                url: s.url.into(),
                editable: s.editable,
                sharepoint: s.sharepoint,
                icon: s.icon.into(),
                busy: s.busy.into(),
                highlight: s.highlight,
            })
            .collect(),
    )
}

impl UiHandle {
    fn find_store(&self, f: impl FnOnce(FindStore<'_>) + Send + 'static) {
        self.run(move |w: AppWindow| f(w.global::<FindStore>()));
    }

    /// The Search page results.
    pub fn search_results(&self, rows: Vec<HitData>, searching: bool, searched: bool, more: bool) {
        self.find_store(move |s| {
            s.set_results(hits(rows));
            s.set_searching(searching);
            s.set_searched(searched);
            s.set_more(more);
        });
    }

    /// Labels of the chosen filters and which editor is open.
    pub fn search_filters(
        &self,
        in_label: String,
        person: String,
        date: &'static str,
        editor: String,
    ) {
        self.find_store(move |s| {
            s.set_filter_in(in_label.into());
            s.set_filter_person(person.into());
            s.set_filter_date(date.into());
            s.set_filter_editor(editor.into());
        });
    }

    /// Suggestions of the open filter editor.
    pub fn search_picks(&self, picks: Vec<PickData>) {
        self.find_store(move |s| {
            s.set_picks(model(
                picks
                    .into_iter()
                    .map(|p| PickRow {
                        id: p.id.into(),
                        title: p.title.into(),
                        subtitle: p.subtitle.into(),
                    })
                    .collect(),
            ));
        });
    }

    /// A quick view: its rows and whether it is still loading.
    pub fn quick_rows(&self, kind: &'static str, rows: Vec<HitData>, loading: bool, more: bool) {
        self.find_store(move |s| {
            s.set_quick_kind(kind.into());
            s.set_quick_rows(hits(rows));
            s.set_quick_loading(loading);
            s.set_quick_more(more);
        });
    }

    /// Chats with an unsent draft (the Quick views' Drafts row).
    pub fn quick_drafts(&self, n: usize) {
        self.run(move |w| {
            w.global::<crate::QuickViewsStore>()
                .set_drafts(i32::try_from(n).unwrap_or(i32::MAX));
        });
    }

    pub fn quick_open(&self, open: bool) {
        self.find_store(move |s| s.set_quick_open(open));
    }

    /// Find in chat: the bar's state and the match counter (1-based `index`, 0 for none).
    pub fn chat_find(&self, open: bool, count: usize, index: usize) {
        self.find_store(move |s| {
            s.set_chat_find_open(open);
            if !open {
                s.set_chat_find_query("".into());
            }
            s.set_chat_find_count(count as i32);
            s.set_chat_find_index(index as i32);
        });
    }

    /// The open channel's tabs beyond Posts and Files: (name, address, tab id).
    pub fn channel_tabs(&self, tabs: Vec<(String, String, String)>) {
        self.find_store(move |s| {
            s.set_channel_tabs(slint::ModelRc::new(slint::VecModel::from(
                tabs.into_iter()
                    .map(|(name, url, id)| ChannelTabItem {
                        name: name.into(),
                        url: url.into(),
                        id: id.into(),
                    })
                    .collect::<Vec<_>>(),
            )));
        });
    }

    /// The installed apps the "Add a tab" dialog offers: (app id, name).
    pub fn tab_apps(&self, apps: Vec<(String, String)>) {
        self.find_store(move |s| {
            s.set_tab_apps(slint::ModelRc::new(slint::VecModel::from(
                apps.into_iter()
                    .map(|(id, name)| ChannelTabApp {
                        id: id.into(),
                        name: name.into(),
                    })
                    .collect::<Vec<_>>(),
            )));
        });
    }

    /// Whether the Files tab shows New folder and New document (a channel with a files folder),
    /// and a short message from the last file action ("" for none; `error`: it failed).
    pub fn shared_tools(&self, can_create: bool, note: String, error: bool) {
        self.find_store(move |s| {
            s.set_shared_can_create(can_create);
            s.set_shared_note(note.into());
            s.set_shared_note_error(error);
        });
    }

    /// Switches the Files tab's filter pill (a new folder or document shows under Files).
    pub fn shared_filter(&self, filter: &'static str) {
        self.find_store(move |s| s.set_shared_filter(filter.into()));
    }

    /// The Shared tab rows (filtered), the recent five of the details pane and the loading flag.
    pub fn shared(&self, rows: Vec<SharedData>, recent: Vec<SharedData>, loading: bool) {
        self.find_store(move |s| {
            s.set_shared(shared_rows(rows));
            s.set_recent(shared_rows(recent));
            s.set_shared_loading(loading);
        });
    }

    /// Back to the Chat tab with the filters cleared (a chat was opened).
    pub fn chat_tab_reset(&self) {
        self.find_store(|s| {
            s.set_chat_tab("chat".into());
            s.set_shared_filter("recent".into());
            s.set_chat_find_open(false);
            s.set_chat_find_query("".into());
            s.set_chat_find_count(0);
            s.set_chat_find_index(0);
        });
    }

    /// Shows the Chat (`chat`) or Shared (`shared`) tab.
    pub fn chat_tab(&self, tab: &'static str) {
        self.find_store(move |s| s.set_chat_tab(tab.into()));
    }

    /// Whether the open message menu's thread is followed.
    pub fn menu_following(&self, on: bool) {
        self.find_store(move |s| s.set_menu_following(on));
    }
}
