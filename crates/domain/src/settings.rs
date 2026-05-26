// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::path::{Path, PathBuf};

use crate::{PlaylistItem, VolumePercent};

/// Volume picked the first time the app runs, before any persisted value
/// exists. 80% matches the previous UI-side constant and is loud enough to
/// be obviously audible without startling anyone with sensitive headphones.
pub const DEFAULT_PLAYBACK_VOLUME_PERCENT: u8 = 80;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LibrarySettings {
    pub path: Option<PathBuf>,
    pub management_mode: LibraryManagementMode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LibraryManagementMode {
    #[default]
    ReferenceFilesInPlace,
    CopyAddedFilesIntoLibrary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaybackSettings {
    pub volume: VolumePercent,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            volume: VolumePercent::from_clamped(DEFAULT_PLAYBACK_VOLUME_PERCENT),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UiSettings {
    pub search_text: String,
    pub view_mode: UiViewMode,
    pub playlist_selection: Option<PlaylistItem>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UiViewMode {
    #[default]
    Songs,
    Albums,
    Playlists,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UserSettings {
    pub library: LibrarySettings,
    pub playback: PlaybackSettings,
    pub ui: UiSettings,
}

impl UserSettings {
    pub fn with_library_path(library_path: Option<PathBuf>) -> Self {
        Self {
            library: LibrarySettings {
                path: library_path,
                management_mode: LibraryManagementMode::ReferenceFilesInPlace,
            },
            playback: PlaybackSettings::default(),
            ui: UiSettings::default(),
        }
    }

    pub fn library_path(&self) -> Option<&Path> {
        self.library.path.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{LibraryManagementMode, UserSettings};

    #[test]
    fn library_path_is_unset_by_default() {
        assert_eq!(UserSettings::default().library.path, None);
        assert_eq!(
            UserSettings::default().library.management_mode,
            LibraryManagementMode::ReferenceFilesInPlace
        );
    }

    #[test]
    fn settings_can_hold_a_library_path() {
        let settings = UserSettings::with_library_path(Some(PathBuf::from("/music")));

        assert_eq!(settings.library.path, Some(PathBuf::from("/music")));
        assert_eq!(
            settings.library.management_mode,
            LibraryManagementMode::ReferenceFilesInPlace
        );
    }
}
