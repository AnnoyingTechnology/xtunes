// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::cmp::Ordering as CmpOrdering;

use super::row::TrackTableRow;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TrackTableColumn {
    TrackName,
    Artist,
    Album,
    Genre,
    Year,
    Bpm,
    Bitrate,
    FileType,
    Duration,
    Rating,
    Plays,
    LastPlayed,
    DateAdded,
    TrackNumber,
}

pub(super) const TRACK_TABLE_COLUMNS: &[TrackTableColumn] = &[
    TrackTableColumn::TrackName,
    TrackTableColumn::Artist,
    TrackTableColumn::Album,
    TrackTableColumn::Genre,
    TrackTableColumn::Year,
    TrackTableColumn::Bpm,
    TrackTableColumn::Bitrate,
    TrackTableColumn::FileType,
    TrackTableColumn::Duration,
    TrackTableColumn::Rating,
    TrackTableColumn::Plays,
    TrackTableColumn::LastPlayed,
    TrackTableColumn::DateAdded,
    TrackTableColumn::TrackNumber,
];

impl TrackTableColumn {
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::TrackName => "Track Name",
            Self::Artist => "Artist",
            Self::Album => "Album",
            Self::Genre => "Genre",
            Self::Year => "Year",
            Self::Bpm => "BPM",
            Self::Bitrate => "Bitrate",
            Self::FileType => "Type",
            Self::Duration => "Duration",
            Self::Rating => "Rating",
            Self::Plays => "Plays",
            Self::LastPlayed => "Last Played",
            Self::DateAdded => "Date Added",
            Self::TrackNumber => "Track #",
        }
    }

    pub(super) fn action_name(self) -> &'static str {
        match self {
            Self::TrackName => "track_name",
            Self::Artist => "artist",
            Self::Album => "album",
            Self::Genre => "genre",
            Self::Year => "year",
            Self::Bpm => "bpm",
            Self::Bitrate => "bitrate",
            Self::FileType => "file_type",
            Self::Duration => "duration",
            Self::Rating => "rating",
            Self::Plays => "plays",
            Self::LastPlayed => "last_played",
            Self::DateAdded => "date_added",
            Self::TrackNumber => "track_number",
        }
    }

    pub(super) fn default_width(self) -> i32 {
        match self {
            Self::TrackName => 220,
            Self::Artist => 150,
            Self::Album => 170,
            Self::Genre => 120,
            Self::Year => 72,
            Self::Bpm => 72,
            Self::Bitrate => 90,
            Self::FileType => 72,
            Self::Duration => 86,
            Self::Rating => 94,
            Self::Plays => 76,
            Self::LastPlayed => 120,
            Self::DateAdded => 120,
            Self::TrackNumber => 86,
        }
    }

    pub(super) fn expands(self) -> bool {
        false
    }

    pub(super) fn default_visible(self) -> bool {
        true
    }

    pub(super) fn xalign(self) -> f32 {
        match self {
            Self::TrackName
            | Self::Artist
            | Self::Album
            | Self::Genre
            | Self::FileType
            | Self::LastPlayed
            | Self::DateAdded => 0.0,
            Self::Year
            | Self::Bpm
            | Self::Bitrate
            | Self::Duration
            | Self::Rating
            | Self::Plays
            | Self::TrackNumber => 1.0,
        }
    }

    pub(super) fn text(self, row: &TrackTableRow) -> String {
        match self {
            Self::TrackName => row.track_name.clone(),
            Self::Artist => row.artist.clone(),
            Self::Album => row.album.clone(),
            Self::Genre => row.genre.clone(),
            Self::Year => optional_number_text(row.year),
            Self::Bpm => optional_number_text(row.bpm),
            Self::Bitrate => row
                .bitrate_kbps
                .map(|bitrate| format!("{bitrate} kbps"))
                .unwrap_or_default(),
            Self::FileType => row.file_type.label().to_owned(),
            Self::Duration => track_duration_text(row.duration_seconds),
            Self::Rating => row.rating.to_string(),
            Self::Plays => row.plays.to_string(),
            Self::LastPlayed => row.last_played.clone().unwrap_or_default(),
            Self::DateAdded => row.date_added.clone(),
            Self::TrackNumber => optional_number_text(row.track_number),
        }
    }

    pub(super) fn compare(self, left: &TrackTableRow, right: &TrackTableRow) -> CmpOrdering {
        match self {
            Self::TrackName => compare_text(&left.track_name, &right.track_name),
            Self::Artist => compare_text(&left.artist, &right.artist),
            Self::Album => compare_text(&left.album, &right.album),
            Self::Genre => compare_text(&left.genre, &right.genre),
            Self::Year => left.year.cmp(&right.year),
            Self::Bpm => left.bpm.cmp(&right.bpm),
            Self::Bitrate => left.bitrate_kbps.cmp(&right.bitrate_kbps),
            Self::FileType => left.file_type.label().cmp(right.file_type.label()),
            Self::Duration => left.duration_seconds.cmp(&right.duration_seconds),
            Self::Rating => left.rating.cmp(&right.rating),
            Self::Plays => left.plays.cmp(&right.plays),
            Self::LastPlayed => left.last_played.cmp(&right.last_played),
            Self::DateAdded => left.date_added.cmp(&right.date_added),
            Self::TrackNumber => left.track_number.cmp(&right.track_number),
        }
    }
}

fn compare_text(left: &str, right: &str) -> CmpOrdering {
    left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase())
}

fn optional_number_text<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn track_duration_text(duration_seconds: u64) -> String {
    let hours = duration_seconds / 3_600;
    let minutes = duration_seconds % 3_600 / 60;
    let seconds = duration_seconds % 60;

    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_columns_match_the_product_contract() {
        let titles: Vec<&str> = TRACK_TABLE_COLUMNS
            .iter()
            .map(|column| column.title())
            .collect();

        assert_eq!(
            titles,
            vec![
                "Track Name",
                "Artist",
                "Album",
                "Genre",
                "Year",
                "BPM",
                "Bitrate",
                "Type",
                "Duration",
                "Rating",
                "Plays",
                "Last Played",
                "Date Added",
                "Track #",
            ]
        );
    }

    #[test]
    fn table_columns_have_stable_action_names() {
        let action_names: Vec<&str> = TRACK_TABLE_COLUMNS
            .iter()
            .map(|column| column.action_name())
            .collect();

        assert_eq!(
            action_names,
            vec![
                "track_name",
                "artist",
                "album",
                "genre",
                "year",
                "bpm",
                "bitrate",
                "file_type",
                "duration",
                "rating",
                "plays",
                "last_played",
                "date_added",
                "track_number",
            ]
        );
    }

    #[test]
    fn track_duration_text_uses_minutes_until_an_hour() {
        assert_eq!(track_duration_text(244), "4:04");
    }

    #[test]
    fn track_duration_text_uses_hours_when_needed() {
        assert_eq!(track_duration_text(3_904), "1:05:04");
    }
}
