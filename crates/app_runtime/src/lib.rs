#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

pub use xtunes_domain::{
    ApplicationCommand, ApplicationQuery, PlayStatistics, PlaybackCommand, PlaybackOptions,
    PlaybackState, Rating, Track, TrackAvailability, TrackId, TrackLocation, TrackMetadata,
    TrackPlaybackSource, TrackRelativePath, UserSettings, VolumePercent,
};
use xtunes_library_store::LibraryStore;
use xtunes_metadata::{LibraryScan, LibraryScanner, MetadataService, ScannedTrack};
use xtunes_playback::PlaybackService;
use xtunes_settings::SettingsStore;

pub type ApplicationRuntimeResult<T> = Result<T, ApplicationRuntimeError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationRuntimeError {
    LibraryScanFailed,
    LibraryServicesUnavailable,
    LibraryStoreFailed,
    BackgroundTaskRunning,
    PlaybackFailed,
    PlaybackServiceUnavailable,
    SettingsLoadFailed,
    SettingsSaveFailed,
    TrackUnavailable,
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
    playback_options: PlaybackOptions,
    library_tracks: Vec<Track>,
    last_scan_library_path: Option<PathBuf>,
    last_scan_summary: Option<LibraryScanSummary>,
    background_task_status: BackgroundTaskStatus,
}

#[derive(Clone, Copy)]
enum TrackStep {
    Previous,
    Next,
}

impl ApplicationRuntime {
    pub fn new() -> Self {
        Self {
            settings: UserSettings::default(),
            settings_store: None,
            library_store: None,
            metadata_service: None,
            playback_service: None,
            playback_options: PlaybackOptions::default(),
            library_tracks: Vec::new(),
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
            playback_options: PlaybackOptions::default(),
            library_tracks: Vec::new(),
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
        self.library_tracks = load_library_tracks(
            library_store.as_ref(),
            self.settings.library_path.as_deref(),
        )?;
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

    pub fn playback_options(&self) -> PlaybackOptions {
        self.playback_options
    }

    pub fn now_playing(&self) -> NowPlaying {
        let state = self.playback_state();
        let track = playback_track_id(&state)
            .and_then(|track_id| {
                self.library_tracks
                    .iter()
                    .find(|track| track.id == track_id)
            })
            .cloned();

        NowPlaying {
            track,
            state,
            options: self.playback_options,
        }
    }

    pub fn read_artwork(&self, path: &Path) -> Option<Vec<u8>> {
        self.metadata_service
            .as_deref()
            .and_then(|service| service.read_artwork(path).ok().flatten())
    }

    pub fn absolute_track_path(&self, track: &Track) -> Option<PathBuf> {
        self.settings
            .library_path
            .as_deref()
            .map(|library_path| track.location.absolute_path(library_path))
    }

    pub fn handle_command(&mut self, command: ApplicationCommand) -> ApplicationRuntimeResult<()> {
        match command {
            ApplicationCommand::Playback(command) => {
                self.handle_playback_command(command)?;
            }
            ApplicationCommand::UpdateSettings(settings) => {
                if let Some(settings_store) = &self.settings_store {
                    settings_store
                        .save_settings(settings.clone())
                        .map_err(|_| ApplicationRuntimeError::SettingsSaveFailed)?;
                }
                self.settings = settings;
                if let Some(library_path) = self.settings.library_path.as_deref() {
                    self.library_tracks = self
                        .library_tracks
                        .drain(..)
                        .map(|track| track_with_current_availability(library_path, track))
                        .collect();
                }
            }
            ApplicationCommand::ScanLibrary { library_path } => {
                self.scan_library(library_path)?;
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_playback_command(
        &mut self,
        command: PlaybackCommand,
    ) -> ApplicationRuntimeResult<()> {
        match command {
            PlaybackCommand::ToggleShuffle => {
                self.playback_options = self.playback_options.with_shuffle_toggled();
                return Ok(());
            }
            PlaybackCommand::ToggleRepeat => {
                self.playback_options = self.playback_options.with_repeat_toggled();
                return Ok(());
            }
            _ => {}
        }

        let playback_service = self
            .playback_service
            .as_deref()
            .ok_or(ApplicationRuntimeError::PlaybackServiceUnavailable)?;

        match command {
            PlaybackCommand::PlayTrack(track_id) => {
                self.play_track(playback_service, track_id)?;
            }
            PlaybackCommand::PlayPreviousTrack => {
                if let Some(track_id) =
                    self.adjacent_track_id(playback_service.state(), TrackStep::Previous)
                {
                    self.play_track(playback_service, track_id)?;
                }
            }
            PlaybackCommand::PlayNextTrack => {
                if let Some(track_id) =
                    self.adjacent_track_id(playback_service.state(), TrackStep::Next)
                {
                    self.play_track(playback_service, track_id)?;
                }
            }
            PlaybackCommand::Pause => {
                playback_service
                    .pause()
                    .map_err(|_| ApplicationRuntimeError::PlaybackFailed)?;
            }
            PlaybackCommand::Resume => {
                playback_service
                    .resume()
                    .map_err(|_| ApplicationRuntimeError::PlaybackFailed)?;
            }
            PlaybackCommand::TogglePlayPause => match playback_service.state() {
                PlaybackState::Playing { .. } => {
                    playback_service
                        .pause()
                        .map_err(|_| ApplicationRuntimeError::PlaybackFailed)?;
                }
                PlaybackState::Paused { .. } => {
                    playback_service
                        .resume()
                        .map_err(|_| ApplicationRuntimeError::PlaybackFailed)?;
                }
                PlaybackState::Stopped | PlaybackState::Loading { .. } => {}
            },
            PlaybackCommand::Stop => {
                playback_service
                    .stop()
                    .map_err(|_| ApplicationRuntimeError::PlaybackFailed)?;
            }
            PlaybackCommand::Seek(position) => {
                playback_service
                    .seek(position)
                    .map_err(|_| ApplicationRuntimeError::PlaybackFailed)?;
            }
            PlaybackCommand::SetVolume(volume) => {
                playback_service
                    .set_volume(volume)
                    .map_err(|_| ApplicationRuntimeError::PlaybackFailed)?;
            }
            PlaybackCommand::ToggleShuffle | PlaybackCommand::ToggleRepeat => {
                unreachable!("playback option commands return before requiring a playback service")
            }
        }

        Ok(())
    }

    fn play_track(
        &self,
        playback_service: &dyn PlaybackService,
        track_id: TrackId,
    ) -> ApplicationRuntimeResult<()> {
        let track = self
            .library_tracks
            .iter()
            .find(|track| track.id == track_id && !track.location.is_missing())
            .ok_or(ApplicationRuntimeError::TrackUnavailable)?;
        let path = self
            .absolute_track_path(track)
            .ok_or(ApplicationRuntimeError::TrackUnavailable)?;
        playback_service
            .play_track(TrackPlaybackSource::new(track_id, path))
            .map_err(|_| ApplicationRuntimeError::PlaybackFailed)
    }

    fn adjacent_track_id(&self, state: PlaybackState, step: TrackStep) -> Option<TrackId> {
        let current_track_id = playback_track_id(&state)?;
        let playable_tracks = self
            .library_tracks
            .iter()
            .filter(|track| !track.location.is_missing())
            .collect::<Vec<_>>();
        let current_index = playable_tracks
            .iter()
            .position(|track| track.id == current_track_id)?;
        let next_index = match step {
            TrackStep::Previous => current_index.checked_sub(1)?,
            TrackStep::Next => current_index.checked_add(1)?,
        };

        playable_tracks.get(next_index).map(|track| track.id)
    }

    fn scan_library(&mut self, library_path: PathBuf) -> ApplicationRuntimeResult<()> {
        let task = self.prepare_library_scan(library_path)?;
        match run_library_scan_task(task) {
            Ok(result) => {
                self.apply_library_scan_result(result);
                Ok(())
            }
            Err(error) => {
                self.fail_library_scan(error.clone());
                Err(error)
            }
        }
    }

    pub fn prepare_library_scan(
        &mut self,
        library_path: PathBuf,
    ) -> ApplicationRuntimeResult<LibraryScanTask> {
        if self.background_task_status.is_running() {
            return Err(ApplicationRuntimeError::BackgroundTaskRunning);
        }

        self.last_scan_library_path = Some(library_path.clone());
        let library_store = self
            .library_store
            .clone()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;
        let metadata_service = self
            .metadata_service
            .clone()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;

        self.background_task_status = BackgroundTaskStatus::LibraryScanRunning;

        Ok(LibraryScanTask {
            library_path,
            existing_tracks: self.library_tracks.clone(),
            library_store,
            metadata_service,
        })
    }

    pub fn apply_library_scan_result(&mut self, result: LibraryScanResult) {
        let summary = result.summary;
        self.last_scan_summary = Some(summary.clone());
        self.library_tracks = result.tracks;
        self.background_task_status = BackgroundTaskStatus::LibraryScanCompleted(summary);
    }

    pub fn fail_library_scan(&mut self, error: ApplicationRuntimeError) {
        self.background_task_status = BackgroundTaskStatus::LibraryScanFailed(error);
    }
}

pub fn run_library_scan_task(task: LibraryScanTask) -> ApplicationRuntimeResult<LibraryScanResult> {
    let scan = LibraryScanner::new(task.metadata_service.as_ref())
        .scan(&task.library_path)
        .map_err(|_| ApplicationRuntimeError::LibraryScanFailed)?;
    let result = reconcile_library_scan(&task.library_path, task.existing_tracks, scan)?;

    for track in &result.tracks {
        task.library_store
            .save_track(track.clone())
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
    }

    Ok(result)
}

fn reconcile_library_scan(
    library_path: &Path,
    existing_tracks: Vec<Track>,
    scan: LibraryScan,
) -> ApplicationRuntimeResult<LibraryScanResult> {
    let skipped_unsupported_files = scan.skipped_unsupported_files;
    let failed_files = scan.failures.len();
    let scanned_tracks = scan.tracks;
    let mut tracks_by_path = tracks_by_path(existing_tracks.clone());
    let mut scanned_paths = BTreeSet::new();
    let mut tracks = Vec::new();
    let mut next_track_id = next_track_id(&existing_tracks)?;
    let mut added_tracks = 0;
    let mut updated_tracks = 0;

    let scanned_track_count = scanned_tracks.len();
    for scanned_track in scanned_tracks {
        scanned_paths.insert(scanned_track.relative_path.clone());
        let existing_track = tracks_by_path.remove(&scanned_track.relative_path);
        if existing_track.is_some() {
            updated_tracks += 1;
        } else {
            added_tracks += 1;
        }
        let track = track_from_scanned_track(scanned_track, existing_track, &mut next_track_id)?;
        tracks.push(track);
    }

    let mut missing_tracks = 0;
    for track in existing_tracks
        .into_iter()
        .filter(|track| !scanned_paths.contains(&track.location.relative_path))
    {
        let track = track_with_current_availability(library_path, track);
        if track.location.is_missing() {
            missing_tracks += 1;
        }
        tracks.push(track);
    }

    tracks.sort_by_key(|track| track.id);

    Ok(LibraryScanResult {
        summary: LibraryScanSummary {
            scanned_tracks: scanned_track_count,
            added_tracks,
            updated_tracks,
            missing_tracks,
            skipped_unsupported_files,
            failed_files,
        },
        tracks,
    })
}

fn tracks_by_path(tracks: Vec<Track>) -> BTreeMap<TrackRelativePath, Track> {
    tracks
        .into_iter()
        .map(|track| (track.location.relative_path.clone(), track))
        .collect()
}

fn track_from_scanned_track(
    scanned_track: ScannedTrack,
    existing_track: Option<Track>,
    next_track_id: &mut i64,
) -> ApplicationRuntimeResult<Track> {
    let (id, statistics) = match existing_track {
        Some(track) => (track.id, track.statistics),
        None => {
            let Some(track_id) = TrackId::new(*next_track_id) else {
                return Err(ApplicationRuntimeError::LibraryStoreFailed);
            };
            *next_track_id += 1;
            (track_id, PlayStatistics::default())
        }
    };

    Ok(Track {
        id,
        location: TrackLocation::available(scanned_track.relative_path),
        metadata: scanned_track.metadata,
        rating: scanned_track.rating,
        statistics,
    })
}

fn track_with_current_availability(library_path: &Path, track: Track) -> Track {
    let Track {
        id,
        location,
        metadata,
        rating,
        statistics,
    } = track;
    let availability = if location.absolute_path(library_path).exists() {
        TrackAvailability::Available
    } else {
        TrackAvailability::Missing
    };

    Track {
        id,
        location: match availability {
            TrackAvailability::Available => TrackLocation::available(location.relative_path),
            TrackAvailability::Missing => TrackLocation::missing(location.relative_path),
        },
        metadata,
        rating,
        statistics,
    }
}

fn load_library_tracks(
    library_store: &dyn LibraryStore,
    library_path: Option<&Path>,
) -> ApplicationRuntimeResult<Vec<Track>> {
    let tracks = library_store
        .tracks()
        .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;

    Ok(match library_path {
        Some(library_path) => tracks
            .into_iter()
            .map(|track| track_with_current_availability(library_path, track))
            .collect(),
        None => tracks,
    })
}

fn next_track_id(existing_tracks: &[Track]) -> ApplicationRuntimeResult<i64> {
    let next_id = existing_tracks
        .iter()
        .map(|track| track.id.get())
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(ApplicationRuntimeError::LibraryStoreFailed)?;

    if TrackId::new(next_id).is_some() {
        Ok(next_id)
    } else {
        Err(ApplicationRuntimeError::LibraryStoreFailed)
    }
}

fn playback_track_id(state: &PlaybackState) -> Option<TrackId> {
    match state {
        PlaybackState::Loading { track_id }
        | PlaybackState::Playing { track_id, .. }
        | PlaybackState::Paused { track_id, .. } => Some(*track_id),
        PlaybackState::Stopped => None,
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
        ApplicationCommand, PlayStatistics, PlaybackCommand, PlaybackOptions, PlaybackState,
        Playlist, PlaylistId, Rating, Track, TrackId, TrackLocation, TrackMetadata, UserSettings,
    };
    use xtunes_library_store::{InMemoryLibraryStore, LibraryStore, StoreResult};
    use xtunes_metadata::{MetadataChange, MetadataError, MetadataResult};
    use xtunes_playback::NullPlaybackService;
    use xtunes_settings::{SettingsError, SettingsResult, SettingsStore};

    use super::{ApplicationRuntime, ApplicationRuntimeError, LibraryScanSummary, MetadataService};

    #[test]
    fn runtime_starts_with_default_settings() {
        let runtime = ApplicationRuntime::new();

        assert_eq!(runtime.settings().library_path, None);
    }

    #[test]
    fn runtime_accepts_settings_command() {
        let mut runtime = ApplicationRuntime::new();

        let settings = UserSettings {
            library_path: Some(PathBuf::from("/music")),
        };

        assert_eq!(
            runtime.handle_command(ApplicationCommand::UpdateSettings(settings.clone())),
            Ok(())
        );

        assert_eq!(runtime.settings(), &settings);
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

        let settings_store = Box::new(TestSettingsStore::new(UserSettings {
            library_path: Some(old_root),
        }));
        let mut runtime =
            ApplicationRuntime::with_settings_store(settings_store).expect("load settings");
        runtime = runtime
            .with_library_services(store, Arc::new(TestMetadataService))
            .expect("library services initialize");

        assert!(runtime.library_tracks()[0].location.is_missing());
        assert_eq!(
            runtime.handle_command(ApplicationCommand::UpdateSettings(UserSettings {
                library_path: Some(new_root.clone()),
            })),
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
        let store = Box::new(TestSettingsStore::new(UserSettings {
            library_path: Some(PathBuf::from("/initial")),
        }));
        let mut runtime =
            ApplicationRuntime::with_settings_store(store).expect("load settings from test store");
        let updated_settings = UserSettings {
            library_path: Some(PathBuf::from("/updated")),
        };

        assert_eq!(
            runtime.settings(),
            &UserSettings {
                library_path: Some(PathBuf::from("/initial")),
            }
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
            TestSettingsStore::new(UserSettings {
                library_path: Some(root.clone()),
            }),
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
                repeat_enabled: false,
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
                repeat_enabled: true,
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
                repeat_enabled: false,
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
            TestSettingsStore::new(UserSettings {
                library_path: Some(root.clone()),
            }),
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
            TestSettingsStore::new(UserSettings {
                library_path: Some(root.clone()),
            }),
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
