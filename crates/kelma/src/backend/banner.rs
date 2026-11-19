//! The desktop notification for a new message, shaped like Teams': the chat as the title, the
//! sender in bold above the message in a group, a camera line for a picture, the sender's photo,
//! and Reply and Like buttons. Clicking it opens the chat.
//!
//! What the notification server can do is asked once (`body-markup`, `actions`); without
//! markup the sender is written as "Name: message". Settings › Notifications turns the photo and
//! the buttons off (`GeneralPrefs::banner_photo`, `banner_actions`).

use super::Engine;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

/// Summary and body of a message notification.
#[derive(Debug, PartialEq, Eq)]
pub struct Banner {
    pub summary: String,
    pub body: String,
}

/// The message as one notification line: pictures as "📷 Image", nothing as "New message".
pub fn friendly(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| if l == "[image]" { "📷 Image" } else { l })
        .collect();
    if lines.is_empty() {
        "New message".into()
    } else {
        lines.join(" ")
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Builds the notification. `title` is the chat's name (empty or the sender's name for a 1:1),
/// `line` the message as [`friendly`] made it (or "New message" when previews are off).
pub fn compose(sender: &str, title: &str, line: &str, markup: bool) -> Banner {
    let group = !title.is_empty() && title != sender;
    let summary = if group { title } else { sender }.to_string();
    let body = match (group, markup) {
        (true, true) => format!("<b>{}</b>\n{}", escape(sender), escape(line)),
        (true, false) => format!("{sender}: {line}"),
        (false, true) => escape(line),
        (false, false) => line.to_string(),
    };
    Banner { summary, body }
}

/// What the notification server supports, asked once.
struct Caps {
    markup: bool,
    actions: bool,
}

fn caps() -> &'static Caps {
    static CAPS: OnceLock<Caps> = OnceLock::new();
    CAPS.get_or_init(|| {
        let c = notify_rust::get_capabilities().unwrap_or_default();
        Caps {
            markup: c.iter().any(|x| x == "body-markup"),
            actions: c.iter().any(|x| x == "actions"),
        }
    })
}

/// The sender's photo as a file the notification server can read (written once per person).
fn photo_file(mri: &str, bytes: &[u8]) -> Option<PathBuf> {
    let dir = session::Paths::new().ok()?.cache_dir.join("notify");
    std::fs::create_dir_all(&dir).ok()?;
    let name: String = mri
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let path = dir.join(format!("{name}.img"));
    if !path.exists() {
        std::fs::write(&path, bytes).ok()?;
    }
    Some(path)
}

/// One message notification, ready to show.
pub(super) struct Pending {
    pub conversation: String,
    pub message: String,
    pub sender: String,
    pub sender_mri: String,
    pub title: String,
    pub line: String,
    pub photo: Option<Vec<u8>>,
}

impl Engine {
    /// Shows the notification on a blocking thread and acts on its buttons.
    pub(super) fn show_banner(self: &Arc<Self>, p: Pending) {
        let prefs = session::Paths::new()
            .map(|paths| super::settings::Local::load(&paths.config_dir).general)
            .unwrap_or_default();
        let e = self.clone();
        let rt = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let caps = caps();
            let b = compose(&p.sender, &p.title, &p.line, caps.markup);
            let mut n = notify_rust::Notification::new();
            n.appname("kelma")
                .summary(&b.summary)
                .body(&b.body)
                .icon("kelma")
                .hint(notify_rust::Hint::DesktopEntry("kelma".into()))
                .hint(notify_rust::Hint::Category("im.received".into()));
            if prefs.banner_photo
                && let Some(path) = p
                    .photo
                    .as_deref()
                    .and_then(|b| photo_file(&p.sender_mri, b))
            {
                n.image_path(&path.to_string_lossy());
            }
            let actions = prefs.banner_actions && caps.actions;
            if actions {
                n.action("default", "Open")
                    .action("reply", "Reply")
                    .action("like", "Like");
            }
            let Ok(handle) = n.show() else { return };
            if !actions {
                return;
            }
            handle.wait_for_action(|action| {
                let (e, conv, msg) = (e.clone(), p.conversation.clone(), p.message.clone());
                match action {
                    "default" | "reply" => {
                        let reply = action == "reply";
                        rt.spawn(async move {
                            if let Err(err) = e.open(conv, None).await {
                                tracing::warn!(%err, "notification: could not open the chat");
                            }
                            e.ui.raise(reply);
                        });
                    }
                    "like" => {
                        rt.spawn(async move {
                            if let Err(err) = e.react(&msg, "like").await {
                                tracing::warn!(%err, "notification: could not like the message");
                            }
                        });
                    }
                    _ => {}
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_and_empty_messages_read_like_teams() {
        assert_eq!(friendly("[image]"), "📷 Image");
        assert_eq!(friendly("look\n[image]"), "look 📷 Image");
        assert_eq!(friendly("  \n"), "New message");
        assert_eq!(friendly("hello\nthere"), "hello there");
    }

    #[test]
    fn a_group_shows_the_chat_then_the_sender_in_bold() {
        let b = compose("Abdullah", "Tech Team", "📷 Image", true);
        assert_eq!(b.summary, "Tech Team");
        assert_eq!(b.body, "<b>Abdullah</b>\n📷 Image");
        let plain = compose("Abdullah", "Tech Team", "hi", false);
        assert_eq!(plain.body, "Abdullah: hi");
    }

    #[test]
    fn a_one_to_one_chat_shows_the_sender_and_escapes_markup() {
        let b = compose("Sara", "Sara", "a <b> & c", true);
        assert_eq!(b.summary, "Sara");
        assert_eq!(b.body, "a &lt;b&gt; &amp; c");
        assert_eq!(compose("Sara", "", "x", true).summary, "Sara");
    }
}
