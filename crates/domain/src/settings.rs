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
    /// Persisted shuffle preference. Restored at startup into the
    /// runtime's initial `PlaybackQueue::options()` so a user who
    /// closed the app with shuffle on reopens with shuffle on.
    pub shuffle_enabled: bool,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            volume: VolumePercent::from_clamped(DEFAULT_PLAYBACK_VOLUME_PERCENT),
            shuffle_enabled: false,
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

/// Background-capability toggles for local audio analysis. Each flag enables
/// a paced background worker that fills the matching value on tracks that
/// are missing it. Flags never gate manual right-click runs — those are
/// always available and intentionally overwrite existing values.
///
/// The `waveform` flag covers beatgrid plus the preview and detail
/// waveforms with color — they share a single DSP pass.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AnalysisSettings {
    pub bpm: bool,
    pub key: bool,
    pub waveform: bool,
}

/// Background-capability toggles for network-bound retrieval. Same
/// missing-only, paced-background semantics as [`AnalysisSettings`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OnlineSettings {
    pub artwork: bool,
    pub tags: bool,
    pub lyrics: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UserSettings {
    pub library: LibrarySettings,
    pub playback: PlaybackSettings,
    pub ui: UiSettings,
    pub analysis: AnalysisSettings,
    pub online: OnlineSettings,
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
            analysis: AnalysisSettings::default(),
            online: OnlineSettings::default(),
        }
    }

    pub fn library_path(&self) -> Option<&Path> {
        self.library.path.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{AnalysisSettings, LibraryManagementMode, OnlineSettings, UserSettings};

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

    #[test]
    fn background_capability_toggles_are_off_by_default() {
        let settings = UserSettings::default();

        assert_eq!(settings.analysis, AnalysisSettings::default());
        assert_eq!(settings.online, OnlineSettings::default());
        assert!(!settings.analysis.bpm);
        assert!(!settings.analysis.key);
        assert!(!settings.analysis.waveform);
        assert!(!settings.online.artwork);
        assert!(!settings.online.tags);
        assert!(!settings.online.lyrics);
    }
}
