// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::Path,
    sync::{Mutex, MutexGuard},
};

use rusqlite::{Connection, OptionalExtension, params, params_from_iter, types::Value as SqlValue};
use sustain_domain::SmartPlaylistRuleSet;
pub use sustain_domain::{
    AcousticFeatures, LibraryQuery, Playlist, PlaylistFolder, PlaylistFolderId, PlaylistId, Rating,
    SmartPlaylist, SmartPlaylistId, SyncedLyrics, Track, TrackAnalysis, TrackColumnEntry,
    TrackColumnLayout, TrackColumnLayoutScope, TrackId, WaveformSegments,
};

mod memory;
mod query;
mod schema;
mod sqlite_rows;

pub use memory::InMemoryLibraryStore;

use query::{sort_tracks, track_matches_search};
use schema::{
    DELETE_SMART_SHUFFLE_INDEX_SQL, DELETE_TRACK_SYNCED_LYRICS_SQL, FILL_TRACK_BPM_IF_NULL_SQL,
    FILL_TRACK_MUSICAL_KEY_IF_NULL_SQL, SAVE_TRACK_SQL, SCHEMA_SQL, SELECT_ALL_TRACK_ACOUSTICS_SQL,
    SELECT_ALL_TRACKS_SQL, SELECT_SMART_SHUFFLE_INDEX_SQL, SELECT_TRACK_BY_CONTENT_HASH_SQL,
    SELECT_TRACK_BY_ID_SQL, SELECT_TRACK_SYNCED_LYRICS_SQL, SELECT_TRACK_WAVEFORM_SQL,
    SELECT_TRACKS_NEEDING_ANALYSIS_SQL, SELECT_TRACKS_NEEDING_ONLINE_SQL,
    UPSERT_SMART_SHUFFLE_INDEX_SQL, UPSERT_TRACK_ACOUSTICS_SQL, UPSERT_TRACK_ANALYSIS_SQL,
    UPSERT_TRACK_ONLINE_STATUS_SQL, UPSERT_TRACK_SYNCED_LYRICS_SQL, UPSERT_TRACK_WAVEFORM_SQL,
};
use sqlite_rows::{
    blob_to_waveform_segments, build_limit, duration_to_seconds, limit_selection_name,
    load_smart_playlist_rules, match_kind_from_name, match_kind_name, optional_i64,
    optional_playlist_folder_id_from_row, optional_string, playlist_entries,
    playlist_folder_from_row, playlist_id_from_db, rule_to_columns, smart_playlist_id_from_db,
    system_time_to_unix, track_from_row, u32_from_row, waveform_segments_to_blob,
};

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreError {
    Database(String),
    InvalidStoredId(i64),
    InvalidStoredHash(String),
    InvalidStoredPath(String),
    InvalidStoredEnum(String),
    StoreUnavailable,
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error.to_string())
    }
}

/// Which DSP passes a write or query applies to.
///
/// On the write path (`record_analysis`, `record_analysis_attempt_failure`)
/// only the requested capabilities get their `*_attempted_at_unix`
/// timestamps stamped; the others preserve whatever value was already
/// stored. On the read path (`tracks_needing_analysis`) a track
/// qualifies as needing analysis if **any** of the requested
/// capabilities is either un-attempted (NULL) or stamped by an older
/// `analyzer_version`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AnalysisCapabilities {
    pub bpm: bool,
    pub key: bool,
    /// The single heavy full-decode pass. One decode produces both the
    /// waveform overview and the perceptual acoustic features (loudness,
    /// onset density, timbre) Smart Shuffle consumes; the waveform and the
    /// acoustics are byproducts of the same work, so they share one
    /// attempt timestamp and one opt-in toggle.
    pub audio: bool,
}

impl AnalysisCapabilities {
    /// No capabilities requested. Useful as a sentinel; passing this
    /// to a write call is a no-op.
    pub const fn none() -> Self {
        Self {
            bpm: false,
            key: false,
            audio: false,
        }
    }

    /// All capabilities requested.
    pub const fn all() -> Self {
        Self {
            bpm: true,
            key: true,
            audio: true,
        }
    }

    pub const fn is_empty(self) -> bool {
        !(self.bpm || self.key || self.audio)
    }
}

/// Which network-bound retrievals a write or query applies to.
///
/// Symmetric counterpart of [`AnalysisCapabilities`] for online work
/// (artwork, tag enrichment, lyrics). Tag-fetch is included in the
/// flag set but is not yet wired through the runtime — the
/// scheduler ignores it today and the field exists so the storage
/// layer's row shape does not have to change when tag-fetch lands.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OnlineCapabilities {
    pub artwork: bool,
    pub tags: bool,
    pub lyrics: bool,
}

impl OnlineCapabilities {
    pub const fn none() -> Self {
        Self {
            artwork: false,
            tags: false,
            lyrics: false,
        }
    }

    pub const fn all() -> Self {
        Self {
            artwork: true,
            tags: true,
            lyrics: true,
        }
    }

    pub const fn is_empty(self) -> bool {
        !(self.artwork || self.tags || self.lyrics)
    }
}

/// Per-attempt facts stamped onto `track_online_status`. `provider_version`
/// is the watermark the scheduler compares against to invalidate stale
/// rows after a provider switch or significant client change; `now_unix`
/// is the wall clock at the moment the attempt completed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OnlineContext {
    pub provider_version: u32,
    pub now_unix: i64,
}

/// Both waveform tiers as returned by [`LibraryStore::load_waveform`].
/// Cheap to clone (`Vec<u8>` of segment bytes, decoded once into
/// `WaveformSegment` on read). Renderers typically request this lazily
/// — only for the active track — so the BLOB pages are touched on
/// demand rather than during library load.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredWaveform {
    pub preview: WaveformSegments,
    pub detail: WaveformSegments,
}

/// Per-attempt facts the storage layer stamps onto `track_analysis`
/// alongside the capability timestamps. `analyzer_version` is the
/// watermark the scheduler compares against to invalidate stale rows
/// after a DSP change; `now_unix` is the wall clock at the moment the
/// attempt completed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnalysisContext {
    pub analyzer_version: u32,
    pub now_unix: i64,
}

/// Synced lyrics as returned by [`LibraryStore::load_synced_lyrics`].
/// `source` is the short provider identifier under which the lines were
/// originally written (e.g. `"lrclib"`); a future diagnostic UI can
/// surface it without reaching back into logs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSyncedLyrics {
    pub lyrics: SyncedLyrics,
    pub source: String,
}

/// Persisted Smart Shuffle index — the background rebuild writes one
/// of these into the `smart_shuffle_index` table, and the runtime
/// reads it back at startup. There is no trained model; the blob holds
/// the prepared, library-dependent state (genre IDF and, later,
/// normalization statistics) defined by `sustain_smart_shuffle`.
/// `schema_version` is broken out of the blob so the runtime can
/// discard a stale-shaped blob without paying to deserialise it first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSmartShuffleIndex {
    pub index_blob: Vec<u8>,
    pub schema_version: u32,
}

pub trait LibraryStore: Send + Sync {
    fn save_track(&self, track: Track) -> StoreResult<()>;
    fn save_tracks(&self, tracks: &[Track]) -> StoreResult<()> {
        for track in tracks {
            self.save_track(track.clone())?;
        }
        Ok(())
    }
    fn delete_track(&self, track_id: TrackId) -> StoreResult<()>;
    fn track(&self, track_id: TrackId) -> StoreResult<Option<Track>>;
    fn track_by_content_hash(
        &self,
        content_hash: &sustain_domain::TrackContentHash,
    ) -> StoreResult<Option<Track>>;
    fn tracks(&self) -> StoreResult<Vec<Track>>;

    /// Return the set of distinct, non-empty genre values currently
    /// stored on tracks in the library. Order is not specified; the
    /// caller normalizes for whatever comparison it needs. Used by
    /// the online tag-enrichment path to bias its genre choice
    /// toward genres the user already curates, avoiding genre
    /// sprawl (e.g. picking "House" over "Electronica" when the
    /// library already has House tracks). Default implementation
    /// scans `tracks()` for crates that don't have a cheaper
    /// projection.
    fn distinct_genres(&self) -> StoreResult<Vec<String>> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for track in self.tracks()? {
            if let Some(genre) = track.metadata.genre
                && !genre.trim().is_empty()
            {
                seen.insert(genre);
            }
        }
        Ok(seen.into_iter().collect())
    }
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
    fn load_track_column_layout(
        &self,
        scope: TrackColumnLayoutScope,
    ) -> StoreResult<Option<TrackColumnLayout>>;
    fn save_track_column_layout(
        &self,
        scope: TrackColumnLayoutScope,
        layout: &TrackColumnLayout,
    ) -> StoreResult<()>;
    fn delete_track_column_layout(&self, scope: TrackColumnLayoutScope) -> StoreResult<()>;

    /// Persist a successful analysis pass. Stamps each requested
    /// capability's `*_attempted_at_unix` timestamp regardless of
    /// whether the corresponding field in `analysis` is `Some`
    /// (a successful run that produced no result is still an
    /// attempt, and we want the scheduler to stop retrying it).
    ///
    /// Writes the waveform BLOBs only when the waveform capability
    /// was requested and `analysis.waveform_detail` is non-empty.
    /// Updates `tracks.bpm` and `tracks.musical_key` **only when
    /// those columns are currently NULL** — file-tag values from
    /// import and explicit user edits win, the analyzer fills in
    /// missing data rather than overriding existing data.
    fn record_analysis(
        &self,
        track_id: TrackId,
        analysis: &TrackAnalysis,
        capabilities: AnalysisCapabilities,
        context: AnalysisContext,
    ) -> StoreResult<()>;

    /// Record a failed analysis attempt. Stamps the requested
    /// capabilities' `*_attempted_at_unix` so the scheduler does not
    /// keep retrying every cycle; does not touch
    /// `tracks.bpm`/`musical_key` or the waveform table.
    fn record_analysis_attempt_failure(
        &self,
        track_id: TrackId,
        capabilities: AnalysisCapabilities,
        context: AnalysisContext,
    ) -> StoreResult<()>;

    /// Return up to `limit` track IDs that need at least one of the
    /// requested capabilities re-run. Excludes tracks marked missing.
    /// Returns an empty list when `capabilities.is_empty()`.
    fn tracks_needing_analysis(
        &self,
        capabilities: AnalysisCapabilities,
        analyzer_version: u32,
        limit: usize,
    ) -> StoreResult<Vec<TrackId>>;

    /// Filter `track_ids` down to the subset that still needs at
    /// least one of the requested capabilities run, preserving input
    /// order. Same predicate as [`Self::tracks_needing_analysis`]
    /// (also excludes tracks marked missing). Used by the per-set
    /// explicit run path so re-analyzing a playlist whose tracks are
    /// already cached is a no-op.
    fn filter_tracks_needing_analysis(
        &self,
        track_ids: &[TrackId],
        capabilities: AnalysisCapabilities,
        analyzer_version: u32,
    ) -> StoreResult<Vec<TrackId>>;

    /// Load the stored waveform for a track, if any. Returns `None`
    /// when the track has not been waveform-analyzed yet (or analysis
    /// failed).
    fn load_waveform(&self, track_id: TrackId) -> StoreResult<Option<StoredWaveform>>;

    /// Load every track's acoustic features in one sweep, for the
    /// Smart Shuffle index rebuild. Tracks without an acoustics row
    /// are simply absent from the result — the scorer masks them.
    fn load_all_acoustics(&self) -> StoreResult<Vec<(TrackId, AcousticFeatures)>>;

    /// Persist time-coded lyrics for a track. Overwrites any previous
    /// entry — synced lyrics are a single-shot store-or-replace, not
    /// a merge. Passing an empty [`SyncedLyrics`] is a no-op and
    /// leaves any existing row untouched (call [`Self::clear_synced_lyrics`]
    /// to delete instead).
    fn record_synced_lyrics(
        &self,
        track_id: TrackId,
        lyrics: &SyncedLyrics,
        source: &str,
    ) -> StoreResult<()>;

    /// Load synced lyrics for a track, if any.
    fn load_synced_lyrics(&self, track_id: TrackId) -> StoreResult<Option<StoredSyncedLyrics>>;

    /// Drop any stored synced lyrics for the track. Used by the
    /// "remove lyrics" path (a future UI affordance) and by the
    /// online scheduler when a re-fetch returns nothing.
    fn clear_synced_lyrics(&self, track_id: TrackId) -> StoreResult<()>;

    /// Stamp an online retrieval attempt against the track. Each
    /// requested capability's `*_attempted_at_unix` is set to the
    /// supplied wall clock; capabilities that were not requested
    /// preserve their previous value. This is the only write the
    /// online scheduler makes regardless of whether the underlying
    /// provider returned data — "attempted but found nothing" must
    /// be recorded so the scheduler does not retry every cycle.
    fn record_online_attempt(
        &self,
        track_id: TrackId,
        capabilities: OnlineCapabilities,
        context: OnlineContext,
    ) -> StoreResult<()>;

    /// Return up to `limit` track IDs that need at least one of the
    /// requested online capabilities attempted. Excludes tracks
    /// marked missing. Returns an empty list when
    /// `capabilities.is_empty()`.
    fn tracks_needing_online(
        &self,
        capabilities: OnlineCapabilities,
        provider_version: u32,
        limit: usize,
    ) -> StoreResult<Vec<TrackId>>;

    /// Filter `track_ids` down to the subset that still needs at
    /// least one of the requested online capabilities attempted,
    /// preserving input order. Same predicate as
    /// [`Self::tracks_needing_online`] (the artwork branch still
    /// excludes tracks with embedded artwork).
    fn filter_tracks_needing_online(
        &self,
        track_ids: &[TrackId],
        capabilities: OnlineCapabilities,
        provider_version: u32,
    ) -> StoreResult<Vec<TrackId>>;

    /// Write (or overwrite) the singleton Smart Shuffle index. The
    /// blob format is opaque to the store; `sustain_smart_shuffle`
    /// decides its shape, and `schema_version` lets a future
    /// incompatible change cause the runtime to discard the stored row
    /// at load time without deserialising it.
    fn save_smart_shuffle_index(&self, index: &StoredSmartShuffleIndex) -> StoreResult<()>;

    /// Load the Smart Shuffle index, if one has been written. Returns
    /// `None` when the table is empty (the index has never been built
    /// yet).
    fn load_smart_shuffle_index(&self) -> StoreResult<Option<StoredSmartShuffleIndex>>;

    /// Drop any stored Smart Shuffle index. Used by the runtime when a
    /// load discovers a schema mismatch, so the next rebuild starts
    /// from a clean slate.
    fn clear_smart_shuffle_index(&self) -> StoreResult<()>;

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
    freshly_created: bool,
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
        let freshly_created = !path.exists();
        let connection = Connection::open(path).map_err(StoreError::from)?;
        Self::from_connection(connection, freshly_created)
    }

    pub fn open_in_memory() -> StoreResult<Self> {
        let connection = Connection::open_in_memory().map_err(StoreError::from)?;
        Self::from_connection(connection, true)
    }

    fn from_connection(connection: Connection, freshly_created: bool) -> StoreResult<Self> {
        // SQLite silently ignores WAL on `:memory:` databases, so tests
        // keep working without a special case here.
        connection
            .execute_batch(
                r#"
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;
                "#,
            )
            .map_err(StoreError::from)?;
        let store = Self {
            connection: Mutex::new(connection),
            freshly_created,
        };
        store.migrate()?;
        Ok(store)
    }

    // True when this open() call brought the database file into
    // existence. Callers (typically the runtime at startup) use this to
    // decide whether to perform one-shot first-run setup such as seeding
    // the default smart playlists. Always true for in-memory stores.
    pub fn was_freshly_created(&self) -> bool {
        self.freshly_created
    }

    fn connection_guard(&self) -> StoreResult<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| StoreError::StoreUnavailable)
    }

    fn migrate(&self) -> StoreResult<()> {
        self.connection_guard()?
            .execute_batch(SCHEMA_SQL)
            .map_err(StoreError::from)
    }
}

/// Resolve the on-disk SQLite library database path Sustain reads from when
/// no explicit override is supplied. Returns `None` when neither
/// `XDG_DATA_HOME` nor `HOME` is set, in which case no default location can be
/// derived and the caller is expected to surface the failure (Sustain has no
/// reasonable fallback there).
///
/// Exposed publicly so callers that need the resolved path *before* opening
/// the store (e.g. the application-startup single-instance lock, which
/// flocks a sidecar in the same directory) can reuse the same resolution
/// rule rather than re-deriving it.
pub fn default_database_path() -> Option<std::path::PathBuf> {
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        return Some(
            std::path::PathBuf::from(data_home)
                .join("sustain")
                .join("library.sqlite"),
        );
    }

    std::env::var_os("HOME").map(|home| {
        std::path::PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("sustain")
            .join("library.sqlite")
    })
}

fn save_track_with_connection(connection: &Connection, track: &Track) -> StoreResult<()> {
    let metadata = &track.metadata;
    let statistics = &track.statistics;
    let relative_path = track.location.relative_path.as_path().to_string_lossy();
    connection
        .execute(
            SAVE_TRACK_SQL.as_str(),
            params![
                track.id.get(),
                relative_path,
                metadata.title.as_deref(),
                metadata.artist.as_deref(),
                metadata.album.as_deref(),
                metadata.album_artist.as_deref(),
                metadata.composer.as_deref(),
                metadata.genre.as_deref(),
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
                metadata.grouping.as_deref(),
                metadata.track_total.map(i64::from),
                metadata.disc_total.map(i64::from),
                metadata.compilation,
                metadata.bpm.map(i64::from),
                metadata.key.as_deref(),
                metadata.comments.as_deref(),
                metadata.sample_rate_hz.map(i64::from),
                metadata.channels.map(i64::from),
                metadata.lyrics.as_deref(),
                track.content_hash.as_ref().map(|hash| hash.as_str()),
                track.file_size_bytes.map(|size| size as i64),
                track.has_embedded_artwork.map(i64::from),
            ],
        )
        .map(|_| ())
        .map_err(StoreError::from)
}

impl LibraryStore for SqliteLibraryStore {
    fn save_track(&self, track: Track) -> StoreResult<()> {
        let connection = self.connection_guard()?;
        save_track_with_connection(&connection, &track)
    }

    fn save_tracks(&self, tracks: &[Track]) -> StoreResult<()> {
        let mut connection = self.connection_guard()?;
        let transaction = connection.transaction().map_err(StoreError::from)?;
        for track in tracks {
            save_track_with_connection(&transaction, track)?;
        }
        transaction.commit().map_err(StoreError::from)
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
            .prepare(SELECT_TRACK_BY_ID_SQL.as_str())
            .map_err(StoreError::from)?;
        let mut rows = statement
            .query(params![track_id.get()])
            .map_err(StoreError::from)?;

        rows.next()
            .map_err(StoreError::from)?
            .map(track_from_row)
            .transpose()
    }

    fn track_by_content_hash(
        &self,
        content_hash: &sustain_domain::TrackContentHash,
    ) -> StoreResult<Option<Track>> {
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare(SELECT_TRACK_BY_CONTENT_HASH_SQL.as_str())
            .map_err(StoreError::from)?;
        let mut rows = statement
            .query(params![content_hash.as_str()])
            .map_err(StoreError::from)?;

        rows.next()
            .map_err(StoreError::from)?
            .map(track_from_row)
            .transpose()
    }

    fn tracks(&self) -> StoreResult<Vec<Track>> {
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare(SELECT_ALL_TRACKS_SQL.as_str())
            .map_err(StoreError::from)?;
        let mut rows = statement.query([]).map_err(StoreError::from)?;
        let mut tracks = Vec::new();

        while let Some(row) = rows.next().map_err(StoreError::from)? {
            tracks.push(track_from_row(row)?);
        }

        Ok(tracks)
    }

    fn distinct_genres(&self) -> StoreResult<Vec<String>> {
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare(
                "SELECT DISTINCT genre FROM tracks \
                 WHERE genre IS NOT NULL AND TRIM(genre) <> '' \
                 ORDER BY genre",
            )
            .map_err(StoreError::from)?;
        let mut rows = statement.query([]).map_err(StoreError::from)?;
        let mut genres = Vec::new();
        while let Some(row) = rows.next().map_err(StoreError::from)? {
            let value: String = row.get(0).map_err(StoreError::from)?;
            if !value.trim().is_empty() {
                genres.push(value);
            }
        }
        Ok(genres)
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

    fn load_track_column_layout(
        &self,
        scope: TrackColumnLayoutScope,
    ) -> StoreResult<Option<TrackColumnLayout>> {
        let connection = self.connection_guard()?;
        let entries = match scope {
            TrackColumnLayoutScope::Default => load_layout_rows(
                &connection,
                "SELECT column_id, visible, width_px \
                 FROM track_column_layout_default \
                 ORDER BY position",
                params![],
            )?,
            TrackColumnLayoutScope::Playlist(playlist_id) => load_layout_rows(
                &connection,
                "SELECT column_id, visible, width_px \
                 FROM track_column_layout_playlist_override \
                 WHERE playlist_id = ?1 \
                 ORDER BY position",
                params![playlist_id.get()],
            )?,
            TrackColumnLayoutScope::SmartPlaylist(smart_playlist_id) => load_layout_rows(
                &connection,
                "SELECT column_id, visible, width_px \
                 FROM track_column_layout_smart_playlist_override \
                 WHERE smart_playlist_id = ?1 \
                 ORDER BY position",
                params![smart_playlist_id.get()],
            )?,
        };

        if entries.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TrackColumnLayout::new(entries)))
        }
    }

    fn save_track_column_layout(
        &self,
        scope: TrackColumnLayoutScope,
        layout: &TrackColumnLayout,
    ) -> StoreResult<()> {
        let mut connection = self.connection_guard()?;
        let transaction = connection.transaction().map_err(StoreError::from)?;

        match scope {
            TrackColumnLayoutScope::Default => {
                transaction
                    .execute("DELETE FROM track_column_layout_default", params![])
                    .map_err(StoreError::from)?;
                for (position, entry) in layout.entries.iter().enumerate() {
                    transaction
                        .execute(
                            "INSERT INTO track_column_layout_default \
                             (column_id, position, visible, width_px) \
                             VALUES (?1, ?2, ?3, ?4)",
                            params![
                                entry.column_id,
                                position as i64,
                                i64::from(entry.visible),
                                i64::from(entry.width_px),
                            ],
                        )
                        .map_err(StoreError::from)?;
                }
            }
            TrackColumnLayoutScope::Playlist(playlist_id) => {
                transaction
                    .execute(
                        "DELETE FROM track_column_layout_playlist_override \
                         WHERE playlist_id = ?1",
                        params![playlist_id.get()],
                    )
                    .map_err(StoreError::from)?;
                for (position, entry) in layout.entries.iter().enumerate() {
                    transaction
                        .execute(
                            "INSERT INTO track_column_layout_playlist_override \
                             (playlist_id, column_id, position, visible, width_px) \
                             VALUES (?1, ?2, ?3, ?4, ?5)",
                            params![
                                playlist_id.get(),
                                entry.column_id,
                                position as i64,
                                i64::from(entry.visible),
                                i64::from(entry.width_px),
                            ],
                        )
                        .map_err(StoreError::from)?;
                }
            }
            TrackColumnLayoutScope::SmartPlaylist(smart_playlist_id) => {
                transaction
                    .execute(
                        "DELETE FROM track_column_layout_smart_playlist_override \
                         WHERE smart_playlist_id = ?1",
                        params![smart_playlist_id.get()],
                    )
                    .map_err(StoreError::from)?;
                for (position, entry) in layout.entries.iter().enumerate() {
                    transaction
                        .execute(
                            "INSERT INTO track_column_layout_smart_playlist_override \
                             (smart_playlist_id, column_id, position, visible, width_px) \
                             VALUES (?1, ?2, ?3, ?4, ?5)",
                            params![
                                smart_playlist_id.get(),
                                entry.column_id,
                                position as i64,
                                i64::from(entry.visible),
                                i64::from(entry.width_px),
                            ],
                        )
                        .map_err(StoreError::from)?;
                }
            }
        }

        transaction.commit().map_err(StoreError::from)
    }

    fn delete_track_column_layout(&self, scope: TrackColumnLayoutScope) -> StoreResult<()> {
        let connection = self.connection_guard()?;
        match scope {
            TrackColumnLayoutScope::Default => connection
                .execute("DELETE FROM track_column_layout_default", params![])
                .map(|_| ())
                .map_err(StoreError::from),
            TrackColumnLayoutScope::Playlist(playlist_id) => connection
                .execute(
                    "DELETE FROM track_column_layout_playlist_override WHERE playlist_id = ?1",
                    params![playlist_id.get()],
                )
                .map(|_| ())
                .map_err(StoreError::from),
            TrackColumnLayoutScope::SmartPlaylist(smart_playlist_id) => connection
                .execute(
                    "DELETE FROM track_column_layout_smart_playlist_override \
                     WHERE smart_playlist_id = ?1",
                    params![smart_playlist_id.get()],
                )
                .map(|_| ())
                .map_err(StoreError::from),
        }
    }

    fn record_analysis(
        &self,
        track_id: TrackId,
        analysis: &TrackAnalysis,
        capabilities: AnalysisCapabilities,
        context: AnalysisContext,
    ) -> StoreResult<()> {
        if capabilities.is_empty() {
            return Ok(());
        }
        let mut connection = self.connection_guard()?;
        let transaction = connection.transaction().map_err(StoreError::from)?;

        upsert_track_analysis(&transaction, track_id, capabilities, context)?;

        if capabilities.audio && !analysis.waveform_detail.segments.is_empty() {
            transaction
                .execute(
                    UPSERT_TRACK_WAVEFORM_SQL,
                    params![
                        track_id.get(),
                        f64::from(analysis.waveform_preview.segment_duration_ms),
                        waveform_segments_to_blob(&analysis.waveform_preview.segments),
                        f64::from(analysis.waveform_detail.segment_duration_ms),
                        waveform_segments_to_blob(&analysis.waveform_detail.segments),
                    ],
                )
                .map_err(StoreError::from)?;
        }

        if capabilities.bpm
            && let Some(bpm) = analysis.bpm
        {
            transaction
                .execute(
                    FILL_TRACK_BPM_IF_NULL_SQL,
                    params![bpm.round() as i64, track_id.get()],
                )
                .map_err(StoreError::from)?;
        }

        if capabilities.key
            && let Some(key) = analysis.key
        {
            transaction
                .execute(
                    FILL_TRACK_MUSICAL_KEY_IF_NULL_SQL,
                    params![key.short_code(), track_id.get()],
                )
                .map_err(StoreError::from)?;
        }

        if capabilities.audio
            && let Some(acoustics) = analysis.acoustics
        {
            transaction
                .execute(
                    UPSERT_TRACK_ACOUSTICS_SQL,
                    params![
                        track_id.get(),
                        f64::from(acoustics.integrated_lufs),
                        f64::from(acoustics.short_term_lufs_max),
                        f64::from(acoustics.loudness_range_lu),
                        f64::from(acoustics.onset_rate_hz),
                        f64::from(acoustics.low_band_ratio),
                        f64::from(acoustics.mid_band_ratio),
                        f64::from(acoustics.high_band_ratio),
                        f64::from(acoustics.low_band_variation),
                        f64::from(acoustics.tonalness),
                    ],
                )
                .map_err(StoreError::from)?;
        }

        transaction.commit().map_err(StoreError::from)
    }

    fn record_analysis_attempt_failure(
        &self,
        track_id: TrackId,
        capabilities: AnalysisCapabilities,
        context: AnalysisContext,
    ) -> StoreResult<()> {
        if capabilities.is_empty() {
            return Ok(());
        }
        let connection = self.connection_guard()?;
        upsert_track_analysis(&connection, track_id, capabilities, context)
    }

    fn tracks_needing_analysis(
        &self,
        capabilities: AnalysisCapabilities,
        analyzer_version: u32,
        limit: usize,
    ) -> StoreResult<Vec<TrackId>> {
        if capabilities.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare(SELECT_TRACKS_NEEDING_ANALYSIS_SQL)
            .map_err(StoreError::from)?;
        let mut rows = statement
            .query(params![
                i64::from(capabilities.bpm),
                i64::from(capabilities.key),
                i64::from(capabilities.audio),
                i64::from(analyzer_version),
                limit as i64,
            ])
            .map_err(StoreError::from)?;
        let mut ids = Vec::new();
        while let Some(row) = rows.next().map_err(StoreError::from)? {
            let raw: i64 = row.get(0).map_err(StoreError::from)?;
            let id = TrackId::new(raw).ok_or(StoreError::InvalidStoredId(raw))?;
            ids.push(id);
        }
        Ok(ids)
    }

    fn filter_tracks_needing_analysis(
        &self,
        track_ids: &[TrackId],
        capabilities: AnalysisCapabilities,
        analyzer_version: u32,
    ) -> StoreResult<Vec<TrackId>> {
        if capabilities.is_empty() || track_ids.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.connection_guard()?;
        let mut needing: HashSet<TrackId> = HashSet::with_capacity(track_ids.len());
        for chunk in track_ids.chunks(FILTER_IN_LIST_CHUNK_SIZE) {
            let sql = build_filter_tracks_needing_analysis_sql(chunk.len());
            let mut statement = connection.prepare(&sql).map_err(StoreError::from)?;
            let mut params: Vec<SqlValue> =
                chunk.iter().map(|id| SqlValue::Integer(id.get())).collect();
            params.push(SqlValue::Integer(i64::from(capabilities.bpm)));
            params.push(SqlValue::Integer(i64::from(capabilities.key)));
            params.push(SqlValue::Integer(i64::from(capabilities.audio)));
            params.push(SqlValue::Integer(i64::from(analyzer_version)));
            let mut rows = statement
                .query(params_from_iter(params.iter()))
                .map_err(StoreError::from)?;
            while let Some(row) = rows.next().map_err(StoreError::from)? {
                let raw: i64 = row.get(0).map_err(StoreError::from)?;
                let id = TrackId::new(raw).ok_or(StoreError::InvalidStoredId(raw))?;
                needing.insert(id);
            }
        }
        // Preserve caller order — playlist order is what the user
        // sees, and downstream FIFO dispatch carries that order
        // through to the scheduler.
        Ok(track_ids
            .iter()
            .copied()
            .filter(|id| needing.contains(id))
            .collect())
    }

    fn load_waveform(&self, track_id: TrackId) -> StoreResult<Option<StoredWaveform>> {
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare(SELECT_TRACK_WAVEFORM_SQL)
            .map_err(StoreError::from)?;
        let mut rows = statement
            .query(params![track_id.get()])
            .map_err(StoreError::from)?;
        let Some(row) = rows.next().map_err(StoreError::from)? else {
            return Ok(None);
        };
        let preview_duration: f64 = row.get(0).map_err(StoreError::from)?;
        let preview_bytes: Vec<u8> = row.get(1).map_err(StoreError::from)?;
        let detail_duration: f64 = row.get(2).map_err(StoreError::from)?;
        let detail_bytes: Vec<u8> = row.get(3).map_err(StoreError::from)?;
        Ok(Some(StoredWaveform {
            preview: WaveformSegments {
                segment_duration_ms: preview_duration as f32,
                segments: blob_to_waveform_segments(&preview_bytes),
            },
            detail: WaveformSegments {
                segment_duration_ms: detail_duration as f32,
                segments: blob_to_waveform_segments(&detail_bytes),
            },
        }))
    }

    fn load_all_acoustics(&self) -> StoreResult<Vec<(TrackId, AcousticFeatures)>> {
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare(SELECT_ALL_TRACK_ACOUSTICS_SQL)
            .map_err(StoreError::from)?;
        let mut rows = statement.query([]).map_err(StoreError::from)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(StoreError::from)? {
            let raw: i64 = row.get(0).map_err(StoreError::from)?;
            let Some(track_id) = TrackId::new(raw) else {
                continue;
            };
            let value = |index: usize| -> StoreResult<f32> {
                Ok(row.get::<_, f64>(index).map_err(StoreError::from)? as f32)
            };
            out.push((
                track_id,
                AcousticFeatures {
                    integrated_lufs: value(1)?,
                    short_term_lufs_max: value(2)?,
                    loudness_range_lu: value(3)?,
                    onset_rate_hz: value(4)?,
                    low_band_ratio: value(5)?,
                    mid_band_ratio: value(6)?,
                    high_band_ratio: value(7)?,
                    low_band_variation: value(8)?,
                    tonalness: value(9)?,
                },
            ));
        }
        Ok(out)
    }

    fn record_synced_lyrics(
        &self,
        track_id: TrackId,
        lyrics: &SyncedLyrics,
        source: &str,
    ) -> StoreResult<()> {
        if lyrics.is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string(&lyrics.lines)
            .map_err(|error| StoreError::Database(error.to_string()))?;
        let connection = self.connection_guard()?;
        connection
            .execute(
                UPSERT_TRACK_SYNCED_LYRICS_SQL,
                params![track_id.get(), json, source],
            )
            .map(|_| ())
            .map_err(StoreError::from)
    }

    fn load_synced_lyrics(&self, track_id: TrackId) -> StoreResult<Option<StoredSyncedLyrics>> {
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare(SELECT_TRACK_SYNCED_LYRICS_SQL)
            .map_err(StoreError::from)?;
        let mut rows = statement
            .query(params![track_id.get()])
            .map_err(StoreError::from)?;
        let Some(row) = rows.next().map_err(StoreError::from)? else {
            return Ok(None);
        };
        let json: String = row.get(0).map_err(StoreError::from)?;
        let source: String = row.get(1).map_err(StoreError::from)?;
        let lines =
            serde_json::from_str(&json).map_err(|error| StoreError::Database(error.to_string()))?;
        Ok(Some(StoredSyncedLyrics {
            lyrics: SyncedLyrics { lines },
            source,
        }))
    }

    fn clear_synced_lyrics(&self, track_id: TrackId) -> StoreResult<()> {
        let connection = self.connection_guard()?;
        connection
            .execute(DELETE_TRACK_SYNCED_LYRICS_SQL, params![track_id.get()])
            .map(|_| ())
            .map_err(StoreError::from)
    }

    fn save_smart_shuffle_index(&self, index: &StoredSmartShuffleIndex) -> StoreResult<()> {
        let connection = self.connection_guard()?;
        connection
            .execute(
                UPSERT_SMART_SHUFFLE_INDEX_SQL,
                params![index.index_blob, i64::from(index.schema_version)],
            )
            .map(|_| ())
            .map_err(StoreError::from)
    }

    fn load_smart_shuffle_index(&self) -> StoreResult<Option<StoredSmartShuffleIndex>> {
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare(SELECT_SMART_SHUFFLE_INDEX_SQL)
            .map_err(StoreError::from)?;
        let row = statement
            .query_row([], |row| {
                let blob: Vec<u8> = row.get(0)?;
                let schema_version: i64 = row.get(1)?;
                Ok((blob, schema_version))
            })
            .optional()
            .map_err(StoreError::from)?;
        Ok(row.map(|(blob, schema_version)| StoredSmartShuffleIndex {
            index_blob: blob,
            // Schema version is a non-negative integer; widen at the
            // boundary so the in-memory value carries a u32.
            schema_version: schema_version.max(0) as u32,
        }))
    }

    fn clear_smart_shuffle_index(&self) -> StoreResult<()> {
        let connection = self.connection_guard()?;
        connection
            .execute(DELETE_SMART_SHUFFLE_INDEX_SQL, [])
            .map(|_| ())
            .map_err(StoreError::from)
    }

    fn record_online_attempt(
        &self,
        track_id: TrackId,
        capabilities: OnlineCapabilities,
        context: OnlineContext,
    ) -> StoreResult<()> {
        if capabilities.is_empty() {
            return Ok(());
        }
        let artwork_at = capabilities.artwork.then_some(context.now_unix);
        let tags_at = capabilities.tags.then_some(context.now_unix);
        let lyrics_at = capabilities.lyrics.then_some(context.now_unix);
        let connection = self.connection_guard()?;
        connection
            .execute(
                UPSERT_TRACK_ONLINE_STATUS_SQL,
                params![
                    track_id.get(),
                    artwork_at,
                    tags_at,
                    lyrics_at,
                    i64::from(context.provider_version),
                ],
            )
            .map(|_| ())
            .map_err(StoreError::from)
    }

    fn tracks_needing_online(
        &self,
        capabilities: OnlineCapabilities,
        provider_version: u32,
        limit: usize,
    ) -> StoreResult<Vec<TrackId>> {
        if capabilities.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.connection_guard()?;
        let mut statement = connection
            .prepare(SELECT_TRACKS_NEEDING_ONLINE_SQL)
            .map_err(StoreError::from)?;
        let mut rows = statement
            .query(params![
                i64::from(capabilities.artwork),
                i64::from(capabilities.tags),
                i64::from(capabilities.lyrics),
                i64::from(provider_version),
                limit as i64,
            ])
            .map_err(StoreError::from)?;
        let mut ids = Vec::new();
        while let Some(row) = rows.next().map_err(StoreError::from)? {
            let raw: i64 = row.get(0).map_err(StoreError::from)?;
            let id = TrackId::new(raw).ok_or(StoreError::InvalidStoredId(raw))?;
            ids.push(id);
        }
        Ok(ids)
    }

    fn filter_tracks_needing_online(
        &self,
        track_ids: &[TrackId],
        capabilities: OnlineCapabilities,
        provider_version: u32,
    ) -> StoreResult<Vec<TrackId>> {
        if capabilities.is_empty() || track_ids.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.connection_guard()?;
        let mut needing: HashSet<TrackId> = HashSet::with_capacity(track_ids.len());
        for chunk in track_ids.chunks(FILTER_IN_LIST_CHUNK_SIZE) {
            let sql = build_filter_tracks_needing_online_sql(chunk.len());
            let mut statement = connection.prepare(&sql).map_err(StoreError::from)?;
            let mut params: Vec<SqlValue> =
                chunk.iter().map(|id| SqlValue::Integer(id.get())).collect();
            params.push(SqlValue::Integer(i64::from(capabilities.artwork)));
            params.push(SqlValue::Integer(i64::from(capabilities.tags)));
            params.push(SqlValue::Integer(i64::from(capabilities.lyrics)));
            params.push(SqlValue::Integer(i64::from(provider_version)));
            let mut rows = statement
                .query(params_from_iter(params.iter()))
                .map_err(StoreError::from)?;
            while let Some(row) = rows.next().map_err(StoreError::from)? {
                let raw: i64 = row.get(0).map_err(StoreError::from)?;
                let id = TrackId::new(raw).ok_or(StoreError::InvalidStoredId(raw))?;
                needing.insert(id);
            }
        }
        Ok(track_ids
            .iter()
            .copied()
            .filter(|id| needing.contains(id))
            .collect())
    }
}

/// Per-call chunk size for the `t.id IN (?, ?, ...)` clause used by
/// the two filter queries. Stays well under SQLite's default 32k
/// bound-variable cap (each chunk binds N IDs + 4 fixed params).
const FILTER_IN_LIST_CHUNK_SIZE: usize = 500;

fn build_filter_tracks_needing_analysis_sql(id_count: usize) -> String {
    debug_assert!(id_count > 0);
    let id_placeholders = (1..=id_count)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let bpm = id_count + 1;
    let key = id_count + 2;
    let audio = id_count + 3;
    let version = id_count + 4;
    format!(
        "SELECT t.id FROM tracks t
LEFT JOIN track_analysis ta ON ta.track_id = t.id
WHERE t.is_missing = 0
  AND t.id IN ({id_placeholders})
  AND (
        (?{bpm} = 1 AND (ta.bpm_attempted_at_unix   IS NULL OR ta.analyzer_version < ?{version}))
     OR (?{key} = 1 AND (ta.key_attempted_at_unix   IS NULL OR ta.analyzer_version < ?{version}))
     OR (?{audio} = 1 AND (ta.audio_attempted_at_unix IS NULL OR ta.analyzer_version < ?{version}))
      )"
    )
}

fn build_filter_tracks_needing_online_sql(id_count: usize) -> String {
    debug_assert!(id_count > 0);
    let id_placeholders = (1..=id_count)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let artwork = id_count + 1;
    let tags = id_count + 2;
    let lyrics = id_count + 3;
    let version = id_count + 4;
    // Mirrors `SELECT_TRACKS_NEEDING_ONLINE_SQL`: the artwork branch
    // still excludes tracks with embedded artwork so we never fetch a
    // remote picture for a file that already carries one.
    format!(
        "SELECT t.id FROM tracks t
LEFT JOIN track_online_status s ON s.track_id = t.id
WHERE t.is_missing = 0
  AND t.id IN ({id_placeholders})
  AND (
        (?{artwork} = 1
            AND COALESCE(t.has_embedded_artwork, 0) = 0
            AND (s.artwork_attempted_at_unix IS NULL OR s.provider_version < ?{version}))
     OR (?{tags}    = 1 AND (s.tags_attempted_at_unix   IS NULL OR s.provider_version < ?{version}))
     OR (?{lyrics}  = 1 AND (s.lyrics_attempted_at_unix IS NULL OR s.provider_version < ?{version}))
      )"
    )
}

/// Shared upsert helper for [`SqliteLibraryStore::record_analysis`] and
/// [`SqliteLibraryStore::record_analysis_attempt_failure`]. NULL is
/// passed for any `*_attempted_at_unix` column the caller did not
/// request, so the SQL's `COALESCE` preserves the existing value.
fn upsert_track_analysis(
    connection: &Connection,
    track_id: TrackId,
    capabilities: AnalysisCapabilities,
    context: AnalysisContext,
) -> StoreResult<()> {
    let bpm_at = capabilities.bpm.then_some(context.now_unix);
    let key_at = capabilities.key.then_some(context.now_unix);
    let audio_at = capabilities.audio.then_some(context.now_unix);
    connection
        .execute(
            UPSERT_TRACK_ANALYSIS_SQL,
            params![
                track_id.get(),
                bpm_at,
                key_at,
                audio_at,
                i64::from(context.analyzer_version),
            ],
        )
        .map(|_| ())
        .map_err(StoreError::from)
}

fn load_layout_rows(
    connection: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> StoreResult<Vec<TrackColumnEntry>> {
    let mut statement = connection.prepare(sql).map_err(StoreError::from)?;
    let mut rows = statement.query(params).map_err(StoreError::from)?;
    let mut entries = Vec::new();
    while let Some(row) = rows.next().map_err(StoreError::from)? {
        let column_id: String = row.get(0).map_err(StoreError::from)?;
        let visible_flag: i64 = row.get(1).map_err(StoreError::from)?;
        let width_px: i64 = row.get(2).map_err(StoreError::from)?;
        entries.push(TrackColumnEntry {
            column_id,
            visible: visible_flag != 0,
            width_px: width_px.max(0) as u32,
        });
    }
    Ok(entries)
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
