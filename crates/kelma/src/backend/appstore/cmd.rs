//! The commands the app screens send, as `(name, argument)` strings from the `AppsStore`,
//! `TasksStore`, `FlowsStore`, `NotesStore`, `ListsStore` and `RecapsStore` globals.

/// Everything the app bar, the Store and the built-in app screens ask the engine to do. The ones
/// that change something on the server (add, remove, update, a task, a flow, a page, an item)
/// first open the confirm dialog (`Form*`); the request is sent when the user confirms.
#[derive(Debug, Clone, PartialEq)]
pub enum AppsCmd {
    /// Loads the app bar (the engine does this at start-up).
    Load,
    // -- app bar and flyout --
    RailCollapse,
    /// Pin an installed app to the bar (kept here, and sent to Teams).
    Pin(String),
    Unpin(String),
    /// Moves a pinned app to a position among the pinned apps (0 is first).
    Move(String, usize),
    /// Ctrl+Shift+1 to 9: the nth rail entry.
    Shortcut(usize),
    FlyoutOpen,
    FlyoutClose,
    FlyoutQuery(String),
    /// Opens an installed app in the main area.
    OpenApp(String),
    // -- the Store --
    /// The Store view appeared: loads the home the first time.
    StoreEnter,
    StoreOpen,
    StoreHome,
    StoreSearch(String),
    StoreCategory(String),
    /// `all`, `personal`, `chat`, `team` or `meeting`.
    StoreScope(String),
    /// `all`, `apps` or `agents`.
    StoreKind(String),
    AppPage(String),
    /// `me`, `team`, `chat` or `meeting`: opens the confirm dialog of an install.
    AppAdd(String),
    AppRemove(String),
    AppUpdate(String),
    AppShare(String),
    /// `privacy`, `terms`, `devices` or `web` (the app in Teams on the web).
    AppLink(String),
    Manage,
    StoreSettings,
    /// `<app id>|<permission>|on` or `…|off`.
    Grant(String, String, bool),
    // -- Planner and To Do --
    TasksEnter,
    TasksNav(String),
    TasksMode(String),
    TasksRefresh,
    /// Opens the New task dialog for the open list or plan.
    TaskNew,
    /// More actions › Create a task: the message's text.
    TaskFromMessage(String),
    /// Completes a task, or undoes it.
    TaskToggle(String),
    TaskEdit(String),
    TaskDelete(String),
    // -- Workflows --
    FlowsEnter,
    FlowsSelect(String),
    FlowsRefresh,
    /// Asks to run a flow now.
    FlowRun(String),
    /// Asks to turn a flow on or off.
    FlowToggle(String),
    // -- OneNote --
    NotesEnter,
    NotesNotebook(String),
    NotesSection(String),
    NotesPage(String),
    NotesOpenWeb,
    /// Opens the New page dialog for the open section.
    PageNew,
    // -- Lists --
    ListsEnter,
    ListsSite(String),
    ListsList(String),
    ItemNew,
    ItemEdit(String),
    // -- the confirm and input dialog --
    /// The confirm button: the text fields joined with a newline.
    FormSubmit(String),
    FormCancel,
    /// A choice of the dialog was picked.
    FormChoice(String),
    /// The dialog's link button.
    FormLink,
    // -- Meeting Recaps --
    RecapsEnter,
    RecapsRefresh,
    RecapsQuery(String),
    /// Opens a meeting's recap (the calendar event id).
    RecapsOpen(String),
    /// Back to the list of meetings.
    RecapsHome,
    /// Picks a recording part (0 is Part 1).
    RecapsPart(usize),
    /// `notes` or `transcript`.
    RecapsTab(String),
    /// Opens the recording in the browser.
    RecapsWatch,
    /// Copies the recording's link.
    RecapsShare,
    RecapsNotes,
    RecapsTranscriptWeb,
    /// Opens a file shared in the meeting chat (its address).
    RecapsFile(String),
    /// The Recap tab of a meeting chat (its thread id).
    RecapsChat(String),
}

impl AppsCmd {
    pub fn parse(name: &str, arg: &str) -> Option<AppsCmd> {
        let s = || arg.to_owned();
        Some(match name {
            "load" => AppsCmd::Load,
            "rail-collapse" => AppsCmd::RailCollapse,
            "pin" => AppsCmd::Pin(s()),
            "unpin" => AppsCmd::Unpin(s()),
            "move" => {
                let (id, to) = arg.rsplit_once(',')?;
                AppsCmd::Move(id.into(), to.parse().ok()?)
            }
            "shortcut" => AppsCmd::Shortcut(arg.parse().ok()?),
            "flyout-open" => AppsCmd::FlyoutOpen,
            "flyout-close" => AppsCmd::FlyoutClose,
            "flyout-query" => AppsCmd::FlyoutQuery(s()),
            "open-app" => AppsCmd::OpenApp(s()),
            "store-enter" => AppsCmd::StoreEnter,
            "store-open" => AppsCmd::StoreOpen,
            "store-home" => AppsCmd::StoreHome,
            "store-search" => AppsCmd::StoreSearch(s()),
            "store-category" => AppsCmd::StoreCategory(s()),
            "store-scope" => AppsCmd::StoreScope(s()),
            "store-kind" => AppsCmd::StoreKind(s()),
            "app-page" => AppsCmd::AppPage(s()),
            "app-add" => AppsCmd::AppAdd(s()),
            "app-remove" => AppsCmd::AppRemove(s()),
            "app-update" => AppsCmd::AppUpdate(s()),
            "app-share" => AppsCmd::AppShare(s()),
            "app-link" => AppsCmd::AppLink(s()),
            "manage" => AppsCmd::Manage,
            "store-settings" => AppsCmd::StoreSettings,
            "grant" => {
                let mut p = arg.splitn(3, '|');
                let (app, perm, on) = (p.next()?, p.next()?, p.next()?);
                AppsCmd::Grant(app.into(), perm.into(), on == "on")
            }
            "tasks-enter" => AppsCmd::TasksEnter,
            "tasks-nav" => AppsCmd::TasksNav(s()),
            "tasks-mode" => AppsCmd::TasksMode(s()),
            "tasks-refresh" => AppsCmd::TasksRefresh,
            "task-new" => AppsCmd::TaskNew,
            "task-from-message" => AppsCmd::TaskFromMessage(s()),
            "task-toggle" => AppsCmd::TaskToggle(s()),
            "task-edit" => AppsCmd::TaskEdit(s()),
            "task-delete" => AppsCmd::TaskDelete(s()),
            "flows-enter" => AppsCmd::FlowsEnter,
            "flows-select" => AppsCmd::FlowsSelect(s()),
            "flows-refresh" => AppsCmd::FlowsRefresh,
            "flow-run" => AppsCmd::FlowRun(s()),
            "flow-toggle" => AppsCmd::FlowToggle(s()),
            "notes-enter" => AppsCmd::NotesEnter,
            "notes-notebook" => AppsCmd::NotesNotebook(s()),
            "notes-section" => AppsCmd::NotesSection(s()),
            "notes-page" => AppsCmd::NotesPage(s()),
            "notes-open-web" => AppsCmd::NotesOpenWeb,
            "page-new" => AppsCmd::PageNew,
            "lists-enter" => AppsCmd::ListsEnter,
            "lists-site" => AppsCmd::ListsSite(s()),
            "lists-list" => AppsCmd::ListsList(s()),
            "item-new" => AppsCmd::ItemNew,
            "item-edit" => AppsCmd::ItemEdit(s()),
            "form-submit" => AppsCmd::FormSubmit(s()),
            "form-cancel" => AppsCmd::FormCancel,
            "form-choice" => AppsCmd::FormChoice(s()),
            "form-link" => AppsCmd::FormLink,
            "recaps-enter" => AppsCmd::RecapsEnter,
            "recaps-refresh" => AppsCmd::RecapsRefresh,
            "recaps-query" => AppsCmd::RecapsQuery(s()),
            "recaps-open" => AppsCmd::RecapsOpen(s()),
            "recaps-home" => AppsCmd::RecapsHome,
            "recaps-part" => AppsCmd::RecapsPart(arg.parse().ok()?),
            "recaps-tab" => AppsCmd::RecapsTab(s()),
            "recaps-watch" => AppsCmd::RecapsWatch,
            "recaps-share" => AppsCmd::RecapsShare,
            "recaps-notes" => AppsCmd::RecapsNotes,
            "recaps-transcript-web" => AppsCmd::RecapsTranscriptWeb,
            "recaps-file" => AppsCmd::RecapsFile(s()),
            "recaps-chat" => AppsCmd::RecapsChat(s()),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_commands_with_arguments() {
        assert_eq!(
            AppsCmd::parse("move", "com.app.id,3"),
            Some(AppsCmd::Move("com.app.id".into(), 3))
        );
        assert_eq!(AppsCmd::parse("move", "no-index"), None);
        assert_eq!(
            AppsCmd::parse("grant", "app|media|on"),
            Some(AppsCmd::Grant("app".into(), "media".into(), true))
        );
        assert_eq!(
            AppsCmd::parse("grant", "app|media|off"),
            Some(AppsCmd::Grant("app".into(), "media".into(), false))
        );
        assert_eq!(
            AppsCmd::parse("form-submit", "a\nb"),
            Some(AppsCmd::FormSubmit("a\nb".into()))
        );
        assert_eq!(
            AppsCmd::parse("task-toggle", "t1"),
            Some(AppsCmd::TaskToggle("t1".into()))
        );
        assert_eq!(
            AppsCmd::parse("recaps-watch", ""),
            Some(AppsCmd::RecapsWatch)
        );
        assert_eq!(
            AppsCmd::parse("recaps-part", "1"),
            Some(AppsCmd::RecapsPart(1))
        );
        assert_eq!(AppsCmd::parse("recaps-part", "x"), None);
        assert_eq!(AppsCmd::parse("shortcut", "x"), None);
        assert_eq!(AppsCmd::parse("nope", ""), None);
    }
}
