//! Finding and launching a Chromium-family browser.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Browsers tried in order when no path is configured. All are Chromium-based and speak CDP.
pub const CANDIDATES: &[&str] = &[
    "chromium",
    "chromium-browser",
    "google-chrome",
    "google-chrome-stable",
    "microsoft-edge",
    "microsoft-edge-stable",
    "brave-browser",
    "brave",
    "vivaldi",
    "vivaldi-stable",
];

/// The empty page a host window starts on. A `data:` URL, because some browsers (Vivaldi)
/// replace `about:blank` with their start page.
pub const START_URL: &str = "data:text/html,";

/// Environment variable that overrides browser discovery.
pub const BROWSER_ENV: &str = "KELMA_BROWSER";

/// The first of `candidates` found as an executable file in `path_var` (a `PATH` value).
pub fn find_in_path(path_var: &OsStr, candidates: &[&str]) -> Option<PathBuf> {
    candidates.iter().find_map(|name| {
        std::env::split_paths(path_var)
            .map(|dir| dir.join(name))
            .find(|p| is_executable(p))
    })
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Resolves the browser: an explicit path, else `$KELMA_BROWSER`, else the first candidate on
/// `PATH`.
pub fn find_browser(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return is_executable(p).then(|| p.to_path_buf());
    }
    if let Some(p) = std::env::var_os(BROWSER_ENV).filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        return is_executable(&p).then_some(p);
    }
    find_in_path(&std::env::var_os("PATH")?, CANDIDATES)
}

/// Command-line arguments for a host window. The window starts on [`START_URL`]; the page is
/// loaded over CDP once the shim is registered, so the shim runs before the app's scripts.
pub fn launch_args(profile: &Path, size: Option<(u32, u32)>) -> Vec<String> {
    let mut args = vec![
        format!("--app={START_URL}"),
        format!("--user-data-dir={}", profile.display()),
        "--remote-debugging-port=0".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--class=kelma-webhost".into(),
    ];
    if let Some((w, h)) = size {
        args.push(format!("--window-size={w},{h}"));
    }
    args
}

/// Parses `<profile>/DevToolsActivePort`: the port on the first line and the browser target
/// path (`/devtools/browser/<id>`) on the second.
pub fn parse_devtools_active_port(text: &str) -> Option<(u16, String)> {
    let mut lines = text.lines();
    let port = lines.next()?.trim().parse().ok()?;
    let path = lines.next()?.trim();
    path.starts_with('/').then(|| (port, path.to_string()))
}

/// Whether another running browser holds `profile` (its `SingletonLock` symlink names
/// `<hostname>-<pid>` of a live process). A second launch would hand the URL to that browser
/// and exit, so the host could not drive the window.
pub fn profile_in_use(profile: &Path) -> bool {
    std::fs::read_link(profile.join("SingletonLock"))
        .ok()
        .and_then(|t| parse_singleton_lock(&t.to_string_lossy()))
        .is_some_and(|pid| Path::new(&format!("/proc/{pid}")).exists())
}

/// Sends `signal` to the process group `pgid`.
pub(crate) fn signal_group(pgid: u32, signal: i32) {
    // SAFETY: killpg only sends a signal; a stale group id makes it fail with ESRCH.
    unsafe {
        libc::killpg(pgid as libc::pid_t, signal);
    }
}

/// Whether any process of group `pgid` is still running.
pub(crate) fn group_alive(pgid: u32) -> bool {
    // SAFETY: signal 0 only checks that the group exists.
    unsafe { libc::killpg(pgid as libc::pid_t, 0) == 0 }
}

fn parse_singleton_lock(target: &str) -> Option<u32> {
    target.rsplit_once('-')?.1.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn touch_exe(dir: &Path, name: &str, mode: u32) {
        let p = dir.join(name);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn finds_the_first_candidate_that_is_an_executable_file() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        touch_exe(a.path(), "vivaldi", 0o755);
        touch_exe(b.path(), "brave-browser", 0o755);
        touch_exe(b.path(), "chromium", 0o644); // not executable
        std::fs::create_dir(a.path().join("google-chrome")).unwrap(); // a directory
        let path = std::env::join_paths([a.path(), b.path()]).unwrap();
        assert_eq!(
            find_in_path(&path, CANDIDATES),
            Some(b.path().join("brave-browser"))
        );
        assert_eq!(find_in_path(&path, &["nothing"]), None);
    }

    #[test]
    fn an_explicit_browser_path_wins_when_it_exists() {
        let d = tempfile::tempdir().unwrap();
        touch_exe(d.path(), "my-chrome", 0o755);
        let p = d.path().join("my-chrome");
        assert_eq!(find_browser(Some(&p)), Some(p));
        assert_eq!(find_browser(Some(&d.path().join("missing"))), None);
    }

    #[test]
    fn launch_args_use_an_app_window_a_private_profile_and_a_free_debug_port() {
        let args = launch_args(Path::new("/data/kelma/webhost-profile"), Some((1200, 800)));
        for want in [
            "--app=data:text/html,",
            "--user-data-dir=/data/kelma/webhost-profile",
            "--remote-debugging-port=0",
            "--no-first-run",
            "--no-default-browser-check",
            "--window-size=1200,800",
        ] {
            assert!(args.iter().any(|a| a == want), "missing {want} in {args:?}");
        }
        let no_size = launch_args(Path::new("/p"), None);
        assert!(!no_size.iter().any(|a| a.starts_with("--window-size")));
    }

    #[test]
    fn parses_devtools_active_port() {
        assert_eq!(
            parse_devtools_active_port("39123\n/devtools/browser/0b1c-2d\n"),
            Some((39123, "/devtools/browser/0b1c-2d".into()))
        );
        assert_eq!(parse_devtools_active_port("39123\n"), None);
        assert_eq!(parse_devtools_active_port("x\n/devtools/browser/1"), None);
        assert_eq!(parse_devtools_active_port(""), None);
    }

    #[test]
    fn parses_the_pid_from_a_singleton_lock_target() {
        assert_eq!(parse_singleton_lock("fedora-box-4242"), Some(4242));
        assert_eq!(parse_singleton_lock("host"), None);
        assert_eq!(parse_singleton_lock("host-abc"), None);
    }

    #[test]
    fn a_profile_is_in_use_only_while_its_lock_owner_lives() {
        let d = tempfile::tempdir().unwrap();
        assert!(!profile_in_use(d.path()), "no lock");
        let lock = d.path().join("SingletonLock");
        std::os::unix::fs::symlink(format!("host-{}", std::process::id()), &lock).unwrap();
        assert!(profile_in_use(d.path()), "this test process is alive");
        std::fs::remove_file(&lock).unwrap();
        // pid_max is at most 2^22 on Linux, so this pid cannot exist.
        std::os::unix::fs::symlink("host-99999999", &lock).unwrap();
        assert!(!profile_in_use(d.path()), "stale lock");
    }
}
