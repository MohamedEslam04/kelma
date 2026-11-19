//! UI-thread side of the OneDrive view: turns the backend's view data into the `DriveStore`
//! global.

use super::{UiHandle, img, model, styled};
use crate::backend::drive::{
    DriveData, PermData, PersonData, PickerData, PreviewData, RowData, ShareData, VersionData,
};
use crate::{
    AppWindow, DriveCard, DriveNav, DriveRow, DriveStore, DriveVersion, SharePerm, SharePerson,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static ROWS: RefCell<Option<Rc<VecModel<DriveRow>>>> = const { RefCell::new(None) };
}

pub(crate) fn row(r: RowData) -> DriveRow {
    let (thumb, has_thumb) = img(&r.thumb);
    DriveRow {
        id: r.id.into(),
        name: r.name.into(),
        kind: r.kind.into(),
        modified: r.modified.into(),
        modified_by: r.modified_by.into(),
        size: r.size.into(),
        location: r.location.into(),
        activity: r.activity.into(),
        selected: r.selected,
        favorite: r.favorite,
        is_folder: r.is_folder,
        shortcut: r.shortcut,
        header: r.header,
        index: r.index,
        thumb,
        has_thumb,
    }
}

fn rows_model(rows: Vec<RowData>) -> ModelRc<DriveRow> {
    model(rows.into_iter().map(row).collect())
}

fn strings(v: Vec<String>) -> ModelRc<SharedString> {
    model(v.into_iter().map(SharedString::from).collect())
}

/// Updates the list rows in place when the ids are unchanged, so the scroll position and hover
/// state survive a selection change.
fn fill_rows(w: &AppWindow, rows: Vec<RowData>) {
    ROWS.with(|cell| {
        let mut cell = cell.borrow_mut();
        let m = cell.get_or_insert_with(|| {
            let m = Rc::new(VecModel::default());
            w.global::<DriveStore>().set_rows(ModelRc::from(m.clone()));
            m
        });
        let same = m.row_count() == rows.len()
            && rows
                .iter()
                .enumerate()
                .all(|(i, x)| m.row_data(i).is_some_and(|r| r.id == x.id.as_str()));
        if same {
            for (i, x) in rows.into_iter().enumerate() {
                m.set_row_data(i, row(x));
            }
        } else {
            m.set_vec(rows.into_iter().map(row).collect::<Vec<_>>());
        }
    });
}

pub(crate) fn set(w: &AppWindow, d: DriveData) {
    let s = w.global::<DriveStore>();
    s.set_section(d.section.into());
    s.set_title(d.title.into());
    s.set_item_count(d.rows.iter().filter(|r| !r.header).count() as i32);
    fill_rows(w, d.rows);
    s.set_cards(model(
        d.cards
            .into_iter()
            .map(|c| {
                let (thumb, has_thumb) = img(&c.thumb);
                DriveCard {
                    id: c.id.into(),
                    title: c.title.into(),
                    reason: c.reason.into(),
                    kind: c.kind.into(),
                    thumb,
                    has_thumb,
                }
            })
            .collect(),
    ));
    s.set_crumbs(strings(d.crumbs));
    s.set_quick(model(
        d.quick
            .into_iter()
            .map(|(key, label)| DriveNav {
                key: key.into(),
                label: label.into(),
            })
            .collect(),
    ));
    s.set_grid(d.grid);
    s.set_sort(d.sort.into());
    s.set_ascending(d.ascending);
    s.set_group(d.group.into());
    s.set_col_modified(d.columns.modified);
    s.set_col_modified_by(d.columns.modified_by);
    s.set_col_size(d.columns.size);
    s.set_col_location(d.columns.location);
    s.set_col_activity(d.columns.activity);
    s.set_type_filter(d.type_filter.into());
    if s.get_filter() != d.filter.as_str() {
        s.set_filter(d.filter.into());
    }
    s.set_status(d.status.into());
    s.set_loading(d.loading);
    s.set_more(d.more);
    s.set_sel_count(d.selection.count as i32);
    s.set_sel_name(d.selection.name.into());
    s.set_sel_has_folder(d.selection.has_folder);
    s.set_sel_all_favorite(d.selection.all_favorite);
}

fn person(p: PersonData) -> SharePerson {
    SharePerson {
        id: p.id.into(),
        name: p.name.into(),
        detail: p.detail.into(),
        picked: p.picked,
    }
}

fn perm(p: PermData) -> SharePerm {
    SharePerm {
        id: p.id.into(),
        who: p.who.into(),
        role: p.role.into(),
        detail: p.detail.into(),
        link: p.link,
        editable: p.editable,
    }
}

pub(crate) fn set_share(w: &AppWindow, d: ShareData) {
    let s = w.global::<DriveStore>();
    s.set_share_name(d.name.into());
    s.set_share_results(model(d.results.into_iter().map(person).collect()));
    s.set_share_picked(model(d.picked.into_iter().map(person).collect()));
    s.set_share_perms(model(d.perms.into_iter().map(perm).collect()));
    s.set_share_role(d.role.into());
    s.set_share_status(d.status.into());
    s.set_dialog("share".into());
}

pub(crate) fn set_picker(w: &AppWindow, d: PickerData) {
    let s = w.global::<DriveStore>();
    s.set_picker_purpose(d.purpose.into());
    s.set_picker_title(d.title.into());
    s.set_picker_folders_only(d.folders_only);
    s.set_picker_rows(rows_model(d.rows));
    s.set_picker_crumbs(strings(d.crumbs));
    s.set_picker_loading(d.loading);
    s.set_picker_can_choose(d.can_choose);
    s.set_dialog("picker".into());
}

pub(crate) fn set_versions(w: &AppWindow, name: String, v: Vec<VersionData>) {
    let s = w.global::<DriveStore>();
    s.set_versions_name(name.into());
    s.set_versions(model(
        v.into_iter()
            .map(|v| DriveVersion {
                id: v.id.into(),
                when: v.when.into(),
                by: v.by.into(),
                size: v.size.into(),
                current: v.current,
            })
            .collect(),
    ));
    s.set_dialog("versions".into());
}

pub(crate) fn set_preview(w: &AppWindow, p: PreviewData) {
    let s = w.global::<DriveStore>();
    let (image, _) = img(&p.image);
    s.set_preview_title(p.title.into());
    s.set_preview_kind(p.kind.into());
    s.set_preview_image(image);
    s.set_preview_body(styled(&p.text));
    s.set_preview_text(p.text.into());
    s.set_dialog("preview".into());
}

impl UiHandle {
    /// Replaces what the main list shows.
    pub fn drive(&self, d: DriveData) {
        self.run(move |w| set(&w, d));
    }

    pub fn drive_share(&self, d: ShareData) {
        self.run(move |w| set_share(&w, d));
    }

    pub fn drive_picker(&self, d: PickerData) {
        self.run(move |w| set_picker(&w, d));
    }

    pub fn drive_versions(&self, name: String, v: Vec<VersionData>) {
        self.run(move |w| set_versions(&w, name, v));
    }

    pub fn drive_preview(&self, p: PreviewData) {
        self.run(move |w| set_preview(&w, p));
    }

    /// Shows a text dialog (Details, Sync).
    pub fn drive_info(&self, title: String, text: String) {
        self.run(move |w| {
            let s = w.global::<DriveStore>();
            s.set_info_title(title.into());
            s.set_info_text(text.into());
            s.set_dialog("info".into());
        });
    }

    /// Closes whatever dialog is open (`""`) or opens one the backend already filled.
    pub fn drive_dialog(&self, name: &'static str) {
        self.run(move |w| w.global::<DriveStore>().set_dialog(name.into()));
    }
}
