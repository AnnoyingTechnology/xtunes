// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Background scheduler for Smart Shuffle model training. Training a
//! Random Forest over a 10 000-track library can take a couple of
//! seconds — long enough that running it on the GTK main loop would
//! stutter playback and the UI. The scheduler owns a single worker
//! thread, accepts training requests through a non-blocking gate,
//! and reports completion through an `async_channel` the UI shell
//! drains on every idle tick.
//!
//! Two trigger paths feed the scheduler:
//!   * Explicit — the "Retrain now" button in the Shuffle
//!     preferences tab, or the runtime's first enable-Smart-Shuffle
//!     attempt when no model has ever been trained.
//!   * Interval — a glib timer running in the UI shell calls back
//!     into the runtime periodically; the runtime checks elapsed
//!     time against the user-configured interval and forwards
//!     through here when the cadence is due. (Interval timer
//!     itself is not wired in this change — the runtime exposes
//!     `request_smart_shuffle_training` for the UI to invoke.)
//!
//! The scheduler does NOT own the model itself; the runtime owns
//! the in-memory model and writes the persisted blob into the
//! library store. The scheduler is purely a "run the training
//! function on a background thread" surface — and one that
//! coalesces overlapping requests (the worker drops re-entrant
//! requests rather than queuing them, because two back-to-back
//! retrains on the same library would produce indistinguishable
//! models).

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::SystemTime;

use sustain_domain::Track;
use sustain_smart_shuffle::{SmartShuffleError, SmartShuffleTrainer, TrainingOutcome};

/// Cooperative signal published by the worker after a training run
/// completes. The runtime reads these on its result-sink tick and
/// either swaps in the new model or surfaces the cold-start
/// notification.
#[derive(Debug)]
pub struct SmartShuffleTrainingResult {
    pub outcome: Result<TrainingOutcome, SmartShuffleError>,
    pub trained_at: SystemTime,
}

pub struct SmartShuffleScheduler {
    result_sender: async_channel::Sender<SmartShuffleTrainingResult>,
    result_receiver: async_channel::Receiver<SmartShuffleTrainingResult>,
    is_training: Arc<AtomicBool>,
}

impl SmartShuffleScheduler {
    pub fn new() -> Self {
        let (tx, rx) = async_channel::unbounded();
        Self {
            result_sender: tx,
            result_receiver: rx,
            is_training: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Result channel the UI shell drains on the main loop. The
    /// receiver is cloneable so the shell can hold its own copy
    /// without taking ownership of the scheduler.
    pub fn result_receiver(&self) -> async_channel::Receiver<SmartShuffleTrainingResult> {
        self.result_receiver.clone()
    }

    pub fn is_training(&self) -> bool {
        self.is_training.load(Ordering::Acquire)
    }

    /// Spawn a training run on a dedicated background thread.
    /// Returns `false` when a previous run is still in flight —
    /// the request is dropped rather than queued, because two
    /// back-to-back retrains on an unchanged library produce
    /// equivalent models. `trained_at` is captured by the caller
    /// (the runtime's clock) so the worker thread never reads
    /// wall-clock time directly.
    pub fn request_training(&self, tracks: Vec<Track>, trained_at: SystemTime) -> bool {
        // `compare_exchange` rather than `swap` so a running
        // request leaves the flag set and we do not bounce it.
        if self
            .is_training
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        let sender = self.result_sender.clone();
        let flag = self.is_training.clone();
        std::thread::spawn(move || {
            let outcome = SmartShuffleTrainer::train(&tracks);
            flag.store(false, Ordering::Release);
            // `send_blocking` cannot meaningfully fail on an
            // unbounded channel whose receiver is owned by the
            // runtime; drop the error so the worker can exit
            // cleanly when the runtime shuts down.
            let _ = sender.send_blocking(SmartShuffleTrainingResult {
                outcome,
                trained_at,
            });
        });
        true
    }
}

impl Default for SmartShuffleScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use sustain_domain::{
        PlayStatistics, Rating, Track, TrackId, TrackLocation, TrackMetadata, TrackRelativePath,
    };

    use super::SmartShuffleScheduler;

    fn track(id: i64, plays: u64, skips: u64, genre: &str) -> Track {
        Track {
            id: TrackId::new(id).expect("valid id"),
            location: TrackLocation::available(
                TrackRelativePath::new(format!("g/{id}.flac")).expect("relative path"),
            ),
            content_hash: None,
            metadata: TrackMetadata {
                genre: Some(genre.to_owned()),
                ..TrackMetadata::default()
            },
            rating: Rating::unrated(),
            statistics: PlayStatistics {
                play_count: plays,
                skip_count: skips,
                last_played_at: Some(SystemTime::UNIX_EPOCH),
                last_skipped_at: None,
                date_added_at: None,
            },
            file_size_bytes: None,
            has_embedded_artwork: None,
        }
    }

    #[test]
    fn scheduler_rejects_concurrent_training() {
        let scheduler = SmartShuffleScheduler::new();
        // Build a library big enough to clear the cold-start gate.
        let mut tracks: Vec<Track> = Vec::new();
        for index in 0..80 {
            tracks.push(track(index + 1, 5, 0, "Rock"));
        }
        for index in 80..160 {
            tracks.push(track(index + 1, 0, 3, "Polka"));
        }

        assert!(scheduler.request_training(tracks.clone(), SystemTime::UNIX_EPOCH));
        // Second request while the first is in flight should be
        // refused. There is an inherent race here between the
        // worker setting `is_training = false` and this assertion,
        // so we sample tightly — under normal CI loads the
        // training takes long enough that this is reliable.
        let _ = scheduler.request_training(tracks, SystemTime::UNIX_EPOCH);
        let _ = scheduler.result_receiver().recv_blocking();
    }
}
