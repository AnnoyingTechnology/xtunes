#![forbid(unsafe_code)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use directories::BaseDirs;
use serde::{Deserialize, Serialize};

pub use xtunes_domain::{LibrarySettings, UserSettings};

pub type SettingsResult<T> = Result<T, SettingsError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettingsError {
    ConfigDirectoryUnavailable,
    LoadFailed,
    SaveFailed,
    StoreUnavailable,
}

pub trait SettingsStore {
    fn load_settings(&self) -> SettingsResult<UserSettings>;
    fn save_settings(&self, settings: UserSettings) -> SettingsResult<()>;
}

#[derive(Debug)]
pub struct InMemorySettingsStore {
    settings: Mutex<UserSettings>,
}

impl InMemorySettingsStore {
    pub fn new(settings: UserSettings) -> Self {
        Self {
            settings: Mutex::new(settings),
        }
    }

    fn settings_guard(&self) -> SettingsResult<MutexGuard<'_, UserSettings>> {
        self.settings
            .lock()
            .map_err(|_| SettingsError::StoreUnavailable)
    }
}

impl Default for InMemorySettingsStore {
    fn default() -> Self {
        Self::new(UserSettings::default())
    }
}

impl SettingsStore for InMemorySettingsStore {
    fn load_settings(&self) -> SettingsResult<UserSettings> {
        Ok(self.settings_guard()?.clone())
    }

    fn save_settings(&self, settings: UserSettings) -> SettingsResult<()> {
        *self.settings_guard()? = settings;
        Ok(())
    }
}

#[derive(Debug)]
pub struct TomlSettingsStore {
    path: PathBuf,
}

impl TomlSettingsStore {
    pub fn open_default() -> SettingsResult<Self> {
        let base_dirs = BaseDirs::new().ok_or(SettingsError::ConfigDirectoryUnavailable)?;
        Ok(Self::new(
            base_dirs.config_dir().join("xtunes").join("settings.toml"),
        ))
    }

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl SettingsStore for TomlSettingsStore {
    fn load_settings(&self) -> SettingsResult<UserSettings> {
        if !self.path.exists() {
            return Ok(UserSettings::default());
        }

        let document = fs::read_to_string(&self.path).map_err(|_| SettingsError::LoadFailed)?;
        toml::from_str::<SettingsDocument>(&document)
            .map(SettingsDocument::into_settings)
            .map_err(|_| SettingsError::LoadFailed)
    }

    fn save_settings(&self, settings: UserSettings) -> SettingsResult<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|_| SettingsError::SaveFailed)?;
        }

        let document = SettingsDocument::from_settings(settings);
        let serialized =
            toml::to_string_pretty(&document).map_err(|_| SettingsError::SaveFailed)?;
        fs::write(&self.path, serialized).map_err(|_| SettingsError::SaveFailed)
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct SettingsDocument {
    library: LibrarySettingsDocument,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct LibrarySettingsDocument {
    path: Option<PathBuf>,
}

impl SettingsDocument {
    fn from_settings(settings: UserSettings) -> Self {
        Self {
            library: LibrarySettingsDocument {
                path: settings.library.path,
            },
        }
    }

    fn into_settings(self) -> UserSettings {
        UserSettings {
            library: LibrarySettings {
                path: self.library.path,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::{InMemorySettingsStore, SettingsStore, TomlSettingsStore, UserSettings};

    #[test]
    fn in_memory_settings_store_defaults_to_no_library_path() {
        let store = InMemorySettingsStore::default();

        assert_eq!(store.load_settings(), Ok(UserSettings::default()));
    }

    #[test]
    fn in_memory_settings_store_saves_settings() {
        let store = InMemorySettingsStore::default();
        let settings = UserSettings::with_library_path(Some(PathBuf::from("/music")));

        assert_eq!(store.save_settings(settings.clone()), Ok(()));

        assert_eq!(store.load_settings(), Ok(settings));
    }

    #[test]
    fn toml_settings_store_defaults_when_file_is_missing() {
        let path = unique_settings_path();
        let store = TomlSettingsStore::new(&path);

        assert_eq!(store.load_settings(), Ok(UserSettings::default()));
    }

    #[test]
    fn toml_settings_store_saves_and_loads_library_path() {
        let path = unique_settings_path();
        let store = TomlSettingsStore::new(&path);
        let settings = UserSettings::with_library_path(Some(PathBuf::from("/music")));

        assert_eq!(store.save_settings(settings.clone()), Ok(()));
        assert_eq!(store.load_settings(), Ok(settings));

        let root = path
            .parent()
            .and_then(|parent| parent.parent())
            .expect("test path has two parents");
        fs::remove_dir_all(root).expect("remove test settings directory");
    }

    fn unique_settings_path() -> PathBuf {
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("xtunes_settings_test_{unique_suffix}"))
            .join("xtunes")
            .join("settings.toml")
    }
}
