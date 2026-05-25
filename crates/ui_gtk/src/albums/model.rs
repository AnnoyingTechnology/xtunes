// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::collections::BTreeMap;
use std::path::PathBuf;

use sustain_app_runtime::{Track, TrackId, TrackMetadata};

/// Stable identity for an album, derived from the normalized
/// (artist, title) pair used to bucket tracks. Equality matches the
/// bucketing logic, so two `AlbumViewModel`s produced from different
/// `group_albums` calls share a key iff they cover the same tracks.
#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub(super) struct AlbumKey {
    artist: String,
    title: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AlbumViewModel {
    pub(super) key: AlbumKey,
    pub(super) title: String,
    pub(super) artist: String,
    pub(super) year: Option<i32>,
    pub(super) tracks: Vec<AlbumTrackViewModel>,
    /// Audio file the artwork loader should read to extract this album's
    /// cover, expressed as the track's `TrackLocation::path()` (relative
    /// when the library has a root, absolute when imported as such). The
    /// view resolves it against the current library root before queueing.
    /// `None` when every track is missing on disk and the album is being
    /// rendered purely from cached metadata.
    pub(super) representative_track_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AlbumTrackViewModel {
    pub(super) id: TrackId,
    pub(super) file_path: PathBuf,
    pub(super) title: String,
    pub(super) disc_number: Option<u32>,
    pub(super) track_number: Option<u32>,
    pub(super) duration_seconds: u64,
    pub(super) is_missing: bool,
}

#[derive(Clone, Debug)]
struct AlbumBucket {
    key: AlbumKey,
    title: String,
    artist: String,
    year: Option<i32>,
    tracks: Vec<AlbumTrackViewModel>,
}

const UNKNOWN_ALBUM: &str = "Unknown Album";
const UNKNOWN_ARTIST: &str = "Unknown Artist";

pub(super) fn group_albums(tracks: &[Track]) -> Vec<AlbumViewModel> {
    let mut albums = BTreeMap::<AlbumKey, AlbumBucket>::new();

    for track in tracks {
        let title = album_title(&track.metadata);
        let artist = album_artist(&track.metadata);
        let key = AlbumKey {
            artist: normalize_album_key(&artist),
            title: normalize_album_key(&title),
        };
        let bucket = albums
            .entry(key.clone())
            .or_insert_with(|| AlbumBucket {
                key,
                title,
                artist,
                year: track.metadata.year,
                tracks: Vec::new(),
            });
        if bucket.year.is_none() {
            bucket.year = track.metadata.year;
        }
        bucket.tracks.push(album_track(track));
    }

    albums
        .into_values()
        .map(|mut bucket| {
            bucket.tracks.sort_by(compare_album_tracks);
            let representative_track_path = bucket
                .tracks
                .iter()
                .find(|track| !track.is_missing)
                .or_else(|| bucket.tracks.first())
                .map(|track| track.file_path.clone());
            AlbumViewModel {
                key: bucket.key,
                title: bucket.title,
                artist: bucket.artist,
                year: bucket.year,
                tracks: bucket.tracks,
                representative_track_path,
            }
        })
        .collect()
}

pub(super) fn album_subtitle(album: &AlbumViewModel) -> String {
    match album.year {
        Some(year) => format!("{} ({year})", album.artist),
        None => album.artist.clone(),
    }
}

pub(super) fn track_number_text(track: &AlbumTrackViewModel) -> String {
    match (track.disc_number, track.track_number) {
        (Some(disc), Some(number)) if disc > 1 => format!("{disc}-{number}"),
        (_, Some(number)) => number.to_string(),
        _ => String::new(),
    }
}

pub(super) fn duration_text(duration_seconds: u64) -> String {
    let hours = duration_seconds / 3_600;
    let minutes = duration_seconds % 3_600 / 60;
    let seconds = duration_seconds % 60;

    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn album_track(track: &Track) -> AlbumTrackViewModel {
    AlbumTrackViewModel {
        id: track.id,
        file_path: track.location.path().to_path_buf(),
        title: track_title(track),
        disc_number: track.metadata.disc_number,
        track_number: track.metadata.track_number,
        duration_seconds: track
            .metadata
            .duration
            .map(|duration| duration.as_secs())
            .unwrap_or_default(),
        is_missing: track.location.is_missing(),
    }
}

fn compare_album_tracks(
    left: &AlbumTrackViewModel,
    right: &AlbumTrackViewModel,
) -> std::cmp::Ordering {
    left.disc_number
        .unwrap_or(0)
        .cmp(&right.disc_number.unwrap_or(0))
        .then_with(|| {
            left.track_number
                .unwrap_or(u32::MAX)
                .cmp(&right.track_number.unwrap_or(u32::MAX))
        })
        .then_with(|| normalize_album_key(&left.title).cmp(&normalize_album_key(&right.title)))
        .then_with(|| left.id.cmp(&right.id))
}

fn album_title(metadata: &TrackMetadata) -> String {
    non_empty_text(&metadata.album).unwrap_or_else(|| UNKNOWN_ALBUM.to_owned())
}

fn album_artist(metadata: &TrackMetadata) -> String {
    non_empty_text(&metadata.album_artist)
        .or_else(|| non_empty_text(&metadata.artist))
        .unwrap_or_else(|| UNKNOWN_ARTIST.to_owned())
}

fn track_title(track: &Track) -> String {
    non_empty_text(&track.metadata.title)
        .or_else(|| {
            track
                .location
                .path()
                .file_stem()
                .and_then(|file_stem| file_stem.to_str())
                .map(str::trim)
                .filter(|file_stem| !file_stem.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_default()
}

fn non_empty_text(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_album_key(value: &str) -> String {
    value.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use sustain_app_runtime::{
        PlayStatistics, Rating, Track, TrackId, TrackLocation, TrackMetadata, TrackRelativePath,
    };

    use super::{AlbumTrackViewModel, duration_text, group_albums, track_number_text};

    #[test]
    fn groups_tracks_by_album_artist_and_album_title() {
        let tracks = vec![
            track(1, "b.flac", "Album", "Artist", Some(2), Some(200)),
            track(2, "a.flac", "Album", "Artist", Some(1), Some(200)),
            track(3, "other.flac", "Other", "Artist", Some(1), None),
        ];

        let albums = group_albums(&tracks);

        assert_eq!(albums.len(), 2);
        assert_eq!(albums[0].title, "Album");
        assert_eq!(albums[0].artist, "Artist");
        assert_eq!(albums[0].year, Some(200));
        assert_eq!(
            albums[0]
                .tracks
                .iter()
                .map(|track| track.id.get())
                .collect::<Vec<_>>(),
            vec![2, 1]
        );
        assert_eq!(albums[1].title, "Other");
    }

    #[test]
    fn representative_track_prefers_first_non_missing() {
        let tracks = vec![
            missing_track(1, "missing.flac", "Album", "Artist"),
            track(2, "present.flac", "Album", "Artist", Some(2), Some(200)),
        ];

        let albums = group_albums(&tracks);

        assert_eq!(albums.len(), 1);
        assert_eq!(
            albums[0].representative_track_path.as_deref(),
            Some(std::path::Path::new("present.flac"))
        );
    }

    #[test]
    fn representative_track_falls_back_to_first_when_all_missing() {
        let tracks = vec![
            missing_track(1, "a.flac", "Album", "Artist"),
            missing_track(2, "b.flac", "Album", "Artist"),
        ];

        let albums = group_albums(&tracks);

        assert_eq!(albums.len(), 1);
        // Track ordering inside the bucket runs through `compare_album_tracks`,
        // which here ties on disc/track and falls back to title comparison; the
        // important contract is just "some path, not None".
        assert!(albums[0].representative_track_path.is_some());
    }

    #[test]
    fn uses_unknown_album_and_artist_when_metadata_is_missing() {
        let albums = group_albums(&[Track {
            id: track_id(1),
            location: TrackLocation::available(relative_path("track.flac")),
            content_hash: None,
            metadata: TrackMetadata::default(),
            rating: Rating::unrated(),
            statistics: PlayStatistics::default(),
        }]);

        assert_eq!(albums[0].title, "Unknown Album");
        assert_eq!(albums[0].artist, "Unknown Artist");
    }

    #[test]
    fn track_number_text_includes_disc_when_needed() {
        let single_disc = album_track(Some(1), Some(7));
        let multi_disc = album_track(Some(2), Some(7));

        assert_eq!(track_number_text(&single_disc), "7");
        assert_eq!(track_number_text(&multi_disc), "2-7");
    }

    #[test]
    fn duration_text_uses_hours_when_needed() {
        assert_eq!(duration_text(245), "4:05");
        assert_eq!(duration_text(3_904), "1:05:04");
    }

    fn track(
        id: i64,
        path: &str,
        album: &str,
        artist: &str,
        track_number: Option<u32>,
        year: Option<i32>,
    ) -> Track {
        Track {
            id: track_id(id),
            location: TrackLocation::available(relative_path(path)),
            content_hash: None,
            metadata: TrackMetadata {
                title: Some(path.trim_end_matches(".flac").to_owned()),
                artist: Some(artist.to_owned()),
                album: Some(album.to_owned()),
                year,
                track_number,
                duration: Some(Duration::from_secs(180)),
                ..TrackMetadata::default()
            },
            rating: Rating::unrated(),
            statistics: PlayStatistics::default(),
        }
    }

    fn missing_track(id: i64, path: &str, album: &str, artist: &str) -> Track {
        let mut track = track(id, path, album, artist, None, None);
        track.location = TrackLocation::missing(relative_path(path));
        track
    }

    fn album_track(disc_number: Option<u32>, track_number: Option<u32>) -> AlbumTrackViewModel {
        AlbumTrackViewModel {
            id: track_id(1),
            file_path: PathBuf::from("track.flac"),
            title: "Track".to_owned(),
            disc_number,
            track_number,
            duration_seconds: 180,
            is_missing: false,
        }
    }

    fn track_id(value: i64) -> TrackId {
        match TrackId::new(value) {
            Some(track_id) => track_id,
            None => unreachable!("test helper only constructs positive ids"),
        }
    }

    fn relative_path(path: &str) -> TrackRelativePath {
        TrackRelativePath::new(PathBuf::from(path)).expect("test path is relative")
    }
}
