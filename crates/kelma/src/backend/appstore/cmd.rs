//! The commands the app screens send, as `(name, argument)` strings from the `AppsStore`,
//! `TasksStore`, `FlowsStore`, `NotesStore` and `ListsStore` globals.

/// Everything the app bar, the Store and the built-in app screens ask the engine to do. None of
/// them writes to the server: pin, add and remove only show the request they would send.
#[derive(Debug, Clone, PartialEq)]
pub enum AppsCmd {
    /// Loads the app bar (the engine does this at start-up).
    Load,
    // -- app bar and flyout --
    RailCollapse,
    /// Pin an installed app to the bar (kept locally; the request is shown, not sent).
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
    /// `me`, `team`, `chat` or `meeting`: shows the install request, never sends it.
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
    // -- Workflows --
    FlowsEnter,
    FlowsSelect(String),
    FlowsRefresh,
    // -- OneNote --
    NotesEnter,
    NotesNotebook(String),
    NotesSection(String),
    NotesPage(String),
    NotesOpenWeb,
    // -- Lists --
    ListsEnter,
    ListsSite(String),
    ListsList(String),
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
            "flows-enter" => AppsCmd::FlowsEnter,
            "flows-select" => AppsCmd::FlowsSelect(s()),
            "flows-refresh" => AppsCmd::FlowsRefresh,
            "notes-enter" => AppsCmd::NotesEnter,
            "notes-notebook" => AppsCmd::NotesNotebook(s()),
            "notes-section" => AppsCmd::NotesSection(s()),
            "notes-page" => AppsCmd::NotesPage(s()),
            "notes-open-web" => AppsCmd::NotesOpenWeb,
            "lists-enter" => AppsCmd::ListsEnter,
            "lists-site" => AppsCmd::ListsSite(s()),
            "lists-list" => AppsCmd::ListsList(s()),
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
        assert_eq!(AppsCmd::parse("shortcut", "x"), None);
        assert_eq!(AppsCmd::parse("nope", ""), None);
    }
}
