//! Snippets driven through the real window: typing `/br` in the composer offers the saved quick
//! replies and Enter puts the text in the draft; Settings saves and deletes them in a temporary
//! config directory (never the real one).

use super::*;
use crate::{ChatRow, SnippetStore};
use std::path::PathBuf;

/// A temporary config directory, removed when dropped.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let d = std::env::temp_dir().join(format!(
            "kelma-ui-snippets-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn picker_rows(app: &AppWindow) -> usize {
    ElementHandle::find_by_element_type_name(app, "SnippetPicker").count()
}

fn ready(tag: &str) -> (AppWindow, Dir, Log) {
    let dir = Dir::new(tag);
    let app = app_with_messages(vec![]);
    crate::snippets::install(&app, dir.0.clone(), Some("Omar Ali".into()));
    let sent = record_send(&app);
    composer(&app).mock_single_click(PointerEventButton::Left);
    (app, dir, sent)
}

#[test]
fn slash_br_then_enter_puts_the_snippet_in_the_composer() {
    let (app, _dir, sent) = ready("enter");
    type_text(&app, "/br");
    assert_eq!(picker_rows(&app), 1, "the picker is open");
    enter(&app);
    assert_eq!(app.get_draft(), "Be right back");
    assert!(sent.borrow().is_empty(), "Enter picked, it did not send");
    assert_eq!(picker_rows(&app), 0);
    // The next Enter sends the inserted text.
    enter(&app);
    assert_eq!(*sent.borrow(), vec!["Be right back".to_string()]);
}

#[test]
fn placeholders_use_the_other_persons_first_name_in_a_one_to_one_chat() {
    let (app, _dir, _sent) = ready("first");
    app.set_chats_main(ModelRc::new(VecModel::from(vec![ChatRow {
        id: "48:notes".into(),
        title: "Sara Hassan".into(),
        person: "Sara Hassan".into(),
        ..Default::default()
    }])));
    type_text(&app, "/thanks");
    enter(&app);
    assert_eq!(app.get_draft(), "Thanks, Sara!");
}

#[test]
fn arrows_and_tab_choose_and_escape_closes() {
    let (app, _dir, _sent) = ready("keys");
    // "/" lists all three examples in order: brb, omw, thanks.
    type_text(&app, "/");
    key(&app, Key::DownArrow, true);
    key(&app, Key::DownArrow, false);
    key(&app, Key::Tab, true);
    key(&app, Key::Tab, false);
    assert_eq!(app.get_draft(), "On my way");
}

#[test]
fn escape_closes_the_picker_and_enter_then_sends_the_text_as_typed() {
    let (app, _dir, sent) = ready("escape");
    type_text(&app, "/br");
    key(&app, Key::Escape, true);
    key(&app, Key::Escape, false);
    assert_eq!(picker_rows(&app), 0, "Escape closes the picker");
    enter(&app);
    assert_eq!(*sent.borrow(), vec!["/br".to_string()], "then Enter sends");
}

#[test]
fn urls_and_paths_do_not_open_the_picker() {
    let (app, _dir, sent) = ready("url");
    type_text(&app, "see http://example.invalid/br");
    assert_eq!(picker_rows(&app), 0);
    enter(&app);
    assert_eq!(
        *sent.borrow(),
        vec!["see http://example.invalid/br".to_string()]
    );
    type_text(&app, "/usr/bin");
    assert_eq!(picker_rows(&app), 0, "a path is not a query");
}

#[test]
fn a_slash_that_matches_nothing_leaves_enter_alone() {
    let (app, _dir, sent) = ready("nomatch");
    type_text(&app, "/zzz");
    assert_eq!(picker_rows(&app), 0);
    enter(&app);
    assert_eq!(*sent.borrow(), vec!["/zzz".to_string()]);
}

#[test]
fn settings_add_edit_delete_persist_and_reload() {
    let (app, dir, _sent) = ready("settings");
    let store = app.global::<SnippetStore>();
    let names = |app: &AppWindow| -> Vec<String> {
        app.global::<SnippetStore>()
            .get_items()
            .iter()
            .map(|i| i.name.to_string())
            .collect()
    };
    assert_eq!(names(&app), ["brb", "omw", "thanks"], "the examples");

    // Add, with a multi-line text.
    let save = |n: &str, t: &str, o: &str| store.invoke_save(n.into(), t.into(), o.into());
    assert_eq!(save("lunch", "At lunch\nback at 1", ""), "");
    // Refused: a name that is empty, has a space, repeats one (any case), or no text.
    assert_eq!(save(" ", "x", ""), "name-empty");
    assert_eq!(save("a b", "x", ""), "name-spaces");
    assert_eq!(save("BRB", "x", ""), "name-taken");
    assert_eq!(save("new", "  ", ""), "text-empty");
    // Edit: rename and change the text.
    assert_eq!(save("/lunchtime", "Out", "lunch"), "");
    assert_eq!(names(&app), ["brb", "omw", "thanks", "lunchtime"]);
    store.invoke_remove("omw".into());
    assert_eq!(names(&app), ["brb", "thanks", "lunchtime"]);

    // A new window over the same directory sees the saved list, not the examples.
    let again = app_with_messages(vec![]);
    crate::snippets::install(&again, dir.0.clone(), None);
    assert_eq!(names(&again), ["brb", "thanks", "lunchtime"]);

    // Deleting everything keeps it empty: the examples do not come back.
    for n in ["brb", "thanks", "lunchtime"] {
        again.global::<SnippetStore>().invoke_remove(n.into());
    }
    let third = app_with_messages(vec![]);
    crate::snippets::install(&third, dir.0.clone(), None);
    assert!(names(&third).is_empty());
}
