//! UI-thread side of the app screens: turns the backend's view data into the `AppsStore`,
//! `TasksStore`, `FlowsStore`, `NotesStore`, `ListsStore` and `RecapsStore` globals.

use super::{UiHandle, img, model, rgb};
use crate::backend::appstore::view::{
    AppPage, Card, FlowsData, FormData, ListsData, Nav, NotesData, PermItem, RailData, RailItem,
    RecapsData, Row, StoreData, TableRow, TasksData,
};
use crate::backend::{AppsCmd, Cmd};
use crate::{
    AppCard, AppNav, AppRow, AppWindow, AppsStore, BoardColumn, FlowsStore, ListsStore, NotesStore,
    PermRow, RailApp, RecapsStore, TableRow as SlintTableRow, TasksStore,
};
use slint::{ComponentHandle, Image, ModelRc, SharedString};
use tokio::sync::mpsc::UnboundedSender;

fn card(c: Card) -> AppCard {
    let (icon, has_icon) = img(&c.icon);
    AppCard {
        id: c.id.into(),
        name: c.name.into(),
        developer: c.developer.into(),
        blurb: c.blurb.into(),
        meta: c.meta.into(),
        icon,
        has_icon,
        letter: c.letter.into(),
        color: rgb(c.color),
        installed: c.installed,
        agent: c.agent,
        update: c.update,
        pinned: c.pinned,
    }
}

fn cards(v: Vec<Card>) -> ModelRc<AppCard> {
    model(v.into_iter().map(card).collect())
}

fn nav(n: Nav) -> AppNav {
    AppNav {
        key: n.key.into(),
        label: n.label.into(),
        count: n.count.into(),
    }
}

fn navs(v: Vec<Nav>) -> ModelRc<AppNav> {
    model(v.into_iter().map(nav).collect())
}

fn rail_app(r: RailItem) -> RailApp {
    let (icon, has_icon) = img(&r.icon);
    RailApp {
        id: r.id.into(),
        name: r.name.into(),
        letter: r.letter.into(),
        color: rgb(r.color),
        icon,
        has_icon,
        view: r.view.into(),
        locked: r.locked,
    }
}

fn row(r: Row) -> AppRow {
    AppRow {
        id: r.id.into(),
        title: r.title.into(),
        subtitle: r.subtitle.into(),
        meta: r.meta.into(),
        badge: r.badge.into(),
        tone: r.tone.into(),
        done: r.done,
        selected: r.selected,
        header: r.header,
    }
}

fn rows(v: Vec<Row>) -> ModelRc<AppRow> {
    model(v.into_iter().map(row).collect())
}

fn strings(v: Vec<String>) -> ModelRc<SharedString> {
    model(v.into_iter().map(SharedString::from).collect())
}

fn set_rail(w: &AppWindow, d: RailData) {
    let s = w.global::<AppsStore>();
    s.set_rail(model(d.items.into_iter().map(rail_app).collect()));
    s.set_has_temp_app(d.temp.is_some());
    s.set_temp_app(d.temp.map(rail_app).unwrap_or_default());
    s.set_rail_collapsed(d.collapsed);
    s.set_active_app(d.active.into());
    s.set_flyout_open(d.flyout_open);
    // The search box owns the text while it is open; write it only when it differs (a re-push
    // must not move the cursor).
    if s.get_flyout_query() != d.flyout_query.as_str() {
        s.set_flyout_query(d.flyout_query.into());
    }
    s.set_flyout_apps(cards(d.flyout));
    s.set_flyout_loading(d.flyout_loading);
}

fn perm(p: PermItem) -> PermRow {
    PermRow {
        app_id: p.app_id.into(),
        app_name: p.app_name.into(),
        perm: p.perm.into(),
        label: p.label.into(),
        allowed: p.allowed,
    }
}

fn set_app(s: &AppsStore<'_>, a: AppPage) {
    let (icon, has_icon) = img(&a.icon);
    s.set_app_id(a.id.into());
    s.set_app_name(a.name.into());
    s.set_app_developer(a.developer.into());
    s.set_app_version(a.version.into());
    s.set_app_blurb(a.blurb.into());
    s.set_app_description(a.description.into());
    s.set_app_icon(icon);
    s.set_app_has_icon(has_icon);
    s.set_app_letter(a.letter.into());
    s.set_app_color(rgb(a.color));
    s.set_app_rating(a.rating.into());
    s.set_app_price(a.price.into());
    s.set_app_cert(a.cert.into());
    s.set_app_works_in(a.works_in.into());
    s.set_app_categories(a.categories.into());
    s.set_app_rsc(strings(a.rsc));
    s.set_app_device(strings(a.device));
    s.set_app_targets(navs(a.targets));
    s.set_app_installed(a.installed);
    s.set_app_update(a.update);
    s.set_app_pinned(a.pinned);
    s.set_app_can_pin(a.can_pin);
    s.set_app_opens_here(a.opens_here);
    s.set_app_privacy(a.privacy.into());
    s.set_app_terms(a.terms.into());
    s.set_app_shot_count(a.shot_count as i32);
    s.set_app_shots(model(
        a.shots.iter().map(|p| img(p).0).collect::<Vec<Image>>(),
    ));
}

fn set_store(w: &AppWindow, d: StoreData) {
    let s = w.global::<AppsStore>();
    if s.get_store_page() != d.page.as_str() {
        // A new page starts at the first screenshot.
        s.set_app_shot(0);
    }
    s.set_store_page(d.page.into());
    s.set_store_title(d.title.into());
    s.set_store_status(d.status.into());
    s.set_popular(cards(d.popular));
    s.set_org_apps(cards(d.org));
    s.set_cards(cards(d.cards));
    s.set_featured(navs(d.featured));
    s.set_categories(navs(d.categories));
    s.set_scope_filter(d.scope.into());
    s.set_kind_filter(d.kind.into());
    s.set_store_loading(d.loading);
    set_app(&s, d.app);
    s.set_manage(cards(d.manage));
    s.set_manage_note(d.manage_note.into());
    s.set_policy_text(d.policy.into());
    s.set_perms(model(d.perms.into_iter().map(perm).collect()));
    s.set_host_name(d.host_name.into());
    s.set_host_blurb(d.host_blurb.into());
    s.set_host_link(d.host_link.into());
    s.set_host_tabs(navs(d.host_tabs));
}

fn set_tasks(w: &AppWindow, d: TasksData) {
    let s = w.global::<TasksStore>();
    s.set_nav(rows(d.nav));
    s.set_tasks(rows(d.tasks));
    s.set_columns(model(
        d.columns
            .into_iter()
            .map(|c| BoardColumn {
                title: c.title.into(),
                count: c.cards.len() as i32,
                cards: rows(c.cards),
            })
            .collect(),
    ));
    s.set_title(d.title.into());
    s.set_mode(if d.board { "board" } else { "list" }.into());
    s.set_can_board(d.can_board);
    s.set_summary(d.summary.into());
    s.set_progress(d.progress);
    s.set_status(d.status.into());
    s.set_loading(d.loading);
    s.set_can_add(d.can_add);
}

fn set_flows(w: &AppWindow, d: FlowsData) {
    let s = w.global::<FlowsStore>();
    s.set_flows(rows(d.flows));
    s.set_runs(rows(d.runs));
    s.set_environment(d.environment.into());
    s.set_selected_name(d.selected_name.into());
    s.set_selected_id(d.selected_id.into());
    s.set_status(d.status.into());
    s.set_loading(d.loading);
    s.set_can_run(d.can_run);
    s.set_can_toggle(d.can_toggle);
    s.set_selected_on(d.selected_on);
}

fn set_notes(w: &AppWindow, d: NotesData) {
    let s = w.global::<NotesStore>();
    s.set_notebooks(rows(d.notebooks));
    s.set_sections(rows(d.sections));
    s.set_pages(rows(d.pages));
    s.set_page_title(d.page_title.into());
    s.set_page_text(d.page_text.into());
    s.set_can_open_web(d.can_open_web);
    s.set_can_new_page(d.can_new_page);
    s.set_status(d.status.into());
    s.set_loading(d.loading);
}

fn table_row(r: TableRow) -> SlintTableRow {
    SlintTableRow {
        id: r.id.into(),
        cells: strings(r.cells),
    }
}

fn set_lists(w: &AppWindow, d: ListsData) {
    let s = w.global::<ListsStore>();
    s.set_sites(rows(d.sites));
    s.set_lists(rows(d.lists));
    s.set_columns(strings(d.columns));
    s.set_items(model(d.items.into_iter().map(table_row).collect()));
    s.set_title(d.title.into());
    s.set_status(d.status.into());
    s.set_loading(d.loading);
    s.set_can_edit(d.can_edit);
}

fn set_form(w: &AppWindow, d: FormData) {
    let s = w.global::<AppsStore>();
    if !d.open {
        s.set_form_title("".into());
        return;
    }
    let field = |i: usize| d.fields.get(i).cloned().unwrap_or_default();
    s.set_form_body(d.body.into());
    s.set_form_confirm(d.confirm.into());
    s.set_form_danger(d.danger);
    s.set_form_link_label(d.link_label.into());
    s.set_form_error(d.error.into());
    s.set_form_choices(rows(d.choices));
    s.set_form_label1(field(0).0.into());
    s.set_form_label2(field(1).0.into());
    s.set_form_label3(field(2).0.into());
    // A refresh (a choice picked, an error shown) must keep what the user typed.
    if d.fresh {
        s.set_form_text1(field(0).1.into());
        s.set_form_text2(field(1).1.into());
        s.set_form_text3(field(2).1.into());
    }
    s.set_form_title(d.title.into());
}

fn set_recaps(w: &AppWindow, d: RecapsData) {
    let s = w.global::<RecapsStore>();
    s.set_meetings(rows(d.meetings));
    // The search box owns the text while the user types; write it only when it differs.
    if s.get_query() != d.query.as_str() {
        s.set_query(d.query.into());
    }
    s.set_note(d.note.into());
    s.set_loading(d.loading);
    let t = d.detail;
    s.set_detail_open(t.open);
    s.set_detail_title(t.title.into());
    s.set_detail_when(t.when.into());
    s.set_detail_organizer(t.organizer.into());
    s.set_has_recording(t.has_recording);
    s.set_has_transcript(t.has_transcript);
    s.set_transcript(t.transcript.into());
    s.set_transcript_note(t.transcript_note.into());
    s.set_has_notes(t.has_notes);
    s.set_notes_note(t.notes_note.into());
    s.set_ai(t.ai.into());
    s.set_has_ai_link(t.has_ai_link);
    s.set_can_share(t.can_share);
}

impl UiHandle {
    /// Switches the main area to a view (`store`, `planner`, `workflows`, `onenote`, `lists`,
    /// `app`, or one of the chat views).
    /// Shows an app's page. Settings covers the pages, so it closes (an app opened from the More
    /// flyout or a menu while Settings is open).
    pub fn open_view(&self, view: String) {
        self.run(move |w| {
            w.set_view(view.into());
            let settings = w.global::<crate::SettingsStore>();
            if settings.get_open() {
                settings.invoke_close();
            }
        });
    }

    pub fn apps_rail(&self, d: RailData) {
        self.run(move |w| set_rail(&w, d));
    }

    pub fn apps_store(&self, d: StoreData) {
        self.run(move |w| set_store(&w, d));
    }

    pub fn apps_tasks(&self, d: TasksData) {
        self.run(move |w| set_tasks(&w, d));
    }

    pub fn apps_flows(&self, d: FlowsData) {
        self.run(move |w| set_flows(&w, d));
    }

    pub fn apps_notes(&self, d: NotesData) {
        self.run(move |w| set_notes(&w, d));
    }

    pub fn apps_lists(&self, d: ListsData) {
        self.run(move |w| set_lists(&w, d));
    }

    /// Shows, refreshes or (with `open` false) closes the confirm and input dialog.
    pub fn apps_form(&self, d: FormData) {
        self.run(move |w| set_form(&w, d));
    }

    pub fn apps_recaps(&self, d: RecapsData) {
        self.run(move |w| set_recaps(&w, d));
    }
}

/// Connects the app globals' `command` callbacks to the engine's command channel.
pub fn wire(window: &AppWindow, tx: UnboundedSender<Cmd>) {
    macro_rules! wire_store {
        ($global:ident) => {{
            let tx = tx.clone();
            window
                .global::<$global>()
                .on_command(move |name, arg| match AppsCmd::parse(&name, &arg) {
                    Some(c) => {
                        let _ = tx.send(Cmd::Apps(c));
                    }
                    None => tracing::warn!(%name, "unknown app command"),
                });
        }};
    }
    wire_store!(AppsStore);
    wire_store!(TasksStore);
    wire_store!(FlowsStore);
    wire_store!(NotesStore);
    wire_store!(ListsStore);
    wire_store!(RecapsStore);
}
