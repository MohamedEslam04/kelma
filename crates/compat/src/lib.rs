//! What still works from before kelma became IronTeams.
//!
//! - Environment variables: `IRONTEAMS_X` is read first, then the old `KELMA_X`.
//! - The per-user config, data and cache directories: moved once from `kelma` to `ironteams`.

use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::sync::Once;

const NEW_PREFIX: &str = "IRONTEAMS_";
const OLD_PREFIX: &str = "KELMA_";

/// The old name of an `IRONTEAMS_*` variable.
fn legacy_name(name: &str) -> Option<String> {
    name.strip_prefix(NEW_PREFIX)
        .map(|rest| format!("{OLD_PREFIX}{rest}"))
}

/// `std::env::var` for an `IRONTEAMS_*` name, falling back to the old `KELMA_*` one when the new
/// one is not set.
pub fn var(name: &str) -> Result<String, std::env::VarError> {
    match std::env::var(name) {
        Err(std::env::VarError::NotPresent) => match legacy_name(name) {
            Some(old) => std::env::var(old),
            None => Err(std::env::VarError::NotPresent),
        },
        other => other,
    }
}

/// `std::env::var_os` for an `IRONTEAMS_*` name, falling back to the old `KELMA_*` one.
pub fn var_os(name: &str) -> Option<OsString> {
    std::env::var_os(name).or_else(|| legacy_name(name).and_then(std::env::var_os))
}

/// What [`migrate_dir`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Migrated {
    /// Nothing to do: there was no old directory, or the new one exists already.
    No,
    /// The old directory was renamed to the new one.
    Renamed,
    /// The rename failed (another file system, say), so the tree was copied and the old one removed.
    Copied,
}

/// Moves `old` to `new` when `new` does not exist and `old` does: a rename, else a recursive
/// copy. A failed copy leaves `old` untouched and no partial `new`.
pub fn migrate_dir(old: &Path, new: &Path) -> io::Result<Migrated> {
    if new.exists() || !old.is_dir() {
        return Ok(Migrated::No);
    }
    if let Some(parent) = new.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::rename(old, new).is_ok() {
        return Ok(Migrated::Renamed);
    }
    if let Err(e) = copy_tree(old, new) {
        let _ = std::fs::remove_dir_all(new);
        return Err(e);
    }
    // The data is safe in `new`; a leftover old tree is only clutter.
    let _ = std::fs::remove_dir_all(old);
    Ok(Migrated::Copied)
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Moves the config, data and cache directories of the `kelma` app to the `ironteams` ones,
/// once per process. Call it before anything opens the store, the session or the preferences.
/// Does nothing when `IRONTEAMS_HOME` (or the old `KELMA_HOME`) places the directories.
pub fn migrate_legacy_dirs() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if var_os("IRONTEAMS_HOME").is_some() {
            return;
        }
        let (Some(old), Some(new)) = (
            directories::ProjectDirs::from("", "", "kelma"),
            directories::ProjectDirs::from("", "", "ironteams"),
        ) else {
            return;
        };
        let dirs = [
            ("config", old.config_dir(), new.config_dir()),
            ("data", old.data_dir(), new.data_dir()),
            ("cache", old.cache_dir(), new.cache_dir()),
        ];
        for (what, old, new) in dirs {
            match migrate_dir(old, new) {
                Ok(Migrated::No) => {}
                Ok(how) => tracing::info!(
                    "moved the {what} directory {} to {} ({how:?})",
                    old.display(),
                    new.display()
                ),
                Err(e) => tracing::warn!(
                    "could not move the {what} directory {} to {}: {e}",
                    old.display(),
                    new.display()
                ),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_new_variable_wins_and_the_old_one_is_the_fallback() {
        // Names no other test or program reads, so setting them cannot race.
        let (new, old) = ("IRONTEAMS_COMPAT_T1", "KELMA_COMPAT_T1");
        // SAFETY: only this test touches these two variables.
        unsafe {
            std::env::remove_var(new);
            std::env::remove_var(old);
        }
        assert_eq!(var(new), Err(std::env::VarError::NotPresent));
        assert_eq!(var_os(new), None);
        // SAFETY: as above.
        unsafe { std::env::set_var(old, "old") };
        assert_eq!(var(new).as_deref(), Ok("old"));
        assert_eq!(var_os(new), Some("old".into()));
        // SAFETY: as above.
        unsafe { std::env::set_var(new, "new") };
        assert_eq!(var(new).as_deref(), Ok("new"));
        assert_eq!(var_os(new), Some("new".into()));
        // SAFETY: as above.
        unsafe {
            std::env::remove_var(new);
            std::env::remove_var(old);
        }
    }

    #[test]
    fn a_name_without_the_prefix_has_no_fallback() {
        assert_eq!(legacy_name("HOME"), None);
        assert_eq!(legacy_name("IRONTEAMS_LOG").as_deref(), Some("KELMA_LOG"));
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn an_old_dir_is_renamed_with_everything_in_it() {
        let tmp = tempfile::tempdir().unwrap();
        let (old, new) = (tmp.path().join("kelma"), tmp.path().join("x/ironteams"));
        write(&old.join("config.json"), "{}");
        write(&old.join("sub/cache.sqlite"), "db");
        assert_eq!(migrate_dir(&old, &new).unwrap(), Migrated::Renamed);
        assert!(!old.exists());
        assert_eq!(
            std::fs::read_to_string(new.join("config.json")).unwrap(),
            "{}"
        );
        assert_eq!(
            std::fs::read_to_string(new.join("sub/cache.sqlite")).unwrap(),
            "db"
        );
    }

    #[test]
    fn nothing_moves_when_the_new_dir_exists_or_the_old_one_does_not() {
        let tmp = tempfile::tempdir().unwrap();
        let (old, new) = (tmp.path().join("kelma"), tmp.path().join("ironteams"));
        assert_eq!(migrate_dir(&old, &new).unwrap(), Migrated::No);
        assert!(!new.exists());
        write(&old.join("a"), "old");
        write(&new.join("a"), "new");
        assert_eq!(migrate_dir(&old, &new).unwrap(), Migrated::No);
        assert_eq!(std::fs::read_to_string(old.join("a")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(new.join("a")).unwrap(), "new");
    }

    #[test]
    fn the_copy_fallback_moves_a_whole_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let (old, new) = (tmp.path().join("kelma"), tmp.path().join("ironteams"));
        write(&old.join("a/b/c.txt"), "deep");
        write(&old.join("top.txt"), "top");
        copy_tree(&old, &new).unwrap();
        assert_eq!(
            std::fs::read_to_string(new.join("a/b/c.txt")).unwrap(),
            "deep"
        );
        assert_eq!(std::fs::read_to_string(new.join("top.txt")).unwrap(), "top");
        // The original is still there: copy_tree alone never removes it.
        assert!(old.join("top.txt").exists());
    }
}
