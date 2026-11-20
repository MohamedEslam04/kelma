//! OneNote (notebooks, sections, pages, and New page) and Lists (SharePoint lists: columns and
//! items, with New item and Edit item). Editing a OneNote page's content stays in OneNote.

use super::form::{self, Pending, field};
use super::view::{ListsData, NotesData, Row, TableRow, day};
use crate::backend::Engine;
use serde_json::{Map, Value};
use std::sync::Arc;
use teams_proto::lists::{
    Column, ListItem, Site, SpList, create_item_request, field_text, field_value,
    update_item_request,
};
use teams_proto::onenote::{Notebook, Page, Section, create_page_request};

/// How many columns the Lists table shows.
const MAX_COLUMNS: usize = 8;
/// How many columns the New item and Edit item dialog has fields for (Title first).
const FORM_FIELDS: usize = 3;

#[derive(Default)]
pub struct Notes {
    loaded: bool,
    notebooks: Vec<Notebook>,
    notebook: Option<String>,
    sections: Vec<Section>,
    section: Option<String>,
    pages: Vec<Page>,
    page: Option<String>,
    text: String,
    status: String,
    loading: bool,
}

fn modified(ms: Option<i64>) -> String {
    ms.map(|m| format!("Modified {}", day(m)))
        .unwrap_or_default()
}

fn row(id: &str, title: &str, subtitle: String, selected: bool) -> Row {
    Row {
        id: id.into(),
        title: title.into(),
        subtitle,
        selected,
        ..Default::default()
    }
}

impl Notes {
    fn current_notebook(&self) -> Option<&Notebook> {
        self.notebooks
            .iter()
            .find(|n| self.notebook.as_deref() == Some(n.id.as_str()))
    }

    fn current_section(&self) -> Option<&Section> {
        self.sections
            .iter()
            .find(|s| self.section.as_deref() == Some(s.id.as_str()))
    }

    /// Where a new page of the open section is posted.
    fn pages_url(&self) -> Option<&str> {
        self.current_section().and_then(|s| s.pages_url.as_deref())
    }

    fn current_page(&self) -> Option<&Page> {
        self.pages
            .iter()
            .find(|p| self.page.as_deref() == Some(p.id.as_str()))
    }

    pub fn data(&self) -> NotesData {
        let sel = |id: &str, cur: &Option<String>| cur.as_deref() == Some(id);
        let status = if !self.status.is_empty() {
            self.status.clone()
        } else if !self.loaded || self.loading {
            String::new()
        } else if self.notebooks.is_empty() {
            "You have no notebooks.".into()
        } else if self.sections.is_empty() && self.notebook.is_some() {
            "This notebook has no sections yet.".into()
        } else if self.pages.is_empty() && self.section.is_some() {
            "This section has no pages.".into()
        } else {
            String::new()
        };
        NotesData {
            notebooks: self
                .notebooks
                .iter()
                .map(|n| {
                    row(
                        &n.id,
                        &n.name,
                        modified(n.modified_ms),
                        sel(&n.id, &self.notebook),
                    )
                })
                .collect(),
            sections: self
                .sections
                .iter()
                .map(|s| {
                    row(
                        &s.id,
                        &s.name,
                        s.group_path.clone(),
                        sel(&s.id, &self.section),
                    )
                })
                .collect(),
            pages: self
                .pages
                .iter()
                .map(|p| {
                    row(
                        &p.id,
                        &p.title,
                        modified(p.modified_ms),
                        sel(&p.id, &self.page),
                    )
                })
                .collect(),
            page_title: self
                .current_page()
                .map(|p| p.title.clone())
                .unwrap_or_default(),
            page_text: self.text.clone(),
            can_open_web: self.current_page().is_some_and(|p| p.web_url.is_some())
                || self.current_notebook().is_some_and(|n| n.web_url.is_some()),
            can_new_page: self.pages_url().is_some(),
            status,
            loading: self.loading,
        }
    }
}

#[derive(Default)]
pub struct Lists {
    loaded: bool,
    sites: Vec<Site>,
    site: Option<String>,
    lists: Vec<SpList>,
    list: Option<String>,
    columns: Vec<Column>,
    items: Vec<ListItem>,
    status: String,
    loading: bool,
}

impl Lists {
    /// The columns the dialog has fields for: Title first, then other editable ones.
    fn form_columns(&self) -> Vec<&Column> {
        let mut cols: Vec<&Column> = self.columns.iter().filter(|c| c.is_editable()).collect();
        cols.sort_by_key(|c| c.name != "Title");
        cols.truncate(FORM_FIELDS);
        cols
    }

    /// A user list (not a library) is open: items can be added and edited.
    fn can_edit(&self) -> bool {
        self.lists
            .iter()
            .any(|l| self.list.as_deref() == Some(l.id.as_str()) && l.is_user_list())
            && !self.form_columns().is_empty()
    }

    pub fn data(&self) -> ListsData {
        let sel = |id: &str, cur: &Option<String>| cur.as_deref() == Some(id);
        let columns: Vec<&Column> = self
            .columns
            .iter()
            .filter(|c| !c.hidden)
            .take(MAX_COLUMNS)
            .collect();
        let status = if !self.status.is_empty() {
            self.status.clone()
        } else if !self.loaded || self.loading {
            String::new()
        } else if self.sites.is_empty() {
            "No team sites found.".into()
        } else if self.lists.is_empty() && self.site.is_some() {
            "This site has no lists.".into()
        } else if self.items.is_empty() && self.list.is_some() {
            "This list has no items.".into()
        } else {
            String::new()
        };
        ListsData {
            sites: self
                .sites
                .iter()
                .map(|s| row(&s.id, &s.name, String::new(), sel(&s.id, &self.site)))
                .collect(),
            lists: self
                .lists
                .iter()
                .filter(|l| !l.hidden)
                .map(|l| {
                    let kind = if l.is_user_list() { "List" } else { "Library" };
                    row(&l.id, &l.name, kind.into(), sel(&l.id, &self.list))
                })
                .collect(),
            title: self
                .lists
                .iter()
                .find(|l| sel(&l.id, &self.list))
                .map(|l| l.name.clone())
                .unwrap_or_default(),
            columns: columns.iter().map(|c| c.display_name.clone()).collect(),
            items: self
                .items
                .iter()
                .map(|it| TableRow {
                    id: it.id.clone(),
                    cells: columns
                        .iter()
                        .map(|c| it.fields.get(&c.name).map(field_text).unwrap_or_default())
                        .collect(),
                })
                .collect(),
            can_edit: self.can_edit(),
            status,
            loading: self.loading,
        }
    }
}

/// How a dialog field is labelled: the column name, with the format a typed value needs.
fn column_label(c: &Column) -> String {
    match c.kind.as_str() {
        "boolean" => format!("{} (yes or no)", c.display_name),
        "dateTime" => format!("{} (2026-11-25)", c.display_name),
        "number" => format!("{} (a number)", c.display_name),
        _ => c.display_name.clone(),
    }
}

/// An item's field as the dialog shows it.
fn column_text(c: &Column, item: &ListItem) -> String {
    let text = item.fields.get(&c.name).map(field_text).unwrap_or_default();
    if c.kind == "dateTime" {
        text.chars().take(10).collect()
    } else {
        text
    }
}

impl Engine {
    // -- OneNote --

    fn notes_push(self: &Arc<Self>) {
        let data = self.apps.lock().unwrap().notes.data();
        self.ui.apps_notes(data);
    }

    fn notes_edit(self: &Arc<Self>, f: impl FnOnce(&mut Notes)) {
        f(&mut self.apps.lock().unwrap().notes);
        self.notes_push();
    }

    pub(super) async fn notes_enter(self: &Arc<Self>) -> anyhow::Result<()> {
        self.ui.open_view("onenote".into());
        let busy = {
            let n = &self.apps.lock().unwrap().notes;
            n.loaded || n.loading
        };
        if busy {
            self.notes_push();
            return Ok(());
        }
        self.notes_edit(|n| {
            n.loading = true;
            n.status.clear();
        });
        let notebooks = self.src_notebooks().await;
        let first = {
            let mut a = self.apps.lock().unwrap();
            let n = &mut a.notes;
            // A failed load is tried again the next time the view opens.
            n.loaded = notebooks.is_ok();
            n.loading = false;
            match notebooks {
                Ok(list) => n.notebooks = list,
                Err(e) => n.status = format!("Loading your notebooks failed: {e}"),
            }
            // The first notebook opens, unless one was picked while the list loaded.
            n.notebooks
                .first()
                .filter(|_| n.notebook.is_none())
                .map(|b| b.id.clone())
        };
        self.notes_push();
        if let Some(id) = first {
            self.notes_notebook(&id).await?;
        }
        Ok(())
    }

    pub(super) async fn notes_notebook(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let nb = {
            let mut a = self.apps.lock().unwrap();
            let n = &mut a.notes;
            n.notebook = Some(id.to_owned());
            n.sections.clear();
            n.section = None;
            n.pages.clear();
            n.page = None;
            n.text.clear();
            n.status.clear();
            n.loading = true;
            n.current_notebook().cloned()
        };
        self.notes_push();
        let Some(nb) = nb else { return Ok(()) };
        let sections = self.src_sections(&nb).await;
        let first = {
            let mut a = self.apps.lock().unwrap();
            let n = &mut a.notes;
            if n.notebook.as_deref() != Some(id) {
                return Ok(());
            }
            n.loading = false;
            match sections {
                Ok(s) => n.sections = s,
                Err(e) => n.status = format!("Loading sections failed: {e}"),
            }
            n.sections.first().map(|s| s.id.clone())
        };
        self.notes_push();
        if let Some(sec) = first {
            self.notes_section(&sec).await?;
        }
        Ok(())
    }

    pub(super) async fn notes_section(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let url = {
            let mut a = self.apps.lock().unwrap();
            let n = &mut a.notes;
            n.section = Some(id.to_owned());
            n.pages.clear();
            n.page = None;
            n.text.clear();
            n.status.clear();
            n.loading = true;
            n.sections
                .iter()
                .find(|s| s.id == id)
                .and_then(|s| s.pages_url.clone())
        };
        self.notes_push();
        let pages = match url {
            Some(u) => self.src_pages(&u).await,
            None => Ok(Vec::new()),
        };
        self.notes_edit(|n| {
            if n.section.as_deref() != Some(id) {
                return;
            }
            n.loading = false;
            match pages {
                Ok(p) => n.pages = p,
                Err(e) => n.status = format!("Loading pages failed: {e}"),
            }
        });
        Ok(())
    }

    pub(super) async fn notes_page(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let url = {
            let mut a = self.apps.lock().unwrap();
            let n = &mut a.notes;
            n.page = Some(id.to_owned());
            n.text.clear();
            n.status.clear();
            n.loading = true;
            n.current_page().and_then(|p| p.content_url.clone())
        };
        self.notes_push();
        let text = match url {
            Some(u) => self.src_page_text(&u).await,
            None => Ok(String::new()),
        };
        self.notes_edit(|n| {
            if n.page.as_deref() != Some(id) {
                return;
            }
            n.loading = false;
            match text {
                Ok(t) => n.text = t,
                Err(e) => n.status = format!("Loading the page failed: {e}"),
            }
        });
        Ok(())
    }

    /// Opens the page (or, without one, the notebook) in OneNote on the web.
    pub(super) fn notes_open_web(self: &Arc<Self>) -> anyhow::Result<()> {
        let url = {
            let a = self.apps.lock().unwrap();
            a.notes
                .current_page()
                .and_then(|p| p.web_url.clone())
                .or_else(|| a.notes.current_notebook().and_then(|n| n.web_url.clone()))
        };
        if let Some(url) = url {
            self.apps_open_link(&url);
        }
        Ok(())
    }

    /// Opens the New page dialog for the open section.
    pub(super) fn notes_begin_new_page(self: &Arc<Self>) -> anyhow::Result<()> {
        let name = {
            let a = self.apps.lock().unwrap();
            a.notes
                .pages_url()
                .and(a.notes.current_section())
                .map(|s| s.name.clone())
        };
        let Some(name) = name else {
            self.ui.banner("Open a section to add a page to it.".into());
            return Ok(());
        };
        let mut spec = form::confirm(format!("New page in {name}"), String::new(), "Add", false);
        spec.fields = vec![
            ("Title".into(), String::new()),
            ("Text (optional)".into(), String::new()),
        ];
        self.form_open(Pending::PageNew, spec, String::new());
        Ok(())
    }

    pub(super) async fn notes_do_new_page(
        self: &Arc<Self>,
        fields: &[String],
    ) -> Result<(), String> {
        let title = field(fields, 0);
        if title.is_empty() {
            return Err("Give the page a title.".into());
        }
        let (url, section) = {
            let a = self.apps.lock().unwrap();
            let n = &a.notes;
            (
                n.pages_url().map(str::to_owned),
                n.section.clone().unwrap_or_default(),
            )
        };
        let url = url.ok_or("Open a section to add a page to it.")?;
        let w = create_page_request(&url, &title, &field(fields, 1));
        match self.src_write("new page", w).await {
            Ok(()) => {
                self.ui.banner(format!("Page \"{title}\" added."));
                let _ = self.notes_section(&section).await;
            }
            Err(err) => {
                tracing::warn!(%err, "new page failed");
                self.ui.banner(format!("Could not add the page: {err}"));
            }
        }
        Ok(())
    }

    // -- Lists --

    fn lists_push(self: &Arc<Self>) {
        let data = self.apps.lock().unwrap().lists.data();
        self.ui.apps_lists(data);
    }

    fn lists_edit(self: &Arc<Self>, f: impl FnOnce(&mut Lists)) {
        f(&mut self.apps.lock().unwrap().lists);
        self.lists_push();
    }

    pub(super) async fn lists_enter(self: &Arc<Self>) -> anyhow::Result<()> {
        self.ui.open_view("lists".into());
        let busy = {
            let l = &self.apps.lock().unwrap().lists;
            l.loaded || l.loading
        };
        if busy {
            self.lists_push();
            return Ok(());
        }
        self.lists_edit(|l| {
            l.loading = true;
            l.status.clear();
        });
        let sites = self.src_sites().await;
        let first = {
            let mut a = self.apps.lock().unwrap();
            let l = &mut a.lists;
            l.loaded = sites.is_ok();
            l.loading = false;
            match sites {
                Ok(s) => l.sites = s,
                Err(e) => l.status = format!("Loading your sites failed: {e}"),
            }
            // The first site opens, unless one was picked while the list loaded.
            l.sites
                .first()
                .filter(|_| l.site.is_none())
                .map(|s| s.id.clone())
        };
        self.lists_push();
        if let Some(id) = first {
            self.lists_site(&id).await?;
        }
        Ok(())
    }

    pub(super) async fn lists_site(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        self.lists_edit(|l| {
            l.site = Some(id.to_owned());
            l.lists.clear();
            l.list = None;
            l.columns.clear();
            l.items.clear();
            l.status.clear();
            l.loading = true;
        });
        let lists = self.src_site_lists(id).await;
        let first = {
            let mut a = self.apps.lock().unwrap();
            let l = &mut a.lists;
            if l.site.as_deref() != Some(id) {
                return Ok(());
            }
            l.loading = false;
            match lists {
                Ok(v) => l.lists = v,
                Err(e) => l.status = format!("Loading the lists failed: {e}"),
            }
            // A user list is more useful to open than a document library.
            l.lists
                .iter()
                .find(|x| x.is_user_list())
                .or(l.lists.iter().find(|x| !x.hidden))
                .map(|x| x.id.clone())
        };
        self.lists_push();
        if let Some(list) = first {
            self.lists_list(&list).await?;
        }
        Ok(())
    }

    pub(super) async fn lists_list(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let site = {
            let mut a = self.apps.lock().unwrap();
            let l = &mut a.lists;
            l.list = Some(id.to_owned());
            l.columns.clear();
            l.items.clear();
            l.status.clear();
            l.loading = true;
            l.site.clone()
        };
        self.lists_push();
        let Some(site) = site else { return Ok(()) };
        let (columns, items) = tokio::join!(self.src_columns(&site, id), self.src_items(&site, id));
        self.lists_edit(|l| {
            if l.list.as_deref() != Some(id) {
                return;
            }
            l.loading = false;
            match (columns, items) {
                (Ok(c), Ok(i)) => {
                    l.columns = c;
                    l.items = i;
                }
                (Err(e), _) | (_, Err(e)) => l.status = format!("Loading the list failed: {e}"),
            }
        });
        Ok(())
    }
}

impl Engine {
    /// Opens the New item dialog (`item` is `None`) or the Edit item dialog of that item.
    pub(super) fn lists_begin_item(self: &Arc<Self>, item: Option<&str>) -> anyhow::Result<()> {
        let spec = {
            let a = self.apps.lock().unwrap();
            let l = &a.lists;
            if !l.can_edit() {
                None
            } else {
                let existing = item.and_then(|id| l.items.iter().find(|i| i.id == id));
                let fields: Vec<(String, String)> = l
                    .form_columns()
                    .into_iter()
                    .map(|c| {
                        let text = existing.map(|i| column_text(c, i)).unwrap_or_default();
                        (column_label(c), text)
                    })
                    .collect();
                let title = l
                    .lists
                    .iter()
                    .find(|x| l.list.as_deref() == Some(x.id.as_str()))
                    .map(|x| x.name.clone())
                    .unwrap_or_default();
                let mut spec = if item.is_some() {
                    form::confirm(
                        format!("Edit item in {title}"),
                        String::new(),
                        "Save",
                        false,
                    )
                } else {
                    form::confirm(format!("New item in {title}"), String::new(), "Add", false)
                };
                spec.fields = fields;
                Some(spec)
            }
        };
        let Some(spec) = spec else {
            self.ui
                .banner("Open a list to add or edit its items.".into());
            return Ok(());
        };
        let pending = match item {
            Some(id) => Pending::ItemEdit(id.to_owned()),
            None => Pending::ItemNew,
        };
        self.form_open(pending, spec, String::new());
        Ok(())
    }

    /// Sends the new item or the edit. Typed values that do not fit their column keep the
    /// dialog open with the reason.
    pub(super) async fn lists_do_item(
        self: &Arc<Self>,
        item: Option<&str>,
        fields: &[String],
    ) -> Result<(), String> {
        let (site, list, columns) = {
            let a = self.apps.lock().unwrap();
            let l = &a.lists;
            (
                l.site.clone().unwrap_or_default(),
                l.list.clone().unwrap_or_default(),
                l.form_columns()
                    .into_iter()
                    .map(|c| (c.name.clone(), c.kind.clone(), c.display_name.clone()))
                    .collect::<Vec<_>>(),
            )
        };
        let mut values = Map::new();
        for (i, (name, kind, label)) in columns.iter().enumerate() {
            let v = field_value(kind, &field(fields, i)).map_err(|e| format!("{label}: {e}"))?;
            // A new item leaves empty fields out; an edit clears them.
            if v != Value::Null || item.is_some() {
                values.insert(name.clone(), v);
            }
        }
        if item.is_none()
            && columns.first().is_some_and(|c| c.0 == "Title")
            && !values.contains_key("Title")
        {
            return Err("Give the item a title.".into());
        }
        if item.is_none() && values.is_empty() {
            return Err("Fill in at least one field.".into());
        }
        let w = match item {
            Some(id) => update_item_request(&site, &list, id, values),
            None => create_item_request(&site, &list, values),
        };
        let (done, failed) = if item.is_some() {
            ("Item saved.", "Could not save the item")
        } else {
            ("Item added.", "Could not add the item")
        };
        match self.src_write("list item", w).await {
            Ok(()) => {
                self.ui.banner(done.to_owned());
                let _ = self.lists_list(&list).await;
            }
            Err(err) => {
                tracing::warn!(%err, "{failed}");
                self.ui.banner(format!("{failed}: {err}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teams_proto::lists::{parse_columns, parse_items};

    #[test]
    fn the_table_shows_visible_columns_in_order() {
        let l = Lists {
            loaded: true,
            columns: parse_columns(&super::super::fixtures::list_columns()),
            items: parse_items(&super::super::fixtures::list_items()),
            list: Some("list-1".into()),
            ..Default::default()
        };
        let d = l.data();
        assert_eq!(
            d.columns,
            ["Title", "Status", "Owner", "Done"],
            "hidden column dropped"
        );
        assert_eq!(
            d.items[0].cells,
            ["Login fails on mobile", "Open", "Ada", "no"]
        );
        assert_eq!(d.items[2].cells[3], "yes");
    }

    #[test]
    fn empty_states_name_what_is_missing() {
        let n = Notes {
            loaded: true,
            ..Default::default()
        };
        assert_eq!(n.data().status, "You have no notebooks.");
        let l = Lists {
            loaded: true,
            ..Default::default()
        };
        assert_eq!(l.data().status, "No team sites found.");
    }
}
