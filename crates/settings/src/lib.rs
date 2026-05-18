#![forbid(unsafe_code)]

use std::sync::{Mutex, MutexGuard};

pub use xtunes_domain::{ThemeMode, UserSettings};

pub type SettingsResult<T> = Result<T, SettingsError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettingsError {
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

#[cfg(test)]
mod tests {
    use super::{InMemorySettingsStore, SettingsStore, ThemeMode, UserSettings};

    #[test]
    fn in_memory_settings_store_defaults_to_system_theme() {
        let store = InMemorySettingsStore::default();

        assert_eq!(
            store.load_settings(),
            Ok(UserSettings {
                theme_mode: ThemeMode::System
            })
        );
    }

    #[test]
    fn in_memory_settings_store_saves_settings() {
        let store = InMemorySettingsStore::default();

        assert_eq!(
            store.save_settings(UserSettings {
                theme_mode: ThemeMode::Dark
            }),
            Ok(())
        );

        assert_eq!(
            store.load_settings(),
            Ok(UserSettings {
                theme_mode: ThemeMode::Dark
            })
        );
    }
}
