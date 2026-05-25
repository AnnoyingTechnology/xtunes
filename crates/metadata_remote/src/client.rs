// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Blocking HTTP client wrapper with per-host rate limiting.
//!
//! The remote providers (MusicBrainz, Cover Art Archive, AcoustID) all
//! publish strict rate-limit policies. MusicBrainz caps a single
//! application at one request per second, and Cover Art Archive — which
//! is a thin redirect layer on top of the MusicBrainz database — shares
//! that ceiling. AcoustID is more permissive but documents a soft
//! three-per-second limit. Sending a burst of requests will quickly
//! return HTTP 503 and, with repeated offence, get the User-Agent
//! blocked. The wrapper enforces a minimum gap per host so concurrent
//! requests across providers cannot collectively violate either policy.
//!
//! Synchronous blocking I/O is intentional. The networked metadata path
//! is gated behind explicit user actions (a click, a settings tickbox);
//! its callers always run on a dedicated worker thread that already owns
//! a `RemoteMetadataService` handle. Bringing in a Tokio runtime just to
//! issue one HTTPS request would be substantially heavier than the
//! request itself.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};

use serde::de::DeserializeOwned;
use ureq::Agent;

use crate::error::{RemoteError, RemoteResult};

/// Minimum gap MusicBrainz enforces for a single client; we match it
/// exactly rather than running close to the edge.
pub const MUSICBRAINZ_MIN_REQUEST_GAP: Duration = Duration::from_millis(1_050);

/// Cover Art Archive piggybacks on MusicBrainz infrastructure, so the
/// same gap applies to it.
pub const COVER_ART_ARCHIVE_MIN_REQUEST_GAP: Duration = MUSICBRAINZ_MIN_REQUEST_GAP;

/// AcoustID documents a soft three-per-second limit for fingerprint
/// lookups; we sit just under it.
pub const ACOUSTID_MIN_REQUEST_GAP: Duration = Duration::from_millis(360);

/// HTTP request timeout. The remote endpoints answer in well under a
/// second on a healthy network; anything past a few seconds is almost
/// certainly a network problem rather than a slow server. Failing fast
/// keeps the UI's spinner from staring at a hung socket.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Standard `Accept` header for JSON endpoints across all three
/// providers. MusicBrainz and AcoustID return JSON when asked; Cover
/// Art Archive's `/release/{mbid}` endpoint also returns JSON for the
/// index and binary for the image endpoints.
const ACCEPT_JSON: &str = "application/json";

/// User-supplied configuration for the HTTP client. The User-Agent
/// matters: MusicBrainz documents that requests without a meaningful
/// User-Agent are subject to rate-limit blocks and the contact URL is
/// expected to be reachable for abuse reports.
#[derive(Clone, Debug)]
pub struct HttpClientConfig {
    pub user_agent: String,
}

/// Per-host rate limit policy. Hosts not listed fall through to no
/// limiter — a convenience for tests that hit a localhost mock.
struct RateLimitPolicy {
    minimum_gap: Duration,
}

#[derive(Default)]
struct HostState {
    last_request_at: Option<Instant>,
}

pub struct HttpClient {
    agent: Agent,
    user_agent: String,
    rate_limits: HashMap<&'static str, RateLimitPolicy>,
    host_states: Mutex<HashMap<&'static str, HostState>>,
}

impl HttpClient {
    pub fn new(config: HttpClientConfig) -> Self {
        let agent: Agent = Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            .build()
            .into();

        let mut rate_limits = HashMap::new();
        rate_limits.insert(
            "musicbrainz.org",
            RateLimitPolicy {
                minimum_gap: MUSICBRAINZ_MIN_REQUEST_GAP,
            },
        );
        rate_limits.insert(
            "coverartarchive.org",
            RateLimitPolicy {
                minimum_gap: COVER_ART_ARCHIVE_MIN_REQUEST_GAP,
            },
        );
        rate_limits.insert(
            "api.acoustid.org",
            RateLimitPolicy {
                minimum_gap: ACOUSTID_MIN_REQUEST_GAP,
            },
        );

        Self {
            agent,
            user_agent: config.user_agent,
            rate_limits,
            host_states: Mutex::new(HashMap::new()),
        }
    }

    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// Issues a GET that returns deserialised JSON. Caller is
    /// responsible for assembling the full URL (including query
    /// parameters) — the rate limiter and User-Agent are applied here.
    pub fn get_json<T: DeserializeOwned>(&self, url: &str) -> RemoteResult<T> {
        self.respect_rate_limit(url);
        let response = self
            .agent
            .get(url)
            .header("User-Agent", &self.user_agent)
            .header("Accept", ACCEPT_JSON)
            .call()
            .map_err(|error| map_ureq_error(error, url))?;

        if !response.status().is_success() {
            return Err(RemoteError::BadStatus(response.status().as_u16()));
        }

        response
            .into_body()
            .read_json::<T>()
            .map_err(|_| RemoteError::InvalidResponse)
    }

    /// Issues a GET that returns a raw byte payload, following
    /// redirects (Cover Art Archive's image endpoints reply with a 307
    /// to the actual image URL on archive.org). Returns `None` for 404,
    /// which the providers use to indicate "no image present" and is
    /// not an error in our model.
    pub fn get_bytes(&self, url: &str) -> RemoteResult<Option<Vec<u8>>> {
        self.respect_rate_limit(url);
        let response = match self
            .agent
            .get(url)
            .header("User-Agent", &self.user_agent)
            .call()
        {
            Ok(response) => response,
            Err(error) => return map_get_bytes_error(error, url),
        };

        let status = response.status();
        if status.as_u16() == 404 {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(RemoteError::BadStatus(status.as_u16()));
        }

        let bytes = response
            .into_body()
            .read_to_vec()
            .map_err(|_| RemoteError::InvalidResponse)?;
        if bytes.is_empty() {
            return Ok(None);
        }
        Ok(Some(bytes))
    }

    fn respect_rate_limit(&self, url: &str) {
        let Some(host) = host_from_url(url) else {
            return;
        };
        let Some(policy) = self.rate_limits.get(host) else {
            return;
        };

        // We use a single mutex guarding the whole host-state map. The
        // lock is only held long enough to read/update one timestamp;
        // the sleep itself happens outside any lock so other threads
        // targeting different hosts are not blocked behind us.
        let sleep_for = {
            let mut states = match self.host_states.lock() {
                Ok(guard) => guard,
                Err(poisoned) => recover_poisoned(poisoned),
            };
            let entry = states.entry(host).or_default();
            let now = Instant::now();
            let sleep_for = entry
                .last_request_at
                .map(|last| {
                    let elapsed = now.duration_since(last);
                    if elapsed >= policy.minimum_gap {
                        Duration::ZERO
                    } else {
                        policy.minimum_gap - elapsed
                    }
                })
                .unwrap_or_default();
            // Record the time we *intend* to fire — not the time we
            // actually fire — so concurrent threads serialise against
            // an honest schedule even if some of them are still sleeping.
            entry.last_request_at = Some(now + sleep_for);
            sleep_for
        };

        if !sleep_for.is_zero() {
            thread::sleep(sleep_for);
        }
    }
}

fn map_ureq_error(error: ureq::Error, url: &str) -> RemoteError {
    match error {
        ureq::Error::StatusCode(code) => RemoteError::BadStatus(code),
        ureq::Error::Timeout(_) => RemoteError::Network,
        // Transport-level failures (DNS, TLS, connect refused, etc.).
        // Log the underlying ureq error so diagnosis isn't blocked by
        // the coarse user-facing `RemoteError::Network` value — this
        // is the only place provider-side detail surfaces.
        other => {
            eprintln!("Sustain: remote request to {url} failed: {other:?}");
            RemoteError::Network
        }
    }
}

fn map_get_bytes_error(error: ureq::Error, url: &str) -> RemoteResult<Option<Vec<u8>>> {
    if let ureq::Error::StatusCode(404) = error {
        return Ok(None);
    }
    Err(map_ureq_error(error, url))
}

fn host_from_url(url: &str) -> Option<&'static str> {
    // We don't take a `url` crate dependency just to extract the host;
    // a tiny string scan is enough for our small fixed set of
    // recognised hosts. Returning &'static str lets the host string be
    // a key directly into the rate-limit map.
    const HOSTS: &[&str] = &["musicbrainz.org", "coverartarchive.org", "api.acoustid.org"];
    HOSTS
        .iter()
        .copied()
        .find(|host| url_matches_host(url, host))
}

fn url_matches_host(url: &str, host: &str) -> bool {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let authority = after_scheme.split(['/', '?']).next().unwrap_or("");
    let authority = authority
        .split_once('@')
        .map_or(authority, |(_, rest)| rest);
    let authority = authority
        .split_once(':')
        .map_or(authority, |(host, _)| host);
    authority.eq_ignore_ascii_case(host)
}

fn recover_poisoned<T>(poisoned: std::sync::PoisonError<MutexGuard<'_, T>>) -> MutexGuard<'_, T> {
    // Poisoning here would mean another thread panicked while updating
    // a rate-limit timestamp. The stored state is just a `HashMap` of
    // `Instant`s; nothing about it is corrupted by a panic during an
    // update, so recovering and continuing is safe. We do not propagate
    // the panic — a single network call should not bring down every
    // future fetch.
    poisoned.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_known_hosts_case_insensitively() {
        assert_eq!(
            host_from_url("https://MusicBrainz.org/ws/2/recording/"),
            Some("musicbrainz.org")
        );
        assert_eq!(
            host_from_url("https://coverartarchive.org/release/abc/front"),
            Some("coverartarchive.org")
        );
        assert_eq!(
            host_from_url("https://api.acoustid.org/v2/lookup"),
            Some("api.acoustid.org")
        );
    }

    #[test]
    fn ignores_unknown_hosts() {
        assert_eq!(host_from_url("https://example.com/whatever"), None);
        assert_eq!(host_from_url("not a url"), None);
    }
}
