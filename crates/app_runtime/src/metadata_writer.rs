// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Off-thread metadata writer.
//!
//! Tag writes (rating, metadata fields, embedded artwork) used to run
//! synchronously on the GTK main thread, which made a single star click
//! block the UI for hundreds of milliseconds — orders of magnitude worse
//! when an embedded cover image had to be rewritten. This module owns a
//! dedicated worker thread that drains a queue of [`MetadataWriteJob`]s
//! against the project's [`MetadataService`], so the UI can apply its
//! optimistic in-memory + SQLite update and return immediately.
//!
//! This single worker is also the *only* tag-write surface for the
//! whole application: background workers (online retrieval, future
//! analysis tag mirroring) must route through [`MetadataWriteHandle`]
//! rather than calling [`MetadataService::write_metadata`] directly.
//! That serialisation prevents two threads from racing on the same
//! file's tags (e.g. a UI rating click landing while the online worker
//! is mid-way through a lyrics fill).
//!
//! ## Lifecycle
//!
//! The worker is started by [`MetadataWriter::start`] and runs until its
//! request channel is closed; [`MetadataWriter::shutdown`] drops the
//! sender and joins the worker, draining any queued jobs before
//! returning. The runtime is responsible for calling `shutdown` during
//! app teardown so no in-flight rating click is lost. Because every
//! outstanding [`MetadataWriteHandle`] holds a sender clone, the
//! shutdown sequence must drop all consumers (online scheduler,
//! analysis scheduler if it ever grows tag writes) **before** calling
//! [`MetadataWriter::shutdown`], or the join will hang.
//!
//! ## Failure surface
//!
//! Per-job completion is forwarded through a caller-supplied
//! [`WriteCompletionCallback`]; it runs on the worker thread, so the
//! caller is responsible for marshalling back to the UI's main loop
//! (typically by posting through an `async_channel` consumed by a
//! `glib::MainContext::spawn_local`). The completion only reports
//! success/failure — it does not carry the underlying error, since the
//! UI's recourse is the same in either failure mode: surface a message
//! to the user and let the next library scan reconcile state.
//!
//! Blocking callers use [`MetadataWriteHandle`] instead: each call
//! enqueues the job, blocks the calling thread on a one-shot rendezvous
//! channel, and returns the outcome. The handle is cloneable and
//! `Send + Sync`, suitable for background workers that need to await
//! the result before deciding how to mirror state into SQLite.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Sender, SyncSender},
    },
    thread::{self, JoinHandle},
};

use sustain_domain::{MetadataChange, Rating, TrackId};
use sustain_metadata::MetadataService;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataWriteOutcome {
    Succeeded,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataWriteKind {
    Rating,
    Metadata,
    Artwork,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataWriteResult {
    pub track_id: TrackId,
    pub kind: MetadataWriteKind,
    pub outcome: MetadataWriteOutcome,
}

pub(crate) enum MetadataWriteJob {
    Rating {
        path: PathBuf,
        rating: Rating,
    },
    Metadata {
        path: PathBuf,
        change: Box<MetadataChange>,
    },
    Artwork {
        path: PathBuf,
        artwork: Option<Vec<u8>>,
    },
}

pub(crate) type WriteCompletionCallback = Box<dyn FnOnce(MetadataWriteOutcome) + Send + 'static>;

pub(crate) struct MetadataWriteRequest {
    pub(crate) job: MetadataWriteJob,
    pub(crate) completion: WriteCompletionCallback,
}

/// Owns a worker thread that serialises tag writes against the
/// underlying [`MetadataService`]. Cloneable handles to submit work
/// share the same channel — there is exactly one worker per writer.
pub(crate) struct MetadataWriter {
    sender: Sender<MetadataWriteRequest>,
    handle: Option<JoinHandle<()>>,
}

impl MetadataWriter {
    pub(crate) fn start(metadata_service: Arc<dyn MetadataService>) -> Self {
        let (sender, receiver) = mpsc::channel::<MetadataWriteRequest>();
        let handle = thread::Builder::new()
            .name("sustain-metadata-writer".to_owned())
            .spawn(move || worker_loop(receiver, metadata_service))
            .expect("spawn metadata writer thread");
        Self {
            sender,
            handle: Some(handle),
        }
    }

    pub(crate) fn submit(&self, request: MetadataWriteRequest) {
        // The worker thread only exits when the channel is closed, which
        // only happens during `shutdown`. A send failure here therefore
        // means the writer is being torn down concurrently with a
        // mutation; the right thing is to drop the request silently —
        // the in-memory + SQLite state already reflects the desired
        // outcome, and the next scan will reconcile.
        let _ = self.sender.send(request);
    }

    /// Returns a cloneable handle suitable for background workers that
    /// need synchronous tag writes serialised against the UI.
    pub(crate) fn handle(&self) -> MetadataWriteHandle {
        MetadataWriteHandle {
            sender: self.sender.clone(),
        }
    }

    /// Drops the sender and waits for the worker to drain its queue and
    /// exit. The caller must have already dropped every
    /// [`MetadataWriteHandle`] clone — outstanding clones keep the
    /// channel alive and the join would hang.
    pub(crate) fn shutdown(mut self) {
        // Close the channel by dropping the sender; the worker's `recv`
        // returns `Err` once the queue is empty AND every clone has
        // been dropped.
        let (placeholder, _) = mpsc::channel();
        let _ = std::mem::replace(&mut self.sender, placeholder);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Cloneable handle to the [`MetadataWriter`] actor for callers that
/// need to block on the result of a single tag write. Used by
/// background workers (online retrieval, future analysis tag mirroring)
/// so every tag write — UI or background — flows through the same
/// single worker thread and cannot race on a shared file.
#[derive(Clone)]
pub(crate) struct MetadataWriteHandle {
    sender: Sender<MetadataWriteRequest>,
}

impl MetadataWriteHandle {
    pub(crate) fn write_metadata(&self, path: PathBuf, change: MetadataChange) -> bool {
        self.submit_blocking(MetadataWriteJob::Metadata {
            path,
            change: Box::new(change),
        })
    }

    pub(crate) fn write_artwork(&self, path: PathBuf, artwork: Option<Vec<u8>>) -> bool {
        self.submit_blocking(MetadataWriteJob::Artwork { path, artwork })
    }

    #[allow(dead_code)]
    pub(crate) fn write_rating(&self, path: PathBuf, rating: Rating) -> bool {
        self.submit_blocking(MetadataWriteJob::Rating { path, rating })
    }

    fn submit_blocking(&self, job: MetadataWriteJob) -> bool {
        // sync_channel(1) gives the worker a slot to drop the outcome
        // into without blocking, even though we are guaranteed to be
        // recv()ing — using bound 1 (rather than 0) means the worker
        // does not have to rendezvous with this thread to deliver the
        // result.
        let (tx, rx): (SyncSender<MetadataWriteOutcome>, _) = mpsc::sync_channel(1);
        let completion: WriteCompletionCallback = Box::new(move |outcome| {
            let _ = tx.send(outcome);
        });
        if self
            .sender
            .send(MetadataWriteRequest { job, completion })
            .is_err()
        {
            // Writer is gone (shutdown in progress). Treat as failure;
            // the caller will log and SQLite/in-memory state will be
            // reconciled by the next library scan.
            return false;
        }
        matches!(rx.recv(), Ok(MetadataWriteOutcome::Succeeded))
    }
}

impl Drop for MetadataWriter {
    fn drop(&mut self) {
        // Best-effort cleanup if `shutdown` was not called: drop the
        // sender so the worker exits, but don't block on join — Drop
        // running on the GTK main thread must not stall the app.
        let (placeholder, _) = mpsc::channel();
        let _ = std::mem::replace(&mut self.sender, placeholder);
    }
}

fn worker_loop(
    receiver: mpsc::Receiver<MetadataWriteRequest>,
    metadata_service: Arc<dyn MetadataService>,
) {
    while let Ok(request) = receiver.recv() {
        let result = match request.job {
            MetadataWriteJob::Rating { path, rating } => {
                metadata_service.write_rating(&path, rating)
            }
            MetadataWriteJob::Metadata { path, change } => {
                metadata_service.write_metadata(&path, *change)
            }
            MetadataWriteJob::Artwork { path, artwork } => {
                metadata_service.write_artwork(&path, artwork)
            }
        };
        let outcome = match result {
            Ok(()) => MetadataWriteOutcome::Succeeded,
            Err(_) => MetadataWriteOutcome::Failed,
        };
        (request.completion)(outcome);
    }
}
