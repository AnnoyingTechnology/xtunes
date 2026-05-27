// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

#![forbid(unsafe_code)]

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub use sustain_domain::{
    ApplicationCommand, ApplicationQuery, Clock, DEFAULT_PLAYBACK_VOLUME_PERCENT, FieldChange,
    LibraryManagementMode, LibrarySettings, MetadataChange, PlayStatistics, PlaybackCommand,
    PlaybackOptions, PlaybackQueue, PlaybackQueueRequest, PlaybackQueueSource, PlaybackSession,
    PlaybackSettings, PlaybackState, Playlist, PlaylistEntry, PlaylistFolder, PlaylistFolderId,
    PlaylistId, PlaylistItem, Rating, RepeatMode, SmartPlaylist, SmartPlaylistDateField,
    SmartPlaylistId, SmartPlaylistLimit, SmartPlaylistLimitSelection, SmartPlaylistMatchKind,
    SmartPlaylistNumberField, SmartPlaylistNumberOperator, SmartPlaylistRule, SmartPlaylistRuleSet,
    SmartPlaylistTextField, SmartPlaylistTextOperator, SystemClock, Track, TrackAvailability,
    TrackColumnEntry, TrackColumnLayout, TrackColumnLayoutScope, TrackContentHash, TrackId,
    TrackLocation, TrackMetadata, TrackPlaybackSource, TrackRelativePath, UiSettings, UiViewMode,
    UserSettings, VolumePercent, matching_tracks,
};
use sustain_library_store::LibraryStore;
pub use sustain_metadata::MetadataService;
pub use sustain_metadata_remote::{
    AudioFingerprint, FetchedArtwork, RemoteError, RemoteMetadataService, RemoteResult, TrackMatch,
    TrackMatchSource, TrackQuery,
};
use sustain_playback::PlaybackService;
pub use sustain_playback::TrackEndedCallback;
pub use sustain_search::{
    album_matches_search_text, filter_tracks_by_search_text, track_matches_search_text,
};
use sustain_settings::SettingsStore;

pub mod analysis_scheduler;
pub use analysis_scheduler::SchedulerProgress as AnalysisProgress;

pub(crate) mod artwork_fetcher;
mod commands;
mod library_mutation;
mod library_scan;
pub mod managed_library;
pub(crate) mod metadata_writer;
pub mod notifications;
mod playback;
mod playlist_folders;
mod playlist_items;
mod playlists;
mod smart_playlists;

pub use artwork_fetcher::{ArtworkFetchOutcome, ArtworkFetchResult};
pub use library_scan::run_library_scan_task;
pub use managed_library::{run_library_consolidation_task, run_library_import_task};
pub use metadata_writer::{MetadataWriteKind, MetadataWriteOutcome, MetadataWriteResult};
pub use notifications::{
    EPHEMERAL_NOTIFICATION_DURATION, NOTIFICATION_QUEUE_HARD_CAP, NOTIFICATION_TRANSITION,
    Notification, NotificationCategory, NotificationCenter, NotificationId, NotificationKind,
    NotificationSeverity, analysis_background_outcome_text, analysis_background_running_text,
    library_consolidation_outcome_text, library_consolidation_running_text,
    library_import_outcome_text, library_import_running_text, library_path_change_outcome_text,
    library_scan_outcome_text, library_scan_running_text, runtime_error_text,
};

pub type ApplicationRuntimeResult<T> = Result<T, ApplicationRuntimeError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationRuntimeError {
    ArtworkFetchingUnavailable,
    LibraryPathUnavailable,
    LibraryConsolidationFailed,
    LibraryImportFailed,
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
    // True when the scan stopped because the user asked it to. The
    // numbers above reflect the partial work that completed; we do not
    // sweep the unwalked portion of the library for missing tracks.
    pub cancelled: bool,
}

/// Live truth about which (if any) mutually-exclusive background task
/// owns the runtime right now. Outcome and failure messaging is no
/// longer routed through this enum — completed and failed states are
/// reported as notifications via [`NotificationCenter`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BackgroundTaskStatus {
    #[default]
    Idle,
    LibraryScanRunning,
    LibraryImportRunning,
    LibraryConsolidationRunning,
}

impl BackgroundTaskStatus {
    pub fn is_running(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    pub fn is_library_consolidation_running(&self) -> bool {
        matches!(self, Self::LibraryConsolidationRunning)
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
    cancellation_requested: Arc<AtomicBool>,
}

pub struct LibraryImportTask {
    paths: Vec<PathBuf>,
    settings: UserSettings,
    existing_tracks: Vec<Track>,
    library_store: Arc<dyn LibraryStore>,
    metadata_service: Arc<dyn MetadataService>,
    cancellation_requested: Arc<AtomicBool>,
}

pub struct LibraryConsolidationTask {
    settings: UserSettings,
    existing_tracks: Vec<Track>,
    library_store: Arc<dyn LibraryStore>,
    cancellation_requested: Arc<AtomicBool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LibraryScanResult {
    pub tracks: Vec<Track>,
    pub summary: LibraryScanSummary,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LibraryImportResult {
    pub tracks: Vec<Track>,
    pub summary: LibraryImportSummary,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LibraryConsolidationResult {
    pub tracks: Vec<Track>,
    pub summary: LibraryConsolidationSummary,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LibraryImportSummary {
    pub discovered_files: usize,
    pub imported_tracks: usize,
    pub duplicate_files: usize,
    // True when the import stopped because the user asked it to. The
    // import is all-or-nothing: a cancelled run rolls back any files
    // already copied and never partially populates the library.
    pub cancelled: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LibraryConsolidationSummary {
    pub planned_tracks: usize,
    pub moved_tracks: usize,
    pub already_organized_tracks: usize,
    pub missing_tracks: usize,
    pub cancelled: bool,
}

pub struct ApplicationRuntime {
    settings: UserSettings,
    settings_store: Option<Box<dyn SettingsStore>>,
    library_store: Option<Arc<dyn LibraryStore>>,
    metadata_service: Option<Arc<dyn MetadataService>>,
    playback_service: Option<Box<dyn PlaybackService>>,
    playback_queue: PlaybackQueue,
    // Tracks how much of the currently playing track the listener has
    // actually heard, so we can decide whether to register a play or a
    // skip when the track changes. `None` whenever nothing is playing.
    pub(crate) playback_session: Option<PlaybackSession>,
    library_tracks: Vec<Track>,
    playlists: Vec<Playlist>,
    playlist_folders: Vec<PlaylistFolder>,
    smart_playlists: Vec<SmartPlaylist>,
    last_scan_library_path: Option<PathBuf>,
    last_scan_summary: Option<LibraryScanSummary>,
    last_library_import_summary: Option<LibraryImportSummary>,
    last_library_consolidation_summary: Option<LibraryConsolidationSummary>,
    background_task_status: BackgroundTaskStatus,
    library_scan_cancellation: Option<Arc<AtomicBool>>,
    library_import_cancellation: Option<Arc<AtomicBool>>,
    library_consolidation_cancellation: Option<Arc<AtomicBool>>,
    // Id of the persistent notification published while a given task
    // is running, so apply/fail can dismiss the exact entry they own.
    library_scan_notification_id: Option<NotificationId>,
    library_import_notification_id: Option<NotificationId>,
    library_consolidation_notification_id: Option<NotificationId>,
    metadata_writer: Option<metadata_writer::MetadataWriter>,
    metadata_write_result_sink: Option<async_channel::Sender<MetadataWriteResult>>,
    remote_metadata_service: Option<Arc<dyn RemoteMetadataService>>,
    artwork_fetcher: Option<artwork_fetcher::ArtworkFetcher>,
    artwork_fetch_result_sink: Option<async_channel::Sender<ArtworkFetchResult>>,
    analysis_scheduler: Option<analysis_scheduler::AnalysisScheduler>,
    analysis_progress_sink: Option<async_channel::Sender<analysis_scheduler::SchedulerProgress>>,
    analysis_notification_id: Option<NotificationId>,
    clock: Arc<dyn Clock>,
    notifications: NotificationCenter,
    // Fires after every push/dismiss/expire on `notifications`. Set by
    // the UI shell once during startup; the callback is responsible for
    // deferring its re-render (the runtime is mid-borrow when this
    // fires, so calling back into the runtime synchronously would
    // panic).
    notification_observer: Option<NotificationObserver>,
    // Fires whenever the runtime flips any track's `is_missing` flag
    // outside the scan path (e.g. lazy detection on a failed play, a
    // library-path change that re-stats every track). The UI shell
    // installs this once to drive its narrow per-row icon refresh —
    // see the design note on [`TrackAvailabilityObserver`].
    track_availability_observer: Option<TrackAvailabilityObserver>,
}

/// Callback fired after every mutation of [`NotificationCenter`]. Held
/// as a trait object so feature crates can plug GTK-specific dispatch
/// without coupling `app_runtime` to GTK.
pub type NotificationObserver = Box<dyn Fn()>;

/// Callback fired whenever the runtime flips at least one persisted
/// track's `is_missing` flag *outside* the bulk scan path. The
/// observer receives no payload — the UI is expected to re-read
/// [`ApplicationRuntime::library_tracks`] and patch its own row data
/// for the (typically narrow) set of tracks whose availability now
/// differs. Like [`NotificationObserver`], the runtime is mid-borrow
/// when this fires; observers must defer their work onto the main
/// loop (e.g. `glib::idle_add_local_once`).
pub type TrackAvailabilityObserver = Box<dyn Fn()>;

impl ApplicationRuntime {
    pub fn new() -> Self {
        Self {
            settings: UserSettings::default(),
            settings_store: None,
            library_store: None,
            metadata_service: None,
            playback_service: None,
            playback_queue: PlaybackQueue::default(),
            playback_session: None,
            library_tracks: Vec::new(),
            playlists: Vec::new(),
            playlist_folders: Vec::new(),
            smart_playlists: Vec::new(),
            last_scan_library_path: None,
            last_scan_summary: None,
            last_library_import_summary: None,
            last_library_consolidation_summary: None,
            background_task_status: BackgroundTaskStatus::Idle,
            library_scan_cancellation: None,
            library_import_cancellation: None,
            library_consolidation_cancellation: None,
            library_scan_notification_id: None,
            library_import_notification_id: None,
            library_consolidation_notification_id: None,
            metadata_writer: None,
            metadata_write_result_sink: None,
            remote_metadata_service: None,
            artwork_fetcher: None,
            artwork_fetch_result_sink: None,
            analysis_scheduler: None,
            analysis_progress_sink: None,
            analysis_notification_id: None,
            clock: Arc::new(SystemClock),
            notifications: NotificationCenter::new(),
            notification_observer: None,
            track_availability_observer: None,
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
            playback_session: None,
            library_tracks: Vec::new(),
            playlists: Vec::new(),
            playlist_folders: Vec::new(),
            smart_playlists: Vec::new(),
            last_scan_library_path: None,
            last_scan_summary: None,
            last_library_import_summary: None,
            last_library_consolidation_summary: None,
            background_task_status: BackgroundTaskStatus::Idle,
            library_scan_cancellation: None,
            library_import_cancellation: None,
            library_consolidation_cancellation: None,
            library_scan_notification_id: None,
            library_import_notification_id: None,
            library_consolidation_notification_id: None,
            metadata_writer: None,
            metadata_write_result_sink: None,
            remote_metadata_service: None,
            artwork_fetcher: None,
            artwork_fetch_result_sink: None,
            analysis_scheduler: None,
            analysis_progress_sink: None,
            analysis_notification_id: None,
            clock: Arc::new(SystemClock),
            notifications: NotificationCenter::new(),
            notification_observer: None,
            track_availability_observer: None,
        })
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
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
        if let Some(library_path) = self.settings.library_path() {
            managed_library::recover_library_consolidation_journal(
                library_path,
                library_store.as_ref(),
            )?;
        }
        self.library_tracks = library_scan::load_library_tracks(library_store.as_ref())?;
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

    /// Starts the async metadata writer, using the same `MetadataService`
    /// the runtime already holds. The writer owns a dedicated worker
    /// thread that drains tag writes off the GTK main loop.
    ///
    /// Pair with [`Self::set_metadata_write_result_sink`] so failures can
    /// be reported to the user. Without a sink, the writer still
    /// processes jobs but completions are silently consumed.
    pub fn start_metadata_writer(&mut self) -> ApplicationRuntimeResult<()> {
        let metadata_service = self
            .metadata_service
            .clone()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;
        self.metadata_writer = Some(metadata_writer::MetadataWriter::start(metadata_service));
        Ok(())
    }

    /// Registers the sink that receives per-write completions. Senders
    /// of a closed channel are silently dropped — the worker keeps
    /// processing jobs regardless. UI layer typically holds the
    /// matching receiver and consumes from the GTK main loop.
    pub fn set_metadata_write_result_sink(
        &mut self,
        sink: async_channel::Sender<MetadataWriteResult>,
    ) {
        self.metadata_write_result_sink = Some(sink);
    }

    /// Drains pending tag writes and joins the worker thread. Call from
    /// the app's shutdown path so an in-flight rating click is not lost
    /// when the window closes.
    pub fn shutdown_metadata_writer(&mut self) {
        if let Some(writer) = self.metadata_writer.take() {
            writer.shutdown();
        }
    }

    /// Install a networked metadata service. The service is consumed
    /// by the artwork fetcher (and, in time, by tag-backfill and
    /// fingerprint-identification pipelines). Calling this without
    /// also calling [`Self::start_artwork_fetcher`] simply stores
    /// the handle; submissions return
    /// [`ApplicationRuntimeError::ArtworkFetchingUnavailable`] until
    /// the worker is started.
    pub fn set_remote_metadata_service(&mut self, service: Arc<dyn RemoteMetadataService>) {
        self.remote_metadata_service = Some(service);
    }

    pub fn remote_metadata_service(&self) -> Option<Arc<dyn RemoteMetadataService>> {
        self.remote_metadata_service.clone()
    }

    /// Spin up the artwork-fetcher worker against the previously
    /// installed remote metadata service. Returns
    /// [`ApplicationRuntimeError::ArtworkFetchingUnavailable`] if no
    /// service has been set — that state is legitimate (a build
    /// without a remote service still has to start).
    pub fn start_artwork_fetcher(&mut self) -> ApplicationRuntimeResult<()> {
        let service = self
            .remote_metadata_service
            .clone()
            .ok_or(ApplicationRuntimeError::ArtworkFetchingUnavailable)?;
        self.artwork_fetcher = Some(artwork_fetcher::ArtworkFetcher::start(service));
        Ok(())
    }

    /// Register the sink that receives per-fetch outcomes. The UI
    /// layer typically holds the matching receiver and consumes from
    /// the GTK main loop, dispatching `SetArtwork` for successful
    /// outcomes and surfacing a status-bar message otherwise.
    pub fn set_artwork_fetch_result_sink(
        &mut self,
        sink: async_channel::Sender<ArtworkFetchResult>,
    ) {
        self.artwork_fetch_result_sink = Some(sink);
    }

    /// Drop the fetcher's sender and join its worker. Safe at app
    /// shutdown; idempotent across multiple calls.
    pub fn shutdown_artwork_fetcher(&mut self) {
        if let Some(fetcher) = self.artwork_fetcher.take() {
            fetcher.shutdown();
        }
    }

    /// Spin up the background analysis scheduler against the previously
    /// installed [`LibraryStore`]. The scheduler observes the current
    /// `AnalysisSettings` (`bpm` / `key` / `waveform` tickboxes) and
    /// the library root; toggling either through the settings command
    /// path automatically propagates to the worker. Returns
    /// [`ApplicationRuntimeError::LibraryServicesUnavailable`] if no
    /// library store has been set yet.
    pub fn start_analysis_scheduler(&mut self) -> ApplicationRuntimeResult<()> {
        let library_store = self
            .library_store
            .clone()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;

        // Production analyzer: the real DSP. Tests substitute a stub via
        // analysis_scheduler::AnalysisScheduler::start directly.
        let analyzer: analysis_scheduler::AnalyzerFn = Arc::new(sustain_analysis::analyze);
        let clock: analysis_scheduler::UnixClockFn = Arc::new(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        });

        // Marshal progress events from the worker thread onto whatever
        // sink the UI installed via `set_analysis_progress_sink`. If no
        // sink is installed, drop silently — the worker keeps doing
        // useful work even if the UI does not surface its progress.
        let progress_sink = self.analysis_progress_sink.clone();
        let progress: analysis_scheduler::ProgressSink = Arc::new(move |progress| {
            if let Some(sender) = &progress_sink {
                // try_send so a slow consumer cannot back-pressure the
                // worker thread. Dropping a Tick is fine — the next
                // Tick or Idle carries the same running totals.
                let _ = sender.try_send(progress);
            }
        });

        let scheduler = analysis_scheduler::AnalysisScheduler::start(
            analysis_scheduler::AnalysisSchedulerConfig {
                analyzer,
                progress,
                clock,
                library_store,
                initial_settings: self.settings.analysis,
                library_path: self.settings.library.path.clone(),
                analyzer_version: sustain_analysis::ANALYZER_VERSION,
                analysis_options: sustain_analysis::AnalysisOptions::default(),
            },
        );
        self.analysis_scheduler = Some(scheduler);
        Ok(())
    }

    /// Register the async-channel sink that receives
    /// [`AnalysisProgress`] events. The UI typically holds the
    /// matching receiver and forwards each event into
    /// [`Self::apply_analysis_progress`] from the GTK main loop.
    pub fn set_analysis_progress_sink(
        &mut self,
        sink: async_channel::Sender<analysis_scheduler::SchedulerProgress>,
    ) {
        self.analysis_progress_sink = Some(sink);
    }

    /// Drop the scheduler's sender and join its worker. Safe at app
    /// shutdown; idempotent across calls.
    pub fn shutdown_analysis_scheduler(&mut self) {
        if let Some(id) = self.analysis_notification_id.take() {
            self.dismiss_notification(id);
        }
        if let Some(scheduler) = self.analysis_scheduler.take() {
            scheduler.shutdown();
        }
    }

    /// Apply an [`AnalysisProgress`] event to the notification center.
    /// Called from the UI loop after receiving an event from the sink
    /// installed by [`Self::set_analysis_progress_sink`] — the
    /// scheduler runs on its own thread, so the runtime cannot push
    /// notifications synchronously from inside the worker.
    pub fn apply_analysis_progress(&mut self, progress: analysis_scheduler::SchedulerProgress) {
        match progress {
            analysis_scheduler::SchedulerProgress::Tick {
                completed,
                failed: _,
                remaining,
            } => {
                let body = notifications::analysis_background_running_text(completed, remaining);
                if let Some(existing) = self.analysis_notification_id {
                    self.update_notification_body(existing, body);
                } else {
                    let id = self.push_persistent_notification(
                        NotificationCategory::AnalysisBackground,
                        NotificationSeverity::Info,
                        body,
                        false,
                    );
                    self.analysis_notification_id = Some(id);
                }
            }
            analysis_scheduler::SchedulerProgress::Idle { completed, failed } => {
                if let Some(id) = self.analysis_notification_id.take() {
                    self.dismiss_notification(id);
                }
                // Emit an ephemeral summary only when there is something
                // to summarise — Idle also fires on initial start-up
                // with capabilities disabled, and we do not want a
                // ghost "Analyzed 0 tracks" toast every launch.
                if completed > 0 || failed > 0 {
                    self.push_ephemeral_notification(
                        NotificationCategory::AnalysisBackground,
                        NotificationSeverity::Info,
                        notifications::analysis_background_outcome_text(completed, failed),
                    );
                }
            }
        }
    }

    pub(crate) fn analysis_scheduler(&self) -> Option<&analysis_scheduler::AnalysisScheduler> {
        self.analysis_scheduler.as_ref()
    }

    pub(crate) fn artwork_fetcher(&self) -> Option<&artwork_fetcher::ArtworkFetcher> {
        self.artwork_fetcher.as_ref()
    }

    pub(crate) fn artwork_fetch_result_sink(
        &self,
    ) -> Option<async_channel::Sender<ArtworkFetchResult>> {
        self.artwork_fetch_result_sink.clone()
    }

    pub(crate) fn metadata_writer(&self) -> Option<&metadata_writer::MetadataWriter> {
        self.metadata_writer.as_ref()
    }

    pub(crate) fn metadata_write_result_sink(
        &self,
    ) -> Option<async_channel::Sender<MetadataWriteResult>> {
        self.metadata_write_result_sink.clone()
    }

    pub fn settings(&self) -> &UserSettings {
        &self.settings
    }

    pub fn metadata_service(&self) -> Option<Arc<dyn MetadataService>> {
        self.metadata_service.clone()
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

    pub fn last_library_import_summary(&self) -> Option<&LibraryImportSummary> {
        self.last_library_import_summary.as_ref()
    }

    pub fn last_library_consolidation_summary(&self) -> Option<&LibraryConsolidationSummary> {
        self.last_library_consolidation_summary.as_ref()
    }

    pub fn background_task_status(&self) -> &BackgroundTaskStatus {
        &self.background_task_status
    }

    pub fn request_library_consolidation_cancellation(&self) {
        if let Some(cancellation_requested) = &self.library_consolidation_cancellation {
            cancellation_requested.store(true, Ordering::SeqCst);
        }
    }

    pub fn request_library_scan_cancellation(&self) {
        if let Some(cancellation_requested) = &self.library_scan_cancellation {
            cancellation_requested.store(true, Ordering::SeqCst);
        }
    }

    pub fn request_library_import_cancellation(&self) {
        if let Some(cancellation_requested) = &self.library_import_cancellation {
            cancellation_requested.store(true, Ordering::SeqCst);
        }
    }

    // Dispatches a cancellation request to whichever background task is
    // currently running. Background tasks are mutually exclusive (see
    // `BackgroundTaskStatus::is_running`), so at most one of these
    // tokens is live at any moment. Idempotent: calling this with no
    // task running is a no-op, as is calling it twice while one is
    // winding down. Also pokes the notification observer so the lane
    // can repaint the running notification's label as "Cancelling..."
    // without waiting for the next worker poll tick.
    pub fn request_background_task_cancellation(&self) {
        self.request_library_scan_cancellation();
        self.request_library_import_cancellation();
        self.request_library_consolidation_cancellation();
        self.notify_notification_observer();
    }

    // True while a cancellation request has been issued but the
    // background task has not yet reported back with its final status.
    // UI surfaces use this to flip the status label to "Cancelling..."
    // so the user sees their click was received.
    pub fn background_task_cancellation_requested(&self) -> bool {
        fn flag_set(token: Option<&Arc<AtomicBool>>) -> bool {
            token
                .map(|token| token.load(Ordering::SeqCst))
                .unwrap_or(false)
        }
        flag_set(self.library_scan_cancellation.as_ref())
            || flag_set(self.library_import_cancellation.as_ref())
            || flag_set(self.library_consolidation_cancellation.as_ref())
    }

    /// Read-only view onto the notification surface for renderers.
    pub fn notifications(&self) -> &NotificationCenter {
        &self.notifications
    }

    /// Install the observer that the runtime fires after every
    /// notification mutation. The observer must not synchronously
    /// reach back into the runtime — it runs while a mutable borrow
    /// is held — so implementations should defer their work onto the
    /// main loop (e.g. `glib::idle_add_local_once`).
    pub fn set_notification_observer(&mut self, observer: NotificationObserver) {
        self.notification_observer = Some(observer);
    }

    pub fn push_persistent_notification(
        &mut self,
        category: NotificationCategory,
        severity: NotificationSeverity,
        body: String,
        cancellable: bool,
    ) -> NotificationId {
        let id = self
            .notifications
            .push_persistent(category, severity, body, cancellable);
        self.notify_notification_observer();
        id
    }

    pub fn push_ephemeral_notification(
        &mut self,
        category: NotificationCategory,
        severity: NotificationSeverity,
        body: String,
    ) -> NotificationId {
        let id = self.notifications.push_ephemeral(category, severity, body);
        self.notify_notification_observer();
        id
    }

    pub fn dismiss_notification(&mut self, id: NotificationId) {
        self.notifications.dismiss(id);
        self.notify_notification_observer();
    }

    /// Replace the body text of an existing notification in place,
    /// firing the observer so the lane re-renders without animating
    /// through a dismiss+repush.
    pub fn update_notification_body(&mut self, id: NotificationId, body: String) {
        if self.notifications.update_body(id, body) {
            self.notify_notification_observer();
        }
    }

    /// Pop the currently-displayed ephemeral. Called by the widget
    /// when its display timer has elapsed; the widget then renders the
    /// next head (or falls back to the persistent stack).
    pub fn expire_current_ephemeral_notification(&mut self) -> Option<Notification> {
        let expired = self.notifications.expire_current_ephemeral();
        if expired.is_some() {
            self.notify_notification_observer();
        }
        expired
    }

    fn notify_notification_observer(&self) {
        if let Some(observer) = &self.notification_observer {
            observer();
        }
    }

    /// Install the observer fired after every lazy availability flip
    /// (failed-play detection, library-path re-stat). The observer
    /// must not synchronously re-enter the runtime — defer onto the
    /// main loop, same contract as [`Self::set_notification_observer`].
    pub fn set_track_availability_observer(&mut self, observer: TrackAvailabilityObserver) {
        self.track_availability_observer = Some(observer);
    }

    pub(crate) fn notify_track_availability_observer(&self) {
        if let Some(observer) = &self.track_availability_observer {
            observer();
        }
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

    pub fn playback_queue_next_track_id(&self) -> Option<TrackId> {
        self.playback_queue.next_track_id()
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

    /// Persist the playback volume preference. The audio path is updated
    /// separately via `PlaybackCommand::SetVolume` (which the UI dispatches
    /// immediately for responsive feedback) — this method only writes the
    /// user setting so the choice survives a restart.
    pub fn save_playback_volume(&mut self, volume: VolumePercent) -> ApplicationRuntimeResult<()> {
        if self.settings.playback.volume == volume {
            return Ok(());
        }
        self.settings.playback.volume = volume;
        if let Some(store) = self.settings_store.as_ref() {
            store
                .save_settings(self.settings.clone())
                .map_err(|_| ApplicationRuntimeError::SettingsSaveFailed)?;
        }
        Ok(())
    }

    pub fn save_ui_settings(&mut self, ui: UiSettings) -> ApplicationRuntimeResult<()> {
        if self.settings.ui == ui {
            return Ok(());
        }
        self.settings.ui = ui;
        if let Some(store) = self.settings_store.as_ref() {
            store
                .save_settings(self.settings.clone())
                .map_err(|_| ApplicationRuntimeError::SettingsSaveFailed)?;
        }
        Ok(())
    }

    pub fn load_track_column_layout(
        &self,
        scope: TrackColumnLayoutScope,
    ) -> ApplicationRuntimeResult<Option<TrackColumnLayout>> {
        let Some(store) = self.library_store.as_deref() else {
            return Ok(None);
        };
        store
            .load_track_column_layout(scope)
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)
    }

    pub fn save_track_column_layout(
        &self,
        scope: TrackColumnLayoutScope,
        layout: &TrackColumnLayout,
    ) -> ApplicationRuntimeResult<()> {
        let Some(store) = self.library_store.as_deref() else {
            return Err(ApplicationRuntimeError::LibraryStoreFailed);
        };
        store
            .save_track_column_layout(scope, layout)
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)
    }

    pub fn smart_playlist_matching_tracks(
        &self,
        smart_playlist_id: SmartPlaylistId,
    ) -> Vec<&Track> {
        let Some(smart_playlist) = self
            .smart_playlists
            .iter()
            .find(|smart_playlist| smart_playlist.id == smart_playlist_id)
        else {
            return Vec::new();
        };
        matching_tracks(
            &self.library_tracks,
            &smart_playlist.rules,
            self.clock.now(),
        )
    }
}

impl Default for ApplicationRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::panic, reason = "test failures use panic! to report context")]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::{Arc, Mutex, MutexGuard},
    };

    use sustain_domain::{
        ApplicationCommand, Clock, FieldChange, LibraryManagementMode, PlayStatistics,
        PlaybackCommand, PlaybackOptions, PlaybackState, Playlist, PlaylistFolderId, PlaylistId,
        PlaylistItem, Rating, RepeatMode, SmartPlaylist, SmartPlaylistDateField, SmartPlaylistId,
        SmartPlaylistMatchKind, SmartPlaylistRule, SmartPlaylistRuleSet, SmartPlaylistTextField,
        SmartPlaylistTextOperator, Track, TrackId, TrackLocation, TrackMetadata, UiSettings,
        UiViewMode, UserSettings, VolumePercent,
    };
    use sustain_library_store::{InMemoryLibraryStore, LibraryStore, StoreResult};
    use sustain_metadata::{MetadataChange, MetadataError, MetadataResult};
    use sustain_playback::NullPlaybackService;
    use sustain_settings::{SettingsError, SettingsResult, SettingsStore};

    use super::{
        ApplicationRuntime, ApplicationRuntimeError, LibraryConsolidationSummary,
        LibraryScanSummary, MetadataService, PlaybackQueueRequest, run_library_consolidation_task,
        run_library_scan_task,
    };

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
                ApplicationCommand::Playback(PlaybackCommand::PlayTrack {
                    track_id,
                    queue: PlaybackQueueRequest::Library,
                }),
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
                ApplicationCommand::Playback(PlaybackCommand::SetShuffleEnabled(false)),
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
                ApplicationCommand::MovePlaylistEntries {
                    playlist_id,
                    track_ids: vec![track_id],
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
                    change: Box::new(metadata_change.clone()),
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
            (
                ApplicationCommand::FetchArtwork { track_id },
                Err(ApplicationRuntimeError::TrackUnavailable),
            ),
            (
                ApplicationCommand::AddExternalLibraryItems {
                    paths: vec![PathBuf::from("/music/track.flac")],
                },
                Err(ApplicationRuntimeError::LibraryServicesUnavailable),
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
        assert_eq!(runtime.library_tracks()[0].content_hash, None);
        assert_eq!(
            runtime
                .last_scan_summary()
                .map(|summary| summary.scanned_tracks),
            Some(1)
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn cancelled_scan_preserves_existing_tracks_without_marking_them_missing() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");

        let store = Arc::new(InMemoryLibraryStore::new());
        let existing_track = test_track(track_id(1), "leftover.mp3");
        assert_eq!(store.save_track(existing_track.clone()), Ok(()));

        let metadata_service = Arc::new(TestMetadataService);
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store, metadata_service)
            .expect("library services initialize");

        // Trip the cancellation flag *before* the worker observes it.
        // That is the worst case for the missing-track sweep: the
        // walker aborts on its first iteration without indexing the
        // empty library, and we must not interpret the unwalked
        // existing track as missing.
        let task = runtime
            .prepare_library_scan(root.clone())
            .expect("prepare scan");
        runtime.request_library_scan_cancellation();
        let result = run_library_scan_task(task).expect("scan finishes cleanly");
        runtime.apply_library_scan_result(result);

        let summary = runtime.last_scan_summary().expect("scan summary present");
        assert!(summary.cancelled, "cancellation flag must propagate");
        assert_eq!(
            summary.missing_tracks, 0,
            "a partial scan must not mark unwalked tracks as missing"
        );
        let tracks = runtime.library_tracks();
        assert_eq!(tracks.len(), 1, "the pre-existing track must be preserved");
        assert_eq!(tracks[0].id, existing_track.id);

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
                cancelled: false,
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

        // Startup no longer re-polls every track's on-disk existence (iTunes-
        // like lazy availability), so the loaded track keeps the persisted
        // Available flag here. The scan below is what reconciles availability
        // against the new library root.
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
                cancelled: false,
            })
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn managed_import_copies_external_files_into_planned_library_path() {
        let library_root = unique_test_directory();
        let external_root = unique_test_directory();
        std::fs::create_dir_all(&library_root).expect("create library root");
        std::fs::create_dir_all(&external_root).expect("create external root");
        let source_path = external_root.join("source.flac");
        std::fs::write(&source_path, b"audio bytes").expect("write external source");
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut settings = UserSettings::with_library_path(Some(library_root.clone()));
        settings.library.management_mode = LibraryManagementMode::CopyAddedFilesIntoLibrary;
        let mut runtime =
            ApplicationRuntime::with_settings_store(Box::new(TestSettingsStore::new(settings)))
                .expect("load settings")
                .with_library_services(store.clone(), Arc::new(TestMetadataService))
                .expect("library services initialize");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::AddExternalLibraryItems {
                paths: vec![source_path.clone()]
            }),
            Ok(())
        );

        let expected_destination = library_root
            .join("Unknown Artist")
            .join("Unknown Album")
            .join("Track.flac");
        assert_eq!(
            std::fs::read(&expected_destination).expect("copied file exists"),
            b"audio bytes"
        );
        assert_eq!(
            std::fs::read(&source_path).expect("source remains untouched"),
            b"audio bytes"
        );
        assert_eq!(
            runtime.last_library_import_summary(),
            Some(&super::LibraryImportSummary {
                discovered_files: 1,
                imported_tracks: 1,
                duplicate_files: 0,
                cancelled: false,
            })
        );

        let tracks = runtime.library_tracks();
        assert_eq!(tracks.len(), 1);
        assert_eq!(
            tracks[0].location.relative_path.as_path(),
            std::path::Path::new("Unknown Artist/Unknown Album/Track.flac")
        );
        assert!(tracks[0].content_hash.is_some());
        assert_eq!(tracks[0].rating, Rating::new(3).expect("valid rating"));
        assert_eq!(store.tracks().expect("store tracks"), tracks);

        std::fs::remove_dir_all(library_root).expect("remove library root");
        std::fs::remove_dir_all(external_root).expect("remove external root");
    }

    #[test]
    fn managed_import_skips_duplicate_content_hashes_in_same_batch() {
        let library_root = unique_test_directory();
        let external_root = unique_test_directory();
        std::fs::create_dir_all(&library_root).expect("create library root");
        std::fs::create_dir_all(&external_root).expect("create external root");
        let first_source = external_root.join("first.flac");
        let second_source = external_root.join("second.flac");
        std::fs::write(&first_source, b"same audio").expect("write first source");
        std::fs::write(&second_source, b"same audio").expect("write second source");
        let mut settings = UserSettings::with_library_path(Some(library_root.clone()));
        settings.library.management_mode = LibraryManagementMode::CopyAddedFilesIntoLibrary;
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut runtime =
            ApplicationRuntime::with_settings_store(Box::new(TestSettingsStore::new(settings)))
                .expect("load settings")
                .with_library_services(store, Arc::new(TestMetadataService))
                .expect("library services initialize");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::AddExternalLibraryItems {
                paths: vec![first_source, second_source]
            }),
            Ok(())
        );

        assert_eq!(runtime.library_tracks().len(), 1);
        assert_eq!(
            runtime.last_library_import_summary(),
            Some(&super::LibraryImportSummary {
                discovered_files: 2,
                imported_tracks: 1,
                duplicate_files: 1,
                cancelled: false,
            })
        );

        std::fs::remove_dir_all(library_root).expect("remove library root");
        std::fs::remove_dir_all(external_root).expect("remove external root");
    }

    #[test]
    fn managed_import_lazily_hashes_same_size_existing_tracks_for_duplicates() {
        let library_root = unique_test_directory();
        let external_root = unique_test_directory();
        std::fs::create_dir_all(&library_root).expect("create library root");
        std::fs::create_dir_all(&external_root).expect("create external root");
        let existing_path = library_root.join("existing.flac");
        let source_path = external_root.join("source.flac");
        std::fs::write(&existing_path, b"same audio").expect("write existing track");
        std::fs::write(&source_path, b"same audio").expect("write external source");

        let mut settings = UserSettings::with_library_path(Some(library_root.clone()));
        settings.library.management_mode = LibraryManagementMode::CopyAddedFilesIntoLibrary;
        let store = Arc::new(InMemoryLibraryStore::new());
        let existing_track = test_track(track_id(7), "existing.flac");
        assert_eq!(store.save_track(existing_track.clone()), Ok(()));
        let mut runtime =
            ApplicationRuntime::with_settings_store(Box::new(TestSettingsStore::new(settings)))
                .expect("load settings")
                .with_library_services(store, Arc::new(TestMetadataService))
                .expect("library services initialize");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::AddExternalLibraryItems {
                paths: vec![source_path]
            }),
            Ok(())
        );

        assert_eq!(runtime.library_tracks(), &[existing_track]);
        assert_eq!(
            runtime.last_library_import_summary(),
            Some(&super::LibraryImportSummary {
                discovered_files: 1,
                imported_tracks: 0,
                duplicate_files: 1,
                cancelled: false,
            })
        );

        std::fs::remove_dir_all(library_root).expect("remove library root");
        std::fs::remove_dir_all(external_root).expect("remove external root");
    }

    #[test]
    fn unmanaged_external_import_indexes_library_files_in_place() {
        let library_root = unique_test_directory();
        std::fs::create_dir_all(&library_root).expect("create library root");
        let source_path = library_root.join("source.flac");
        std::fs::write(&source_path, b"audio bytes").expect("write source");
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(library_root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store.clone(), Arc::new(TestMetadataService))
        .expect("library services initialize");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::AddExternalLibraryItems {
                paths: vec![source_path.clone()]
            }),
            Ok(())
        );

        let tracks = runtime.library_tracks();
        assert_eq!(tracks.len(), 1);
        assert_eq!(
            tracks[0].location.relative_path.as_path(),
            Path::new("source.flac")
        );
        assert_eq!(tracks[0].content_hash, None);
        assert_eq!(store.tracks().expect("store tracks"), tracks);
        assert_eq!(
            runtime.last_library_import_summary(),
            Some(&super::LibraryImportSummary {
                discovered_files: 1,
                imported_tracks: 1,
                duplicate_files: 0,
                cancelled: false,
            })
        );

        std::fs::remove_dir_all(library_root).expect("remove library root");
    }

    #[test]
    fn unmanaged_external_import_rejects_files_outside_library_path() {
        let library_root = unique_test_directory();
        let external_root = unique_test_directory();
        std::fs::create_dir_all(&library_root).expect("create library root");
        std::fs::create_dir_all(&external_root).expect("create external root");
        let source_path = external_root.join("source.flac");
        std::fs::write(&source_path, b"audio bytes").expect("write source");
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(library_root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store.clone(), Arc::new(TestMetadataService))
        .expect("library services initialize");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::AddExternalLibraryItems {
                paths: vec![source_path]
            }),
            Err(ApplicationRuntimeError::LibraryImportFailed)
        );
        assert_eq!(runtime.library_tracks(), &[]);
        assert_eq!(store.tracks(), Ok(Vec::new()));

        std::fs::remove_dir_all(library_root).expect("remove library root");
        std::fs::remove_dir_all(external_root).expect("remove external root");
    }

    #[test]
    fn managed_consolidation_moves_existing_tracks_to_planned_paths() {
        let library_root = unique_test_directory();
        std::fs::create_dir_all(&library_root).expect("create library root");
        let source_path = library_root.join("loose.flac");
        std::fs::write(&source_path, b"audio bytes").expect("write existing file");

        let track_id = track_id(21);
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut track = test_track(track_id, "loose.flac");
        track.metadata.artist = Some("Artist".to_owned());
        track.metadata.album = Some("Album".to_owned());
        track.metadata.title = Some("Song".to_owned());
        track.metadata.track_number = Some(1);
        track.rating = Rating::new(5).expect("valid rating");
        track.statistics.play_count = 9;
        assert_eq!(store.save_track(track), Ok(()));

        let mut settings = UserSettings::with_library_path(Some(library_root.clone()));
        settings.library.management_mode = LibraryManagementMode::CopyAddedFilesIntoLibrary;
        let mut runtime =
            ApplicationRuntime::with_settings_store(Box::new(TestSettingsStore::new(settings)))
                .expect("load settings")
                .with_library_services(store.clone(), Arc::new(TestMetadataService))
                .expect("library services initialize");

        let task = runtime
            .prepare_library_consolidation()
            .expect("prepare consolidation");
        let result = run_library_consolidation_task(task).expect("run consolidation");
        runtime.apply_library_consolidation_result(result);

        let destination_path = library_root.join("Artist/Album/01 Song.flac");
        assert!(!source_path.exists());
        assert_eq!(
            std::fs::read(&destination_path).expect("destination exists"),
            b"audio bytes"
        );
        assert_eq!(
            runtime.last_library_consolidation_summary(),
            Some(&LibraryConsolidationSummary {
                planned_tracks: 1,
                moved_tracks: 1,
                already_organized_tracks: 0,
                missing_tracks: 0,
                cancelled: false,
            })
        );

        let runtime_track = runtime
            .library_tracks()
            .iter()
            .find(|track| track.id == track_id)
            .expect("runtime track exists");
        assert_eq!(
            runtime_track.location.relative_path.as_path(),
            Path::new("Artist/Album/01 Song.flac")
        );
        assert_eq!(runtime_track.rating, Rating::new(5).expect("valid rating"));
        assert_eq!(runtime_track.statistics.play_count, 9);
        assert_eq!(
            store
                .track(track_id)
                .expect("load stored track")
                .map(|track| track.location.relative_path.to_path_buf()),
            Some(PathBuf::from("Artist/Album/01 Song.flac"))
        );

        std::fs::remove_dir_all(library_root).expect("remove library root");
    }

    #[test]
    fn disabling_managed_mode_requests_consolidation_cancellation() {
        let library_root = unique_test_directory();
        std::fs::create_dir_all(&library_root).expect("create library root");
        let source_path = library_root.join("loose.flac");
        std::fs::write(&source_path, b"audio bytes").expect("write existing file");

        let store = Arc::new(InMemoryLibraryStore::new());
        let mut track = test_track(track_id(22), "loose.flac");
        track.metadata.title = Some("Song".to_owned());
        assert_eq!(store.save_track(track), Ok(()));

        let mut settings = UserSettings::with_library_path(Some(library_root.clone()));
        settings.library.management_mode = LibraryManagementMode::CopyAddedFilesIntoLibrary;
        let mut runtime =
            ApplicationRuntime::with_settings_store(Box::new(TestSettingsStore::new(settings)))
                .expect("load settings")
                .with_library_services(store.clone(), Arc::new(TestMetadataService))
                .expect("library services initialize");

        let task = runtime
            .prepare_library_consolidation()
            .expect("prepare consolidation");
        assert_eq!(
            runtime.handle_command(ApplicationCommand::ScanLibrary {
                library_path: library_root.clone()
            }),
            Err(ApplicationRuntimeError::BackgroundTaskRunning)
        );

        let mut updated_settings = runtime.settings().clone();
        updated_settings.library.management_mode = LibraryManagementMode::ReferenceFilesInPlace;
        assert_eq!(
            runtime.handle_command(ApplicationCommand::UpdateSettings(updated_settings)),
            Ok(())
        );

        let result = run_library_consolidation_task(task).expect("run cancelled consolidation");
        runtime.apply_library_consolidation_result(result);

        assert!(source_path.exists());
        assert!(
            !library_root
                .join("Unknown Artist/Unknown Album/Song.flac")
                .exists()
        );
        assert_eq!(
            runtime.last_library_consolidation_summary(),
            Some(&LibraryConsolidationSummary {
                planned_tracks: 1,
                moved_tracks: 0,
                already_organized_tracks: 0,
                missing_tracks: 0,
                cancelled: true,
            })
        );
        assert_eq!(
            runtime.settings().library.management_mode,
            LibraryManagementMode::ReferenceFilesInPlace
        );

        std::fs::remove_dir_all(library_root).expect("remove library root");
    }

    #[test]
    fn consolidation_journal_recovery_retargets_moved_tracks_on_startup() {
        let library_root = unique_test_directory();
        std::fs::create_dir_all(library_root.join("Artist/Album"))
            .expect("create destination directory");
        let destination_path = library_root.join("Artist/Album/01 Song.flac");
        std::fs::write(&destination_path, b"audio bytes").expect("write moved file");
        std::fs::write(
            library_root.join(".sustain-consolidation-journal"),
            format!(
                "# sustain managed library consolidation journal v1\nmove\t23\t{}\t{}\n",
                hex_path("loose.flac"),
                hex_path("Artist/Album/01 Song.flac")
            ),
        )
        .expect("write journal");

        let store = Arc::new(InMemoryLibraryStore::new());
        let track_id = track_id(23);
        assert_eq!(store.save_track(test_track(track_id, "loose.flac")), Ok(()));
        let mut settings = UserSettings::with_library_path(Some(library_root.clone()));
        settings.library.management_mode = LibraryManagementMode::CopyAddedFilesIntoLibrary;

        let runtime =
            ApplicationRuntime::with_settings_store(Box::new(TestSettingsStore::new(settings)))
                .expect("load settings")
                .with_library_services(store.clone(), Arc::new(TestMetadataService))
                .expect("library services initialize");

        assert_eq!(
            runtime.library_tracks()[0].location.relative_path.as_path(),
            Path::new("Artist/Album/01 Song.flac")
        );
        assert!(!library_root.join(".sustain-consolidation-journal").exists());
        assert_eq!(
            store
                .track(track_id)
                .expect("load recovered track")
                .map(|track| track.location.relative_path.to_path_buf()),
            Some(PathBuf::from("Artist/Album/01 Song.flac"))
        );

        std::fs::remove_dir_all(library_root).expect("remove library root");
    }

    #[test]
    fn update_settings_does_not_re_stat_existing_tracks_when_path_is_unchanged() {
        // UpdateSettings re-stats tracks ONLY when the user changes
        // `library.path` (see
        // `update_settings_re_stats_existing_tracks_when_library_path_changes`).
        // Every other settings mutation — management-mode toggle,
        // playback volume, anything stored on `UserSettings` — must
        // stay free of stat() syscalls so toggling a Preferences
        // checkbox on a 10k library does not freeze the UI thread.
        let library_root = unique_test_directory();
        std::fs::create_dir_all(&library_root).expect("create test library");
        let track_path = library_root.join("track.flac");
        std::fs::write(&track_path, b"audio bytes").expect("write track");

        let track_id = track_id(7);
        let store = Arc::new(InMemoryLibraryStore::new());
        assert_eq!(store.save_track(test_track(track_id, "track.flac")), Ok(()));

        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(library_root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store, Arc::new(TestMetadataService))
        .expect("library services initialize");

        assert!(!runtime.library_tracks()[0].location.is_missing());

        // Remove the file behind the runtime's back, then dispatch
        // UpdateSettings. The track must keep its persisted
        // Available flag — UpdateSettings has no business
        // discovering missing files.
        std::fs::remove_file(&track_path).expect("remove track from disk");
        let settings = runtime.settings().clone();
        assert_eq!(
            runtime.handle_command(ApplicationCommand::UpdateSettings(settings)),
            Ok(())
        );
        assert!(!runtime.library_tracks()[0].location.is_missing());

        std::fs::remove_dir_all(library_root).expect("remove test library");
    }

    #[test]
    fn update_settings_re_stats_existing_tracks_when_library_path_changes() {
        // A library-path change is structural reconciliation: every
        // persisted track must be re-stat'd against the new root and
        // its availability flag flushed to SQLite, so the missing-file
        // indicator lights up the moment the user confirms the new
        // path instead of waiting for the next scan.
        let old_root = unique_test_directory();
        let new_root = unique_test_directory();
        std::fs::create_dir_all(&old_root).expect("create old library root");
        std::fs::create_dir_all(&new_root).expect("create new library root");
        std::fs::write(old_root.join("present.flac"), b"audio").expect("write present file");
        std::fs::write(new_root.join("present.flac"), b"audio").expect("mirror present file");
        // `vanished.flac` lives under the OLD root only. After the
        // path change, its persisted relative path resolves to a
        // non-existent file under `new_root`.
        std::fs::write(old_root.join("vanished.flac"), b"audio").expect("write vanished file");

        let store = Arc::new(InMemoryLibraryStore::new());
        let present_id = track_id(101);
        let vanished_id = track_id(102);
        assert_eq!(
            store.save_track(test_track(present_id, "present.flac")),
            Ok(())
        );
        assert_eq!(
            store.save_track(test_track(vanished_id, "vanished.flac")),
            Ok(())
        );

        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(old_root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store.clone(), Arc::new(TestMetadataService))
        .expect("library services initialize");

        for track in runtime.library_tracks() {
            assert!(!track.location.is_missing());
        }

        let new_settings = UserSettings::with_library_path(Some(new_root.clone()));
        assert_eq!(
            runtime.handle_command(ApplicationCommand::UpdateSettings(new_settings)),
            Ok(())
        );

        let present = runtime
            .library_tracks()
            .iter()
            .find(|track| track.id == present_id)
            .expect("present track survives path change");
        let vanished = runtime
            .library_tracks()
            .iter()
            .find(|track| track.id == vanished_id)
            .expect("vanished track survives path change");
        assert!(!present.location.is_missing(), "mirrored file resolves");
        assert!(
            vanished.location.is_missing(),
            "absent file flips to Missing"
        );

        // SQLite is the source of truth — the flag must be durable
        // across a reload, not merely flipped in memory.
        let reloaded = store
            .track(vanished_id)
            .expect("reload vanished")
            .expect("vanished row exists");
        assert!(reloaded.location.is_missing());

        std::fs::remove_dir_all(old_root).expect("remove old library root");
        std::fs::remove_dir_all(new_root).expect("remove new library root");
    }

    #[test]
    fn play_track_flips_is_missing_when_file_has_vanished() {
        // Lazy availability detection: clicking a track whose file is
        // no longer on disk must (a) return TrackUnavailable so the
        // UI shows the missing-file feedback, and (b) flip the
        // persisted `is_missing` flag so the table's warning
        // indicator lights up immediately and subsequent reads of
        // SQLite see the corrected state.
        let library_root = unique_test_directory();
        std::fs::create_dir_all(&library_root).expect("create library root");
        let track_path = library_root.join("ghost.flac");
        std::fs::write(&track_path, b"audio").expect("write track");

        let store = Arc::new(InMemoryLibraryStore::new());
        let id = track_id(33);
        assert_eq!(store.save_track(test_track(id, "ghost.flac")), Ok(()));

        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(library_root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store.clone(), Arc::new(TestMetadataService))
        .expect("library services initialize");

        assert!(!runtime.library_tracks()[0].location.is_missing());

        std::fs::remove_file(&track_path).expect("remove track");

        let outcome =
            runtime.handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayTrack {
                track_id: id,
                queue: sustain_domain::PlaybackQueueRequest::Library,
            }));
        assert_eq!(outcome, Err(ApplicationRuntimeError::TrackUnavailable));
        assert!(runtime.library_tracks()[0].location.is_missing());

        let reloaded = store
            .track(id)
            .expect("reload track")
            .expect("track row exists");
        assert!(reloaded.location.is_missing());

        std::fs::remove_dir_all(library_root).expect("remove library root");
    }

    #[test]
    fn play_track_recovers_availability_when_file_reappears() {
        // The `is_missing` flag is a cache of the last observed
        // availability, never a gate. Once a track has been flipped
        // to Missing, a subsequent play attempt must still re-stat
        // the path: if the file is back (rename undone, volume
        // remounted, restored from trash), the flag flips back to
        // Available and playback proceeds. Without this, a typo'd
        // rename would soft-brick the row forever.
        let library_root = unique_test_directory();
        std::fs::create_dir_all(&library_root).expect("create library root");
        let track_path = library_root.join("returning.flac");
        std::fs::write(&track_path, b"audio").expect("write track");

        let store = Arc::new(InMemoryLibraryStore::new());
        let id = track_id(34);
        assert_eq!(store.save_track(test_track(id, "returning.flac")), Ok(()));

        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(library_root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store.clone(), Arc::new(TestMetadataService))
        .expect("library services initialize")
        .with_playback_service(Box::new(NullPlaybackService::new()));

        // Step 1: remove the file, fail a play, observe the flag flip.
        std::fs::remove_file(&track_path).expect("remove track");
        let first =
            runtime.handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayTrack {
                track_id: id,
                queue: sustain_domain::PlaybackQueueRequest::Library,
            }));
        assert_eq!(first, Err(ApplicationRuntimeError::TrackUnavailable));
        assert!(runtime.library_tracks()[0].location.is_missing());

        // Step 2: put the file back. The flag still says Missing —
        // nothing else has touched the row.
        std::fs::write(&track_path, b"audio").expect("restore track");
        assert!(runtime.library_tracks()[0].location.is_missing());

        // Step 3: a fresh play succeeds because `play_track` re-stats
        // the resolved path; both the in-memory and persisted flags
        // flip back to Available.
        let second =
            runtime.handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayTrack {
                track_id: id,
                queue: sustain_domain::PlaybackQueueRequest::Library,
            }));
        assert_eq!(second, Ok(()));
        assert!(!runtime.library_tracks()[0].location.is_missing());

        let reloaded = store
            .track(id)
            .expect("reload track")
            .expect("track row exists");
        assert!(!reloaded.location.is_missing());

        std::fs::remove_dir_all(library_root).expect("remove library root");
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
    fn runtime_saves_ui_settings_with_settings_store() {
        let store = Box::new(TestSettingsStore::new(UserSettings::default()));
        let mut runtime =
            ApplicationRuntime::with_settings_store(store).expect("load settings from test store");
        let ui = UiSettings {
            search_text: "jazz".to_owned(),
            view_mode: UiViewMode::Albums,
            playlist_selection: None,
        };

        assert_eq!(runtime.save_ui_settings(ui.clone()), Ok(()));

        assert_eq!(runtime.settings().ui, ui);
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
            content_hash: None,
            metadata: TrackMetadata::default(),
            rating: Rating::unrated(),
            statistics: PlayStatistics::default(),
            file_size_bytes: None,
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
            runtime.handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayTrack {
                track_id,
                queue: PlaybackQueueRequest::Library,
            })),
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
    fn runtime_sets_shuffle_without_playback_service() {
        let mut runtime = ApplicationRuntime::new();

        assert_eq!(
            runtime.handle_command(ApplicationCommand::Playback(
                PlaybackCommand::SetShuffleEnabled(true)
            )),
            Ok(())
        );
        assert!(runtime.playback_options().shuffle_enabled);

        assert_eq!(
            runtime.handle_command(ApplicationCommand::Playback(
                PlaybackCommand::SetShuffleEnabled(false)
            )),
            Ok(())
        );
        assert!(!runtime.playback_options().shuffle_enabled);
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
            runtime.handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayTrack {
                track_id: track_id(1),
                queue: PlaybackQueueRequest::Library,
            })),
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
            runtime.handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayTrack {
                track_id: track_id(3),
                queue: PlaybackQueueRequest::Library,
            })),
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
    fn runtime_set_rating_applies_optimistic_update_and_reports_tag_write_failure() {
        // The new contract: the in-memory + SQLite update is applied
        // immediately and SetRating returns Ok(()) synchronously, so the
        // UI never blocks on the tag write. Tag-write failure surfaces
        // through the result sink rather than as a command error — the
        // next library scan reconciles the SQLite cache against disk.
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
        let (result_tx, result_rx) = async_channel::unbounded::<crate::MetadataWriteResult>();
        runtime.set_metadata_write_result_sink(result_tx);
        let rating = Rating::new(4).expect("valid test rating");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::SetRating { track_id, rating }),
            Ok(())
        );

        assert_eq!(
            metadata_service.rating_writes(),
            vec![(track_path.clone(), rating)]
        );
        // Optimistic state: in-memory + SQLite both reflect the new rating,
        // even though the disk tag write failed.
        assert_eq!(runtime.library_tracks()[0].rating, rating);
        assert_eq!(
            store
                .track(track_id)
                .expect("load updated track")
                .map(|track| track.rating),
            Some(rating)
        );
        // Failure is reported to the sink (UI surfaces a status-bar
        // message and refreshes the affected row).
        let posted = result_rx
            .try_recv()
            .expect("metadata writer posts the failure");
        assert_eq!(posted.track_id, track_id);
        assert_eq!(posted.kind, crate::MetadataWriteKind::Rating);
        assert_eq!(posted.outcome, crate::MetadataWriteOutcome::Failed);

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
                change: Box::new(change.clone()),
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
    fn managed_metadata_update_moves_file_when_planned_path_changes() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        let source_path = root.join("loose.flac");
        std::fs::write(&source_path, b"audio bytes").expect("write fake track");

        let track_id = track_id(31);
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut track = test_track(track_id, "loose.flac");
        track.metadata.title = Some("Old".to_owned());
        track.metadata.artist = Some("Old Artist".to_owned());
        track.metadata.album = Some("Old Album".to_owned());
        assert_eq!(store.save_track(track), Ok(()));
        let mut settings = UserSettings::with_library_path(Some(root.clone()));
        settings.library.management_mode = LibraryManagementMode::CopyAddedFilesIntoLibrary;
        let metadata_service = Arc::new(RecordingMetadataService::new(false));
        let mut runtime =
            ApplicationRuntime::with_settings_store(Box::new(TestSettingsStore::new(settings)))
                .expect("load settings")
                .with_library_services(store.clone(), metadata_service.clone())
                .expect("library services initialize");
        let change = MetadataChange {
            title: FieldChange::Set("Song".to_owned()),
            artist: FieldChange::Set("Artist".to_owned()),
            album: FieldChange::Set("Album".to_owned()),
            track_number: FieldChange::Set(3),
            ..MetadataChange::default()
        };

        assert_eq!(
            runtime.handle_command(ApplicationCommand::UpdateMetadata {
                track_id,
                change: Box::new(change.clone()),
            }),
            Ok(())
        );

        let destination_path = root.join("Artist/Album/03 Song.flac");
        assert!(!source_path.exists());
        assert_eq!(
            std::fs::read(&destination_path).expect("destination exists"),
            b"audio bytes"
        );
        assert_eq!(
            metadata_service.metadata_writes(),
            vec![(source_path.clone(), change)]
        );
        assert_eq!(
            runtime.library_tracks()[0].location.relative_path.as_path(),
            Path::new("Artist/Album/03 Song.flac")
        );
        assert_eq!(
            store
                .track(track_id)
                .expect("load updated track")
                .map(|track| track.location.relative_path.to_path_buf()),
            Some(PathBuf::from("Artist/Album/03 Song.flac"))
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn managed_metadata_update_keeps_file_in_place_for_non_path_fields() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        let track_path = root.join("Artist/Album/01 Song.flac");
        std::fs::create_dir_all(track_path.parent().expect("test path has parent"))
            .expect("create album directory");
        std::fs::write(&track_path, b"audio bytes").expect("write fake track");

        let track_id = track_id(32);
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut track = test_track(track_id, "Artist/Album/01 Song.flac");
        track.metadata.title = Some("Song".to_owned());
        track.metadata.artist = Some("Artist".to_owned());
        track.metadata.album = Some("Album".to_owned());
        track.metadata.track_number = Some(1);
        assert_eq!(store.save_track(track), Ok(()));
        let mut settings = UserSettings::with_library_path(Some(root.clone()));
        settings.library.management_mode = LibraryManagementMode::CopyAddedFilesIntoLibrary;
        let metadata_service = Arc::new(RecordingMetadataService::new(false));
        let mut runtime =
            ApplicationRuntime::with_settings_store(Box::new(TestSettingsStore::new(settings)))
                .expect("load settings")
                .with_library_services(store.clone(), metadata_service.clone())
                .expect("library services initialize");
        let change = MetadataChange {
            year: FieldChange::Set(1999),
            genre: FieldChange::Set("Rock".to_owned()),
            ..MetadataChange::default()
        };

        assert_eq!(
            runtime.handle_command(ApplicationCommand::UpdateMetadata {
                track_id,
                change: Box::new(change.clone()),
            }),
            Ok(())
        );

        assert!(track_path.exists());
        assert_eq!(
            metadata_service.metadata_writes(),
            vec![(track_path.clone(), change)]
        );
        assert_eq!(
            runtime.library_tracks()[0].location.relative_path.as_path(),
            Path::new("Artist/Album/01 Song.flac")
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn runtime_update_metadata_applies_optimistic_update_and_reports_tag_write_failure() {
        // Same contract as set_rating in the non-managed-rename branch:
        // in-memory + SQLite update is applied synchronously, tag write
        // is dispatched to the async writer, failure surfaces on the
        // result sink.
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
        let (result_tx, result_rx) = async_channel::unbounded::<crate::MetadataWriteResult>();
        runtime.set_metadata_write_result_sink(result_tx);
        let change = MetadataChange {
            title: FieldChange::Set("New".to_owned()),
            ..MetadataChange::default()
        };

        assert_eq!(
            runtime.handle_command(ApplicationCommand::UpdateMetadata {
                track_id,
                change: Box::new(change.clone()),
            }),
            Ok(())
        );

        assert_eq!(
            metadata_service.metadata_writes(),
            vec![(track_path.clone(), change)]
        );
        // Optimistic state holds even though the disk tag write failed.
        assert_eq!(
            runtime.library_tracks()[0].metadata.title.as_deref(),
            Some("New")
        );
        assert_eq!(
            store
                .track(track_id)
                .expect("load updated track")
                .and_then(|track| track.metadata.title),
            Some("New".to_owned())
        );
        let posted = result_rx
            .try_recv()
            .expect("metadata writer posts the failure");
        assert_eq!(posted.track_id, track_id);
        assert_eq!(posted.kind, crate::MetadataWriteKind::Metadata);
        assert_eq!(posted.outcome, crate::MetadataWriteOutcome::Failed);

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
            runtime.handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayTrack {
                track_id: removed_id,
                queue: PlaybackQueueRequest::Library,
            })),
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
            runtime.handle_command(ApplicationCommand::MovePlaylistEntries {
                playlist_id,
                track_ids: vec![track_id(1)],
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
    fn runtime_move_playlist_entries_relocates_a_contiguous_block_atomically() {
        let store = Arc::new(InMemoryLibraryStore::new());
        for id in 1..=5 {
            assert_eq!(
                store.save_track(test_track(track_id(id), &format!("track-{id}.flac"))),
                Ok(())
            );
        }
        let playlist_id = playlist_id(1);
        assert_eq!(
            store.save_playlist(Playlist {
                id: playlist_id,
                name: "Set".to_owned(),
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
                track_ids: (1..=5).map(track_id).collect(),
            }),
            Ok(())
        );
        assert_playlist_track_ids(
            runtime.playlists(),
            playlist_id,
            &[
                track_id(1),
                track_id(2),
                track_id(3),
                track_id(4),
                track_id(5),
            ],
        );

        // Move tracks 3 and 4 to the head: post-removal list is
        // [1, 2, 5] (len 3), insertion at index 0 lands the block ahead
        // of every other entry.
        assert_eq!(
            runtime.handle_command(ApplicationCommand::MovePlaylistEntries {
                playlist_id,
                track_ids: vec![track_id(3), track_id(4)],
                new_position: 0,
            }),
            Ok(())
        );
        assert_playlist_track_ids(
            runtime.playlists(),
            playlist_id,
            &[
                track_id(3),
                track_id(4),
                track_id(1),
                track_id(2),
                track_id(5),
            ],
        );

        // Move tracks 4 and 1 to the tail: caller passes them in arbitrary
        // order, but the post-removal block must reflect the playlist's
        // own current order (1 comes before 4 in [3, 4, 1, 2, 5]),
        // landing as [..., 4, 1] would be wrong; the correct outcome is
        // [3, 2, 5, 4, 1] because at extraction time 4 still precedes 1
        // in the playlist's entries. Saturating new_position to u32::MAX
        // pins the block at the tail.
        assert_eq!(
            runtime.handle_command(ApplicationCommand::MovePlaylistEntries {
                playlist_id,
                track_ids: vec![track_id(1), track_id(4)],
                new_position: u32::MAX,
            }),
            Ok(())
        );
        assert_playlist_track_ids(
            runtime.playlists(),
            playlist_id,
            &[
                track_id(3),
                track_id(2),
                track_id(5),
                track_id(4),
                track_id(1),
            ],
        );

        // Same outcome must be visible in the underlying store, not just
        // the runtime cache.
        assert_playlist_track_ids(
            &[store
                .playlist(playlist_id)
                .expect("playlist loads")
                .expect("playlist exists")],
            playlist_id,
            &[
                track_id(3),
                track_id(2),
                track_id(5),
                track_id(4),
                track_id(1),
            ],
        );
    }

    #[test]
    fn runtime_move_playlist_entries_rejects_empty_track_list() {
        let store = Arc::new(InMemoryLibraryStore::new());
        let playlist_id = playlist_id(1);
        assert_eq!(
            store.save_playlist(Playlist {
                id: playlist_id,
                name: "Set".to_owned(),
                parent_folder_id: None,
                position: 0,
                entries: Vec::new(),
            }),
            Ok(())
        );
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store, Arc::new(TestMetadataService))
            .expect("library services initialize");

        assert_eq!(
            runtime.handle_command(ApplicationCommand::MovePlaylistEntries {
                playlist_id,
                track_ids: Vec::new(),
                new_position: 0,
            }),
            Err(ApplicationRuntimeError::PlaylistEntryNotFound),
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
    fn seeding_default_smart_playlists_installs_the_starter_set() {
        let store = Arc::new(InMemoryLibraryStore::new());
        let mut runtime = ApplicationRuntime::new()
            .with_library_services(store, Arc::new(TestMetadataService))
            .expect("library services initialize");

        runtime
            .seed_default_smart_playlists()
            .expect("seed succeeds on fresh library");

        let names: Vec<&str> = runtime
            .smart_playlists()
            .iter()
            .map(|smart| smart.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "Recently Added",
                "Recently Played",
                "Top 25 Most Played",
                "4+ Stars",
                "Unplayed",
            ]
        );
    }

    #[test]
    fn smart_playlist_evaluation_uses_injected_clock() {
        use std::num::NonZeroU32;
        use std::time::{Duration, SystemTime};

        let last_played = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let store = Arc::new(InMemoryLibraryStore::new());

        let mut track = test_track(track_id(1), "track.flac");
        track.statistics.last_played_at = Some(last_played);
        store.save_track(track).expect("save track");

        let recently_played = SmartPlaylist {
            id: smart_id(1),
            name: "Recently Played".to_owned(),
            parent_folder_id: None,
            position: 0,
            rules: SmartPlaylistRuleSet {
                match_kind: SmartPlaylistMatchKind::All,
                rules: vec![SmartPlaylistRule::DateInLast {
                    field: SmartPlaylistDateField::LastPlayed,
                    days: NonZeroU32::new(7).expect("positive days"),
                }],
                limit: None,
            },
        };
        store
            .save_smart_playlist(recently_played)
            .expect("save smart playlist");

        let fake_clock = Arc::new(FakeClock::new(last_played));
        let runtime = ApplicationRuntime::new()
            .with_library_services(store, Arc::new(TestMetadataService))
            .expect("library services initialize")
            .with_clock(fake_clock.clone());

        fake_clock.set(last_played + Duration::from_secs(86_400));
        assert_eq!(
            runtime.smart_playlist_matching_tracks(smart_id(1)).len(),
            1,
            "track played within the window must match"
        );

        fake_clock.set(last_played + Duration::from_secs(86_400 * 10));
        assert_eq!(
            runtime.smart_playlist_matching_tracks(smart_id(1)).len(),
            0,
            "track played outside the window must not match"
        );
    }

    #[derive(Debug)]
    struct FakeClock {
        now: Mutex<std::time::SystemTime>,
    }

    impl FakeClock {
        fn new(now: std::time::SystemTime) -> Self {
            Self {
                now: Mutex::new(now),
            }
        }

        fn set(&self, now: std::time::SystemTime) {
            *self.now.lock().expect("fake clock lock") = now;
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> std::time::SystemTime {
            *self.now.lock().expect("fake clock lock")
        }
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

        fn write_artwork(&self, _path: &Path, _artwork: Option<Vec<u8>>) -> MetadataResult<()> {
            Ok(())
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

        fn write_artwork(&self, _path: &Path, _artwork: Option<Vec<u8>>) -> MetadataResult<()> {
            Ok(())
        }
    }

    #[test]
    fn on_playback_tick_registers_play_after_threshold() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        std::fs::write(root.join("song.flac"), b"not real audio").expect("write fake track");

        let store = Arc::new(InMemoryLibraryStore::new());
        let id = track_id(1);
        let mut track = test_track(id, "song.flac");
        track.metadata.duration = Some(std::time::Duration::from_secs(60));
        assert_eq!(store.save_track(track), Ok(()));

        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store, Arc::new(TestMetadataService))
        .expect("library services initialize")
        .with_playback_service(Box::new(NullPlaybackService::new()));

        runtime
            .handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayTrack {
                track_id: id,
                queue: PlaybackQueueRequest::Library,
            }))
            .expect("play track");

        // Threshold for a 60s track is 30s. 29 ticks of 1s each must
        // not be enough to register the play.
        for _ in 0..29 {
            runtime
                .on_playback_tick(std::time::Duration::from_secs(1))
                .expect("tick");
        }
        let track = runtime
            .library_tracks()
            .iter()
            .find(|track| track.id == id)
            .expect("track present");
        assert_eq!(
            track.statistics.play_count, 0,
            "play count must not increment before threshold cross"
        );

        runtime
            .on_playback_tick(std::time::Duration::from_secs(1))
            .expect("tick that crosses threshold");
        let track = runtime
            .library_tracks()
            .iter()
            .find(|track| track.id == id)
            .expect("track present");
        assert_eq!(
            track.statistics.play_count, 1,
            "play count must increment exactly once when threshold is crossed"
        );
        assert!(
            track.statistics.last_played_at.is_some(),
            "last_played_at must be set when play registers"
        );

        // Further ticks past threshold must not re-increment within the
        // same listening session.
        for _ in 0..60 {
            runtime
                .on_playback_tick(std::time::Duration::from_secs(1))
                .expect("post-threshold tick");
        }
        let track = runtime
            .library_tracks()
            .iter()
            .find(|track| track.id == id)
            .expect("track present");
        assert_eq!(
            track.statistics.play_count, 1,
            "play must register exactly once per session"
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn skip_current_track_registers_skip_before_play_threshold() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        std::fs::write(root.join("a.flac"), b"audio").expect("write a");
        std::fs::write(root.join("b.flac"), b"audio").expect("write b");

        let store = Arc::new(InMemoryLibraryStore::new());
        let mut a = test_track(track_id(1), "a.flac");
        a.metadata.duration = Some(std::time::Duration::from_secs(60));
        let mut b = test_track(track_id(2), "b.flac");
        b.metadata.duration = Some(std::time::Duration::from_secs(60));
        assert_eq!(store.save_track(a), Ok(()));
        assert_eq!(store.save_track(b), Ok(()));

        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store, Arc::new(TestMetadataService))
        .expect("library services initialize")
        .with_playback_service(Box::new(NullPlaybackService::new()));

        runtime
            .handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayTrack {
                track_id: track_id(1),
                queue: PlaybackQueueRequest::Library,
            }))
            .expect("play A");

        // Listen briefly — well short of the 30s threshold — then skip.
        for _ in 0..5 {
            runtime
                .on_playback_tick(std::time::Duration::from_secs(1))
                .expect("tick");
        }

        runtime
            .handle_command(ApplicationCommand::Playback(
                PlaybackCommand::SkipCurrentTrack,
            ))
            .expect("skip current track");

        let track_a = runtime
            .library_tracks()
            .iter()
            .find(|track| track.id == track_id(1))
            .expect("track A present");
        assert_eq!(
            track_a.statistics.skip_count, 1,
            "skip must increment when threshold not yet reached"
        );
        assert!(
            track_a.statistics.last_skipped_at.is_some(),
            "last_skipped_at must be set on skip"
        );
        assert_eq!(
            track_a.statistics.play_count, 0,
            "skip must not also register a play"
        );

        // Track B is now playing as a result of the advance.
        match runtime.playback_state() {
            PlaybackState::Playing {
                track_id: playing, ..
            } => assert_eq!(playing, track_id(2)),
            other => panic!("expected B to be playing, got {other:?}"),
        }

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn skip_current_track_does_not_register_skip_after_play_threshold() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        std::fs::write(root.join("a.flac"), b"audio").expect("write a");
        std::fs::write(root.join("b.flac"), b"audio").expect("write b");

        let store = Arc::new(InMemoryLibraryStore::new());
        let mut a = test_track(track_id(1), "a.flac");
        a.metadata.duration = Some(std::time::Duration::from_secs(60));
        let b = test_track(track_id(2), "b.flac");
        assert_eq!(store.save_track(a), Ok(()));
        assert_eq!(store.save_track(b), Ok(()));

        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store, Arc::new(TestMetadataService))
        .expect("library services initialize")
        .with_playback_service(Box::new(NullPlaybackService::new()));

        runtime
            .handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayTrack {
                track_id: track_id(1),
                queue: PlaybackQueueRequest::Library,
            }))
            .expect("play A");

        // Cross the play threshold for the 60s track.
        for _ in 0..30 {
            runtime
                .on_playback_tick(std::time::Duration::from_secs(1))
                .expect("tick");
        }

        runtime
            .handle_command(ApplicationCommand::Playback(
                PlaybackCommand::SkipCurrentTrack,
            ))
            .expect("skip after play registered");

        let track_a = runtime
            .library_tracks()
            .iter()
            .find(|track| track.id == track_id(1))
            .expect("track A present");
        assert_eq!(
            track_a.statistics.play_count, 1,
            "play already counted before skip"
        );
        assert_eq!(
            track_a.statistics.skip_count, 0,
            "post-threshold next must not increment skip"
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn play_next_track_never_registers_skip() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        std::fs::write(root.join("a.flac"), b"audio").expect("write a");
        std::fs::write(root.join("b.flac"), b"audio").expect("write b");

        let store = Arc::new(InMemoryLibraryStore::new());
        let mut a = test_track(track_id(1), "a.flac");
        a.metadata.duration = Some(std::time::Duration::from_secs(60));
        let b = test_track(track_id(2), "b.flac");
        assert_eq!(store.save_track(a), Ok(()));
        assert_eq!(store.save_track(b), Ok(()));

        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store, Arc::new(TestMetadataService))
        .expect("library services initialize")
        .with_playback_service(Box::new(NullPlaybackService::new()));

        runtime
            .handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayTrack {
                track_id: track_id(1),
                queue: PlaybackQueueRequest::Library,
            }))
            .expect("play A");

        // Briefly listen — well short of the play threshold.
        for _ in 0..5 {
            runtime
                .on_playback_tick(std::time::Duration::from_secs(1))
                .expect("tick");
        }

        // EOS-style auto-advance must never affect skip statistics,
        // regardless of how much of the previous track was listened.
        runtime
            .handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayNextTrack))
            .expect("auto-advance");

        let track_a = runtime
            .library_tracks()
            .iter()
            .find(|track| track.id == track_id(1))
            .expect("track A present");
        assert_eq!(
            track_a.statistics.skip_count, 0,
            "auto-advance must never inflate skip count"
        );
        assert_eq!(
            track_a.statistics.play_count, 0,
            "auto-advance below threshold must not register a play either"
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn on_playback_tick_does_not_accumulate_when_stopped() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");

        let store = Arc::new(InMemoryLibraryStore::new());
        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store, Arc::new(TestMetadataService))
        .expect("library services initialize")
        .with_playback_service(Box::new(NullPlaybackService::new()));

        // No PlayTrack — runtime is in the Stopped state, no session.
        for _ in 0..100 {
            runtime
                .on_playback_tick(std::time::Duration::from_secs(1))
                .expect("tick is a no-op while stopped");
        }
        assert!(
            runtime.playback_session.is_none(),
            "no session should be created when nothing is playing"
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn play_track_starts_session_immediately_so_rapid_skip_counts() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        std::fs::write(root.join("a.flac"), b"audio").expect("write a");
        std::fs::write(root.join("b.flac"), b"audio").expect("write b");

        let store = Arc::new(InMemoryLibraryStore::new());
        let mut a = test_track(track_id(1), "a.flac");
        a.metadata.duration = Some(std::time::Duration::from_secs(60));
        let b = test_track(track_id(2), "b.flac");
        assert_eq!(store.save_track(a), Ok(()));
        assert_eq!(store.save_track(b), Ok(()));

        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store, Arc::new(TestMetadataService))
        .expect("library services initialize")
        .with_playback_service(Box::new(NullPlaybackService::new()));

        runtime
            .handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayTrack {
                track_id: track_id(1),
                queue: PlaybackQueueRequest::Library,
            }))
            .expect("play A");

        // No ticks have fired yet. Skip immediately. The session must
        // already exist (populated synchronously by play_track) so the
        // skip is captured rather than silently dropped.
        runtime
            .handle_command(ApplicationCommand::Playback(
                PlaybackCommand::SkipCurrentTrack,
            ))
            .expect("immediate skip");

        let track_a = runtime
            .library_tracks()
            .iter()
            .find(|track| track.id == track_id(1))
            .expect("track A present");
        assert_eq!(
            track_a.statistics.skip_count, 1,
            "skip must register even with zero listened time"
        );

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    #[test]
    fn rapid_double_skip_does_not_double_count() {
        let root = unique_test_directory();
        std::fs::create_dir_all(&root).expect("create test library");
        std::fs::write(root.join("a.flac"), b"audio").expect("write a");
        std::fs::write(root.join("b.flac"), b"audio").expect("write b");
        std::fs::write(root.join("c.flac"), b"audio").expect("write c");

        let store = Arc::new(InMemoryLibraryStore::new());
        let mut a = test_track(track_id(1), "a.flac");
        a.metadata.duration = Some(std::time::Duration::from_secs(60));
        let mut b = test_track(track_id(2), "b.flac");
        b.metadata.duration = Some(std::time::Duration::from_secs(60));
        let c = test_track(track_id(3), "c.flac");
        assert_eq!(store.save_track(a), Ok(()));
        assert_eq!(store.save_track(b), Ok(()));
        assert_eq!(store.save_track(c), Ok(()));

        let mut runtime = ApplicationRuntime::with_settings_store(Box::new(
            TestSettingsStore::new(UserSettings::with_library_path(Some(root.clone()))),
        ))
        .expect("load settings")
        .with_library_services(store, Arc::new(TestMetadataService))
        .expect("library services initialize")
        .with_playback_service(Box::new(NullPlaybackService::new()));

        runtime
            .handle_command(ApplicationCommand::Playback(PlaybackCommand::PlayTrack {
                track_id: track_id(1),
                queue: PlaybackQueueRequest::Library,
            }))
            .expect("play A");
        runtime
            .handle_command(ApplicationCommand::Playback(
                PlaybackCommand::SkipCurrentTrack,
            ))
            .expect("first skip — A → B");
        // Immediately skip again before any tick has accumulated time
        // on B. A second skip on A would be a double-count bug; this
        // exercises the "play_track installs a fresh session" guard.
        runtime
            .handle_command(ApplicationCommand::Playback(
                PlaybackCommand::SkipCurrentTrack,
            ))
            .expect("second skip — B → C");

        let track_a = runtime
            .library_tracks()
            .iter()
            .find(|track| track.id == track_id(1))
            .expect("track A present");
        let track_b = runtime
            .library_tracks()
            .iter()
            .find(|track| track.id == track_id(2))
            .expect("track B present");
        assert_eq!(track_a.statistics.skip_count, 1, "A skipped exactly once");
        assert_eq!(track_b.statistics.skip_count, 1, "B skipped exactly once");

        std::fs::remove_dir_all(root).expect("remove test library");
    }

    fn unique_test_directory() -> PathBuf {
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("sustain_runtime_test_{unique_suffix}"))
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
            content_hash: None,
            metadata: TrackMetadata::default(),
            rating: Rating::unrated(),
            statistics: PlayStatistics::default(),
            file_size_bytes: None,
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

    fn hex_path(path: &str) -> String {
        path.as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
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
