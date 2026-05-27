// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Symphonia-based audio decoder. Two entry points: a full decode for
//! waveform generation and a capped decode for BPM/key detection,
//! sharing one inner routine. Both produce interleaved-collapsed mono
//! `f32` samples plus the original sample rate.
//!
//! The decoder is loaded by symphonia's default registry; only the
//! codecs enabled in `Cargo.toml`'s `symphonia` features are actually
//! linked. Unsupported codecs surface as `AnalysisError::DecoderError`.

use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::AnalysisError;

/// One contiguous block of mono `f32` samples plus the rate they were
/// captured at. Multi-channel sources are averaged down to mono so
/// downstream DSP only ever sees one stream.
pub(crate) struct DecodedAudio {
    pub(crate) samples: Vec<f32>,
    pub(crate) sample_rate: u32,
}

/// Decode the entire audio file to mono `f32`. Used by the waveform
/// pass so that preview/detail reflect the full track.
pub(crate) fn decode_full(path: &Path) -> Result<DecodedAudio, AnalysisError> {
    decode_inner(path, None)
}

/// Decode the audio file to mono `f32`, stopping after `max_seconds`
/// of audio. Used by the BPM/key pass to bound memory and DSP cost on
/// long tracks; 120 s is the upstream-tested figure.
pub(crate) fn decode_capped(path: &Path, max_seconds: u32) -> Result<DecodedAudio, AnalysisError> {
    decode_inner(path, Some(max_seconds))
}

fn decode_inner(path: &Path, max_seconds: Option<u32>) -> Result<DecodedAudio, AnalysisError> {
    let path_label = path.display().to_string();

    let file = std::fs::File::open(path).map_err(|source| AnalysisError::OpenFailed {
        path: path_label.clone(),
        source,
    })?;
    let media_source = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
        hint.with_extension(extension);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            media_source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|err| AnalysisError::DecoderError {
            path: path_label.clone(),
            message: format!("probe failed: {err}"),
        })?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| AnalysisError::NoAudioTrack {
            path: path_label.clone(),
        })?;

    let track_id = track.id;
    let sample_rate =
        track
            .codec_params
            .sample_rate
            .ok_or_else(|| AnalysisError::DecoderError {
                path: path_label.clone(),
                message: "no sample rate reported by decoder".to_string(),
            })?;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|err| AnalysisError::DecoderError {
            path: path_label.clone(),
            message: format!("decoder factory failed: {err}"),
        })?;

    let max_samples = max_seconds.map(|seconds| sample_rate as usize * seconds as usize);
    let mut samples: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            // Symphonia signals end-of-stream as an UnexpectedEof IoError,
            // which is the expected terminator — not a failure.
            Err(SymphoniaError::IoError(io)) if io.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(err) => {
                return Err(AnalysisError::DecoderError {
                    path: path_label.clone(),
                    message: format!("read packet: {err}"),
                });
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(buffer) => buffer,
            // A bad packet should not abort the whole decode; symphonia
            // recommends skipping and continuing. Real-world files
            // occasionally have a corrupted frame in the middle.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(err) => {
                return Err(AnalysisError::DecoderError {
                    path: path_label.clone(),
                    message: format!("decode packet: {err}"),
                });
            }
        };

        let spec = *decoded.spec();
        let capacity = decoded.capacity() as u64;
        let mut buffer = SampleBuffer::<f32>::new(capacity, spec);
        buffer.copy_interleaved_ref(decoded);
        let channel_count = spec.channels.count();

        if channel_count <= 1 {
            samples.extend_from_slice(buffer.samples());
        } else {
            // Average across channels for mono. Sum-then-divide on f32
            // is fine; if any one channel saturates we accept the
            // attenuation.
            let inverse = 1.0_f32 / channel_count as f32;
            for frame in buffer.samples().chunks_exact(channel_count) {
                let mono = frame.iter().sum::<f32>() * inverse;
                samples.push(mono);
            }
        }

        if let Some(cap) = max_samples
            && samples.len() >= cap
        {
            samples.truncate(cap);
            break;
        }
    }

    Ok(DecodedAudio {
        samples,
        sample_rate,
    })
}
