// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! MusicBrainz Web Service v2 client.
//!
//! Two operations matter for Sustain today:
//!
//! 1. **Recording search**: given the local tag values we already have
//!    (artist, title, optional album and duration), find the best
//!    MusicBrainz recording that matches them. The recording carries
//!    the canonical metadata fields and points at the releases it
//!    appears on, which is what we need to look up cover art.
//! 2. **Recording lookup by ID**: confirm an AcoustID match by
//!    resolving its recording MBID into a full record. AcoustID's
//!    lookup returns recording IDs only; the rest of the metadata
//!    lives on MusicBrainz proper.
//!
//! The client deliberately does *not* expose every field MusicBrainz
//! returns. It exposes the small structured view that Sustain's
//! [`crate::service::RemoteMetadataService`] can collapse into either
//! [`crate::service::TrackMatch`] or `FetchedArtwork`. Adding fields
//! later is cheap; surfacing the whole MusicBrainz schema upfront
//! would couple the consumer to provider details it should not care
//! about.

use std::sync::Arc;

use serde::Deserialize;

use crate::client::HttpClient;
use crate::error::RemoteResult;
use crate::mbid::is_well_formed;

const SEARCH_BASE: &str = "https://musicbrainz.org/ws/2/recording/";
const LOOKUP_BASE: &str = "https://musicbrainz.org/ws/2/recording";
/// Includes for the lookup-by-id endpoint. We need release-level
/// details (and the release-group MBID) to drive Cover Art Archive
/// fallbacks; artist credit comes along for the read-back display.
const LOOKUP_INCLUDES: &str = "releases+release-groups+artist-credits+media";

/// Number of recordings the search asks MusicBrainz for. Five is
/// enough to disambiguate noisy tags (a track with no album, several
/// remasters, etc.) without making MusicBrainz do more index work
/// than necessary.
const SEARCH_LIMIT: u32 = 5;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordingSearchTerms {
    pub artist: Option<String>,
    pub title: Option<String>,
    pub album: Option<String>,
    /// Track duration in milliseconds, when known. MusicBrainz scores
    /// matches partly by length, so passing this consistently
    /// improves precision on common-title tracks.
    pub duration_ms: Option<u64>,
}

impl RecordingSearchTerms {
    pub fn is_usable(&self) -> bool {
        self.title.as_deref().is_some_and(is_non_blank)
            || self.artist.as_deref().is_some_and(is_non_blank)
    }
}

/// A recording-level view of one search hit. Only the fields Sustain
/// can act on are surfaced; everything else (relations, ISRC, work
/// credits) is dropped at parse time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingMatch {
    pub recording_mbid: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub duration_ms: Option<u64>,
    /// Server-assigned score, 0..=100. Used to drop low-confidence
    /// matches before they reach the UI.
    pub score: u8,
    /// Releases the recording appears on, in MusicBrainz order. Empty
    /// for recordings that exist in the database but are not
    /// associated with any release — those are skipped by Cover Art
    /// Archive lookup.
    pub releases: Vec<RecordingRelease>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingRelease {
    pub release_mbid: String,
    pub release_group_mbid: Option<String>,
    pub title: Option<String>,
    pub year: Option<i32>,
    pub track_number: Option<u32>,
    pub track_total: Option<u32>,
    pub disc_number: Option<u32>,
}

#[derive(Clone)]
pub struct MusicBrainzClient {
    http: Arc<HttpClient>,
}

impl MusicBrainzClient {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    /// Search MusicBrainz for recordings matching the given terms.
    /// Returns an empty vector if the terms carry no usable text;
    /// MusicBrainz would otherwise reject the query as malformed.
    pub fn search_recordings(
        &self,
        terms: &RecordingSearchTerms,
    ) -> RemoteResult<Vec<RecordingMatch>> {
        if !terms.is_usable() {
            return Ok(Vec::new());
        }

        let query = build_search_query(terms);
        let url = format!(
            "{SEARCH_BASE}?query={query}&fmt=json&limit={SEARCH_LIMIT}",
            query = url_encode(&query),
        );

        let payload: SearchPayload = self.http.get_json(&url)?;
        Ok(payload
            .recordings
            .into_iter()
            .filter_map(into_recording_match)
            .collect())
    }

    /// Look up a recording by its MusicBrainz ID. This is the
    /// preferred path once an MBID is known (e.g. after an AcoustID
    /// fingerprint match) because the API returns the canonical
    /// record rather than a ranked search result.
    pub fn lookup_recording(&self, recording_mbid: &str) -> RemoteResult<Option<RecordingMatch>> {
        if !is_well_formed(recording_mbid) {
            return Ok(None);
        }
        let url = format!(
            "{LOOKUP_BASE}/{recording_mbid}?inc={includes}&fmt=json",
            includes = LOOKUP_INCLUDES,
        );
        let recording: RawRecording = match self.http.get_json(&url) {
            Ok(value) => value,
            Err(crate::error::RemoteError::BadStatus(404)) => return Ok(None),
            Err(error) => return Err(error),
        };
        // The lookup endpoint omits the `score` field — it returns
        // exactly one record. Synthesise a maximum score so the
        // caller's confidence checks pass uniformly.
        let recording = RawRecording {
            score: Some(100),
            ..recording
        };
        Ok(into_recording_match(recording))
    }
}

/// Construct the Lucene query MusicBrainz expects. Each field is
/// quoted and Lucene-escaped; missing fields are omitted entirely so
/// they don't constrain the match. Duration is expressed as a
/// loosely-bracketed range (±5s) because tag-derived durations
/// frequently drift from MusicBrainz's by a few hundred milliseconds.
fn build_search_query(terms: &RecordingSearchTerms) -> String {
    let mut clauses: Vec<String> = Vec::new();
    if let Some(title) = terms.title.as_deref().filter(|value| is_non_blank(value)) {
        clauses.push(format!("recording:\"{}\"", lucene_escape(title)));
    }
    if let Some(artist) = terms.artist.as_deref().filter(|value| is_non_blank(value)) {
        clauses.push(format!("artist:\"{}\"", lucene_escape(artist)));
    }
    if let Some(album) = terms.album.as_deref().filter(|value| is_non_blank(value)) {
        clauses.push(format!("release:\"{}\"", lucene_escape(album)));
    }
    if let Some(duration_ms) = terms.duration_ms {
        let lower = duration_ms.saturating_sub(5_000);
        let upper = duration_ms.saturating_add(5_000);
        clauses.push(format!("dur:[{lower} TO {upper}]"));
    }
    clauses.join(" AND ")
}

fn url_encode(value: &str) -> String {
    // Conservative percent-encoding: only the unreserved set per
    // RFC 3986 stays literal. The Lucene grammar itself is delivered
    // through the percent-encoded query — MusicBrainz decodes the
    // value before parsing, so quoted strings, colons, and bracketed
    // ranges all reach the parser intact. Encoding strictly here
    // avoids URL-parse rejections by HTTP clients that refuse raw
    // reserved characters (notably brackets) in the query.
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        let safe = matches!(
            byte,
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~'
        );
        if safe {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn lucene_escape(value: &str) -> String {
    // MusicBrainz's Lucene-based query parser treats these characters
    // as syntax; escape them with a backslash so user-supplied strings
    // (which routinely contain `+`, `-`, `&`, `:`, etc.) are treated as
    // text. We do not escape double quotes — the caller wraps the
    // entire value in quotes already, and escaping the quote inside a
    // quoted value would terminate the field early. If the input
    // contains an actual `"`, we drop it: there is no clean way to
    // embed a quote inside a quoted Lucene value without significantly
    // expanding the grammar we accept here.
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => continue,
            '\\' | '+' | '-' | '!' | '(' | ')' | '{' | '}' | '[' | ']' | '^' | '~' | '*' | '?'
            | ':' | '/' | '&' | '|' => {
                escaped.push('\\');
                escaped.push(character);
            }
            _ => escaped.push(character),
        }
    }
    escaped
}

fn into_recording_match(raw: RawRecording) -> Option<RecordingMatch> {
    if raw.id.is_empty() {
        return None;
    }
    let releases = raw
        .releases
        .unwrap_or_default()
        .into_iter()
        .filter_map(into_recording_release)
        .collect();
    Some(RecordingMatch {
        recording_mbid: raw.id,
        title: raw.title.filter(|value| is_non_blank(value)),
        artist: raw
            .artist_credit
            .as_deref()
            .and_then(format_artist_credit)
            .filter(|value| is_non_blank(value)),
        duration_ms: raw.length,
        score: raw.score.unwrap_or(0).min(100),
        releases,
    })
}

fn into_recording_release(raw: RawRelease) -> Option<RecordingRelease> {
    if raw.id.is_empty() {
        return None;
    }
    let (track_number, track_total, disc_number) = raw
        .media
        .as_deref()
        .map(extract_track_position)
        .unwrap_or((None, None, None));
    Some(RecordingRelease {
        release_mbid: raw.id,
        release_group_mbid: raw
            .release_group
            .as_ref()
            .map(|group| group.id.clone())
            .filter(|value| is_non_blank(value)),
        title: raw.title.filter(|value| is_non_blank(value)),
        year: raw.date.as_deref().and_then(parse_year),
        track_number,
        track_total,
        disc_number,
    })
}

fn format_artist_credit(credits: &[RawArtistCredit]) -> Option<String> {
    if credits.is_empty() {
        return None;
    }
    let mut output = String::new();
    for credit in credits {
        if let Some(name) = credit.name.as_deref() {
            output.push_str(name);
        } else if let Some(artist) = &credit.artist
            && let Some(name) = artist.name.as_deref()
        {
            output.push_str(name);
        }
        if let Some(joinphrase) = credit.joinphrase.as_deref() {
            output.push_str(joinphrase);
        }
    }
    Some(output.trim().to_owned())
}

fn extract_track_position(media: &[RawMedium]) -> (Option<u32>, Option<u32>, Option<u32>) {
    for medium in media {
        let Some(tracks) = medium.tracks.as_deref() else {
            continue;
        };
        if let Some(track) = tracks.first() {
            return (
                track.number.as_deref().and_then(parse_track_position),
                medium.track_count,
                medium.position,
            );
        }
    }
    (None, None, None)
}

fn parse_track_position(value: &str) -> Option<u32> {
    // MusicBrainz exposes the track's printed position, which can be
    // alphanumeric on vinyl ("A1", "B2"). For our purposes we only
    // recover the integer prefix when one exists.
    let digits: String = value
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn parse_year(value: &str) -> Option<i32> {
    // MusicBrainz date strings are ISO-ish: "1973", "1973-05", "1973-05-12".
    // We only want the year prefix.
    value
        .split('-')
        .next()
        .and_then(|year| year.parse::<i32>().ok())
}

fn is_non_blank(value: &str) -> bool {
    !value.trim().is_empty()
}

#[derive(Deserialize)]
struct SearchPayload {
    #[serde(default)]
    recordings: Vec<RawRecording>,
}

#[derive(Deserialize)]
struct RawRecording {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    length: Option<u64>,
    #[serde(default)]
    score: Option<u8>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Option<Vec<RawArtistCredit>>,
    #[serde(default)]
    releases: Option<Vec<RawRelease>>,
}

#[derive(Deserialize)]
struct RawArtistCredit {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    joinphrase: Option<String>,
    #[serde(default)]
    artist: Option<RawArtist>,
}

#[derive(Deserialize)]
struct RawArtist {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct RawRelease {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(rename = "release-group", default)]
    release_group: Option<RawReleaseGroup>,
    #[serde(default)]
    media: Option<Vec<RawMedium>>,
}

#[derive(Deserialize)]
struct RawReleaseGroup {
    #[serde(default)]
    id: String,
}

#[derive(Deserialize)]
struct RawMedium {
    #[serde(default)]
    position: Option<u32>,
    #[serde(rename = "track-count", default)]
    track_count: Option<u32>,
    #[serde(default)]
    tracks: Option<Vec<RawTrack>>,
}

#[derive(Deserialize)]
struct RawTrack {
    #[serde(default)]
    number: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lucene_escape_protects_syntax_characters() {
        assert_eq!(lucene_escape("AC/DC"), "AC\\/DC");
        assert_eq!(lucene_escape("a+b"), "a\\+b");
        assert_eq!(lucene_escape("title: subtitle"), "title\\: subtitle");
    }

    #[test]
    fn lucene_escape_drops_inner_quotes() {
        assert_eq!(lucene_escape("She said \"hello\""), "She said hello");
    }

    #[test]
    fn url_encode_passes_safe_characters_through() {
        assert_eq!(url_encode("abc-123.xyz"), "abc-123.xyz");
    }

    #[test]
    fn url_encode_escapes_unsafe_characters() {
        assert_eq!(url_encode("a b"), "a%20b");
        assert_eq!(url_encode("foo&bar"), "foo%26bar");
    }

    #[test]
    fn parse_year_handles_partial_dates() {
        assert_eq!(parse_year("1973"), Some(1973));
        assert_eq!(parse_year("1973-05"), Some(1973));
        assert_eq!(parse_year("1973-05-12"), Some(1973));
        assert_eq!(parse_year("not a year"), None);
    }

    #[test]
    fn parse_track_position_recovers_integer_prefix() {
        assert_eq!(parse_track_position("3"), Some(3));
        assert_eq!(parse_track_position("12"), Some(12));
        assert_eq!(parse_track_position("A1"), None);
        assert_eq!(parse_track_position(""), None);
    }

    #[test]
    fn search_query_skips_blank_fields() {
        let terms = RecordingSearchTerms {
            artist: Some("  ".to_owned()),
            title: Some("Stairway".to_owned()),
            album: None,
            duration_ms: None,
        };
        assert_eq!(build_search_query(&terms), "recording:\"Stairway\"");
    }

    #[test]
    fn search_query_includes_all_fields_when_present() {
        let terms = RecordingSearchTerms {
            artist: Some("Led Zeppelin".to_owned()),
            title: Some("Stairway to Heaven".to_owned()),
            album: Some("Led Zeppelin IV".to_owned()),
            duration_ms: Some(482_000),
        };
        assert_eq!(
            build_search_query(&terms),
            "recording:\"Stairway to Heaven\" AND artist:\"Led Zeppelin\" AND release:\"Led Zeppelin IV\" AND dur:[477000 TO 487000]"
        );
    }

    #[test]
    fn unusable_terms_skip_the_network_call() {
        let http = Arc::new(HttpClient::new(crate::client::HttpClientConfig {
            user_agent: "test".to_owned(),
        }));
        let client = MusicBrainzClient::new(http);
        let empty = RecordingSearchTerms::default();
        let result = client
            .search_recordings(&empty)
            .expect("blank terms must not error");
        assert!(result.is_empty());
    }

    #[test]
    fn artist_credit_falls_back_to_nested_artist_name() {
        let credits = vec![
            RawArtistCredit {
                name: None,
                joinphrase: Some(" & ".to_owned()),
                artist: Some(RawArtist {
                    name: Some("Simon".to_owned()),
                }),
            },
            RawArtistCredit {
                name: Some("Garfunkel".to_owned()),
                joinphrase: None,
                artist: None,
            },
        ];
        assert_eq!(
            format_artist_credit(&credits).as_deref(),
            Some("Simon & Garfunkel")
        );
    }
}
