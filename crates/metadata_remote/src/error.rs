// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Error surface shared by every networked metadata client.
//!
//! Per-provider failure modes collapse into a small enum so the
//! caller doesn't have to learn three different vocabularies. The
//! variants are coarse on purpose — the application has the same
//! recourse for "network failed" and "HTTP 502" (retry or give up),
//! and the user-facing message at the UI layer is the same string.
//! Finer-grained diagnostics belong in log lines, not in the type
//! the runtime branches on.

use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteError {
    /// Network reachability or transport failure (DNS, TCP, TLS,
    /// timeout). The user might fix it by checking connectivity; the
    /// app's job is to back off without spinning on the failure.
    Network,
    /// Server responded but with a status code we cannot use. Held
    /// for diagnostics; the UI does not branch on the specific code.
    BadStatus(u16),
    /// Server responded with a payload that did not match the
    /// expected schema (truncated JSON, unexpected shape, missing
    /// fields we cannot recover from).
    InvalidResponse,
    /// The remote provider is not configured (e.g. AcoustID requires
    /// an application key that was not built into the binary). The
    /// caller is expected to skip the feature gracefully.
    NotConfigured,
}

impl fmt::Display for RemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network => f.write_str("network unavailable"),
            Self::BadStatus(code) => write!(f, "remote service returned HTTP {code}"),
            Self::InvalidResponse => f.write_str("remote service returned an unexpected payload"),
            Self::NotConfigured => f.write_str("remote service not configured"),
        }
    }
}

impl std::error::Error for RemoteError {}

pub type RemoteResult<T> = Result<T, RemoteError>;
