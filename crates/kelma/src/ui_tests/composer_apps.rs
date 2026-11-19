//! The composer's "+" apps picker driven through the real window. The engine runs behind it in
//! demo mode (no network): every `command` goes through `ComposerAppsCmd::parse` and the engine,
//! the apps and their search results come from the demo fixtures, and what the window shows is
//! read back from the `ComposerApps` global and from the rendered elements. Nothing here can
//! reach a server, and opening a form is only recorded (`external::open` does not launch in
//! tests).

use super::*;
use crate::backend::ComposerAppsCmd;
use crate::{CaCard, CaCommand, CaResult, ComposerApps};
use i_slint_backend_testing::AccessibleRole;

const TASK_URL: &str = "https://tickets.example.invalid/kelma-demo-task";
const ONE_ON_ONE: &str =
    "19:00000000-0000-0000-0000-000000000001_00000000-0000-0000-0000-000000000002@unq.gbl.spaces";
const CHANNEL: &str = "19:team1@thread.tacv2";

struct Picker {
    h: Harness,
    /// Every `name argument` the picker sent, in order.
    log: Log,
}

impl Picker {
    /// A window with the picker wired to the engine, showing `chat`.
    fn open(chat: &str) -> Self {
        let h = Harness::new();
        h.app.window().set_size(LogicalSize::new(1400., 900.));
        let log = Log::default();
        {
            let (engine, rt, log) = (h.engine.clone(), h.rt.clone(), log.clone());
            h.app.global::<ComposerApps>().on_command(move |name, arg| {
                log.borrow_mut()
                    .push(format!("{name} {arg}").trim().to_owned());
                if let Some(c) = ComposerAppsCmd::parse(&name, &arg) {
                    rt.block_on(engine.clone().handle(Cmd::ComposerApps(c)));
                }
            });
        }
        h.select(chat);
        Picker { h, log }
    }

    fn store(&self) -> ComposerApps<'_> {
        self.h.app.global::<ComposerApps>()
    }

    fn find(&self, label: &str) -> Vec<ElementHandle> {
        ElementHandle::find_by_accessible_label(&self.h.app, label).collect()
    }

    fn has_button(&self, label: &str) -> bool {
        self.find(label)
            .iter()
            .any(|e| e.accessible_role() == Some(AccessibleRole::Button))
    }

    fn click(&self, label: &str) {
        self.find(label)
            .into_iter()
            .find(|e| e.accessible_role() == Some(AccessibleRole::Button))
            .unwrap_or_else(|| panic!("no button labelled {label:?}"))
            .mock_single_click(PointerEventButton::Left);
    }

    /// Opens the picker with the "+" button.
    fn plus(&self) {
        self.click("Actions and apps");
    }

    fn type_in_query(&self, text: &str) {
        self.find("Search text")
            .into_iter()
            .next()
            .expect("the query box")
            .mock_single_click(PointerEventButton::Left);
        type_text(&self.h.app, text);
    }

    /// The commands of the list, without the app headers.
    fn commands(&self) -> Vec<String> {
        self.store()
            .get_commands()
            .iter()
            .filter(|c: &CaCommand| !c.header)
            .map(|c| c.title.to_string())
            .collect()
    }

    fn headers(&self) -> Vec<String> {
        self.store()
            .get_commands()
            .iter()
            .filter(|c: &CaCommand| c.header)
            .map(|c| c.app.to_string())
            .collect()
    }

    fn results(&self) -> Vec<String> {
        self.store()
            .get_results()
            .iter()
            .map(|r: CaResult| r.title.to_string())
            .collect()
    }

    fn cards(&self) -> Vec<String> {
        self.store()
            .get_cards()
            .iter()
            .map(|c: CaCard| c.title.to_string())
            .collect()
    }
}

#[test]
fn the_plus_button_opens_the_picker_with_the_apps_of_the_chat() {
    let p = Picker::open(BACKEND);
    assert!(!p.store().get_open(), "closed until the button is pressed");
    p.plus();
    assert!(p.store().get_open());
    assert_eq!(p.log.borrow().first().map(String::as_str), Some("list"));
    // GIF Search and Contoso Tickets add compose commands. The app with only a message command,
    // the bot-only app and the app that is merely featured do not.
    assert_eq!(p.headers(), ["GIF Search", "Contoso Tickets"]);
    assert_eq!(
        p.commands(),
        ["Search GIFs", "Find a ticket", "Create a ticket"],
        "the ticket command of messages is not for the composer"
    );
    for label in [
        "Search GIFs",
        "Find a ticket",
        "Create a ticket",
        "Close apps",
    ] {
        assert!(p.has_button(label), "{label} is on screen");
    }

    p.click("Close apps");
    assert!(!p.store().get_open());
    assert!(!p.has_button("Search GIFs"), "the picker is gone");
}

#[test]
fn the_commands_listed_depend_on_the_kind_of_chat() {
    let one = Picker::open(ONE_ON_ONE);
    one.plus();
    assert_eq!(
        one.commands(),
        ["Search GIFs"],
        "tickets work in teams and groups"
    );

    // A channel has no composer until a post is started, so the list is asked for directly.
    let channel = Picker::open(CHANNEL);
    channel.h.runner()(Cmd::ComposerApps(ComposerAppsCmd::List));
    assert_eq!(
        channel.commands(),
        ["Search GIFs", "Find a ticket", "Create a ticket"]
    );
}

#[test]
fn typing_a_query_lists_the_fake_results_as_previews() {
    let p = Picker::open(BACKEND);
    p.plus();
    p.click("Search GIFs");
    assert_eq!(p.store().get_command_key(), "0.0.0");
    assert!(
        p.results().is_empty(),
        "nothing is asked before the first letter"
    );

    p.type_in_query("c");
    assert_eq!(p.results(), ["Cat typing", "Capybara bath"]);
    // The box keeps the focus, so more keys go on typing.
    type_text(&p.h.app, "at");
    assert_eq!(p.store().get_query(), "cat");
    assert_eq!(p.results(), ["Cat typing"]);
    let first = p.store().get_results().row_data(0).unwrap();
    assert_eq!(first.subtitle, "Cats");
    assert_eq!(
        first.letter, "C",
        "no thumbnail is loaded offline, so the tile has a letter"
    );
    assert!(p.has_button("Cat typing"));
    assert_eq!(p.store().get_status(), "");

    type_text(&p.h.app, "zz");
    assert!(p.results().is_empty());
    assert_eq!(p.store().get_status(), "No results.");

    // The text went to the engine as it was typed.
    let queries: Vec<String> = p
        .log
        .borrow()
        .iter()
        .filter(|l| l.starts_with("query "))
        .cloned()
        .collect();
    assert_eq!(queries.first().map(String::as_str), Some("query c"));
    assert_eq!(queries.last().map(String::as_str), Some("query catzz"));

    // Back returns to the list.
    p.click("Back");
    assert_eq!(p.store().get_command_key(), "");
    assert!(p.results().is_empty());
    assert!(p.has_button("Search GIFs"));
}

#[test]
fn choosing_a_result_adds_a_card_to_the_draft_and_it_can_be_taken_out() {
    let p = Picker::open(BACKEND);
    p.plus();
    p.click("Find a ticket");
    p.type_in_query("login");
    assert_eq!(
        p.results(),
        ["TCK-101 Login fails"],
        "an adaptive card gets its title from its text"
    );
    p.click("TCK-101 Login fails");

    let cards = p.h.engine.composer_cards(BACKEND);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].app_name, "Contoso Tickets");
    assert_eq!(
        cards[0].content_type,
        "application/vnd.microsoft.card.adaptive"
    );
    assert_eq!(cards[0].content["type"], "AdaptiveCard");
    assert_eq!(cards[0].preview.subtitle, "Open, assigned to Sam");
    assert_eq!(p.cards(), ["TCK-101 Login fails"]);
    assert_eq!(
        p.store().get_note(),
        "Added TCK-101 Login fails to your message."
    );
    assert!(p.log.borrow().iter().any(|l| l == "choose 0"));
    assert!(p.store().get_open(), "the picker stays open for more");

    // Another chat has its own draft.
    p.click("Close apps");
    p.h.select(NOTES);
    p.plus();
    assert!(p.h.engine.composer_cards(NOTES).is_empty());
    assert!(p.cards().is_empty());
    p.click("Close apps");

    // The card is still in the first chat's draft; the picker can drop it.
    p.h.select(BACKEND);
    p.plus();
    assert_eq!(p.cards(), ["TCK-101 Login fails"]);
    p.click("Remove TCK-101 Login fails");
    assert!(p.h.engine.composer_cards(BACKEND).is_empty());
    assert!(p.cards().is_empty());
}

#[test]
fn an_action_command_opens_its_form_and_records_the_target() {
    let p = Picker::open(BACKEND);
    p.plus();
    p.click("Create a ticket");
    assert!(
        crate::external::suppressed_targets()
            .iter()
            .any(|t| t == TASK_URL),
        "the task module link was handed to the opener (nothing launches in tests)"
    );
    assert_eq!(p.store().get_note(), "Opened Create a ticket.");
    assert_eq!(
        p.store().get_command_key(),
        "",
        "an action has no search box"
    );
    assert!(
        p.h.engine.composer_cards(BACKEND).is_empty(),
        "an action adds no card"
    );
    assert!(p.log.borrow().iter().any(|l| l == "action 1.0.1"));
}
