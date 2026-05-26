// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::sync::LazyLock;

// Sustain is in pre-release development: the SQLite schema is not yet stable.
// Schema changes are made by editing these CREATE TABLE statements; any
// existing local database is expected to be wiped and rebuilt from a library
// re-scan, not migrated. Do not add migration code for in-development schemas.
pub(super) const SCHEMA_SQL: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS tracks (
    id INTEGER PRIMARY KEY,
    relative_path TEXT NOT NULL UNIQUE,
    title TEXT,
    artist TEXT,
    album TEXT,
    album_artist TEXT,
    composer TEXT,
    genre TEXT,
    track_number INTEGER,
    disc_number INTEGER,
    year INTEGER,
    duration_seconds INTEGER,
    bitrate_kbps INTEGER,
    rating INTEGER NOT NULL DEFAULT 0,
    play_count INTEGER NOT NULL DEFAULT 0,
    skip_count INTEGER NOT NULL DEFAULT 0,
    last_played_at_unix INTEGER,
    last_skipped_at_unix INTEGER,
    date_added_at_unix INTEGER,
    is_missing INTEGER NOT NULL DEFAULT 0,
    grouping TEXT,
    track_total INTEGER,
    disc_total INTEGER,
    compilation INTEGER,
    bpm INTEGER,
    musical_key TEXT,
    comments TEXT,
    sample_rate_hz INTEGER,
    channels INTEGER,
    lyrics TEXT,
    content_hash TEXT,
    file_size_bytes INTEGER
);

CREATE INDEX IF NOT EXISTS tracks_content_hash_idx
    ON tracks(content_hash)
    WHERE content_hash IS NOT NULL;

CREATE TABLE IF NOT EXISTS playlist_folders (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    parent_folder_id INTEGER,
    position INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (parent_folder_id) REFERENCES playlist_folders(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS playlists (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    parent_folder_id INTEGER,
    position INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (parent_folder_id) REFERENCES playlist_folders(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS playlist_entries (
    playlist_id INTEGER NOT NULL,
    track_id INTEGER NOT NULL,
    position INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, track_id),
    FOREIGN KEY (playlist_id) REFERENCES playlists(id) ON DELETE CASCADE,
    FOREIGN KEY (track_id) REFERENCES tracks(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS smart_playlists (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    parent_folder_id INTEGER,
    position INTEGER NOT NULL DEFAULT 0,
    match_kind TEXT NOT NULL,
    limit_count INTEGER,
    limit_selection TEXT,
    FOREIGN KEY (parent_folder_id) REFERENCES playlist_folders(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS smart_playlist_rules (
    smart_playlist_id INTEGER NOT NULL,
    position INTEGER NOT NULL,
    kind TEXT NOT NULL,
    field TEXT,
    text_operator TEXT,
    text_value TEXT,
    number_operator TEXT,
    number_value INTEGER,
    rating_stars INTEGER,
    date_unix INTEGER,
    days_value INTEGER,
    PRIMARY KEY (smart_playlist_id, position),
    FOREIGN KEY (smart_playlist_id) REFERENCES smart_playlists(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS track_column_layout_default (
    column_id TEXT PRIMARY KEY,
    position  INTEGER NOT NULL,
    visible   INTEGER NOT NULL,
    width_px  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS track_column_layout_playlist_override (
    playlist_id INTEGER NOT NULL,
    column_id   TEXT    NOT NULL,
    position    INTEGER NOT NULL,
    visible     INTEGER NOT NULL,
    width_px    INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, column_id),
    FOREIGN KEY (playlist_id) REFERENCES playlists(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS track_column_layout_smart_playlist_override (
    smart_playlist_id INTEGER NOT NULL,
    column_id         TEXT    NOT NULL,
    position          INTEGER NOT NULL,
    visible           INTEGER NOT NULL,
    width_px          INTEGER NOT NULL,
    PRIMARY KEY (smart_playlist_id, column_id),
    FOREIGN KEY (smart_playlist_id) REFERENCES smart_playlists(id) ON DELETE CASCADE
);
"#;

#[derive(Clone, Copy)]
struct TrackColumn {
    name: &'static str,
    updatable: bool,
}

impl TrackColumn {
    const fn primary_key(name: &'static str) -> Self {
        Self {
            name,
            updatable: false,
        }
    }

    const fn stored_value(name: &'static str) -> Self {
        Self {
            name,
            updatable: true,
        }
    }
}

const TRACK_COLUMNS: &[TrackColumn] = &[
    TrackColumn::primary_key("id"),
    TrackColumn::stored_value("relative_path"),
    TrackColumn::stored_value("title"),
    TrackColumn::stored_value("artist"),
    TrackColumn::stored_value("album"),
    TrackColumn::stored_value("album_artist"),
    TrackColumn::stored_value("composer"),
    TrackColumn::stored_value("genre"),
    TrackColumn::stored_value("track_number"),
    TrackColumn::stored_value("disc_number"),
    TrackColumn::stored_value("year"),
    TrackColumn::stored_value("duration_seconds"),
    TrackColumn::stored_value("bitrate_kbps"),
    TrackColumn::stored_value("rating"),
    TrackColumn::stored_value("play_count"),
    TrackColumn::stored_value("skip_count"),
    TrackColumn::stored_value("last_played_at_unix"),
    TrackColumn::stored_value("last_skipped_at_unix"),
    TrackColumn::stored_value("date_added_at_unix"),
    TrackColumn::stored_value("is_missing"),
    TrackColumn::stored_value("grouping"),
    TrackColumn::stored_value("track_total"),
    TrackColumn::stored_value("disc_total"),
    TrackColumn::stored_value("compilation"),
    TrackColumn::stored_value("bpm"),
    TrackColumn::stored_value("musical_key"),
    TrackColumn::stored_value("comments"),
    TrackColumn::stored_value("sample_rate_hz"),
    TrackColumn::stored_value("channels"),
    TrackColumn::stored_value("lyrics"),
    TrackColumn::stored_value("content_hash"),
    TrackColumn::stored_value("file_size_bytes"),
];

pub(crate) mod track_column_index {
    pub(crate) const ID: usize = 0;
    pub(crate) const RELATIVE_PATH: usize = 1;
    pub(crate) const TITLE: usize = 2;
    pub(crate) const ARTIST: usize = 3;
    pub(crate) const ALBUM: usize = 4;
    pub(crate) const ALBUM_ARTIST: usize = 5;
    pub(crate) const COMPOSER: usize = 6;
    pub(crate) const GENRE: usize = 7;
    pub(crate) const TRACK_NUMBER: usize = 8;
    pub(crate) const DISC_NUMBER: usize = 9;
    pub(crate) const YEAR: usize = 10;
    pub(crate) const DURATION_SECONDS: usize = 11;
    pub(crate) const BITRATE_KBPS: usize = 12;
    pub(crate) const RATING: usize = 13;
    pub(crate) const PLAY_COUNT: usize = 14;
    pub(crate) const SKIP_COUNT: usize = 15;
    pub(crate) const LAST_PLAYED_AT_UNIX: usize = 16;
    pub(crate) const LAST_SKIPPED_AT_UNIX: usize = 17;
    pub(crate) const DATE_ADDED_AT_UNIX: usize = 18;
    pub(crate) const IS_MISSING: usize = 19;
    pub(crate) const GROUPING: usize = 20;
    pub(crate) const TRACK_TOTAL: usize = 21;
    pub(crate) const DISC_TOTAL: usize = 22;
    pub(crate) const COMPILATION: usize = 23;
    pub(crate) const BPM: usize = 24;
    pub(crate) const MUSICAL_KEY: usize = 25;
    pub(crate) const COMMENTS: usize = 26;
    pub(crate) const SAMPLE_RATE_HZ: usize = 27;
    pub(crate) const CHANNELS: usize = 28;
    pub(crate) const LYRICS: usize = 29;
    pub(crate) const CONTENT_HASH: usize = 30;
    pub(crate) const FILE_SIZE_BYTES: usize = 31;
}

pub(super) static SAVE_TRACK_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        r#"
INSERT INTO tracks (
{}
)
VALUES (
{}
)
ON CONFLICT(id) DO UPDATE SET
{}
"#,
        indented_track_column_names("    "),
        indented_insert_placeholders("    "),
        indented_track_update_assignments("    "),
    )
});

pub(super) static SELECT_TRACK_BY_ID_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        r#"
SELECT
{}
FROM tracks
WHERE id = ?1
"#,
        indented_track_column_names("    "),
    )
});

pub(super) static SELECT_TRACK_BY_CONTENT_HASH_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        r#"
SELECT
{}
FROM tracks
WHERE content_hash = ?1
ORDER BY id
LIMIT 1
"#,
        indented_track_column_names("    "),
    )
});

pub(super) static SELECT_ALL_TRACKS_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        r#"
SELECT
{}
FROM tracks
ORDER BY id
"#,
        indented_track_column_names("    "),
    )
});

fn indented_track_column_names(indent: &str) -> String {
    TRACK_COLUMNS
        .iter()
        .map(|column| format!("{indent}{}", column.name))
        .collect::<Vec<_>>()
        .join(",\n")
}

fn indented_insert_placeholders(indent: &str) -> String {
    (1..=TRACK_COLUMNS.len())
        .map(|index| format!("{indent}?{index}"))
        .collect::<Vec<_>>()
        .join(",\n")
}

fn indented_track_update_assignments(indent: &str) -> String {
    TRACK_COLUMNS
        .iter()
        .filter(|column| column.updatable)
        .map(|column| format!("{indent}{name} = excluded.{name}", name = column.name))
        .collect::<Vec<_>>()
        .join(",\n")
}

#[cfg(test)]
mod tests {
    use super::{TRACK_COLUMNS, track_column_index};

    #[test]
    fn track_column_indices_match_column_order() {
        let expected = [
            (track_column_index::ID, "id"),
            (track_column_index::RELATIVE_PATH, "relative_path"),
            (track_column_index::TITLE, "title"),
            (track_column_index::ARTIST, "artist"),
            (track_column_index::ALBUM, "album"),
            (track_column_index::ALBUM_ARTIST, "album_artist"),
            (track_column_index::COMPOSER, "composer"),
            (track_column_index::GENRE, "genre"),
            (track_column_index::TRACK_NUMBER, "track_number"),
            (track_column_index::DISC_NUMBER, "disc_number"),
            (track_column_index::YEAR, "year"),
            (track_column_index::DURATION_SECONDS, "duration_seconds"),
            (track_column_index::BITRATE_KBPS, "bitrate_kbps"),
            (track_column_index::RATING, "rating"),
            (track_column_index::PLAY_COUNT, "play_count"),
            (track_column_index::SKIP_COUNT, "skip_count"),
            (
                track_column_index::LAST_PLAYED_AT_UNIX,
                "last_played_at_unix",
            ),
            (
                track_column_index::LAST_SKIPPED_AT_UNIX,
                "last_skipped_at_unix",
            ),
            (track_column_index::DATE_ADDED_AT_UNIX, "date_added_at_unix"),
            (track_column_index::IS_MISSING, "is_missing"),
            (track_column_index::GROUPING, "grouping"),
            (track_column_index::TRACK_TOTAL, "track_total"),
            (track_column_index::DISC_TOTAL, "disc_total"),
            (track_column_index::COMPILATION, "compilation"),
            (track_column_index::BPM, "bpm"),
            (track_column_index::MUSICAL_KEY, "musical_key"),
            (track_column_index::COMMENTS, "comments"),
            (track_column_index::SAMPLE_RATE_HZ, "sample_rate_hz"),
            (track_column_index::CHANNELS, "channels"),
            (track_column_index::LYRICS, "lyrics"),
            (track_column_index::CONTENT_HASH, "content_hash"),
            (track_column_index::FILE_SIZE_BYTES, "file_size_bytes"),
        ];

        assert_eq!(TRACK_COLUMNS.len(), expected.len());
        for (index, name) in expected {
            assert_eq!(TRACK_COLUMNS[index].name, name);
        }
    }
}
