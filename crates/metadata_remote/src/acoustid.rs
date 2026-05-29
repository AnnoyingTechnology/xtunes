// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! AcoustID lookup client.
//!
//! AcoustID identifies recordings from a Chromaprint audio
//! fingerprint, which makes it the only path for tracks whose tags
//! are too sparse for MusicBrainz to match by text. The lookup
//! returns one or more candidate MusicBrainz recording IDs with a
//! confidence score; Sustain then uses MusicBrainz to resolve those
//! IDs into real metadata and Cover Art Archive to fetch the cover.
//!
//! ## Scope of this module
//!
//! This module owns the API call only. Computing the actual
//! Chromaprint fingerprint requires decoding the audio file and is
//! out of scope here — it'll land alongside the bulk
//! `Fetch missing tags` action, where the cost of pulling in a
//! Chromaprint dependency is justified. The client accepts a
//! precomputed [`AudioFingerprint`] so the high-level service can be
//! wired today and the fingerprint side can be added cleanly later.
//!
//! ## Configuration
//!
//! AcoustID requires an application API key. Sustain treats it as a
//! build-time secret: the `app` crate reads it from a compile-time
//! environment variable and passes it to the composed service at
//! startup. Builds without a key still work — the lookup just
//! returns [`RemoteError::NotConfigured`], which the caller handles
//! by skipping fingerprint-based identification.

use std::sync::Arc;

use serde::Deserialize;

use crate::client::HttpClient;
use crate::error::{RemoteError, RemoteResult};
use crate::http::url_encode;

const LOOKUP_BASE: &str = "https://api.acoustid.org/v2/lookup";

/// Precomputed Chromaprint fingerprint of an audio file. The
/// fingerprint is the canonical base-64-ish text string emitted by
/// Chromaprint (the `fpcalc` CLI prints it as `FINGERPRINT=...`);
/// `duration_seconds` is the integer-truncated track length the
/// fingerprint was computed against.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioFingerprint {
    pub chromaprint: String,
    pub duration_seconds: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AcoustIdMatch {
    /// AcoustID's confidence score for this candidate, 0.0..=1.0.
    pub score: f32,
    /// MusicBrainz recording IDs the fingerprint maps to. AcoustID
    /// may associate several recordings with the same fingerprint
    /// (different masters, alternate versions); the caller resolves
    /// each candidate against MusicBrainz to pick the best one.
    pub recording_mbids: Vec<String>,
}

#[derive(Clone)]
pub struct AcoustIdClient {
    http: Arc<HttpClient>,
    api_key: String,
}

impl AcoustIdClient {
    /// Construct a configured client. The key must be a real
    /// AcoustID application key — empty or whitespace keys are
    /// treated as unconfigured.
    pub fn new(http: Arc<HttpClient>, api_key: impl Into<String>) -> Option<Self> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return None;
        }
        Some(Self { http, api_key })
    }

    /// Look up a fingerprint and return the AcoustID-side candidate
    /// matches. The matches do not carry any descriptive metadata —
    /// only MBIDs and scores. Resolving them is the caller's job.
    pub fn lookup_fingerprint(
        &self,
        fingerprint: &AudioFingerprint,
    ) -> RemoteResult<Vec<AcoustIdMatch>> {
        if fingerprint.chromaprint.trim().is_empty() {
            return Ok(Vec::new());
        }

        let url = format!(
            "{LOOKUP_BASE}?client={client}&meta=recordingids&format=json&duration={duration}&fingerprint={fingerprint}",
            client = url_encode(&self.api_key),
            duration = fingerprint.duration_seconds,
            fingerprint = url_encode(&fingerprint.chromaprint),
        );

        let payload: LookupPayload = self.http.get_json(&url)?;
        if payload.status != "ok" {
            return Err(RemoteError::InvalidResponse);
        }
        Ok(payload
            .results
            .unwrap_or_default()
            .into_iter()
            .filter_map(into_match)
            .collect())
    }
}

fn into_match(raw: RawResult) -> Option<AcoustIdMatch> {
    let recordings = raw.recordings.unwrap_or_default();
    let recording_mbids: Vec<String> = recordings
        .into_iter()
        .map(|recording| recording.id)
        .filter(|id| !id.is_empty())
        .collect();
    if recording_mbids.is_empty() {
        return None;
    }
    Some(AcoustIdMatch {
        score: raw.score.unwrap_or(0.0).clamp(0.0, 1.0),
        recording_mbids,
    })
}

#[derive(Deserialize)]
struct LookupPayload {
    #[serde(default)]
    status: String,
    #[serde(default)]
    results: Option<Vec<RawResult>>,
}

#[derive(Deserialize)]
struct RawResult {
    #[serde(default)]
    score: Option<f32>,
    #[serde(default)]
    recordings: Option<Vec<RawRecordingId>>,
}

#[derive(Deserialize)]
struct RawRecordingId {
    #[serde(default)]
    id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_api_key_is_rejected() {
        let http = Arc::new(HttpClient::new(crate::client::HttpClientConfig {
            user_agent: "test".to_owned(),
        }));
        assert!(AcoustIdClient::new(http, "").is_none());
    }

    #[test]
    fn whitespace_api_key_is_rejected() {
        let http = Arc::new(HttpClient::new(crate::client::HttpClientConfig {
            user_agent: "test".to_owned(),
        }));
        assert!(AcoustIdClient::new(http, "   ").is_none());
    }

    #[test]
    fn blank_fingerprint_skips_the_network_call() {
        let http = Arc::new(HttpClient::new(crate::client::HttpClientConfig {
            user_agent: "test".to_owned(),
        }));
        let client = AcoustIdClient::new(http, "key").expect("client constructs with a key");
        let result = client
            .lookup_fingerprint(&AudioFingerprint {
                chromaprint: String::new(),
                duration_seconds: 0,
            })
            .expect("blank fingerprint returns empty");
        assert!(result.is_empty());
    }
}
