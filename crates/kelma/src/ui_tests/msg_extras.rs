//! More of the message menu and the chat around it: Unpin, links, Immersive Reader, Share to
//! Outlook, Report, Pin on channel posts, the file card's menu, Loop cards, presence in the
//! people lists and the pop-out chat window.

use super::*;
use crate::backend::MediaCmd;
use crate::{FileItem, LoopCard, MediaStore, PeopleStore};

/// Records the `MediaStore` and `PeopleStore` commands as `"<store> <name> <argument>"` (the unit
/// separator shown as `|`).
fn store_log(app: &AppWindow) -> Log {
    let log = Log::default();
    let l = log.clone();
    app.global::<MediaStore>().on_command(move |n, a| {
        l.borrow_mut()
            .push(format!("media {n} {}", a.replace('\u{1f}', "|")))
    });
    let l = log.clone();
    app.global::<PeopleStore>()
        .on_command(move |n, a| l.borrow_mut().push(format!("people {n} {a}")));
    log
}

fn urls(list: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        list.iter()
            .map(|u| SharedString::from(*u))
            .collect::<Vec<_>>(),
    ))
}

fn has_label(app: &AppWindow, text: &str) -> bool {
    !find_label(app, text).is_empty()
}

fn tap(app: &AppWindow, text: &str) {
    let e = find_label(app, text)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("an element labelled {text:?}"));
    click(&e);
}

#[test]
fn a_pinned_message_offers_unpin_instead_of_pin() {
    let mut pinned = msg("m1", "pinned one", false);
    pinned.pinned = true;
    let app = app_with_messages(vec![pinned, msg("m2", "plain", false)]);
    let log = store_log(&app);

    open_menu(&app, &rows(&app)[0]);
    let labels = menu_labels(&app);
    assert!(labels.contains(&"Unpin".to_string()), "{labels:?}");
    assert!(!labels.contains(&"Pin for everyone".to_string()));
    click(&menu_item(&app, "Unpin"));
    assert_eq!(*log.borrow(), ["people pin-unpin m1"]);
    assert!(!app.get_menu_open());

    open_menu(&app, &rows(&app)[1]);
    let labels = menu_labels(&app);
    assert!(labels.contains(&"Pin for everyone".to_string()));
    assert!(!labels.contains(&"Unpin".to_string()));
}

#[test]
fn the_link_of_a_message_can_be_opened_and_copied_from_its_menu() {
    let mut m = msg("m1", "see the docs", false);
    m.urls = urls(&["https://example.com/a"]);
    let app = app_with_messages(vec![m, msg("m2", "no links", false)]);
    let log = store_log(&app);
    let opened = Log::default();
    let o = opened.clone();
    app.on_open_url(move |u| o.borrow_mut().push(u.to_string()));

    open_menu(&app, &rows(&app)[0]);
    click(&menu_item(&app, "Open link"));
    assert_eq!(*opened.borrow(), ["https://example.com/a"]);
    open_menu(&app, &rows(&app)[0]);
    click(&menu_item(&app, "Copy link"));
    assert_eq!(*log.borrow(), ["media link-copy https://example.com/a"]);

    open_menu(&app, &rows(&app)[1]);
    let labels = menu_labels(&app);
    assert!(!labels.contains(&"Open link".to_string()), "{labels:?}");
}

#[test]
fn several_links_are_listed_under_open_link_and_copy_link() {
    let mut m = msg("m1", "two links", false);
    m.urls = urls(&["https://example.com/a", "https://example.com/b"]);
    let app = app_with_messages(vec![m]);
    let log = store_log(&app);
    let opened = Log::default();
    let o = opened.clone();
    app.on_open_url(move |u| o.borrow_mut().push(u.to_string()));

    open_menu(&app, &rows(&app)[0]);
    click(&menu_item(&app, "Open link"));
    assert!(app.get_menu_open(), "the links are listed");
    click(&menu_item(&app, "https://example.com/b"));
    assert_eq!(*opened.borrow(), ["https://example.com/b"]);

    open_menu(&app, &rows(&app)[0]);
    click(&menu_item(&app, "Copy link"));
    click(&menu_item(&app, "https://example.com/a"));
    assert_eq!(*log.borrow(), ["media link-copy https://example.com/a"]);
}

#[test]
fn immersive_reader_shows_the_message_alone_with_size_and_line_focus() {
    let app = app_with_messages(vec![msg("m1", "a long message to read slowly", false)]);
    open_menu(&app, &rows(&app)[0]);
    click(&menu_item(&app, "Immersive Reader"));
    assert!(!app.get_menu_open());
    assert!(app.get_reader_open());
    assert_eq!(app.get_reader_msg().plain, "a long message to read slowly");
    assert!(has_label(&app, "a long message to read slowly"));

    let three = find_label(&app, "3 lines").remove(0);
    assert_eq!(three.accessible_checked(), Some(false));
    click(&three);
    assert_eq!(
        find_label(&app, "3 lines")[0].accessible_checked(),
        Some(true)
    );
    assert_eq!(
        find_label(&app, "Line focus: off")[0].accessible_checked(),
        Some(false)
    );
    tap(&app, "A+");

    tap(&app, "Close Immersive Reader");
    assert!(!app.get_reader_open());

    // Escape closes it too.
    open_menu(&app, &rows(&app)[0]);
    click(&menu_item(&app, "Immersive Reader"));
    assert!(app.get_reader_open());
    key(&app, Key::Escape, true);
    key(&app, Key::Escape, false);
    assert!(!app.get_reader_open());
}

#[test]
fn share_to_outlook_and_report_go_to_the_engine() {
    let app = app_with_messages(vec![msg("m1", "theirs", false), msg("m2", "mine", true)]);
    let log = store_log(&app);
    open_menu(&app, &rows(&app)[0]);
    click(&menu_item(&app, "Share to Outlook"));
    open_menu(&app, &rows(&app)[0]);
    click(&menu_item(&app, "Report this message"));
    assert_eq!(*log.borrow(), ["media outlook m1", "media report m1"]);

    open_menu(&app, &rows(&app)[1]);
    let labels = menu_labels(&app);
    assert!(
        !labels.contains(&"Report this message".to_string()),
        "no report of my own message: {labels:?}"
    );
}

#[test]
fn channel_posts_offer_pin_for_everyone() {
    let app = app_with_messages(vec![msg("m1", "a post", false)]);
    app.set_current_is_channel(true);
    let log = record_actions(&app);
    open_menu(&app, &rows(&app)[0]);
    click(&menu_item(&app, "Pin for everyone"));
    assert_eq!(*log.borrow(), ["pin m1"]);
}

const SP_FILE: &str = "https://contoso-my.sharepoint.com/personal/u/Documents/plan.docx";

#[test]
fn a_file_card_says_where_it_opens_and_its_menu_downloads_copies_shares_and_opens_it_online() {
    let mut m = msg("m1", "the plan", false);
    m.files = ModelRc::new(VecModel::from(vec![FileItem {
        name: "plan.docx".into(),
        url: SP_FILE.into(),
        ext: "DOCX".into(),
        place: "OneDrive".into(),
        detail: "Opens in the desktop app".into(),
    }]));
    let app = app_with_messages(vec![m]);
    let log = store_log(&app);
    let opened = Log::default();
    let o = opened.clone();
    app.on_open_url(move |u| o.borrow_mut().push(u.to_string()));

    assert!(has_label(&app, "Opens in the desktop app"));
    assert!(!has_label(&app, "Open in browser"));

    for (item, cmd) in [
        ("Download", "download"),
        ("Copy link", "copy"),
        ("Share", "share"),
        ("Open in OneDrive", "web"),
    ] {
        tap(&app, "More options for plan.docx");
        tap(&app, item);
        assert_eq!(
            log.borrow().last().unwrap(),
            &format!("media file {cmd}|{SP_FILE}")
        );
    }
    tap(&app, "More options for plan.docx");
    tap(&app, "Open");
    assert_eq!(*opened.borrow(), [format!("kelma-file:DOCX:{SP_FILE}")]);
}

#[test]
fn a_file_elsewhere_has_no_open_in_entry() {
    let mut m = msg("m1", "a zip", false);
    m.files = ModelRc::new(VecModel::from(vec![FileItem {
        name: "logs.zip".into(),
        url: "https://example.com/logs.zip".into(),
        ext: "ZIP".into(),
        place: "".into(),
        detail: "Opens in the browser".into(),
    }]));
    let app = app_with_messages(vec![m]);
    tap(&app, "More options for logs.zip");
    assert!(has_label(&app, "Download"));
    assert!(!has_label(&app, "Open in OneDrive") && !has_label(&app, "Open in SharePoint"));
}

#[test]
fn a_loop_link_is_drawn_as_a_card_that_opens_in_loop() {
    let mut m = msg("m1", "our plan", false);
    m.loops = ModelRc::new(VecModel::from(vec![LoopCard {
        url: "https://loop.cloud.microsoft/p/abc".into(),
        title: "Sprint plan".into(),
    }]));
    let app = app_with_messages(vec![m]);
    let opened = Log::default();
    let o = opened.clone();
    app.on_open_url(move |u| o.borrow_mut().push(u.to_string()));
    assert!(has_label(&app, "Open in Loop"));
    tap(&app, "Open Sprint plan in Loop");
    assert_eq!(*opened.borrow(), ["https://loop.cloud.microsoft/p/abc"]);
}

// ---- engine-backed ----------------------------------------------------------------------

/// Sends `MediaStore` commands to the engine the way `main.rs` wires them.
fn wire_media(h: &Harness) {
    let run = h.runner();
    h.app.global::<MediaStore>().on_command(move |n, a| {
        if let Some(c) = MediaCmd::parse(&n, &a) {
            run(Cmd::Media(c));
        }
    });
}

#[test]
fn the_participants_list_shows_each_persons_presence() {
    let h = Harness::new();
    let other = "8:orgid:presence-test-participant";
    h.engine.test_set_presence(other, "Busy");
    h.engine.test_set_members(BACKEND, &[demo::ME, other]);
    h.select(BACKEND);
    h.runner()(Cmd::People(
        crate::backend::PeopleCmd::parse("participants-open", "").unwrap(),
    ));
    let members = h.app.global::<PeopleStore>().get_members();
    let presence: Vec<String> = members.iter().map(|p| p.presence.to_string()).collect();
    assert_eq!(presence.len(), 2);
    assert_eq!(presence[1], "Busy");
}

#[test]
fn pop_out_opens_the_chat_in_its_own_window_and_sends_from_there() {
    let h = Harness::new();
    wire_media(&h);
    h.app
        .global::<MediaStore>()
        .invoke_command("popout".into(), BACKEND.into());
    let cw = crate::ui::popout::popout_window().expect("the pop-out window");
    assert_eq!(cw.get_conv_id(), BACKEND);
    assert_eq!(cw.get_chat_title(), "Backend Team");
    let before = cw.get_messages().row_count();
    assert!(before > 0, "the chat's messages are shown");

    // Its composer sends through the outbox (offline here, so the message waits there).
    cw.invoke_send("hello from the pop-out".into());
    let msgs = cw.get_messages();
    assert_eq!(msgs.row_count(), before + 1);
    assert_eq!(
        msgs.row_data(before).unwrap().plain,
        "hello from the pop-out"
    );

    // A channel stays in the main window.
    h.app
        .global::<MediaStore>()
        .invoke_command("popout".into(), "19:team1@thread.tacv2".into());
    assert_eq!(cw.get_conv_id(), BACKEND);
}
