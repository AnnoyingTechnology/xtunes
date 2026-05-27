// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Three-band IIR splitter used to derive per-segment low/mid/high
//! energies for the waveform. Built from three independent biquad
//! filters (low-pass, band-pass, high-pass) rather than a Linkwitz-
//! Riley crossover, because the waveform's purpose is *visual* band
//! energy display — overlap between adjacent bands at the crossover
//! points is acceptable, and a crossover would cost us a second
//! cascaded biquad per band without changing the visual result.
//!
//! The splitter is single-sample and stateful: the waveform module
//! drives one decoded sample at a time and the splitter returns the
//! three filtered values in lockstep. Memory cost is constant — six
//! `f32` of filter state plus three filter coefficient sets per track.

use biquad::{Biquad, Coefficients, DirectForm1, Q_BUTTERWORTH_F32, ToHertz, Type as FilterType};

/// Crossover between low and mid bands, in Hz. Below this the signal
/// is mostly sub-bass / kick-drum body; matches the lower edge of the
/// "mid" band on most spectrum analyzers and the rendering convention
/// used by Serato/Rekordbox-style colored waveforms.
const LOW_MID_CROSSOVER_HZ: f32 = 250.0;
/// Crossover between mid and high bands, in Hz. Above this the signal
/// is dominated by cymbals, sibilance, and air; below this is vocals,
/// snares, and most harmonic content.
const MID_HIGH_CROSSOVER_HZ: f32 = 4_000.0;
/// Center frequency for the band-pass biquad covering the mid band.
/// Geometric mean of the two crossovers (≈ 1 kHz) so the response is
/// symmetric in log-frequency space.
const MID_CENTER_HZ: f32 = 1_000.0;

pub(crate) struct ThreeBandSplitter {
    low: DirectForm1<f32>,
    mid: DirectForm1<f32>,
    high: DirectForm1<f32>,
}

impl ThreeBandSplitter {
    /// Build a splitter for audio captured at `sample_rate` Hz.
    /// Returns `None` if the sample rate is too low for the chosen
    /// crossovers (Nyquist below 4 kHz — i.e. sample rate below 8 kHz)
    /// since the high band cannot exist there. Callers should treat
    /// `None` as "skip the band split for this track" rather than
    /// fail the whole analysis.
    pub(crate) fn new(sample_rate: u32) -> Option<Self> {
        if sample_rate < 8_000 {
            return None;
        }
        let rate = (sample_rate as f32).hz();
        let low = Coefficients::<f32>::from_params(
            FilterType::LowPass,
            rate,
            LOW_MID_CROSSOVER_HZ.hz(),
            Q_BUTTERWORTH_F32,
        )
        .ok()?;
        let mid = Coefficients::<f32>::from_params(
            FilterType::BandPass,
            rate,
            MID_CENTER_HZ.hz(),
            Q_BUTTERWORTH_F32,
        )
        .ok()?;
        let high = Coefficients::<f32>::from_params(
            FilterType::HighPass,
            rate,
            MID_HIGH_CROSSOVER_HZ.hz(),
            Q_BUTTERWORTH_F32,
        )
        .ok()?;
        Some(Self {
            low: DirectForm1::<f32>::new(low),
            mid: DirectForm1::<f32>::new(mid),
            high: DirectForm1::<f32>::new(high),
        })
    }

    /// Push one mono sample through the splitter; returns
    /// `(low, mid, high)` filtered values for that sample.
    #[inline]
    pub(crate) fn process(&mut self, sample: f32) -> (f32, f32, f32) {
        (
            self.low.run(sample),
            self.mid.run(sample),
            self.high.run(sample),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::ThreeBandSplitter;
    use std::f32::consts::TAU;

    const SAMPLE_RATE: u32 = 44_100;

    /// Generate a steady sine at `freq` Hz for `duration_secs` seconds
    /// at amplitude 1.0. Deterministic — no audio files in repo.
    fn sine(freq: f32, duration_secs: f32) -> Vec<f32> {
        let count = (duration_secs * SAMPLE_RATE as f32) as usize;
        (0..count)
            .map(|i| (TAU * freq * i as f32 / SAMPLE_RATE as f32).sin())
            .collect()
    }

    /// RMS of a slice, ignoring the first half (to give the IIR
    /// filters time to reach steady state).
    fn settled_rms(samples: &[f32]) -> f32 {
        let start = samples.len() / 2;
        let tail = &samples[start..];
        let sum_sq: f64 = tail.iter().map(|x| (*x as f64).powi(2)).sum();
        (sum_sq / tail.len() as f64).sqrt() as f32
    }

    #[test]
    fn splitter_rejects_too_low_sample_rate() {
        assert!(ThreeBandSplitter::new(4_000).is_none());
        assert!(ThreeBandSplitter::new(48_000).is_some());
    }

    #[test]
    fn low_tone_concentrates_in_low_band() {
        let mut splitter = ThreeBandSplitter::new(SAMPLE_RATE).expect("rate");
        let input = sine(80.0, 2.0);
        let mut low = Vec::with_capacity(input.len());
        let mut high = Vec::with_capacity(input.len());
        for &sample in &input {
            let (l, _m, h) = splitter.process(sample);
            low.push(l);
            high.push(h);
        }
        let low_rms = settled_rms(&low);
        let high_rms = settled_rms(&high);
        assert!(
            low_rms > 0.5,
            "80 Hz should pass the low-band low-pass; got {low_rms}"
        );
        assert!(
            high_rms < 0.05,
            "80 Hz should be heavily attenuated in the high band; got {high_rms}"
        );
    }

    #[test]
    fn high_tone_concentrates_in_high_band() {
        let mut splitter = ThreeBandSplitter::new(SAMPLE_RATE).expect("rate");
        let input = sine(8_000.0, 2.0);
        let mut low = Vec::with_capacity(input.len());
        let mut high = Vec::with_capacity(input.len());
        for &sample in &input {
            let (l, _m, h) = splitter.process(sample);
            low.push(l);
            high.push(h);
        }
        let low_rms = settled_rms(&low);
        let high_rms = settled_rms(&high);
        assert!(
            high_rms > 0.5,
            "8 kHz should pass the high-band high-pass; got {high_rms}"
        );
        assert!(
            low_rms < 0.05,
            "8 kHz should be heavily attenuated in the low band; got {low_rms}"
        );
    }
}
