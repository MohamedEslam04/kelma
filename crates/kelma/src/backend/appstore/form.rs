//! The one confirm and input dialog of the app screens. Every action that changes something on
//! the server (add, remove or update an app, a task, a flow, a page, a list item) opens it first
//! and sends its request only when the user confirms. The engine keeps what the dialog is for
//! (`Pending`); the window only shows `AppsStore.form-*` and sends `form-submit` with the text
//! fields joined by newlines.
//!
//! A problem the user can fix (an empty title, a date that is not a date) keeps the dialog open
//! with the reason. A request that fails closes it and shows the error in a banner, so the
//! message is never a success that did not happen.

use super::view::{FormData, Row};
use crate::backend::Engine;
use std::sync::Arc;

/// What the open dialog is for.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Pending {
    Install { app: String, target: String },
    Remove(String),
    Update(String),
    TaskNew,
    TaskEdit(String),
    TaskDelete(String),
    FlowRun(String),
    FlowToggle(String),
    PageNew,
    ItemNew,
    ItemEdit(String),
}

#[derive(Default)]
pub struct Form {
    pending: Option<Pending>,
    spec: FormData,
    choice: Option<String>,
    link: String,
}

impl Form {
    fn data(&self, error: &str, fresh: bool) -> FormData {
        FormData {
            fresh,
            error: error.to_owned(),
            choices: self
                .spec
                .choices
                .iter()
                .map(|c| Row {
                    selected: self.choice.as_deref() == Some(c.id.as_str()),
                    ..c.clone()
                })
                .collect(),
            ..self.spec.clone()
        }
    }
}

/// A plain dialog: a title, a sentence and a confirm button.
pub(super) fn confirm(title: String, body: String, confirm: &str, danger: bool) -> FormData {
    FormData {
        title,
        body,
        confirm: confirm.to_owned(),
        danger,
        ..Default::default()
    }
}

/// `a`, `b` and `c` of the newline-joined fields, trimmed (missing ones are empty).
pub(super) fn field(fields: &[String], i: usize) -> String {
    fields
        .get(i)
        .map(|f| f.trim().to_owned())
        .unwrap_or_default()
}

impl Engine {
    /// Opens the dialog for `pending`. `link` is what the link button opens.
    pub(super) fn form_open(self: &Arc<Self>, pending: Pending, mut spec: FormData, link: String) {
        spec.open = true;
        let data = {
            let mut a = self.apps.lock().unwrap();
            a.form = Form {
                pending: Some(pending),
                spec,
                choice: None,
                link,
            };
            a.form.data("", true)
        };
        self.ui.apps_form(data);
    }

    fn form_push(self: &Arc<Self>, error: &str) {
        let data = self.apps.lock().unwrap().form.data(error, false);
        self.ui.apps_form(data);
    }

    pub(super) fn form_close(self: &Arc<Self>) {
        self.apps.lock().unwrap().form = Form::default();
        self.ui.apps_form(FormData::default());
    }

    pub(super) fn form_choice(self: &Arc<Self>, id: &str) {
        {
            let mut a = self.apps.lock().unwrap();
            if a.form.pending.is_none() {
                return;
            }
            a.form.choice = Some(id.to_owned());
        }
        self.form_push("");
    }

    pub(super) fn form_link(self: &Arc<Self>) {
        let link = self.apps.lock().unwrap().form.link.clone();
        if !link.is_empty() {
            self.apps_open_link(&link);
        }
    }

    /// The confirm button. Runs what the dialog is for; on a problem the user can fix it stays
    /// open with the reason.
    pub(super) async fn form_submit(self: &Arc<Self>, text: &str) -> anyhow::Result<()> {
        let (pending, choice) = {
            let mut a = self.apps.lock().unwrap();
            // Taking it makes a second click on Confirm do nothing while this one is sent.
            (a.form.pending.take(), a.form.choice.clone())
        };
        let Some(p) = pending else {
            return Ok(());
        };
        let fields: Vec<String> = text.split('\n').map(str::to_owned).collect();
        let done: Result<(), String> = match p.clone() {
            Pending::Install { app, target } => {
                self.shop_do_install(&app, &target, choice.as_deref()).await
            }
            Pending::Remove(id) => self.shop_do_remove(&id).await,
            Pending::Update(id) => self.shop_do_update(&id).await,
            Pending::TaskNew => self.tasks_do_new(&fields).await,
            Pending::TaskEdit(id) => self.tasks_do_edit(&id, &fields).await,
            Pending::TaskDelete(id) => self.tasks_do_delete(&id).await,
            Pending::FlowRun(id) => self.flows_do_run(&id).await,
            Pending::FlowToggle(id) => self.flows_do_toggle(&id).await,
            Pending::PageNew => self.notes_do_new_page(&fields).await,
            Pending::ItemNew => self.lists_do_item(None, &fields).await,
            Pending::ItemEdit(id) => self.lists_do_item(Some(&id), &fields).await,
        };
        match done {
            Ok(()) => self.form_close(),
            Err(reason) => {
                self.apps.lock().unwrap().form.pending = Some(p);
                self.form_push(&reason);
            }
        }
        Ok(())
    }

    pub(super) fn form_cancel(self: &Arc<Self>) {
        self.form_close();
    }
}
