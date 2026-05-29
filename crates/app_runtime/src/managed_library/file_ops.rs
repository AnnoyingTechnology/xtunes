// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Filesystem primitives for the managed library: verified copies and
//! hard-link moves that never overwrite an existing destination, plus the
//! small `stat`-based helpers the import, consolidation, and journal-recovery
//! paths share.

use std::{
    fs,
    io::{BufReader, BufWriter, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use sustain_domain::TrackContentHash;
use sustain_metadata::hash_file_content;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct VerifiedFileCopy {
    pub(super) destination_path: PathBuf,
    pub(super) bytes_copied: u64,
    pub(super) content_hash: TrackContentHash,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum VerifiedFileCopyError {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum FileMoveError {
    SourceUnavailable,
    SourceIsNotFile,
    DestinationHasNoParent,
    DestinationExists,
    CreateDestinationDirectoryFailed,
    LinkFailed,
    RemoveSourceFailed,
}

pub(super) fn copy_file_verified(
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

pub(super) fn move_file_without_copy_or_overwrite(
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

pub(super) fn rollback_file_move(
    source_path: &Path,
    destination_path: &Path,
) -> Result<(), FileMoveError> {
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

pub(super) fn path_is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_file())
        .unwrap_or(false)
}

pub(super) fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    match (fs::metadata(left), fs::metadata(right)) {
        (Ok(left), Ok(right)) => left.dev() == right.dev() && left.ino() == right.ino(),
        _ => false,
    }
}

pub(super) fn remove_copied_files(paths: &[PathBuf]) {
    for path in paths.iter().rev() {
        let _ = fs::remove_file(path);
    }
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
