//! Plays meeting recordings and recaps inside IronTeams instead of a browser tab.
//!
//! The recording's Stream page opens in a web host window (`crates/webhost`, see
//! `docs/adr/0003-web-host.md`): a window of its own, without tabs or an address bar, signed in
//! through the web host profile (the user signs in there once). Stream's page brings the player
//! (seek, speed, volume, full screen), the transcript beside the video with click-to-seek, the
//! chapters and, with a license, the AI notes. A native player (GStreamer or ffmpeg decoding into
//! a Slint image) was weighed against it: it needs the codecs and their libraries on every
//! desktop and in CI, and would still have no Stream transcript, chapters or notes.
//!
//! Before the window opens, Graph is asked about the recording's drive item
//! (`teams_proto::recaps::recording_access`), so a recording the user was not given, one that
//! expired and one still being processed get a plain line instead of an error page. When Graph
//! cannot say, the window opens anyway. One player window is kept: playing another recording
//! loads it there and brings the window forward. "Open in browser" stays in the More options
//! menus as the fallback.
//!
//! In demo mode and in tests nothing opens: the target is recorded as `player:<url>` in
//! `Apps::opened`, like the web host's `web-host:<url>`.

use super::Engine;
use std::sync::Arc;
use teams_proto::recaps::RecordingAccess;

/// A recording to play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlayTarget {
    /// Its Stream or SharePoint page (https), as the chat message gave it.
    pub url: String,
    /// The window title until the page sets its own; empty for "Meeting recording".
    pub title: String,
    pub drive_id: Option<String>,
    pub item_id: Option<String>,
}

const NO_RECORDING: &str = "There is no recording to open.";
const NO_ACCESS: &str =
    "You don't have access to this recording. Ask the meeting organizer to share it with you.";
const GONE: &str = "This recording is no longer available. It may have expired or been deleted.";
const PROCESSING: &str = "This recording is still being processed. Try again in a few minutes.";
const NO_BROWSER: &str = "Recordings play in an IronTeams window that needs Chrome, Edge, \
     Chromium or Brave. Install one, or choose Open in browser under More options.";
const PROFILE_IN_USE: &str = "An app window is open. Close it, then play the recording again.";
const NOT_OPENED: &str =
    "The player could not open. Try again, or choose Open in browser under More options.";

/// The page to open for `target` given Graph's answer, or the line shown instead.
pub(super) fn plan_play(
    target: &PlayTarget,
    access: &RecordingAccess,
) -> Result<String, &'static str> {
    match access {
        RecordingAccess::NoAccess => Err(NO_ACCESS),
        RecordingAccess::Gone => Err(GONE),
        RecordingAccess::Processing => Err(PROCESSING),
        RecordingAccess::Ready | RecordingAccess::Unknown => Ok(target.url.clone()),
    }
}

/// The line shown when the window could not open (the error itself only goes to the log).
pub(super) fn open_failure(err: &webhost::Error) -> &'static str {
    match err {
        webhost::Error::NoBrowser => NO_BROWSER,
        webhost::Error::ProfileInUse(_) => PROFILE_IN_USE,
        _ => NOT_OPENED,
    }
}

impl Engine {
    /// Play on a meeting card, which only knows the recording's link.
    pub(super) fn recaps_play_link(self: &Arc<Self>, url: &str) -> anyhow::Result<()> {
        self.play_recording(PlayTarget {
            url: url.to_owned(),
            title: String::new(),
            drive_id: None,
            item_id: None,
        });
        Ok(())
    }

    /// Plays a recording in the player window (see the module docs). Never opens the browser.
    pub(super) fn play_recording(self: &Arc<Self>, target: PlayTarget) {
        if !target.url.starts_with("https://") {
            self.ui.banner(NO_RECORDING.into());
            return;
        }
        self.apps
            .lock()
            .unwrap()
            .opened
            .push(format!("player:{}", target.url));
        if self.apps_offline() || crate::external::suppressed() {
            return;
        }
        let e = self.clone();
        tokio::spawn(async move {
            let access = e
                .client
                .recording_access(
                    target.drive_id.as_deref(),
                    target.item_id.as_deref(),
                    Some(&target.url),
                )
                .await
                .unwrap_or_else(|err| {
                    tracing::debug!(%err, "could not ask Graph about the recording");
                    RecordingAccess::Unknown
                });
            let url = match plan_play(&target, &access) {
                Ok(url) => url,
                Err(line) => {
                    tracing::info!(?access, "the recording cannot be played");
                    e.ui.banner(line.into());
                    return;
                }
            };
            if let Err(err) = e.player_show(&url, &target.title).await {
                tracing::warn!(%err, "the recording player did not open");
                e.ui.banner(open_failure(&err).into());
            }
        });
    }

    /// Loads `url` in the open player window, else opens one and keeps it until it closes.
    async fn player_show(self: &Arc<Self>, url: &str, title: &str) -> webhost::Result<()> {
        let open = self
            .apps
            .lock()
            .unwrap()
            .player
            .clone()
            .filter(|w| !w.is_closed());
        if let Some(win) = open {
            return win.navigate(url).await;
        }
        let paths = session::Paths::new().map_err(|e| webhost::Error::Startup(e.to_string()))?;
        let mut opts = webhost::WebOptions::new(webhost::default_profile_dir(&paths.data_dir));
        opts.title = Some(match title.trim() {
            "" => "Meeting recording".to_owned(),
            t => t.to_owned(),
        });
        opts.size = Some((1280, 800));
        let win = Arc::new(webhost::open(url, opts).await?);
        self.apps.lock().unwrap().player = Some(win.clone());
        win.closed().await;
        let mut a = self.apps.lock().unwrap();
        if a.player.as_ref().is_some_and(|p| Arc::ptr_eq(p, &win)) {
            a.player = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> PlayTarget {
        PlayTarget {
            url: "https://contoso-my.sharepoint.invalid/:v:/g/rec".into(),
            title: "Weekly sync".into(),
            drive_id: Some("b!drive1".into()),
            item_id: Some("01ITEM".into()),
        }
    }

    #[test]
    fn a_playable_recording_opens_its_page_and_the_others_say_why_not() {
        let t = target();
        assert_eq!(plan_play(&t, &RecordingAccess::Ready), Ok(t.url.clone()));
        // Graph could not say: the page opens and Stream says what it knows.
        assert_eq!(plan_play(&t, &RecordingAccess::Unknown), Ok(t.url.clone()));
        assert_eq!(plan_play(&t, &RecordingAccess::NoAccess), Err(NO_ACCESS));
        assert_eq!(plan_play(&t, &RecordingAccess::Gone), Err(GONE));
        assert_eq!(plan_play(&t, &RecordingAccess::Processing), Err(PROCESSING));
        // Calm lines, no status codes or raw errors.
        for line in [NO_ACCESS, GONE, PROCESSING] {
            assert!(!line.contains("HTTP") && !line.chars().any(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn a_window_that_cannot_open_says_what_to_do() {
        assert_eq!(open_failure(&webhost::Error::NoBrowser), NO_BROWSER);
        assert_eq!(
            open_failure(&webhost::Error::ProfileInUse("/tmp/p".into())),
            PROFILE_IN_USE
        );
        assert_eq!(
            open_failure(&webhost::Error::Startup("timed out".into())),
            NOT_OPENED
        );
        assert_eq!(open_failure(&webhost::Error::Closed), NOT_OPENED);
    }
}
