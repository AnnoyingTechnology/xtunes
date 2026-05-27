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

use sustain_domain::MusicalKey;

pub use crate::waveform::{
    DETAIL_SEGMENTS_PER_SECOND, PREVIEW_SEGMENT_COUNT, WaveformSegment, WaveformSegments,
};

/// Result of analyzing a single track.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackAnalysis {
    /// Detected tempo in beats per minute, octave-normalized to fall
    /// within [`AnalysisOptions::min_bpm`]..=[`AnalysisOptions::max_bpm`].
    /// `None` if BPM detection was disabled, the file is too short, or
    /// the underlying engine could not produce a confident value.
    pub bpm: Option<f32>,
    /// Detected musical key. `None` if key detection was disabled or
    /// the underlying engine returned a value we could not map onto
    /// [`MusicalKey`] (rare; only happens for non-standard names).
    pub key: Option<MusicalKey>,
    /// Beat positions in milliseconds from the start of the audio,
    /// derived alongside BPM. The first revision of this crate
    /// always returns `None`; the field is reserved so the storage
    /// schema does not have to grow when beat extraction lands.
    pub beatgrid: Option<BeatGrid>,
    /// Fixed-resolution overview (PREVIEW_SEGMENT_COUNT segments).
    /// Suitable for a thumbnail / pre-roll waveform; constant size
    /// regardless of track length.
    pub waveform_preview: WaveformSegments,
    /// Time-resolution detail (DETAIL_SEGMENTS_PER_SECOND segments per
    /// second of audio). Suitable for the active-track scrubber and
    /// for re-encoding into hardware-specific formats downstream.
    pub waveform_detail: WaveformSegments,
}

/// Beat-grid information. Reserved; not populated yet.
#[derive(Clone, Debug, PartialEq)]
pub struct BeatGrid {
    /// BPM the beats were laid out against. Matches
    /// [`TrackAnalysis::bpm`] in the common case but is kept on the
    /// grid itself so renderers do not need to look at two fields.
    pub bpm: f32,
    /// Beat positions, in milliseconds from the start of audio.
    pub beats: Vec<f32>,
    /// Subset of `beats` marking the first beat of each bar.
    pub downbeats: Vec<f32>,
}

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
