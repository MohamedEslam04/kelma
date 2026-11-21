//! The chat list beyond Favorites / Chats / Teams (plan 8.1 to 8.3, 8.5), driven through the real
//! window: sections, the chat item menu, the filter pills, the list options and the channel
//! menu. The window's `ListStore` commands are wired to the engine the way `main.rs` wires them;
//! the engine has no network, so account calls are skipped and only the local state changes.

use super::*;
use crate::ListStore;
use crate::backend::ListsCmd;
use serde_json::json;
use store::{ConvKind, ConversationRow};
use teams_proto::sections::{Folder, FolderItem, FolderKind, Folders};

const STANDUP: &str = "19:standup@thread.v2";
const GENERAL: &str = "19:team1@thread.tacv2";
const TECH: &str = "19:tech@thread.tacv2";
const MEETING: &str = "19:meeting@thread.v2";
const AHMED: &str =
    "19:00000000-0000-0000-0000-000000000001_00000000-0000-0000-0000-000000000002@unq.gbl.spaces";

/// Sends every `ListStore` command to the engine and records `"<name> <argument>"`.
fn wire(h: &Harness) -> Log {
    let log = Log::default();
    let run = h.runner();
    let l = log.clone();
    h.app.global::<ListStore>().on_command(move |name, arg| {
        l.borrow_mut()
            .push(format!("{name} {}", arg.replace('\t', " | ")));
        if let Some(c) = ListsCmd::parse(&name, &arg) {
            run(Cmd::Lists(c));
        }
    });
    log
}

fn ls(app: &AppWindow) -> ListStore<'_> {
    app.global::<ListStore>()
}

fn section_names(app: &AppWindow) -> Vec<String> {
    ls(app)
        .get_sections()
        .iter()
        .map(|s| s.name.to_string())
        .collect()
}

fn section_items(app: &AppWindow, name: &str) -> Vec<String> {
    ls(app)
        .get_sections()
        .iter()
        .find(|s| s.name == name)
        .map(|s| s.items.iter().map(|c| c.id.to_string()).collect())
        .unwrap_or_default()
}

fn ids_in(rows: slint::ModelRc<crate::ChatRow>) -> Vec<String> {
    rows.iter().map(|c| c.id.to_string()).collect()
}

fn folders(list: Vec<(&str, &str, Vec<&str>)>) -> Folders {
    Folders {
        folders: list
            .into_iter()
            .map(|(id, name, items)| Folder {
                id: id.into(),
                name: name.into(),
                kind: FolderKind::User,
                folder_type: "UserCreated".into(),
                expanded: true,
                items: items
                    .into_iter()
                    .map(|c| FolderItem {
                        conversation_id: c.into(),
                        thread_type: "chat".into(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// A harness whose demo store also has a meeting chat.
fn harness() -> Harness {
    let store = Store::open_in_memory().unwrap();
    demo::seed(&store, teams_proto::time::now_ms()).unwrap();
    store
        .upsert_conversation(&ConversationRow {
            id: MEETING.into(),
            kind: ConvKind::Meeting,
            title: "Sprint review".into(),
            team_id: None,
            team_name: None,
            last_activity_ms: teams_proto::time::now_ms() - 60_000,
            last_preview: "Recording started".into(),
            unread: false,
            favorite: false,
            read_horizon_ms: 0,
        })
        .unwrap();
    // A private channel that has not had any activity yet (it only shows in the Teams view).
    store
        .upsert_conversation(&ConversationRow {
            id: "19:private@thread.tacv2".into(),
            kind: ConvKind::Channel,
            title: "Leads".into(),
            team_id: Some("19:team1@thread.tacv2".into()),
            team_name: Some("Contoso Ltd.".into()),
            last_activity_ms: 0,
            last_preview: String::new(),
            unread: false,
            favorite: false,
            read_horizon_ms: 0,
        })
        .unwrap();
    let h = Harness::with_store(store);
    // Tall enough for the channels below the Quick views header (scrolled-out rows are not found).
    h.app.window().set_size(LogicalSize::new(1200., 900.));
    // Render the list once so the rows exist.
    h.runner()(Cmd::SetFilter("all".into()));
    h
}

fn right_click(app: &AppWindow, e: &ElementHandle) {
    let p = e.absolute_position();
    let s = e.size();
    let at = LogicalPosition::new(p.x + s.width / 2., p.y + s.height / 2.);
    app.window()
        .dispatch_event(WindowEvent::PointerMoved { position: at });
    app.window().dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button: PointerEventButton::Right,
    });
    app.window().dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button: PointerEventButton::Right,
    });
}

/// Right-clicks the chat list row titled `title`.
fn open_item_menu(app: &AppWindow, title: &str) {
    let rows = find_label(app, title);
    assert!(!rows.is_empty(), "a row titled {title:?}");
    right_click(app, &rows[0]);
}

/// Labels of the open list menu (the rows of `MenuRow`), top to bottom.
fn menu_rows(app: &AppWindow) -> Vec<String> {
    let mut v: Vec<_> = ElementHandle::find_by_element_type_name(app, "MenuRow").collect();
    v.sort_by(|a, b| a.absolute_position().y.total_cmp(&b.absolute_position().y));
    v.iter().map(label).collect()
}

fn menu_row(app: &AppWindow, name: &str) -> ElementHandle {
    ElementHandle::find_by_element_type_name(app, "MenuRow")
        .find(|r| label(r) == name)
        .unwrap_or_else(|| panic!("menu row {name:?} in {:?}", menu_rows(app)))
}

fn alt_key(app: &AppWindow, ch: &str) {
    key(app, Key::Alt, true);
    key(app, ch, true);
    key(app, ch, false);
    key(app, Key::Alt, false);
}

// ---- 8.1 sections ------------------------------------------------------------------------

#[test]
fn sections_of_the_account_show_between_favorites_and_chats_in_teams_order() {
    let h = harness();
    h.engine.test_set_folders(&folders(vec![
        ("s1", "Operations", vec![BACKEND]),
        ("s2", "Dev", vec![STANDUP, AHMED]),
    ]));
    h.runner()(Cmd::SetFilter("all".into()));
    assert_eq!(section_names(&h.app), vec!["Operations", "Dev"]);
    assert_eq!(section_items(&h.app, "Operations"), vec![BACKEND]);
    assert_eq!(section_items(&h.app, "Dev"), vec![STANDUP, AHMED]);
    // A chat in a section is not also in Chats or Favorites, even though it was starred.
    assert!(!ids_in(h.app.get_chats_main()).contains(&STANDUP.to_string()));
    assert!(!ids_in(h.app.get_chats_favorites()).contains(&BACKEND.to_string()));
    // The headers are drawn between the Favorites and Chats headers.
    let y = |name: &str| {
        find_label(&h.app, name)
            .iter()
            .map(|e| e.absolute_position().y as i32)
            .max()
    };
    // "Chats" is also a filter pill above the list: the header is the lowest one.
    let (fav, ops, dev, chats) = (
        y("Favorites"),
        y("Operations, expanded"),
        y("Dev, expanded"),
        y("Chats"),
    );
    assert!(
        fav < ops && ops < dev && dev < chats,
        "{fav:?} {ops:?} {dev:?} {chats:?}"
    );
}

/// At start-up the list draws the sections and favorites Teams had last time, before the
/// `conversationFolders` read answers: no empty Favorites, no missing sections.
#[test]
fn the_last_known_sections_and_favorites_show_at_start_before_teams_answers() {
    let h = harness();
    let mut f = folders(vec![("s1", "Operations", vec![BACKEND])]);
    f.folders.push(Folder {
        id: "fav".into(),
        name: "Favorites".into(),
        kind: FolderKind::Favorites,
        folder_type: "Favorites".into(),
        expanded: true,
        items: vec![FolderItem {
            conversation_id: STANDUP.into(),
            thread_type: "chat".into(),
        }],
    });
    h.engine.test_set_folders(&f);
    let saved = h
        .engine
        .test_store(|st| st.get_kv("lists-server").unwrap())
        .expect("the sections are saved");

    // The next start: the same store, nothing read from Teams yet.
    let store = Store::open_in_memory().unwrap();
    demo::seed(&store, teams_proto::time::now_ms()).unwrap();
    store.set_kv("lists-server", &saved).unwrap();
    let next = Harness::with_store(store);
    next.runner()(Cmd::SetFilter("all".into()));
    assert_eq!(section_names(&next.app), vec!["Operations"]);
    assert_eq!(section_items(&next.app, "Operations"), vec![BACKEND]);
    assert!(ids_in(next.app.get_chats_favorites()).contains(&STANDUP.to_string()));

    // What Teams answers then replaces them.
    next.engine
        .test_set_folders(&folders(vec![("s2", "Dev", vec![AHMED])]));
    next.runner()(Cmd::SetFilter("all".into()));
    assert_eq!(section_names(&next.app), vec!["Dev"]);
    assert!(!ids_in(next.app.get_chats_favorites()).contains(&STANDUP.to_string()));
}

#[test]
fn a_new_section_from_the_move_menu_is_created_and_takes_the_chat() {
    let h = harness();
    let log = wire(&h);
    open_item_menu(&h.app, "Morning Standup");
    click(&menu_row(&h.app, "Move to section"));
    // The submenu offers Favorites, then "New section".
    assert_eq!(
        menu_rows(&h.app).last().map(String::as_str),
        Some("New section")
    );
    click(&menu_row(&h.app, "New section"));
    assert_eq!(ls(&h.app).get_dialog(), "new-section");
    assert_eq!(ls(&h.app).get_dialog_target(), STANDUP);
    // The name dialog has the focus: type a name and press Enter.
    type_text(&h.app, "Ops");
    enter(&h.app);
    assert_eq!(ls(&h.app).get_dialog(), "", "the dialog closes");
    assert_eq!(*log.borrow(), vec![format!("new-section {STANDUP} | Ops")]);
    assert_eq!(section_names(&h.app), vec!["Ops"]);
    assert_eq!(section_items(&h.app, "Ops"), vec![STANDUP]);
    let (local, _) = h.engine.test_lists();
    assert_eq!(local.sections.len(), 1);
    assert_eq!(local.assign.get(STANDUP), Some(&local.sections[0].0));
}

#[test]
fn move_to_a_section_and_remove_from_section_round_trip() {
    let h = harness();
    wire(&h);
    let run = h.runner();
    run(Cmd::Lists(ListsCmd::NewSection {
        move_chat: None,
        name: "Ops".into(),
    }));
    let id = h.engine.test_lists().0.sections[0].0.clone();
    run(Cmd::Lists(ListsCmd::Move(AHMED.into(), id.clone())));
    assert_eq!(section_items(&h.app, "Ops"), vec![AHMED]);
    // The item menu of a chat in a section offers "Remove from section".
    open_item_menu(&h.app, "Ahmed Ali");
    assert!(menu_rows(&h.app).contains(&"Remove from section".to_string()));
    click(&menu_row(&h.app, "Remove from section"));
    assert!(section_items(&h.app, "Ops").is_empty());
    assert!(ids_in(h.app.get_chats_main()).contains(&AHMED.to_string()));
    // Moving to Favorites uses the Favorites section.
    run(Cmd::Lists(ListsCmd::Move(AHMED.into(), "favorites".into())));
    assert!(ids_in(h.app.get_chats_favorites()).contains(&AHMED.to_string()));
}

#[test]
fn sections_can_be_renamed_and_deleted_and_names_are_checked() {
    let h = harness();
    let run = h.runner();
    let new = |name: &str| {
        Cmd::Lists(ListsCmd::NewSection {
            move_chat: Some(STANDUP.into()),
            name: name.into(),
        })
    };
    run(new("Ops"));
    run(new("ops")); // same name, other case: refused
    run(new("   ")); // empty: refused
    assert_eq!(section_names(&h.app), vec!["Ops"]);
    let id = h.engine.test_lists().0.sections[0].0.clone();
    run(Cmd::Lists(ListsCmd::RenameSection(
        id.clone(),
        "Platform".into(),
    )));
    assert_eq!(section_names(&h.app), vec!["Platform"]);
    run(Cmd::Lists(ListsCmd::DeleteSection(id)));
    assert!(section_names(&h.app).is_empty());
    assert!(
        ids_in(h.app.get_chats_main()).contains(&STANDUP.to_string()),
        "the chat goes back to Chats"
    );
}

#[test]
fn clicking_a_section_header_collapses_it_and_alt_q_collapses_them_all() {
    let h = harness();
    let log = wire(&h);
    h.engine
        .test_set_folders(&folders(vec![("s1", "Operations", vec![BACKEND])]));
    h.runner()(Cmd::SetFilter("all".into()));
    assert!(ls(&h.app).get_sections().row_data(0).unwrap().expanded);
    click(&find_label(&h.app, "Operations, expanded")[0]);
    assert_eq!(*log.borrow(), vec!["section-toggle s1"]);
    assert!(!ls(&h.app).get_sections().row_data(0).unwrap().expanded);
    // Open it again; "Collapse all sections" from the list menu collapses them all.
    click(&find_label(&h.app, "Operations, collapsed")[0]);
    assert!(ls(&h.app).get_sections().row_data(0).unwrap().expanded);
    let more = ElementHandle::find_by_element_id(&h.app, "IconButton::touch")
        .filter(|b| b.absolute_position().x < 400. && b.absolute_position().y < 120.)
        .max_by(|a, b| a.absolute_position().x.total_cmp(&b.absolute_position().x))
        .expect("list menu button");
    click(&more);
    click(&find_label(&h.app, "Collapse all sections")[0]);
    assert!(!ls(&h.app).get_sections().row_data(0).unwrap().expanded);
    assert!(!ls(&h.app).get_favorites_open());
    assert!(!ls(&h.app).get_chats_open());
    assert!(!ls(&h.app).get_channels_open());
}

#[test]
fn alt_q_asks_to_collapse_every_section() {
    let app = app_with_messages(vec![]);
    let log = Log::default();
    {
        let l = log.clone();
        ls(&app).on_command(move |name, arg| l.borrow_mut().push(format!("{name} {arg}")));
    }
    composer(&app).mock_single_click(PointerEventButton::Left);
    alt_key(&app, "q");
    assert_eq!(*log.borrow(), vec!["collapse-all "]);
}

// ---- 8.2 chat item menu -----------------------------------------------------------------

#[test]
fn the_item_menu_of_a_chat_has_the_teams_entries() {
    let h = harness();
    wire(&h);
    open_item_menu(&h.app, "Ahmed Ali");
    assert_eq!(
        menu_rows(&h.app),
        vec![
            "Mark as unread",
            "Favorite",
            "Pop out chat",
            "Move to section",
            "Snooze",
            "Mute",
            "Manage apps",
            "Set workflows",
            "Copy link",
            "Remove chat history",
            "Hide"
        ]
    );
}

#[test]
fn the_item_menu_manages_apps_honestly_opens_workflows_and_asks_before_removing_history() {
    let h = harness();
    let log = wire(&h);
    let flows = Log::default();
    let f = flows.clone();
    h.app
        .global::<crate::FlowsStore>()
        .on_command(move |n, a| f.borrow_mut().push(format!("{n} {a}")));
    open_item_menu(&h.app, "Ahmed Ali");
    click(&menu_row(&h.app, "Manage apps"));
    assert_eq!(
        log.borrow().last().unwrap(),
        &format!("manage-apps {AHMED}")
    );
    assert_eq!(h.app.get_banner(), crate::backend::MANAGE_APPS_NOTE);
    open_item_menu(&h.app, "Ahmed Ali");
    click(&menu_row(&h.app, "Set workflows"));
    assert_eq!(*flows.borrow(), ["flows-enter "]);
    open_item_menu(&h.app, "Ahmed Ali");
    click(&menu_row(&h.app, "Remove chat history"));
    assert_eq!(
        h.app
            .global::<crate::ChatHeaderStore>()
            .get_remove_history(),
        AHMED,
        "the window's confirmation opens"
    );
}

#[test]
fn a_group_chat_can_also_be_left_and_a_channel_has_the_channel_entries() {
    let h = harness();
    wire(&h);
    open_item_menu(&h.app, "Morning Standup");
    assert!(menu_rows(&h.app).contains(&"Leave".to_string()));
    // Another right click elsewhere replaces the menu; a channel row has the channel items.
    open_item_menu(&h.app, "tech only");
    let rows = menu_rows(&h.app);
    for want in [
        "Mark as unread",
        "Move to section",
        "Manage notifications",
        "Copy link",
        "Go to team",
        "Hide channel",
    ] {
        assert!(rows.contains(&want.to_string()), "{want:?} in {rows:?}");
    }
    assert!(
        !rows.contains(&"Mute".to_string()),
        "channels use Manage notifications"
    );
}

#[test]
fn mute_marks_the_row_silences_notifications_and_offers_unmute() {
    let h = harness();
    let log = wire(&h);
    open_item_menu(&h.app, "Ahmed Ali");
    click(&menu_row(&h.app, "Mute"));
    assert_eq!(*log.borrow(), vec![format!("mute {AHMED}")]);
    assert!(h.engine.test_is_muted(AHMED));
    assert!(
        h.engine.test_silenced(AHMED),
        "no desktop notification for it"
    );
    let row = h.chats().into_iter().find(|c| c.id == AHMED).unwrap();
    assert!(row.muted);
    open_item_menu(&h.app, "Ahmed Ali");
    click(&menu_row(&h.app, "Unmute"));
    assert!(!h.engine.test_is_muted(AHMED));
    assert!(!h.chats().into_iter().find(|c| c.id == AHMED).unwrap().muted);
}

#[test]
fn hide_removes_the_chat_until_it_gets_a_newer_message() {
    let h = harness();
    wire(&h);
    open_item_menu(&h.app, "Ahmed Ali");
    click(&menu_row(&h.app, "Hide"));
    assert!(!h.chats().iter().any(|c| c.id == AHMED));
    // Another chat is untouched, and a newer message brings it back.
    assert!(h.chats().iter().any(|c| c.id == STANDUP));
    let (local, _) = h.engine.test_lists();
    assert!(local.hidden.contains_key(AHMED));
    h.engine
        .test_touch_conversation(AHMED, teams_proto::time::now_ms() + 5_000);
    h.runner()(Cmd::SetFilter("all".into()));
    assert!(
        h.chats().iter().any(|c| c.id == AHMED),
        "back with its next message"
    );
}

#[test]
fn mark_as_unread_and_copy_link_from_the_menu() {
    let h = harness();
    let log = wire(&h);
    let unread = Log::default();
    {
        let l = unread.clone();
        h.app
            .on_mark_chat_unread(move |id| l.borrow_mut().push(format!("unread {id}")));
    }
    open_item_menu(&h.app, "Ahmed Ali");
    click(&menu_row(&h.app, "Mark as unread"));
    assert_eq!(*unread.borrow(), vec![format!("unread {AHMED}")]);
    // Copy link asks the engine (the clipboard is skipped without a display in demo mode).
    open_item_menu(&h.app, "Ahmed Ali");
    click(&menu_row(&h.app, "Copy link"));
    assert_eq!(*log.borrow(), vec![format!("copy-link {AHMED}")]);
}

// ---- 8.3 filters and list options -------------------------------------------------------

#[test]
fn each_filter_pill_narrows_the_list_and_a_second_click_clears_it() {
    let h = harness();
    for (pill, filter) in [
        ("Unread", "unread"),
        ("Channels", "channels"),
        ("Chats", "chats"),
        ("Meeting chats", "meetings"),
    ] {
        let seen = Log::default();
        let s = seen.clone();
        let run = h.runner();
        h.app.on_set_filter(move |f| {
            s.borrow_mut().push(f.to_string());
            run(Cmd::SetFilter(f.into()));
        });
        click(&find_label(&h.app, pill)[0]);
        assert_eq!(*seen.borrow(), vec![filter], "{pill}");
        let rows = h.chats();
        match filter {
            "unread" => assert!(
                !rows.is_empty() && rows.iter().all(|r| r.unread),
                "unread only"
            ),
            "channels" => assert!(!rows.is_empty() && rows.iter().all(|r| r.channel)),
            "chats" => assert!(!rows.is_empty() && rows.iter().all(|r| !r.channel)),
            _ => assert_eq!(
                rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
                vec![MEETING],
                "meeting chats only"
            ),
        }
        // Clicking the active pill clears the filter.
        click(&find_label(&h.app, pill)[0]);
        assert_eq!(seen.borrow().last().map(String::as_str), Some("all"));
        assert!(h.chats().len() > 4, "everything again");
    }
}

#[test]
fn the_drafts_pill_still_lists_only_chats_with_a_draft() {
    let h = harness();
    h.select(NOTES);
    h.type_in_composer("half a thought");
    h.select(BACKEND);
    h.app.on_set_filter({
        let run = h.runner();
        move |f| run(Cmd::SetFilter(f.into()))
    });
    click(&find_label(&h.app, "Drafts")[0]);
    let ids: Vec<String> = h.chats().into_iter().map(|c| c.id.to_string()).collect();
    assert_eq!(ids, vec![NOTES]);
}

#[test]
fn list_options_hide_previews_and_time_stamps_and_combine_chats_and_channels() {
    let h = harness();
    let log = wire(&h);
    let preview = "Youssef: Fixed in 2.4.1";
    assert_eq!(find_label(&h.app, preview).len(), 1, "previews are on");
    // The "…" menu of the list carries the options behind "Customize view", and stays open
    // while they are toggled.
    let more = ElementHandle::find_by_element_id(&h.app, "IconButton::touch")
        .filter(|b| b.absolute_position().x < 400. && b.absolute_position().y < 120.)
        .max_by(|a, b| a.absolute_position().x.total_cmp(&b.absolute_position().x))
        .expect("list menu button");
    click(&more);
    click(&find_label(&h.app, "Customize view")[0]);
    click(&find_label(&h.app, "Show message previews")[0]);
    assert!(!ls(&h.app).get_show_previews());
    assert!(find_label(&h.app, preview).is_empty(), "previews hidden");
    click(&find_label(&h.app, "Show time stamps")[0]);
    assert!(!ls(&h.app).get_show_times());
    // Chats and channels together: the channel rows move into the Chats section.
    assert!(
        !h.app.get_chats_channels().iter().next().is_none(),
        "separate at first"
    );
    click(&find_label(&h.app, "Chats and channels together")[0]);
    assert!(ls(&h.app).get_combined());
    assert_eq!(h.app.get_chats_channels().row_count(), 0);
    assert!(h.app.get_chats_main().iter().any(|c| c.channel));
    assert_eq!(
        *log.borrow(),
        vec![
            "option previews | off",
            "option times | off",
            "option combined | on"
        ]
    );
    // And they come back.
    h.runner()(Cmd::Lists(ListsCmd::Option("combined".into(), false)));
    h.runner()(Cmd::Lists(ListsCmd::Option("previews".into(), true)));
    assert!(h.app.get_chats_channels().row_count() > 0);
    assert_eq!(find_label(&h.app, preview).len(), 1);
}

// ---- 8.5 channel menu --------------------------------------------------------------------

fn teams_json() -> teams_proto::teams::TeamEntry {
    let v = json!({"teams": [{
        "id": "19:team1@thread.tacv2", "displayName": "Contoso Ltd.",
        "teamSiteInformation": {"groupId": "g-1"}, "isFavorite": true,
        "channels": [
            {"id": GENERAL, "displayName": "General", "isGeneral": true, "isFavorite": true},
            {"id": TECH, "displayName": "tech only", "isFavorite": true},
            {"id": "19:private@thread.tacv2", "displayName": "Leads", "channelType": 1, "isFavorite": true}
        ]
    }]});
    teams_proto::teams::parse_teams(&v).remove(0)
}

fn teams_view(h: &Harness) {
    h.app.set_view("teams".into());
    // Expand the team (the demo team starts collapsed state-free: expanded).
}

#[test]
fn the_channel_menu_hides_a_channel_and_shows_it_again_in_the_teams_view() {
    let h = harness();
    let log = wire(&h);
    h.engine.test_set_teams(&[teams_json()]);
    teams_view(&h);
    h.runner()(Cmd::SetFilter("all".into()));
    let row = find_label(&h.app, "tech only");
    assert!(!row.is_empty(), "the channel is in the Teams view");
    right_click(&h.app, &row[0]);
    assert_eq!(
        menu_rows(&h.app),
        vec![
            "Manage notifications",
            "Hide channel",
            "Pin",
            "Copy link",
            "Go to team"
        ]
    );
    click(&menu_row(&h.app, "Hide channel"));
    assert_eq!(h.engine.test_channel_shown(TECH), Some(false));
    assert_eq!(
        log.borrow().last().map(String::as_str),
        Some(format!("channel-hide {TECH}").as_str())
    );
    // Hidden channels leave the chat list and show dimmed in the Teams view, where the menu
    // offers Show channel.
    assert!(!h.chats().iter().any(|c| c.id == TECH));
    assert_eq!(find_label(&h.app, "tech only (hidden)").len(), 1);
    right_click(&h.app, &find_label(&h.app, "tech only (hidden)")[0]);
    click(&menu_row(&h.app, "Show channel"));
    assert_eq!(h.engine.test_channel_shown(TECH), Some(true));
    assert!(h.chats().iter().any(|c| c.id == TECH));
}

#[test]
fn manage_notifications_offers_three_levels_and_off_silences_the_channel() {
    let h = harness();
    let log = wire(&h);
    h.engine.test_set_teams(&[teams_json()]);
    teams_view(&h);
    h.runner()(Cmd::SetFilter("all".into()));
    right_click(&h.app, &find_label(&h.app, "tech only")[0]);
    click(&menu_row(&h.app, "Manage notifications"));
    assert_eq!(
        menu_rows(&h.app),
        vec![
            "‹ Manage notifications",
            "All activity",
            "Only show in the feed",
            "Off"
        ]
    );
    assert!(!h.engine.test_silenced(TECH));
    click(&menu_row(&h.app, "Off"));
    assert_eq!(
        log.borrow().last().map(String::as_str),
        Some(format!("channel-notify {TECH} | off").as_str())
    );
    assert!(
        h.engine.test_silenced(TECH),
        "no notifications for a channel set to Off"
    );
    // The choice shows as checked when the menu opens again.
    right_click(&h.app, &find_label(&h.app, "tech only")[0]);
    click(&menu_row(&h.app, "Manage notifications"));
    assert_eq!(ls(&h.app).get_channel_level(), "off");
}

#[test]
fn go_to_team_opens_the_teams_view_and_a_private_channel_can_be_left_after_confirming() {
    let h = harness();
    let log = wire(&h);
    h.engine.test_set_teams(&[teams_json()]);
    // From the chat list: Go to team switches the rail to Teams.
    assert_eq!(h.app.get_view(), "chat");
    open_item_menu(&h.app, "tech only");
    click(&menu_row(&h.app, "Go to team"));
    assert_eq!(h.app.get_view(), "teams");
    // A private channel's menu has Leave channel, which asks first.
    right_click(&h.app, &find_label(&h.app, "Leads")[0]);
    assert!(ls(&h.app).get_channel_private());
    click(&menu_row(&h.app, "Leave channel"));
    assert_eq!(ls(&h.app).get_dialog(), "leave-channel");
    assert!(
        !log.borrow().iter().any(|l| l.starts_with("channel-leave")),
        "nothing is sent before the answer"
    );
    click(&find_label(&h.app, "Leave")[0]);
    assert!(
        log.borrow()
            .iter()
            .any(|l| l == "channel-leave 19:private@thread.tacv2")
    );
    assert_eq!(ls(&h.app).get_dialog(), "");
}

// ---- "…" buttons, Pin and the team menu in the sidebar -----------------------------------

/// Records the `TeamsStore` and `MediaStore` commands as `"<name> <argument>"` (the unit
/// separator shown as `|`).
fn wire_other_stores(h: &Harness) -> Log {
    let log = Log::default();
    let l = log.clone();
    h.app.global::<crate::TeamsStore>().on_command(move |n, a| {
        l.borrow_mut()
            .push(format!("{n} {}", a.replace('\u{1f}', "|")))
    });
    let l = log.clone();
    h.app
        .global::<crate::MediaStore>()
        .on_command(move |n, a| l.borrow_mut().push(format!("{n} {a}")));
    log
}

#[test]
fn hovering_a_chat_shows_a_more_button_that_opens_its_menu() {
    let h = harness();
    wire(&h);
    let other = wire_other_stores(&h);
    let row = find_label(&h.app, "Ahmed Ali").remove(0);
    hover(&h.app, &row);
    let more = find_label(&h.app, "More options");
    assert_eq!(more.len(), 1, "one \"…\" on the hovered row");
    click(&more[0]);
    let rows = menu_rows(&h.app);
    assert!(rows.contains(&"Favorite".to_string()), "{rows:?}");
    click(&menu_row(&h.app, "Pop out chat"));
    assert_eq!(other.borrow().len(), 1);
    assert!(
        other.borrow()[0].starts_with("popout 19:"),
        "{:?}",
        other.borrow()
    );
}

#[test]
fn favorite_in_the_item_menu_toggles_the_favorite() {
    let h = harness();
    wire(&h);
    let toggled = Log::default();
    let t = toggled.clone();
    h.app
        .on_toggle_favorite(move |id| t.borrow_mut().push(id.to_string()));
    open_item_menu(&h.app, "Ahmed Ali");
    click(&menu_row(&h.app, "Favorite"));
    assert_eq!(toggled.borrow().len(), 1);
}

#[test]
fn a_channel_in_the_teams_view_has_a_more_button_and_can_be_pinned() {
    let h = harness();
    wire(&h);
    let other = wire_other_stores(&h);
    h.engine.test_set_teams(&[teams_json()]);
    teams_view(&h);
    h.runner()(Cmd::SetFilter("all".into()));
    let row = find_label(&h.app, "tech only").remove(0);
    hover(&h.app, &row);
    click(&find_label(&h.app, "More options for tech only")[0]);
    click(&menu_row(&h.app, "Pin"));
    assert_eq!(
        other.borrow().last().unwrap(),
        &format!("chan-pin {TECH}|1")
    );
}

#[test]
fn a_team_in_the_teams_view_has_the_team_menu_on_its_more_button_and_right_click() {
    let h = harness();
    wire(&h);
    let other = wire_other_stores(&h);
    h.engine.test_set_teams(&[teams_json()]);
    h.engine.manage_set_entries(vec![teams_json()]);
    teams_view(&h);
    h.runner()(Cmd::SetFilter("all".into()));
    let team = find_label(&h.app, "Contoso Ltd.").remove(0);
    right_click(&h.app, &team);
    for want in [
        "Go to team",
        "New channel",
        "Manage team",
        "Get link to team",
        "Leave the team",
    ] {
        assert!(
            !find_label(&h.app, want).is_empty(),
            "{want:?} in the team menu"
        );
    }
    click(&find_label(&h.app, "Get link to team")[0]);
    assert_eq!(
        other.borrow().last().unwrap(),
        "team-link 19:team1@thread.tacv2"
    );

    hover(&h.app, &find_label(&h.app, "Contoso Ltd.").remove(0));
    click(&find_label(&h.app, "More options for Contoso Ltd.")[0]);
    click(&find_label(&h.app, "Hide team")[0]);
    assert_eq!(
        other.borrow().last().unwrap(),
        "team-show 19:team1@thread.tacv2|0"
    );
}
