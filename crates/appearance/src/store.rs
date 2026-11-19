//! Where preferences live and how they are read and written.

use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Same app name `session` uses for `directories::ProjectDirs`.
const APP: &str = "kelma";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no home directory to keep preferences in")]
    NoHome,
    #[error("could not write preferences: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not encode preferences: {0}")]
    Encode(#[from] serde_json::Error),
}

/// The app config directory (`~/.config/kelma`). `KELMA_HOME` overrides it, exactly as
/// `session::Paths` does, so tests and the other crates agree on the location.
pub fn config_dir() -> Result<PathBuf, Error> {
    if let Ok(home) = std::env::var("KELMA_HOME") {
        return Ok(PathBuf::from(home).join("config"));
    }
    let d = directories::ProjectDirs::from("", "", APP).ok_or(Error::NoHome)?;
    Ok(d.config_dir().to_path_buf())
}

/// Reads a JSON preferences file. A missing or corrupt file gives the defaults.
pub(crate) fn load_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Writes JSON atomically: a temp file in the same directory, flushed, then renamed over the target.
pub(crate) fn save_json<T: Serialize>(path: &Path, value: &T) -> Result<(), Error> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("prefs");
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let mut body = serde_json::to_vec_pretty(value)?;
    body.push(b'\n');
    let write = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&body)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if write.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    Ok(write?)
}

/// A field that falls back to its default when the stored value has the wrong shape, so one
/// bad field (a value from a newer or older version) does not throw away the rest of the file.
pub(crate) fn lenient<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(T::deserialize(v).unwrap_or_default())
}

/// Adds `path`, `load`, `load_from`, `save` and `save_to` to a preferences type that is
/// `Default + Serialize + DeserializeOwned`, kept in `$file` in the config dir.
macro_rules! prefs_file {
    ($ty:ty, $file:expr) => {
        impl $ty {
            /// Where the file lives (in the app config dir).
            pub fn path() -> Result<std::path::PathBuf, $crate::store::Error> {
                Ok($crate::store::config_dir()?.join($file))
            }

            /// Loads the preferences; a missing or corrupt file, or no home, gives the defaults.
            pub fn load() -> Self {
                Self::path()
                    .map(|p| Self::load_from(&p))
                    .unwrap_or_default()
            }

            pub fn load_from(path: &std::path::Path) -> Self {
                $crate::store::load_json(path)
            }

            /// Saves atomically (temp file, then rename).
            pub fn save(&self) -> Result<(), $crate::store::Error> {
                self.save_to(&Self::path()?)
            }

            pub fn save_to(&self, path: &std::path::Path) -> Result<(), $crate::store::Error> {
                $crate::store::save_json(path, self)
            }
        }
    };
}
pub(crate) use prefs_file;
