// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! The Smart Shuffle *index*: the prepared, library-dependent state
//! the picker needs that cannot be derived from a single track in
//! isolation.
//!
//! This is the artefact the design brief (§12) calls "the index" and
//! deliberately *not* a trained model — there is none. With fixed,
//! hand-set perceptual weights the only genuinely library-dependent
//! work is:
//!
//!   * the **genre-token vocabulary and its IDF weights** — "Rock" on
//!     40% of a library is nearly uninformative; "Shoegaze" on 2% is
//!     highly informative, and only a sweep of the whole library can
//!     tell the two apart (§6.1);
//!   * (added in the DSP stage) the **robust normalization
//!     statistics** for timbral features whose scale depends on the
//!     collection (§8).
//!
//! The index is rebuilt on the user's chosen cadence, on the "Rebuild
//! index" button, and once in the background after launch. It is
//! milliseconds of work on a 10 000-track library but it is real, and
//! it is what makes the fixed weights honest. It is persisted as an
//! opaque blob in a singleton row, tagged with [`INDEX_SCHEMA_VERSION`]
//! so a stale shape is silently discarded rather than fed mismatched
//! inputs.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sustain_domain::Track;

use crate::SmartShuffleError;

/// Bump when the *shape* of the index — the set of features it
/// prepares, the genre tokenisation, or the normalization scheme —
/// changes in a way that invalidates a previously-persisted blob. The
/// runtime compares this against the stored value and discards a
/// mismatched blob (no migration: pre-release, the index is cheap to
/// rebuild from scratch).
///
/// Version 1: genre-token vocabulary + IDF only (metadata-feature
/// scorer; the DSP/timbral normalization parameters arrive in a later
/// schema version).
pub const INDEX_SCHEMA_VERSION: u32 = 1;

/// Hard cap on the genre-token vocabulary, a safety net against a
/// pathologically exotic library blowing up the blob. 4096 is an
/// order of magnitude beyond any plausible curated collection's
/// distinct genre tokens; the truncation keeps the most frequent
/// tokens (the rare ones it drops carry the *highest* IDF, but a
/// token appearing on a single track contributes to no pair's
/// similarity anyway, so dropping the long tail is harmless).
const MAX_GENRE_TOKENS: usize = 4096;

/// The prepared, persisted Smart Shuffle index. Cheap to clone (a
/// `BTreeMap` of short strings to `f32` plus a few scalars).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SmartShuffleIndex {
    /// Schema version the blob was written under. Checked on load.
    schema_version: u32,
    /// Genre token → inverse-document-frequency weight. A token's IDF
    /// reflects how *rare* it is across the library, so shared rare
    /// genres count for far more than shared common ones (§6.1).
    /// `BTreeMap` for deterministic serialisation.
    genre_idf: BTreeMap<String, f32>,
    /// Number of (available) tracks the index was built from. Surfaced
    /// in the preferences status caption ("Library indexed: N tracks").
    indexed_track_count: u32,
    /// Fraction in `[0.0, 1.0]` of indexed tracks that carry the DSP
    /// acoustic features the timbral affinity terms need. Always `0.0`
    /// until the DSP analysis stage lands; surfaced as "Analysis
    /// coverage: NN%".
    analysis_coverage: f32,
    /// Wall-clock unix timestamp of the rebuild that produced this
    /// index, stamped by the runtime (this crate never reads the
    /// clock). Surfaced as "Last index rebuild: …".
    built_at_unix: i64,
}

impl SmartShuffleIndex {
    /// Build a fresh index from the live library. Missing-file tracks
    /// are excluded — they can never be picked, so they must not skew
    /// the IDF document frequencies. `built_at_unix` is supplied by
    /// the caller's clock.
    pub fn build(tracks: &[Track], built_at_unix: i64) -> Self {
        // Document frequency: how many tracks carry each genre token.
        let mut document_frequency: BTreeMap<String, u32> = BTreeMap::new();
        let mut indexed_track_count: u32 = 0;
        for track in tracks.iter().filter(|t| !t.location.is_missing()) {
            indexed_track_count = indexed_track_count.saturating_add(1);
            // De-duplicate tokens within a track so a genre tag of
            // "Rock/Rock" counts "rock" once toward its frequency.
            let mut seen: BTreeMap<String, ()> = BTreeMap::new();
            for token in genre_tokens(track.metadata.genre.as_deref()) {
                seen.entry(token).or_default();
            }
            for token in seen.into_keys() {
                *document_frequency.entry(token).or_default() += 1;
            }
        }

        // Keep only the most frequent tokens if the vocabulary is
        // absurdly large (see MAX_GENRE_TOKENS).
        if document_frequency.len() > MAX_GENRE_TOKENS {
            let mut ranked: Vec<(String, u32)> = document_frequency.into_iter().collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            ranked.truncate(MAX_GENRE_TOKENS);
            document_frequency = ranked.into_iter().collect();
        }

        let genre_idf = document_frequency
            .into_iter()
            .map(|(token, df)| (token, idf(indexed_track_count, df)))
            .collect();

        Self {
            schema_version: INDEX_SCHEMA_VERSION,
            genre_idf,
            indexed_track_count,
            analysis_coverage: 0.0,
            built_at_unix,
        }
    }

    /// IDF weight for a genre token. Tokens the index has never seen
    /// (a track added since the last rebuild) are treated as maximally
    /// informative for the current library size, which is the honest
    /// default for "rare enough that we have not catalogued it yet".
    pub fn genre_token_idf(&self, token: &str) -> f32 {
        self.genre_idf
            .get(token)
            .copied()
            .unwrap_or_else(|| idf(self.indexed_track_count, 0))
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn indexed_track_count(&self) -> u32 {
        self.indexed_track_count
    }

    pub fn analysis_coverage(&self) -> f32 {
        self.analysis_coverage
    }

    pub fn built_at_unix(&self) -> i64 {
        self.built_at_unix
    }

    pub fn to_blob(&self) -> Result<Vec<u8>, SmartShuffleError> {
        serde_json::to_vec(self).map_err(|_| SmartShuffleError::IndexSerialisationFailed)
    }

    pub fn from_blob(blob: &[u8]) -> Result<Self, SmartShuffleError> {
        serde_json::from_slice(blob).map_err(|_| SmartShuffleError::IndexDeserialisationFailed)
    }
}

/// Smoothed inverse document frequency. `+1` smoothing keeps the
/// weight strictly positive even for a token present on every track
/// (so the weighted Jaccard denominator never collapses) and bounded
/// for a token present on none. With `total = 0` the library is empty
/// and every token gets the same neutral weight.
fn idf(total: u32, document_frequency: u32) -> f32 {
    let total = f32::from(u16::try_from(total.min(u32::from(u16::MAX))).unwrap_or(u16::MAX));
    let df =
        f32::from(u16::try_from(document_frequency.min(u32::from(u16::MAX))).unwrap_or(u16::MAX));
    ((total + 1.0) / (df + 1.0)).ln() + 1.0
}

/// Slugify-and-explode a raw `genre` tag into comparable tokens.
/// ASCII-folded, lowercase; every run of non-alphanumerics becomes a
/// single separator, the result is split on it, and single-character
/// tokens are dropped to keep noise low. `"House/Tech"` → `["house",
/// "tech"]`; `"Alternative Rock"` → `["alternative", "rock"]`;
/// `"R&B"` → `[]` (both single-char after slugification). Empty for
/// `None`/blank input.
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
    while slug.ends_with('-') {
        slug.pop();
    }
    slug.split('-')
        .filter(|token| token.len() >= 2)
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{INDEX_SCHEMA_VERSION, SmartShuffleIndex, genre_tokens, idf};
    use sustain_domain::{
        PlayStatistics, Rating, Track, TrackId, TrackLocation, TrackMetadata, TrackRelativePath,
    };

    fn track(id: i64, genre: Option<&str>) -> Track {
        Track {
            id: TrackId::new(id).expect("valid id"),
            location: TrackLocation::available(
                TrackRelativePath::new(format!("t/{id}.flac")).expect("relative path"),
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
    fn genre_tokens_slugify_and_explode() {
        assert_eq!(genre_tokens(Some("House/Tech")), vec!["house", "tech"]);
        assert_eq!(
            genre_tokens(Some("Alternative Rock")),
            vec!["alternative", "rock"]
        );
        assert_eq!(genre_tokens(Some("R&B")), Vec::<String>::new());
        assert_eq!(genre_tokens(None), Vec::<String>::new());
    }

    #[test]
    fn rarer_genre_token_earns_a_higher_idf() {
        // "rock" appears on 9 tracks, "shoegaze" on 1.
        let mut tracks: Vec<Track> = (0..9).map(|i| track(i + 1, Some("Rock"))).collect();
        tracks.push(track(10, Some("Shoegaze")));
        let index = SmartShuffleIndex::build(&tracks, 0);
        assert_eq!(index.indexed_track_count(), 10);
        assert!(
            index.genre_token_idf("shoegaze") > index.genre_token_idf("rock"),
            "the rare token must weigh more"
        );
    }

    #[test]
    fn missing_tracks_are_excluded_from_the_index() {
        let mut present = track(1, Some("Rock"));
        present.location =
            TrackLocation::missing(TrackRelativePath::new("t/1.flac").expect("relative path"));
        let index = SmartShuffleIndex::build(&[present], 0);
        assert_eq!(index.indexed_track_count(), 0);
    }

    #[test]
    fn idf_is_strictly_positive_even_for_ubiquitous_tokens() {
        // Present on every one of 1000 tracks → still > 0.
        assert!(idf(1000, 1000) > 0.0);
        // Present on none → finite and larger.
        assert!(idf(1000, 0) > idf(1000, 1000));
    }

    #[test]
    fn blob_round_trips() {
        let index = SmartShuffleIndex::build(&[track(1, Some("Rock"))], 123);
        let blob = index.to_blob().expect("serialise");
        let restored = SmartShuffleIndex::from_blob(&blob).expect("deserialise");
        assert_eq!(restored, index);
        assert_eq!(restored.schema_version(), INDEX_SCHEMA_VERSION);
        assert_eq!(restored.built_at_unix(), 123);
    }
}
