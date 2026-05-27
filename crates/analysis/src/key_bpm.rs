// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! BPM and musical-key detection. Wraps `stratum-dsp` and maps its
//! string-form key labels onto [`MusicalKey`]. No file-tag write-back
//! side-channel — Sustain's policy is that SQLite owns these values,
//! and the metadata layer (not the analyzer) decides if/when to mirror
//! them to file tags.

use std::path::Path;

use stratum_dsp::{AnalysisConfig, analyze_audio};
use sustain_domain::MusicalKey;

use crate::{AnalysisError, AnalysisOptions, decode::decode_capped};

/// Cap (seconds) on how much audio the BPM/key pass decodes. Long
/// enough that BPM/key estimates are reliable on full tracks, short
/// enough that the working set stays bounded. Matches the upstream
/// rhythmbox-to-pioneer-xdj-exporter figure.
const BPM_KEY_DECODE_CAP_SECONDS: u32 = 120;

/// Minimum number of samples the DSP engine needs to do anything
/// meaningful. Roughly one second at 44.1 kHz; below that the FFT
/// windows do not have enough material to analyze and `stratum-dsp`
/// either errors or returns garbage.
const MIN_SAMPLES_FOR_ANALYSIS: usize = 44_100;

pub(crate) struct DetectionResult {
    pub(crate) bpm: Option<f32>,
    pub(crate) key: Option<MusicalKey>,
}

pub(crate) fn detect(path: &Path, opts: AnalysisOptions) -> Result<DetectionResult, AnalysisError> {
    let decoded = decode_capped(path, BPM_KEY_DECODE_CAP_SECONDS)?;

    if decoded.samples.len() < MIN_SAMPLES_FOR_ANALYSIS {
        return Err(AnalysisError::TooShort {
            path: path.display().to_string(),
            samples: decoded.samples.len(),
        });
    }

    let raw = analyze_audio(
        &decoded.samples,
        decoded.sample_rate,
        AnalysisConfig::default(),
    )
    .map_err(|err| AnalysisError::DspFailed {
        path: path.display().to_string(),
        message: format!("{err:?}"),
    })?;

    drop(decoded);

    let bpm = if raw.bpm > 0.0 {
        Some(octave_normalize(raw.bpm, opts.min_bpm, opts.max_bpm))
    } else {
        None
    };

    let key = map_stratum_key(&raw.key.name());

    Ok(DetectionResult { bpm, key })
}

/// Pull `bpm` into `[min, max]` by repeated doubling/halving. Tracks
/// detected at a sub-bass periodicity (typically 60 BPM downbeat for
/// a 120 BPM tune) get doubled into the main band; double-time
/// detections (typically 200 BPM for a 100 BPM tune) get halved.
fn octave_normalize(mut bpm: f32, min: f32, max: f32) -> f32 {
    if !(min > 0.0 && max > min && bpm > 0.0) {
        return bpm;
    }
    while bpm < min && bpm * 2.0 <= max {
        bpm *= 2.0;
    }
    while bpm > max && bpm / 2.0 >= min {
        bpm /= 2.0;
    }
    bpm
}

/// Best-effort mapping from `stratum-dsp`'s key labels onto
/// [`MusicalKey`]. The DSP layer returns labels like "C major", "F#
/// minor", "Db major"; we lower-case and trim before matching. Returns
/// `None` if the label does not correspond to one of our 24 variants
/// (rare; only happens for non-standard names the engine might
/// produce). Enharmonic equivalents collapse onto Sustain's canonical
/// spelling (e.g. "D# major" → `DbMajor`'s neighbor `EbMajor`).
fn map_stratum_key(name: &str) -> Option<MusicalKey> {
    let lower = name.trim().to_ascii_lowercase();
    match lower.as_str() {
        "c major" | "cmaj" | "c" => Some(MusicalKey::CMajor),
        "c# major" | "db major" | "c#maj" | "dbmaj" | "c#" | "db" => Some(MusicalKey::DbMajor),
        "d major" | "dmaj" | "d" => Some(MusicalKey::DMajor),
        "d# major" | "eb major" | "d#maj" | "ebmaj" | "d#" | "eb" => Some(MusicalKey::EbMajor),
        "e major" | "emaj" | "e" => Some(MusicalKey::EMajor),
        "f major" | "fmaj" | "f" => Some(MusicalKey::FMajor),
        "f# major" | "gb major" | "f#maj" | "gbmaj" | "f#" | "gb" => Some(MusicalKey::GbMajor),
        "g major" | "gmaj" | "g" => Some(MusicalKey::GMajor),
        "g# major" | "ab major" | "g#maj" | "abmaj" | "g#" | "ab" => Some(MusicalKey::AbMajor),
        "a major" | "amaj" | "a" => Some(MusicalKey::AMajor),
        "a# major" | "bb major" | "a#maj" | "bbmaj" | "a#" | "bb" => Some(MusicalKey::BbMajor),
        "b major" | "bmaj" | "b" => Some(MusicalKey::BMajor),
        "c minor" | "cm" | "cmin" => Some(MusicalKey::CMinor),
        "c# minor" | "db minor" | "c#m" | "c#min" | "dbm" | "dbmin" => Some(MusicalKey::CsMinor),
        "d minor" | "dm" | "dmin" => Some(MusicalKey::DMinor),
        "d# minor" | "eb minor" | "d#m" | "d#min" | "ebm" | "ebmin" => Some(MusicalKey::EbMinor),
        "e minor" | "em" | "emin" => Some(MusicalKey::EMinor),
        "f minor" | "fm" | "fmin" => Some(MusicalKey::FMinor),
        "f# minor" | "gb minor" | "f#m" | "f#min" | "gbm" | "gbmin" => Some(MusicalKey::FsMinor),
        "g minor" | "gm" | "gmin" => Some(MusicalKey::GMinor),
        "g# minor" | "ab minor" | "g#m" | "g#min" | "abm" | "abmin" => Some(MusicalKey::AbMinor),
        "a minor" | "am" | "amin" => Some(MusicalKey::AMinor),
        "a# minor" | "bb minor" | "a#m" | "a#min" | "bbm" | "bbmin" => Some(MusicalKey::BbMinor),
        "b minor" | "bm" | "bmin" => Some(MusicalKey::BMinor),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{map_stratum_key, octave_normalize};
    use sustain_domain::MusicalKey;

    #[test]
    fn octave_normalize_doubles_subbass_tempo() {
        // 60 BPM with [70..170] window doubles to 120.
        assert!((octave_normalize(60.0, 70.0, 170.0) - 120.0).abs() < f32::EPSILON);
    }

    #[test]
    fn octave_normalize_halves_double_time() {
        // 200 BPM with [70..170] window halves to 100.
        assert!((octave_normalize(200.0, 70.0, 170.0) - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn octave_normalize_passes_in_range_unchanged() {
        assert!((octave_normalize(128.0, 70.0, 170.0) - 128.0).abs() < f32::EPSILON);
    }

    #[test]
    fn octave_normalize_keeps_value_when_no_octave_lands_in_range() {
        // A track tagged 40 BPM in a narrow [60..70] window cannot be
        // doubled (80 > 70) — return what we have rather than spin
        // forever.
        assert!((octave_normalize(40.0, 60.0, 70.0) - 40.0).abs() < f32::EPSILON);
    }

    #[test]
    fn stratum_key_labels_map_to_canonical_variants() {
        assert_eq!(map_stratum_key("C major"), Some(MusicalKey::CMajor));
        assert_eq!(map_stratum_key("c minor"), Some(MusicalKey::CMinor));
        assert_eq!(map_stratum_key("D# major"), Some(MusicalKey::EbMajor));
        assert_eq!(map_stratum_key("F# minor"), Some(MusicalKey::FsMinor));
        assert_eq!(map_stratum_key("Bb"), Some(MusicalKey::BbMajor));
    }

    #[test]
    fn stratum_key_labels_reject_unknown_input() {
        assert_eq!(map_stratum_key(""), None);
        assert_eq!(map_stratum_key("H major"), None);
    }
}
