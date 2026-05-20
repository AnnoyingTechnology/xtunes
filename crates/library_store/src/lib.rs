#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Mutex, MutexGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, Row, params};
pub use xtunes_domain::{LibraryQuery, Playlist, PlaylistId, Rating, Track, TrackId};
use xtunes_domain::{PlayStatistics, PlaylistEntry, TrackLocation, TrackMetadata};

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreError {
    Database(String),
    InvalidStoredId(i64),
    StoreUnavailable,
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error.to_string())
    }
}

pub trait LibraryStore: Send + Sync {
    fn save_track(&self, track: Track) -> StoreResult<()>;
    fn track(&self, track_id: TrackId) -> StoreResult<Option<Track>>;
    fn tracks(&self) -> StoreResult<Vec<Track>>;
    fn save_playlist(&self, playlist: Playlist) -> StoreResult<()>;
    fn playlist(&self, playlist_id: PlaylistId) -> StoreResult<Option<Playlist>>;
    fn playlists(&self) -> StoreResult<Vec<Playlist>>;
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

    fn migrate(&self) -> StoreResult<()> {
        self.connection_guard()?
            .execute_batch(
                r#"
                PRAGMA foreign_keys = ON;

                CREATE TABLE IF NOT EXISTS tracks (
                    id INTEGER PRIMARY KEY,
                    path TEXT NOT NULL UNIQUE,
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
                    is_missing INTEGER NOT NULL DEFAULT 0
                );

                CREATE TABLE IF NOT EXISTS playlists (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS playlist_entries (
                    playlist_id INTEGER NOT NULL,
                    track_id INTEGER NOT NULL,
                    position INTEGER NOT NULL,
                    PRIMARY KEY (playlist_id, track_id),
                    FOREIGN KEY (playlist_id) REFERENCES playlists(id) ON DELETE CASCADE,
                    FOREIGN KEY (track_id) REFERENCES tracks(id) ON DELETE CASCADE
                );
                "#,
            )
            .map_err(StoreError::from)?;
        self.add_column_if_missing("tracks", "is_missing", "INTEGER NOT NULL DEFAULT 0")
    }

    fn add_column_if_missing(
        &self,
        table: &str,
        column: &str,
        definition: &str,
    ) -> StoreResult<()> {
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(StoreError::from)?;
        let mut rows = statement.query([]).map_err(StoreError::from)?;

        while let Some(row) = rows.next().map_err(StoreError::from)? {
            let existing_column: String = row.get(1).map_err(StoreError::from)?;
            if existing_column == column {
                return Ok(());
            }
        }

        connection
            .execute(
                &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
                [],
            )
            .map(|_| ())
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

impl LibraryStore for SqliteLibraryStore {
    fn save_track(&self, track: Track) -> StoreResult<()> {
        let metadata = track.metadata;
        let statistics = track.statistics;
        self.connection_guard()?
            .execute(
                r#"
                INSERT INTO tracks (
                    id,
                    path,
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
                    is_missing
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
                ON CONFLICT(id) DO UPDATE SET
                    path = excluded.path,
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
                    is_missing = excluded.is_missing
                "#,
                params![
                    track.id.get(),
                    track.location.path.to_string_lossy(),
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
                    track.location.is_missing(),
                ],
            )
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
                    path,
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
                    is_missing
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
                    path,
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
                    is_missing
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
                INSERT INTO playlists (id, name)
                VALUES (?1, ?2)
                ON CONFLICT(id) DO UPDATE SET name = excluded.name
                "#,
                params![playlist.id.get(), playlist.name],
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
            .prepare("SELECT id, name FROM playlists WHERE id = ?1")
            .map_err(StoreError::from)?;
        let mut rows = statement
            .query(params![playlist_id.get()])
            .map_err(StoreError::from)?;

        let Some(row) = rows.next().map_err(StoreError::from)? else {
            return Ok(None);
        };
        let id = playlist_id_from_db(row.get(0).map_err(StoreError::from)?)?;
        let name = row.get(1).map_err(StoreError::from)?;
        let entries = playlist_entries(&connection, id)?;

        Ok(Some(Playlist { id, name, entries }))
    }

    fn playlists(&self) -> StoreResult<Vec<Playlist>> {
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare("SELECT id, name FROM playlists ORDER BY id")
            .map_err(StoreError::from)?;
        let mut rows = statement.query([]).map_err(StoreError::from)?;
        let mut playlists = Vec::new();

        while let Some(row) = rows.next().map_err(StoreError::from)? {
            let id = playlist_id_from_db(row.get(0).map_err(StoreError::from)?)?;
            let name = row.get(1).map_err(StoreError::from)?;
            playlists.push(Playlist {
                id,
                name,
                entries: playlist_entries(&connection, id)?,
            });
        }

        Ok(playlists)
    }
}

#[derive(Debug, Default)]
pub struct InMemoryLibraryStore {
    tracks: Mutex<BTreeMap<TrackId, Track>>,
    playlists: Mutex<BTreeMap<PlaylistId, Playlist>>,
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
            genre: row.get(7).map_err(StoreError::from)?,
            track_number: optional_u32(row, 8)?,
            disc_number: optional_u32(row, 9)?,
            year: optional_i64(row, 10)?.map(|value| value as i32),
            duration: duration_seconds.map(seconds_to_duration),
            bitrate_kbps: optional_u32(row, 12)?,
        },
        rating: Rating::new(rating_value as u8).unwrap_or_else(Rating::unrated),
        statistics: PlayStatistics {
            play_count: row.get::<_, i64>(14).map_err(StoreError::from)? as u64,
            skip_count: row.get::<_, i64>(15).map_err(StoreError::from)? as u64,
            last_played_at: optional_i64(row, 16)?.map(unix_to_system_time),
            last_skipped_at: optional_i64(row, 17)?.map(unix_to_system_time),
        },
    })
}

fn track_location_from_row(row: &Row<'_>) -> StoreResult<TrackLocation> {
    let path = row.get::<_, String>(1).map_err(StoreError::from)?;
    let is_missing = row.get::<_, bool>(18).map_err(StoreError::from)?;

    if is_missing {
        Ok(TrackLocation::missing(path))
    } else {
        Ok(TrackLocation::new(path))
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

fn track_id_from_db(value: i64) -> StoreResult<TrackId> {
    TrackId::new(value).ok_or(StoreError::InvalidStoredId(value))
}

fn playlist_id_from_db(value: i64) -> StoreResult<PlaylistId> {
    PlaylistId::new(value).ok_or(StoreError::InvalidStoredId(value))
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
}

impl LibraryStore for InMemoryLibraryStore {
    fn save_track(&self, track: Track) -> StoreResult<()> {
        self.tracks_guard()?.insert(track.id, track);
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
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use xtunes_domain::{
        PlayStatistics, PlaylistEntry, Rating, TrackLocation, TrackMetadata, TrackSort,
    };

    use super::{
        InMemoryLibraryStore, LibraryQuery, LibraryStore, Playlist, SqliteLibraryStore, Track,
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
        let track = track(1, "/music/a.flac");

        assert_eq!(store.save_track(track.clone()), Ok(()));

        assert_eq!(store.track(track.id), Ok(Some(track.clone())));
        assert_eq!(store.tracks(), Ok(vec![track]));
    }

    #[test]
    fn in_memory_store_replaces_tracks_by_id() {
        let store = InMemoryLibraryStore::new();
        let first = track(1, "/music/old.flac");
        let replacement = track(1, "/music/new.flac");

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
    fn library_query_remains_a_domain_input_type() {
        let query = LibraryQuery::all().sorted_by(TrackSort::default());

        assert_eq!(query, LibraryQuery::default());
    }

    #[test]
    fn sqlite_store_saves_and_loads_tracks() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let mut track = track(1, "/music/a.flac");
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
        let mut track = track(1, "/music/missing.flac");
        track.location = TrackLocation::missing("/music/missing.flac");

        assert_eq!(store.save_track(track.clone()), Ok(()));

        assert_eq!(store.track(track.id), Ok(Some(track.clone())));
        assert_eq!(store.tracks(), Ok(vec![track]));
    }

    #[test]
    fn sqlite_store_saves_and_loads_playlists() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let track = track(2, "/music/a.flac");
        let playlist = playlist(1, "Favorites", vec![entry(1, 2, 0)]);

        assert_eq!(store.save_track(track), Ok(()));
        assert_eq!(store.save_playlist(playlist.clone()), Ok(()));

        assert_eq!(store.playlist(playlist.id), Ok(Some(playlist.clone())));
        assert_eq!(store.playlists(), Ok(vec![playlist]));
    }

    fn track(id: i64, path: &str) -> Track {
        Track {
            id: track_id(id),
            location: TrackLocation::new(PathBuf::from(path)),
            metadata: TrackMetadata::default(),
            rating: Rating::unrated(),
            statistics: PlayStatistics::default(),
        }
    }

    fn playlist(id: i64, name: &str, entries: Vec<PlaylistEntry>) -> Playlist {
        Playlist {
            id: playlist_id(id),
            name: name.to_owned(),
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
