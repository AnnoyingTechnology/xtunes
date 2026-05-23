// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

#![forbid(unsafe_code)]

use std::{
    cmp::Ordering,
    collections::BTreeMap,
    path::Path,
    sync::{Mutex, MutexGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use std::num::NonZeroU32;

use rusqlite::{Connection, Row, params};
pub use xtunes_domain::{
    LibraryQuery, Playlist, PlaylistFolder, PlaylistFolderId, PlaylistId, Rating, SmartPlaylist,
    SmartPlaylistId, Track, TrackId,
};
use xtunes_domain::{
    PlayStatistics, PlaylistEntry, SmartPlaylistDateField, SmartPlaylistLimit,
    SmartPlaylistLimitSelection, SmartPlaylistMatchKind, SmartPlaylistNumberField,
    SmartPlaylistNumberOperator, SmartPlaylistRule, SmartPlaylistRuleSet, SmartPlaylistTextField,
    SmartPlaylistTextOperator, SortDirection, TrackLocation, TrackMetadata, TrackRelativePath,
    TrackSortColumn,
};

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreError {
    Database(String),
    InvalidStoredId(i64),
    InvalidStoredPath(String),
    InvalidStoredEnum(String),
    StoreUnavailable,
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error.to_string())
    }
}

pub trait LibraryStore: Send + Sync {
    fn save_track(&self, track: Track) -> StoreResult<()>;
    fn delete_track(&self, track_id: TrackId) -> StoreResult<()>;
    fn track(&self, track_id: TrackId) -> StoreResult<Option<Track>>;
    fn tracks(&self) -> StoreResult<Vec<Track>>;
    fn save_playlist(&self, playlist: Playlist) -> StoreResult<()>;
    fn playlist(&self, playlist_id: PlaylistId) -> StoreResult<Option<Playlist>>;
    fn playlists(&self) -> StoreResult<Vec<Playlist>>;
    fn delete_playlist(&self, playlist_id: PlaylistId) -> StoreResult<()>;
    fn save_playlist_folder(&self, folder: PlaylistFolder) -> StoreResult<()>;
    fn playlist_folder(&self, folder_id: PlaylistFolderId) -> StoreResult<Option<PlaylistFolder>>;
    fn playlist_folders(&self) -> StoreResult<Vec<PlaylistFolder>>;
    fn delete_playlist_folder(&self, folder_id: PlaylistFolderId) -> StoreResult<()>;
    fn save_smart_playlist(&self, smart_playlist: SmartPlaylist) -> StoreResult<()>;
    fn smart_playlist(
        &self,
        smart_playlist_id: SmartPlaylistId,
    ) -> StoreResult<Option<SmartPlaylist>>;
    fn smart_playlists(&self) -> StoreResult<Vec<SmartPlaylist>>;
    fn delete_smart_playlist(&self, smart_playlist_id: SmartPlaylistId) -> StoreResult<()>;

    fn tracks_matching(&self, query: LibraryQuery) -> StoreResult<Vec<Track>> {
        let mut tracks = if let Some(playlist_id) = query.playlist_id {
            let Some(playlist) = self.playlist(playlist_id)? else {
                return Ok(Vec::new());
            };
            let tracks_by_id = self
                .tracks()?
                .into_iter()
                .map(|track| (track.id, track))
                .collect::<BTreeMap<_, _>>();

            playlist
                .entries
                .into_iter()
                .filter_map(|entry| tracks_by_id.get(&entry.track_id).cloned())
                .collect()
        } else {
            self.tracks()?
        };

        if let Some(search_text) = query.search_text.as_deref() {
            tracks.retain(|track| track_matches_search(track, search_text));
        }

        sort_tracks(&mut tracks, query.sort);
        Ok(tracks)
    }
}

#[derive(Debug)]
pub struct SqliteLibraryStore {
    connection: Mutex<Connection>,
}

impl SqliteLibraryStore {
    pub fn open_default() -> StoreResult<Self> {
        Self::open(default_database_path().ok_or(StoreError::StoreUnavailable)?)
    }

    pub fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| StoreError::Database(error.to_string()))?;
        }
        let connection = Connection::open(path).map_err(StoreError::from)?;
        Self::from_connection(connection)
    }

    pub fn open_in_memory() -> StoreResult<Self> {
        let connection = Connection::open_in_memory().map_err(StoreError::from)?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> StoreResult<Self> {
        let store = Self {
            connection: Mutex::new(connection),
        };
        store.migrate()?;
        Ok(store)
    }

    fn connection_guard(&self) -> StoreResult<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| StoreError::StoreUnavailable)
    }

    // xTunes is in pre-release development: the SQLite schema is not yet stable.
    // Schema changes are made by editing the CREATE TABLE statements below; any
    // existing local database is expected to be wiped and rebuilt from a library
    // re-scan, not migrated. Do not add migration code for in-development schemas.
    fn migrate(&self) -> StoreResult<()> {
        self.connection_guard()?
            .execute_batch(
                r#"
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
                    channels INTEGER
                );

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
                "#,
            )
            .map_err(StoreError::from)
    }
}

fn default_database_path() -> Option<std::path::PathBuf> {
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        return Some(
            std::path::PathBuf::from(data_home)
                .join("xtunes")
                .join("library.sqlite"),
        );
    }

    std::env::var_os("HOME").map(|home| {
        std::path::PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("xtunes")
            .join("library.sqlite")
    })
}

fn track_matches_search(track: &Track, search_text: &str) -> bool {
    let needle = search_text.to_ascii_lowercase();
    [
        track.metadata.title.as_deref(),
        track.metadata.artist.as_deref(),
        track.metadata.album.as_deref(),
        track.metadata.album_artist.as_deref(),
        track.metadata.composer.as_deref(),
        track.metadata.genre.as_deref(),
        track.location.relative_path.as_path().to_str(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_ascii_lowercase().contains(&needle))
}

fn sort_tracks(tracks: &mut [Track], sort: xtunes_domain::TrackSort) {
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
        TrackSortColumn::DateAdded => left.id.cmp(&right.id),
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

impl LibraryStore for SqliteLibraryStore {
    fn save_track(&self, track: Track) -> StoreResult<()> {
        let metadata = track.metadata;
        let statistics = track.statistics;
        self.connection_guard()?
            .execute(
                r#"
                INSERT INTO tracks (
                    id,
                    relative_path,
                    title,
                    artist,
                    album,
                    album_artist,
                    composer,
                    genre,
                    track_number,
                    disc_number,
                    year,
                    duration_seconds,
                    bitrate_kbps,
                    rating,
                    play_count,
                    skip_count,
                    last_played_at_unix,
                    last_skipped_at_unix,
                    date_added_at_unix,
                    is_missing,
                    grouping,
                    track_total,
                    disc_total,
                    compilation,
                    bpm,
                    musical_key,
                    comments,
                    sample_rate_hz,
                    channels
                )
                VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                    ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                    ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29
                )
                ON CONFLICT(id) DO UPDATE SET
                    relative_path = excluded.relative_path,
                    title = excluded.title,
                    artist = excluded.artist,
                    album = excluded.album,
                    album_artist = excluded.album_artist,
                    composer = excluded.composer,
                    genre = excluded.genre,
                    track_number = excluded.track_number,
                    disc_number = excluded.disc_number,
                    year = excluded.year,
                    duration_seconds = excluded.duration_seconds,
                    bitrate_kbps = excluded.bitrate_kbps,
                    rating = excluded.rating,
                    play_count = excluded.play_count,
                    skip_count = excluded.skip_count,
                    last_played_at_unix = excluded.last_played_at_unix,
                    last_skipped_at_unix = excluded.last_skipped_at_unix,
                    date_added_at_unix = excluded.date_added_at_unix,
                    is_missing = excluded.is_missing,
                    grouping = excluded.grouping,
                    track_total = excluded.track_total,
                    disc_total = excluded.disc_total,
                    compilation = excluded.compilation,
                    bpm = excluded.bpm,
                    musical_key = excluded.musical_key,
                    comments = excluded.comments,
                    sample_rate_hz = excluded.sample_rate_hz,
                    channels = excluded.channels
                "#,
                params![
                    track.id.get(),
                    track.location.relative_path.as_path().to_string_lossy(),
                    metadata.title,
                    metadata.artist,
                    metadata.album,
                    metadata.album_artist,
                    metadata.composer,
                    metadata.genre,
                    metadata.track_number.map(i64::from),
                    metadata.disc_number.map(i64::from),
                    metadata.year.map(i64::from),
                    metadata.duration.map(duration_to_seconds),
                    metadata.bitrate_kbps.map(i64::from),
                    i64::from(track.rating.stars()),
                    statistics.play_count as i64,
                    statistics.skip_count as i64,
                    statistics.last_played_at.and_then(system_time_to_unix),
                    statistics.last_skipped_at.and_then(system_time_to_unix),
                    statistics.date_added_at.and_then(system_time_to_unix),
                    track.location.is_missing(),
                    metadata.grouping,
                    metadata.track_total.map(i64::from),
                    metadata.disc_total.map(i64::from),
                    metadata.compilation,
                    metadata.bpm.map(i64::from),
                    metadata.key,
                    metadata.comments,
                    metadata.sample_rate_hz.map(i64::from),
                    metadata.channels.map(i64::from),
                ],
            )
            .map(|_| ())
            .map_err(StoreError::from)
    }

    fn delete_track(&self, track_id: TrackId) -> StoreResult<()> {
        self.connection_guard()?
            .execute("DELETE FROM tracks WHERE id = ?1", params![track_id.get()])
            .map(|_| ())
            .map_err(StoreError::from)
    }

    fn track(&self, track_id: TrackId) -> StoreResult<Option<Track>> {
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare(
                r#"
                SELECT
                    id,
                    relative_path,
                    title,
                    artist,
                    album,
                    album_artist,
                    composer,
                    genre,
                    track_number,
                    disc_number,
                    year,
                    duration_seconds,
                    bitrate_kbps,
                    rating,
                    play_count,
                    skip_count,
                    last_played_at_unix,
                    last_skipped_at_unix,
                    date_added_at_unix,
                    is_missing,
                    grouping,
                    track_total,
                    disc_total,
                    compilation,
                    bpm,
                    musical_key,
                    comments,
                    sample_rate_hz,
                    channels
                FROM tracks
                WHERE id = ?1
                "#,
            )
            .map_err(StoreError::from)?;
        let mut rows = statement
            .query(params![track_id.get()])
            .map_err(StoreError::from)?;

        rows.next()
            .map_err(StoreError::from)?
            .map(track_from_row)
            .transpose()
    }

    fn tracks(&self) -> StoreResult<Vec<Track>> {
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare(
                r#"
                SELECT
                    id,
                    relative_path,
                    title,
                    artist,
                    album,
                    album_artist,
                    composer,
                    genre,
                    track_number,
                    disc_number,
                    year,
                    duration_seconds,
                    bitrate_kbps,
                    rating,
                    play_count,
                    skip_count,
                    last_played_at_unix,
                    last_skipped_at_unix,
                    date_added_at_unix,
                    is_missing,
                    grouping,
                    track_total,
                    disc_total,
                    compilation,
                    bpm,
                    musical_key,
                    comments,
                    sample_rate_hz,
                    channels
                FROM tracks
                ORDER BY id
                "#,
            )
            .map_err(StoreError::from)?;
        let mut rows = statement.query([]).map_err(StoreError::from)?;
        let mut tracks = Vec::new();

        while let Some(row) = rows.next().map_err(StoreError::from)? {
            tracks.push(track_from_row(row)?);
        }

        Ok(tracks)
    }

    fn save_playlist(&self, playlist: Playlist) -> StoreResult<()> {
        let mut connection = self.connection_guard()?;
        let transaction = connection.transaction().map_err(StoreError::from)?;
        transaction
            .execute(
                r#"
                INSERT INTO playlists (id, name, parent_folder_id, position)
                VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    parent_folder_id = excluded.parent_folder_id,
                    position = excluded.position
                "#,
                params![
                    playlist.id.get(),
                    playlist.name,
                    playlist.parent_folder_id.map(PlaylistFolderId::get),
                    i64::from(playlist.position),
                ],
            )
            .map_err(StoreError::from)?;
        transaction
            .execute(
                "DELETE FROM playlist_entries WHERE playlist_id = ?1",
                params![playlist.id.get()],
            )
            .map_err(StoreError::from)?;

        for entry in playlist.entries {
            transaction
                .execute(
                    r#"
                    INSERT INTO playlist_entries (playlist_id, track_id, position)
                    VALUES (?1, ?2, ?3)
                    "#,
                    params![
                        entry.playlist_id.get(),
                        entry.track_id.get(),
                        i64::from(entry.position),
                    ],
                )
                .map_err(StoreError::from)?;
        }

        transaction.commit().map_err(StoreError::from)
    }

    fn playlist(&self, playlist_id: PlaylistId) -> StoreResult<Option<Playlist>> {
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare("SELECT id, name, parent_folder_id, position FROM playlists WHERE id = ?1")
            .map_err(StoreError::from)?;
        let mut rows = statement
            .query(params![playlist_id.get()])
            .map_err(StoreError::from)?;

        let Some(row) = rows.next().map_err(StoreError::from)? else {
            return Ok(None);
        };
        let id = playlist_id_from_db(row.get(0).map_err(StoreError::from)?)?;
        let name = row.get(1).map_err(StoreError::from)?;
        let parent_folder_id = optional_playlist_folder_id_from_row(row, 2)?;
        let position = u32_from_row(row, 3)?;
        let entries = playlist_entries(&connection, id)?;

        Ok(Some(Playlist {
            id,
            name,
            parent_folder_id,
            position,
            entries,
        }))
    }

    fn playlists(&self) -> StoreResult<Vec<Playlist>> {
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare("SELECT id, name, parent_folder_id, position FROM playlists ORDER BY id")
            .map_err(StoreError::from)?;
        let mut rows = statement.query([]).map_err(StoreError::from)?;
        let mut playlists = Vec::new();

        while let Some(row) = rows.next().map_err(StoreError::from)? {
            let id = playlist_id_from_db(row.get(0).map_err(StoreError::from)?)?;
            let name = row.get(1).map_err(StoreError::from)?;
            let parent_folder_id = optional_playlist_folder_id_from_row(row, 2)?;
            let position = u32_from_row(row, 3)?;
            playlists.push(Playlist {
                id,
                name,
                parent_folder_id,
                position,
                entries: playlist_entries(&connection, id)?,
            });
        }

        Ok(playlists)
    }

    fn delete_playlist(&self, playlist_id: PlaylistId) -> StoreResult<()> {
        self.connection_guard()?
            .execute(
                "DELETE FROM playlists WHERE id = ?1",
                params![playlist_id.get()],
            )
            .map(|_| ())
            .map_err(StoreError::from)
    }

    fn save_playlist_folder(&self, folder: PlaylistFolder) -> StoreResult<()> {
        self.connection_guard()?
            .execute(
                r#"
                INSERT INTO playlist_folders (id, name, parent_folder_id, position)
                VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    parent_folder_id = excluded.parent_folder_id,
                    position = excluded.position
                "#,
                params![
                    folder.id.get(),
                    folder.name,
                    folder.parent_folder_id.map(PlaylistFolderId::get),
                    i64::from(folder.position),
                ],
            )
            .map(|_| ())
            .map_err(StoreError::from)
    }

    fn playlist_folder(&self, folder_id: PlaylistFolderId) -> StoreResult<Option<PlaylistFolder>> {
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare(
                "SELECT id, name, parent_folder_id, position FROM playlist_folders WHERE id = ?1",
            )
            .map_err(StoreError::from)?;
        let mut rows = statement
            .query(params![folder_id.get()])
            .map_err(StoreError::from)?;

        let Some(row) = rows.next().map_err(StoreError::from)? else {
            return Ok(None);
        };
        Ok(Some(playlist_folder_from_row(row)?))
    }

    fn playlist_folders(&self) -> StoreResult<Vec<PlaylistFolder>> {
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare(
                "SELECT id, name, parent_folder_id, position FROM playlist_folders ORDER BY id",
            )
            .map_err(StoreError::from)?;
        let mut rows = statement.query([]).map_err(StoreError::from)?;
        let mut folders = Vec::new();

        while let Some(row) = rows.next().map_err(StoreError::from)? {
            folders.push(playlist_folder_from_row(row)?);
        }

        Ok(folders)
    }

    fn delete_playlist_folder(&self, folder_id: PlaylistFolderId) -> StoreResult<()> {
        self.connection_guard()?
            .execute(
                "DELETE FROM playlist_folders WHERE id = ?1",
                params![folder_id.get()],
            )
            .map(|_| ())
            .map_err(StoreError::from)
    }

    fn save_smart_playlist(&self, smart_playlist: SmartPlaylist) -> StoreResult<()> {
        let mut connection = self.connection_guard()?;
        let transaction = connection.transaction().map_err(StoreError::from)?;
        let (limit_count, limit_selection) = match smart_playlist.rules.limit {
            Some(limit) => (
                Some(i64::from(limit.count.get())),
                Some(limit_selection_name(limit.selection).to_owned()),
            ),
            None => (None, None),
        };
        transaction
            .execute(
                r#"
                INSERT INTO smart_playlists (
                    id, name, parent_folder_id, position, match_kind, limit_count, limit_selection
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    parent_folder_id = excluded.parent_folder_id,
                    position = excluded.position,
                    match_kind = excluded.match_kind,
                    limit_count = excluded.limit_count,
                    limit_selection = excluded.limit_selection
                "#,
                params![
                    smart_playlist.id.get(),
                    smart_playlist.name,
                    smart_playlist.parent_folder_id.map(PlaylistFolderId::get),
                    i64::from(smart_playlist.position),
                    match_kind_name(smart_playlist.rules.match_kind),
                    limit_count,
                    limit_selection,
                ],
            )
            .map_err(StoreError::from)?;

        transaction
            .execute(
                "DELETE FROM smart_playlist_rules WHERE smart_playlist_id = ?1",
                params![smart_playlist.id.get()],
            )
            .map_err(StoreError::from)?;

        for (position, rule) in smart_playlist.rules.rules.iter().enumerate() {
            let row = rule_to_columns(rule);
            transaction
                .execute(
                    r#"
                    INSERT INTO smart_playlist_rules (
                        smart_playlist_id, position, kind, field, text_operator, text_value,
                        number_operator, number_value, rating_stars, date_unix, days_value
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                    "#,
                    params![
                        smart_playlist.id.get(),
                        position as i64,
                        row.kind,
                        row.field,
                        row.text_operator,
                        row.text_value,
                        row.number_operator,
                        row.number_value,
                        row.rating_stars,
                        row.date_unix,
                        row.days_value,
                    ],
                )
                .map_err(StoreError::from)?;
        }

        transaction.commit().map_err(StoreError::from)
    }

    fn smart_playlist(
        &self,
        smart_playlist_id: SmartPlaylistId,
    ) -> StoreResult<Option<SmartPlaylist>> {
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare(
                r#"
                SELECT id, name, parent_folder_id, position, match_kind, limit_count, limit_selection
                FROM smart_playlists
                WHERE id = ?1
                "#,
            )
            .map_err(StoreError::from)?;
        let mut rows = statement
            .query(params![smart_playlist_id.get()])
            .map_err(StoreError::from)?;

        let Some(row) = rows.next().map_err(StoreError::from)? else {
            return Ok(None);
        };
        let id = smart_playlist_id_from_db(row.get(0).map_err(StoreError::from)?)?;
        let name = row.get(1).map_err(StoreError::from)?;
        let parent_folder_id = optional_playlist_folder_id_from_row(row, 2)?;
        let position = u32_from_row(row, 3)?;
        let match_kind = match_kind_from_name(&row.get::<_, String>(4).map_err(StoreError::from)?)?;
        let limit_count = optional_i64(row, 5)?;
        let limit_selection_name = optional_string(row, 6)?;
        let limit = build_limit(limit_count, limit_selection_name.as_deref())?;
        let rules = load_smart_playlist_rules(&connection, id)?;

        Ok(Some(SmartPlaylist {
            id,
            name,
            parent_folder_id,
            position,
            rules: SmartPlaylistRuleSet {
                match_kind,
                rules,
                limit,
            },
        }))
    }

    fn smart_playlists(&self) -> StoreResult<Vec<SmartPlaylist>> {
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare(
                r#"
                SELECT id, name, parent_folder_id, position, match_kind, limit_count, limit_selection
                FROM smart_playlists
                ORDER BY id
                "#,
            )
            .map_err(StoreError::from)?;
        let mut rows = statement.query([]).map_err(StoreError::from)?;
        let mut smart_playlists = Vec::new();

        while let Some(row) = rows.next().map_err(StoreError::from)? {
            let id = smart_playlist_id_from_db(row.get(0).map_err(StoreError::from)?)?;
            let name = row.get(1).map_err(StoreError::from)?;
            let parent_folder_id = optional_playlist_folder_id_from_row(row, 2)?;
            let position = u32_from_row(row, 3)?;
            let match_kind =
                match_kind_from_name(&row.get::<_, String>(4).map_err(StoreError::from)?)?;
            let limit_count = optional_i64(row, 5)?;
            let limit_selection_name = optional_string(row, 6)?;
            let limit = build_limit(limit_count, limit_selection_name.as_deref())?;
            let rules = load_smart_playlist_rules(&connection, id)?;
            smart_playlists.push(SmartPlaylist {
                id,
                name,
                parent_folder_id,
                position,
                rules: SmartPlaylistRuleSet {
                    match_kind,
                    rules,
                    limit,
                },
            });
        }

        Ok(smart_playlists)
    }

    fn delete_smart_playlist(&self, smart_playlist_id: SmartPlaylistId) -> StoreResult<()> {
        self.connection_guard()?
            .execute(
                "DELETE FROM smart_playlists WHERE id = ?1",
                params![smart_playlist_id.get()],
            )
            .map(|_| ())
            .map_err(StoreError::from)
    }
}

#[derive(Debug, Default)]
pub struct InMemoryLibraryStore {
    tracks: Mutex<BTreeMap<TrackId, Track>>,
    playlists: Mutex<BTreeMap<PlaylistId, Playlist>>,
    folders: Mutex<BTreeMap<PlaylistFolderId, PlaylistFolder>>,
    smart_playlists: Mutex<BTreeMap<SmartPlaylistId, SmartPlaylist>>,
}

fn track_from_row(row: &Row<'_>) -> StoreResult<Track> {
    let duration_seconds = optional_i64(row, 11)?;
    let rating_value = row.get::<_, i64>(13).map_err(StoreError::from)?;

    Ok(Track {
        id: track_id_from_db(row.get(0).map_err(StoreError::from)?)?,
        location: track_location_from_row(row)?,
        metadata: TrackMetadata {
            title: row.get(2).map_err(StoreError::from)?,
            artist: row.get(3).map_err(StoreError::from)?,
            album: row.get(4).map_err(StoreError::from)?,
            album_artist: row.get(5).map_err(StoreError::from)?,
            composer: row.get(6).map_err(StoreError::from)?,
            grouping: row.get(20).map_err(StoreError::from)?,
            genre: row.get(7).map_err(StoreError::from)?,
            track_number: optional_u32(row, 8)?,
            track_total: optional_u32(row, 21)?,
            disc_number: optional_u32(row, 9)?,
            disc_total: optional_u32(row, 22)?,
            year: optional_i64(row, 10)?.map(|value| value as i32),
            compilation: row.get(23).map_err(StoreError::from)?,
            bpm: optional_u32(row, 24)?,
            key: row.get(25).map_err(StoreError::from)?,
            comments: row.get(26).map_err(StoreError::from)?,
            duration: duration_seconds.map(seconds_to_duration),
            bitrate_kbps: optional_u32(row, 12)?,
            sample_rate_hz: optional_u32(row, 27)?,
            channels: optional_u8(row, 28)?,
        },
        rating: Rating::new(rating_value as u8).unwrap_or_else(Rating::unrated),
        statistics: PlayStatistics {
            play_count: row.get::<_, i64>(14).map_err(StoreError::from)? as u64,
            skip_count: row.get::<_, i64>(15).map_err(StoreError::from)? as u64,
            last_played_at: optional_i64(row, 16)?.map(unix_to_system_time),
            last_skipped_at: optional_i64(row, 17)?.map(unix_to_system_time),
            date_added_at: optional_i64(row, 18)?.map(unix_to_system_time),
        },
    })
}

fn track_location_from_row(row: &Row<'_>) -> StoreResult<TrackLocation> {
    let path = row.get::<_, String>(1).map_err(StoreError::from)?;
    let is_missing = row.get::<_, bool>(19).map_err(StoreError::from)?;
    let relative_path =
        TrackRelativePath::new(path.clone()).ok_or(StoreError::InvalidStoredPath(path))?;

    if is_missing {
        Ok(TrackLocation::missing(relative_path))
    } else {
        Ok(TrackLocation::available(relative_path))
    }
}

fn playlist_entries(
    connection: &Connection,
    playlist_id: PlaylistId,
) -> StoreResult<Vec<PlaylistEntry>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT playlist_id, track_id, position
            FROM playlist_entries
            WHERE playlist_id = ?1
            ORDER BY position
            "#,
        )
        .map_err(StoreError::from)?;
    let mut rows = statement
        .query(params![playlist_id.get()])
        .map_err(StoreError::from)?;
    let mut entries = Vec::new();

    while let Some(row) = rows.next().map_err(StoreError::from)? {
        entries.push(PlaylistEntry {
            playlist_id: playlist_id_from_db(row.get(0).map_err(StoreError::from)?)?,
            track_id: track_id_from_db(row.get(1).map_err(StoreError::from)?)?,
            position: row.get::<_, i64>(2).map_err(StoreError::from)? as u32,
        });
    }

    Ok(entries)
}

fn optional_i64(row: &Row<'_>, index: usize) -> StoreResult<Option<i64>> {
    row.get(index).map_err(StoreError::from)
}

fn optional_u32(row: &Row<'_>, index: usize) -> StoreResult<Option<u32>> {
    optional_i64(row, index).map(|value| value.map(|value| value as u32))
}

fn optional_u8(row: &Row<'_>, index: usize) -> StoreResult<Option<u8>> {
    optional_i64(row, index)
        .map(|value| value.map(|value| value.clamp(0, i64::from(u8::MAX)) as u8))
}

fn optional_string(row: &Row<'_>, index: usize) -> StoreResult<Option<String>> {
    row.get(index).map_err(StoreError::from)
}

fn u32_from_row(row: &Row<'_>, index: usize) -> StoreResult<u32> {
    Ok(row.get::<_, i64>(index).map_err(StoreError::from)?.max(0) as u32)
}

fn track_id_from_db(value: i64) -> StoreResult<TrackId> {
    TrackId::new(value).ok_or(StoreError::InvalidStoredId(value))
}

fn playlist_id_from_db(value: i64) -> StoreResult<PlaylistId> {
    PlaylistId::new(value).ok_or(StoreError::InvalidStoredId(value))
}

fn playlist_folder_id_from_db(value: i64) -> StoreResult<PlaylistFolderId> {
    PlaylistFolderId::new(value).ok_or(StoreError::InvalidStoredId(value))
}

fn smart_playlist_id_from_db(value: i64) -> StoreResult<SmartPlaylistId> {
    SmartPlaylistId::new(value).ok_or(StoreError::InvalidStoredId(value))
}

fn optional_playlist_folder_id_from_row(
    row: &Row<'_>,
    index: usize,
) -> StoreResult<Option<PlaylistFolderId>> {
    optional_i64(row, index)?
        .map(playlist_folder_id_from_db)
        .transpose()
}

fn playlist_folder_from_row(row: &Row<'_>) -> StoreResult<PlaylistFolder> {
    Ok(PlaylistFolder {
        id: playlist_folder_id_from_db(row.get(0).map_err(StoreError::from)?)?,
        name: row.get(1).map_err(StoreError::from)?,
        parent_folder_id: optional_playlist_folder_id_from_row(row, 2)?,
        position: u32_from_row(row, 3)?,
    })
}

fn duration_to_seconds(duration: Duration) -> i64 {
    duration.as_secs() as i64
}

fn seconds_to_duration(seconds: i64) -> Duration {
    Duration::from_secs(seconds.max(0) as u64)
}

fn system_time_to_unix(system_time: SystemTime) -> Option<i64> {
    system_time
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs() as i64)
}

fn unix_to_system_time(seconds: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds.max(0) as u64)
}

fn match_kind_name(kind: SmartPlaylistMatchKind) -> &'static str {
    match kind {
        SmartPlaylistMatchKind::All => "All",
        SmartPlaylistMatchKind::Any => "Any",
    }
}

fn match_kind_from_name(name: &str) -> StoreResult<SmartPlaylistMatchKind> {
    match name {
        "All" => Ok(SmartPlaylistMatchKind::All),
        "Any" => Ok(SmartPlaylistMatchKind::Any),
        other => Err(StoreError::InvalidStoredEnum(other.to_owned())),
    }
}

fn limit_selection_name(selection: SmartPlaylistLimitSelection) -> &'static str {
    match selection {
        SmartPlaylistLimitSelection::Random => "Random",
        SmartPlaylistLimitSelection::AlbumAscending => "AlbumAscending",
        SmartPlaylistLimitSelection::ArtistAscending => "ArtistAscending",
        SmartPlaylistLimitSelection::GenreAscending => "GenreAscending",
        SmartPlaylistLimitSelection::TitleAscending => "TitleAscending",
        SmartPlaylistLimitSelection::HighestRating => "HighestRating",
        SmartPlaylistLimitSelection::LowestRating => "LowestRating",
        SmartPlaylistLimitSelection::MostRecentlyPlayed => "MostRecentlyPlayed",
        SmartPlaylistLimitSelection::LeastRecentlyPlayed => "LeastRecentlyPlayed",
        SmartPlaylistLimitSelection::MostOftenPlayed => "MostOftenPlayed",
        SmartPlaylistLimitSelection::LeastOftenPlayed => "LeastOftenPlayed",
        SmartPlaylistLimitSelection::MostRecentlyAdded => "MostRecentlyAdded",
        SmartPlaylistLimitSelection::LeastRecentlyAdded => "LeastRecentlyAdded",
    }
}

fn limit_selection_from_name(name: &str) -> StoreResult<SmartPlaylistLimitSelection> {
    match name {
        "Random" => Ok(SmartPlaylistLimitSelection::Random),
        "AlbumAscending" => Ok(SmartPlaylistLimitSelection::AlbumAscending),
        "ArtistAscending" => Ok(SmartPlaylistLimitSelection::ArtistAscending),
        "GenreAscending" => Ok(SmartPlaylistLimitSelection::GenreAscending),
        "TitleAscending" => Ok(SmartPlaylistLimitSelection::TitleAscending),
        "HighestRating" => Ok(SmartPlaylistLimitSelection::HighestRating),
        "LowestRating" => Ok(SmartPlaylistLimitSelection::LowestRating),
        "MostRecentlyPlayed" => Ok(SmartPlaylistLimitSelection::MostRecentlyPlayed),
        "LeastRecentlyPlayed" => Ok(SmartPlaylistLimitSelection::LeastRecentlyPlayed),
        "MostOftenPlayed" => Ok(SmartPlaylistLimitSelection::MostOftenPlayed),
        "LeastOftenPlayed" => Ok(SmartPlaylistLimitSelection::LeastOftenPlayed),
        "MostRecentlyAdded" => Ok(SmartPlaylistLimitSelection::MostRecentlyAdded),
        "LeastRecentlyAdded" => Ok(SmartPlaylistLimitSelection::LeastRecentlyAdded),
        other => Err(StoreError::InvalidStoredEnum(other.to_owned())),
    }
}

fn build_limit(
    count: Option<i64>,
    selection_name: Option<&str>,
) -> StoreResult<Option<SmartPlaylistLimit>> {
    match (count, selection_name) {
        (Some(count), Some(name)) => {
            let count = u32::try_from(count)
                .ok()
                .and_then(NonZeroU32::new)
                .ok_or_else(|| StoreError::InvalidStoredEnum(format!("limit_count={count}")))?;
            let selection = limit_selection_from_name(name)?;
            Ok(Some(SmartPlaylistLimit { count, selection }))
        }
        (None, None) => Ok(None),
        _ => Err(StoreError::InvalidStoredEnum(
            "limit_count and limit_selection must both be set or both be NULL".to_owned(),
        )),
    }
}

fn text_field_name(field: SmartPlaylistTextField) -> &'static str {
    match field {
        SmartPlaylistTextField::Title => "Title",
        SmartPlaylistTextField::Artist => "Artist",
        SmartPlaylistTextField::Album => "Album",
        SmartPlaylistTextField::AlbumArtist => "AlbumArtist",
        SmartPlaylistTextField::Composer => "Composer",
        SmartPlaylistTextField::Genre => "Genre",
        SmartPlaylistTextField::FileName => "FileName",
    }
}

fn text_field_from_name(name: &str) -> StoreResult<SmartPlaylistTextField> {
    match name {
        "Title" => Ok(SmartPlaylistTextField::Title),
        "Artist" => Ok(SmartPlaylistTextField::Artist),
        "Album" => Ok(SmartPlaylistTextField::Album),
        "AlbumArtist" => Ok(SmartPlaylistTextField::AlbumArtist),
        "Composer" => Ok(SmartPlaylistTextField::Composer),
        "Genre" => Ok(SmartPlaylistTextField::Genre),
        "FileName" => Ok(SmartPlaylistTextField::FileName),
        other => Err(StoreError::InvalidStoredEnum(other.to_owned())),
    }
}

fn text_operator_name(operator: SmartPlaylistTextOperator) -> &'static str {
    match operator {
        SmartPlaylistTextOperator::Contains => "Contains",
        SmartPlaylistTextOperator::DoesNotContain => "DoesNotContain",
        SmartPlaylistTextOperator::Is => "Is",
        SmartPlaylistTextOperator::IsNot => "IsNot",
        SmartPlaylistTextOperator::StartsWith => "StartsWith",
        SmartPlaylistTextOperator::EndsWith => "EndsWith",
    }
}

fn text_operator_from_name(name: &str) -> StoreResult<SmartPlaylistTextOperator> {
    match name {
        "Contains" => Ok(SmartPlaylistTextOperator::Contains),
        "DoesNotContain" => Ok(SmartPlaylistTextOperator::DoesNotContain),
        "Is" => Ok(SmartPlaylistTextOperator::Is),
        "IsNot" => Ok(SmartPlaylistTextOperator::IsNot),
        "StartsWith" => Ok(SmartPlaylistTextOperator::StartsWith),
        "EndsWith" => Ok(SmartPlaylistTextOperator::EndsWith),
        other => Err(StoreError::InvalidStoredEnum(other.to_owned())),
    }
}

fn number_field_name(field: SmartPlaylistNumberField) -> &'static str {
    match field {
        SmartPlaylistNumberField::PlayCount => "PlayCount",
        SmartPlaylistNumberField::SkipCount => "SkipCount",
        SmartPlaylistNumberField::TrackNumber => "TrackNumber",
        SmartPlaylistNumberField::DiscNumber => "DiscNumber",
        SmartPlaylistNumberField::Year => "Year",
        SmartPlaylistNumberField::DurationSeconds => "DurationSeconds",
        SmartPlaylistNumberField::BitrateKbps => "BitrateKbps",
    }
}

fn number_field_from_name(name: &str) -> StoreResult<SmartPlaylistNumberField> {
    match name {
        "PlayCount" => Ok(SmartPlaylistNumberField::PlayCount),
        "SkipCount" => Ok(SmartPlaylistNumberField::SkipCount),
        "TrackNumber" => Ok(SmartPlaylistNumberField::TrackNumber),
        "DiscNumber" => Ok(SmartPlaylistNumberField::DiscNumber),
        "Year" => Ok(SmartPlaylistNumberField::Year),
        "DurationSeconds" => Ok(SmartPlaylistNumberField::DurationSeconds),
        "BitrateKbps" => Ok(SmartPlaylistNumberField::BitrateKbps),
        other => Err(StoreError::InvalidStoredEnum(other.to_owned())),
    }
}

fn number_operator_name(operator: SmartPlaylistNumberOperator) -> &'static str {
    match operator {
        SmartPlaylistNumberOperator::Equal => "Equal",
        SmartPlaylistNumberOperator::NotEqual => "NotEqual",
        SmartPlaylistNumberOperator::GreaterThan => "GreaterThan",
        SmartPlaylistNumberOperator::GreaterThanOrEqual => "GreaterThanOrEqual",
        SmartPlaylistNumberOperator::LessThan => "LessThan",
        SmartPlaylistNumberOperator::LessThanOrEqual => "LessThanOrEqual",
    }
}

fn number_operator_from_name(name: &str) -> StoreResult<SmartPlaylistNumberOperator> {
    match name {
        "Equal" => Ok(SmartPlaylistNumberOperator::Equal),
        "NotEqual" => Ok(SmartPlaylistNumberOperator::NotEqual),
        "GreaterThan" => Ok(SmartPlaylistNumberOperator::GreaterThan),
        "GreaterThanOrEqual" => Ok(SmartPlaylistNumberOperator::GreaterThanOrEqual),
        "LessThan" => Ok(SmartPlaylistNumberOperator::LessThan),
        "LessThanOrEqual" => Ok(SmartPlaylistNumberOperator::LessThanOrEqual),
        other => Err(StoreError::InvalidStoredEnum(other.to_owned())),
    }
}

fn date_field_name(field: SmartPlaylistDateField) -> &'static str {
    match field {
        SmartPlaylistDateField::DateAdded => "DateAdded",
        SmartPlaylistDateField::LastPlayed => "LastPlayed",
        SmartPlaylistDateField::LastSkipped => "LastSkipped",
    }
}

fn date_field_from_name(name: &str) -> StoreResult<SmartPlaylistDateField> {
    match name {
        "DateAdded" => Ok(SmartPlaylistDateField::DateAdded),
        "LastPlayed" => Ok(SmartPlaylistDateField::LastPlayed),
        "LastSkipped" => Ok(SmartPlaylistDateField::LastSkipped),
        other => Err(StoreError::InvalidStoredEnum(other.to_owned())),
    }
}

#[derive(Default)]
struct RuleColumns {
    kind: &'static str,
    field: Option<&'static str>,
    text_operator: Option<&'static str>,
    text_value: Option<String>,
    number_operator: Option<&'static str>,
    number_value: Option<i64>,
    rating_stars: Option<i64>,
    date_unix: Option<i64>,
    days_value: Option<i64>,
}

fn rule_to_columns(rule: &SmartPlaylistRule) -> RuleColumns {
    match rule {
        SmartPlaylistRule::Text {
            field,
            operator,
            value,
        } => RuleColumns {
            kind: "Text",
            field: Some(text_field_name(*field)),
            text_operator: Some(text_operator_name(*operator)),
            text_value: Some(value.clone()),
            ..RuleColumns::default()
        },
        SmartPlaylistRule::TextIsEmpty { field } => RuleColumns {
            kind: "TextIsEmpty",
            field: Some(text_field_name(*field)),
            ..RuleColumns::default()
        },
        SmartPlaylistRule::TextIsPresent { field } => RuleColumns {
            kind: "TextIsPresent",
            field: Some(text_field_name(*field)),
            ..RuleColumns::default()
        },
        SmartPlaylistRule::Number {
            field,
            operator,
            value,
        } => RuleColumns {
            kind: "Number",
            field: Some(number_field_name(*field)),
            number_operator: Some(number_operator_name(*operator)),
            number_value: Some(*value),
            ..RuleColumns::default()
        },
        SmartPlaylistRule::Rating { operator, value } => RuleColumns {
            kind: "Rating",
            number_operator: Some(number_operator_name(*operator)),
            rating_stars: Some(i64::from(value.stars())),
            ..RuleColumns::default()
        },
        SmartPlaylistRule::DateBefore { field, date } => RuleColumns {
            kind: "DateBefore",
            field: Some(date_field_name(*field)),
            date_unix: system_time_to_unix(*date),
            ..RuleColumns::default()
        },
        SmartPlaylistRule::DateAfter { field, date } => RuleColumns {
            kind: "DateAfter",
            field: Some(date_field_name(*field)),
            date_unix: system_time_to_unix(*date),
            ..RuleColumns::default()
        },
        SmartPlaylistRule::DateInLast { field, days } => RuleColumns {
            kind: "DateInLast",
            field: Some(date_field_name(*field)),
            days_value: Some(i64::from(days.get())),
            ..RuleColumns::default()
        },
        SmartPlaylistRule::DateNotInLast { field, days } => RuleColumns {
            kind: "DateNotInLast",
            field: Some(date_field_name(*field)),
            days_value: Some(i64::from(days.get())),
            ..RuleColumns::default()
        },
        SmartPlaylistRule::DateIsEmpty { field } => RuleColumns {
            kind: "DateIsEmpty",
            field: Some(date_field_name(*field)),
            ..RuleColumns::default()
        },
        SmartPlaylistRule::DateIsPresent { field } => RuleColumns {
            kind: "DateIsPresent",
            field: Some(date_field_name(*field)),
            ..RuleColumns::default()
        },
    }
}

fn load_smart_playlist_rules(
    connection: &Connection,
    smart_playlist_id: SmartPlaylistId,
) -> StoreResult<Vec<SmartPlaylistRule>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT kind, field, text_operator, text_value, number_operator, number_value,
                   rating_stars, date_unix, days_value
            FROM smart_playlist_rules
            WHERE smart_playlist_id = ?1
            ORDER BY position
            "#,
        )
        .map_err(StoreError::from)?;
    let mut rows = statement
        .query(params![smart_playlist_id.get()])
        .map_err(StoreError::from)?;
    let mut rules = Vec::new();

    while let Some(row) = rows.next().map_err(StoreError::from)? {
        rules.push(rule_from_row(row)?);
    }

    Ok(rules)
}

fn rule_from_row(row: &Row<'_>) -> StoreResult<SmartPlaylistRule> {
    let kind = row.get::<_, String>(0).map_err(StoreError::from)?;
    let field_name = optional_string(row, 1)?;
    let text_operator_name_value = optional_string(row, 2)?;
    let text_value = optional_string(row, 3)?;
    let number_operator_name_value = optional_string(row, 4)?;
    let number_value = optional_i64(row, 5)?;
    let rating_stars = optional_i64(row, 6)?;
    let date_unix = optional_i64(row, 7)?;
    let days_value = optional_i64(row, 8)?;

    let rule_field_name = || -> StoreResult<&str> {
        field_name
            .as_deref()
            .ok_or_else(|| StoreError::InvalidStoredEnum(format!("{kind} rule missing field")))
    };
    let require_text_operator = || -> StoreResult<SmartPlaylistTextOperator> {
        text_operator_name_value
            .as_deref()
            .ok_or_else(|| {
                StoreError::InvalidStoredEnum(format!("{kind} rule missing text_operator"))
            })
            .and_then(text_operator_from_name)
    };
    let require_text_value = || -> StoreResult<String> {
        text_value
            .clone()
            .ok_or_else(|| StoreError::InvalidStoredEnum(format!("{kind} rule missing text_value")))
    };
    let require_number_operator = || -> StoreResult<SmartPlaylistNumberOperator> {
        number_operator_name_value
            .as_deref()
            .ok_or_else(|| {
                StoreError::InvalidStoredEnum(format!("{kind} rule missing number_operator"))
            })
            .and_then(number_operator_from_name)
    };
    let require_number_value = || -> StoreResult<i64> {
        number_value.ok_or_else(|| {
            StoreError::InvalidStoredEnum(format!("{kind} rule missing number_value"))
        })
    };
    let require_date_unix = || -> StoreResult<SystemTime> {
        date_unix
            .ok_or_else(|| StoreError::InvalidStoredEnum(format!("{kind} rule missing date_unix")))
            .map(unix_to_system_time)
    };
    let require_days_value = || -> StoreResult<NonZeroU32> {
        days_value
            .ok_or_else(|| StoreError::InvalidStoredEnum(format!("{kind} rule missing days_value")))
            .and_then(|days| {
                u32::try_from(days)
                    .ok()
                    .and_then(NonZeroU32::new)
                    .ok_or_else(|| {
                        StoreError::InvalidStoredEnum(format!("{kind} rule days={days}"))
                    })
            })
    };
    let require_rating = || -> StoreResult<Rating> {
        let stars = rating_stars
            .ok_or_else(|| StoreError::InvalidStoredEnum("Rating rule missing stars".to_owned()))?;
        let stars = u8::try_from(stars)
            .ok()
            .and_then(Rating::new)
            .ok_or_else(|| StoreError::InvalidStoredEnum(format!("Rating rule stars={stars}")))?;
        Ok(stars)
    };

    match kind.as_str() {
        "Text" => Ok(SmartPlaylistRule::Text {
            field: text_field_from_name(rule_field_name()?)?,
            operator: require_text_operator()?,
            value: require_text_value()?,
        }),
        "TextIsEmpty" => Ok(SmartPlaylistRule::TextIsEmpty {
            field: text_field_from_name(rule_field_name()?)?,
        }),
        "TextIsPresent" => Ok(SmartPlaylistRule::TextIsPresent {
            field: text_field_from_name(rule_field_name()?)?,
        }),
        "Number" => Ok(SmartPlaylistRule::Number {
            field: number_field_from_name(rule_field_name()?)?,
            operator: require_number_operator()?,
            value: require_number_value()?,
        }),
        "Rating" => Ok(SmartPlaylistRule::Rating {
            operator: require_number_operator()?,
            value: require_rating()?,
        }),
        "DateBefore" => Ok(SmartPlaylistRule::DateBefore {
            field: date_field_from_name(rule_field_name()?)?,
            date: require_date_unix()?,
        }),
        "DateAfter" => Ok(SmartPlaylistRule::DateAfter {
            field: date_field_from_name(rule_field_name()?)?,
            date: require_date_unix()?,
        }),
        "DateInLast" => Ok(SmartPlaylistRule::DateInLast {
            field: date_field_from_name(rule_field_name()?)?,
            days: require_days_value()?,
        }),
        "DateNotInLast" => Ok(SmartPlaylistRule::DateNotInLast {
            field: date_field_from_name(rule_field_name()?)?,
            days: require_days_value()?,
        }),
        "DateIsEmpty" => Ok(SmartPlaylistRule::DateIsEmpty {
            field: date_field_from_name(rule_field_name()?)?,
        }),
        "DateIsPresent" => Ok(SmartPlaylistRule::DateIsPresent {
            field: date_field_from_name(rule_field_name()?)?,
        }),
        other => Err(StoreError::InvalidStoredEnum(other.to_owned())),
    }
}

impl InMemoryLibraryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn tracks_guard(&self) -> StoreResult<MutexGuard<'_, BTreeMap<TrackId, Track>>> {
        self.tracks.lock().map_err(|_| StoreError::StoreUnavailable)
    }

    fn playlists_guard(&self) -> StoreResult<MutexGuard<'_, BTreeMap<PlaylistId, Playlist>>> {
        self.playlists
            .lock()
            .map_err(|_| StoreError::StoreUnavailable)
    }

    fn folders_guard(
        &self,
    ) -> StoreResult<MutexGuard<'_, BTreeMap<PlaylistFolderId, PlaylistFolder>>> {
        self.folders
            .lock()
            .map_err(|_| StoreError::StoreUnavailable)
    }

    fn smart_playlists_guard(
        &self,
    ) -> StoreResult<MutexGuard<'_, BTreeMap<SmartPlaylistId, SmartPlaylist>>> {
        self.smart_playlists
            .lock()
            .map_err(|_| StoreError::StoreUnavailable)
    }
}

impl LibraryStore for InMemoryLibraryStore {
    fn save_track(&self, track: Track) -> StoreResult<()> {
        self.tracks_guard()?.insert(track.id, track);
        Ok(())
    }

    fn delete_track(&self, track_id: TrackId) -> StoreResult<()> {
        let mut tracks = self.tracks_guard()?;
        tracks.remove(&track_id);
        drop(tracks);
        for playlist in self.playlists_guard()?.values_mut() {
            playlist.entries.retain(|entry| entry.track_id != track_id);
        }
        Ok(())
    }

    fn track(&self, track_id: TrackId) -> StoreResult<Option<Track>> {
        Ok(self.tracks_guard()?.get(&track_id).cloned())
    }

    fn tracks(&self) -> StoreResult<Vec<Track>> {
        Ok(self.tracks_guard()?.values().cloned().collect())
    }

    fn save_playlist(&self, playlist: Playlist) -> StoreResult<()> {
        self.playlists_guard()?.insert(playlist.id, playlist);
        Ok(())
    }

    fn playlist(&self, playlist_id: PlaylistId) -> StoreResult<Option<Playlist>> {
        Ok(self.playlists_guard()?.get(&playlist_id).cloned())
    }

    fn playlists(&self) -> StoreResult<Vec<Playlist>> {
        Ok(self.playlists_guard()?.values().cloned().collect())
    }

    fn delete_playlist(&self, playlist_id: PlaylistId) -> StoreResult<()> {
        self.playlists_guard()?.remove(&playlist_id);
        Ok(())
    }

    fn save_playlist_folder(&self, folder: PlaylistFolder) -> StoreResult<()> {
        self.folders_guard()?.insert(folder.id, folder);
        Ok(())
    }

    fn playlist_folder(&self, folder_id: PlaylistFolderId) -> StoreResult<Option<PlaylistFolder>> {
        Ok(self.folders_guard()?.get(&folder_id).cloned())
    }

    fn playlist_folders(&self) -> StoreResult<Vec<PlaylistFolder>> {
        Ok(self.folders_guard()?.values().cloned().collect())
    }

    fn delete_playlist_folder(&self, folder_id: PlaylistFolderId) -> StoreResult<()> {
        let mut deleted = std::collections::BTreeSet::new();
        deleted.insert(folder_id);

        let mut folders = self.folders_guard()?;
        loop {
            let mut grew = false;
            for child_id in folders.keys().copied().collect::<Vec<_>>() {
                if deleted.contains(&child_id) {
                    continue;
                }
                let child = folders.get(&child_id).expect("iterated id exists in map");
                if let Some(parent) = child.parent_folder_id {
                    if deleted.contains(&parent) {
                        deleted.insert(child_id);
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }
        folders.retain(|id, _| !deleted.contains(id));
        drop(folders);

        self.playlists_guard()?
            .retain(|_, playlist| match playlist.parent_folder_id {
                Some(parent) => !deleted.contains(&parent),
                None => true,
            });
        self.smart_playlists_guard()?
            .retain(|_, smart| match smart.parent_folder_id {
                Some(parent) => !deleted.contains(&parent),
                None => true,
            });
        Ok(())
    }

    fn save_smart_playlist(&self, smart_playlist: SmartPlaylist) -> StoreResult<()> {
        self.smart_playlists_guard()?
            .insert(smart_playlist.id, smart_playlist);
        Ok(())
    }

    fn smart_playlist(
        &self,
        smart_playlist_id: SmartPlaylistId,
    ) -> StoreResult<Option<SmartPlaylist>> {
        Ok(self
            .smart_playlists_guard()?
            .get(&smart_playlist_id)
            .cloned())
    }

    fn smart_playlists(&self) -> StoreResult<Vec<SmartPlaylist>> {
        Ok(self.smart_playlists_guard()?.values().cloned().collect())
    }

    fn delete_smart_playlist(&self, smart_playlist_id: SmartPlaylistId) -> StoreResult<()> {
        self.smart_playlists_guard()?.remove(&smart_playlist_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use std::{num::NonZeroU32, time::SystemTime};

    use xtunes_domain::{
        PlayStatistics, PlaylistEntry, Rating, SmartPlaylistDateField, SmartPlaylistLimit,
        SmartPlaylistLimitSelection, SmartPlaylistMatchKind, SmartPlaylistNumberField,
        SmartPlaylistNumberOperator, SmartPlaylistRule, SmartPlaylistRuleSet,
        SmartPlaylistTextField, SmartPlaylistTextOperator, SortDirection, TrackLocation,
        TrackMetadata, TrackRelativePath, TrackSort, TrackSortColumn,
    };

    use super::{
        InMemoryLibraryStore, LibraryQuery, LibraryStore, Playlist, PlaylistFolder,
        PlaylistFolderId, SmartPlaylist, SmartPlaylistId, SqliteLibraryStore, Track,
    };
    use crate::{PlaylistId, StoreResult, TrackId};

    #[test]
    fn in_memory_store_starts_empty() {
        let store = InMemoryLibraryStore::new();

        assert_eq!(store.tracks(), Ok(Vec::new()));
        assert_eq!(store.playlists(), Ok(Vec::new()));
    }

    #[test]
    fn in_memory_store_saves_and_loads_tracks() {
        let store = InMemoryLibraryStore::new();
        let track = track(1, "a.flac");

        assert_eq!(store.save_track(track.clone()), Ok(()));

        assert_eq!(store.track(track.id), Ok(Some(track.clone())));
        assert_eq!(store.tracks(), Ok(vec![track]));
    }

    #[test]
    fn in_memory_store_replaces_tracks_by_id() {
        let store = InMemoryLibraryStore::new();
        let first = track(1, "old.flac");
        let replacement = track(1, "new.flac");

        assert_eq!(store.save_track(first), Ok(()));
        assert_eq!(store.save_track(replacement.clone()), Ok(()));

        assert_eq!(store.track(replacement.id), Ok(Some(replacement)));
    }

    #[test]
    fn in_memory_store_saves_and_loads_playlists() {
        let store = InMemoryLibraryStore::new();
        let playlist = playlist(1, "Favorites", vec![entry(1, 2, 0)]);

        assert_eq!(store.save_playlist(playlist.clone()), Ok(()));

        assert_eq!(store.playlist(playlist.id), Ok(Some(playlist.clone())));
        assert_eq!(store.playlists(), Ok(vec![playlist]));
    }

    #[test]
    fn in_memory_store_deletes_playlists() {
        let store = InMemoryLibraryStore::new();
        let playlist = playlist(1, "Favorites", Vec::new());

        assert_eq!(store.save_playlist(playlist.clone()), Ok(()));
        assert_eq!(store.delete_playlist(playlist.id), Ok(()));

        assert_eq!(store.playlist(playlist.id), Ok(None));
        assert_eq!(store.playlists(), Ok(Vec::new()));
    }

    #[test]
    fn library_query_remains_a_domain_input_type() {
        let query = LibraryQuery::all().sorted_by(TrackSort::default());

        assert_eq!(query, LibraryQuery::default());
    }

    #[test]
    fn sqlite_store_saves_and_loads_tracks() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let mut track = track(1, "a.flac");
        track.metadata.title = Some("Track".to_owned());
        track.metadata.artist = Some("Artist".to_owned());
        track.metadata.bitrate_kbps = Some(1411);
        track.metadata.duration = Some(std::time::Duration::from_secs(245));
        track.rating = Rating::new(4).expect("valid test rating");

        assert_eq!(store.save_track(track.clone()), Ok(()));

        assert_eq!(store.track(track.id), Ok(Some(track.clone())));
        assert_eq!(store.tracks(), Ok(vec![track]));
    }

    #[test]
    fn sqlite_store_preserves_missing_track_location_state() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let mut track = track(1, "missing.flac");
        track.location = TrackLocation::missing(relative_path("missing.flac"));

        assert_eq!(store.save_track(track.clone()), Ok(()));

        assert_eq!(store.track(track.id), Ok(Some(track.clone())));
        assert_eq!(store.tracks(), Ok(vec![track]));
    }

    #[test]
    fn in_memory_store_deletes_tracks_and_clears_playlist_entries() {
        let store = InMemoryLibraryStore::new();
        let first_track = track(1, "a.flac");
        let other_track = track(2, "b.flac");
        let stored_playlist = playlist(1, "Favorites", vec![entry(1, 1, 0), entry(1, 2, 1)]);

        assert_eq!(store.save_track(first_track.clone()), Ok(()));
        assert_eq!(store.save_track(other_track.clone()), Ok(()));
        assert_eq!(store.save_playlist(stored_playlist.clone()), Ok(()));

        assert_eq!(store.delete_track(first_track.id), Ok(()));
        assert_eq!(store.track(first_track.id), Ok(None));
        assert_eq!(store.tracks(), Ok(vec![other_track]));

        let stored = store
            .playlist(stored_playlist.id)
            .expect("playlist loads")
            .expect("playlist exists");
        assert_eq!(stored.entries.len(), 1);
        assert_eq!(stored.entries[0].track_id, track_id(2));
    }

    #[test]
    fn sqlite_store_deletes_tracks_and_cascades_to_playlist_entries() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let first_track = track(1, "a.flac");
        let second_track = track(2, "b.flac");
        let stored_playlist = playlist(1, "Favorites", vec![entry(1, 1, 0), entry(1, 2, 1)]);

        assert_eq!(store.save_track(first_track.clone()), Ok(()));
        assert_eq!(store.save_track(second_track.clone()), Ok(()));
        assert_eq!(store.save_playlist(stored_playlist.clone()), Ok(()));

        assert_eq!(store.delete_track(first_track.id), Ok(()));
        assert_eq!(store.track(first_track.id), Ok(None));
        assert_eq!(store.tracks(), Ok(vec![second_track]));

        let stored = store
            .playlist(stored_playlist.id)
            .expect("playlist loads")
            .expect("playlist exists");
        assert_eq!(stored.entries.len(), 1);
        assert_eq!(stored.entries[0].track_id, track_id(2));
    }

    #[test]
    fn deleting_a_missing_track_is_a_no_op() {
        let store = InMemoryLibraryStore::new();

        assert_eq!(store.delete_track(track_id(42)), Ok(()));
    }

    #[test]
    fn sqlite_store_saves_and_loads_playlists() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let track = track(2, "a.flac");
        let playlist = playlist(1, "Favorites", vec![entry(1, 2, 0)]);

        assert_eq!(store.save_track(track), Ok(()));
        assert_eq!(store.save_playlist(playlist.clone()), Ok(()));

        assert_eq!(store.playlist(playlist.id), Ok(Some(playlist.clone())));
        assert_eq!(store.playlists(), Ok(vec![playlist]));
    }

    #[test]
    fn sqlite_store_deletes_playlists() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let playlist = playlist(1, "Favorites", Vec::new());

        assert_eq!(store.save_playlist(playlist.clone()), Ok(()));
        assert_eq!(store.delete_playlist(playlist.id), Ok(()));

        assert_eq!(store.playlist(playlist.id), Ok(None));
        assert_eq!(store.playlists(), Ok(Vec::new()));
    }

    #[test]
    fn library_query_can_select_tracks_in_playlist_order() {
        let store = InMemoryLibraryStore::new();
        let first = track(1, "first.flac");
        let second = track(2, "second.flac");
        let playlist = playlist(1, "Favorites", vec![entry(1, 2, 0), entry(1, 1, 1)]);

        assert_eq!(store.save_track(first.clone()), Ok(()));
        assert_eq!(store.save_track(second.clone()), Ok(()));
        assert_eq!(store.save_playlist(playlist.clone()), Ok(()));

        assert_eq!(
            store.tracks_matching(LibraryQuery::all().in_playlist(playlist.id)),
            Ok(vec![second, first])
        );
    }

    #[test]
    fn library_query_filters_and_sorts_tracks() {
        let store = InMemoryLibraryStore::new();
        let mut first = track(1, "first.flac");
        first.metadata.title = Some("Beta".to_owned());
        first.metadata.artist = Some("Massive Attack".to_owned());
        let mut second = track(2, "second.flac");
        second.metadata.title = Some("Alpha".to_owned());
        second.metadata.artist = Some("Massive Attack".to_owned());
        let mut third = track(3, "third.flac");
        third.metadata.title = Some("Ignored".to_owned());
        third.metadata.artist = Some("Other".to_owned());

        assert_eq!(store.save_track(first.clone()), Ok(()));
        assert_eq!(store.save_track(second.clone()), Ok(()));
        assert_eq!(store.save_track(third), Ok(()));

        let query = LibraryQuery::all()
            .with_search_text("massive")
            .sorted_by(TrackSort {
                column: TrackSortColumn::Title,
                direction: SortDirection::Ascending,
            });

        assert_eq!(store.tracks_matching(query), Ok(vec![second, first]));
    }

    #[test]
    fn sqlite_store_persists_playlist_folder_membership_and_position() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let folder = folder(1, "Mixes", None, 0);
        let mut stored_playlist = playlist(1, "Favorites", Vec::new());
        stored_playlist.parent_folder_id = Some(folder.id);
        stored_playlist.position = 3;

        assert_eq!(store.save_playlist_folder(folder.clone()), Ok(()));
        assert_eq!(store.save_playlist(stored_playlist.clone()), Ok(()));

        let loaded = store
            .playlist(stored_playlist.id)
            .expect("load succeeds")
            .expect("playlist exists");
        assert_eq!(loaded.parent_folder_id, Some(folder.id));
        assert_eq!(loaded.position, 3);
    }

    #[test]
    fn in_memory_store_saves_and_loads_folders() {
        let store = InMemoryLibraryStore::new();
        let folder = folder(1, "Mixes", None, 0);

        assert_eq!(store.save_playlist_folder(folder.clone()), Ok(()));

        assert_eq!(store.playlist_folder(folder.id), Ok(Some(folder.clone())));
        assert_eq!(store.playlist_folders(), Ok(vec![folder]));
    }

    #[test]
    fn sqlite_store_saves_and_loads_folders() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let folder = folder(1, "Mixes", None, 2);

        assert_eq!(store.save_playlist_folder(folder.clone()), Ok(()));

        assert_eq!(store.playlist_folder(folder.id), Ok(Some(folder.clone())));
        assert_eq!(store.playlist_folders(), Ok(vec![folder]));
    }

    #[test]
    fn sqlite_store_persists_nested_folder_parent() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let parent = folder(1, "Mixes", None, 0);
        let child = folder(2, "Long Drives", Some(parent.id), 0);

        assert_eq!(store.save_playlist_folder(parent.clone()), Ok(()));
        assert_eq!(store.save_playlist_folder(child.clone()), Ok(()));

        let loaded = store
            .playlist_folder(child.id)
            .expect("load succeeds")
            .expect("child exists");
        assert_eq!(loaded.parent_folder_id, Some(parent.id));
    }

    #[test]
    fn sqlite_store_cascade_deletes_folder_and_contents() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let folder = folder(1, "Mixes", None, 0);
        let child_folder = folder_with_id(2, "Long Drives", Some(folder.id), 0);
        let mut child_playlist = playlist(1, "Late Night", Vec::new());
        child_playlist.parent_folder_id = Some(folder.id);
        let child_smart = smart_playlist_with_rules(
            1,
            "Recently Added",
            Some(folder.id),
            0,
            SmartPlaylistRuleSet {
                match_kind: SmartPlaylistMatchKind::All,
                rules: vec![SmartPlaylistRule::DateInLast {
                    field: SmartPlaylistDateField::DateAdded,
                    days: NonZeroU32::new(7).unwrap(),
                }],
                limit: None,
            },
        );

        assert_eq!(store.save_playlist_folder(folder.clone()), Ok(()));
        assert_eq!(store.save_playlist_folder(child_folder.clone()), Ok(()));
        assert_eq!(store.save_playlist(child_playlist.clone()), Ok(()));
        assert_eq!(store.save_smart_playlist(child_smart.clone()), Ok(()));

        assert_eq!(store.delete_playlist_folder(folder.id), Ok(()));

        assert_eq!(store.playlist_folder(folder.id), Ok(None));
        assert_eq!(store.playlist_folder(child_folder.id), Ok(None));
        assert_eq!(store.playlist(child_playlist.id), Ok(None));
        assert_eq!(store.smart_playlist(child_smart.id), Ok(None));
    }

    #[test]
    fn in_memory_store_cascade_deletes_folder_and_contents() {
        let store = InMemoryLibraryStore::new();
        let folder = folder(1, "Mixes", None, 0);
        let child_folder = folder_with_id(2, "Long Drives", Some(folder.id), 0);
        let mut child_playlist = playlist(1, "Late Night", Vec::new());
        child_playlist.parent_folder_id = Some(folder.id);
        let child_smart =
            smart_playlist_with_rules(1, "Recent", Some(folder.id), 0, simple_text_rule_set());

        assert_eq!(store.save_playlist_folder(folder.clone()), Ok(()));
        assert_eq!(store.save_playlist_folder(child_folder.clone()), Ok(()));
        assert_eq!(store.save_playlist(child_playlist.clone()), Ok(()));
        assert_eq!(store.save_smart_playlist(child_smart.clone()), Ok(()));

        assert_eq!(store.delete_playlist_folder(folder.id), Ok(()));

        assert_eq!(store.playlist_folder(folder.id), Ok(None));
        assert_eq!(store.playlist_folder(child_folder.id), Ok(None));
        assert_eq!(store.playlist(child_playlist.id), Ok(None));
        assert_eq!(store.smart_playlist(child_smart.id), Ok(None));
    }

    #[test]
    fn in_memory_store_saves_and_loads_smart_playlists() {
        let store = InMemoryLibraryStore::new();
        let smart = smart_playlist_with_rules(1, "Recent", None, 0, simple_text_rule_set());

        assert_eq!(store.save_smart_playlist(smart.clone()), Ok(()));

        assert_eq!(store.smart_playlist(smart.id), Ok(Some(smart.clone())));
        assert_eq!(store.smart_playlists(), Ok(vec![smart]));
    }

    #[test]
    fn sqlite_store_round_trips_every_rule_variant() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let smart = smart_playlist_with_rules(
            1,
            "Variants",
            None,
            0,
            SmartPlaylistRuleSet {
                match_kind: SmartPlaylistMatchKind::Any,
                limit: None,
                rules: vec![
                    SmartPlaylistRule::Text {
                        field: SmartPlaylistTextField::Artist,
                        operator: SmartPlaylistTextOperator::Contains,
                        value: "Massive Attack".to_owned(),
                    },
                    SmartPlaylistRule::TextIsEmpty {
                        field: SmartPlaylistTextField::Composer,
                    },
                    SmartPlaylistRule::TextIsPresent {
                        field: SmartPlaylistTextField::Album,
                    },
                    SmartPlaylistRule::Number {
                        field: SmartPlaylistNumberField::PlayCount,
                        operator: SmartPlaylistNumberOperator::GreaterThan,
                        value: 5,
                    },
                    SmartPlaylistRule::Rating {
                        operator: SmartPlaylistNumberOperator::GreaterThanOrEqual,
                        value: Rating::new(4).unwrap(),
                    },
                    SmartPlaylistRule::DateBefore {
                        field: SmartPlaylistDateField::LastPlayed,
                        date: SystemTime::UNIX_EPOCH
                            + std::time::Duration::from_secs(1_700_000_000),
                    },
                    SmartPlaylistRule::DateAfter {
                        field: SmartPlaylistDateField::DateAdded,
                        date: SystemTime::UNIX_EPOCH
                            + std::time::Duration::from_secs(1_600_000_000),
                    },
                    SmartPlaylistRule::DateInLast {
                        field: SmartPlaylistDateField::LastPlayed,
                        days: NonZeroU32::new(30).unwrap(),
                    },
                    SmartPlaylistRule::DateNotInLast {
                        field: SmartPlaylistDateField::LastSkipped,
                        days: NonZeroU32::new(90).unwrap(),
                    },
                    SmartPlaylistRule::DateIsEmpty {
                        field: SmartPlaylistDateField::LastPlayed,
                    },
                    SmartPlaylistRule::DateIsPresent {
                        field: SmartPlaylistDateField::DateAdded,
                    },
                ],
            },
        );

        assert_eq!(store.save_smart_playlist(smart.clone()), Ok(()));
        assert_eq!(store.smart_playlist(smart.id), Ok(Some(smart)));
    }

    #[test]
    fn sqlite_store_persists_rule_order() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let rules = vec![
            SmartPlaylistRule::Text {
                field: SmartPlaylistTextField::Genre,
                operator: SmartPlaylistTextOperator::Is,
                value: "Trip-Hop".to_owned(),
            },
            SmartPlaylistRule::Number {
                field: SmartPlaylistNumberField::Year,
                operator: SmartPlaylistNumberOperator::GreaterThanOrEqual,
                value: 1995,
            },
            SmartPlaylistRule::Rating {
                operator: SmartPlaylistNumberOperator::Equal,
                value: Rating::new(5).unwrap(),
            },
        ];
        let smart = smart_playlist_with_rules(
            1,
            "Mix",
            None,
            0,
            SmartPlaylistRuleSet {
                match_kind: SmartPlaylistMatchKind::All,
                limit: None,
                rules: rules.clone(),
            },
        );

        assert_eq!(store.save_smart_playlist(smart.clone()), Ok(()));
        let loaded = store
            .smart_playlist(smart.id)
            .expect("load succeeds")
            .expect("exists");
        assert_eq!(loaded.rules.rules, rules);
    }

    #[test]
    fn sqlite_store_persists_smart_playlist_limit() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let smart = smart_playlist_with_rules(
            1,
            "Top 25",
            None,
            0,
            SmartPlaylistRuleSet {
                match_kind: SmartPlaylistMatchKind::All,
                limit: Some(SmartPlaylistLimit {
                    count: NonZeroU32::new(25).unwrap(),
                    selection: SmartPlaylistLimitSelection::MostRecentlyAdded,
                }),
                rules: vec![SmartPlaylistRule::Rating {
                    operator: SmartPlaylistNumberOperator::GreaterThanOrEqual,
                    value: Rating::new(4).unwrap(),
                }],
            },
        );

        assert_eq!(store.save_smart_playlist(smart.clone()), Ok(()));
        assert_eq!(store.smart_playlist(smart.id), Ok(Some(smart)));
    }

    #[test]
    fn sqlite_store_persists_match_kind_any() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let smart = smart_playlist_with_rules(
            1,
            "Either Or",
            None,
            0,
            SmartPlaylistRuleSet {
                match_kind: SmartPlaylistMatchKind::Any,
                limit: None,
                rules: vec![SmartPlaylistRule::Text {
                    field: SmartPlaylistTextField::Artist,
                    operator: SmartPlaylistTextOperator::Is,
                    value: "Portishead".to_owned(),
                }],
            },
        );

        assert_eq!(store.save_smart_playlist(smart.clone()), Ok(()));
        let loaded = store
            .smart_playlist(smart.id)
            .expect("load succeeds")
            .expect("exists");
        assert_eq!(loaded.rules.match_kind, SmartPlaylistMatchKind::Any);
    }

    #[test]
    fn sqlite_store_cascade_deletes_rules_when_smart_playlist_deleted() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let smart = smart_playlist_with_rules(1, "Recent", None, 0, simple_text_rule_set());

        assert_eq!(store.save_smart_playlist(smart.clone()), Ok(()));
        assert_eq!(store.delete_smart_playlist(smart.id), Ok(()));
        assert_eq!(store.smart_playlist(smart.id), Ok(None));

        let resaved = smart_playlist_with_rules(1, "Recent", None, 0, simple_text_rule_set());
        assert_eq!(store.save_smart_playlist(resaved.clone()), Ok(()));
        let loaded = store
            .smart_playlist(resaved.id)
            .expect("load succeeds")
            .expect("exists");
        assert_eq!(loaded.rules.rules.len(), resaved.rules.rules.len());
    }

    fn track(id: i64, path: &str) -> Track {
        Track {
            id: track_id(id),
            location: TrackLocation::available(relative_path(path)),
            metadata: TrackMetadata::default(),
            rating: Rating::unrated(),
            statistics: PlayStatistics::default(),
        }
    }

    fn relative_path(path: &str) -> TrackRelativePath {
        TrackRelativePath::new(PathBuf::from(path)).expect("test path is relative")
    }

    fn playlist(id: i64, name: &str, entries: Vec<PlaylistEntry>) -> Playlist {
        Playlist {
            id: playlist_id(id),
            name: name.to_owned(),
            parent_folder_id: None,
            position: 0,
            entries,
        }
    }

    fn entry(playlist_id_value: i64, track_id_value: i64, position: u32) -> PlaylistEntry {
        PlaylistEntry {
            playlist_id: playlist_id(playlist_id_value),
            track_id: track_id(track_id_value),
            position,
        }
    }

    fn track_id(value: i64) -> TrackId {
        positive_id(TrackId::new(value))
    }

    fn playlist_id(value: i64) -> PlaylistId {
        positive_id(PlaylistId::new(value))
    }

    fn folder_id(value: i64) -> PlaylistFolderId {
        positive_id(PlaylistFolderId::new(value))
    }

    fn smart_id(value: i64) -> SmartPlaylistId {
        positive_id(SmartPlaylistId::new(value))
    }

    fn folder(
        id: i64,
        name: &str,
        parent_folder_id: Option<PlaylistFolderId>,
        position: u32,
    ) -> PlaylistFolder {
        PlaylistFolder {
            id: folder_id(id),
            name: name.to_owned(),
            parent_folder_id,
            position,
        }
    }

    fn folder_with_id(
        id: i64,
        name: &str,
        parent_folder_id: Option<PlaylistFolderId>,
        position: u32,
    ) -> PlaylistFolder {
        folder(id, name, parent_folder_id, position)
    }

    fn smart_playlist_with_rules(
        id: i64,
        name: &str,
        parent_folder_id: Option<PlaylistFolderId>,
        position: u32,
        rules: SmartPlaylistRuleSet,
    ) -> SmartPlaylist {
        SmartPlaylist {
            id: smart_id(id),
            name: name.to_owned(),
            parent_folder_id,
            position,
            rules,
        }
    }

    fn simple_text_rule_set() -> SmartPlaylistRuleSet {
        SmartPlaylistRuleSet {
            match_kind: SmartPlaylistMatchKind::All,
            rules: vec![SmartPlaylistRule::Text {
                field: SmartPlaylistTextField::Artist,
                operator: SmartPlaylistTextOperator::Contains,
                value: "Portishead".to_owned(),
            }],
            limit: None,
        }
    }

    fn positive_id<T>(id: Option<T>) -> T {
        match id {
            Some(id) => id,
            None => unreachable!("test helper only constructs positive ids"),
        }
    }

    fn _assert_store_result_is_public<T>(result: StoreResult<T>) -> StoreResult<T> {
        result
    }
}
