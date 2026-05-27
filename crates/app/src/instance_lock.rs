// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Filesystem-level single-instance enforcement.
//!
//! Sustain is a single-instance application: the GTK application id is the
//! fixed reverse-DNS string defined in `main.rs`, so GApplication's own
//! D-Bus uniqueness check routes any second activation to the first
//! instance's window. This module is the belt-and-suspenders layer that
//! covers the cases GApplication cannot — most importantly the integrity
//! rationale itself (interleaved SQLite writes across multi-statement
//! invariants, racing tag/rating writes, duplicate scan inserts, MPRIS
//! bus-name collisions) when two processes somehow bypass the D-Bus
//! check (different session buses, manual `GApplicationFlags`, NFS-mounted
//! libraries, etc.).
//!
//! [`acquire`] takes an `flock(LOCK_EX | LOCK_NB)` on a sidecar `.lock`
//! file living next to the database file. The lock is released when the
//! returned [`InstanceLock`] is dropped (process exit, including crashes,
//! since the kernel closes file descriptors on `exit_group`). It uses a
//! sidecar rather than the database file itself so the lock cannot be
//! confused with SQLite's own POSIX-fcntl transaction locks.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use rustix::fs::{FlockOperation, flock};

/// Holds the live lock file descriptor for the process lifetime. Dropping the
/// value (process exit, panic-unwind to `main`) releases the kernel-level
/// flock automatically; we intentionally do not unlock explicitly because
/// the OS-level release on `close(2)` is the canonical contract and a manual
/// `flock(LOCK_UN)` would be redundant.
#[must_use = "the instance lock is released when this value is dropped"]
pub(crate) struct InstanceLock {
    _file: File,
}

/// Outcome of a single-instance acquire attempt. The `Held` arm carries the
/// path of the lock file that was found locked, so callers can include it in
/// diagnostic messages without re-deriving the path.
pub(crate) enum AcquireOutcome {
    Acquired(InstanceLock),
    Held {
        lock_path: PathBuf,
    },
    Failed {
        lock_path: PathBuf,
        error: io::Error,
    },
}

/// Try to acquire the per-database single-instance lock.
///
/// The lock file is a zero-byte sidecar at `<database_path>.lock`. The parent
/// directory is created if it does not yet exist — the typical case is a
/// fresh install where the SQLite file has not been opened yet, so the XDG
/// data directory may not be on disk at acquire time.
pub(crate) fn acquire(database_path: &Path) -> AcquireOutcome {
    let lock_path = lock_path_for(database_path);

    if let Some(parent) = lock_path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        return AcquireOutcome::Failed { lock_path, error };
    }

    let file = match OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
    {
        Ok(file) => file,
        Err(error) => return AcquireOutcome::Failed { lock_path, error },
    };

    match flock(&file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => AcquireOutcome::Acquired(InstanceLock { _file: file }),
        Err(errno) if errno == rustix::io::Errno::WOULDBLOCK => AcquireOutcome::Held { lock_path },
        Err(errno) => AcquireOutcome::Failed {
            lock_path,
            error: io::Error::from(errno),
        },
    }
}

fn lock_path_for(database_path: &Path) -> PathBuf {
    let parent = database_path.parent().unwrap_or_else(|| Path::new("."));
    let mut file_name = database_path
        .file_name()
        .map(std::ffi::OsString::from)
        .unwrap_or_default();
    file_name.push(".lock");
    parent.join(file_name)
}

#[cfg(test)]
#[allow(clippy::panic, reason = "test failures use panic! to report context")]
mod tests {
    use super::*;

    #[test]
    fn sidecar_lock_path_lives_next_to_the_database() {
        let path = Path::new("/var/lib/sustain/library.sqlite");
        assert_eq!(
            lock_path_for(path),
            PathBuf::from("/var/lib/sustain/library.sqlite.lock")
        );
    }

    #[test]
    fn second_acquire_against_the_same_path_reports_held() {
        let dir =
            std::env::temp_dir().join(format!("sustain_instance_lock_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let db_path = dir.join("library.sqlite");

        let first = match acquire(&db_path) {
            AcquireOutcome::Acquired(lock) => lock,
            other => panic!(
                "first acquire should succeed; got {}",
                describe_outcome(&other)
            ),
        };

        match acquire(&db_path) {
            AcquireOutcome::Held { lock_path } => {
                assert_eq!(lock_path, lock_path_for(&db_path));
            }
            other => panic!(
                "second acquire should report Held; got {}",
                describe_outcome(&other)
            ),
        }

        drop(first);

        // After releasing the first lock, a fresh acquire should succeed.
        match acquire(&db_path) {
            AcquireOutcome::Acquired(_) => {}
            other => panic!(
                "post-release acquire should succeed; got {}",
                describe_outcome(&other)
            ),
        }

        std::fs::remove_dir_all(&dir).expect("clean up temp dir");
    }

    fn describe_outcome(outcome: &AcquireOutcome) -> &'static str {
        match outcome {
            AcquireOutcome::Acquired(_) => "Acquired",
            AcquireOutcome::Held { .. } => "Held",
            AcquireOutcome::Failed { .. } => "Failed",
        }
    }
}
