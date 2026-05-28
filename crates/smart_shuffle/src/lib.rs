// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Smart Shuffle for Sustain — trains a Random Forest engagement
//! classifier on the user's library and uses it together with a
//! deterministic similarity-to-seed scorer to pick the next track
//! during Smart Shuffle playback.
//!
//! This crate intentionally has no Sustain runtime dependency: it
//! borrows `&[Track]` slices, produces a [`TrainingOutcome`] or a
//! [`PickedTrack`], and otherwise stays pure. The runtime
//! (`sustain_app_runtime`) is responsible for scheduling training,
//! persisting the model blob via the library store, and feeding
//! pick context to [`pick_next_track`]. See `docs/features.md` for
//! the user-facing description.
//!
//! ## Cold start
//!
//! Until [`SmartShuffleTrainer::train`] has succeeded once (at
//! least [`MIN_LABELED_TRACKS`] tracks with clear positive /
//! negative engagement signals), the picker still works — it just
//! treats every candidate's engagement probability as the neutral
//! 0.5 baseline and lets the similarity-to-seed signal carry the
//! decision. The runtime surfaces a notification on first Smart
//! Shuffle enable while in this state.
//!
//! ## Determinism
//!
//! Both training and picking are seeded by inputs the runtime
//! controls — the library content for training, the current seed
//! track id plus history length for picking — so identical
//! inputs always produce identical outputs. This makes the
//! `SUSTAIN_LOG_SMART_SHUFFLE=1` debug surface useful even after
//! the fact.

#![forbid(unsafe_code)]

pub mod feature;
pub mod forest;
mod model;
pub mod picker;
pub mod similarity;
mod trainer;

pub use feature::{FEATURE_SCHEMA_VERSION, FeatureExtractor, FeatureVector};
pub use forest::{ForestHyperparameters, RandomForest};
pub use model::SmartShuffleModel;
pub use picker::{
    PickContext, PickDebug, PickDebugEntry, PickMode, PickedTrack, format_debug, pick_next_track,
};
pub use trainer::{MIN_LABELED_TRACKS, SmartShuffleTrainer, TrainingOutcome, label_for_track};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SmartShuffleError {
    /// Library does not (yet) contain enough labelled tracks to
    /// train a model. The runtime translates this into the
    /// cold-start notification surfaced to the user.
    InsufficientTrainingData { positives: u32, negatives: u32 },
    /// Stored model blob could not be decoded — either it was
    /// written by a different `FEATURE_SCHEMA_VERSION` or the row
    /// is corrupt. Either way the runtime clears the row and
    /// schedules a retrain.
    ModelDeserialisationFailed,
    /// Failure while persisting a newly trained model. Bubbled up
    /// to the runtime so it can surface a notification rather than
    /// silently dropping the model.
    ModelSerialisationFailed,
}
