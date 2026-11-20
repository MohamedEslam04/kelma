//! XDG autostart: `$XDG_CONFIG_HOME/autostart/kelma.desktop` (default `~/.config`).

use std::io;
use std::path::{Path, PathBuf};

/// File name of the autostart entry.
pub const FILE_NAME: &str = "kelma.desktop";

/// The desktop entry the app ships; the autostart file reuses its descriptive fields.
const PACKAGED_ENTRY: &str =
    include_str!("../../../packaging/io.github.mohamedeslam04.kelma.desktop");

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

/// `<config_home>/autostart/kelma.desktop` for an explicit config home.
pub fn entry_path_in(config_home: &Path) -> PathBuf {
    config_home.join("autostart").join(FILE_NAME)
}

pub fn enable_in(config_home: &Path, exec: &str, hidden_flag: Option<&str>) -> io::Result<PathBuf> {
    let path = entry_path_in(config_home);
    let dir = path.parent().expect("entry path has a parent");
    std::fs::create_dir_all(dir)?;
    // Write beside the target and rename, so a login never reads a half-written file.
    let tmp = dir.join(format!(".{FILE_NAME}.tmp"));
    std::fs::write(&tmp, desktop_entry(exec, hidden_flag))?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

pub fn disable_in(config_home: &Path) -> io::Result<()> {
    match std::fs::remove_file(entry_path_in(config_home)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

pub fn is_enabled_in(config_home: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(entry_path_in(config_home)) else {
        return false;
    };
    !content
        .lines()
        .any(|l| matches!(l.trim(), "Hidden=true" | "X-GNOME-Autostart-enabled=false"))
}

fn home() -> io::Result<PathBuf> {
    config_home()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no XDG_CONFIG_HOME or HOME"))
}

/// Write the autostart entry for the current user and return its path.
pub fn enable(exec: &str, hidden_flag: Option<&str>) -> io::Result<PathBuf> {
    enable_in(&home()?, exec, hidden_flag)
}

/// Remove the autostart entry (no error when it is already gone).
pub fn disable() -> io::Result<()> {
    disable_in(&home()?)
}

/// Whether the autostart entry exists and is not switched off.
pub fn is_enabled() -> bool {
    config_home().is_some_and(|h| is_enabled_in(&h))
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
        let c = desktop_entry("kelma", Some("--hidden"));
        assert!(c.starts_with("[Desktop Entry]\n"));
        assert_eq!(field(&c, "Type"), Some("Application"));
        assert_eq!(field(&c, "Name"), Some("kelma"));
        assert_eq!(field(&c, "Icon"), Some(crate::APP_ID));
        assert_eq!(field(&c, "StartupWMClass"), Some(crate::APP_ID));
        assert_eq!(field(&c, "Terminal"), Some("false"));
        assert_eq!(field(&c, "Exec"), Some("kelma --hidden"));
        assert_eq!(field(&c, "X-GNOME-Autostart-enabled"), Some("true"));
    }

    #[test]
    fn entry_drops_actions_which_autostart_does_not_need() {
        let c = desktop_entry("kelma", None);
        assert!(!c.contains("[Desktop Action"));
        assert!(field(&c, "Actions").is_none());
        assert_eq!(c.matches("Exec=").count(), 1);
        assert_eq!(field(&c, "Exec"), Some("kelma"));
    }

    #[test]
    fn exec_path_with_spaces_and_specials_is_quoted() {
        let c = desktop_entry("/opt/my apps/kelma", Some("--hidden"));
        assert_eq!(field(&c, "Exec"), Some("\"/opt/my apps/kelma\" --hidden"));
        let c = desktop_entry("/opt/k$elma", None);
        assert_eq!(field(&c, "Exec"), Some("\"/opt/k\\\\$elma\""));
    }

    #[test]
    fn enable_disable_roundtrip_in_a_temp_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_enabled_in(tmp.path()));
        let path = enable_in(tmp.path(), "/usr/bin/kelma", Some("--hidden")).unwrap();
        assert_eq!(path, tmp.path().join("autostart/kelma.desktop"));
        assert!(is_enabled_in(tmp.path()));
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, desktop_entry("/usr/bin/kelma", Some("--hidden")));
        disable_in(tmp.path()).unwrap();
        assert!(!is_enabled_in(tmp.path()));
        assert!(!path.exists());
        disable_in(tmp.path()).unwrap(); // already gone: not an error
    }

    #[test]
    fn enable_twice_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        enable_in(tmp.path(), "kelma", None).unwrap();
        enable_in(tmp.path(), "kelma", Some("--minimized")).unwrap();
        let c = std::fs::read_to_string(entry_path_in(tmp.path())).unwrap();
        assert_eq!(field(&c, "Exec"), Some("kelma --minimized"));
    }

    #[test]
    fn switched_off_entry_is_not_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let path = enable_in(tmp.path(), "kelma", None).unwrap();
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
