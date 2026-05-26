// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

#![forbid(unsafe_code)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use directories::BaseDirs;
use serde::{Deserialize, Serialize};

pub use sustain_domain::{
    DEFAULT_PLAYBACK_VOLUME_PERCENT, LibraryManagementMode, LibrarySettings, PlaybackSettings,
    PlaylistFolderId, PlaylistId, PlaylistItem, SmartPlaylistId, UiSettings, UiViewMode,
    UserSettings, VolumePercent,
};

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
            base_dirs.config_dir().join("sustain").join("settings.toml"),
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
    #[serde(default)]
    library: LibrarySettingsDocument,
    #[serde(default)]
    playback: PlaybackSettingsDocument,
    #[serde(default)]
    ui: UiSettingsDocument,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct LibrarySettingsDocument {
    path: Option<PathBuf>,
    #[serde(default)]
    management_mode: LibraryManagementModeDocument,
}

#[derive(Debug, Deserialize, Serialize)]
struct PlaybackSettingsDocument {
    /// Percent (0..=100). Defaults to [`DEFAULT_PLAYBACK_VOLUME_PERCENT`]
    /// when absent from disk, and is clamped on read so a hand-edited TOML
    /// with an out-of-range value can never crash the app at startup.
    #[serde(default = "default_volume_percent")]
    volume_percent: u8,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct UiSettingsDocument {
    #[serde(default)]
    search_text: String,
    #[serde(default)]
    view_mode: UiViewModeDocument,
    #[serde(default)]
    playlist_selection: Option<PlaylistSelectionDocument>,
}

impl Default for PlaybackSettingsDocument {
    fn default() -> Self {
        Self {
            volume_percent: DEFAULT_PLAYBACK_VOLUME_PERCENT,
        }
    }
}

fn default_volume_percent() -> u8 {
    DEFAULT_PLAYBACK_VOLUME_PERCENT
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum LibraryManagementModeDocument {
    #[default]
    ReferenceFilesInPlace,
    CopyAddedFilesIntoLibrary,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum UiViewModeDocument {
    #[default]
    Songs,
    Albums,
    Playlists,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
enum PlaylistSelectionDocument {
    Playlist(i64),
    SmartPlaylist(i64),
    Folder(i64),
}

impl SettingsDocument {
    fn from_settings(settings: UserSettings) -> Self {
        Self {
            library: LibrarySettingsDocument {
                path: settings.library.path,
                management_mode: LibraryManagementModeDocument::from_domain(
                    settings.library.management_mode,
                ),
            },
            playback: PlaybackSettingsDocument {
                volume_percent: settings.playback.volume.get(),
            },
            ui: UiSettingsDocument {
                search_text: settings.ui.search_text,
                view_mode: UiViewModeDocument::from_domain(settings.ui.view_mode),
                playlist_selection: settings
                    .ui
                    .playlist_selection
                    .map(PlaylistSelectionDocument::from_domain),
            },
        }
    }

    fn into_settings(self) -> UserSettings {
        UserSettings {
            library: LibrarySettings {
                path: self.library.path,
                management_mode: self.library.management_mode.into_domain(),
            },
            playback: PlaybackSettings {
                volume: VolumePercent::from_clamped(self.playback.volume_percent),
            },
            ui: UiSettings {
                search_text: self.ui.search_text,
                view_mode: self.ui.view_mode.into_domain(),
                playlist_selection: self
                    .ui
                    .playlist_selection
                    .and_then(PlaylistSelectionDocument::into_domain),
            },
        }
    }
}

impl LibraryManagementModeDocument {
    fn from_domain(mode: LibraryManagementMode) -> Self {
        match mode {
            LibraryManagementMode::ReferenceFilesInPlace => Self::ReferenceFilesInPlace,
            LibraryManagementMode::CopyAddedFilesIntoLibrary => Self::CopyAddedFilesIntoLibrary,
        }
    }

    fn into_domain(self) -> LibraryManagementMode {
        match self {
            Self::ReferenceFilesInPlace => LibraryManagementMode::ReferenceFilesInPlace,
            Self::CopyAddedFilesIntoLibrary => LibraryManagementMode::CopyAddedFilesIntoLibrary,
        }
    }
}

impl UiViewModeDocument {
    fn from_domain(mode: UiViewMode) -> Self {
        match mode {
            UiViewMode::Songs => Self::Songs,
            UiViewMode::Albums => Self::Albums,
            UiViewMode::Playlists => Self::Playlists,
        }
    }

    fn into_domain(self) -> UiViewMode {
        match self {
            Self::Songs => UiViewMode::Songs,
            Self::Albums => UiViewMode::Albums,
            Self::Playlists => UiViewMode::Playlists,
        }
    }
}

impl PlaylistSelectionDocument {
    fn from_domain(selection: PlaylistItem) -> Self {
        match selection {
            PlaylistItem::Playlist(id) => Self::Playlist(id.get()),
            PlaylistItem::SmartPlaylist(id) => Self::SmartPlaylist(id.get()),
            PlaylistItem::Folder(id) => Self::Folder(id.get()),
        }
    }

    fn into_domain(self) -> Option<PlaylistItem> {
        match self {
            Self::Playlist(id) => PlaylistId::new(id).map(PlaylistItem::Playlist),
            Self::SmartPlaylist(id) => SmartPlaylistId::new(id).map(PlaylistItem::SmartPlaylist),
            Self::Folder(id) => PlaylistFolderId::new(id).map(PlaylistItem::Folder),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::{
        DEFAULT_PLAYBACK_VOLUME_PERCENT, InMemorySettingsStore, LibraryManagementMode, PlaylistId,
        PlaylistItem, SettingsStore, TomlSettingsStore, UiSettings, UiViewMode, UserSettings,
        VolumePercent,
    };

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
        let mut settings = UserSettings::with_library_path(Some(PathBuf::from("/music")));
        settings.library.management_mode = LibraryManagementMode::CopyAddedFilesIntoLibrary;

        assert_eq!(store.save_settings(settings.clone()), Ok(()));
        assert_eq!(store.load_settings(), Ok(settings));

        let root = path
            .parent()
            .and_then(|parent| parent.parent())
            .expect("test path has two parents");
        fs::remove_dir_all(root).expect("remove test settings directory");
    }

    #[test]
    fn toml_settings_store_round_trips_playback_volume() {
        let path = unique_settings_path();
        let store = TomlSettingsStore::new(&path);
        let mut settings = UserSettings::default();
        settings.playback.volume = VolumePercent::from_clamped(37);

        assert_eq!(store.save_settings(settings.clone()), Ok(()));
        assert_eq!(store.load_settings(), Ok(settings));

        let root = path
            .parent()
            .and_then(|parent| parent.parent())
            .expect("test path has two parents");
        fs::remove_dir_all(root).expect("remove test settings directory");
    }

    #[test]
    fn toml_settings_store_round_trips_ui_state() {
        let path = unique_settings_path();
        let store = TomlSettingsStore::new(&path);
        let settings = UserSettings {
            ui: UiSettings {
                search_text: "radiohead".to_owned(),
                view_mode: UiViewMode::Playlists,
                playlist_selection: Some(PlaylistItem::Playlist(
                    PlaylistId::new(7).expect("positive playlist id"),
                )),
            },
            ..UserSettings::default()
        };

        assert_eq!(store.save_settings(settings.clone()), Ok(()));
        assert_eq!(store.load_settings(), Ok(settings));

        let root = path
            .parent()
            .and_then(|parent| parent.parent())
            .expect("test path has two parents");
        fs::remove_dir_all(root).expect("remove test settings directory");
    }

    #[test]
    fn toml_settings_store_defaults_volume_when_section_is_missing() {
        let path = unique_settings_path();
        let store = TomlSettingsStore::new(&path);
        fs::create_dir_all(path.parent().expect("settings path has parent"))
            .expect("create settings dir");
        fs::write(&path, "[library]\npath = \"/music\"\n").expect("write settings");

        let settings = store.load_settings().expect("settings load");

        assert_eq!(
            settings.playback.volume,
            VolumePercent::from_clamped(DEFAULT_PLAYBACK_VOLUME_PERCENT)
        );

        let root = path
            .parent()
            .and_then(|parent| parent.parent())
            .expect("test path has two parents");
        fs::remove_dir_all(root).expect("remove test settings directory");
    }

    #[test]
    fn toml_settings_store_clamps_out_of_range_volume_on_load() {
        let path = unique_settings_path();
        let store = TomlSettingsStore::new(&path);
        fs::create_dir_all(path.parent().expect("settings path has parent"))
            .expect("create settings dir");
        fs::write(&path, "[playback]\nvolume_percent = 250\n").expect("write settings");

        let settings = store.load_settings().expect("settings load");

        assert_eq!(settings.playback.volume, VolumePercent::from_clamped(100));

        let root = path
            .parent()
            .and_then(|parent| parent.parent())
            .expect("test path has two parents");
        fs::remove_dir_all(root).expect("remove test settings directory");
    }

    #[test]
    fn toml_settings_store_defaults_management_mode_when_missing() {
        let path = unique_settings_path();
        let store = TomlSettingsStore::new(&path);
        fs::create_dir_all(path.parent().expect("settings path has parent"))
            .expect("create settings dir");
        fs::write(&path, "[library]\npath = \"/music\"\n").expect("write settings");

        let settings = store.load_settings().expect("settings load");

        assert_eq!(
            settings.library.management_mode,
            LibraryManagementMode::ReferenceFilesInPlace
        );

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
            .join(format!("sustain_settings_test_{unique_suffix}"))
            .join("sustain")
            .join("settings.toml")
    }
}
