// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::path::Path;

use sustain_app_runtime::{Track, TrackId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AudioFileType {
    Flac,
    M4a,
    Mp4,
    Mp3,
    Ogg,
    Unknown,
}

impl AudioFileType {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Flac => "FLAC",
            Self::M4a => "M4A",
            Self::Mp4 => "MP4",
            Self::Mp3 => "MP3",
            Self::Ogg => "OGG",
            Self::Unknown => "",
        }
    }

    fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("flac") => Self::Flac,
            Some("m4a") | Some("m4b") => Self::M4a,
            Some("mp4") => Self::Mp4,
            Some("mp3") => Self::Mp3,
            Some("ogg") | Some("oga") | Some("opus") => Self::Ogg,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TrackTableRow {
    pub(crate) track_id: Option<TrackId>,
    pub(crate) track_name: String,
    pub(crate) artist: String,
    pub(crate) album: String,
    pub(crate) genre: String,
    pub(crate) year: Option<i32>,
    pub(crate) bpm: Option<u16>,
    pub(crate) bitrate_kbps: Option<u32>,
    pub(super) file_type: AudioFileType,
    pub(crate) duration_seconds: u64,
    pub(crate) rating: u8,
    pub(crate) plays: u64,
    pub(crate) last_played: Option<String>,
    pub(crate) date_added: String,
    pub(crate) track_number: Option<u32>,
    pub(crate) file_size_bytes: u64,
    pub(crate) is_missing: bool,
}

impl TrackTableRow {
    pub(crate) fn from_track(track: &Track, library_root: Option<&Path>) -> Self {
        let absolute_path =
            library_root.map(|library_root| track.location.absolute_path(library_root));
        let file_metadata = absolute_path
            .as_ref()
            .and_then(|path| std::fs::metadata(path).ok());
        let is_missing = track.location.is_missing() || file_metadata.is_none();

        Self {
            track_id: Some(track.id),
            track_name: non_empty_text(&track.metadata.title)
                .or_else(|| file_stem_text(track.location.path()))
                .unwrap_or_default(),
            artist: non_empty_text(&track.metadata.artist).unwrap_or_default(),
            album: non_empty_text(&track.metadata.album).unwrap_or_default(),
            genre: non_empty_text(&track.metadata.genre).unwrap_or_default(),
            year: track.metadata.year,
            bpm: None,
            bitrate_kbps: track.metadata.bitrate_kbps,
            file_type: AudioFileType::from_path(track.location.path()),
            duration_seconds: track
                .metadata
                .duration
                .map(|duration| duration.as_secs())
                .unwrap_or_default(),
            rating: track.rating.stars(),
            plays: track.statistics.play_count,
            last_played: None,
            date_added: String::new(),
            track_number: track.metadata.track_number,
            file_size_bytes: file_metadata
                .map(|metadata| metadata.len())
                .unwrap_or_default(),
            is_missing,
        }
    }
}

fn non_empty_text(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn file_stem_text(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|file_stem| file_stem.to_str())
        .map(str::trim)
        .filter(|file_stem| !file_stem.is_empty())
        .map(ToOwned::to_owned)
}
