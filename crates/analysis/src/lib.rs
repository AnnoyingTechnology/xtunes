// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Audio analysis for Sustain — pure DSP, no I/O beyond reading the
//! audio file the caller hands us.
//!
//! [`Analyzer`] is the single public surface: a stateful,
//! capability-driven analyzer that owns a single track and caches the
//! intermediate work shared between bands (a mono decode and an STFT
//! of that decode). Callers pick exactly which bands they want by
//! calling [`Analyzer::bpm`], [`Analyzer::key`], and
//! [`Analyzer::waveform`] independently — a method that is never
//! called does zero work. This is the API the background scheduler
//! uses, so toggling off (say) waveform generation in Preferences
//! actually skips the waveform decode pass.
//!
//! The Analyzer reaches into `stratum_dsp::features::*` directly
//! (chroma extractor for the STFT, period::tempogram for BPM, key
//! detector for key) rather than going through
//! [`stratum_dsp::analyze_audio`]'s compute-everything orchestration.
//! That orchestration is intentionally bypassed: it always computes
//! every band, so calling it for "BPM only" still pays for chroma +
//! key detection. The trade-off is that we lose some of the
//! orchestration's confidence-boosting heuristics (multi-resolution
//! tempogram, onset consensus); accuracy on a 200-track validation
//! set was measured at ~85% with the plain tempogram path versus
//! ~92% with the full orchestration. The maintainer's call is that
//! 7 percentage points is the right price for an honest skip path.
//!
//! Persistence, paced scheduling, and "needs analysis" bookkeeping
//! live in `sustain-library-store` and `sustain-app-runtime`
//! respectively. This crate touches the filesystem only to read the
//! audio file.

mod bands;
mod decode;
mod waveform;

use std::cell::OnceCell;
use std::path::PathBuf;

// Re-exported from sustain_domain so callers can `use sustain_analysis::*`
// without also pulling sustain_domain into their imports for what is
// conceptually one cohesive surface. The canonical home for these types
// is the domain layer — the storage crate needs them but should not
// pull in symphonia / stratum-dsp.
pub use sustain_domain::{
    AcousticFeatures, BeatGrid, DETAIL_SEGMENTS_PER_SECOND, MusicalKey, PREVIEW_SEGMENT_COUNT,
    TrackAnalysis, WaveformSegment, WaveformSegments,
};

use decode::{DecodedAudio, decode_capped, decode_full};
use stratum_dsp::features::chroma::extractor::{
    compute_stft, extract_chroma_from_spectrogram_with_options,
};
use stratum_dsp::features::key::detect_key;
use stratum_dsp::features::key::templates::KeyTemplates;
use stratum_dsp::features::onset::spectral_flux::detect_spectral_flux_onsets;
use stratum_dsp::features::period::tempogram::estimate_bpm_tempogram;
use stratum_dsp::preprocessing::normalization::{
    NormalizationConfig, NormalizationMethod, normalize,
};

pub use waveform::WaveformTiers;

/// Monotonically-increasing identifier for the DSP algorithms in this
/// crate. Bumped centrally when a change to the band split, BPM/key
/// engine, or waveform encoding would invalidate previously-stored
/// `track_analysis` rows. The storage layer compares stored rows
/// against this value to decide whether a track should be re-queued
/// by the runtime scheduler — no migration code, just a version
/// bump that the scheduler walks past in the background.
///
/// Version 2: the BPM and key bands now route through
/// `stratum_dsp::features::*` directly via [`Analyzer`] instead of
/// `stratum_dsp::analyze_audio`'s compute-everything orchestration, so
/// previously-attempted rows must be re-attempted under the new
/// capability-driven pipeline.
///
/// Version 3: the audio pass gained the perceptual acoustic feature
/// set (loudness, onset density, timbral band ratios, low-band
/// variation, tonalness) Smart Shuffle consumes. Tracks attempted under
/// version 2 have no `track_acoustics` row, so they must be re-attempted.
pub const ANALYZER_VERSION: u32 = 3;

/// DSP tunables exposed to callers. Defaults reflect the values the
/// rhythmbox-to-pioneer-xdj-exporter author landed on after testing on
/// a large DJ-style library, with one Sustain-specific deviation:
/// no max-sample cap is applied to the waveform decode (the whole
/// track is decoded so the preview/detail reflect the full audio),
/// while BPM/key detection still observes the 120-second cap to keep
/// the working set bounded.
///
/// Capability gating (which bands to compute) is **not** in this
/// struct — that lives on the call site, which simply chooses which
/// [`Analyzer`] methods to invoke. Anything in here is DSP tuning
/// every band-method-call uses identically.
#[derive(Clone, Copy, Debug)]
pub struct AnalysisOptions {
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
            min_bpm: 70.0,
            max_bpm: 170.0,
        }
    }
}

/// Failure modes produced by the DSP pipeline. The per-band
/// [`Analyzer`] methods collapse failures to `None` so the caller can
/// decide whether a partial result still counts as an attempt; the
/// scheduler surfaces the typed error from its own file-open probe
/// before constructing the Analyzer, so this enum is what reaches
/// `record_analysis_attempt_failure`.
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

/// STFT frame size used for both BPM and key. Matches stratum-dsp's
/// own default (`AnalysisConfig::frame_size`); a smaller frame loses
/// frequency resolution that the key detector needs, a larger frame
/// blurs onsets the tempogram looks for.
const STFT_FRAME_SIZE: usize = 2048;

/// STFT hop size. Matches stratum-dsp's default — 75% overlap, which
/// the tempogram's novelty curve was tuned against.
const STFT_HOP_SIZE: usize = 512;

/// BPM resolution (in BPM) the tempogram searches at. 1.0 BPM matches
/// the upstream default and the resolution our UI displays at.
const TEMPOGRAM_BPM_RESOLUTION: f32 = 1.0;

/// Band-split crossovers (Hz) for the acoustic brightness ratios.
/// Matches the waveform's visual band split (`bands.rs`) so "low /
/// mid / high" means the same thing wherever it appears in the app.
const ACOUSTIC_LOW_MID_HZ: f32 = 250.0;
const ACOUSTIC_MID_HIGH_HZ: f32 = 4_000.0;

/// Short-term loudness window and hop, in seconds. EBU R128 defines
/// short-term loudness over a 3 s window; we slide it by 1 s and take
/// the max as the loud *boundary* a transition can hit (§7).
const SHORT_TERM_WINDOW_SECS: f32 = 3.0;
const SHORT_TERM_HOP_SECS: f32 = 1.0;

/// Percentile threshold for spectral-flux onset detection. 0.8 keeps
/// only the strongest flux peaks (matches the upstream example), so
/// the count reflects real rhythmic events rather than noise.
const ONSET_FLUX_PERCENTILE: f32 = 0.8;

/// Relative gate (LU below the loudest short-term window) below which
/// windows are dropped from the loudness-range estimate, à la the
/// EBU R128 relative gate.
const LOUDNESS_RANGE_GATE_LU: f32 = 20.0;

/// Capability-driven, stateful analyzer for one audio file. Cheap to
/// construct — the constructor does no I/O — and caches every
/// intermediate result the band methods share, so a caller that
/// requests both BPM and key only pays for one decode + one STFT.
///
/// The cache is per-band and lazy. Calling [`Self::bpm`] decodes the
/// audio (capped at 120 s), runs the STFT, and produces a BPM
/// estimate. A subsequent [`Self::key`] call reuses the same decode
/// and STFT — only the chroma extraction and key matching run
/// fresh. [`Self::waveform`] is independent: it uses the full
/// uncapped decode and never touches the STFT.
///
/// Failures inside any band collapse to `None`; the caller decides
/// how to record the attempt. Errors that prevent any band from
/// producing a result (file does not exist, decoder rejects the
/// container) surface as `None` from every method — semantically
/// "we tried, nothing came out". Callers that need a strict
/// open-fail signal (the scheduler does, so it can route the track
/// to `record_analysis_attempt_failure`) probe the file with
/// `std::fs::File::open` before constructing the analyzer.
pub struct Analyzer {
    path: PathBuf,
    options: AnalysisOptions,
    capped_audio: OnceCell<Option<DecodedAudio>>,
    full_audio: OnceCell<Option<DecodedAudio>>,
    stft: OnceCell<Option<Vec<Vec<f32>>>>,
}

impl Analyzer {
    /// Build an analyzer bound to `path` with the given DSP tunings.
    /// Performs no I/O — the audio file is opened lazily on the first
    /// band call.
    pub fn new(path: impl Into<PathBuf>, options: AnalysisOptions) -> Self {
        Self {
            path: path.into(),
            options,
            capped_audio: OnceCell::new(),
            full_audio: OnceCell::new(),
            stft: OnceCell::new(),
        }
    }

    /// Detected tempo in beats per minute, after octave normalization
    /// to `[options.min_bpm, options.max_bpm]`. Returns `None` if the
    /// file cannot be decoded, is too short for analysis, or the DSP
    /// engine cannot produce a confident estimate.
    pub fn bpm(&self) -> Option<f32> {
        let stft = self.stft_for_capped()?;
        let capped = self.capped()?;
        let estimate = estimate_bpm_tempogram(
            stft,
            capped.sample_rate,
            u32::try_from(STFT_HOP_SIZE).unwrap_or(u32::MAX),
            self.options.min_bpm,
            self.options.max_bpm,
            TEMPOGRAM_BPM_RESOLUTION,
        )
        .ok()?;
        if !(estimate.bpm > 0.0 && estimate.bpm.is_finite()) {
            return None;
        }
        Some(octave_normalize(
            estimate.bpm,
            self.options.min_bpm,
            self.options.max_bpm,
        ))
    }

    /// Detected musical key. Returns `None` if the file cannot be
    /// decoded, the chroma extraction produces no usable frames, or
    /// the key detector's best match does not correspond to one of
    /// Sustain's 24 canonical [`MusicalKey`] variants.
    pub fn key(&self) -> Option<MusicalKey> {
        let stft = self.stft_for_capped()?;
        let capped = self.capped()?;
        let chroma = extract_chroma_from_spectrogram_with_options(
            stft,
            capped.sample_rate,
            STFT_FRAME_SIZE,
            true,
            0.5,
        )
        .ok()?;
        if chroma.is_empty() {
            return None;
        }
        let templates = KeyTemplates::new();
        let detection = detect_key(&chroma, &templates).ok()?;
        let label = stratum_key_label(&detection.key);
        map_stratum_key(&label)
    }

    /// Both waveform tiers (preview + detail). Returns `None` if the
    /// file cannot be decoded; an empty-but-valid pair (no segments)
    /// is still returned as `Some(_)` so the caller can record the
    /// attempt and persist a zero-length BLOB rather than treating
    /// "silent track" as a failure.
    pub fn waveform(&self) -> Option<WaveformTiers> {
        let full = self.full()?;
        Some(waveform::build_tiers(&full.samples, full.sample_rate))
    }

    /// Perceptual acoustic features for Smart Shuffle — loudness
    /// (integrated, short-term max, range), onset density, timbral
    /// band ratios + low-band variation, and tonalness. Returns `None`
    /// if the file cannot be decoded, is too short, or carries no
    /// measurable loudness (effectively silent).
    ///
    /// Measured over the **whole track** via the full decode — not the
    /// 120 s BPM/key cap — because the loudness guard keys off the
    /// short-term *max*, and a loud finale after the cap must still
    /// count (§7). That full decode is the same one [`Self::waveform`]
    /// uses, shared through the analyzer's cache, so enabling waveform
    /// and acoustics together decodes the track only once. The STFT for
    /// the spectral features is computed here over the full samples
    /// (acoustics is its only consumer, so it is not cached).
    pub fn acoustics(&self) -> Option<AcousticFeatures> {
        let full = self.full()?;
        let sample_rate = full.sample_rate;
        let samples = &full.samples;
        if samples.len() < MIN_SAMPLES_FOR_ANALYSIS {
            return None;
        }
        let stft = compute_stft(samples, STFT_FRAME_SIZE, STFT_HOP_SIZE).ok()?;
        if stft.is_empty() {
            return None;
        }

        let integrated_lufs = measure_integrated_lufs(samples, sample_rate)?;
        let short_term = measure_short_term_lufs(samples, sample_rate);
        let short_term_lufs_max = short_term.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        // No measurable short-term window → effectively silent, no
        // useful loudness signal to offer the picker.
        if !short_term_lufs_max.is_finite() {
            return None;
        }
        let loudness_range_lu = loudness_range(&short_term);

        let duration_secs = samples.len() as f32 / sample_rate as f32;
        let onset_rate_hz = onset_density(&stft, duration_secs);

        let bands = band_energies(&stft, sample_rate);
        let tonalness = mean_tonalness(&stft);

        Some(AcousticFeatures {
            integrated_lufs,
            short_term_lufs_max,
            loudness_range_lu,
            onset_rate_hz,
            low_band_ratio: bands.low_ratio,
            mid_band_ratio: bands.mid_ratio,
            high_band_ratio: bands.high_ratio,
            low_band_variation: bands.low_variation,
            tonalness,
        })
    }

    fn capped(&self) -> Option<&DecodedAudio> {
        self.capped_audio
            .get_or_init(
                || match decode_capped(&self.path, BPM_KEY_DECODE_CAP_SECONDS) {
                    Ok(audio) if audio.samples.len() >= MIN_SAMPLES_FOR_ANALYSIS => Some(audio),
                    _ => None,
                },
            )
            .as_ref()
    }

    fn full(&self) -> Option<&DecodedAudio> {
        self.full_audio
            .get_or_init(|| decode_full(&self.path).ok())
            .as_ref()
    }

    fn stft_for_capped(&self) -> Option<&Vec<Vec<f32>>> {
        // Initialize the STFT cache. We can't call `capped()` inside
        // `get_or_init` because that would borrow `self` twice, so we
        // explicitly pull the decoded samples first and pass them in.
        if self.stft.get().is_none() {
            let computed = match self.capped() {
                Some(audio) => compute_stft(&audio.samples, STFT_FRAME_SIZE, STFT_HOP_SIZE).ok(),
                None => None,
            };
            // OnceCell::set returns Err if already initialised by a
            // concurrent call (impossible here since OnceCell is
            // !Sync), so just ignore that case.
            let _ = self.stft.set(computed);
        }
        self.stft.get().and_then(|cached| cached.as_ref())
    }
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

/// Normalization config that asks stratum-dsp for an ITU-R BS.1770-4
/// loudness *measurement*. We only consume the measured value, not the
/// gained audio, so the target/headroom are irrelevant — but they must
/// be set for the LUFS path to run.
fn loudness_config() -> NormalizationConfig {
    NormalizationConfig {
        target_loudness_lufs: -14.0,
        max_headroom_db: 1.0,
        method: NormalizationMethod::Loudness,
    }
}

/// Integrated (gated) loudness in LUFS via stratum-dsp's BS.1770-4
/// measurement. `normalize` applies gain in place, so we measure on a
/// scratch copy and keep only `measured_lufs` (the *input* loudness).
fn measure_integrated_lufs(samples: &[f32], sample_rate: u32) -> Option<f32> {
    let mut scratch = samples.to_vec();
    normalize(&mut scratch, loudness_config(), sample_rate as f32)
        .ok()
        .and_then(|metadata| metadata.measured_lufs)
        .filter(|value| value.is_finite())
}

/// Short-term loudness (LUFS) sampled over sliding ~3 s windows.
/// Measuring each window with stratum-dsp's own LUFS path yields a
/// faithful short-term curve without re-implementing K-weighting. A
/// clip shorter than one window is measured whole.
fn measure_short_term_lufs(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    let window = (SHORT_TERM_WINDOW_SECS * sample_rate as f32) as usize;
    let hop = ((SHORT_TERM_HOP_SECS * sample_rate as f32) as usize).max(1);
    if window == 0 || samples.len() < window {
        return measure_integrated_lufs(samples, sample_rate)
            .into_iter()
            .collect();
    }
    let mut values = Vec::new();
    let mut start = 0;
    while start + window <= samples.len() {
        if let Some(lufs) = measure_integrated_lufs(&samples[start..start + window], sample_rate) {
            values.push(lufs);
        }
        start += hop;
    }
    values
}

/// Loudness range (LU): the spread between quiet and loud passages,
/// approximated as the 95th minus 10th percentile of the short-term
/// loudness values after dropping windows more than
/// [`LOUDNESS_RANGE_GATE_LU`] below the loudest (the relative gate).
fn loudness_range(short_term: &[f32]) -> f32 {
    if short_term.len() < 2 {
        return 0.0;
    }
    let max = short_term.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let gate = max - LOUDNESS_RANGE_GATE_LU;
    let mut gated: Vec<f32> = short_term
        .iter()
        .copied()
        .filter(|value| *value >= gate)
        .collect();
    if gated.len() < 2 {
        return 0.0;
    }
    gated.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    (percentile(&gated, 0.95) - percentile(&gated, 0.10)).max(0.0)
}

/// Nearest-rank percentile of an already-sorted slice. `q` in `[0, 1]`.
fn percentile(sorted: &[f32], q: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() - 1) as f32 * q).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

/// Onset density in events per second from the spectral-flux detector.
fn onset_density(stft: &[Vec<f32>], duration_secs: f32) -> f32 {
    if duration_secs <= 0.0 {
        return 0.0;
    }
    let onsets = detect_spectral_flux_onsets(stft, ONSET_FLUX_PERCENTILE)
        .map(|onsets| onsets.len())
        .unwrap_or(0);
    onsets as f32 / duration_secs
}

/// Per-band energy fractions plus the low band's temporal variation.
struct BandEnergies {
    low_ratio: f32,
    mid_ratio: f32,
    high_ratio: f32,
    low_variation: f32,
}

/// Sum power per band (low / mid / high) across the whole STFT and
/// return each band's fraction of the total, plus the coefficient of
/// variation of the low band over time (the "kick-drum check": low =
/// steady dominant pulse, high = fluid/ambient or syncopated low end).
fn band_energies(stft: &[Vec<f32>], sample_rate: u32) -> BandEnergies {
    let bin_hz = sample_rate as f32 / STFT_FRAME_SIZE as f32;
    let (mut low_total, mut mid_total, mut high_total) = (0.0_f64, 0.0_f64, 0.0_f64);
    let mut low_fractions: Vec<f32> = Vec::with_capacity(stft.len());
    for frame in stft {
        let (mut low, mut mid, mut high) = (0.0_f64, 0.0_f64, 0.0_f64);
        for (bin, magnitude) in frame.iter().enumerate() {
            let freq = bin as f32 * bin_hz;
            let power = f64::from(*magnitude) * f64::from(*magnitude);
            if freq < ACOUSTIC_LOW_MID_HZ {
                low += power;
            } else if freq < ACOUSTIC_MID_HIGH_HZ {
                mid += power;
            } else {
                high += power;
            }
        }
        let frame_total = low + mid + high;
        if frame_total > 0.0 {
            low_fractions.push((low / frame_total) as f32);
        }
        low_total += low;
        mid_total += mid;
        high_total += high;
    }
    let grand = low_total + mid_total + high_total;
    let (low_ratio, mid_ratio, high_ratio) = if grand > 0.0 {
        (
            (low_total / grand) as f32,
            (mid_total / grand) as f32,
            (high_total / grand) as f32,
        )
    } else {
        (0.0, 0.0, 0.0)
    };
    BandEnergies {
        low_ratio,
        mid_ratio,
        high_ratio,
        low_variation: coefficient_of_variation(&low_fractions),
    }
}

/// Coefficient of variation (stddev / mean), clamped to a sane ceiling
/// so a near-zero mean can't blow the value up.
fn coefficient_of_variation(values: &[f32]) -> f32 {
    if values.len() < 2 {
        return 0.0;
    }
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    if mean <= 0.0 {
        return 0.0;
    }
    let variance =
        values.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / values.len() as f32;
    (variance.sqrt() / mean).min(4.0)
}

/// Mean tonalness across frames: `1 − spectral_flatness`. Spectral
/// flatness (geometric ÷ arithmetic mean of the power spectrum) is ≈1
/// for noise and ≈0 for a pure tone, so its complement rises with how
/// *pitched* the material is. Silent frames are skipped.
///
/// We use spectral flatness rather than the brief's suggested
/// chroma-energy ratio because it is self-contained (no dependency on
/// the key-detection chroma pass running) and is the textbook
/// pitched-vs-noisy measure; the brief explicitly left this open ("or
/// the key detector's own confidence").
fn mean_tonalness(stft: &[Vec<f32>]) -> f32 {
    let mut sum = 0.0_f64;
    let mut count = 0_u64;
    for frame in stft {
        if let Some(tonalness) = frame_tonalness(frame) {
            sum += f64::from(tonalness);
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        (sum / count as f64) as f32
    }
}

/// Tonalness of a single frame, or `None` if the frame is silent. The
/// DC bin is skipped (it carries no pitch information).
fn frame_tonalness(frame: &[f32]) -> Option<f32> {
    let powers: Vec<f64> = frame
        .iter()
        .skip(1)
        .map(|magnitude| f64::from(*magnitude) * f64::from(*magnitude))
        .collect();
    if powers.is_empty() {
        return None;
    }
    let arithmetic_mean = powers.iter().sum::<f64>() / powers.len() as f64;
    if arithmetic_mean <= 1e-12 {
        return None;
    }
    let log_mean = powers.iter().map(|p| (p + 1e-12).ln()).sum::<f64>() / powers.len() as f64;
    let geometric_mean = log_mean.exp();
    let flatness = (geometric_mean / arithmetic_mean).clamp(0.0, 1.0);
    Some((1.0 - flatness) as f32)
}

/// Render a `stratum_dsp::Key` as the lower-case label our mapper
/// expects. `stratum_dsp::Key` exposes a `name()` accessor on
/// `KeyType`, but its `Debug` is what other call sites use; we
/// format explicitly so the mapping table below stays canonical.
fn stratum_key_label(key: &stratum_dsp::Key) -> String {
    use stratum_dsp::Key;
    let (root, mode) = match key {
        Key::Major(idx) => (*idx, "major"),
        Key::Minor(idx) => (*idx, "minor"),
    };
    let name = match root % 12 {
        0 => "c",
        1 => "c#",
        2 => "d",
        3 => "d#",
        4 => "e",
        5 => "f",
        6 => "f#",
        7 => "g",
        8 => "g#",
        9 => "a",
        10 => "a#",
        _ => "b",
    };
    format!("{name} {mode}")
}

/// Best-effort mapping from a normalised `stratum-dsp` key label to a
/// [`MusicalKey`]. Returns `None` for labels that do not correspond
/// to one of our 24 variants (vanishingly rare — only happens for
/// non-standard names the engine might produce). Enharmonic equivalents
/// collapse onto Sustain's canonical spelling.
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
    use super::{AnalysisOptions, Analyzer, map_stratum_key, octave_normalize, stratum_key_label};
    use std::path::PathBuf;
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

    #[test]
    fn stratum_key_label_formats_match_mapper() {
        // Every label `stratum_key_label` produces must round-trip
        // through `map_stratum_key` to a `MusicalKey`. Guard against
        // a typo introducing a label the mapper does not know.
        for root in 0_u32..12 {
            for mode in [stratum_dsp::Key::Major(root), stratum_dsp::Key::Minor(root)] {
                let label = stratum_key_label(&mode);
                assert!(
                    map_stratum_key(&label).is_some(),
                    "label {label:?} from stratum_key_label has no mapper entry"
                );
            }
        }
    }

    #[test]
    fn analyzer_returns_none_for_missing_file() {
        // Capability-gating boils down to "the method that was not
        // called did no work". Here we call only `bpm()` on a path
        // that does not exist; the band-specific result is None and
        // the unrequested bands are never touched (a successful
        // `key()`/`waveform()` call would be impossible too, since
        // the file does not exist — but the point of this test is
        // that the absence of an explicit Result still surfaces the
        // failure cleanly).
        let analyzer = Analyzer::new(
            PathBuf::from("/does/not/exist/sustain_tests_missing.flac"),
            AnalysisOptions::default(),
        );
        assert_eq!(analyzer.bpm(), None);
        assert_eq!(analyzer.key(), None);
        assert!(analyzer.waveform().is_none());
    }

    #[test]
    fn analyzer_constructed_lazily_without_io() {
        // Constructing an Analyzer must not touch the filesystem.
        // The path here is intentionally invalid — if the
        // constructor opened the file, this test would not even
        // compile a working analyzer for the assertion below.
        let _analyzer = Analyzer::new(
            PathBuf::from("/this/path/does/not/exist.flac"),
            AnalysisOptions::default(),
        );
        // Reaching this line proves no I/O happened in `new`; the
        // call sites lower in the method chain (bpm/key/waveform)
        // are where the actual `File::open` lives.
    }
}

#[cfg(test)]
mod acoustic_tests {
    use super::{
        STFT_FRAME_SIZE, band_energies, coefficient_of_variation, frame_tonalness, loudness_range,
        mean_tonalness, measure_integrated_lufs, percentile,
    };

    const SAMPLE_RATE: u32 = 44_100;

    /// A magnitude-spectrum frame (`frame_size/2 + 1` bins) with all
    /// energy in a single bin. Bin `b` maps to `b * SR / FRAME_SIZE` Hz.
    fn frame_with_energy_at_bin(bin: usize, magnitude: f32) -> Vec<f32> {
        let mut frame = vec![0.0_f32; STFT_FRAME_SIZE / 2 + 1];
        frame[bin] = magnitude;
        frame
    }

    fn sine(freq: f32, secs: f32, amplitude: f32) -> Vec<f32> {
        let count = (secs * SAMPLE_RATE as f32) as usize;
        (0..count)
            .map(|i| {
                amplitude * (std::f32::consts::TAU * freq * i as f32 / SAMPLE_RATE as f32).sin()
            })
            .collect()
    }

    #[test]
    fn band_ratios_follow_the_dominant_frequency() {
        // Bin 5 ≈ 108 Hz (low band); bin 500 ≈ 10.8 kHz (high band).
        let low_heavy = vec![frame_with_energy_at_bin(5, 1.0); 4];
        let bands = band_energies(&low_heavy, SAMPLE_RATE);
        assert!(
            bands.low_ratio > 0.99,
            "low tone should land in the low band, got low={} mid={} high={}",
            bands.low_ratio,
            bands.mid_ratio,
            bands.high_ratio
        );

        let high_heavy = vec![frame_with_energy_at_bin(500, 1.0); 4];
        let bands = band_energies(&high_heavy, SAMPLE_RATE);
        assert!(bands.high_ratio > 0.99, "high tone → high band");
    }

    #[test]
    fn steady_low_band_has_low_variation_jittery_has_high() {
        // Identical low-band frames → zero variation.
        let steady = vec![frame_with_energy_at_bin(5, 1.0); 8];
        assert!(band_energies(&steady, SAMPLE_RATE).low_variation < 1e-3);

        // Alternating low-only / high-only frames → the low-band
        // fraction swings between ~1 and ~0, a high coefficient of
        // variation.
        let mut jittery = Vec::new();
        for i in 0..8 {
            jittery.push(frame_with_energy_at_bin(
                if i % 2 == 0 { 5 } else { 500 },
                1.0,
            ));
        }
        assert!(band_energies(&jittery, SAMPLE_RATE).low_variation > 0.5);
    }

    #[test]
    fn tonalness_is_high_for_a_peak_and_low_for_a_flat_spectrum() {
        // One dominant bin → very peaky → tonalness near 1.
        let peaky = frame_with_energy_at_bin(40, 10.0);
        let peaky_tonalness = frame_tonalness(&peaky).expect("non-silent");
        assert!(peaky_tonalness > 0.9, "peak tonalness {peaky_tonalness}");

        // Flat spectrum (every bin equal) → flatness ≈ 1 → tonalness ≈ 0.
        let flat = vec![1.0_f32; STFT_FRAME_SIZE / 2 + 1];
        let flat_tonalness = frame_tonalness(&flat).expect("non-silent");
        assert!(flat_tonalness < 0.05, "flat tonalness {flat_tonalness}");

        // A silent frame contributes nothing.
        assert_eq!(frame_tonalness(&[0.0_f32; 8]), None);

        // Mean across frames sits between the two.
        let mixed = vec![peaky.clone(), flat.clone()];
        let mean = mean_tonalness(&mixed);
        assert!(mean > flat_tonalness && mean < peaky_tonalness);
    }

    #[test]
    fn percentile_and_loudness_range() {
        let sorted = [-20.0, -18.0, -16.0, -14.0, -12.0];
        assert_eq!(percentile(&sorted, 0.0), -20.0);
        assert_eq!(percentile(&sorted, 1.0), -12.0);

        // Range over a spread set is positive; a flat set is ~0.
        let varied = [-30.0, -22.0, -18.0, -10.0, -8.0];
        assert!(loudness_range(&varied) > 0.0);
        let flat = [-14.0, -14.0, -14.0, -14.0];
        assert_eq!(loudness_range(&flat), 0.0);
        assert_eq!(loudness_range(&[-14.0]), 0.0);
    }

    #[test]
    fn coefficient_of_variation_basics() {
        assert_eq!(coefficient_of_variation(&[]), 0.0);
        assert_eq!(coefficient_of_variation(&[0.5]), 0.0);
        assert_eq!(coefficient_of_variation(&[0.3, 0.3, 0.3]), 0.0);
        assert!(coefficient_of_variation(&[0.1, 0.9]) > 0.0);
    }

    #[test]
    fn louder_audio_measures_higher_lufs() {
        // Same 440 Hz tone at two amplitudes 14 dB apart. The louder
        // one must report a higher integrated LUFS (exercises the
        // stratum-dsp BS.1770-4 path end to end).
        let loud = sine(440.0, 2.0, 0.5);
        let quiet = sine(440.0, 2.0, 0.1);
        let loud_lufs = measure_integrated_lufs(&loud, SAMPLE_RATE).expect("loud lufs");
        let quiet_lufs = measure_integrated_lufs(&quiet, SAMPLE_RATE).expect("quiet lufs");
        assert!(
            loud_lufs > quiet_lufs + 3.0,
            "louder tone should measure clearly higher: {loud_lufs} vs {quiet_lufs}"
        );
    }
}
