#![forbid(unsafe_code)]

use std::cell::Cell;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub use xtunes_domain::{
    ApplicationCommand, ApplicationQuery, PlayStatistics, PlaybackCommand, PlaybackState, Track,
    TrackId, TrackLocation, TrackPlaybackSource, UserSettings,
};

use xtunes_library_store::LibraryStore;
pub use xtunes_metadata::MetadataService;
use xtunes_metadata::LibraryScanner;
use xtunes_playback::PlaybackService;
use xtunes_settings::SettingsStore;

pub type ApplicationRuntimeResult<T> = Result<T, ApplicationRuntimeError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationRuntimeError {
    LibraryScanFailed,
    LibraryServicesUnavailable,
    LibraryStoreFailed,
    PlaybackFailed,
    PlaybackServiceUnavailable,
    SettingsLoadFailed,
    SettingsSaveFailed,
    TrackUnavailable,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LibraryScanSummary {
    pub scanned_tracks: usize,
    pub skipped_unsupported_files: usize,
    pub failed_files: usize,
}

/// Parameters extracted from the runtime for off-thread scanning.
pub struct ScanParameters {
    pub library_path: PathBuf,
    pub library_store: Arc<dyn LibraryStore>,
    pub metadata_service: Arc<dyn MetadataService>,
}

/// Result of an off-thread library scan.
#[derive(Clone, Debug)]
pub struct ScanResult {
    pub tracks: Vec<Track>,
    pub skipped_unsupported_files: usize,
    pub failed_files: usize,
}

/// Run the full library scan: walk filesystem, read metadata, persist to store.
/// Designed to be called from a background thread.
pub fn run_library_scan(
    library_path: PathBuf,
    library_store: Arc<dyn LibraryStore>,
    metadata_service: Arc<dyn MetadataService>,
) -> ApplicationRuntimeResult<ScanResult> {
    let scan = LibraryScanner::new(metadata_service.as_ref())
        .scan(&library_path)
        .map_err(|_| ApplicationRuntimeError::LibraryScanFailed)?;

    let mut tracks = Vec::with_capacity(scan.tracks.len());
    for (index, scanned_track) in scan.tracks.into_iter().enumerate() {
        let Some(track_id) = TrackId::new(index as i64 + 1) else {
            return Err(ApplicationRuntimeError::LibraryStoreFailed);
        };
        let track = Track {
            id: track_id,
            location: TrackLocation::new(scanned_track.path),
            metadata: scanned_track.metadata,
            rating: scanned_track.rating,
            statistics: PlayStatistics::default(),
        };
        library_store
            .save_track(track.clone())
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        tracks.push(track);
    }

    Ok(ScanResult {
        tracks,
        skipped_unsupported_files: scan.skipped_unsupported_files,
        failed_files: scan.failures.len(),
    })
}

/// Messages sent from the background scan thread to the main loop.
pub enum ScanMessage {
    /// A new track was found and saved — append it to the library.
    TrackFound(Track),
    /// The scan completed (success or error).
    Done(ApplicationRuntimeResult<LibraryScanSummary>),
}

/// Run an incremental library scan: sends each discovered track through `tx` immediately,
/// then sends a final `Done` message when complete. Designed for background threads.
pub fn run_incremental_scan(
    params: ScanParameters,
    tx: std::sync::mpsc::Sender<ScanMessage>,
) {
    let ScanParameters {
        library_path,
        library_store,
        metadata_service,
    } = params;

    if !library_path.is_dir() {
        let _ = tx.send(ScanMessage::Done(Err(ApplicationRuntimeError::LibraryScanFailed)));
        return;
    }

    let track_index = Cell::new(0i64);
    let failed_files = Cell::new(0usize);

    let result = LibraryScanner::new(metadata_service.as_ref()).scan_incremental(
        &library_path,
        |scanned_track| {
            track_index.set(track_index.get() + 1);
            let Some(track_id) = TrackId::new(track_index.get()) else {
                return;
            };
            let track = Track {
                id: track_id,
                location: TrackLocation::new(scanned_track.path),
                metadata: scanned_track.metadata,
                rating: scanned_track.rating,
                statistics: PlayStatistics::default(),
            };
            if library_store.save_track(track.clone()).is_err() {
                failed_files.set(failed_files.get() + 1);
                return;
            }
            let _ = tx.send(ScanMessage::TrackFound(track));
        },
    );

    match result {
        Ok(scan) => {
            let summary = LibraryScanSummary {
                scanned_tracks: track_index.get() as usize,
                skipped_unsupported_files: scan.skipped_unsupported_files,
                failed_files: failed_files.get() + scan.failures.len(),
            };
            let _ = tx.send(ScanMessage::Done(Ok(summary)));
        }
        Err(_) => {
            let _ = tx.send(ScanMessage::Done(Err(ApplicationRuntimeError::LibraryScanFailed)));
        }
    }
}

pub struct ApplicationRuntime {
    settings: UserSettings,
    settings_store: Option<Box<dyn SettingsStore>>,
    library_store: Option<Arc<dyn LibraryStore>>,
    metadata_service: Option<Arc<dyn MetadataService>>,
    playback_service: Option<Box<dyn PlaybackService>>,
    library_tracks: Vec<Track>,
    last_scan_library_path: Option<PathBuf>,
    last_scan_summary: Option<LibraryScanSummary>,
}

impl ApplicationRuntime {
    pub fn new() -> Self {
        Self {
            settings: UserSettings::default(),
            settings_store: None,
            library_store: None,
            metadata_service: None,
            playback_service: None,
            library_tracks: Vec::new(),
            last_scan_library_path: None,
            last_scan_summary: None,
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
            library_tracks: Vec::new(),
            last_scan_library_path: None,
            last_scan_summary: None,
        })
    }

    pub fn with_library_services(
        mut self,
        library_store: Arc<dyn LibraryStore>,
        metadata_service: Arc<dyn MetadataService>,
    ) -> Self {
        self.library_tracks = library_store.tracks().unwrap_or_default();
        self.library_store = Some(library_store);
        self.metadata_service = Some(metadata_service);
        self
    }

    pub fn clone_library_services(
        &self,
    ) -> Option<(Arc<dyn LibraryStore>, Arc<dyn MetadataService>)> {
        match (&self.library_store, &self.metadata_service) {
            (Some(store), Some(metadata)) => Some((store.clone(), metadata.clone())),
            _ => None,
        }
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

    pub fn playback_state(&self) -> PlaybackState {
        self.playback_service
            .as_deref()
            .map(PlaybackService::state)
            .unwrap_or_default()
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
            }
            ApplicationCommand::ScanLibrary { library_path } => {
                self.scan_library(library_path)?;
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_playback_command(&self, command: PlaybackCommand) -> ApplicationRuntimeResult<()> {
        let playback_service = self
            .playback_service
            .as_deref()
            .ok_or(ApplicationRuntimeError::PlaybackServiceUnavailable)?;

        match command {
            PlaybackCommand::PlayTrack(track_id) => {
                let track = self
                    .library_tracks
                    .iter()
                    .find(|track| track.id == track_id)
                    .ok_or(ApplicationRuntimeError::TrackUnavailable)?;
                playback_service
                    .play_track(TrackPlaybackSource::new(
                        track_id,
                        track.location.path.clone(),
                    ))
                    .map_err(|_| ApplicationRuntimeError::PlaybackFailed)?;
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
        }

        Ok(())
    }

    fn scan_library(&mut self, library_path: PathBuf) -> ApplicationRuntimeResult<()> {
        self.last_scan_library_path = Some(library_path.clone());

        let (library_store, metadata_service) = self
            .clone_library_services()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;

        let scan_result = run_library_scan(library_path, library_store, metadata_service)?;

        self.last_scan_summary = Some(LibraryScanSummary {
            scanned_tracks: scan_result.tracks.len(),
            skipped_unsupported_files: scan_result.skipped_unsupported_files,
            failed_files: scan_result.failed_files,
        });
        self.library_tracks = scan_result.tracks;
        Ok(())
    }

    /// Returns the services needed for scanning, to be consumed off-thread.
    pub fn take_scan_parameters(
        &self,
        library_path: PathBuf,
    ) -> ApplicationRuntimeResult<ScanParameters> {
        let (library_store, metadata_service) = self
            .clone_library_services()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;
        Ok(ScanParameters {
            library_path,
            library_store,
            metadata_service,
        })
    }

    /// Apply the results of a background scan to this runtime.
    pub fn apply_scan_result(&mut self, result: ScanResult) -> LibraryScanSummary {
        let summary = LibraryScanSummary {
            scanned_tracks: result.tracks.len(),
            skipped_unsupported_files: result.skipped_unsupported_files,
            failed_files: result.failed_files,
        };
        self.library_tracks = result.tracks;
        self.last_scan_summary = Some(summary.clone());
        summary
    }

    /// Append a single track discovered during an incremental scan.
    /// Returns `true` when the UI should refresh (batched to avoid O(N²) table rebuilds).
    pub fn apply_scanned_track(&mut self, track: Track) -> bool {
        self.library_tracks.push(track);
        const BATCH_THRESHOLD: usize = 50;
        self.library_tracks.len() % BATCH_THRESHOLD == 0
    }

    /// Finalize an incremental scan by updating the summary.
    pub fn finalize_scan(&mut self, summary: LibraryScanSummary) {
        self.last_scan_summary = Some(summary);
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
        sync::{Mutex, MutexGuard},
    };

    use xtunes_domain::{
        ApplicationCommand, PlayStatistics, PlaybackCommand, PlaybackState, Playlist, PlaylistId,
        Rating, Track, TrackId, TrackLocation, TrackMetadata, UserSettings,
    };
    use xtunes_library_store::{InMemoryLibraryStore, LibraryStore, StoreResult};
    use xtunes_metadata::{MetadataChange, MetadataError, MetadataResult};
    use xtunes_playback::NullPlaybackService;
    use xtunes_settings::{SettingsError, SettingsResult, SettingsStore};

    use super::{ApplicationRuntime, ApplicationRuntimeError, MetadataService};

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
        use std::sync::Arc;

        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        let track_path = root.join("track.mp3");
        std::fs::write(&track_path, b"not real audio").expect("write fake track");

        let store = Arc::new(InMemoryLibraryStore::new());
        let metadata_service = Arc::new(TestMetadataService);
        let mut runtime = ApplicationRuntime::new().with_library_services(store, metadata_service);

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
        use std::sync::Arc;

        let track_id = positive_track_id();
        let store = Arc::new(InMemoryLibraryStore::new());
        let track = Track {
            id: track_id,
            location: TrackLocation::new("/music/track.flac"),
            metadata: TrackMetadata::default(),
            rating: Rating::unrated(),
            statistics: PlayStatistics::default(),
        };
        assert_eq!(store.save_track(track), Ok(()));

        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store, Arc::new(TestMetadataService))
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
    }

    fn unique_test_directory() -> PathBuf {
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("xtunes_runtime_test_{unique_suffix}"))
    }

    fn positive_track_id() -> TrackId {
        match TrackId::new(1) {
            Some(track_id) => track_id,
            None => unreachable!("hard-coded positive track id should be valid"),
        }
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
