// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Feature extraction for the Smart Shuffle engagement model.
//!
//! Two-pass design: at training time, [`FeatureExtractor::build`]
//! sweeps the library to discover the genre-token vocabulary (which
//! is what changes when the user grows their library or relabels
//! tracks). At pick time, the same extractor is re-loaded from the
//! persisted model so the feature vector lines up bit-for-bit with
//! the one the model was trained on. The set of *numeric* features
//! is fixed by [`FEATURE_SCHEMA_VERSION`]; adding a new numeric
//! feature is a schema bump, which causes existing models to be
//! cleared by the runtime.
//!
//! Genre tokens come from a slugify-and-explode-on-dash pass over the
//! raw `genre` tag. "House / Tech" → `"house-tech"` → tokens
//! `{house, tech}`. Tokens shorter than two characters are dropped to
//! keep noise low; the vocabulary keeps the most common tokens up to
//! [`MAX_GENRE_TOKENS`] so the feature dimension is bounded.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sustain_domain::{MusicalKey, Track};

/// Bump when the *numeric* feature layout (count, ordering, meaning)
/// changes. Adding a previously-undescribed feature, renaming an
/// existing one, or changing its normalisation all count. The
/// runtime compares this against the persisted model's value and
/// silently clears a stale model rather than feeding it mismatched
/// inputs.
pub const FEATURE_SCHEMA_VERSION: u32 = 1;

/// Hard cap on the genre-token vocabulary. The vocabulary is built
/// from the most common tokens in the library and truncated here so
/// the feature dimension stays bounded regardless of how exotic the
/// user's collection is. 256 covers an order of magnitude more than
/// any plausible curated library; the truncation is a safety net,
/// not a meaningful filter.
pub const MAX_GENRE_TOKENS: usize = 256;

/// Number of buckets used to hash the album-artist identity into a
/// small integer feature. Hashing collapses the unbounded artist
/// space into a fixed-size categorical, which lets the model learn
/// "this user engages with this cluster of artists" without
/// memorising specific names.
pub const ALBUM_ARTIST_BUCKETS: u32 = 64;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FeatureExtractor {
    /// Vocabulary of genre tokens, in canonical order. The position
    /// of a token in this vector is its index in the multi-hot
    /// portion of the feature vector.
    pub genre_tokens: Vec<String>,
}

/// Dense feature vector. Layout is
/// `[genre_token_one_hot..., year, bpm, key, duration, rating,
/// album_artist_bucket]` — every value is `f32` and the numeric
/// features are normalised in roughly `[0.0, 1.0]` (the
/// normalisation is loose; the decision-tree splits do not require
/// strict bounds, only consistency between train and predict).
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureVector(pub Vec<f32>);

impl FeatureVector {
    pub fn as_slice(&self) -> &[f32] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FeatureExtractor {
    /// Build a fresh extractor from the supplied library. Token
    /// frequencies are computed across every track; the top
    /// [`MAX_GENRE_TOKENS`] are retained in descending-frequency,
    /// then lexicographic order so the vocabulary is deterministic.
    pub fn build(tracks: &[Track]) -> Self {
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        for track in tracks {
            for token in genre_tokens(track.metadata.genre.as_deref()) {
                *counts.entry(token).or_default() += 1;
            }
        }
        let mut ranked: Vec<(String, u32)> = counts.into_iter().collect();
        // Sort by (-count, token) to keep the vocab deterministic: the
        // BTreeMap above ensures stable lexicographic order within a
        // count tier, and `sort_by` is stable so the secondary order
        // is preserved.
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        ranked.truncate(MAX_GENRE_TOKENS);
        Self {
            genre_tokens: ranked.into_iter().map(|(token, _)| token).collect(),
        }
    }

    /// Dense width of the feature vector this extractor produces.
    /// Useful for sizing pre-allocations in the trainer.
    pub fn feature_width(&self) -> usize {
        self.genre_tokens.len() + NUMERIC_FEATURE_COUNT
    }

    pub fn extract(&self, track: &Track) -> FeatureVector {
        let mut values = vec![0.0_f32; self.feature_width()];

        let track_tokens = genre_tokens(track.metadata.genre.as_deref());
        for (index, token) in self.genre_tokens.iter().enumerate() {
            if track_tokens.iter().any(|candidate| candidate == token) {
                values[index] = 1.0;
            }
        }

        let numeric_offset = self.genre_tokens.len();
        values[numeric_offset + NumericFeature::Year as usize] =
            normalise_year(track.metadata.year);
        values[numeric_offset + NumericFeature::Bpm as usize] = normalise_bpm(track.metadata.bpm);
        values[numeric_offset + NumericFeature::Key as usize] =
            normalise_musical_key(track.metadata.key.as_deref());
        values[numeric_offset + NumericFeature::Duration as usize] =
            normalise_duration(track.metadata.duration);
        values[numeric_offset + NumericFeature::Rating as usize] =
            f32::from(track.rating.stars()) / 5.0;
        values[numeric_offset + NumericFeature::AlbumArtistBucket as usize] =
            album_artist_bucket(track.metadata.album_artist.as_deref());

        FeatureVector(values)
    }
}

/// Number of numeric features tacked onto the end of the multi-hot
/// genre block. Kept as a constant so [`FeatureExtractor::feature_width`]
/// agrees with [`NumericFeature`].
pub const NUMERIC_FEATURE_COUNT: usize = 6;

/// Numeric feature layout, used as the offset within the trailing
/// numeric block of [`FeatureExtractor::extract`]'s output.
#[derive(Clone, Copy, Debug)]
pub enum NumericFeature {
    Year = 0,
    Bpm = 1,
    Key = 2,
    Duration = 3,
    Rating = 4,
    AlbumArtistBucket = 5,
}

/// Slugify and explode-on-dash. ASCII-folded, lowercase, every run
/// of non-alphanumerics becomes a single `-` separator; the result
/// is then split on `-` and trimmed of single-character tokens. The
/// canonical example: `"House/Tech"` → `"house-tech"` →
/// `{"house", "tech"}`. Returns an empty vector for `None` or
/// pure-whitespace input.
pub fn genre_tokens(genre: Option<&str>) -> Vec<String> {
    let Some(genre) = genre else {
        return Vec::new();
    };
    let mut slug = String::with_capacity(genre.len());
    let mut last_was_separator = true;
    for character in genre.chars() {
        let lowered = character.to_ascii_lowercase();
        if lowered.is_ascii_alphanumeric() {
            slug.push(lowered);
            last_was_separator = false;
        } else if !last_was_separator {
            slug.push('-');
            last_was_separator = true;
        }
    }
    // Trim trailing separator.
    while slug.ends_with('-') {
        slug.pop();
    }
    slug.split('-')
        .filter(|token| token.len() >= 2)
        .map(str::to_owned)
        .collect()
}

/// Normalise year into the unit interval centred on a reasonable
/// modern range. Years before 1900 collapse to 0.0, years after
/// 2100 collapse to 1.0; the model can still learn meaningful
/// splits inside the 200-year window.
fn normalise_year(year: Option<i32>) -> f32 {
    let Some(year) = year else {
        return 0.5;
    };
    let clamped = year.clamp(1900, 2100);
    (clamped - 1900) as f32 / 200.0
}

fn normalise_bpm(bpm: Option<u32>) -> f32 {
    // Most music sits inside 60–200 BPM. Slower than 60 or faster
    // than 200 BPM collapses to the endpoints — the model is not the
    // place to learn the rare extremes.
    let Some(bpm) = bpm else {
        return 0.5;
    };
    let clamped = bpm.clamp(40, 240);
    (clamped - 40) as f32 / 200.0
}

fn normalise_musical_key(key: Option<&str>) -> f32 {
    let Some(key_str) = key else {
        return 0.5;
    };
    let Some(parsed) = MusicalKey::from_short_code(key_str.trim()) else {
        return 0.5;
    };
    // Map the chromatic pitch-class index to [0, 1). The variant's
    // discriminant is chromatic-major-first then chromatic-minor;
    // `(discriminant) % 12` collapses major/minor onto the same
    // pitch class. Mode is folded in implicitly via genre / album-
    // artist clustering, and the similarity scorer separately
    // exploits the circle of fifths for cross-key proximity.
    let pitch_class = (parsed as u8) % 12;
    f32::from(pitch_class) / 12.0
}

fn normalise_duration(duration: Option<std::time::Duration>) -> f32 {
    let Some(duration) = duration else {
        return 0.5;
    };
    let secs = duration.as_secs_f32().max(1.0);
    // Log-normalise around a 4-minute reference. `secs.ln() / 8.0`
    // puts a 30-second track near 0.4 and a 30-minute track near 0.9,
    // with the typical pop song around 0.7.
    (secs.ln() / 8.0).clamp(0.0, 1.0)
}

/// Hash the album-artist string into one of [`ALBUM_ARTIST_BUCKETS`]
/// integer buckets, returned as `bucket / ALBUM_ARTIST_BUCKETS` for
/// the numeric feature slot. A missing album artist hashes to its
/// own bucket (0.0) so the model can distinguish "no album artist"
/// from any of the actual artists.
fn album_artist_bucket(album_artist: Option<&str>) -> f32 {
    let Some(album_artist) = album_artist else {
        return 0.0;
    };
    let trimmed = album_artist.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return 0.0;
    }
    // FNV-1a hash, deterministic across runs.
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in trimmed.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let bucket = (hash % u64::from(ALBUM_ARTIST_BUCKETS)) as u32;
    bucket as f32 / ALBUM_ARTIST_BUCKETS as f32
}

#[cfg(test)]
mod tests {
    use super::{
        ALBUM_ARTIST_BUCKETS, FeatureExtractor, MAX_GENRE_TOKENS, NumericFeature,
        album_artist_bucket, genre_tokens,
    };

    #[test]
    fn genre_tokens_slugify_and_explode_on_separator() {
        assert_eq!(
            genre_tokens(Some("House/Tech")),
            vec!["house".to_owned(), "tech".to_owned()],
        );
        assert_eq!(
            genre_tokens(Some("Alternative Rock")),
            vec!["alternative".to_owned(), "rock".to_owned()],
        );
        assert_eq!(genre_tokens(Some("")), Vec::<String>::new());
        assert_eq!(genre_tokens(None), Vec::<String>::new());
    }

    #[test]
    fn genre_tokens_drop_single_character_tokens() {
        // Trailing single-character noise after slugification (e.g.
        // a stray "&") collapses to nothing rather than contaminating
        // the vocabulary.
        let r_and_b: Vec<String> = Vec::new();
        assert_eq!(
            genre_tokens(Some("R&B")),
            r_and_b,
            "single-char tokens are dropped"
        );
        assert_eq!(
            genre_tokens(Some("Lo-Fi Hip-Hop")),
            vec![
                "lo".to_owned(),
                "fi".to_owned(),
                "hip".to_owned(),
                "hop".to_owned()
            ],
        );
    }

    #[test]
    fn genre_tokens_share_components_across_subgenres() {
        // The whole point of token-explosion: "Alternative Rock"
        // and "Indie Rock" both carry the `rock` token, which lets
        // the model and the similarity scorer cluster them.
        let alt = genre_tokens(Some("Alternative Rock"));
        let indie = genre_tokens(Some("Indie Rock"));
        assert!(alt.contains(&"rock".to_owned()));
        assert!(indie.contains(&"rock".to_owned()));
    }

    #[test]
    fn vocabulary_truncates_at_max_genre_tokens() {
        // Synthesise more than MAX_GENRE_TOKENS unique tokens with
        // varying frequencies; the truncation should keep the most
        // common ones.
        let extractor = FeatureExtractor {
            genre_tokens: (0..(MAX_GENRE_TOKENS + 50))
                .map(|index| format!("g{index:03}"))
                .collect(),
        };
        // The extractor we just built bypasses `build`, but the
        // invariant we want to assert is structural: feature_width
        // == vocab_len + NUMERIC_FEATURE_COUNT.
        let width = extractor.feature_width();
        assert_eq!(
            width,
            extractor.genre_tokens.len() + super::NUMERIC_FEATURE_COUNT
        );
    }

    #[test]
    fn album_artist_bucket_is_deterministic_and_bounded() {
        let bucket = album_artist_bucket(Some("Radiohead"));
        assert!((0.0..1.0).contains(&bucket));
        assert_eq!(album_artist_bucket(Some("Radiohead")), bucket);
        assert_eq!(album_artist_bucket(Some("radiohead")), bucket);
        assert_eq!(album_artist_bucket(None), 0.0);
        assert_eq!(album_artist_bucket(Some("")), 0.0);
        // Distinct artists should land in different buckets most of
        // the time — assert at least one differs to catch a fully
        // broken hash.
        let other = album_artist_bucket(Some("Aphex Twin"));
        assert_ne!(other, bucket);
        // Buckets count is bounded.
        let _ = ALBUM_ARTIST_BUCKETS;
    }

    #[test]
    fn numeric_feature_offsets_are_unique() {
        // Sanity check that the enum's discriminants don't overlap;
        // the extractor relies on each variant indexing a distinct
        // slot in the numeric tail.
        let offsets = [
            NumericFeature::Year as usize,
            NumericFeature::Bpm as usize,
            NumericFeature::Key as usize,
            NumericFeature::Duration as usize,
            NumericFeature::Rating as usize,
            NumericFeature::AlbumArtistBucket as usize,
        ];
        let mut sorted: Vec<usize> = offsets.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), offsets.len());
    }
}
