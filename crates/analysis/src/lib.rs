// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Audio analysis for Sustain — pure DSP, no I/O beyond reading the
//! audio file the caller hands us.
//!
//! The crate exposes a single public entry point, [`analyze`], which
//! decodes the file once and returns:
//!   - detected BPM (octave-normalized to a configurable range)
//!   - detected musical key
//!   - a beat grid placeholder (extraction lives in a future revision)
//!   - two waveform tiers: a fixed-resolution preview (~400 segments,
//!     full-track overview) and a time-resolution detail
//!     (150 segments/sec).
//!
//! Each waveform segment carries the per-window peak amplitude **and**
//! per-band RMS energies (low / mid / high) so the renderer in
//! `sustain-ui-gtk` can draw either monochrome or color waveforms, and
//! a future Pioneer export crate can derive PWAV/PWV2/PWV3/PWV4/PWV5
//! byte formats without re-decoding the audio.
//!
//! The crate is intentionally I/O-free beyond the audio file itself:
//! no SQLite, no settings, no tag writes. Persistence, paced
//! scheduling, and the user-visible "needs analysis" bookkeeping live
//! in `sustain-library-store` and `sustain-app-runtime` respectively.

#![forbid(unsafe_code)]

mod bands;
mod decode;
mod key_bpm;
mod waveform;

use std::path::Path;

// Re-exported from sustain_domain so callers can `use sustain_analysis::*`
// without also pulling sustain_domain into their imports for what is
// conceptually one cohesive surface. The canonical home for these types
// is the domain layer — the storage crate needs them but should not
// pull in symphonia / stratum-dsp.
pub use sustain_domain::{
    BeatGrid, DETAIL_SEGMENTS_PER_SECOND, MusicalKey, PREVIEW_SEGMENT_COUNT, TrackAnalysis,
    WaveformSegment, WaveformSegments,
};

/// Monotonically-increasing identifier for the DSP algorithms in this
/// crate. Bumped centrally when a change to the band split, BPM/key
/// engine, or waveform encoding would invalidate previously-stored
/// `track_analysis` rows. The storage layer compares stored rows
/// against this value to decide whether a track should be re-queued
/// by the runtime scheduler — no migration code, just a version
/// bump that the scheduler walks past in the background.
pub const ANALYZER_VERSION: u32 = 1;

/// Tunables exposed to callers. Defaults reflect the values the
/// rhythmbox-to-pioneer-xdj-exporter author landed on after testing on
/// a large DJ-style library, with one Sustain-specific deviation:
/// no max-sample cap is applied to the waveform decode (the whole
/// track is decoded so the preview/detail reflect the full audio),
/// while BPM/key detection still observes the 120-second cap to keep
/// the working set bounded.
#[derive(Clone, Copy, Debug)]
pub struct AnalysisOptions {
    pub detect_bpm: bool,
    pub detect_key: bool,
    /// Lower bound of the BPM range used for octave normalization
    /// (detected BPM is doubled while below this value, if doubling
    /// keeps it inside the range).
    pub min_bpm: f32,
    /// Upper bound of the BPM range used for octave normalization
    /// (detected BPM is halved while above this value, if halving
    /// keeps it inside the range).
    pub max_bpm: f32,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            detect_bpm: true,
            detect_key: true,
            min_bpm: 70.0,
            max_bpm: 170.0,
        }
    }
}

/// Failure modes produced by [`analyze`]. Callers can choose to skip
/// individual tracks or persist a failure marker without crashing the
/// batch.
#[derive(Debug, thiserror::Error)]
pub enum AnalysisError {
    #[error("failed to open audio file {path}: {source}")]
    OpenFailed {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("audio decoder error for {path}: {message}")]
    DecoderError { path: String, message: String },
    #[error("audio file {path} contains no usable audio track")]
    NoAudioTrack { path: String },
    #[error("audio file {path} is too short for analysis ({samples} samples)")]
    TooShort { path: String, samples: usize },
    #[error("DSP engine failed for {path}: {message}")]
    DspFailed { path: String, message: String },
}

/// Analyze the audio file at `path` and return BPM, key, and both
/// waveform tiers. Decodes the audio twice: once capped to the BPM
/// window for the key/BPM pass, once in full for the waveform pass.
/// The two passes are independent so a decoder error in one does not
/// invalidate the other; if BPM/key detection fails the corresponding
/// fields are `None` and the waveforms still ship.
pub fn analyze(path: &Path, opts: AnalysisOptions) -> Result<TrackAnalysis, AnalysisError> {
    let (bpm, key) = if opts.detect_bpm || opts.detect_key {
        match key_bpm::detect(path, opts) {
            Ok(found) => (
                if opts.detect_bpm { found.bpm } else { None },
                if opts.detect_key { found.key } else { None },
            ),
            Err(_) => (None, None),
        }
    } else {
        (None, None)
    };

    let waveform = waveform::generate(path)?;

    Ok(TrackAnalysis {
        bpm,
        key,
        beatgrid: None,
        waveform_preview: waveform.preview,
        waveform_detail: waveform.detail,
    })
}
