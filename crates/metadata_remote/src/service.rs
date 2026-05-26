// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! High-level service trait composed from the three underlying clients.
//!
//! This is the surface the rest of the application is expected to
//! depend on. Callers do not import [`crate::musicbrainz`],
//! [`crate::cover_art_archive`], or [`crate::acoustid`] directly —
//! those modules are the building blocks; this trait is the contract.
//!
//! The composed implementation lives below ([`ComposedRemoteMetadataService`])
//! and wires the three clients together with a small set of selection
//! rules:
//!
//! * **Track identification** first asks MusicBrainz to search by
//!   the supplied tag terms. If no confident match is returned and a
//!   precomputed audio fingerprint is provided, the AcoustID client
//!   is used as a fallback. The first usable candidate wins; the
//!   service does not aggregate scores or attempt ML-style ranking.
//! * **Artwork lookup** takes the chosen [`TrackMatch`], walks its
//!   releases in MusicBrainz-provided order, and queries Cover Art
//!   Archive for each release MBID until one returns bytes. If every
//!   release misses, the release-group MBID is tried as a final
//!   fallback.
//!
//! Errors propagate; partial successes (e.g. identification works,
//! artwork lookup fails) are not silently converted into "no match".
//! Mixing those would make the failure surface ambiguous at the UI
//! layer, which is the only useful place to distinguish them.

use std::sync::Arc;

use crate::acoustid::{AcoustIdClient, AudioFingerprint};
use crate::cover_art_archive::CoverArtArchiveClient;
use crate::error::RemoteResult;
use crate::musicbrainz::{MusicBrainzClient, RecordingMatch, RecordingSearchTerms};

/// Minimum MusicBrainz score (0..=100) we accept from a text-based
/// recording search before we'd rather fall back to fingerprint
/// identification (when one is available). Below this threshold, the
/// match is treated as "no confident result" — the search returned
/// *something* but not something we'd risk writing to tags.
///
/// MusicBrainz scores cluster around 100 for clean tag matches, drop
/// to the 80s when one field disagrees (different release of the same
/// recording, slightly off album name, "\[Remastered\]" / "\[Live\]"
/// suffixes), and tail off below 60 once the result is only loosely
/// related. 75 catches the well-tagged cases plus the common
/// real-world drift without crossing into ambiguity — and the
/// non-destructive contract is enforced one layer up (we never
/// overwrite existing artwork or tags), so a wrong match on a track
/// that already has no cover only affects that track.
const MUSICBRAINZ_CONFIDENT_SCORE: u8 = 75;

/// Minimum AcoustID score (0.0..=1.0). The fingerprint scoring
/// behaves differently from text-based scoring: a score above 0.85
/// effectively means "yes, this is the same recording", and anything
/// below 0.5 is statistically indistinguishable from noise. We pick a
/// conservative cutoff because the AcoustID path runs when text
/// matching has already failed and the user has nothing better.
const ACOUSTID_CONFIDENT_SCORE: f32 = 0.7;

/// Sustain's high-level query for "identify this track".
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TrackQuery {
    pub artist: Option<String>,
    pub title: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    /// Precomputed audio fingerprint, if available. Optional — the
    /// service falls back to text-only identification when absent and
    /// AcoustID is not configured.
    pub fingerprint: Option<AudioFingerprint>,
}

impl TrackQuery {
    pub fn has_any_text(&self) -> bool {
        let text = [
            self.artist.as_deref(),
            self.title.as_deref(),
            self.album.as_deref(),
        ];
        text.iter()
            .any(|value| value.is_some_and(|inner| !inner.trim().is_empty()))
    }

    fn to_search_terms(&self) -> RecordingSearchTerms {
        RecordingSearchTerms {
            artist: self.artist.clone(),
            title: self.title.clone(),
            album: self.album.clone(),
            duration_ms: self.duration_ms,
        }
    }
}

/// Resolved identification: a MusicBrainz recording the service is
/// confident represents the track. The struct is intentionally
/// projection-shaped (one row's worth of fields) rather than nested —
/// the caller writes selected fields back through the existing tag
/// path and doesn't need a deeper graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackMatch {
    pub recording_mbid: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    /// Releases the recording appears on, ordered as MusicBrainz
    /// returned them. The first entry is the service's best guess
    /// for the "primary" release; artwork lookup walks the list in
    /// order. Empty if no release is associated with the recording.
    pub releases: Vec<TrackMatchRelease>,
    /// Identification provenance. Useful at the UI layer for
    /// explaining *how* a match was made.
    pub source: TrackMatchSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackMatchRelease {
    pub release_mbid: String,
    pub release_group_mbid: Option<String>,
    pub title: Option<String>,
    pub year: Option<i32>,
    pub track_number: Option<u32>,
    pub track_total: Option<u32>,
    pub disc_number: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackMatchSource {
    /// Match came from MusicBrainz's text search.
    MusicBrainzTags,
    /// Match came from AcoustID fingerprint lookup, then verified
    /// against MusicBrainz.
    AcoustIdFingerprint,
}

/// Cover image bytes plus enough provenance to render diagnostics or
/// a status-bar message. The bytes are exactly what Cover Art Archive
/// returned — they are not re-encoded, cropped, or normalised here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchedArtwork {
    pub bytes: Vec<u8>,
    pub release_mbid: String,
}

/// Service contract. Implementations are required to be `Send + Sync`
/// so the runtime can hand them across worker boundaries.
pub trait RemoteMetadataService: Send + Sync {
    /// Identify the track described by `query`. Returns `Ok(None)`
    /// when no confident match exists; that is a normal outcome and
    /// the caller treats it like any other "no result".
    fn identify_track(&self, query: &TrackQuery) -> RemoteResult<Option<TrackMatch>>;

    /// Fetch artwork for an already-identified match. Returns
    /// `Ok(None)` if Cover Art Archive has no front cover on file
    /// for any of the match's releases.
    fn fetch_artwork_for_match(
        &self,
        track_match: &TrackMatch,
    ) -> RemoteResult<Option<FetchedArtwork>>;

    /// Convenience composition: identify, then fetch artwork. The
    /// individual operations are kept on the trait so callers that
    /// already hold a [`TrackMatch`] (e.g. after a Get Info dialog)
    /// can reuse it without re-identifying.
    fn fetch_artwork(&self, query: &TrackQuery) -> RemoteResult<Option<FetchedArtwork>> {
        let Some(track_match) = self.identify_track(query)? else {
            return Ok(None);
        };
        self.fetch_artwork_for_match(&track_match)
    }
}

/// The production implementation. Owns one [`MusicBrainzClient`], one
/// [`CoverArtArchiveClient`], and optionally one [`AcoustIdClient`].
/// The AcoustID slot is optional because builds without an API key
/// must still work for tag-based identification.
pub struct ComposedRemoteMetadataService {
    musicbrainz: MusicBrainzClient,
    cover_art_archive: CoverArtArchiveClient,
    acoustid: Option<AcoustIdClient>,
}

impl ComposedRemoteMetadataService {
    pub fn new(
        musicbrainz: MusicBrainzClient,
        cover_art_archive: CoverArtArchiveClient,
        acoustid: Option<AcoustIdClient>,
    ) -> Self {
        Self {
            musicbrainz,
            cover_art_archive,
            acoustid,
        }
    }

    /// Convenience wrapper: build the whole service stack from a
    /// shared [`crate::client::HttpClient`] and an optional AcoustID
    /// key. The HTTP client carries the User-Agent and rate-limit
    /// state shared across all three providers.
    pub fn from_http_client(
        http: Arc<crate::client::HttpClient>,
        acoustid_api_key: Option<&str>,
    ) -> Self {
        let musicbrainz = MusicBrainzClient::new(Arc::clone(&http));
        let cover_art_archive = CoverArtArchiveClient::new(Arc::clone(&http));
        let acoustid = acoustid_api_key.and_then(|key| AcoustIdClient::new(http, key));
        Self::new(musicbrainz, cover_art_archive, acoustid)
    }

    fn identify_via_musicbrainz(&self, query: &TrackQuery) -> RemoteResult<Option<TrackMatch>> {
        if !query.has_any_text() {
            return Ok(None);
        }
        let results = self
            .musicbrainz
            .search_recordings(&query.to_search_terms())?;
        Ok(pick_best_musicbrainz(results, MUSICBRAINZ_CONFIDENT_SCORE)
            .map(|recording| recording_to_match(recording, TrackMatchSource::MusicBrainzTags)))
    }

    fn identify_via_acoustid(&self, query: &TrackQuery) -> RemoteResult<Option<TrackMatch>> {
        let Some(acoustid) = &self.acoustid else {
            return Ok(None);
        };
        let Some(fingerprint) = &query.fingerprint else {
            return Ok(None);
        };
        let matches = acoustid.lookup_fingerprint(fingerprint)?;
        let Some(best) = matches
            .into_iter()
            .filter(|candidate| candidate.score >= ACOUSTID_CONFIDENT_SCORE)
            .max_by(|left, right| {
                left.score
                    .partial_cmp(&right.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        else {
            return Ok(None);
        };

        // AcoustID returns bare recording IDs with no descriptive
        // fields. We resolve each candidate against MusicBrainz's
        // lookup-by-id endpoint so the downstream artwork path has a
        // populated release list to walk; an unresolved MBID would
        // carry no releases and would silently produce no artwork.
        for recording_mbid in &best.recording_mbids {
            if let Some(resolved) = self.musicbrainz.lookup_recording(recording_mbid)? {
                return Ok(Some(recording_to_match(
                    resolved,
                    TrackMatchSource::AcoustIdFingerprint,
                )));
            }
        }
        Ok(None)
    }
}

impl RemoteMetadataService for ComposedRemoteMetadataService {
    fn identify_track(&self, query: &TrackQuery) -> RemoteResult<Option<TrackMatch>> {
        if let Some(track_match) = self.identify_via_musicbrainz(query)? {
            return Ok(Some(track_match));
        }
        self.identify_via_acoustid(query)
    }

    fn fetch_artwork_for_match(
        &self,
        track_match: &TrackMatch,
    ) -> RemoteResult<Option<FetchedArtwork>> {
        for release in &track_match.releases {
            if let Some(bytes) = self
                .cover_art_archive
                .fetch_release_front(&release.release_mbid)?
            {
                return Ok(Some(FetchedArtwork {
                    bytes,
                    release_mbid: release.release_mbid.clone(),
                }));
            }
        }
        // Release-level lookups all missed. Try the first
        // release-group, which is normally the broadest available
        // bucket: editions/reissues without their own cover art
        // typically still inherit a group-level front.
        for release in &track_match.releases {
            let Some(release_group_mbid) = release.release_group_mbid.as_deref() else {
                continue;
            };
            if let Some(bytes) = self
                .cover_art_archive
                .fetch_release_group_front(release_group_mbid)?
            {
                return Ok(Some(FetchedArtwork {
                    bytes,
                    release_mbid: release.release_mbid.clone(),
                }));
            }
        }
        Ok(None)
    }
}

fn pick_best_musicbrainz(
    mut results: Vec<RecordingMatch>,
    min_score: u8,
) -> Option<RecordingMatch> {
    results.sort_by_key(|recording| std::cmp::Reverse(recording.score));
    results
        .into_iter()
        .find(|recording| recording.score >= min_score && !recording.releases.is_empty())
}

fn recording_to_match(recording: RecordingMatch, source: TrackMatchSource) -> TrackMatch {
    let releases = recording
        .releases
        .into_iter()
        .map(|release| TrackMatchRelease {
            release_mbid: release.release_mbid,
            release_group_mbid: release.release_group_mbid,
            title: release.title,
            year: release.year,
            track_number: release.track_number,
            track_total: release.track_total,
            disc_number: release.disc_number,
        })
        .collect();
    TrackMatch {
        recording_mbid: recording.recording_mbid,
        title: recording.title,
        artist: recording.artist,
        releases,
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::musicbrainz::RecordingRelease;

    fn make_recording(score: u8, release_mbid: &str) -> RecordingMatch {
        RecordingMatch {
            recording_mbid: format!("rec-{score}"),
            title: Some("Title".to_owned()),
            artist: Some("Artist".to_owned()),
            duration_ms: Some(200_000),
            score,
            releases: vec![RecordingRelease {
                release_mbid: release_mbid.to_owned(),
                release_group_mbid: None,
                title: Some("Album".to_owned()),
                year: Some(2000),
                track_number: Some(1),
                track_total: Some(10),
                disc_number: Some(1),
            }],
        }
    }

    #[test]
    fn pick_best_returns_highest_scoring_match_above_threshold() {
        let results = vec![
            make_recording(70, "release-a"),
            make_recording(95, "release-b"),
            make_recording(85, "release-c"),
        ];
        let best = pick_best_musicbrainz(results, MUSICBRAINZ_CONFIDENT_SCORE);
        assert_eq!(
            best.expect("test seeded a passing match").recording_mbid,
            "rec-95"
        );
    }

    #[test]
    fn pick_best_returns_none_when_no_score_passes_threshold() {
        let results = vec![
            make_recording(60, "release-a"),
            make_recording(70, "release-b"),
        ];
        assert!(pick_best_musicbrainz(results, MUSICBRAINZ_CONFIDENT_SCORE).is_none());
    }

    #[test]
    fn pick_best_skips_releaseless_recordings() {
        let mut releaseless = make_recording(100, "release-a");
        releaseless.releases.clear();
        let results = vec![releaseless, make_recording(91, "release-b")];
        assert_eq!(
            pick_best_musicbrainz(results, MUSICBRAINZ_CONFIDENT_SCORE)
                .expect("releaseless candidates must be skipped, not block the search")
                .recording_mbid,
            "rec-91"
        );
    }

    #[test]
    fn track_query_text_detection_is_blank_safe() {
        assert!(!TrackQuery::default().has_any_text());
        assert!(
            !TrackQuery {
                artist: Some("  ".to_owned()),
                title: Some(String::new()),
                ..Default::default()
            }
            .has_any_text()
        );
        assert!(
            TrackQuery {
                artist: Some("Bowie".to_owned()),
                ..Default::default()
            }
            .has_any_text()
        );
    }

    #[test]
    fn recording_to_match_preserves_release_order() {
        let mut recording = make_recording(95, "release-a");
        recording.releases.push(RecordingRelease {
            release_mbid: "release-b".to_owned(),
            release_group_mbid: Some("group-b".to_owned()),
            title: None,
            year: None,
            track_number: None,
            track_total: None,
            disc_number: None,
        });
        let track_match = recording_to_match(recording, TrackMatchSource::MusicBrainzTags);
        assert_eq!(track_match.releases[0].release_mbid, "release-a");
        assert_eq!(track_match.releases[1].release_mbid, "release-b");
    }
}
