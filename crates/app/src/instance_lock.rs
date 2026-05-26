// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Single-instance enforcement keyed to the resolved library database path.
//!
//! Two Sustain processes pointed at the same on-disk library must not run
//! concurrently — the integrity rationale is library-wide (interleaved SQLite
//! writes across multi-statement invariants, racing tag/rating writes,
//! duplicate scan inserts, MPRIS bus-name collisions). This module owns two
//! complementary primitives:
//!
//! - [`acquire`] takes an `flock(LOCK_EX | LOCK_NB)` on a sidecar `.lock`
//!   file living next to the database file. The lock is released when the
//!   returned [`InstanceLock`] is dropped (process exit, including crashes,
//!   since the kernel closes file descriptors on `exit_group`). It uses a
//!   sidecar rather than the database file itself so the lock cannot be
//!   confused with SQLite's own POSIX-fcntl transaction locks.
//! - [`application_id_for`] derives a stable GTK application id from the
//!   resolved database path so a dev build using `--local-scope` and a
//!   system-installed Sustain pointed at the user's real library resolve
//!   to two distinct GTK applications — neither single-instance check
//!   fires across them — while two processes targeting the same database
//!   produce the same id, letting GTK route the second activation to the
//!   existing window.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use rustix::fs::{FlockOperation, flock};

/// Stable reverse-DNS prefix shared by every Sustain GTK application id, so
/// the database-keyed suffix can be appended without colliding with other
/// applications. The fixed `db_` element in front of the hex hash also
/// ensures the resulting GApplication id satisfies the
/// "element starts with a letter" preference (purely cosmetic — GTK accepts
/// digit-leading elements — but easier to read in `busctl` listings).
const GTK_APPLICATION_ID_PREFIX: &str = "io.github.open_sustain.sustain.db_";

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

/// Derive the GTK application id from the resolved database path. The
/// returned string is the literal value passed to
/// `gtk::Application::application_id` and must be both stable across the
/// process's two roles (primary + remote) and unique per distinct database
/// target, so the contract documented on [`acquire`] holds.
pub(crate) fn application_id_for(database_path: &Path) -> String {
    let hash = fnv1a_64(database_path.as_os_str().as_encoded_bytes());
    format!("{GTK_APPLICATION_ID_PREFIX}{hash:016x}")
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

/// FNV-1a 64-bit. Picked over `std::collections::hash_map::DefaultHasher`
/// because the standard hasher's algorithm is explicitly documented as
/// unstable across Rust versions — we need the same hash for every Sustain
/// binary built with the same source, regardless of which `rustc` produced
/// it. FNV-1a is a fixed reference algorithm with deterministic constants,
/// so two side-by-side builds (e.g. a dev `cargo run` and the system `.deb`)
/// derive identical application ids from the same database path.
fn fnv1a_64(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in data {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
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
    fn application_id_is_stable_for_the_same_path() {
        let id_first =
            application_id_for(Path::new("/home/user/.local/share/sustain/library.sqlite"));
        let id_second =
            application_id_for(Path::new("/home/user/.local/share/sustain/library.sqlite"));
        assert_eq!(id_first, id_second);
    }

    #[test]
    fn application_id_differs_for_distinct_database_paths() {
        let real_library =
            application_id_for(Path::new("/home/user/.local/share/sustain/library.sqlite"));
        let dev_sandbox = application_id_for(Path::new("/home/user/checkout/sustain.sqlite"));
        assert_ne!(real_library, dev_sandbox);
    }

    #[test]
    fn application_id_uses_the_shared_reverse_dns_prefix() {
        let id = application_id_for(Path::new("/tmp/library.sqlite"));
        assert!(id.starts_with(GTK_APPLICATION_ID_PREFIX));
        assert!(
            id.len() <= 255,
            "GApplication ids must fit within 255 chars"
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
