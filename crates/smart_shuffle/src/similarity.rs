// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Deterministic similarity-to-seed scoring. Independent from the
//! trained model — these signals are computed from raw track
//! metadata at pick time and combined with the engagement
//! probability before the softmax sampling step.
//!
//! Each helper returns a value in `[0.0, 1.0]` where higher means
//! "closer to the seed". Missing-data fallbacks land at a neutral
//! mid-range value so sparse metadata does not unfairly punish or
//! reward a candidate; the scoring combiner in
//! [`crate::picker`] weights the individual signals and sums.

use std::collections::HashSet;
use std::time::SystemTime;

use sustain_domain::{MusicalKey, Track};

use crate::feature::genre_tokens;

/// Jaccard similarity over the seed's and candidate's genre-token
/// sets. `house-tech` and `house` share `{house}` → 0.5; `rock`
/// and `electronic` share nothing → 0.0; the empty-vs-empty case
/// returns 0.5 (neutral) rather than a degenerate 0/0.
pub fn genre_token_similarity(seed: &Track, candidate: &Track) -> f32 {
    let seed_tokens: HashSet<String> = genre_tokens(seed.metadata.genre.as_deref())
        .into_iter()
        .collect();
    let candidate_tokens: HashSet<String> = genre_tokens(candidate.metadata.genre.as_deref())
        .into_iter()
        .collect();
    if seed_tokens.is_empty() && candidate_tokens.is_empty() {
        return 0.5;
    }
    let intersection = seed_tokens.intersection(&candidate_tokens).count();
    let union = seed_tokens.union(&candidate_tokens).count().max(1);
    intersection as f32 / union as f32
}

/// 1.0 when the album-artist tags match (case-insensitive), 0.0
/// otherwise. Missing on either side returns the neutral 0.5 —
/// we cannot prove a match, but we can't prove a mismatch
/// either.
pub fn album_artist_match(seed: &Track, candidate: &Track) -> f32 {
    match (
        seed.metadata.album_artist.as_deref(),
        candidate.metadata.album_artist.as_deref(),
    ) {
        (Some(s), Some(c)) => {
            if s.trim().eq_ignore_ascii_case(c.trim()) {
                1.0
            } else {
                0.0
            }
        }
        _ => 0.5,
    }
}

/// Year proximity, falling off linearly outside a six-year window
/// (≈ a creative era), saturating at 0.0 past twenty years apart.
pub fn year_similarity(seed: &Track, candidate: &Track) -> f32 {
    let (Some(seed_year), Some(candidate_year)) = (seed.metadata.year, candidate.metadata.year)
    else {
        return 0.5;
    };
    let delta = (seed_year - candidate_year).unsigned_abs() as f32;
    if delta <= 6.0 {
        return 1.0 - delta / 18.0;
    }
    if delta >= 20.0 {
        return 0.0;
    }
    (20.0 - delta) / 30.0
}

/// BPM proximity with one-octave folding so 90 BPM and 180 BPM
/// score as a close match. The dancer's intuition: half-time and
/// double-time of the same tempo *feel* the same. Without
/// folding, a Smart Shuffle session anchored on a 90 BPM seed
/// would systematically reject otherwise-perfect 180 BPM
/// candidates.
pub fn bpm_similarity(seed: &Track, candidate: &Track) -> f32 {
    let (Some(seed_bpm), Some(candidate_bpm)) = (seed.metadata.bpm, candidate.metadata.bpm) else {
        return 0.5;
    };
    let octave_folded = octave_folded_delta(seed_bpm as f32, candidate_bpm as f32);
    if octave_folded <= 4.0 {
        return 1.0 - octave_folded / 16.0;
    }
    if octave_folded >= 30.0 {
        return 0.0;
    }
    (30.0 - octave_folded) / 52.0
}

fn octave_folded_delta(seed_bpm: f32, candidate_bpm: f32) -> f32 {
    let direct = (seed_bpm - candidate_bpm).abs();
    let halved = (seed_bpm - candidate_bpm * 2.0).abs();
    let doubled = (seed_bpm - candidate_bpm * 0.5).abs();
    direct.min(halved).min(doubled)
}

/// Distance on the circle of fifths, returning 1.0 for the
/// identical key and degrading to 0.0 for the maximal-distance
/// tritonal pair (six fifths apart). Mode (major / minor) is
/// folded onto its pitch class — A minor and C major both map
/// to pitch-class 0/9 respectively but the scorer treats them
/// symmetrically; clusters of jazz / classical sessions that
/// rely on the exact relative-key distinction will still get
/// boosts from genre and artist signals.
pub fn musical_key_similarity(seed: &Track, candidate: &Track) -> f32 {
    let Some(seed_key) = parse_short_code(seed.metadata.key.as_deref()) else {
        return 0.5;
    };
    let Some(candidate_key) = parse_short_code(candidate.metadata.key.as_deref()) else {
        return 0.5;
    };
    // Convert each chromatic pitch class to its position on the
    // circle of fifths via the (pitch * 7) mod 12 mapping —
    // moving up a fifth in pitch is moving one step on the
    // circle of fifths.
    let seed_position = ((seed_key as u8 % 12) as u32 * 7) % 12;
    let candidate_position = ((candidate_key as u8 % 12) as u32 * 7) % 12;
    let raw_delta = seed_position.abs_diff(candidate_position);
    let circular_delta = raw_delta.min(12 - raw_delta);
    1.0 - (circular_delta as f32 / 6.0)
}

fn parse_short_code(value: Option<&str>) -> Option<MusicalKey> {
    let trimmed = value?.trim();
    MusicalKey::from_short_code(trimmed)
}

/// Duration proximity on a log scale, so a 3-minute pop track
/// and a 6-minute prog rock track score reasonably close (one
/// doubling apart) while a 30-second skit lands very far from a
/// 4-minute song.
pub fn duration_similarity(seed: &Track, candidate: &Track) -> f32 {
    let (Some(seed_duration), Some(candidate_duration)) =
        (seed.metadata.duration, candidate.metadata.duration)
    else {
        return 0.5;
    };
    let seed_secs = seed_duration.as_secs_f32().max(1.0);
    let candidate_secs = candidate_duration.as_secs_f32().max(1.0);
    let log_delta = (seed_secs.ln() - candidate_secs.ln()).abs();
    // 0 → 1.0, 0.7 (≈ 2× duration ratio) → 0.5, 2.0 (≈ 7× ratio) → 0.
    (1.0 - log_delta / 2.0).clamp(0.0, 1.0)
}

/// Smooth recency penalty. The candidate's `last_played_at` and
/// `last_skipped_at` (whichever is more recent) drive an
/// exponential decay so a track played five minutes ago is
/// strongly demoted, an hour ago is partly demoted, three hours
/// ago is essentially neutral. The half-life is the time at
/// which the penalty is at 0.5 of its peak.
pub fn recency_penalty(candidate: &Track, now: SystemTime) -> f32 {
    let last_touch = [
        candidate.statistics.last_played_at,
        candidate.statistics.last_skipped_at,
    ]
    .into_iter()
    .flatten()
    .max();
    let Some(last_touch) = last_touch else {
        return 0.0;
    };
    let Ok(elapsed) = now.duration_since(last_touch) else {
        // Clock went backwards (suspend / resume edge case).
        // Treat as "very recent" to be safe — better to under-
        // play a track than spam-pick the same one across a
        // clock anomaly.
        return 1.0;
    };
    let elapsed_secs = elapsed.as_secs_f32();
    // 45-minute half-life. exp(-elapsed / (HALF_LIFE / ln 2)).
    let half_life_secs = 45.0 * 60.0;
    let time_constant = half_life_secs / std::f32::consts::LN_2;
    (-elapsed_secs / time_constant).exp()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use sustain_domain::{
        PlayStatistics, Rating, Track, TrackId, TrackLocation, TrackMetadata, TrackRelativePath,
    };

    use super::{
        album_artist_match, bpm_similarity, duration_similarity, genre_token_similarity,
        musical_key_similarity, octave_folded_delta, recency_penalty, year_similarity,
    };

    fn track_with(metadata: TrackMetadata) -> Track {
        Track {
            id: TrackId::new(1).expect("valid id"),
            location: TrackLocation::available(
                TrackRelativePath::new("artist/album/track.flac").expect("relative path"),
            ),
            content_hash: None,
            metadata,
            rating: Rating::unrated(),
            statistics: PlayStatistics::default(),
            file_size_bytes: None,
            has_embedded_artwork: None,
        }
    }

    #[test]
    fn genre_jaccard_picks_overlap() {
        let seed = track_with(TrackMetadata {
            genre: Some("House/Tech".to_owned()),
            ..TrackMetadata::default()
        });
        let close = track_with(TrackMetadata {
            genre: Some("House".to_owned()),
            ..TrackMetadata::default()
        });
        let far = track_with(TrackMetadata {
            genre: Some("Classical".to_owned()),
            ..TrackMetadata::default()
        });
        assert!(genre_token_similarity(&seed, &close) > genre_token_similarity(&seed, &far));
    }

    #[test]
    fn album_artist_match_distinguishes_known_artists() {
        let seed = track_with(TrackMetadata {
            album_artist: Some("Radiohead".to_owned()),
            ..TrackMetadata::default()
        });
        let same = track_with(TrackMetadata {
            album_artist: Some("radiohead".to_owned()),
            ..TrackMetadata::default()
        });
        let different = track_with(TrackMetadata {
            album_artist: Some("Aphex Twin".to_owned()),
            ..TrackMetadata::default()
        });
        assert_eq!(album_artist_match(&seed, &same), 1.0);
        assert_eq!(album_artist_match(&seed, &different), 0.0);
    }

    #[test]
    fn year_similarity_falls_off_with_distance() {
        let seed = track_with(TrackMetadata {
            year: Some(2010),
            ..TrackMetadata::default()
        });
        let close = track_with(TrackMetadata {
            year: Some(2012),
            ..TrackMetadata::default()
        });
        let far = track_with(TrackMetadata {
            year: Some(1980),
            ..TrackMetadata::default()
        });
        assert!(year_similarity(&seed, &close) > year_similarity(&seed, &far));
    }

    #[test]
    fn bpm_similarity_folds_octave() {
        // 90 vs 180 BPM should score as essentially identical
        // (one octave apart, no penalty) while 90 vs 110 BPM
        // should score lower.
        assert_eq!(octave_folded_delta(90.0, 180.0), 0.0);
        let seed = track_with(TrackMetadata {
            bpm: Some(90),
            ..TrackMetadata::default()
        });
        let doubled = track_with(TrackMetadata {
            bpm: Some(180),
            ..TrackMetadata::default()
        });
        let off_tempo = track_with(TrackMetadata {
            bpm: Some(110),
            ..TrackMetadata::default()
        });
        assert!(bpm_similarity(&seed, &doubled) > bpm_similarity(&seed, &off_tempo));
    }

    #[test]
    fn musical_key_similarity_uses_circle_of_fifths() {
        let seed = track_with(TrackMetadata {
            key: Some("C".to_owned()),
            ..TrackMetadata::default()
        });
        let close = track_with(TrackMetadata {
            key: Some("G".to_owned()),
            ..TrackMetadata::default()
        });
        let far = track_with(TrackMetadata {
            key: Some("F#m".to_owned()),
            ..TrackMetadata::default()
        });
        assert!(musical_key_similarity(&seed, &close) > musical_key_similarity(&seed, &far));
    }

    #[test]
    fn duration_similarity_handles_log_scale() {
        let three_min = track_with(TrackMetadata {
            duration: Some(Duration::from_secs(180)),
            ..TrackMetadata::default()
        });
        let six_min = track_with(TrackMetadata {
            duration: Some(Duration::from_secs(360)),
            ..TrackMetadata::default()
        });
        let skit = track_with(TrackMetadata {
            duration: Some(Duration::from_secs(20)),
            ..TrackMetadata::default()
        });
        assert!(duration_similarity(&three_min, &six_min) > duration_similarity(&three_min, &skit));
    }

    #[test]
    fn recency_penalty_decays_to_zero() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(3 * 60 * 60);
        let recent = track_with(TrackMetadata::default());
        let mut recent = recent;
        recent.statistics = PlayStatistics {
            last_played_at: Some(
                SystemTime::UNIX_EPOCH + Duration::from_secs(3 * 60 * 60 - 5 * 60),
            ),
            ..PlayStatistics::default()
        };
        let stale = track_with(TrackMetadata::default());
        let mut stale = stale;
        stale.statistics = PlayStatistics {
            last_played_at: Some(SystemTime::UNIX_EPOCH),
            ..PlayStatistics::default()
        };
        let untouched = track_with(TrackMetadata::default());

        let recent_penalty = recency_penalty(&recent, now);
        let stale_penalty = recency_penalty(&stale, now);
        let untouched_penalty = recency_penalty(&untouched, now);

        assert!(recent_penalty > stale_penalty);
        assert_eq!(untouched_penalty, 0.0);
        assert!(
            stale_penalty < 0.1,
            "3h-old plays should be near-zero penalty; got {stale_penalty}"
        );
    }
}
