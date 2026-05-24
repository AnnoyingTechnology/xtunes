// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{
    collections::{BTreeSet, HashSet},
    fs,
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use sustain_domain::{
    LibraryManagementMode, ManagedTrackPathInput, ManagedTrackPathPlanner, PlayStatistics, Rating,
    Track, TrackContentHash, TrackLocation,
};
use sustain_metadata::{audio_format_from_path, hash_file_content};

use crate::{
    ApplicationRuntime, ApplicationRuntimeError, ApplicationRuntimeResult, LibraryImportSummary,
    library_scan,
};

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
        if paths.is_empty() {
            self.last_library_import_summary = Some(LibraryImportSummary::default());
            return Ok(());
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
    ) -> ApplicationRuntimeResult<()> {
        let library_path = self
            .settings
            .library_path()
            .ok_or(ApplicationRuntimeError::LibraryPathUnavailable)?
            .to_path_buf();
        let library_store = self
            .library_store
            .clone()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;
        let metadata_service = self
            .metadata_service
            .clone()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;

        let discovered_files = collect_supported_audio_files(&paths)?;
        let mut occupied_paths = self
            .library_tracks
            .iter()
            .filter_map(|track| track.location.library_relative_path().cloned())
            .collect::<BTreeSet<_>>();
        let mut seen_hashes = self
            .library_tracks
            .iter()
            .filter_map(|track| track.content_hash.as_ref().map(TrackContentHash::as_str))
            .map(str::to_owned)
            .collect::<HashSet<_>>();

        let planner = ManagedTrackPathPlanner::default();
        let mut imports = Vec::new();
        let mut duplicate_files = 0;

        for source_path in &discovered_files {
            let content_hash = hash_file_content(source_path)
                .map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;
            if seen_hashes.contains(content_hash.as_str())
                || library_store
                    .track_by_content_hash(&content_hash)
                    .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?
                    .is_some()
            {
                duplicate_files += 1;
                continue;
            }

            let metadata = metadata_service
                .read_metadata(source_path)
                .map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;
            let rating = metadata_service
                .read_rating(source_path)
                .map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?
                .unwrap_or_else(Rating::unrated);
            let plan = plan_destination(
                &planner,
                &mut occupied_paths,
                &library_path,
                source_path,
                &metadata,
            )?;
            seen_hashes.insert(content_hash.as_str().to_owned());
            imports.push(PlannedManagedImport {
                source_path: source_path.clone(),
                destination_path: library_path.join(plan.relative_path.as_path()),
                relative_path: plan.relative_path,
                content_hash,
                metadata,
                rating,
            });
        }

        let mut copied_paths = Vec::new();
        for import in &imports {
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

        let mut next_track_id = library_scan::next_track_id(&self.library_tracks)?;
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
            });
        }

        if library_store.save_tracks(&tracks).is_err() {
            remove_copied_files(&copied_paths);
            return Err(ApplicationRuntimeError::LibraryStoreFailed);
        }

        self.library_tracks.extend(tracks);
        self.library_tracks.sort_by_key(|track| track.id);
        self.refresh_playback_queue_track_ids();
        self.last_library_import_summary = Some(LibraryImportSummary {
            discovered_files: discovered_files.len(),
            imported_tracks: copied_paths.len(),
            duplicate_files,
        });

        Ok(())
    }

    fn add_referenced_external_library_items(
        &mut self,
        paths: Vec<PathBuf>,
    ) -> ApplicationRuntimeResult<()> {
        let library_store = self
            .library_store
            .clone()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;
        let metadata_service = self
            .metadata_service
            .clone()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;

        let discovered_files = collect_supported_audio_files(&paths)?;
        let mut seen_locations = self
            .library_tracks
            .iter()
            .map(|track| track.location.file_path.clone())
            .collect::<HashSet<_>>();
        let mut seen_hashes = self
            .library_tracks
            .iter()
            .filter_map(|track| track.content_hash.as_ref().map(TrackContentHash::as_str))
            .map(str::to_owned)
            .collect::<HashSet<_>>();

        let mut next_track_id = library_scan::next_track_id(&self.library_tracks)?;
        let mut tracks = Vec::new();
        let mut duplicate_files = 0;

        for source_path in &discovered_files {
            let source_path = fs::canonicalize(source_path)
                .map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;
            let location =
                reference_location_for_source(&source_path, self.settings.library_path())?;
            if !seen_locations.insert(location.file_path.clone()) {
                duplicate_files += 1;
                continue;
            }

            let content_hash = hash_file_content(&source_path)
                .map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;
            if seen_hashes.contains(content_hash.as_str())
                || library_store
                    .track_by_content_hash(&content_hash)
                    .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?
                    .is_some()
            {
                duplicate_files += 1;
                continue;
            }

            let metadata = metadata_service
                .read_metadata(&source_path)
                .map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;
            let rating = metadata_service
                .read_rating(&source_path)
                .map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?
                .unwrap_or_else(Rating::unrated);

            let Some(track_id) = sustain_domain::TrackId::new(next_track_id) else {
                return Err(ApplicationRuntimeError::LibraryStoreFailed);
            };
            next_track_id += 1;
            seen_hashes.insert(content_hash.as_str().to_owned());
            tracks.push(Track {
                id: track_id,
                location,
                content_hash: Some(content_hash),
                metadata,
                rating,
                statistics: PlayStatistics {
                    date_added_at: Some(SystemTime::now()),
                    ..PlayStatistics::default()
                },
            });
        }

        if library_store.save_tracks(&tracks).is_err() {
            return Err(ApplicationRuntimeError::LibraryStoreFailed);
        }

        let imported_tracks = tracks.len();
        self.library_tracks.extend(tracks);
        self.library_tracks.sort_by_key(|track| track.id);
        self.refresh_playback_queue_track_ids();
        self.last_library_import_summary = Some(LibraryImportSummary {
            discovered_files: discovered_files.len(),
            imported_tracks,
            duplicate_files,
        });

        Ok(())
    }
}

struct PlannedManagedImport {
    source_path: PathBuf,
    destination_path: PathBuf,
    relative_path: sustain_domain::TrackRelativePath,
    content_hash: TrackContentHash,
    metadata: sustain_domain::TrackMetadata,
    rating: Rating,
}

fn collect_supported_audio_files(paths: &[PathBuf]) -> ApplicationRuntimeResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    for path in paths {
        collect_supported_audio_path(path, &mut files)?;
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn collect_supported_audio_path(
    path: &Path,
    files: &mut Vec<PathBuf>,
) -> ApplicationRuntimeResult<()> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;
    if metadata.file_type().is_dir() {
        let entries =
            fs::read_dir(path).map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;
        for entry in entries {
            let entry = entry.map_err(|_| ApplicationRuntimeError::LibraryImportFailed)?;
            collect_supported_audio_path(&entry.path(), files)?;
        }
    } else if metadata.file_type().is_file() && audio_format_from_path(path).is_ok() {
        files.push(path.to_path_buf());
    }
    Ok(())
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

fn reference_location_for_source(
    source_path: &Path,
    library_path: Option<&Path>,
) -> ApplicationRuntimeResult<TrackLocation> {
    if let Some(library_path) = library_path {
        let library_path =
            fs::canonicalize(library_path).unwrap_or_else(|_| library_path.to_path_buf());
        if let Ok(relative_path) = source_path.strip_prefix(&library_path)
            && let Some(relative_path) =
                sustain_domain::TrackRelativePath::new(relative_path.to_path_buf())
        {
            return Ok(TrackLocation::available(relative_path));
        }
    }

    TrackLocation::available_external(source_path.to_path_buf())
        .ok_or(ApplicationRuntimeError::LibraryImportFailed)
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
    use std::{fs, path::PathBuf};

    use sustain_metadata::hash_file_content;

    use super::{VerifiedFileCopyError, copy_file_verified};

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
