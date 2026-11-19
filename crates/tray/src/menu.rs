//! The tray context menu as plain data, so it is testable without a bus.

/// What a menu entry does when picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    Open,
    ToggleMute,
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEntry {
    Item {
        label: &'static str,
        /// Freedesktop icon name shown next to the label (empty for none).
        icon: &'static str,
        /// `Some` makes the entry a checkmark item with this state.
        checked: Option<bool>,
        action: MenuAction,
    },
    Separator,
}

/// Open, Mute notifications (checkmark), separator, Quit.
pub fn menu_model(muted: bool) -> Vec<MenuEntry> {
    vec![
        MenuEntry::Item {
            label: "Open kelma",
            icon: "window-new",
            checked: None,
            action: MenuAction::Open,
        },
        MenuEntry::Item {
            label: "Mute notifications",
            icon: "notifications-disabled",
            checked: Some(muted),
            action: MenuAction::ToggleMute,
        },
        MenuEntry::Separator,
        MenuEntry::Item {
            label: "Quit",
            icon: "application-exit",
            checked: None,
            action: MenuAction::Quit,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_has_open_mute_separator_quit_in_order() {
        let m = menu_model(false);
        assert_eq!(m.len(), 4);
        assert!(matches!(
            &m[0],
            MenuEntry::Item {
                label: "Open kelma",
                action: MenuAction::Open,
                checked: None,
                ..
            }
        ));
        assert!(matches!(
            &m[1],
            MenuEntry::Item {
                label: "Mute notifications",
                action: MenuAction::ToggleMute,
                checked: Some(false),
                ..
            }
        ));
        assert_eq!(m[2], MenuEntry::Separator);
        assert!(matches!(
            &m[3],
            MenuEntry::Item {
                label: "Quit",
                action: MenuAction::Quit,
                checked: None,
                ..
            }
        ));
    }

    #[test]
    fn mute_checkmark_follows_state() {
        let m = menu_model(true);
        assert!(matches!(
            &m[1],
            MenuEntry::Item {
                checked: Some(true),
                ..
            }
        ));
    }
}
