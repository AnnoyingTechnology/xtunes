#![forbid(unsafe_code)]

use std::cmp::Ordering;

pub use xtunes_domain::{LibraryQuery, SortDirection, Track, TrackSort, TrackSortColumn};

pub type SearchResult<T> = Result<T, SearchError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchError {
    UnsupportedSortColumn(TrackSortColumn),
}

pub fn filter_tracks_by_search_text(tracks: &[Track], search_text: &str) -> Vec<Track> {
    let normalized_search = normalize(search_text);
    if normalized_search.is_empty() {
        return tracks.to_vec();
    }

    tracks
        .iter()
        .filter(|track| track_matches_search_text(track, &normalized_search))
        .cloned()
        .collect()
}

pub fn track_matches_search_text(track: &Track, search_text: &str) -> bool {
    let normalized_search = normalize(search_text);
    if normalized_search.is_empty() {
        return true;
    }

    searchable_fields(track)
        .iter()
        .any(|field| normalize(field).contains(&normalized_search))
}

pub fn sort_tracks(mut tracks: Vec<Track>, sort: TrackSort) -> SearchResult<Vec<Track>> {
    if sort.column == TrackSortColumn::DateAdded {
        return Err(SearchError::UnsupportedSortColumn(
            TrackSortColumn::DateAdded,
        ));
    }

    tracks.sort_by(|left, right| compare_tracks(left, right, sort));
    Ok(tracks)
}

fn compare_tracks(left: &Track, right: &Track, sort: TrackSort) -> Ordering {
    let ordering = match sort.column {
        TrackSortColumn::Title => {
            compare_optional_text(&left.metadata.title, &right.metadata.title)
        }
        TrackSortColumn::Artist => {
            compare_optional_text(&left.metadata.artist, &right.metadata.artist)
        }
        TrackSortColumn::Album => {
            compare_optional_text(&left.metadata.album, &right.metadata.album)
        }
        TrackSortColumn::Genre => {
            compare_optional_text(&left.metadata.genre, &right.metadata.genre)
        }
        TrackSortColumn::Rating => left.rating.cmp(&right.rating),
        TrackSortColumn::PlayCount => left.statistics.play_count.cmp(&right.statistics.play_count),
        TrackSortColumn::LastPlayed => left
            .statistics
            .last_played_at
            .cmp(&right.statistics.last_played_at),
        TrackSortColumn::Duration => left.metadata.duration.cmp(&right.metadata.duration),
        TrackSortColumn::DateAdded => Ordering::Equal,
    };

    let ordering = match sort.direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    };

    ordering.then_with(|| left.id.cmp(&right.id))
}

fn compare_optional_text(left: &Option<String>, right: &Option<String>) -> Ordering {
    normalize(left.as_deref().unwrap_or_default())
        .cmp(&normalize(right.as_deref().unwrap_or_default()))
}

fn searchable_fields(track: &Track) -> Vec<String> {
    let metadata = &track.metadata;
    let mut fields = Vec::new();

    push_optional(&mut fields, &metadata.title);
    push_optional(&mut fields, &metadata.artist);
    push_optional(&mut fields, &metadata.album);
    push_optional(&mut fields, &metadata.album_artist);
    push_optional(&mut fields, &metadata.composer);
    push_optional(&mut fields, &metadata.genre);
    fields.push(track.location.path.to_string_lossy().into_owned());

    fields
}

fn push_optional(fields: &mut Vec<String>, value: &Option<String>) {
    if let Some(value) = value {
        fields.push(value.clone());
    }
}

fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use xtunes_domain::{PlayStatistics, Rating, TrackId, TrackLocation, TrackMetadata};

    use super::{
        SearchError, filter_tracks_by_search_text, sort_tracks, track_matches_search_text,
    };
    use crate::Track;
    use crate::{SortDirection, TrackSort, TrackSortColumn};

    #[test]
    fn blank_search_returns_all_tracks() {
        let tracks = vec![track(1, "Angel", "Massive Attack")];

        assert_eq!(filter_tracks_by_search_text(&tracks, "   "), tracks);
    }

    #[test]
    fn search_matches_track_title_case_insensitively() {
        let track = track(1, "Angel", "Massive Attack");

        assert!(track_matches_search_text(&track, "angel"));
        assert!(track_matches_search_text(&track, "ANGEL"));
    }

    #[test]
    fn search_matches_artist_album_genre_and_path() {
        let mut track = track(1, "Untitled", "Unknown");
        track.metadata.album = Some("Mezzanine".to_owned());
        track.metadata.genre = Some("Trip Hop".to_owned());
        track.location = TrackLocation::new(PathBuf::from("/music/Massive Attack/track.flac"));

        assert!(track_matches_search_text(&track, "mezzanine"));
        assert!(track_matches_search_text(&track, "trip hop"));
        assert!(track_matches_search_text(&track, "massive attack"));
    }

    #[test]
    fn search_excludes_tracks_without_a_matching_field() {
        let tracks = vec![
            track(1, "Angel", "Massive Attack"),
            track(2, "Roads", "Portishead"),
        ];

        assert_eq!(
            filter_tracks_by_search_text(&tracks, "port"),
            vec![track(2, "Roads", "Portishead")]
        );
    }

    #[test]
    fn sort_orders_tracks_by_text_columns_case_insensitively() {
        let tracks = vec![
            track(1, "zebra", "Artist"),
            track(2, "Alpha", "Artist"),
            track(3, "middle", "Artist"),
        ];

        assert_eq!(
            sort_tracks(
                tracks,
                TrackSort {
                    column: TrackSortColumn::Title,
                    direction: SortDirection::Ascending
                }
            ),
            Ok(vec![
                track(2, "Alpha", "Artist"),
                track(3, "middle", "Artist"),
                track(1, "zebra", "Artist")
            ])
        );
    }

    #[test]
    fn sort_supports_descending_rating() {
        let mut low = track(1, "Low", "Artist");
        low.rating = rating(1);
        let mut high = track(2, "High", "Artist");
        high.rating = rating(5);

        assert_eq!(
            sort_tracks(
                vec![low.clone(), high.clone()],
                TrackSort {
                    column: TrackSortColumn::Rating,
                    direction: SortDirection::Descending
                }
            ),
            Ok(vec![high, low])
        );
    }

    #[test]
    fn sort_rejects_date_added_until_track_model_supports_it() {
        assert_eq!(
            sort_tracks(
                Vec::new(),
                TrackSort {
                    column: TrackSortColumn::DateAdded,
                    direction: SortDirection::Ascending
                }
            ),
            Err(SearchError::UnsupportedSortColumn(
                TrackSortColumn::DateAdded
            ))
        );
    }

    fn track(id: i64, title: &str, artist: &str) -> Track {
        Track {
            id: track_id(id),
            location: TrackLocation::new(PathBuf::from(format!("/music/{title}.flac"))),
            metadata: TrackMetadata {
                title: Some(title.to_owned()),
                artist: Some(artist.to_owned()),
                ..TrackMetadata::default()
            },
            rating: Rating::unrated(),
            statistics: PlayStatistics::default(),
        }
    }

    fn track_id(value: i64) -> TrackId {
        match TrackId::new(value) {
            Some(track_id) => track_id,
            None => unreachable!("test helper only constructs positive ids"),
        }
    }

    fn rating(stars: u8) -> Rating {
        match Rating::new(stars) {
            Some(rating) => rating,
            None => unreachable!("test helper only constructs valid ratings"),
        }
    }
}
