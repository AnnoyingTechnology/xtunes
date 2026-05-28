// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Smart Shuffle next-track picker. Scores each candidate by
//! combining the trained engagement probability with a deterministic
//! similarity-to-seed signal and soft same-session/recency
//! penalties, then samples from the softmax-warped distribution.
//!
//! The picker degrades gracefully: if no model has been trained
//! yet (`engagement = None`), every candidate gets a neutral 0.5
//! engagement score and the similarity-to-seed signal does all the
//! work. That keeps Smart Shuffle producing reasonable picks
//! during the cold-start window without forcing the user to wait
//! for training before any music plays.

use std::collections::HashMap;
use std::time::SystemTime;

use sustain_domain::{SmartShuffleEntropy, Track, TrackId};

use crate::forest::SplitMix64;
use crate::model::SmartShuffleModel;
use crate::similarity::{
    album_artist_match, bpm_similarity, duration_similarity, genre_token_similarity,
    musical_key_similarity, recency_penalty, year_similarity,
};

/// Multiplier applied to each similarity component when combining
/// them into the deterministic part of a candidate's score. Sum =
/// 1.0; bumping a value here biases the picker toward the named
/// signal. Held constant because changing the balance is a
/// product call, not a tuning hot-path.
const GENRE_WEIGHT: f32 = 0.35;
const ALBUM_ARTIST_WEIGHT: f32 = 0.15;
const YEAR_WEIGHT: f32 = 0.10;
const BPM_WEIGHT: f32 = 0.15;
const KEY_WEIGHT: f32 = 0.10;
const DURATION_WEIGHT: f32 = 0.05;
const RECENCY_PENALTY_WEIGHT: f32 = 0.10;

/// Trade-off between the deterministic similarity score and the
/// learned engagement probability. `0.6` means engagement
/// dominates slightly when the model is trained, which is the
/// spec the user signed off on ("ML will surprise us more than
/// hand-weighted").
const ENGAGEMENT_BLEND: f32 = 0.6;

/// Soft penalty (in score units) for picking a track that already
/// appears in this session's history. The cursor model in
/// `PlaybackQueue` makes "back to a previously-played track" an
/// explicit user gesture (Previous button); the picker biases
/// against accidentally re-picking the same track via the model.
const SAME_SESSION_PENALTY: f32 = 0.25;

/// Hard cap on how many top candidates the debug logger prints
/// when `SUSTAIN_LOG_SMART_SHUFFLE=1` is set.
const DEBUG_TOP_K: usize = 5;

/// Read-only inputs the runtime hands the picker. `tracks_by_id`
/// is keyed by every id appearing in `candidates` or
/// `played_history`; the runtime owns the actual `Track` data, the
/// picker just borrows it.
pub struct PickContext<'a> {
    pub seed: &'a Track,
    pub candidates: &'a [&'a Track],
    pub played_history: &'a [TrackId],
    pub entropy: SmartShuffleEntropy,
    pub now: SystemTime,
}

/// Result of a successful pick. `mode` reports whether the
/// trained model or the cold-start fallback supplied the
/// engagement term, so the runtime can surface a notification on
/// first cold-start use.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PickedTrack {
    pub track_id: TrackId,
    pub mode: PickMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PickMode {
    Trained,
    ColdStart,
}

/// Top candidates with their score breakdown. Returned alongside
/// the pick when debug logging is enabled so the runtime can
/// stream the contents to stderr.
#[derive(Clone, Debug)]
pub struct PickDebug {
    pub seed_track_id: TrackId,
    pub entries: Vec<PickDebugEntry>,
}

#[derive(Clone, Debug)]
pub struct PickDebugEntry {
    pub track_id: TrackId,
    pub total_score: f32,
    pub engagement: f32,
    pub similarity: f32,
    pub recency_penalty: f32,
    pub session_penalty: f32,
}

pub fn pick_next_track(
    model: Option<&SmartShuffleModel>,
    context: PickContext<'_>,
) -> Option<(PickedTrack, Option<PickDebug>)> {
    if context.candidates.is_empty() {
        return None;
    }

    let played_lookup: HashMap<TrackId, ()> =
        context.played_history.iter().map(|id| (*id, ())).collect();

    let scores = score_candidates(model, &context, &played_lookup);
    if scores.is_empty() {
        return None;
    }

    let mut rng_seed = context
        .seed
        .id
        .get()
        .unsigned_abs()
        .wrapping_mul(0x9E3779B97F4A7C15)
        .wrapping_add(context.played_history.len() as u64);
    rng_seed ^= entropy_seed_bits(context.entropy);
    let mut rng = SplitMix64::new(rng_seed);
    let temperature = context.entropy.temperature();

    let chosen = softmax_sample(&scores, temperature, &mut rng);
    let mode = if model.is_some() {
        PickMode::Trained
    } else {
        PickMode::ColdStart
    };

    let debug = std::env::var_os("SUSTAIN_LOG_SMART_SHUFFLE")
        .filter(|value| value == "1")
        .map(|_| build_debug(&context.seed.id, &scores));

    Some((
        PickedTrack {
            track_id: chosen.track_id,
            mode,
        },
        debug,
    ))
}

struct ScoredCandidate {
    track_id: TrackId,
    total_score: f32,
    engagement: f32,
    similarity: f32,
    recency_penalty: f32,
    session_penalty: f32,
}

fn score_candidates(
    model: Option<&SmartShuffleModel>,
    context: &PickContext<'_>,
    played: &HashMap<TrackId, ()>,
) -> Vec<ScoredCandidate> {
    context
        .candidates
        .iter()
        .filter(|candidate| candidate.id != context.seed.id)
        .map(|candidate| {
            let engagement = engagement_score(model, candidate);
            let similarity = similarity_score(context.seed, candidate);
            let recency = recency_penalty(candidate, context.now);
            let session_penalty = if played.contains_key(&candidate.id) {
                SAME_SESSION_PENALTY
            } else {
                0.0
            };
            let total = ENGAGEMENT_BLEND * engagement + (1.0 - ENGAGEMENT_BLEND) * similarity
                - RECENCY_PENALTY_WEIGHT * recency
                - session_penalty;
            ScoredCandidate {
                track_id: candidate.id,
                total_score: total,
                engagement,
                similarity,
                recency_penalty: recency,
                session_penalty,
            }
        })
        .collect()
}

fn engagement_score(model: Option<&SmartShuffleModel>, candidate: &Track) -> f32 {
    let Some(model) = model else {
        return 0.5;
    };
    let features = model.extractor().extract(candidate);
    let prob = model
        .forest()
        .predict_positive_probability(features.as_slice());
    prob.clamp(0.0, 1.0)
}

fn similarity_score(seed: &Track, candidate: &Track) -> f32 {
    let genre = genre_token_similarity(seed, candidate);
    let artist = album_artist_match(seed, candidate);
    let year = year_similarity(seed, candidate);
    let bpm = bpm_similarity(seed, candidate);
    let key = musical_key_similarity(seed, candidate);
    let duration = duration_similarity(seed, candidate);
    genre * GENRE_WEIGHT
        + artist * ALBUM_ARTIST_WEIGHT
        + year * YEAR_WEIGHT
        + bpm * BPM_WEIGHT
        + key * KEY_WEIGHT
        + duration * DURATION_WEIGHT
}

fn softmax_sample<'a>(
    candidates: &'a [ScoredCandidate],
    temperature: f32,
    rng: &mut SplitMix64,
) -> &'a ScoredCandidate {
    // Numerical-stability trick: subtract the max score before
    // taking exp() so the largest exponent is 0 and we cannot
    // overflow. We trust `candidates` to be non-empty (caller
    // contract).
    let max_score = candidates
        .iter()
        .map(|c| c.total_score)
        .fold(f32::NEG_INFINITY, f32::max);
    let temperature = temperature.max(1e-3);

    let weights: Vec<f32> = candidates
        .iter()
        .map(|c| ((c.total_score - max_score) / temperature).exp())
        .collect();
    let total_weight: f32 = weights.iter().sum();
    if total_weight <= 0.0 {
        // Defensive: a degenerate distribution (every weight 0)
        // would otherwise cause an infinite loop below. Fall back
        // to uniform random selection.
        let index = rng.next_bounded(candidates.len());
        return &candidates[index];
    }
    let pick = rng.next_unit_interval() * total_weight;
    let mut accumulated = 0.0;
    for (index, weight) in weights.iter().enumerate() {
        accumulated += weight;
        if pick <= accumulated {
            return &candidates[index];
        }
    }
    // Floating-point rounding may walk us past `total_weight`; the
    // last candidate is the natural fallback.
    candidates
        .last()
        .expect("candidates is non-empty by caller contract")
}

fn build_debug(seed_track_id: &TrackId, scores: &[ScoredCandidate]) -> PickDebug {
    let mut ranked: Vec<&ScoredCandidate> = scores.iter().collect();
    ranked.sort_by(|a, b| {
        b.total_score
            .partial_cmp(&a.total_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    PickDebug {
        seed_track_id: *seed_track_id,
        entries: ranked
            .into_iter()
            .take(DEBUG_TOP_K)
            .map(|entry| PickDebugEntry {
                track_id: entry.track_id,
                total_score: entry.total_score,
                engagement: entry.engagement,
                similarity: entry.similarity,
                recency_penalty: entry.recency_penalty,
                session_penalty: entry.session_penalty,
            })
            .collect(),
    }
}

fn entropy_seed_bits(entropy: SmartShuffleEntropy) -> u64 {
    match entropy {
        SmartShuffleEntropy::Focused => 0x01020304,
        SmartShuffleEntropy::Balanced => 0xAABBCCDD,
        SmartShuffleEntropy::Adventurous => 0xFEED1234,
    }
}

/// Trace utility used by the runtime when
/// `SUSTAIN_LOG_SMART_SHUFFLE=1` is set. Formats the debug
/// snapshot as a single line per top candidate, ordered by total
/// score (descending). Living here so the runtime does not have
/// to reach into the picker's score fields directly.
pub fn format_debug(debug: &PickDebug, track_label: impl Fn(TrackId) -> String) -> String {
    let mut lines = vec![format!(
        "[smart-shuffle] seed = {}",
        track_label(debug.seed_track_id),
    )];
    for entry in &debug.entries {
        lines.push(format!(
            "  {:>+0.4} = engagement {:.3} × {:.2} + similarity {:.3} × {:.2} − recency {:.3} × {:.2} − session {:.3} | candidate = {}",
            entry.total_score,
            entry.engagement,
            ENGAGEMENT_BLEND,
            entry.similarity,
            1.0 - ENGAGEMENT_BLEND,
            entry.recency_penalty,
            RECENCY_PENALTY_WEIGHT,
            entry.session_penalty,
            track_label(entry.track_id),
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use sustain_domain::{
        PlayStatistics, Rating, SmartShuffleEntropy, Track, TrackId, TrackLocation, TrackMetadata,
        TrackRelativePath,
    };

    use super::{PickContext, pick_next_track};

    fn track(id: i64, genre: Option<&str>) -> Track {
        Track {
            id: TrackId::new(id).expect("valid id"),
            location: TrackLocation::available(
                TrackRelativePath::new(format!("track-{id}.flac")).expect("relative path"),
            ),
            content_hash: None,
            metadata: TrackMetadata {
                genre: genre.map(str::to_owned),
                ..TrackMetadata::default()
            },
            rating: Rating::unrated(),
            statistics: PlayStatistics::default(),
            file_size_bytes: None,
            has_embedded_artwork: None,
        }
    }

    #[test]
    fn picker_returns_none_with_no_candidates() {
        let seed = track(1, Some("Rock"));
        let context = PickContext {
            seed: &seed,
            candidates: &[],
            played_history: &[seed.id],
            entropy: SmartShuffleEntropy::Balanced,
            now: SystemTime::UNIX_EPOCH,
        };
        assert!(pick_next_track(None, context).is_none());
    }

    #[test]
    fn picker_skips_seed_track() {
        let seed = track(1, Some("Rock"));
        let other = track(2, Some("Rock"));
        let candidates = [&seed, &other];
        let context = PickContext {
            seed: &seed,
            candidates: &candidates,
            played_history: &[seed.id],
            entropy: SmartShuffleEntropy::Balanced,
            now: SystemTime::UNIX_EPOCH,
        };
        let pick = pick_next_track(None, context).expect("non-empty pick");
        assert_eq!(pick.0.track_id, other.id);
    }

    #[test]
    fn picker_is_deterministic_given_same_seed_and_history_length() {
        let seed = track(7, Some("House"));
        let candidates_owned: Vec<Track> = (1..=12)
            .map(|index| track(index, Some(if index % 3 == 0 { "House" } else { "Rock" })))
            .collect();
        let candidate_refs: Vec<&Track> = candidates_owned.iter().collect();
        let played: Vec<TrackId> = vec![seed.id];

        let first = pick_next_track(
            None,
            PickContext {
                seed: &seed,
                candidates: &candidate_refs,
                played_history: &played,
                entropy: SmartShuffleEntropy::Balanced,
                now: SystemTime::UNIX_EPOCH,
            },
        )
        .expect("first pick");
        let second = pick_next_track(
            None,
            PickContext {
                seed: &seed,
                candidates: &candidate_refs,
                played_history: &played,
                entropy: SmartShuffleEntropy::Balanced,
                now: SystemTime::UNIX_EPOCH,
            },
        )
        .expect("second pick");
        assert_eq!(first.0.track_id, second.0.track_id);
    }

    #[test]
    fn picker_reports_cold_start_when_no_model() {
        let seed = track(1, Some("Rock"));
        let other = track(2, Some("Rock"));
        let candidates = [&other];
        let pick = pick_next_track(
            None,
            PickContext {
                seed: &seed,
                candidates: &candidates,
                played_history: &[seed.id],
                entropy: SmartShuffleEntropy::Balanced,
                now: SystemTime::UNIX_EPOCH,
            },
        )
        .expect("cold start pick");
        assert_eq!(pick.0.mode, super::PickMode::ColdStart);
    }
}
