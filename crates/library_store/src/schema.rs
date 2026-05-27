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

-- Per-track analysis bookkeeping. Tiny row; never carries BLOB data.
-- The scheduler's "find tracks needing analysis" query LEFT JOINs against
-- this table and tests the *_attempted_at_unix columns to decide whether
-- a capability has been tried yet — distinguishing "not yet attempted"
-- (NULL) from "tried, no result" (timestamp set, tracks.bpm still NULL).
-- analyzer_version is bumped centrally when the DSP changes meaningfully,
-- so older rows are excluded from "fresh enough" checks without any
-- migration step.
CREATE TABLE IF NOT EXISTS track_analysis (
    track_id                    INTEGER PRIMARY KEY
                                  REFERENCES tracks(id) ON DELETE CASCADE,
    bpm_attempted_at_unix       INTEGER,
    key_attempted_at_unix       INTEGER,
    waveform_attempted_at_unix  INTEGER,
    analyzer_version            INTEGER NOT NULL
);

-- Waveform BLOBs only. Split from track_analysis so a future
-- ATTACH-based relocation of the bulk data to a sidecar database is
-- a schema edit, not a refactor. Each segments BLOB is `n * 4` bytes;
-- segment count is recovered as `blob.len() / 4`.
CREATE TABLE IF NOT EXISTS track_waveform (
    track_id                    INTEGER PRIMARY KEY
                                  REFERENCES tracks(id) ON DELETE CASCADE,
    preview_segment_duration_ms REAL    NOT NULL,
    preview_segments            BLOB    NOT NULL,
    detail_segment_duration_ms  REAL    NOT NULL,
    detail_segments             BLOB    NOT NULL
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

/// Upsert into `track_analysis`. Each `*_attempted_at_unix` parameter
/// is either the analysis timestamp (if the capability ran this pass)
/// or `NULL` (if it did not) — `COALESCE` preserves whatever value
/// was already stored in that column, so a BPM-only re-analysis does
/// not clobber the waveform's "attempted" timestamp.
pub(super) const UPSERT_TRACK_ANALYSIS_SQL: &str = r#"
INSERT INTO track_analysis (
    track_id,
    bpm_attempted_at_unix,
    key_attempted_at_unix,
    waveform_attempted_at_unix,
    analyzer_version
)
VALUES (?1, ?2, ?3, ?4, ?5)
ON CONFLICT(track_id) DO UPDATE SET
    bpm_attempted_at_unix = COALESCE(excluded.bpm_attempted_at_unix, bpm_attempted_at_unix),
    key_attempted_at_unix = COALESCE(excluded.key_attempted_at_unix, key_attempted_at_unix),
    waveform_attempted_at_unix = COALESCE(excluded.waveform_attempted_at_unix, waveform_attempted_at_unix),
    analyzer_version = excluded.analyzer_version
"#;

pub(super) const UPSERT_TRACK_WAVEFORM_SQL: &str = r#"
INSERT INTO track_waveform (
    track_id,
    preview_segment_duration_ms,
    preview_segments,
    detail_segment_duration_ms,
    detail_segments
)
VALUES (?1, ?2, ?3, ?4, ?5)
ON CONFLICT(track_id) DO UPDATE SET
    preview_segment_duration_ms = excluded.preview_segment_duration_ms,
    preview_segments = excluded.preview_segments,
    detail_segment_duration_ms = excluded.detail_segment_duration_ms,
    detail_segments = excluded.detail_segments
"#;

/// "Fill in `tracks.bpm` only if it is currently NULL." Honors the
/// rule that user-edited or tag-imported values win — the analyzer
/// supplies missing data, it never overrides existing data.
pub(super) const FILL_TRACK_BPM_IF_NULL_SQL: &str =
    r#"UPDATE tracks SET bpm = ?1 WHERE id = ?2 AND bpm IS NULL"#;

pub(super) const FILL_TRACK_MUSICAL_KEY_IF_NULL_SQL: &str =
    r#"UPDATE tracks SET musical_key = ?1 WHERE id = ?2 AND musical_key IS NULL"#;

pub(super) const SELECT_TRACK_WAVEFORM_SQL: &str = r#"
SELECT
    preview_segment_duration_ms,
    preview_segments,
    detail_segment_duration_ms,
    detail_segments
FROM track_waveform
WHERE track_id = ?1
"#;

/// "Find tracks needing analysis." Returns track IDs that are not
/// marked missing AND have at least one of the requested capabilities
/// either un-attempted (NULL timestamp) or stamped by an older
/// analyzer_version. Bound parameters in order:
///   ?1 = include_bpm        (1 or 0)
///   ?2 = include_key        (1 or 0)
///   ?3 = include_waveform   (1 or 0)
///   ?4 = current analyzer_version
///   ?5 = LIMIT
pub(super) const SELECT_TRACKS_NEEDING_ANALYSIS_SQL: &str = r#"
SELECT t.id
FROM tracks t
LEFT JOIN track_analysis ta ON ta.track_id = t.id
WHERE t.is_missing = 0
  AND (
        (?1 = 1 AND (ta.bpm_attempted_at_unix      IS NULL OR ta.analyzer_version < ?4))
     OR (?2 = 1 AND (ta.key_attempted_at_unix      IS NULL OR ta.analyzer_version < ?4))
     OR (?3 = 1 AND (ta.waveform_attempted_at_unix IS NULL OR ta.analyzer_version < ?4))
      )
ORDER BY t.id
LIMIT ?5
"#;

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
