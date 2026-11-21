//! UI-thread side of the Activity view: the list, the preview pane and the badge counts.

use super::{UiHandle, img, msg, rgb};
use crate::ActivityRow;
use crate::Msg;
use crate::data::{ActivityData, PreviewData};
use slint::{Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static ITEMS: RefCell<Option<Rc<VecModel<ActivityRow>>>> = const { RefCell::new(None) };
    static PREVIEW: RefCell<Option<Rc<VecModel<Msg>>>> = const { RefCell::new(None) };
}

pub fn row(d: ActivityData) -> ActivityRow {
    let (avatar, has_avatar) = img(&d.avatar);
    ActivityRow {
        id: d.id.into(),
        title: d.title.into(),
        initials: d.initials.into(),
        color: rgb(d.color),
        avatar,
        has_avatar,
        badge: d.badge.into(),
        unread: d.unread,
        preview: d.preview.into(),
        time: d.time.into(),
        location: d.location.into(),
        rtl_preview: d.rtl_preview,
        call_mri: d.call_mri.into(),
        person: d.person.into(),
    }
}

/// Fills a persistent model, updating rows in place when the ids are unchanged so the scroll
/// position is kept.
fn fill<T: Clone + 'static>(
    cell: &RefCell<Option<Rc<VecModel<T>>>>,
    set: impl FnOnce(ModelRc<T>),
    rows: Vec<T>,
    id_of: impl Fn(&T) -> SharedString,
) {
    let mut cell = cell.borrow_mut();
    let m = cell.get_or_insert_with(|| {
        let m = Rc::new(VecModel::default());
        set(ModelRc::from(m.clone()));
        m
    });
    let same_ids = m.row_count() == rows.len()
        && rows
            .iter()
            .enumerate()
            .all(|(i, x)| m.row_data(i).is_some_and(|r| id_of(&r) == id_of(x)));
    if same_ids {
        for (i, x) in rows.into_iter().enumerate() {
            m.set_row_data(i, x);
        }
    } else {
        m.set_vec(rows);
    }
}

impl UiHandle {
    /// Replaces the Activity list.
    pub fn activity(&self, items: Vec<ActivityData>) {
        self.run(move |w| {
            ITEMS.with(|c| {
                fill(
                    c,
                    |m| w.set_activity_items(m),
                    items.into_iter().map(row).collect(),
                    |r| r.id.clone(),
                );
            });
        });
    }

    /// Unread counts: Activity items and chats. The window title and the rail show them.
    pub fn badges(&self, activity: i32, chats: i32) {
        self.run(move |w| {
            w.set_activity_unread(activity);
            w.set_chats_unread(chats);
            crate::tray_wiring::unread_changed(crate::tray_wiring::unread_total(activity, chats));
        });
    }

    /// Replaces the preview pane content.
    pub fn activity_preview(&self, p: PreviewData) {
        self.run(move |w| {
            w.set_activity_preview_id(p.id.into());
            w.set_activity_preview_title(p.title.into());
            w.set_activity_preview_location(p.location.into());
            w.set_activity_preview_is_channel(p.is_channel);
            w.set_activity_preview_loading(p.loading);
            w.set_activity_highlight(p.highlight.into());
            PREVIEW.with(|c| {
                fill(
                    c,
                    |m| w.set_activity_preview(m),
                    p.msgs.into_iter().map(msg).collect(),
                    |r| r.id.clone(),
                );
            });
        });
    }
}
