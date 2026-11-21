//! Starting at login. Linux: XDG autostart, `$XDG_CONFIG_HOME/autostart/ironteams.desktop` (default
//! `~/.config`). Windows: a `ironteams` value under the per-user Run key. macOS: a LaunchAgent,
//! `~/Library/LaunchAgents/io.github.mohamedeslam04.ironteams.plist`. [`enable`], [`disable`] and
//! [`is_enabled`] pick the one of the running system; the `*_in` functions always work on the XDG
//! entry in a given directory.

use std::io;
use std::path::{Path, PathBuf};

/// File name of the autostart entry.
pub const FILE_NAME: &str = "ironteams.desktop";
/// The autostart file of the app before the rename from kelma. Enabling autostart removes it
/// (its `Exec` runs a program that no longer exists) and disabling it does too.
const LEGACY_FILE_NAME: &str = "kelma.desktop";

/// The desktop entry the app ships; the autostart file reuses its descriptive fields.
const PACKAGED_ENTRY: &str =
    include_str!("../../../packaging/io.github.mohamedeslam04.ironteams.desktop");

/// Content of the autostart entry running `exec`, with `hidden_flag` (for example `--hidden`)
/// appended so the window stays in the tray at login.
pub fn desktop_entry(exec: &str, hidden_flag: Option<&str>) -> String {
    let mut out = String::from("[Desktop Entry]\n");
    // The descriptive keys of the packaged entry's main group, minus the ones that do not apply
    // to an autostart entry (Exec is replaced, Actions point at groups that are dropped).
    for line in PACKAGED_ENTRY
        .lines()
        .skip_while(|l| *l != "[Desktop Entry]")
        .skip(1)
    {
        if line.starts_with('[') {
            break;
        }
        let key = line.split('=').next().unwrap_or("");
        if line.is_empty() || matches!(key, "Exec" | "Actions") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("Exec=");
    out.push_str(&exec_line(exec, hidden_flag));
    out.push('\n');
    out.push_str("X-GNOME-Autostart-enabled=true\n");
    out
}

/// One argument of an `Exec` line: bare when it has no reserved characters, else double-quoted
/// with `"`, `` ` ``, `$` and `\` escaped (and each backslash doubled again for the file's
/// string escaping), and `%` doubled so it is not read as a field code.
fn exec_arg(arg: &str) -> String {
    let plain = !arg.is_empty()
        && !arg
            .chars()
            .any(|c| c.is_whitespace() || "\"'\\><~|&;$*?#()`%".contains(c));
    if plain {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    for c in arg.chars() {
        match c {
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(c);
            }
            '\\' => out.push_str("\\\\\\\\"),
            '%' => out.push_str("%%"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn exec_line(exec: &str, hidden_flag: Option<&str>) -> String {
    match hidden_flag {
        Some(flag) => format!("{} {}", exec_arg(exec), exec_arg(flag)),
        None => exec_arg(exec),
    }
}

/// `$XDG_CONFIG_HOME` if it is an absolute path, else `$HOME/.config`.
pub fn config_home() -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    match xdg {
        Some(p) if p.is_absolute() => Some(p),
        _ => std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(|h| PathBuf::from(h).join(".config")),
    }
}

/// `<config_home>/autostart/ironteams.desktop` for an explicit config home.
pub fn entry_path_in(config_home: &Path) -> PathBuf {
    config_home.join("autostart").join(FILE_NAME)
}

/// Removes the pre-rename autostart entry, if there is one.
fn remove_legacy_entry_in(config_home: &Path) -> io::Result<()> {
    remove_if_present(&config_home.join("autostart").join(LEGACY_FILE_NAME))
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

pub fn enable_in(config_home: &Path, exec: &str, hidden_flag: Option<&str>) -> io::Result<PathBuf> {
    let path = entry_path_in(config_home);
    let dir = path.parent().expect("entry path has a parent");
    std::fs::create_dir_all(dir)?;
    // Write beside the target and rename, so a login never reads a half-written file.
    let tmp = dir.join(format!(".{FILE_NAME}.tmp"));
    std::fs::write(&tmp, desktop_entry(exec, hidden_flag))?;
    std::fs::rename(&tmp, &path)?;
    remove_legacy_entry_in(config_home)?;
    Ok(path)
}

pub fn disable_in(config_home: &Path) -> io::Result<()> {
    remove_legacy_entry_in(config_home)?;
    remove_if_present(&entry_path_in(config_home))
}

pub fn is_enabled_in(config_home: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(entry_path_in(config_home)) else {
        return false;
    };
    !content
        .lines()
        .any(|l| matches!(l.trim(), "Hidden=true" | "X-GNOME-Autostart-enabled=false"))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn home() -> io::Result<PathBuf> {
    config_home()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no XDG_CONFIG_HOME or HOME"))
}

/// Write the autostart entry for the current user and return its path.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn enable(exec: &str, hidden_flag: Option<&str>) -> io::Result<PathBuf> {
    enable_in(&home()?, exec, hidden_flag)
}

/// Remove the autostart entry (no error when it is already gone).
#[cfg(not(any(windows, target_os = "macos")))]
pub fn disable() -> io::Result<()> {
    disable_in(&home()?)
}

/// Whether the autostart entry exists and is not switched off.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn is_enabled() -> bool {
    config_home().is_some_and(|h| is_enabled_in(&h))
}

// ---- macOS: a LaunchAgent ----

/// The LaunchAgent's label (and file name without `.plist`).
pub const LAUNCH_AGENT_LABEL: &str = crate::APP_ID;

/// `text` with the five XML special characters escaped.
fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// The LaunchAgent property list that runs `exec` (with `hidden_flag`) once at login.
pub fn launch_agent(exec: &str, hidden_flag: Option<&str>) -> String {
    let args: String = std::iter::once(exec)
        .chain(hidden_flag)
        .map(|a| format!("        <string>{}</string>\n", xml_escape(a)))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LAUNCH_AGENT_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
{args}    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#
    )
}

/// The LaunchAgent's label before the rename from kelma.
const LEGACY_LAUNCH_AGENT_LABEL: &str = "io.github.mohamedeslam04.kelma";

fn legacy_launch_agent_path_in(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents")
        .join(format!("{LEGACY_LAUNCH_AGENT_LABEL}.plist"))
}

/// `<home>/Library/LaunchAgents/<label>.plist`.
pub fn launch_agent_path_in(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents")
        .join(format!("{LAUNCH_AGENT_LABEL}.plist"))
}

pub fn launch_agent_enable_in(
    home: &Path,
    exec: &str,
    hidden_flag: Option<&str>,
) -> io::Result<PathBuf> {
    let path = launch_agent_path_in(home);
    let dir = path.parent().expect("agent path has a parent");
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{LAUNCH_AGENT_LABEL}.plist.tmp"));
    std::fs::write(&tmp, launch_agent(exec, hidden_flag))?;
    std::fs::rename(&tmp, &path)?;
    remove_if_present(&legacy_launch_agent_path_in(home))?;
    Ok(path)
}

pub fn launch_agent_disable_in(home: &Path) -> io::Result<()> {
    remove_if_present(&legacy_launch_agent_path_in(home))?;
    remove_if_present(&launch_agent_path_in(home))
}

pub fn launch_agent_enabled_in(home: &Path) -> bool {
    launch_agent_path_in(home).is_file()
}

#[cfg(target_os = "macos")]
fn user_home() -> io::Result<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no HOME"))
}

/// Write the LaunchAgent for the current user and return its path.
#[cfg(target_os = "macos")]
pub fn enable(exec: &str, hidden_flag: Option<&str>) -> io::Result<PathBuf> {
    launch_agent_enable_in(&user_home()?, exec, hidden_flag)
}

/// Remove the LaunchAgent (no error when it is already gone).
#[cfg(target_os = "macos")]
pub fn disable() -> io::Result<()> {
    launch_agent_disable_in(&user_home()?)
}

/// Whether the LaunchAgent exists.
#[cfg(target_os = "macos")]
pub fn is_enabled() -> bool {
    user_home().is_ok_and(|h| launch_agent_enabled_in(&h))
}

// ---- Windows: the Run key ----

/// The per-user key whose values Windows runs at login.
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// The value IronTeams writes there.
pub const RUN_VALUE: &str = "ironteams";
/// The value of the app before the rename from kelma; enabling or disabling removes it.
#[cfg(windows)]
const LEGACY_RUN_VALUE: &str = "kelma";

/// The command line stored in the Run key: the program quoted, then the flag.
pub fn run_command(exec: &str, hidden_flag: Option<&str>) -> String {
    match hidden_flag {
        Some(flag) => format!("\"{exec}\" {flag}"),
        None => format!("\"{exec}\""),
    }
}

#[cfg(windows)]
mod run_key {
    use std::io;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn check(status: u32) -> io::Result<()> {
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(status as i32))
        }
    }

    pub fn set(command: &str) -> io::Result<()> {
        // The pre-rename value would start a program that no longer exists.
        delete_value(super::LEGACY_RUN_VALUE)?;
        let (key, name, data) = (wide(super::RUN_KEY), wide(super::RUN_VALUE), wide(command));
        // SAFETY: every pointer is a NUL-terminated UTF-16 string that outlives the call; the
        // size is the data's length in bytes, terminator included.
        check(unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        })
    }

    pub fn delete() -> io::Result<()> {
        delete_value(super::LEGACY_RUN_VALUE)?;
        delete_value(super::RUN_VALUE)
    }

    fn delete_value(value: &str) -> io::Result<()> {
        let (key, name) = (wide(super::RUN_KEY), wide(value));
        // SAFETY: both strings are NUL-terminated and outlive the call.
        match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) } {
            ERROR_FILE_NOT_FOUND => Ok(()),
            status => check(status),
        }
    }

    pub fn exists() -> bool {
        let (key, name) = (wide(super::RUN_KEY), wide(super::RUN_VALUE));
        // SAFETY: a size query only (no buffer); both strings outlive the call.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        status == ERROR_SUCCESS
    }
}

/// Add IronTeams to the per-user Run key. The returned path names the key, for messages.
#[cfg(windows)]
pub fn enable(exec: &str, hidden_flag: Option<&str>) -> io::Result<PathBuf> {
    run_key::set(&run_command(exec, hidden_flag))?;
    Ok(PathBuf::from(format!(
        r"HKEY_CURRENT_USER\{RUN_KEY}\{RUN_VALUE}"
    )))
}

/// Remove IronTeams from the Run key (no error when it is already gone).
#[cfg(windows)]
pub fn disable() -> io::Result<()> {
    run_key::delete()
}

/// Whether the Run key starts ironteams.
#[cfg(windows)]
pub fn is_enabled() -> bool {
    run_key::exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field<'a>(content: &'a str, key: &str) -> Option<&'a str> {
        content
            .lines()
            .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
    }

    #[test]
    fn entry_reuses_packaged_fields_and_adds_autostart_keys() {
        let c = desktop_entry("ironteams", Some("--hidden"));
        assert!(c.starts_with("[Desktop Entry]\n"));
        assert_eq!(field(&c, "Type"), Some("Application"));
        assert_eq!(field(&c, "Name"), Some("IronTeams"));
        assert_eq!(field(&c, "Icon"), Some(crate::APP_ID));
        assert_eq!(field(&c, "StartupWMClass"), Some(crate::APP_ID));
        assert_eq!(field(&c, "Terminal"), Some("false"));
        assert_eq!(field(&c, "Exec"), Some("ironteams --hidden"));
        assert_eq!(field(&c, "X-GNOME-Autostart-enabled"), Some("true"));
    }

    #[test]
    fn entry_drops_actions_which_autostart_does_not_need() {
        let c = desktop_entry("ironteams", None);
        assert!(!c.contains("[Desktop Action"));
        assert!(field(&c, "Actions").is_none());
        assert_eq!(c.matches("Exec=").count(), 1);
        assert_eq!(field(&c, "Exec"), Some("ironteams"));
    }

    #[test]
    fn exec_path_with_spaces_and_specials_is_quoted() {
        let c = desktop_entry("/opt/my apps/ironteams", Some("--hidden"));
        assert_eq!(
            field(&c, "Exec"),
            Some("\"/opt/my apps/ironteams\" --hidden")
        );
        let c = desktop_entry("/opt/k$elma", None);
        assert_eq!(field(&c, "Exec"), Some("\"/opt/k\\\\$elma\""));
    }

    #[test]
    fn enable_disable_roundtrip_in_a_temp_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_enabled_in(tmp.path()));
        let path = enable_in(tmp.path(), "/usr/bin/ironteams", Some("--hidden")).unwrap();
        assert_eq!(path, tmp.path().join("autostart/ironteams.desktop"));
        assert!(is_enabled_in(tmp.path()));
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            written,
            desktop_entry("/usr/bin/ironteams", Some("--hidden"))
        );
        disable_in(tmp.path()).unwrap();
        assert!(!is_enabled_in(tmp.path()));
        assert!(!path.exists());
        disable_in(tmp.path()).unwrap(); // already gone: not an error
    }

    #[test]
    fn enable_twice_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        enable_in(tmp.path(), "ironteams", None).unwrap();
        enable_in(tmp.path(), "ironteams", Some("--minimized")).unwrap();
        let c = std::fs::read_to_string(entry_path_in(tmp.path())).unwrap();
        assert_eq!(field(&c, "Exec"), Some("ironteams --minimized"));
    }

    fn write_legacy_entry(config_home: &Path) -> PathBuf {
        let legacy = config_home.join("autostart").join(LEGACY_FILE_NAME);
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "[Desktop Entry]\nExec=kelma --hidden\n").unwrap();
        legacy
    }

    #[test]
    fn enabling_removes_the_entry_the_old_name_left() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = write_legacy_entry(tmp.path());
        let path = enable_in(tmp.path(), "ironteams", None).unwrap();
        assert!(path.exists());
        assert!(!legacy.exists());
    }

    #[test]
    fn disabling_removes_both_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = write_legacy_entry(tmp.path());
        let path = enable_in(tmp.path(), "ironteams", None).unwrap();
        write_legacy_entry(tmp.path());
        disable_in(tmp.path()).unwrap();
        assert!(!path.exists() && !legacy.exists());
    }

    #[test]
    fn the_launch_agent_of_the_old_name_is_removed_too() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = legacy_launch_agent_path_in(tmp.path());
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "<plist/>").unwrap();
        launch_agent_enable_in(tmp.path(), "/k", None).unwrap();
        assert!(!legacy.exists());
        assert!(launch_agent_enabled_in(tmp.path()));
        std::fs::write(&legacy, "<plist/>").unwrap();
        launch_agent_disable_in(tmp.path()).unwrap();
        assert!(!legacy.exists() && !launch_agent_enabled_in(tmp.path()));
    }

    #[test]
    fn the_launch_agent_runs_the_program_hidden_at_login() {
        let plist = launch_agent(
            "/Applications/ironteams.app/Contents/MacOS/ironteams",
            Some("--hidden"),
        );
        assert!(plist.contains("<string>io.github.mohamedeslam04.ironteams</string>"));
        assert!(plist.contains(
            "<string>/Applications/ironteams.app/Contents/MacOS/ironteams</string>\n        <string>--hidden</string>"
        ));
        assert!(plist.contains("<key>RunAtLoad</key>\n    <true/>"));
        assert!(launch_agent("/a&b/<k>", None).contains("<string>/a&amp;b/&lt;k&gt;</string>"));
    }

    #[test]
    fn launch_agent_roundtrip_in_a_temp_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!launch_agent_enabled_in(tmp.path()));
        let path = launch_agent_enable_in(tmp.path(), "/k", Some("--hidden")).unwrap();
        assert_eq!(
            path,
            tmp.path()
                .join("Library/LaunchAgents/io.github.mohamedeslam04.ironteams.plist")
        );
        assert!(launch_agent_enabled_in(tmp.path()));
        launch_agent_disable_in(tmp.path()).unwrap();
        assert!(!launch_agent_enabled_in(tmp.path()));
        launch_agent_disable_in(tmp.path()).unwrap();
    }

    #[test]
    fn the_run_key_command_quotes_the_program() {
        assert_eq!(
            run_command(
                r"C:\Program Files\ironteams\ironteams.exe",
                Some("--hidden")
            ),
            r#""C:\Program Files\ironteams\ironteams.exe" --hidden"#
        );
        assert_eq!(run_command("k.exe", None), r#""k.exe""#);
    }

    #[test]
    fn switched_off_entry_is_not_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let path = enable_in(tmp.path(), "ironteams", None).unwrap();
        let off = std::fs::read_to_string(&path).unwrap().replace(
            "X-GNOME-Autostart-enabled=true",
            "X-GNOME-Autostart-enabled=false",
        );
        std::fs::write(&path, off).unwrap();
        assert!(!is_enabled_in(tmp.path()));
        let hidden = std::fs::read_to_string(&path).unwrap() + "Hidden=true\n";
        std::fs::write(&path, hidden).unwrap();
        assert!(!is_enabled_in(tmp.path()));
    }
}
