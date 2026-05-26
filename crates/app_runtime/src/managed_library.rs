// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{
    collections::{BTreeSet, HashSet},
    fs,
    io::{BufReader, BufWriter, Write},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use sustain_domain::{
    FieldChange, LibraryManagementMode, ManagedTrackPathInput, ManagedTrackPathPlanner,
    MetadataChange, PlayStatistics, Rating, Track, TrackAvailability, TrackContentHash, TrackId,
    TrackLocation, TrackRelativePath,
};
use sustain_metadata::{audio_format_from_path, hash_file_content};

use crate::{
    ApplicationRuntime, ApplicationRuntimeError, ApplicationRuntimeResult,
    LibraryConsolidationResult, LibraryConsolidationSummary, LibraryConsolidationTask,
    LibraryImportResult, LibraryImportSummary, LibraryImportTask, NotificationCategory,
    NotificationSeverity, library_scan, notifications,
};

const CONSOLIDATION_JOURNAL_FILE_NAME: &str = ".sustain-consolidation-journal";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedFileCopy {
    pub destination_path: PathBuf,
    pub bytes_copied: u64,
    pub content_hash: TrackContentHash,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VerifiedFileCopyError {
    SourceUnavailable,
    SourceIsNotFile,
    DestinationHasNoParent,
    DestinationExists,
    CreateDestinationDirectoryFailed,
    CreateTemporaryFileFailed,
    CopyFailed,
    SizeMismatch {
        expected: u64,
        actual: u64,
    },
    HashMismatch {
        expected: TrackContentHash,
        actual: TrackContentHash,
    },
    FinalizeFailed,
}

pub fn copy_file_verified(
    source_path: &Path,
    destination_path: &Path,
    expected_hash: &TrackContentHash,
) -> Result<VerifiedFileCopy, VerifiedFileCopyError> {
    let source_metadata =
        fs::metadata(source_path).map_err(|_| VerifiedFileCopyError::SourceUnavailable)?;
    if !source_metadata.is_file() {
        return Err(VerifiedFileCopyError::SourceIsNotFile);
    }
    if destination_path.exists() {
        return Err(VerifiedFileCopyError::DestinationExists);
    }

    let destination_parent = destination_path
        .parent()
        .ok_or(VerifiedFileCopyError::DestinationHasNoParent)?;
    fs::create_dir_all(destination_parent)
        .map_err(|_| VerifiedFileCopyError::CreateDestinationDirectoryFailed)?;

    let temporary_path = create_temporary_copy_path(destination_path)?;
    let result = copy_file_verified_inner(
        source_path,
        destination_path,
        &temporary_path,
        source_metadata.len(),
        source_metadata.permissions(),
        expected_hash,
    );

    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }

    result
}

impl ApplicationRuntime {
    pub(super) fn add_external_library_items(
        &mut self,
        paths: Vec<PathBuf>,
    ) -> ApplicationRuntimeResult<()> {
        let task = self.prepare_library_import(paths)?;
        match run_library_import_task(task) {
            Ok(result) => {
                self.apply_library_import_result(result);
                Ok(())
            }
            Err(error) => {
                self.fail_library_import(error.clone());
                Err(error)
            }
        }
    }

    pub fn prepare_library_import(
        &mut self,
        paths: Vec<PathBuf>,
    ) -> ApplicationRuntimeResult<LibraryImportTask> {
        if self.background_task_status.is_running() {
            return Err(ApplicationRuntimeError::BackgroundTaskRunning);
        }

        let library_store = self
            .library_store
            .clone()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;
        let metadata_service = self
            .metadata_service
            .clone()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;

        let cancellation_requested = Arc::new(AtomicBool::new(false));
        self.library_import_cancellation = Some(cancellation_requested.clone());
        self.background_task_status = crate::BackgroundTaskStatus::LibraryImportRunning;
        let notification_id = self.push_persistent_notification(
            NotificationCategory::LibraryImport,
            NotificationSeverity::Info,
            notifications::library_import_running_text(),
            true,
        );
        self.library_import_notification_id = Some(notification_id);

        Ok(LibraryImportTask {
            paths,
            settings: self.settings.clone(),
            existing_tracks: self.library_tracks.clone(),
            library_store,
            metadata_service,
            cancellation_requested,
        })
    }

    pub fn apply_library_import_result(&mut self, result: LibraryImportResult) {
        let summary = result.summary;
        self.last_library_import_summary = Some(summary.clone());
        self.library_tracks.extend(result.tracks);
        self.library_tracks.sort_by_key(|track| track.id);
        self.refresh_playback_queue_track_ids();
        self.library_import_cancellation = None;
        self.background_task_status = crate::BackgroundTaskStatus::Idle;
        if let Some(id) = self.library_import_notification_id.take() {
            self.dismiss_notification(id);
        }
        self.push_ephemeral_notification(
            NotificationCategory::LibraryImport,
            NotificationSeverity::Info,
            notifications::library_import_outcome_text(&summary),
        );
    }

    pub fn fail_library_import(&mut self, error: ApplicationRuntimeError) {
        self.library_import_cancellation = None;
        self.background_task_status = crate::BackgroundTaskStatus::Idle;
        if let Some(id) = self.library_import_notification_id.take() {
            self.dismiss_notification(id);
        }
        self.push_ephemeral_notification(
            NotificationCategory::LibraryImport,
            NotificationSeverity::Error,
            notifications::runtime_error_text(&error).to_owned(),
        );
    }

    pub fn prepare_library_consolidation(
        &mut self,
    ) -> ApplicationRuntimeResult<LibraryConsolidationTask> {
        if self.background_task_status.is_running() {
            return Err(ApplicationRuntimeError::BackgroundTaskRunning);
        }
        if self.settings.library.management_mode != LibraryManagementMode::CopyAddedFilesIntoLibrary
        {
            return Err(ApplicationRuntimeError::LibraryConsolidationFailed);
        }

        let library_store = self
            .library_store
            .clone()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;
        let cancellation_requested = Arc::new(AtomicBool::new(false));
        self.library_consolidation_cancellation = Some(cancellation_requested.clone());
        self.background_task_status = crate::BackgroundTaskStatus::LibraryConsolidationRunning;
        let notification_id = self.push_persistent_notification(
            NotificationCategory::LibraryConsolidation,
            NotificationSeverity::Info,
            notifications::library_consolidation_running_text(),
            true,
        );
        self.library_consolidation_notification_id = Some(notification_id);

        Ok(LibraryConsolidationTask {
            settings: self.settings.clone(),
            existing_tracks: self.library_tracks.clone(),
            library_store,
            cancellation_requested,
        })
    }

    pub fn apply_library_consolidation_result(&mut self, result: LibraryConsolidationResult) {
        let summary = result.summary;
        self.last_library_consolidation_summary = Some(summary.clone());
        // `result.tracks` now carries both the relocated (still
        // available) tracks AND any rows whose `is_missing` flag the
        // planner flipped because the source file had vanished —
        // fire the availability observer so the UI repaints the
        // status column on those rows without the cost of a full
        // table rebuild.
        let flipped_availability = result.tracks.iter().any(|incoming| {
            self.library_tracks
                .iter()
                .find(|existing| existing.id == incoming.id)
                .is_some_and(|existing| existing.location.is_missing() != incoming.location.is_missing())
        });
        replace_library_tracks_by_id(&mut self.library_tracks, result.tracks);
        self.refresh_playback_queue_track_ids();
        if flipped_availability {
            self.notify_track_availability_observer();
        }
        self.library_consolidation_cancellation = None;
        self.background_task_status = crate::BackgroundTaskStatus::Idle;
        if let Some(id) = self.library_consolidation_notification_id.take() {
            self.dismiss_notification(id);
        }
        // An auto-resume that found nothing to move and nothing
        // missing is silenced by the auto-dismiss timer just like any
        // ephemeral — no special "boring success" branch needed.
        self.push_ephemeral_notification(
            NotificationCategory::LibraryConsolidation,
            NotificationSeverity::Info,
            notifications::library_consolidation_outcome_text(&summary),
        );
    }

    pub fn fail_library_consolidation(&mut self, error: ApplicationRuntimeError) {
        self.library_consolidation_cancellation = None;
        self.background_task_status = crate::BackgroundTaskStatus::Idle;
        if let Some(id) = self.library_consolidation_notification_id.take() {
            self.dismiss_notification(id);
        }
        self.push_ephemeral_notification(
            NotificationCategory::LibraryConsolidation,
            NotificationSeverity::Error,
            notifications::runtime_error_text(&error).to_owned(),
        );
    }
}

pub fn run_library_import_task(
    task: LibraryImportTask,
) -> ApplicationRuntimeResult<LibraryImportResult> {
    let mut context = LibraryImportContext {
        settings: task.settings,
        existing_tracks: task.existing_tracks,
        library_store: task.library_store,
        metadata_service: task.metadata_service,
        cancellation_requested: task.cancellation_requested,
    };

    context.add_external_library_items(task.paths)
}

pub fn run_library_consolidation_task(
    task: LibraryConsolidationTask,
) -> ApplicationRuntimeResult<LibraryConsolidationResult> {
    let context = LibraryConsolidationContext {
        settings: task.settings,
        existing_tracks: task.existing_tracks,
        library_store: task.library_store,
        cancellation_requested: task.cancellation_requested,
    };

    context.consolidate_library()
}

pub(super) fn metadata_change_affects_managed_path(change: &MetadataChange) -> bool {
    !matches!(change.title, FieldChange::Unchanged)
        || !matches!(change.artist, FieldChange::Unchanged)
        || !matches!(change.album, FieldChange::Unchanged)
        || !matches!(change.album_artist, FieldChange::Unchanged)
        || !matches!(change.composer, FieldChange::Unchanged)
        || !matches!(change.track_number, FieldChange::Unchanged)
        || !matches!(change.disc_number, FieldChange::Unchanged)
        || !matches!(change.disc_total, FieldChange::Unchanged)
        || !matches!(change.compilation, FieldChange::Unchanged)
}

pub(super) fn save_managed_metadata_update(
    library_path: &Path,
    library_store: &dyn sustain_library_store::LibraryStore,
    existing_tracks: &[Track],
    track: Track,
) -> ApplicationRuntimeResult<Track> {
    recover_library_consolidation_journal(library_path, library_store)?;

    let plan = plan_managed_track_retarget(library_path, existing_tracks, track.clone())?;
    let Some(planned_move) = plan else {
        library_store
            .save_track(track.clone())
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        return Ok(track);
    };

    write_consolidation_journal(library_path, std::slice::from_ref(&planned_move))?;

    if move_file_without_copy_or_overwrite(
        &planned_move.source_path,
        &planned_move.destination_path,
    )
    .is_err()
    {
        return Err(ApplicationRuntimeError::LibraryConsolidationFailed);
    }

    let updated_track = planned_move.updated_track;
    if library_store.save_track(updated_track.clone()).is_err() {
        rollback_file_move(&planned_move.source_path, &planned_move.destination_path).ok();
        return Err(ApplicationRuntimeError::LibraryStoreFailed);
    }

    remove_consolidation_journal_if_present(library_path)?;
    Ok(updated_track)
}

struct LibraryImportContext {
    settings: sustain_domain::UserSettings,
    existing_tracks: Vec<Track>,
    library_store: std::sync::Arc<dyn sustain_library_store::LibraryStore>,
    metadata_service: std::sync::Arc<dyn sustain_metadata::MetadataService>,
    cancellation_requested: Arc<AtomicBool>,
}

impl LibraryImportContext {
    fn add_external_library_items(
        &mut self,
        paths: Vec<PathBuf>,
    ) -> ApplicationRuntimeResult<LibraryImportResult> {
        if paths.is_empty() {
            return Ok(LibraryImportResult {
                tracks: Vec::new(),
                summary: LibraryImportSummary::default(),
            });
        }

        match self.settings.library.management_mode {
            LibraryManagementMode::ReferenceFilesInPlace => {
                self.add_referenced_external_library_items(paths)
            }
            LibraryManagementMode::CopyAddedFilesIntoLibrary => {
                self.add_managed_external_library_items(paths)
            }
        }
    }

    fn add_managed_external_library_items(
        &mut self,
        paths: Vec<PathBuf>,
    ) -> ApplicationRuntimeResult<LibraryImportResult> {
        let library_path = self
            .settings
            .library_path()
            .ok_or(ApplicationRuntimeError::LibraryPathUnavailable)?
            .to_path_buf();
        let canonical_library_path = fs::canonicalize(&library_path).ok();

        let discovered_files =
            collect_supported_audio_files(&paths, self.cancellation_requested.as_ref())?;
        if self.cancellation_requested.load(Ordering::SeqCst) {
            return Ok(cancelled_import_result(discovered_files.len()));
        }

        let mut occupied_paths = self
            .existing_tracks
            .iter()
            .map(|track| track.location.relative_path.clone())
            .collect::<BTreeSet<_>>();
        let mut seen_hashes = self
            .existing_tracks
            .iter()
            .filter_map(|track| track.content_hash.as_ref().map(TrackContentHash::as_str))
            .map(str::to_owned)
            .collect::<HashSet<_>>();

        let planner = ManagedTrackPathPlanner::default();
        let mut imports = Vec::new();
        let mut duplicate_files = 0;

        for source_path in &discovered_files {
            if self.cancellation_requested.load(Ordering::SeqCst) {
                return Ok(cancelled_import_result(discovered_files.len()));
            }
            let source_path = fs::canonicalize(source_path)
                .map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;
            if let Some(relative_path) =
                source_relative_path_inside_library(&source_path, canonical_library_path.as_deref())
                && occupied_paths.contains(&relative_path)
            {
                duplicate_files += 1;
                continue;
            }

            let source_size = fs::metadata(&source_path)
                .map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?
                .len();
            let content_hash = hash_file_content(&source_path)
                .map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;
            if seen_hashes.contains(content_hash.as_str())
                || self.library_contains_matching_content(
                    &library_path,
                    source_size,
                    &content_hash,
                )?
            {
                duplicate_files += 1;
                continue;
            }

            let mut metadata = self
                .metadata_service
                .read_metadata(&source_path)
                .map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;
            metadata.ensure_title_from_filename(&source_path);
            let rating = self
                .metadata_service
                .read_rating(&source_path)
                .map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?
                .unwrap_or_else(Rating::unrated);
            let plan = plan_destination(
                &planner,
                &mut occupied_paths,
                &library_path,
                &source_path,
                &metadata,
            )?;
            seen_hashes.insert(content_hash.as_str().to_owned());
            imports.push(PlannedManagedImport {
                source_path,
                destination_path: library_path.join(plan.relative_path.as_path()),
                relative_path: plan.relative_path,
                content_hash,
                metadata,
                rating,
                file_size_bytes: source_size,
            });
        }

        let mut copied_paths = Vec::new();
        for import in &imports {
            if self.cancellation_requested.load(Ordering::SeqCst) {
                // Roll back the files we have copied so far so a
                // cancelled import leaves zero filesystem side
                // effects.
                remove_copied_files(&copied_paths);
                return Ok(cancelled_import_result(discovered_files.len()));
            }
            match copy_file_verified(
                &import.source_path,
                &import.destination_path,
                &import.content_hash,
            ) {
                Ok(_) => copied_paths.push(import.destination_path.clone()),
                Err(_) => {
                    remove_copied_files(&copied_paths);
                    return Err(ApplicationRuntimeError::LibraryImportFailed);
                }
            }
        }

        let mut next_track_id = library_scan::next_track_id(&self.existing_tracks)?;
        let mut tracks = Vec::new();
        for import in imports {
            let Some(track_id) = sustain_domain::TrackId::new(next_track_id) else {
                remove_copied_files(&copied_paths);
                return Err(ApplicationRuntimeError::LibraryStoreFailed);
            };
            next_track_id += 1;
            tracks.push(Track {
                id: track_id,
                location: TrackLocation::available(import.relative_path),
                content_hash: Some(import.content_hash),
                metadata: import.metadata,
                rating: import.rating,
                statistics: PlayStatistics {
                    date_added_at: Some(SystemTime::now()),
                    ..PlayStatistics::default()
                },
                file_size_bytes: Some(import.file_size_bytes),
            });
        }

        if self.library_store.save_tracks(&tracks).is_err() {
            remove_copied_files(&copied_paths);
            return Err(ApplicationRuntimeError::LibraryStoreFailed);
        }

        Ok(LibraryImportResult {
            tracks,
            summary: LibraryImportSummary {
                discovered_files: discovered_files.len(),
                imported_tracks: copied_paths.len(),
                duplicate_files,
                cancelled: false,
            },
        })
    }

    fn add_referenced_external_library_items(
        &mut self,
        paths: Vec<PathBuf>,
    ) -> ApplicationRuntimeResult<LibraryImportResult> {
        let library_path = self
            .settings
            .library_path()
            .ok_or(ApplicationRuntimeError::LibraryPathUnavailable)?
            .to_path_buf();
        let canonical_library_path = fs::canonicalize(&library_path)
            .map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;

        let discovered_files =
            collect_supported_audio_files(&paths, self.cancellation_requested.as_ref())?;
        if self.cancellation_requested.load(Ordering::SeqCst) {
            return Ok(cancelled_import_result(discovered_files.len()));
        }
        let mut seen_locations = self
            .existing_tracks
            .iter()
            .map(|track| track.location.relative_path.clone())
            .collect::<BTreeSet<_>>();

        let mut next_track_id = library_scan::next_track_id(&self.existing_tracks)?;
        let mut tracks = Vec::new();
        let mut duplicate_files = 0;

        for source_path in &discovered_files {
            if self.cancellation_requested.load(Ordering::SeqCst) {
                return Ok(cancelled_import_result(discovered_files.len()));
            }
            let source_path = fs::canonicalize(source_path)
                .map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;
            let relative_path =
                reference_relative_path_for_source(&source_path, &canonical_library_path)?;
            if !seen_locations.insert(relative_path.clone()) {
                duplicate_files += 1;
                continue;
            }

            let mut metadata = self
                .metadata_service
                .read_metadata(&source_path)
                .map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;
            metadata.ensure_title_from_filename(&source_path);
            let rating = self
                .metadata_service
                .read_rating(&source_path)
                .map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?
                .unwrap_or_else(Rating::unrated);
            let file_size_bytes = fs::metadata(&source_path)
                .map(|metadata| metadata.len())
                .ok();

            let Some(track_id) = sustain_domain::TrackId::new(next_track_id) else {
                return Err(ApplicationRuntimeError::LibraryStoreFailed);
            };
            next_track_id += 1;
            tracks.push(Track {
                id: track_id,
                location: TrackLocation::available(relative_path),
                content_hash: None,
                metadata,
                rating,
                statistics: PlayStatistics {
                    date_added_at: Some(SystemTime::now()),
                    ..PlayStatistics::default()
                },
                file_size_bytes,
            });
        }

        if self.library_store.save_tracks(&tracks).is_err() {
            return Err(ApplicationRuntimeError::LibraryStoreFailed);
        }

        let imported_tracks = tracks.len();
        Ok(LibraryImportResult {
            tracks,
            summary: LibraryImportSummary {
                discovered_files: discovered_files.len(),
                imported_tracks,
                duplicate_files,
                cancelled: false,
            },
        })
    }

    fn library_contains_matching_content(
        &self,
        library_path: &Path,
        source_size: u64,
        content_hash: &TrackContentHash,
    ) -> ApplicationRuntimeResult<bool> {
        if self
            .library_store
            .track_by_content_hash(content_hash)
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?
            .is_some()
        {
            return Ok(true);
        }

        for track in &self.existing_tracks {
            if track.content_hash.as_ref() == Some(content_hash) {
                return Ok(true);
            }
            if track.content_hash.is_some() {
                continue;
            }

            let track_path = track.location.absolute_path(library_path);
            let Ok(metadata) = fs::metadata(&track_path) else {
                continue;
            };
            if !metadata.is_file() || metadata.len() != source_size {
                continue;
            }
            let Ok(existing_hash) = hash_file_content(&track_path) else {
                continue;
            };
            if &existing_hash == content_hash {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

struct LibraryConsolidationContext {
    settings: sustain_domain::UserSettings,
    existing_tracks: Vec<Track>,
    library_store: std::sync::Arc<dyn sustain_library_store::LibraryStore>,
    cancellation_requested: Arc<AtomicBool>,
}

impl LibraryConsolidationContext {
    fn consolidate_library(self) -> ApplicationRuntimeResult<LibraryConsolidationResult> {
        let library_path = self
            .settings
            .library_path()
            .ok_or(ApplicationRuntimeError::LibraryPathUnavailable)?
            .to_path_buf();

        recover_library_consolidation_journal(&library_path, self.library_store.as_ref())?;

        let plan = plan_library_consolidation(&library_path, &self.existing_tracks)?;

        // Persist any `is_missing` flag corrections discovered during
        // planning before touching any files on disk: the flag flip
        // is durable even if a later move fails, and the result we
        // return always carries the corrected tracks so the runtime's
        // in-memory copy matches SQLite. Done in one transaction via
        // `save_tracks` to keep the cost bounded on a 10k library.
        if !plan.missing_track_updates.is_empty() {
            self.library_store
                .save_tracks(&plan.missing_track_updates)
                .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        }

        if plan.moves.is_empty() {
            remove_consolidation_journal_if_present(&library_path)?;
            return Ok(LibraryConsolidationResult {
                tracks: plan.missing_track_updates,
                summary: LibraryConsolidationSummary {
                    planned_tracks: 0,
                    moved_tracks: 0,
                    already_organized_tracks: plan.already_organized_tracks,
                    missing_tracks: plan.missing_tracks,
                    cancelled: self.cancellation_requested.load(Ordering::SeqCst),
                },
            });
        }

        write_consolidation_journal(&library_path, &plan.moves)?;

        let mut updated_tracks = plan.missing_track_updates;
        let mut moved_tracks = 0;
        let mut cancelled = false;

        for planned_move in &plan.moves {
            if self.cancellation_requested.load(Ordering::SeqCst) {
                cancelled = true;
                break;
            }

            if move_file_without_copy_or_overwrite(
                &planned_move.source_path,
                &planned_move.destination_path,
            )
            .is_err()
            {
                return Err(ApplicationRuntimeError::LibraryConsolidationFailed);
            }

            let updated_track = planned_move.updated_track.clone();
            if self
                .library_store
                .save_track(updated_track.clone())
                .is_err()
            {
                rollback_file_move(&planned_move.source_path, &planned_move.destination_path).ok();
                return Err(ApplicationRuntimeError::LibraryStoreFailed);
            }

            updated_tracks.push(updated_track);
            moved_tracks += 1;
        }

        remove_consolidation_journal_if_present(&library_path)?;

        Ok(LibraryConsolidationResult {
            tracks: updated_tracks,
            summary: LibraryConsolidationSummary {
                planned_tracks: plan.moves.len(),
                moved_tracks,
                already_organized_tracks: plan.already_organized_tracks,
                missing_tracks: plan.missing_tracks,
                cancelled,
            },
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LibraryConsolidationPlan {
    moves: Vec<PlannedLibraryConsolidationMove>,
    already_organized_tracks: usize,
    /// Total number of tracks whose source file was missing or
    /// non-regular at plan time — surfaced in the
    /// [`LibraryConsolidationSummary`] so the user sees a stable
    /// count of orphaned rows regardless of whether the SQLite flag
    /// was already correct.
    missing_tracks: usize,
    /// Subset of the missing tracks whose persisted `is_missing` flag
    /// was still `false` at plan time. The runner flips and persists
    /// these in a single transaction so subsequent reads of SQLite
    /// see the corrected availability, and the per-row warning icon
    /// in the table lights up.
    missing_track_updates: Vec<Track>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PlannedLibraryConsolidationMove {
    track_id: TrackId,
    source_path: PathBuf,
    destination_path: PathBuf,
    source_relative_path: TrackRelativePath,
    destination_relative_path: TrackRelativePath,
    updated_track: Track,
}

fn plan_library_consolidation(
    library_path: &Path,
    existing_tracks: &[Track],
) -> ApplicationRuntimeResult<LibraryConsolidationPlan> {
    let planner = ManagedTrackPathPlanner::default();
    let mut occupied_paths = existing_tracks
        .iter()
        .map(|track| track.location.relative_path.clone())
        .collect::<BTreeSet<_>>();
    let mut moves = Vec::new();
    let mut already_organized_tracks = 0;
    let mut missing_tracks = 0;
    let mut missing_track_updates: Vec<Track> = Vec::new();
    let mut record_missing_track = |track: &Track| {
        missing_tracks += 1;
        // Only push an update when the persisted flag is actually
        // wrong; an already-missing row needs no rewrite. The runner
        // commits the whole batch in a single transaction so the
        // table's missing-file indicator lights up on the very next
        // refresh.
        if !track.location.is_missing() {
            let mut updated = track.clone();
            updated.location = updated.location.with_availability(TrackAvailability::Missing);
            missing_track_updates.push(updated);
        }
    };

    for track in existing_tracks {
        let source_relative_path = track.location.relative_path.clone();
        let source_path = track.location.absolute_path(library_path);
        let Ok(source_metadata) = fs::symlink_metadata(&source_path) else {
            record_missing_track(track);
            continue;
        };
        if !source_metadata.file_type().is_file() {
            record_missing_track(track);
            continue;
        }

        occupied_paths.remove(&source_relative_path);
        let plan = plan_consolidation_destination(
            &planner,
            &mut occupied_paths,
            library_path,
            &source_path,
            &track.metadata,
            &source_relative_path,
        )?;
        occupied_paths.insert(source_relative_path.clone());

        if plan.relative_path == source_relative_path {
            already_organized_tracks += 1;
            continue;
        }

        let destination_path = library_path.join(plan.relative_path.as_path());
        let mut updated_track = track.clone();
        updated_track.location = TrackLocation::available(plan.relative_path.clone());

        moves.push(PlannedLibraryConsolidationMove {
            track_id: track.id,
            source_path,
            destination_path,
            source_relative_path,
            destination_relative_path: plan.relative_path,
            updated_track,
        });
    }

    Ok(LibraryConsolidationPlan {
        moves,
        already_organized_tracks,
        missing_tracks,
        missing_track_updates,
    })
}

fn plan_managed_track_retarget(
    library_path: &Path,
    existing_tracks: &[Track],
    track: Track,
) -> ApplicationRuntimeResult<Option<PlannedLibraryConsolidationMove>> {
    let source_relative_path = track.location.relative_path.clone();
    let source_path = track.location.absolute_path(library_path);
    let source_metadata = fs::symlink_metadata(&source_path)
        .map_err(|_| ApplicationRuntimeError::LibraryConsolidationFailed)?;
    if !source_metadata.file_type().is_file() {
        return Err(ApplicationRuntimeError::LibraryConsolidationFailed);
    }

    let planner = ManagedTrackPathPlanner::default();
    let mut occupied_paths = existing_tracks
        .iter()
        .filter(|existing_track| existing_track.id != track.id)
        .map(|existing_track| existing_track.location.relative_path.clone())
        .collect::<BTreeSet<_>>();

    let plan = plan_consolidation_destination(
        &planner,
        &mut occupied_paths,
        library_path,
        &source_path,
        &track.metadata,
        &source_relative_path,
    )?;
    if plan.relative_path == source_relative_path {
        return Ok(None);
    }

    let destination_path = library_path.join(plan.relative_path.as_path());
    let mut updated_track = track;
    updated_track.location = TrackLocation::available(plan.relative_path.clone());

    Ok(Some(PlannedLibraryConsolidationMove {
        track_id: updated_track.id,
        source_path,
        destination_path,
        source_relative_path,
        destination_relative_path: plan.relative_path,
        updated_track,
    }))
}

fn plan_consolidation_destination(
    planner: &ManagedTrackPathPlanner,
    occupied_paths: &mut BTreeSet<TrackRelativePath>,
    library_path: &Path,
    source_path: &Path,
    metadata: &sustain_domain::TrackMetadata,
    current_relative_path: &TrackRelativePath,
) -> ApplicationRuntimeResult<sustain_domain::ManagedTrackPathPlan> {
    for _attempt in 0..10_000 {
        let plan = planner
            .plan(
                ManagedTrackPathInput {
                    metadata,
                    source_path,
                },
                occupied_paths,
            )
            .map_err(|_| ApplicationRuntimeError::LibraryConsolidationFailed)?;
        if &plan.relative_path == current_relative_path {
            occupied_paths.insert(plan.relative_path.clone());
            return Ok(plan);
        }
        if library_path.join(plan.relative_path.as_path()).exists() {
            occupied_paths.insert(plan.relative_path);
            continue;
        }
        occupied_paths.insert(plan.relative_path.clone());
        return Ok(plan);
    }

    Err(ApplicationRuntimeError::LibraryConsolidationFailed)
}

struct PlannedManagedImport {
    source_path: PathBuf,
    destination_path: PathBuf,
    relative_path: sustain_domain::TrackRelativePath,
    content_hash: TrackContentHash,
    metadata: sustain_domain::TrackMetadata,
    rating: Rating,
    file_size_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConsolidationJournalEntry {
    track_id: TrackId,
    source_relative_path: TrackRelativePath,
    destination_relative_path: TrackRelativePath,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FileMoveError {
    SourceUnavailable,
    SourceIsNotFile,
    DestinationHasNoParent,
    DestinationExists,
    CreateDestinationDirectoryFailed,
    LinkFailed,
    RemoveSourceFailed,
}

pub(super) fn recover_library_consolidation_journal(
    library_path: &Path,
    library_store: &dyn sustain_library_store::LibraryStore,
) -> ApplicationRuntimeResult<()> {
    let journal_path = consolidation_journal_path(library_path);
    if !journal_path.exists() {
        return Ok(());
    }

    let entries = read_consolidation_journal(library_path)?;
    for entry in &entries {
        recover_consolidation_journal_entry(library_path, library_store, entry)?;
    }

    remove_consolidation_journal_if_present(library_path)
}

fn recover_consolidation_journal_entry(
    library_path: &Path,
    library_store: &dyn sustain_library_store::LibraryStore,
    entry: &ConsolidationJournalEntry,
) -> ApplicationRuntimeResult<()> {
    let source_path = entry.source_relative_path.resolve(library_path);
    let destination_path = entry.destination_relative_path.resolve(library_path);
    let source_is_file = path_is_regular_file(&source_path);
    let destination_is_file = path_is_regular_file(&destination_path);

    match (source_is_file, destination_is_file) {
        (false, true) => {
            save_recovered_consolidation_track(library_store, entry)?;
        }
        (true, true) if paths_refer_to_same_file(&source_path, &destination_path) => {
            fs::remove_file(&source_path)
                .map_err(|_| ApplicationRuntimeError::LibraryConsolidationFailed)?;
            save_recovered_consolidation_track(library_store, entry)?;
        }
        (true, false) | (false, false) | (true, true) => {}
    }

    Ok(())
}

fn save_recovered_consolidation_track(
    library_store: &dyn sustain_library_store::LibraryStore,
    entry: &ConsolidationJournalEntry,
) -> ApplicationRuntimeResult<()> {
    let Some(mut track) = library_store
        .track(entry.track_id)
        .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?
    else {
        return Ok(());
    };

    track.location = TrackLocation::available(entry.destination_relative_path.clone());
    library_store
        .save_track(track)
        .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)
}

fn write_consolidation_journal(
    library_path: &Path,
    moves: &[PlannedLibraryConsolidationMove],
) -> ApplicationRuntimeResult<()> {
    let journal_path = consolidation_journal_path(library_path);
    if journal_path.exists() {
        return Err(ApplicationRuntimeError::LibraryConsolidationFailed);
    }

    let temporary_path = temporary_consolidation_journal_path(library_path);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .map_err(|_| ApplicationRuntimeError::LibraryConsolidationFailed)?;

    writeln!(file, "# sustain managed library consolidation journal v1")
        .map_err(|_| ApplicationRuntimeError::LibraryConsolidationFailed)?;
    for planned_move in moves {
        let source = encode_relative_path(&planned_move.source_relative_path);
        let destination = encode_relative_path(&planned_move.destination_relative_path);
        writeln!(
            file,
            "move\t{}\t{}\t{}",
            planned_move.track_id.get(),
            source,
            destination
        )
        .map_err(|_| ApplicationRuntimeError::LibraryConsolidationFailed)?;
    }
    file.flush()
        .map_err(|_| ApplicationRuntimeError::LibraryConsolidationFailed)?;
    file.sync_all()
        .map_err(|_| ApplicationRuntimeError::LibraryConsolidationFailed)?;
    fs::rename(&temporary_path, &journal_path)
        .map_err(|_| ApplicationRuntimeError::LibraryConsolidationFailed)?;

    Ok(())
}

fn read_consolidation_journal(
    library_path: &Path,
) -> ApplicationRuntimeResult<Vec<ConsolidationJournalEntry>> {
    let contents = fs::read_to_string(consolidation_journal_path(library_path))
        .map_err(|_| ApplicationRuntimeError::LibraryConsolidationFailed)?;
    let mut entries = Vec::new();

    for line in contents.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.split('\t');
        let Some("move") = parts.next() else {
            return Err(ApplicationRuntimeError::LibraryConsolidationFailed);
        };
        let track_id = parts
            .next()
            .and_then(|value| value.parse::<i64>().ok())
            .and_then(TrackId::new)
            .ok_or(ApplicationRuntimeError::LibraryConsolidationFailed)?;
        let source_relative_path = parts
            .next()
            .and_then(decode_relative_path)
            .ok_or(ApplicationRuntimeError::LibraryConsolidationFailed)?;
        let destination_relative_path = parts
            .next()
            .and_then(decode_relative_path)
            .ok_or(ApplicationRuntimeError::LibraryConsolidationFailed)?;
        if parts.next().is_some() {
            return Err(ApplicationRuntimeError::LibraryConsolidationFailed);
        }

        entries.push(ConsolidationJournalEntry {
            track_id,
            source_relative_path,
            destination_relative_path,
        });
    }

    Ok(entries)
}

fn remove_consolidation_journal_if_present(library_path: &Path) -> ApplicationRuntimeResult<()> {
    let journal_path = consolidation_journal_path(library_path);
    if !journal_path.exists() {
        return Ok(());
    }

    fs::remove_file(journal_path).map_err(|_| ApplicationRuntimeError::LibraryConsolidationFailed)
}

fn consolidation_journal_path(library_path: &Path) -> PathBuf {
    library_path.join(CONSOLIDATION_JOURNAL_FILE_NAME)
}

fn temporary_consolidation_journal_path(library_path: &Path) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    library_path.join(format!(
        ".sustain-consolidation-journal-{}-{unique}.tmp",
        std::process::id()
    ))
}

fn move_file_without_copy_or_overwrite(
    source_path: &Path,
    destination_path: &Path,
) -> Result<(), FileMoveError> {
    let source_metadata =
        fs::symlink_metadata(source_path).map_err(|_| FileMoveError::SourceUnavailable)?;
    if !source_metadata.file_type().is_file() {
        return Err(FileMoveError::SourceIsNotFile);
    }
    if destination_path.exists() {
        return Err(FileMoveError::DestinationExists);
    }

    let destination_parent = destination_path
        .parent()
        .ok_or(FileMoveError::DestinationHasNoParent)?;
    fs::create_dir_all(destination_parent)
        .map_err(|_| FileMoveError::CreateDestinationDirectoryFailed)?;
    if destination_path.exists() {
        return Err(FileMoveError::DestinationExists);
    }

    fs::hard_link(source_path, destination_path).map_err(|_| FileMoveError::LinkFailed)?;
    if fs::remove_file(source_path).is_err() {
        if paths_refer_to_same_file(source_path, destination_path) {
            let _ = fs::remove_file(destination_path);
        }
        return Err(FileMoveError::RemoveSourceFailed);
    }

    Ok(())
}

fn rollback_file_move(source_path: &Path, destination_path: &Path) -> Result<(), FileMoveError> {
    match (
        path_is_regular_file(source_path),
        path_is_regular_file(destination_path),
    ) {
        (true, true) if paths_refer_to_same_file(source_path, destination_path) => {
            fs::remove_file(destination_path).map_err(|_| FileMoveError::RemoveSourceFailed)
        }
        (true, false) => Ok(()),
        (false, true) => move_file_without_copy_or_overwrite(destination_path, source_path),
        _ => Err(FileMoveError::SourceUnavailable),
    }
}

fn path_is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_file())
        .unwrap_or(false)
}

fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    match (fs::metadata(left), fs::metadata(right)) {
        (Ok(left), Ok(right)) => left.dev() == right.dev() && left.ino() == right.ino(),
        _ => false,
    }
}

fn replace_library_tracks_by_id(library_tracks: &mut [Track], updated_tracks: Vec<Track>) {
    for updated_track in updated_tracks {
        if let Some(track) = library_tracks
            .iter_mut()
            .find(|track| track.id == updated_track.id)
        {
            *track = updated_track;
        }
    }
    library_tracks.sort_by_key(|track| track.id);
}

fn encode_relative_path(relative_path: &TrackRelativePath) -> String {
    relative_path
        .as_path()
        .as_os_str()
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn decode_relative_path(value: &str) -> Option<TrackRelativePath> {
    if value.len() % 2 != 0 {
        return None;
    }

    let bytes = value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let high = hex_value(chunk[0])?;
            let low = hex_value(chunk[1])?;
            Some((high << 4) | low)
        })
        .collect::<Option<Vec<_>>>()?;

    TrackRelativePath::new(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn collect_supported_audio_files(
    paths: &[PathBuf],
    cancellation: &AtomicBool,
) -> ApplicationRuntimeResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    for path in paths {
        if cancellation.load(Ordering::SeqCst) {
            return Ok(files);
        }
        collect_supported_audio_path(path, &mut files, cancellation)?;
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn collect_supported_audio_path(
    path: &Path,
    files: &mut Vec<PathBuf>,
    cancellation: &AtomicBool,
) -> ApplicationRuntimeResult<()> {
    if cancellation.load(Ordering::SeqCst) {
        return Ok(());
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;
    if metadata.file_type().is_dir() {
        let entries =
            fs::read_dir(path).map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;
        for entry in entries {
            if cancellation.load(Ordering::SeqCst) {
                return Ok(());
            }
            let entry = entry.map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;
            collect_supported_audio_path(&entry.path(), files, cancellation)?;
        }
    } else if metadata.file_type().is_file() && audio_format_from_path(path).is_ok() {
        files.push(path.to_path_buf());
    }
    Ok(())
}

fn cancelled_import_result(discovered_files: usize) -> LibraryImportResult {
    LibraryImportResult {
        tracks: Vec::new(),
        summary: LibraryImportSummary {
            discovered_files,
            imported_tracks: 0,
            duplicate_files: 0,
            cancelled: true,
        },
    }
}

fn plan_destination(
    planner: &ManagedTrackPathPlanner,
    occupied_paths: &mut BTreeSet<sustain_domain::TrackRelativePath>,
    library_path: &Path,
    source_path: &Path,
    metadata: &sustain_domain::TrackMetadata,
) -> ApplicationRuntimeResult<sustain_domain::ManagedTrackPathPlan> {
    for _attempt in 0..10_000 {
        let plan = planner
            .plan(
                ManagedTrackPathInput {
                    metadata,
                    source_path,
                },
                occupied_paths,
            )
            .map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;
        if library_path.join(plan.relative_path.as_path()).exists() {
            occupied_paths.insert(plan.relative_path);
            continue;
        }
        occupied_paths.insert(plan.relative_path.clone());
        return Ok(plan);
    }

    Err(ApplicationRuntimeError::LibraryImportFailed)
}

fn reference_relative_path_for_source(
    source_path: &Path,
    library_path: &Path,
) -> ApplicationRuntimeResult<sustain_domain::TrackRelativePath> {
    if let Ok(relative_path) = source_path.strip_prefix(library_path)
        && let Some(relative_path) =
            sustain_domain::TrackRelativePath::new(relative_path.to_path_buf())
    {
        return Ok(relative_path);
    }

    Err(ApplicationRuntimeError::LibraryImportFailed)
}

fn source_relative_path_inside_library(
    source_path: &Path,
    library_path: Option<&Path>,
) -> Option<sustain_domain::TrackRelativePath> {
    let library_path = library_path?;
    source_path
        .strip_prefix(library_path)
        .ok()
        .and_then(|relative_path| {
            sustain_domain::TrackRelativePath::new(relative_path.to_path_buf())
        })
}

fn remove_copied_files(paths: &[PathBuf]) {
    for path in paths.iter().rev() {
        let _ = fs::remove_file(path);
    }
}

fn copy_file_verified_inner(
    source_path: &Path,
    destination_path: &Path,
    temporary_path: &Path,
    expected_size: u64,
    source_permissions: fs::Permissions,
    expected_hash: &TrackContentHash,
) -> Result<VerifiedFileCopy, VerifiedFileCopyError> {
    let bytes_copied = copy_to_temporary_file(source_path, temporary_path)?;
    if bytes_copied != expected_size {
        return Err(VerifiedFileCopyError::SizeMismatch {
            expected: expected_size,
            actual: bytes_copied,
        });
    }

    fs::set_permissions(temporary_path, source_permissions)
        .map_err(|_| VerifiedFileCopyError::CopyFailed)?;

    let actual_hash =
        hash_file_content(temporary_path).map_err(|_| VerifiedFileCopyError::CopyFailed)?;
    if &actual_hash != expected_hash {
        return Err(VerifiedFileCopyError::HashMismatch {
            expected: expected_hash.clone(),
            actual: actual_hash,
        });
    }

    if destination_path.exists() {
        return Err(VerifiedFileCopyError::DestinationExists);
    }

    // `rename` replaces existing files on Unix. A hard link created in the same
    // directory gives us no-overwrite finalization: it fails if the destination
    // appeared while we were copying, and removing the temp name leaves the
    // finalized file intact.
    fs::hard_link(temporary_path, destination_path)
        .map_err(|_| VerifiedFileCopyError::FinalizeFailed)?;
    fs::remove_file(temporary_path).map_err(|_| VerifiedFileCopyError::FinalizeFailed)?;

    Ok(VerifiedFileCopy {
        destination_path: destination_path.to_path_buf(),
        bytes_copied,
        content_hash: expected_hash.clone(),
    })
}

fn copy_to_temporary_file(
    source_path: &Path,
    temporary_path: &Path,
) -> Result<u64, VerifiedFileCopyError> {
    let source =
        fs::File::open(source_path).map_err(|_| VerifiedFileCopyError::SourceUnavailable)?;
    let temporary = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary_path)
        .map_err(|_| VerifiedFileCopyError::CreateTemporaryFileFailed)?;

    let mut reader = BufReader::new(source);
    let mut writer = BufWriter::new(temporary);
    let bytes_copied =
        std::io::copy(&mut reader, &mut writer).map_err(|_| VerifiedFileCopyError::CopyFailed)?;
    writer
        .flush()
        .map_err(|_| VerifiedFileCopyError::CopyFailed)?;
    Ok(bytes_copied)
}

fn create_temporary_copy_path(destination_path: &Path) -> Result<PathBuf, VerifiedFileCopyError> {
    let parent = destination_path
        .parent()
        .ok_or(VerifiedFileCopyError::DestinationHasNoParent)?;
    let file_name = destination_path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .ok_or(VerifiedFileCopyError::DestinationHasNoParent)?;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();

    for attempt in 0..100u32 {
        let temporary_name = format!(
            ".{file_name}.sustain-copy-{}-{unique}-{attempt}.tmp",
            std::process::id()
        );
        let temporary_path = parent.join(temporary_name);
        if !temporary_path.exists() {
            return Ok(temporary_path);
        }
    }

    Err(VerifiedFileCopyError::CreateTemporaryFileFailed)
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::MetadataExt, path::PathBuf};

    use sustain_metadata::hash_file_content;

    use super::{FileMoveError, VerifiedFileCopyError, copy_file_verified};

    #[test]
    fn verified_copy_copies_and_verifies_file_before_finalizing() {
        let root = unique_test_directory();
        let source = root.join("source.flac");
        let destination = root.join("Artist").join("Album").join("01 Song.flac");
        fs::create_dir_all(&root).expect("create test directory");
        fs::write(&source, b"audio bytes").expect("write source");
        let hash = hash_file_content(&source).expect("hash source");

        let copy = copy_file_verified(&source, &destination, &hash).expect("copy succeeds");

        assert_eq!(copy.destination_path, destination);
        assert_eq!(copy.bytes_copied, 11);
        assert_eq!(copy.content_hash, hash);
        assert_eq!(
            fs::read(&copy.destination_path).expect("read dest"),
            b"audio bytes"
        );
        assert_no_temporary_files(&root);

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn verified_copy_refuses_to_overwrite_existing_destination() {
        let root = unique_test_directory();
        let source = root.join("source.flac");
        let destination = root.join("dest.flac");
        fs::create_dir_all(&root).expect("create test directory");
        fs::write(&source, b"new bytes").expect("write source");
        fs::write(&destination, b"existing bytes").expect("write destination");
        let hash = hash_file_content(&source).expect("hash source");

        let result = copy_file_verified(&source, &destination, &hash);

        assert_eq!(result, Err(VerifiedFileCopyError::DestinationExists));
        assert_eq!(
            fs::read(&destination).expect("read destination"),
            b"existing bytes"
        );
        assert_no_temporary_files(&root);

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn verified_copy_removes_temporary_file_on_hash_mismatch() {
        let root = unique_test_directory();
        let source = root.join("source.flac");
        let destination = root.join("dest.flac");
        fs::create_dir_all(&root).expect("create test directory");
        fs::write(&source, b"new bytes").expect("write source");
        let wrong_hash = sustain_domain::TrackContentHash::new(
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect("valid hash");

        let result = copy_file_verified(&source, &destination, &wrong_hash);

        assert!(matches!(
            result,
            Err(VerifiedFileCopyError::HashMismatch { .. })
        ));
        assert!(!destination.exists());
        assert_no_temporary_files(&root);

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn managed_move_uses_metadata_operations_and_refuses_overwrite() {
        let root = unique_test_directory();
        let source = root.join("source.flac");
        let destination = root.join("Artist").join("Album").join("01 Song.flac");
        fs::create_dir_all(&root).expect("create test directory");
        fs::write(&source, b"audio bytes").expect("write source");
        let source_metadata = fs::metadata(&source).expect("source metadata");

        super::move_file_without_copy_or_overwrite(&source, &destination).expect("move succeeds");

        assert!(!source.exists());
        let destination_metadata = fs::metadata(&destination).expect("destination metadata");
        assert_eq!(source_metadata.dev(), destination_metadata.dev());
        assert_eq!(source_metadata.ino(), destination_metadata.ino());

        let second_source = root.join("second.flac");
        fs::write(&second_source, b"other bytes").expect("write second source");
        assert_eq!(
            super::move_file_without_copy_or_overwrite(&second_source, &destination),
            Err(FileMoveError::DestinationExists)
        );
        assert!(second_source.exists());

        fs::remove_dir_all(root).expect("remove test directory");
    }

    fn assert_no_temporary_files(root: &std::path::Path) {
        let entries = fs::read_dir(root).expect("read test directory");
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            assert!(
                !name.contains(".sustain-copy-"),
                "temporary file left behind: {name}"
            );
        }
    }

    fn unique_test_directory() -> PathBuf {
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("sustain_managed_copy_test_{unique_suffix}"))
    }
}
