//! Hands URLs and files to the desktop (browser, file manager, default app).
//!
//! In demo mode (`IRONTEAMS_DEMO`) and in tests nothing is launched: the target is logged and recorded
//! instead, so demo runs and test runs never open the user's browser or apps. Every call that
//! opens something outside IronTeams goes through [`open`].

use std::ffi::OsStr;
use std::sync::Mutex;

/// Targets that were not launched because opening was suppressed, oldest first.
static SUPPRESSED: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub(crate) fn suppressed() -> bool {
    cfg!(test) || compat::var_os("IRONTEAMS_DEMO").is_some()
}

/// Opens `target` (a URL or a path) with the desktop's default handler, without waiting for it.
/// In demo mode and in tests it only records the target.
pub fn open(target: impl AsRef<OsStr>) -> std::io::Result<()> {
    let target = target.as_ref();
    if suppressed() {
        let shown = target.to_string_lossy().into_owned();
        tracing::debug!(target = %shown, "not opened (demo mode or test)");
        SUPPRESSED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(shown);
        return Ok(());
    }
    open::that_detached(target)
}

/// What [`open`] recorded instead of launching (tests).
#[cfg(test)]
pub fn suppressed_targets() -> Vec<String> {
    SUPPRESSED.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_never_launch_and_record_the_target() {
        let url = "https://example.invalid/ironteams-external-open-test";
        assert!(open(url).is_ok());
        assert!(suppressed_targets().iter().any(|t| t == url));
    }
}
