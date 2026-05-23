// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

#![forbid(unsafe_code)]

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub use xtunes_domain::{
    ApplicationCommand, ApplicationQuery, FieldChange, LibrarySettings, MetadataChange,
    PlayStatistics, PlaybackCommand, PlaybackOptions, PlaybackQueue, PlaybackQueueSource,
    PlaybackState, Playlist, PlaylistEntry, PlaylistFolder, PlaylistFolderId, PlaylistId,
    PlaylistItem, Rating, RepeatMode, SmartPlaylist, SmartPlaylistDateField, SmartPlaylistId,
    SmartPlaylistLimit, SmartPlaylistLimitSelection, SmartPlaylistMatchKind,
    SmartPlaylistNumberField, SmartPlaylistNumberOperator, SmartPlaylistRule, SmartPlaylistRuleSet,
    SmartPlaylistTextField, SmartPlaylistTextOperator, Track, TrackAvailability, TrackId,
    TrackLocation, TrackMetadata, TrackPlaybackSource, TrackRelativePath, UserSettings,
    VolumePercent, matching_tracks,
};
use xtunes_library_store::LibraryStore;
use xtunes_metadata::MetadataService;
use xtunes_playback::PlaybackService;
pub use xtunes_playback::TrackEndedCallback;
use xtunes_settings::SettingsStore;

mod commands;
mod library_mutation;
mod library_scan;
mod playback;
mod playlist_folders;
mod playlist_items;
mod playlists;
mod smart_playlists;

pub use library_scan::run_library_scan_task;

pub type ApplicationRuntimeResult<T> = Result<T, ApplicationRuntimeError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationRuntimeError {
    LibraryScanFailed,
    LibraryServicesUnavailable,
    LibraryStoreFailed,
    MetadataWriteFailed,
    InvalidPlaylistName,
    InvalidPlaylistFolderName,
    InvalidSmartPlaylistName,
    InvalidSmartPlaylistRules,
    BackgroundTaskRunning,
    PlaybackFailed,
    PlaybackServiceUnavailable,
    PlaylistEntryNotFound,
    PlaylistNotFound,
    PlaylistFolderNotFound,
    PlaylistFolderWouldCycle,
    SmartPlaylistNotFound,
    SettingsLoadFailed,
    SettingsSaveFailed,
    TrackUnavailable,
    TrackTrashFailed,
    UnsupportedCommand(ApplicationCommand),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LibraryScanSummary {
    pub scanned_tracks: usize,
    pub added_tracks: usize,
    pub updated_tracks: usize,
    pub missing_tracks: usize,
    pub skipped_unsupported_files: usize,
    pub failed_files: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum BackgroundTaskStatus {
    #[default]
    Idle,
    LibraryScanRunning,
    LibraryScanCompleted(LibraryScanSummary),
    LibraryScanFailed(ApplicationRuntimeError),
}

impl BackgroundTaskStatus {
    pub fn is_running(&self) -> bool {
        matches!(self, Self::LibraryScanRunning)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NowPlaying {
    pub track: Option<Track>,
    pub state: PlaybackState,
    pub options: PlaybackOptions,
}

pub struct LibraryScanTask {
    library_path: PathBuf,
    existing_tracks: Vec<Track>,
    library_store: Arc<dyn LibraryStore>,
    metadata_service: Arc<dyn MetadataService>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LibraryScanResult {
    pub tracks: Vec<Track>,
    pub summary: LibraryScanSummary,
}

pub struct ApplicationRuntime {
    settings: UserSettings,
    settings_store: Option<Box<dyn SettingsStore>>,
    library_store: Option<Arc<dyn LibraryStore>>,
    metadata_service: Option<Arc<dyn MetadataService>>,
    playback_service: Option<Box<dyn PlaybackService>>,
    playback_queue: PlaybackQueue,
    library_tracks: Vec<Track>,
    playlists: Vec<Playlist>,
    playlist_folders: Vec<PlaylistFolder>,
    smart_playlists: Vec<SmartPlaylist>,
    last_scan_library_path: Option<PathBuf>,
    last_scan_summary: Option<LibraryScanSummary>,
    background_task_status: BackgroundTaskStatus,
}

impl ApplicationRuntime {
    pub fn new() -> Self {
        Self {
            settings: UserSettings::default(),
            settings_store: None,
            library_store: None,
            metadata_service: None,
            playback_service: None,
            playback_queue: PlaybackQueue::default(),
            library_tracks: Vec::new(),
            playlists: Vec::new(),
            playlist_folders: Vec::new(),
            smart_playlists: Vec::new(),
            last_scan_library_path: None,
            last_scan_summary: None,
            background_task_status: BackgroundTaskStatus::Idle,
        }
    }

    pub fn with_settings_store(
        settings_store: Box<dyn SettingsStore>,
    ) -> ApplicationRuntimeResult<Self> {
        let settings = settings_store
            .load_settings()
            .map_err(|_| ApplicationRuntimeError::SettingsLoadFailed)?;

        Ok(Self {
            settings,
            settings_store: Some(settings_store),
            library_store: None,
            metadata_service: None,
            playback_service: None,
            playback_queue: PlaybackQueue::default(),
            library_tracks: Vec::new(),
            playlists: Vec::new(),
            playlist_folders: Vec::new(),
            smart_playlists: Vec::new(),
            last_scan_library_path: None,
            last_scan_summary: None,
            background_task_status: BackgroundTaskStatus::Idle,
        })
    }

    pub fn with_library_services(
        mut self,
        library_store: Arc<dyn LibraryStore>,
        metadata_service: Arc<dyn MetadataService>,
    ) -> ApplicationRuntimeResult<Self> {
        self.set_library_services(library_store, metadata_service)?;
        Ok(self)
    }

    pub fn set_library_services(
        &mut self,
        library_store: Arc<dyn LibraryStore>,
        metadata_service: Arc<dyn MetadataService>,
    ) -> ApplicationRuntimeResult<()> {
        self.library_tracks = library_scan::load_library_tracks(
            library_store.as_ref(),
            self.settings.library_path(),
        )?;
        self.playlists = library_store
            .playlists()
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        self.playlist_folders = library_store
            .playlist_folders()
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        self.smart_playlists = library_store
            .smart_playlists()
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        self.library_store = Some(library_store);
        self.metadata_service = Some(metadata_service);
        Ok(())
    }

    pub fn with_playback_service(mut self, playback_service: Box<dyn PlaybackService>) -> Self {
        self.playback_service = Some(playback_service);
        self
    }

    pub fn settings(&self) -> &UserSettings {
        &self.settings
    }

    pub fn library_tracks(&self) -> &[Track] {
        &self.library_tracks
    }

    pub fn playlists(&self) -> &[Playlist] {
        &self.playlists
    }

    pub fn playlist_folders(&self) -> &[PlaylistFolder] {
        &self.playlist_folders
    }

    pub fn smart_playlists(&self) -> &[SmartPlaylist] {
        &self.smart_playlists
    }

    pub fn last_scan_library_path(&self) -> Option<&Path> {
        self.last_scan_library_path.as_deref()
    }

    pub fn last_scan_summary(&self) -> Option<&LibraryScanSummary> {
        self.last_scan_summary.as_ref()
    }

    pub fn background_task_status(&self) -> &BackgroundTaskStatus {
        &self.background_task_status
    }

    pub fn playback_state(&self) -> PlaybackState {
        self.playback_service
            .as_deref()
            .map(PlaybackService::state)
            .unwrap_or_default()
    }

    pub fn set_track_ended_callback(&self, callback: TrackEndedCallback) {
        if let Some(service) = self.playback_service.as_deref() {
            service.set_on_track_ended(callback);
        }
    }

    pub fn playback_options(&self) -> PlaybackOptions {
        self.playback_queue.options()
    }

    pub fn playback_queue_current_track_id(&self) -> Option<TrackId> {
        self.playback_queue.current_track_id()
    }

    pub fn now_playing(&self) -> NowPlaying {
        let state = self.playback_state();
        let track = playback::playback_track_id(&state)
            .and_then(|track_id| {
                self.library_tracks
                    .iter()
                    .find(|track| track.id == track_id)
            })
            .cloned();

        NowPlaying {
            track,
            state,
            options: self.playback_queue.options(),
        }
    }

    pub fn read_artwork(&self, path: &Path) -> Option<Vec<u8>> {
        self.metadata_service
            .as_deref()
            .and_then(|service| service.read_artwork(path).ok().flatten())
    }

    pub fn absolute_track_path(&self, track: &Track) -> Option<PathBuf> {
        self.settings
            .library_path()
            .map(|library_path| track.location.absolute_path(library_path))
    }

    pub fn smart_playlist_matching_tracks(
        &self,
        smart_playlist_id: SmartPlaylistId,
        now: std::time::SystemTime,
    ) -> Vec<&Track> {
        let Some(smart_playlist) = self
            .smart_playlists
            .iter()
            .find(|smart_playlist| smart_playlist.id == smart_playlist_id)
        else {
            return Vec::new();
        };
        matching_tracks(&self.library_tracks, &smart_playlist.rules, now)
    }
}

impl Default for ApplicationRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::{Arc, Mutex, MutexGuard},
    };

    use xtunes_domain::{
        ApplicationCommand, FieldChange, PlayStatistics, PlaybackCommand, PlaybackOptions,
        PlaybackState, Playlist, PlaylistFolderId, PlaylistId, PlaylistItem, Rating, RepeatMode,
        SmartPlaylistId, SmartPlaylistMatchKind, SmartPlaylistRule, SmartPlaylistRuleSet,
        SmartPlaylistTextField, SmartPlaylistTextOperator, Track, TrackId, TrackLocation,
        TrackMetadata, UserSettings, VolumePercent,
    };
    use xtunes_library_store::{InMemoryLibraryStore, LibraryStore, StoreResult};
    use xtunes_metadata::{MetadataChange, MetadataError, MetadataResult};
    use xtunes_playback::NullPlaybackService;
    use xtunes_settings::{SettingsError, SettingsResult, SettingsStore};

    use super::{ApplicationRuntime, ApplicationRuntimeError, LibraryScanSummary, MetadataService};

    #[test]
    fn runtime_starts_with_default_settings() {
        let runtime = ApplicationRuntime::new();

        assert_eq!(runtime.settings().library_path(), None);
    }

    #[test]
    fn runtime_accepts_settings_command() {
        let mut runtime = ApplicationRuntime::new();

        let settings = UserSettings::with_library_path(Some(PathBuf::from("/music")));

        assert_eq!(
            runtime.handle_command(ApplicationCommand::UpdateSettings(settings.clone())),
            Ok(())
        );

        assert_eq!(runtime.settings(), &settings);
    }

    #[test]
    fn runtime_handles_every_application_command_intentionally() {
        let track_id = track_id(1);
        let playlist_id = playlist_id(1);
        let rating = Rating::new(4).expect("valid test rating");
        let metadata_change = MetadataChange::default();
        let settings = UserSettings::with_library_path(Some(PathBuf::from("/music")));

        let cases = vec![
            (
                ApplicationCommand::Playback(PlaybackCommand::PlayTrack(track_id)),
                Err(ApplicationRuntimeError::TrackUnavailable),
            ),
            (
                ApplicationCommand::Playback(PlaybackCommand::PlayPreviousTrack),
                Ok(()),
            ),
            (
                ApplicationCommand::Playback(PlaybackCommand::PlayNextTrack),
                Ok(()),
            ),
            (
                ApplicationCommand::Playback(PlaybackCommand::ToggleShuffle),
                Ok(()),
            ),
            (
                ApplicationCommand::Playback(PlaybackCommand::ToggleRepeat),
                Ok(()),
            ),
            (
                ApplicationCommand::Playback(PlaybackCommand::Pause),
                Err(ApplicationRuntimeError::PlaybackServiceUnavailable),
            ),
            (
                ApplicationCommand::Playback(PlaybackCommand::Resume),
                Err(ApplicationRuntimeError::PlaybackServiceUnavailable),
            ),
            (
                ApplicationCommand::Playback(PlaybackCommand::TogglePlayPause),
                Err(ApplicationRuntimeError::PlaybackServiceUnavailable),
            ),
            (
                ApplicationCommand::Playback(PlaybackCommand::Stop),
                Err(ApplicationRuntimeError::PlaybackServiceUnavailable),
            ),
            (
                ApplicationCommand::Playback(PlaybackCommand::Seek(std::time::Duration::ZERO)),
                Err(ApplicationRuntimeError::PlaybackServiceUnavailable),
            ),
            (
                ApplicationCommand::Playback(PlaybackCommand::SetVolume(
                    VolumePercent::from_clamped(50),
                )),
                Err(ApplicationRuntimeError::PlaybackServiceUnavailable),
            ),
            (
                ApplicationCommand::SetRating { track_id, rating },
                Err(ApplicationRuntimeError::LibraryServicesUnavailable),
            ),
            (
                ApplicationCommand::CreatePlaylist {
                    name: "Favorites".to_owned(),
                    parent_folder_id: None,
                },
                Err(ApplicationRuntimeError::LibraryServicesUnavailable),
            ),
            (
                ApplicationCommand::RenamePlaylist {
                    playlist_id,
                    name: "Renamed".to_owned(),
                },
                Err(ApplicationRuntimeError::LibraryServicesUnavailable),
            ),
            (
                ApplicationCommand::DeletePlaylist { playlist_id },
                Err(ApplicationRuntimeError::LibraryServicesUnavailable),
            ),
            (
                ApplicationCommand::AddTracksToPlaylist {
                    playlist_id,
                    track_ids: vec![track_id],
                },
                Err(ApplicationRuntimeError::TrackUnavailable),
            ),
            (
                ApplicationCommand::RemoveTracksFromPlaylist {
                    playlist_id,
                    track_ids: vec![track_id],
                },
                Err(ApplicationRuntimeError::LibraryServicesUnavailable),
            ),
            (
                ApplicationCommand::MovePlaylistEntry {
                    playlist_id,
                    track_id,
                    new_position: 2,
                },
                Err(ApplicationRuntimeError::LibraryServicesUnavailable),
            ),
            (
                ApplicationCommand::CreatePlaylistFolder {
                    name: "Mixes".to_owned(),
                    parent_folder_id: None,
                },
                Err(ApplicationRuntimeError::LibraryServicesUnavailable),
            ),
            (
                ApplicationCommand::RenamePlaylistFolder {
                    folder_id: folder_id(1),
                    name: "Renamed".to_owned(),
                },
                Err(ApplicationRuntimeError::LibraryServicesUnavailable),
            ),
            (
                ApplicationCommand::DeletePlaylistFolder {
                    folder_id: folder_id(1),
                },
                Err(ApplicationRuntimeError::LibraryServicesUnavailable),
            ),
            (
                ApplicationCommand::CreateSmartPlaylist {
                    name: "Recent".to_owned(),
                    parent_folder_id: None,
                    rules: test_rule_set(),
                },
                Err(ApplicationRuntimeError::LibraryServicesUnavailable),
            ),
            (
                ApplicationCommand::UpdateSmartPlaylist {
                    smart_playlist_id: smart_id(1),
                    name: "Updated".to_owned(),
                    rules: test_rule_set(),
                },
                Err(ApplicationRuntimeError::LibraryServicesUnavailable),
            ),
            (
                ApplicationCommand::DeleteSmartPlaylist {
                    smart_playlist_id: smart_id(1),
                },
                Err(ApplicationRuntimeError::LibraryServicesUnavailable),
            ),
            (
                ApplicationCommand::MovePlaylistItem {
                    item: PlaylistItem::Playlist(playlist_id),
                    target_parent_folder_id: None,
                    position: 0,
                },
                Err(ApplicationRuntimeError::LibraryServicesUnavailable),
            ),
            (
                ApplicationCommand::UpdateMetadata {
                    track_id,
                    change: metadata_change.clone(),
                },
                Err(ApplicationRuntimeError::LibraryServicesUnavailable),
            ),
            (
                ApplicationCommand::RemoveTrackFromLibrary { track_id },
                Err(ApplicationRuntimeError::LibraryServicesUnavailable),
            ),
            (
                ApplicationCommand::MoveTrackToTrash { track_id },
                Err(ApplicationRuntimeError::TrackUnavailable),
            ),
            (ApplicationCommand::UpdateSettings(settings.clone()), Ok(())),
            (
                ApplicationCommand::ScanLibrary {
                    library_path: PathBuf::from("/music"),
                },
                Err(ApplicationRuntimeError::LibraryServicesUnavailable),
            ),
        ];

        for (command, expected_result) in cases {
            let mut runtime = ApplicationRuntime::new();

            assert_eq!(runtime.handle_command(command), expected_result);
        }
    }

    #[test]
    fn runtime_records_manual_scan_request() {
        let mut runtime = ApplicationRuntime::new();
        let library_path = PathBuf::from("/music");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::ScanLibrary {
                library_path: library_path.clone()
            }),
            Err(ApplicationRuntimeError::LibraryServicesUnavailable)
        );

        assert_eq!(
            runtime.last_scan_library_path(),
            Some(library_path.as_path())
        );
    }

    #[test]
    fn runtime_scans_library_with_services() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        let track_path = root.join("track.mp3");
        std::fs::write(&track_path, b"not real audio").expect("write fake track");

        let store = Arc::new(InMemoryLibraryStore::new());
        let metadata_service = Arc::new(TestMetadataService);
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store, metadata_service)
            .expect("library services initialize");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::ScanLibrary {
                library_path: root.clone()
            }),
            Ok(())
        );

        assert_eq!(runtime.library_tracks().len(), 1);
        assert_eq!(
            runtime
                .last_scan_summary()
                .map(|summary| summary.scanned_tracks),
            Some(1)
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn runtime_scan_preserves_existing_track_identity_for_known_location() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        let track_path = root.join("track.mp3");
        std::fs::write(&track_path, b"not real audio").expect("write fake track");

        let track_id = track_id(7);
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut existing_track = test_track(track_id, "track.mp3");
        existing_track.statistics.play_count = 12;
        assert_eq!(store.save_track(existing_track), Ok(()));

        let metadata_service = Arc::new(TestMetadataService);
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store, metadata_service)
            .expect("library services initialize");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::ScanLibrary {
                library_path: root.clone()
            }),
            Ok(())
        );

        let tracks = runtime.library_tracks();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].id, track_id);
        assert_eq!(tracks[0].statistics.play_count, 12);
        assert_eq!(
            runtime.last_scan_summary(),
            Some(&LibraryScanSummary {
                scanned_tracks: 1,
                added_tracks: 0,
                updated_tracks: 1,
                missing_tracks: 0,
                skipped_unsupported_files: 0,
                failed_files: 0,
            })
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn runtime_scan_preserves_existing_track_identity_after_library_root_changes() {
        let root = unique_test_directory();
        let old_root = root.join("old-library");
        let new_root = root.join("new-library");
        let relative_path = "Artist/Album/track.mp3";
        let track_path = new_root.join(relative_path);
        std::fs::create_dir_all(track_path.parent().expect("test path has parent"))
            .expect("create test album directory");
        std::fs::write(&track_path, b"not real audio").expect("write fake track");

        let track_id = track_id(11);
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut existing_track = test_track(track_id, relative_path);
        existing_track.statistics.play_count = 22;
        assert_eq!(store.save_track(existing_track), Ok(()));

        let settings_store = Box::new(TestSettingsStore::new(UserSettings::with_library_path(
            Some(old_root),
        )));
        let mut runtime =
            ApplicationRuntime::with_settings_store(settings_store).expect("load settings");
        runtime = runtime
            .with_library_services(store, Arc::new(TestMetadataService))
            .expect("library services initialize");

        assert!(runtime.library_tracks()[0].location.is_missing());
        assert_eq!(
            runtime.handle_command(ApplicationCommand::UpdateSettings(
                UserSettings::with_library_path(Some(new_root.clone()))
            )),
            Ok(())
        );
        assert_eq!(
            runtime.handle_command(ApplicationCommand::ScanLibrary {
                library_path: new_root.clone()
            }),
            Ok(())
        );

        let tracks = runtime.library_tracks();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].id, track_id);
        assert_eq!(tracks[0].statistics.play_count, 22);
        assert!(!tracks[0].location.is_missing());
        assert_eq!(
            runtime.last_scan_summary(),
            Some(&LibraryScanSummary {
                scanned_tracks: 1,
                added_tracks: 0,
                updated_tracks: 1,
                missing_tracks: 0,
                skipped_unsupported_files: 0,
                failed_files: 0,
            })
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn runtime_scan_keeps_missing_tracks_visible() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");

        let track_id = track_id(9);
        let store = Arc::new(InMemoryLibraryStore::new());
        assert_eq!(
            store.save_track(test_track(track_id, "missing.mp3")),
            Ok(())
        );

        let metadata_service = Arc::new(TestMetadataService);
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store, metadata_service)
            .expect("library services initialize");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::ScanLibrary {
                library_path: root.clone()
            }),
            Ok(())
        );

        let tracks = runtime.library_tracks();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].id, track_id);
        assert!(tracks[0].location.is_missing());
        assert_eq!(
            runtime
                .last_scan_summary()
                .map(|summary| summary.missing_tracks),
            Some(1)
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn runtime_loads_and_saves_with_settings_store() {
        let store = Box::new(TestSettingsStore::new(UserSettings::with_library_path(
            Some(PathBuf::from("/initial")),
        )));
        let mut runtime =
            ApplicationRuntime::with_settings_store(store).expect("load settings from test store");
        let updated_settings = UserSettings::with_library_path(Some(PathBuf::from("/updated")));

        assert_eq!(
            runtime.settings(),
            &UserSettings::with_library_path(Some(PathBuf::from("/initial")))
        );
        assert_eq!(
            runtime.handle_command(ApplicationCommand::UpdateSettings(updated_settings.clone())),
            Ok(())
        );
        assert_eq!(runtime.settings(), &updated_settings);
    }

    #[test]
    fn runtime_plays_tracks_through_playback_service() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        std::fs::write(root.join("track.flac"), b"not real audio").expect("write fake track");

        let track_id = positive_track_id();
        let store = Arc::new(InMemoryLibraryStore::new());
        let track = Track {
            id: track_id,
            location: track_location("track.flac"),
            metadata: TrackMetadata::default(),
            rating: Rating::unrated(),
            statistics: PlayStatistics::default(),
        };
        assert_eq!(store.save_track(track), Ok(()));

        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store, Arc::new(TestMetadataService))
        .expect("library services initialize")
        .with_playback_service(Box::new(NullPlaybackService::new()));

        assert_eq!(
            runtime.handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayTrack(
                track_id
            ))),
            Ok(())
        );
        assert_eq!(
            runtime.playback_state(),
            PlaybackState::Playing {
                track_id,
                position: std::time::Duration::ZERO,
            }
        );
        assert_eq!(
            runtime.now_playing().track.map(|track| track.id),
            Some(track_id)
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn runtime_toggles_shuffle_without_playback_service() {
        let mut runtime = ApplicationRuntime::new();

        assert_eq!(runtime.playback_options(), PlaybackOptions::default());
        assert_eq!(
            runtime.handle_command(ApplicationCommand::Playback(PlaybackCommand::ToggleShuffle)),
            Ok(())
        );

        assert_eq!(
            runtime.playback_options(),
            PlaybackOptions {
                shuffle_enabled: true,
                repeat_mode: RepeatMode::Off,
            }
        );
    }

    #[test]
    fn runtime_toggles_repeat_without_playback_service() {
        let mut runtime = ApplicationRuntime::new();

        assert_eq!(
            runtime.handle_command(ApplicationCommand::Playback(PlaybackCommand::ToggleRepeat)),
            Ok(())
        );

        assert_eq!(
            runtime.playback_options(),
            PlaybackOptions {
                shuffle_enabled: false,
                repeat_mode: RepeatMode::All,
            }
        );
    }

    #[test]
    fn now_playing_reports_playback_options() {
        let mut runtime = ApplicationRuntime::new();

        assert_eq!(
            runtime.handle_command(ApplicationCommand::Playback(PlaybackCommand::ToggleShuffle)),
            Ok(())
        );

        assert_eq!(
            runtime.now_playing().options,
            PlaybackOptions {
                shuffle_enabled: true,
                repeat_mode: RepeatMode::Off,
            }
        );
    }

    #[test]
    fn runtime_play_next_track_skips_missing_tracks() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        std::fs::write(root.join("first.flac"), b"not real audio").expect("write first track");
        std::fs::write(root.join("third.flac"), b"not real audio").expect("write third track");

        let store = Arc::new(InMemoryLibraryStore::new());
        let first_track = test_track(track_id(1), "first.flac");
        let mut missing_track = test_track(track_id(2), "missing.flac");
        missing_track.location = missing_track_location("missing.flac");
        let third_track = test_track(track_id(3), "third.flac");
        assert_eq!(store.save_track(first_track), Ok(()));
        assert_eq!(store.save_track(missing_track), Ok(()));
        assert_eq!(store.save_track(third_track), Ok(()));

        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store, Arc::new(TestMetadataService))
        .expect("library services initialize")
        .with_playback_service(Box::new(NullPlaybackService::new()));

        assert_eq!(
            runtime.handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayTrack(
                track_id(1)
            ))),
            Ok(())
        );
        assert_eq!(
            runtime.handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayNextTrack)),
            Ok(())
        );

        assert_eq!(
            runtime.playback_state(),
            PlaybackState::Playing {
                track_id: track_id(3),
                position: std::time::Duration::ZERO,
            }
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn runtime_play_previous_track_skips_missing_tracks() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        std::fs::write(root.join("first.flac"), b"not real audio").expect("write first track");
        std::fs::write(root.join("third.flac"), b"not real audio").expect("write third track");

        let store = Arc::new(InMemoryLibraryStore::new());
        let first_track = test_track(track_id(1), "first.flac");
        let mut missing_track = test_track(track_id(2), "missing.flac");
        missing_track.location = missing_track_location("missing.flac");
        let third_track = test_track(track_id(3), "third.flac");
        assert_eq!(store.save_track(first_track), Ok(()));
        assert_eq!(store.save_track(missing_track), Ok(()));
        assert_eq!(store.save_track(third_track), Ok(()));

        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store, Arc::new(TestMetadataService))
        .expect("library services initialize")
        .with_playback_service(Box::new(NullPlaybackService::new()));

        assert_eq!(
            runtime.handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayTrack(
                track_id(3)
            ))),
            Ok(())
        );
        assert_eq!(
            runtime.handle_command(ApplicationCommand::Playback(
                PlaybackCommand::PlayPreviousTrack
            )),
            Ok(())
        );

        assert_eq!(
            runtime.playback_state(),
            PlaybackState::Playing {
                track_id: track_id(1),
                position: std::time::Duration::ZERO,
            }
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn runtime_set_rating_writes_metadata_and_updates_store_cache() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        let track_path = root.join("track.flac");
        std::fs::write(&track_path, b"not real audio").expect("write fake track");

        let track_id = track_id(1);
        let store = Arc::new(InMemoryLibraryStore::new());
        assert_eq!(store.save_track(test_track(track_id, "track.flac")), Ok(()));
        let metadata_service = Arc::new(RecordingMetadataService::new(false));
        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store.clone(), metadata_service.clone())
        .expect("library services initialize");
        let rating = Rating::new(5).expect("valid test rating");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::SetRating { track_id, rating }),
            Ok(())
        );

        assert_eq!(
            metadata_service.rating_writes(),
            vec![(track_path.clone(), rating)]
        );
        assert_eq!(runtime.library_tracks()[0].rating, rating);
        assert_eq!(
            store
                .track(track_id)
                .expect("load updated track")
                .map(|track| track.rating),
            Some(rating)
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn runtime_set_rating_does_not_update_store_cache_when_metadata_write_fails() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        let track_path = root.join("track.flac");
        std::fs::write(&track_path, b"not real audio").expect("write fake track");

        let track_id = track_id(1);
        let store = Arc::new(InMemoryLibraryStore::new());
        assert_eq!(store.save_track(test_track(track_id, "track.flac")), Ok(()));
        let metadata_service = Arc::new(RecordingMetadataService::new(true));
        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store.clone(), metadata_service.clone())
        .expect("library services initialize");
        let rating = Rating::new(4).expect("valid test rating");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::SetRating { track_id, rating }),
            Err(ApplicationRuntimeError::MetadataWriteFailed)
        );

        assert_eq!(
            metadata_service.rating_writes(),
            vec![(track_path.clone(), rating)]
        );
        assert_eq!(runtime.library_tracks()[0].rating, Rating::unrated());
        assert_eq!(
            store
                .track(track_id)
                .expect("load unchanged track")
                .map(|track| track.rating),
            Some(Rating::unrated())
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn runtime_update_metadata_writes_tags_and_updates_store_cache() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        let track_path = root.join("track.flac");
        std::fs::write(&track_path, b"not real audio").expect("write fake track");

        let track_id = track_id(1);
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut track = test_track(track_id, "track.flac");
        track.metadata.title = Some("Old".to_owned());
        track.metadata.artist = Some("Artist".to_owned());
        assert_eq!(store.save_track(track), Ok(()));
        let metadata_service = Arc::new(RecordingMetadataService::new(false));
        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store.clone(), metadata_service.clone())
        .expect("library services initialize");
        let change = MetadataChange {
            title: FieldChange::Set("New".to_owned()),
            artist: FieldChange::Clear,
            year: FieldChange::Set(2001),
            ..MetadataChange::default()
        };

        assert_eq!(
            runtime.handle_command(ApplicationCommand::UpdateMetadata {
                track_id,
                change: change.clone(),
            }),
            Ok(())
        );

        assert_eq!(
            metadata_service.metadata_writes(),
            vec![(track_path.clone(), change)]
        );
        assert_eq!(
            runtime.library_tracks()[0].metadata.title.as_deref(),
            Some("New")
        );
        assert_eq!(runtime.library_tracks()[0].metadata.artist, None);
        assert_eq!(runtime.library_tracks()[0].metadata.year, Some(2001));
        let stored = store
            .track(track_id)
            .expect("load updated track")
            .expect("track exists");
        assert_eq!(stored.metadata.title.as_deref(), Some("New"));
        assert_eq!(stored.metadata.artist, None);
        assert_eq!(stored.metadata.year, Some(2001));

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn runtime_update_metadata_does_not_update_store_cache_when_tag_write_fails() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        let track_path = root.join("track.flac");
        std::fs::write(&track_path, b"not real audio").expect("write fake track");

        let track_id = track_id(1);
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut track = test_track(track_id, "track.flac");
        track.metadata.title = Some("Old".to_owned());
        assert_eq!(store.save_track(track), Ok(()));
        let metadata_service = Arc::new(RecordingMetadataService::with_metadata_write_failure());
        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store.clone(), metadata_service.clone())
        .expect("library services initialize");
        let change = MetadataChange {
            title: FieldChange::Set("New".to_owned()),
            ..MetadataChange::default()
        };

        assert_eq!(
            runtime.handle_command(ApplicationCommand::UpdateMetadata {
                track_id,
                change: change.clone(),
            }),
            Err(ApplicationRuntimeError::MetadataWriteFailed)
        );

        assert_eq!(
            metadata_service.metadata_writes(),
            vec![(track_path.clone(), change)]
        );
        assert_eq!(
            runtime.library_tracks()[0].metadata.title.as_deref(),
            Some("Old")
        );
        assert_eq!(
            store
                .track(track_id)
                .expect("load unchanged track")
                .and_then(|track| track.metadata.title),
            Some("Old".to_owned())
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn runtime_removes_tracks_from_library_and_stops_playback() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        std::fs::write(root.join("track.flac"), b"not real audio").expect("write fake track");

        let removed_id = track_id(1);
        let store = Arc::new(InMemoryLibraryStore::new());
        assert_eq!(
            store.save_track(test_track(removed_id, "track.flac")),
            Ok(())
        );

        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store.clone(), Arc::new(TestMetadataService))
        .expect("library services initialize")
        .with_playback_service(Box::new(NullPlaybackService::new()));

        assert_eq!(
            runtime.handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayTrack(
                removed_id,
            ))),
            Ok(())
        );
        assert_eq!(
            runtime.handle_command(ApplicationCommand::RemoveTrackFromLibrary {
                track_id: removed_id,
            }),
            Ok(())
        );

        assert!(runtime.library_tracks().is_empty());
        assert_eq!(store.track(removed_id), Ok(None));
        assert_eq!(runtime.playback_state(), PlaybackState::Stopped);

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn runtime_moves_tracks_to_trash_and_removes_underlying_file() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        let track_path = root.join("track.flac");
        std::fs::write(&track_path, b"not real audio").expect("write fake track");

        let trashed_id = track_id(1);
        let store = Arc::new(InMemoryLibraryStore::new());
        assert_eq!(
            store.save_track(test_track(trashed_id, "track.flac")),
            Ok(())
        );

        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store.clone(), Arc::new(TestMetadataService))
        .expect("library services initialize")
        .with_playback_service(Box::new(NullPlaybackService::new()));

        assert_eq!(
            runtime.handle_command(ApplicationCommand::MoveTrackToTrash {
                track_id: trashed_id,
            }),
            Ok(())
        );

        assert!(runtime.library_tracks().is_empty());
        assert_eq!(store.track(trashed_id), Ok(None));
        assert!(!track_path.exists(), "audio file should be moved to trash");

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn runtime_move_to_trash_succeeds_when_file_is_already_missing() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");

        let trashed_id = track_id(1);
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut missing = test_track(trashed_id, "absent.flac");
        missing.location = missing_track_location("absent.flac");
        assert_eq!(store.save_track(missing), Ok(()));

        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store.clone(), Arc::new(TestMetadataService))
        .expect("library services initialize")
        .with_playback_service(Box::new(NullPlaybackService::new()));

        assert_eq!(
            runtime.handle_command(ApplicationCommand::MoveTrackToTrash {
                track_id: trashed_id,
            }),
            Ok(())
        );
        assert!(runtime.library_tracks().is_empty());
        assert_eq!(store.track(trashed_id), Ok(None));

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn runtime_creates_renames_and_deletes_playlists() {
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store.clone(), Arc::new(TestMetadataService))
            .expect("library services initialize");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::CreatePlaylist {
                name: "  Favorites  ".to_owned(),
                parent_folder_id: None,
            }),
            Ok(())
        );
        let playlist_id = playlist_id(1);
        assert_eq!(runtime.playlists()[0].name, "Favorites");
        assert_eq!(
            store
                .playlist(playlist_id)
                .expect("playlist loads")
                .map(|playlist| playlist.name),
            Some("Favorites".to_owned())
        );

        assert_eq!(
            runtime.handle_command(ApplicationCommand::RenamePlaylist {
                playlist_id,
                name: "Road".to_owned(),
            }),
            Ok(())
        );
        assert_eq!(runtime.playlists()[0].name, "Road");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::DeletePlaylist { playlist_id }),
            Ok(())
        );
        assert!(runtime.playlists().is_empty());
        assert_eq!(store.playlist(playlist_id), Ok(None));
    }

    #[test]
    fn runtime_updates_playlist_entries_in_store_and_cache() {
        let store = Arc::new(InMemoryLibraryStore::new());
        for id in [1, 2, 3] {
            assert_eq!(
                store.save_track(test_track(track_id(id), &format!("track-{id}.flac"))),
                Ok(())
            );
        }
        let playlist_id = playlist_id(1);
        assert_eq!(
            store.save_playlist(Playlist {
                id: playlist_id,
                name: "Favorites".to_owned(),
                parent_folder_id: None,
                position: 0,
                entries: Vec::new(),
            }),
            Ok(())
        );
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store.clone(), Arc::new(TestMetadataService))
            .expect("library services initialize");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::AddTracksToPlaylist {
                playlist_id,
                track_ids: vec![track_id(2), track_id(1), track_id(2)],
            }),
            Ok(())
        );
        assert_playlist_track_ids(
            runtime.playlists(),
            playlist_id,
            &[track_id(2), track_id(1)],
        );

        assert_eq!(
            runtime.handle_command(ApplicationCommand::MovePlaylistEntry {
                playlist_id,
                track_id: track_id(1),
                new_position: 0,
            }),
            Ok(())
        );
        assert_playlist_track_ids(
            runtime.playlists(),
            playlist_id,
            &[track_id(1), track_id(2)],
        );

        assert_eq!(
            runtime.handle_command(ApplicationCommand::RemoveTracksFromPlaylist {
                playlist_id,
                track_ids: vec![track_id(2)],
            }),
            Ok(())
        );
        assert_playlist_track_ids(runtime.playlists(), playlist_id, &[track_id(1)]);
        assert_playlist_track_ids(
            &[store
                .playlist(playlist_id)
                .expect("playlist loads")
                .expect("playlist exists")],
            playlist_id,
            &[track_id(1)],
        );
    }

    #[test]
    fn runtime_rejects_blank_playlist_names() {
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store, Arc::new(TestMetadataService))
            .expect("library services initialize");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::CreatePlaylist {
                name: "   ".to_owned(),
                parent_folder_id: None,
            }),
            Err(ApplicationRuntimeError::InvalidPlaylistName)
        );
    }

    #[test]
    fn runtime_creates_renames_and_deletes_playlist_folders() {
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store.clone(), Arc::new(TestMetadataService))
            .expect("library services initialize");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::CreatePlaylistFolder {
                name: "  Mixes  ".to_owned(),
                parent_folder_id: None,
            }),
            Ok(())
        );
        let folder_id = folder_id(1);
        assert_eq!(runtime.playlist_folders().len(), 1);
        assert_eq!(runtime.playlist_folders()[0].name, "Mixes");
        assert_eq!(runtime.playlist_folders()[0].position, 0);

        assert_eq!(
            runtime.handle_command(ApplicationCommand::RenamePlaylistFolder {
                folder_id,
                name: "Long Drives".to_owned(),
            }),
            Ok(())
        );
        assert_eq!(runtime.playlist_folders()[0].name, "Long Drives");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::DeletePlaylistFolder { folder_id }),
            Ok(())
        );
        assert!(runtime.playlist_folders().is_empty());
        assert_eq!(store.playlist_folder(folder_id), Ok(None));
    }

    #[test]
    fn runtime_rejects_blank_playlist_folder_names() {
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store, Arc::new(TestMetadataService))
            .expect("library services initialize");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::CreatePlaylistFolder {
                name: "  ".to_owned(),
                parent_folder_id: None,
            }),
            Err(ApplicationRuntimeError::InvalidPlaylistFolderName)
        );
    }

    #[test]
    fn runtime_rejects_creating_folder_under_missing_parent() {
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store, Arc::new(TestMetadataService))
            .expect("library services initialize");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::CreatePlaylistFolder {
                name: "Inside".to_owned(),
                parent_folder_id: Some(folder_id(999)),
            }),
            Err(ApplicationRuntimeError::PlaylistFolderNotFound)
        );
    }

    #[test]
    fn deleting_a_folder_cascades_and_reloads_runtime_state() {
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store.clone(), Arc::new(TestMetadataService))
            .expect("library services initialize");

        runtime
            .handle_command(ApplicationCommand::CreatePlaylistFolder {
                name: "Mixes".to_owned(),
                parent_folder_id: None,
            })
            .expect("create folder");
        let folder_id_value = folder_id(1);

        runtime
            .handle_command(ApplicationCommand::CreatePlaylist {
                name: "Inside".to_owned(),
                parent_folder_id: Some(folder_id_value),
            })
            .expect("create playlist inside folder");
        runtime
            .handle_command(ApplicationCommand::CreateSmartPlaylist {
                name: "Smart Inside".to_owned(),
                parent_folder_id: Some(folder_id_value),
                rules: test_rule_set(),
            })
            .expect("create smart playlist inside folder");

        assert_eq!(runtime.playlists().len(), 1);
        assert_eq!(runtime.smart_playlists().len(), 1);

        runtime
            .handle_command(ApplicationCommand::DeletePlaylistFolder {
                folder_id: folder_id_value,
            })
            .expect("delete folder cascades");

        assert!(runtime.playlist_folders().is_empty());
        assert!(runtime.playlists().is_empty());
        assert!(runtime.smart_playlists().is_empty());
    }

    #[test]
    fn runtime_creates_updates_and_deletes_smart_playlists() {
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store.clone(), Arc::new(TestMetadataService))
            .expect("library services initialize");

        runtime
            .handle_command(ApplicationCommand::CreateSmartPlaylist {
                name: "Recent".to_owned(),
                parent_folder_id: None,
                rules: test_rule_set(),
            })
            .expect("create smart playlist");
        let smart_id_value = smart_id(1);
        assert_eq!(runtime.smart_playlists().len(), 1);
        assert_eq!(runtime.smart_playlists()[0].name, "Recent");

        let new_rules = SmartPlaylistRuleSet {
            match_kind: SmartPlaylistMatchKind::Any,
            limit: None,
            rules: vec![SmartPlaylistRule::Text {
                field: SmartPlaylistTextField::Genre,
                operator: SmartPlaylistTextOperator::Is,
                value: "Trip-Hop".to_owned(),
            }],
        };
        runtime
            .handle_command(ApplicationCommand::UpdateSmartPlaylist {
                smart_playlist_id: smart_id_value,
                name: "Renamed".to_owned(),
                rules: new_rules.clone(),
            })
            .expect("update smart playlist");
        assert_eq!(runtime.smart_playlists()[0].name, "Renamed");
        assert_eq!(runtime.smart_playlists()[0].rules, new_rules);

        runtime
            .handle_command(ApplicationCommand::DeleteSmartPlaylist {
                smart_playlist_id: smart_id_value,
            })
            .expect("delete smart playlist");
        assert!(runtime.smart_playlists().is_empty());
    }

    #[test]
    fn runtime_rejects_smart_playlist_without_rules() {
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store, Arc::new(TestMetadataService))
            .expect("library services initialize");

        let empty_rules = SmartPlaylistRuleSet {
            match_kind: SmartPlaylistMatchKind::All,
            rules: Vec::new(),
            limit: None,
        };
        assert_eq!(
            runtime.handle_command(ApplicationCommand::CreateSmartPlaylist {
                name: "Empty".to_owned(),
                parent_folder_id: None,
                rules: empty_rules,
            }),
            Err(ApplicationRuntimeError::InvalidSmartPlaylistRules)
        );
    }

    #[test]
    fn new_siblings_get_distinct_positions_across_types() {
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store, Arc::new(TestMetadataService))
            .expect("library services initialize");

        runtime
            .handle_command(ApplicationCommand::CreatePlaylistFolder {
                name: "Mixes".to_owned(),
                parent_folder_id: None,
            })
            .expect("folder");
        runtime
            .handle_command(ApplicationCommand::CreatePlaylist {
                name: "Manual".to_owned(),
                parent_folder_id: None,
            })
            .expect("playlist");
        runtime
            .handle_command(ApplicationCommand::CreateSmartPlaylist {
                name: "Smart".to_owned(),
                parent_folder_id: None,
                rules: test_rule_set(),
            })
            .expect("smart");

        assert_eq!(runtime.playlist_folders()[0].position, 0);
        assert_eq!(runtime.playlists()[0].position, 1);
        assert_eq!(runtime.smart_playlists()[0].position, 2);
    }

    #[test]
    fn moving_a_playlist_within_its_folder_reorders_siblings() {
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store, Arc::new(TestMetadataService))
            .expect("library services initialize");

        for name in ["A", "B", "C"] {
            runtime
                .handle_command(ApplicationCommand::CreatePlaylist {
                    name: name.to_owned(),
                    parent_folder_id: None,
                })
                .expect("create");
        }
        let playlist_b_id = runtime
            .playlists()
            .iter()
            .find(|playlist| playlist.name == "B")
            .map(|playlist| playlist.id)
            .expect("playlist B exists");

        runtime
            .handle_command(ApplicationCommand::MovePlaylistItem {
                item: PlaylistItem::Playlist(playlist_b_id),
                target_parent_folder_id: None,
                position: 0,
            })
            .expect("move within folder");

        let mut ordered: Vec<&Playlist> = runtime.playlists().iter().collect();
        ordered.sort_by_key(|playlist| playlist.position);
        let names: Vec<&str> = ordered
            .iter()
            .map(|playlist| playlist.name.as_str())
            .collect();
        assert_eq!(names, vec!["B", "A", "C"]);
    }

    #[test]
    fn moving_a_playlist_across_folders_resequences_both_sides() {
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store, Arc::new(TestMetadataService))
            .expect("library services initialize");

        runtime
            .handle_command(ApplicationCommand::CreatePlaylistFolder {
                name: "Folder".to_owned(),
                parent_folder_id: None,
            })
            .expect("folder");
        let folder = folder_id(1);
        runtime
            .handle_command(ApplicationCommand::CreatePlaylist {
                name: "Top A".to_owned(),
                parent_folder_id: None,
            })
            .expect("top a");
        runtime
            .handle_command(ApplicationCommand::CreatePlaylist {
                name: "Top B".to_owned(),
                parent_folder_id: None,
            })
            .expect("top b");
        let top_a_id = runtime
            .playlists()
            .iter()
            .find(|playlist| playlist.name == "Top A")
            .map(|playlist| playlist.id)
            .expect("Top A exists");

        runtime
            .handle_command(ApplicationCommand::MovePlaylistItem {
                item: PlaylistItem::Playlist(top_a_id),
                target_parent_folder_id: Some(folder),
                position: 0,
            })
            .expect("move into folder");

        let in_folder: Vec<&Playlist> = runtime
            .playlists()
            .iter()
            .filter(|playlist| playlist.parent_folder_id == Some(folder))
            .collect();
        assert_eq!(in_folder.len(), 1);
        assert_eq!(in_folder[0].name, "Top A");
        assert_eq!(in_folder[0].position, 0);

        let at_top: Vec<&Playlist> = runtime
            .playlists()
            .iter()
            .filter(|playlist| playlist.parent_folder_id.is_none())
            .collect();
        assert_eq!(at_top.len(), 1);
        assert_eq!(at_top[0].name, "Top B");
        assert_eq!(at_top[0].position, 1);
        assert_eq!(runtime.playlist_folders()[0].position, 0);
    }

    #[test]
    fn moving_a_folder_into_its_own_descendant_is_rejected() {
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store, Arc::new(TestMetadataService))
            .expect("library services initialize");

        runtime
            .handle_command(ApplicationCommand::CreatePlaylistFolder {
                name: "Outer".to_owned(),
                parent_folder_id: None,
            })
            .expect("outer");
        let outer = folder_id(1);
        runtime
            .handle_command(ApplicationCommand::CreatePlaylistFolder {
                name: "Inner".to_owned(),
                parent_folder_id: Some(outer),
            })
            .expect("inner");
        let inner = folder_id(2);

        assert_eq!(
            runtime.handle_command(ApplicationCommand::MovePlaylistItem {
                item: PlaylistItem::Folder(outer),
                target_parent_folder_id: Some(inner),
                position: 0,
            }),
            Err(ApplicationRuntimeError::PlaylistFolderWouldCycle)
        );
    }

    fn folder_id(value: i64) -> PlaylistFolderId {
        match PlaylistFolderId::new(value) {
            Some(folder_id) => folder_id,
            None => unreachable!("hard-coded positive folder id should be valid"),
        }
    }

    fn smart_id(value: i64) -> SmartPlaylistId {
        match SmartPlaylistId::new(value) {
            Some(smart_id) => smart_id,
            None => unreachable!("hard-coded positive smart-playlist id should be valid"),
        }
    }

    fn test_rule_set() -> SmartPlaylistRuleSet {
        SmartPlaylistRuleSet {
            match_kind: SmartPlaylistMatchKind::All,
            rules: vec![SmartPlaylistRule::Text {
                field: SmartPlaylistTextField::Artist,
                operator: SmartPlaylistTextOperator::Contains,
                value: "Portishead".to_owned(),
            }],
            limit: None,
        }
    }

    #[derive(Debug)]
    struct TestSettingsStore {
        settings: Mutex<UserSettings>,
    }

    impl TestSettingsStore {
        fn new(settings: UserSettings) -> Self {
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

    impl SettingsStore for TestSettingsStore {
        fn load_settings(&self) -> SettingsResult<UserSettings> {
            Ok(self.settings_guard()?.clone())
        }

        fn save_settings(&self, settings: UserSettings) -> SettingsResult<()> {
            *self.settings_guard()? = settings;
            Ok(())
        }
    }

    #[derive(Debug)]
    struct TestMetadataService;

    impl MetadataService for TestMetadataService {
        fn read_metadata(&self, _path: &Path) -> MetadataResult<TrackMetadata> {
            Ok(TrackMetadata {
                title: Some("Track".to_owned()),
                ..TrackMetadata::default()
            })
        }

        fn write_metadata(&self, _path: &Path, _change: MetadataChange) -> MetadataResult<()> {
            Ok(())
        }

        fn read_rating(&self, _path: &Path) -> MetadataResult<Option<Rating>> {
            Ok(Some(Rating::new(3).expect("valid test rating")))
        }

        fn write_rating(&self, _path: &Path, _rating: Rating) -> MetadataResult<()> {
            Ok(())
        }

        fn read_artwork(&self, _path: &Path) -> MetadataResult<Option<Vec<u8>>> {
            Ok(None)
        }
    }

    #[derive(Debug)]
    struct RecordingMetadataService {
        fail_rating_writes: bool,
        fail_metadata_writes: bool,
        rating_writes: Mutex<Vec<(PathBuf, Rating)>>,
        metadata_writes: Mutex<Vec<(PathBuf, MetadataChange)>>,
    }

    impl RecordingMetadataService {
        fn new(fail_rating_writes: bool) -> Self {
            Self {
                fail_rating_writes,
                fail_metadata_writes: false,
                rating_writes: Mutex::new(Vec::new()),
                metadata_writes: Mutex::new(Vec::new()),
            }
        }

        fn with_metadata_write_failure() -> Self {
            Self {
                fail_rating_writes: false,
                fail_metadata_writes: true,
                rating_writes: Mutex::new(Vec::new()),
                metadata_writes: Mutex::new(Vec::new()),
            }
        }

        fn rating_writes(&self) -> Vec<(PathBuf, Rating)> {
            self.rating_writes
                .lock()
                .expect("rating writes lock is available")
                .clone()
        }

        fn metadata_writes(&self) -> Vec<(PathBuf, MetadataChange)> {
            self.metadata_writes
                .lock()
                .expect("metadata writes lock is available")
                .clone()
        }
    }

    impl MetadataService for RecordingMetadataService {
        fn read_metadata(&self, _path: &Path) -> MetadataResult<TrackMetadata> {
            Ok(TrackMetadata {
                title: Some("Track".to_owned()),
                ..TrackMetadata::default()
            })
        }

        fn write_metadata(&self, path: &Path, change: MetadataChange) -> MetadataResult<()> {
            self.metadata_writes
                .lock()
                .expect("metadata writes lock is available")
                .push((path.to_path_buf(), change));
            if self.fail_metadata_writes {
                Err(MetadataError::WriteFailed)
            } else {
                Ok(())
            }
        }

        fn read_rating(&self, _path: &Path) -> MetadataResult<Option<Rating>> {
            Ok(Some(Rating::new(3).expect("valid test rating")))
        }

        fn write_rating(&self, path: &Path, rating: Rating) -> MetadataResult<()> {
            self.rating_writes
                .lock()
                .expect("rating writes lock is available")
                .push((path.to_path_buf(), rating));
            if self.fail_rating_writes {
                Err(MetadataError::WriteFailed)
            } else {
                Ok(())
            }
        }

        fn read_artwork(&self, _path: &Path) -> MetadataResult<Option<Vec<u8>>> {
            Ok(None)
        }
    }

    fn unique_test_directory() -> PathBuf {
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("xtunes_runtime_test_{unique_suffix}"))
    }

    fn positive_track_id() -> TrackId {
        track_id(1)
    }

    fn track_id(value: i64) -> TrackId {
        match TrackId::new(value) {
            Some(track_id) => track_id,
            None => unreachable!("hard-coded positive track id should be valid"),
        }
    }

    fn playlist_id(value: i64) -> PlaylistId {
        match PlaylistId::new(value) {
            Some(playlist_id) => playlist_id,
            None => unreachable!("hard-coded positive playlist id should be valid"),
        }
    }

    fn assert_playlist_track_ids(
        playlists: &[Playlist],
        playlist_id: PlaylistId,
        expected_track_ids: &[TrackId],
    ) {
        let playlist = playlists
            .iter()
            .find(|playlist| playlist.id == playlist_id)
            .expect("playlist exists");
        let track_ids = playlist
            .entries
            .iter()
            .map(|entry| entry.track_id)
            .collect::<Vec<_>>();
        let positions = playlist
            .entries
            .iter()
            .map(|entry| entry.position)
            .collect::<Vec<_>>();

        assert_eq!(track_ids, expected_track_ids);
        assert_eq!(
            positions,
            (0..expected_track_ids.len() as u32).collect::<Vec<_>>()
        );
    }

    fn test_track(track_id: TrackId, path: &str) -> Track {
        Track {
            id: track_id,
            location: track_location(path),
            metadata: TrackMetadata::default(),
            rating: Rating::unrated(),
            statistics: PlayStatistics::default(),
        }
    }

    fn track_location(path: &str) -> TrackLocation {
        TrackLocation::available(relative_path(path))
    }

    fn missing_track_location(path: &str) -> TrackLocation {
        TrackLocation::missing(relative_path(path))
    }

    fn relative_path(path: &str) -> super::TrackRelativePath {
        super::TrackRelativePath::new(PathBuf::from(path)).expect("test path is relative")
    }

    fn _assert_store_result_is_public<T>(result: StoreResult<T>) -> StoreResult<T> {
        result
    }

    fn _assert_playlist_types_are_public(playlist: Playlist, playlist_id: PlaylistId) {
        let _value = (playlist, playlist_id);
    }

    fn _assert_metadata_error_is_public(error: MetadataError) -> MetadataError {
        error
    }
}
