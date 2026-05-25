// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::cmp::Ordering;

use sustain_domain::{SortDirection, Track, TrackSortColumn};

pub(crate) fn track_matches_search(track: &Track, search_text: &str) -> bool {
    let needle = search_text.to_ascii_lowercase();
    [
        track.metadata.title.as_deref(),
        track.metadata.artist.as_deref(),
        track.metadata.album.as_deref(),
        track.metadata.album_artist.as_deref(),
        track.metadata.composer.as_deref(),
        track.metadata.genre.as_deref(),
        track.location.path().to_str(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_ascii_lowercase().contains(&needle))
}

pub(crate) fn sort_tracks(tracks: &mut [Track], sort: sustain_domain::TrackSort) {
    tracks.sort_by(|left, right| {
        let ordering = compare_tracks(left, right, sort.column);
        let ordering = if sort.column == TrackSortColumn::PlaylistPosition {
            ordering
        } else {
            ordering.then_with(|| left.id.cmp(&right.id))
        };
        match sort.direction {
            SortDirection::Ascending => ordering,
            SortDirection::Descending => ordering.reverse(),
        }
    });
}

fn compare_tracks(left: &Track, right: &Track, column: TrackSortColumn) -> Ordering {
    match column {
        TrackSortColumn::PlaylistPosition => Ordering::Equal,
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
        TrackSortColumn::Rating => left.rating.stars().cmp(&right.rating.stars()),
        TrackSortColumn::PlayCount => left.statistics.play_count.cmp(&right.statistics.play_count),
        TrackSortColumn::LastPlayed => left
            .statistics
            .last_played_at
            .cmp(&right.statistics.last_played_at),
        TrackSortColumn::Duration => left.metadata.duration.cmp(&right.metadata.duration),
        TrackSortColumn::DateAdded => left
            .statistics
            .date_added_at
            .cmp(&right.statistics.date_added_at),
    }
}

fn compare_optional_text(left: &Option<String>, right: &Option<String>) -> Ordering {
    let left = left
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let right = right
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    left.cmp(&right)
}
