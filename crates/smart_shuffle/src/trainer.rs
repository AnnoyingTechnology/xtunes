// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Smart Shuffle trainer entry point. Reads the live library, picks
//! out the labelled subset (positives + negatives — see
//! [`label_for_track`] for the cutoff rules), runs feature
//! extraction, then hands the dense matrix to the Random Forest
//! builder.
//!
//! The trainer is intentionally pure: it borrows `&[Track]`,
//! produces a [`TrainingOutcome`], and is otherwise side-effect
//! free. The runtime is the one that decides when to run it
//! (background timer, "Retrain now" button, after an import) and
//! where to write the resulting blob.

use sustain_domain::Track;

use crate::SmartShuffleError;
use crate::feature::FeatureExtractor;
use crate::forest::{ForestHyperparameters, RandomForest};
use crate::model::SmartShuffleModel;

/// Default cold-start gate. Smart Shuffle refuses to train until
/// the library contains at least this many *labelled* tracks
/// (positives + negatives combined). Picked empirically — fewer
/// than this and the trees memorise individual examples instead
/// of learning useful structure. The runtime surfaces this
/// boundary in the cold-start notification.
pub const MIN_LABELED_TRACKS: u32 = 100;

/// Result of a successful training run. The runtime wraps it in a
/// `StoredSmartShuffleModel` (with `trained_at_unix`) before
/// handing it to the library store; that timestamp belongs to the
/// caller because the trainer crate has no business reading the
/// wall clock.
#[derive(Clone, Debug)]
pub struct TrainingOutcome {
    pub model: SmartShuffleModel,
    pub positive_label_count: u32,
    pub negative_label_count: u32,
}

pub struct SmartShuffleTrainer;

impl SmartShuffleTrainer {
    /// Train a new model from the supplied library. Tracks without
    /// a clear positive or negative signal are excluded from the
    /// labelled set — see [`label_for_track`]. Returns
    /// `Err(InsufficientTrainingData)` when too few labels are
    /// available; the runtime surfaces this as the cold-start
    /// notification.
    pub fn train(tracks: &[Track]) -> Result<TrainingOutcome, SmartShuffleError> {
        Self::train_with_hyperparameters_and_seed(
            tracks,
            ForestHyperparameters::default(),
            training_seed_from_library(tracks),
        )
    }

    /// Test seam: train with explicit hyperparameters and PRNG seed
    /// so unit tests can reproduce a deterministic outcome.
    pub fn train_with_hyperparameters_and_seed(
        tracks: &[Track],
        hyperparameters: ForestHyperparameters,
        seed: u64,
    ) -> Result<TrainingOutcome, SmartShuffleError> {
        let labels = collect_labels(tracks);
        let (positive_count, negative_count) = count_labels(&labels);
        if (positive_count + negative_count) < MIN_LABELED_TRACKS {
            return Err(SmartShuffleError::InsufficientTrainingData {
                positives: positive_count,
                negatives: negative_count,
            });
        }
        if positive_count == 0 || negative_count == 0 {
            // Without at least one of each class the forest cannot
            // learn a meaningful split — flag the same cold-start
            // outcome rather than producing a degenerate model.
            return Err(SmartShuffleError::InsufficientTrainingData {
                positives: positive_count,
                negatives: negative_count,
            });
        }

        let extractor = FeatureExtractor::build(tracks);
        let mut feature_matrix: Vec<Vec<f32>> = Vec::with_capacity(labels.len());
        let mut label_vector: Vec<bool> = Vec::with_capacity(labels.len());
        for (track, label) in labels {
            feature_matrix.push(extractor.extract(track).0);
            label_vector.push(label);
        }

        let forest = RandomForest::train(&feature_matrix, &label_vector, hyperparameters, seed);
        Ok(TrainingOutcome {
            model: SmartShuffleModel::new(extractor, forest),
            positive_label_count: positive_count,
            negative_label_count: negative_count,
        })
    }
}

/// Assign a binary engagement label to a track. The rules are
/// intentionally conservative:
///   * Positive — track has been played at least three times AND
///     has not been skipped more often than played. This rules out
///     "I auto-played it once" noise while keeping any track the
///     user actually finishes more than skips.
///   * Negative — track has been skipped more often than played
///     AND has at least two skip events. Captures the "I keep
///     skipping this one" pattern.
///   * `None` — anything else. The model is not given mixed signals
///     to learn from.
///
/// Rating is intentionally NOT used as a label source here: the
/// engagement classifier is meant to capture *behavioural* signals,
/// not stated preferences. The similarity-to-seed term already
/// uses rating as a feature; pulling rating into labels would
/// double-count it.
pub fn label_for_track(track: &Track) -> Option<bool> {
    let plays = track.statistics.play_count;
    let skips = track.statistics.skip_count;
    if plays >= 3 && plays >= skips {
        return Some(true);
    }
    if skips >= 2 && skips > plays {
        return Some(false);
    }
    None
}

fn collect_labels(tracks: &[Track]) -> Vec<(&Track, bool)> {
    tracks
        .iter()
        .filter(|track| !track.location.is_missing())
        .filter_map(|track| label_for_track(track).map(|label| (track, label)))
        .collect()
}

fn count_labels(labels: &[(&Track, bool)]) -> (u32, u32) {
    let positives = labels.iter().filter(|(_, label)| *label).count() as u32;
    let negatives = labels.len() as u32 - positives;
    (positives, negatives)
}

/// Pick a stable training seed from the library content. Used so
/// retraining produces the same model when the library hasn't
/// changed, and a different model when the labelled set evolves.
/// We sum the track ids and label counts — coarse but enough to
/// shift the seed across meaningful retrains.
fn training_seed_from_library(tracks: &[Track]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for track in tracks {
        hash ^= track.id.get().unsigned_abs();
        hash = hash.wrapping_mul(0x100000001b3);
        hash ^= track.statistics.play_count;
        hash = hash.wrapping_mul(0x100000001b3);
        hash ^= track.statistics.skip_count;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use sustain_domain::{
        PlayStatistics, Rating, Track, TrackId, TrackLocation, TrackMetadata, TrackRelativePath,
    };

    use super::{MIN_LABELED_TRACKS, SmartShuffleTrainer, label_for_track};
    use crate::SmartShuffleError;

    fn synthesise_track(id: i64, genre: &str, plays: u64, skips: u64) -> Track {
        Track {
            id: TrackId::new(id).expect("valid id"),
            location: TrackLocation::available(
                TrackRelativePath::new(format!("g/{id}.flac")).expect("relative path"),
            ),
            content_hash: None,
            metadata: TrackMetadata {
                genre: Some(genre.to_owned()),
                year: Some(2020),
                bpm: Some(120 + (id as u32 % 30)),
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
    fn label_distinguishes_positives_negatives_and_undecideds() {
        let positive = synthesise_track(1, "Rock", 5, 1);
        let negative = synthesise_track(2, "Rock", 1, 4);
        let undecided = synthesise_track(3, "Rock", 1, 1);
        assert_eq!(label_for_track(&positive), Some(true));
        assert_eq!(label_for_track(&negative), Some(false));
        assert_eq!(label_for_track(&undecided), None);
    }

    #[test]
    fn trainer_refuses_to_run_below_the_threshold() {
        let tracks: Vec<Track> = (0..(MIN_LABELED_TRACKS as i64 - 10))
            .map(|index| synthesise_track(index + 1, "Rock", 5, 0))
            .collect();
        assert!(matches!(
            SmartShuffleTrainer::train(&tracks),
            Err(SmartShuffleError::InsufficientTrainingData { .. })
        ));
    }

    #[test]
    fn trainer_refuses_with_no_negatives() {
        let tracks: Vec<Track> = (0..(MIN_LABELED_TRACKS as i64 + 10))
            .map(|index| synthesise_track(index + 1, "Rock", 5, 0))
            .collect();
        assert!(matches!(
            SmartShuffleTrainer::train(&tracks),
            Err(SmartShuffleError::InsufficientTrainingData {
                positives: _,
                negatives: 0
            })
        ));
    }

    #[test]
    fn trainer_produces_a_usable_model_with_enough_labels() {
        let mut tracks: Vec<Track> = Vec::new();
        for index in 0..70 {
            // Positive cluster: Rock, mid BPM.
            tracks.push(synthesise_track(index + 1, "Rock", 8, 0));
        }
        for index in 70..140 {
            // Negative cluster: Polka, high BPM (just a separable genre).
            tracks.push(synthesise_track(index + 1, "Polka", 0, 5));
        }
        let outcome = SmartShuffleTrainer::train(&tracks).expect("model trained");
        assert!(outcome.positive_label_count > 0);
        assert!(outcome.negative_label_count > 0);
        assert!(outcome.model.tree_count() > 0);
    }
}
