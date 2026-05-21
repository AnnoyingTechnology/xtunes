use std::collections::BTreeMap;
use std::path::PathBuf;

use xtunes_app_runtime::{Track, TrackId, TrackMetadata};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AlbumViewModel {
    pub(super) title: String,
    pub(super) artist: String,
    pub(super) year: Option<i32>,
    pub(super) tracks: Vec<AlbumTrackViewModel>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AlbumTrackViewModel {
    pub(super) id: TrackId,
    pub(super) relative_path: PathBuf,
    pub(super) title: String,
    pub(super) disc_number: Option<u32>,
    pub(super) track_number: Option<u32>,
    pub(super) duration_seconds: u64,
    pub(super) is_missing: bool,
}

#[derive(Clone, Debug)]
struct AlbumBucket {
    title: String,
    artist: String,
    year: Option<i32>,
    tracks: Vec<AlbumTrackViewModel>,
}

const UNKNOWN_ALBUM: &str = "Unknown Album";
const UNKNOWN_ARTIST: &str = "Unknown Artist";

pub(super) fn group_albums(tracks: &[Track]) -> Vec<AlbumViewModel> {
    let mut albums = BTreeMap::<(String, String), AlbumBucket>::new();

    for track in tracks {
        let title = album_title(&track.metadata);
        let artist = album_artist(&track.metadata);
        let key = (normalize_album_key(&artist), normalize_album_key(&title));
        let bucket = albums.entry(key).or_insert_with(|| AlbumBucket {
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
            AlbumViewModel {
                title: bucket.title,
                artist: bucket.artist,
                year: bucket.year,
                tracks: bucket.tracks,
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
        relative_path: track.location.relative_path.to_path_buf(),
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
                .relative_path
                .as_path()
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

    use xtunes_app_runtime::{
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
    fn uses_unknown_album_and_artist_when_metadata_is_missing() {
        let albums = group_albums(&[Track {
            id: track_id(1),
            location: TrackLocation::available(relative_path("track.flac")),
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

    fn album_track(disc_number: Option<u32>, track_number: Option<u32>) -> AlbumTrackViewModel {
        AlbumTrackViewModel {
            id: track_id(1),
            relative_path: PathBuf::from("track.flac"),
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
