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
mod tests {
    use std::path::PathBuf;

    use std::{num::NonZeroU32, time::SystemTime};

    use sustain_domain::{
        PlayStatistics, PlaylistEntry, Rating, SmartPlaylistDateField, SmartPlaylistLimit,
        SmartPlaylistLimitSelection, SmartPlaylistMatchKind, SmartPlaylistNumberField,
        SmartPlaylistNumberOperator, SmartPlaylistRule, SmartPlaylistRuleSet,
        SmartPlaylistTextField, SmartPlaylistTextOperator, SortDirection, TrackContentHash,
        TrackLocation, TrackMetadata, TrackRelativePath, TrackSort, TrackSortColumn,
    };

    use sustain_domain::{
        DETAIL_SEGMENTS_PER_SECOND, MusicalKey, PREVIEW_SEGMENT_COUNT, TrackAnalysis,
        WaveformSegment, WaveformSegments,
    };

    use super::{
        AnalysisCapabilities, AnalysisContext, InMemoryLibraryStore, LibraryQuery, LibraryStore,
        OnlineCapabilities, OnlineContext, Playlist, PlaylistFolder, PlaylistFolderId,
        SmartPlaylist, SmartPlaylistId, SqliteLibraryStore, StoredSyncedLyrics, StoredWaveform,
        SyncedLyrics, Track, TrackColumnEntry, TrackColumnLayout, TrackColumnLayoutScope,
    };
    use crate::{PlaylistId, StoreResult, TrackId};
    use sustain_domain::SyncedLyricsLine;

    #[test]
    fn in_memory_store_starts_empty() {
        let store = InMemoryLibraryStore::new();

        assert_eq!(store.tracks(), Ok(Vec::new()));
        assert_eq!(store.playlists(), Ok(Vec::new()));
    }

    #[test]
    fn in_memory_store_saves_and_loads_tracks() {
        let store = InMemoryLibraryStore::new();
        let mut track = track(1, "a.flac");
        track.content_hash = Some(test_hash(1));

        assert_eq!(store.save_track(track.clone()), Ok(()));

        assert_eq!(store.track(track.id), Ok(Some(track.clone())));
        assert_eq!(
            store.track_by_content_hash(track.content_hash.as_ref().expect("hash")),
            Ok(Some(track.clone()))
        );
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
    fn sqlite_store_reports_freshly_created_only_on_first_open() {
        let dir = std::env::temp_dir().join(format!(
            "sustain_freshness_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after unix epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create test directory");
        let path = dir.join("library.sqlite");

        let first = SqliteLibraryStore::open(&path).expect("open creates the database file");
        assert!(first.was_freshly_created());
        drop(first);

        let second = SqliteLibraryStore::open(&path).expect("reopen existing database");
        assert!(!second.was_freshly_created());
        drop(second);

        std::fs::remove_dir_all(&dir).expect("clean up test directory");
    }

    #[test]
    fn sqlite_store_saves_and_loads_tracks() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let mut track = track(1, "a.flac");
        track.metadata.title = Some("Track".to_owned());
        track.metadata.artist = Some("Artist".to_owned());
        track.metadata.bitrate_kbps = Some(1411);
        track.metadata.duration = Some(std::time::Duration::from_secs(245));
        track.content_hash = Some(test_hash(42));
        track.rating = Rating::new(4).expect("valid test rating");

        assert_eq!(store.save_track(track.clone()), Ok(()));

        assert_eq!(store.track(track.id), Ok(Some(track.clone())));
        assert_eq!(
            store.track_by_content_hash(track.content_hash.as_ref().expect("hash")),
            Ok(Some(track.clone()))
        );
        assert_eq!(store.tracks(), Ok(vec![track]));
    }

    #[test]
    fn sqlite_store_rolls_back_batch_track_save_on_failure() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let first = track(1, "same.flac");
        let duplicate_relative_path = track(2, "same.flac");

        assert!(
            store
                .save_tracks(&[first, duplicate_relative_path])
                .is_err()
        );
        assert_eq!(store.tracks(), Ok(Vec::new()));
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
                    days: NonZeroU32::new(7).expect("positive day count"),
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
                        value: Rating::new(4).expect("valid rating"),
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
                        days: NonZeroU32::new(30).expect("positive day count"),
                    },
                    SmartPlaylistRule::DateNotInLast {
                        field: SmartPlaylistDateField::LastSkipped,
                        days: NonZeroU32::new(90).expect("positive day count"),
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
                value: Rating::new(5).expect("valid rating"),
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
                    count: NonZeroU32::new(25).expect("positive limit"),
                    selection: SmartPlaylistLimitSelection::MostRecentlyAdded,
                }),
                rules: vec![SmartPlaylistRule::Rating {
                    operator: SmartPlaylistNumberOperator::GreaterThanOrEqual,
                    value: Rating::new(4).expect("valid rating"),
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

    // -------- analysis storage --------

    /// Build a non-trivial `TrackAnalysis` with a handful of segments
    /// in each tier so blob round-trip and "fill if NULL" semantics
    /// both have real data to act on.
    fn sample_analysis(bpm: Option<f32>, key: Option<MusicalKey>) -> TrackAnalysis {
        let preview = WaveformSegments {
            segment_duration_ms: 25.0,
            segments: (0..PREVIEW_SEGMENT_COUNT)
                .map(|i| WaveformSegment {
                    amplitude: (i % 256) as u8,
                    low_band: ((i * 3) % 256) as u8,
                    mid_band: ((i * 5) % 256) as u8,
                    high_band: ((i * 7) % 256) as u8,
                })
                .collect(),
        };
        let detail = WaveformSegments {
            segment_duration_ms: 1_000.0 / DETAIL_SEGMENTS_PER_SECOND as f32,
            segments: (0..512)
                .map(|i| WaveformSegment {
                    amplitude: (i % 256) as u8,
                    low_band: ((i + 1) % 256) as u8,
                    mid_band: ((i + 2) % 256) as u8,
                    high_band: ((i + 3) % 256) as u8,
                })
                .collect(),
        };
        TrackAnalysis {
            bpm,
            key,
            beatgrid: None,
            waveform_preview: preview,
            waveform_detail: detail,
            acoustics: None,
        }
    }

    /// Standard context used by analysis tests: analyzer_version 1
    /// plus caller-supplied wall-clock.
    fn ctx(now_unix: i64) -> AnalysisContext {
        AnalysisContext {
            analyzer_version: 1,
            now_unix,
        }
    }

    fn run_record_analysis_round_trips_waveform_bytes(store: &dyn LibraryStore) {
        let track = track(1, "a.flac");
        store.save_track(track.clone()).expect("save track");

        let analysis = sample_analysis(Some(126.0), Some(MusicalKey::DMinor));
        store
            .record_analysis(
                track.id,
                &analysis,
                AnalysisCapabilities::all(),
                ctx(1_700_000_000),
            )
            .expect("record analysis");

        let stored = store
            .load_waveform(track.id)
            .expect("load")
            .expect("waveform exists");
        assert_eq!(stored.preview.segments, analysis.waveform_preview.segments);
        assert_eq!(stored.detail.segments, analysis.waveform_detail.segments);
        assert!(
            (stored.preview.segment_duration_ms - analysis.waveform_preview.segment_duration_ms)
                .abs()
                < 1e-3
        );
    }

    #[test]
    fn sqlite_record_analysis_round_trips_waveform_bytes() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory");
        run_record_analysis_round_trips_waveform_bytes(&store);
    }

    #[test]
    fn in_memory_record_analysis_round_trips_waveform_bytes() {
        run_record_analysis_round_trips_waveform_bytes(&InMemoryLibraryStore::new());
    }

    fn run_record_analysis_fills_tracks_columns_only_when_null(store: &dyn LibraryStore) {
        // First track: no pre-existing BPM/key — analyzer fills both.
        let blank = track(1, "blank.flac");
        store.save_track(blank.clone()).expect("save blank");

        // Second track: user-set BPM/key — analyzer must not clobber.
        let mut taken = track(2, "taken.flac");
        taken.metadata.bpm = Some(95);
        taken.metadata.key = Some("Am".to_string());
        store.save_track(taken.clone()).expect("save taken");

        let analysis = sample_analysis(Some(126.0), Some(MusicalKey::DMinor));
        for id in [blank.id, taken.id] {
            store
                .record_analysis(
                    id,
                    &analysis,
                    AnalysisCapabilities::all(),
                    ctx(1_700_000_000),
                )
                .expect("record");
        }

        let loaded_blank = store.track(blank.id).expect("load blank").expect("exists");
        assert_eq!(loaded_blank.metadata.bpm, Some(126));
        assert_eq!(loaded_blank.metadata.key.as_deref(), Some("Dm"));

        let loaded_taken = store.track(taken.id).expect("load taken").expect("exists");
        assert_eq!(loaded_taken.metadata.bpm, Some(95));
        assert_eq!(loaded_taken.metadata.key.as_deref(), Some("Am"));
    }

    #[test]
    fn sqlite_record_analysis_fills_tracks_columns_only_when_null() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory");
        run_record_analysis_fills_tracks_columns_only_when_null(&store);
    }

    #[test]
    fn in_memory_record_analysis_fills_tracks_columns_only_when_null() {
        run_record_analysis_fills_tracks_columns_only_when_null(&InMemoryLibraryStore::new());
    }

    fn run_tracks_needing_analysis_lists_only_un_attempted(store: &dyn LibraryStore) {
        // 3 tracks, 1 missing, 1 already waveform-analyzed.
        let alpha = track(1, "alpha.flac");
        let mut beta = track(2, "beta.flac");
        beta.location = beta
            .location
            .with_availability(sustain_domain::TrackAvailability::Missing);
        let gamma = track(3, "gamma.flac");
        for t in [&alpha, &beta, &gamma] {
            store.save_track(t.clone()).expect("save");
        }

        // Mark gamma's waveform as attempted.
        store
            .record_analysis(
                gamma.id,
                &sample_analysis(None, None),
                AnalysisCapabilities {
                    bpm: false,
                    key: false,
                    audio: true,
                },
                ctx(1_700_000_000),
            )
            .expect("record waveform for gamma");

        let needs = store
            .tracks_needing_analysis(
                AnalysisCapabilities {
                    bpm: false,
                    key: false,
                    audio: true,
                },
                1,
                100,
            )
            .expect("query");
        // Only alpha qualifies: beta is missing, gamma is attempted at version 1.
        assert_eq!(needs, vec![alpha.id]);
    }

    #[test]
    fn sqlite_tracks_needing_analysis_lists_only_un_attempted() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory");
        run_tracks_needing_analysis_lists_only_un_attempted(&store);
    }

    #[test]
    fn in_memory_tracks_needing_analysis_lists_only_un_attempted() {
        run_tracks_needing_analysis_lists_only_un_attempted(&InMemoryLibraryStore::new());
    }

    fn run_filter_tracks_needing_analysis_drops_cached_and_missing(store: &dyn LibraryStore) {
        // Same setup as `run_tracks_needing_analysis_lists_only_un_attempted`
        // but exercised through the bulk-filter path: caller passes the
        // full set of ids it cares about (mirroring a per-playlist
        // explicit run) and the store returns only the ones that still
        // need at least one of the requested capabilities.
        let alpha = track(1, "alpha.flac");
        let mut beta = track(2, "beta.flac");
        beta.location = beta
            .location
            .with_availability(sustain_domain::TrackAvailability::Missing);
        let gamma = track(3, "gamma.flac");
        for t in [&alpha, &beta, &gamma] {
            store.save_track(t.clone()).expect("save");
        }

        store
            .record_analysis(
                gamma.id,
                &sample_analysis(None, None),
                AnalysisCapabilities {
                    bpm: false,
                    key: false,
                    audio: true,
                },
                ctx(1_700_000_000),
            )
            .expect("record waveform for gamma");

        let all_ids = vec![alpha.id, beta.id, gamma.id];

        // Only waveform requested: alpha needs it (never attempted),
        // beta is missing, gamma is already attempted at v1. Filter
        // returns only alpha.
        let filtered = store
            .filter_tracks_needing_analysis(
                &all_ids,
                AnalysisCapabilities {
                    bpm: false,
                    key: false,
                    audio: true,
                },
                1,
            )
            .expect("filter");
        assert_eq!(filtered, vec![alpha.id]);

        // BPM requested: alpha and gamma both qualify (neither was
        // BPM-attempted), beta is still missing. Order matches input
        // order so playlist sequencing survives downstream.
        let filtered_bpm = store
            .filter_tracks_needing_analysis(
                &all_ids,
                AnalysisCapabilities {
                    bpm: true,
                    key: false,
                    audio: false,
                },
                1,
            )
            .expect("filter");
        assert_eq!(filtered_bpm, vec![alpha.id, gamma.id]);

        // Empty capability mask -> empty result, regardless of input.
        let filtered_empty = store
            .filter_tracks_needing_analysis(&all_ids, AnalysisCapabilities::default(), 1)
            .expect("filter");
        assert!(filtered_empty.is_empty());

        // Empty input id list -> empty result.
        let filtered_no_ids = store
            .filter_tracks_needing_analysis(
                &[],
                AnalysisCapabilities {
                    bpm: true,
                    key: true,
                    audio: true,
                },
                1,
            )
            .expect("filter");
        assert!(filtered_no_ids.is_empty());

        // Version bump re-enrolls cached tracks: gamma now needs
        // waveform again (its stamp is at version 1).
        let filtered_v2 = store
            .filter_tracks_needing_analysis(
                &all_ids,
                AnalysisCapabilities {
                    bpm: false,
                    key: false,
                    audio: true,
                },
                2,
            )
            .expect("filter");
        assert_eq!(filtered_v2, vec![alpha.id, gamma.id]);
    }

    #[test]
    fn sqlite_filter_tracks_needing_analysis_drops_cached_and_missing() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory");
        run_filter_tracks_needing_analysis_drops_cached_and_missing(&store);
    }

    #[test]
    fn in_memory_filter_tracks_needing_analysis_drops_cached_and_missing() {
        run_filter_tracks_needing_analysis_drops_cached_and_missing(&InMemoryLibraryStore::new());
    }

    fn run_failed_attempt_prevents_immediate_retry(store: &dyn LibraryStore) {
        let track = track(1, "a.flac");
        store.save_track(track.clone()).expect("save");

        store
            .record_analysis_attempt_failure(
                track.id,
                AnalysisCapabilities {
                    bpm: true,
                    key: false,
                    audio: false,
                },
                ctx(1_700_000_000),
            )
            .expect("record failure");

        let needs = store
            .tracks_needing_analysis(
                AnalysisCapabilities {
                    bpm: true,
                    key: false,
                    audio: false,
                },
                1,
                100,
            )
            .expect("query");
        assert!(
            needs.is_empty(),
            "failed attempts should not requeue at same analyzer_version"
        );

        // But a version bump re-enrolls the track.
        let needs_v2 = store
            .tracks_needing_analysis(
                AnalysisCapabilities {
                    bpm: true,
                    key: false,
                    audio: false,
                },
                2,
                100,
            )
            .expect("query v2");
        assert_eq!(needs_v2, vec![track.id]);
    }

    #[test]
    fn sqlite_failed_attempt_prevents_immediate_retry() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory");
        run_failed_attempt_prevents_immediate_retry(&store);
    }

    #[test]
    fn in_memory_failed_attempt_prevents_immediate_retry() {
        run_failed_attempt_prevents_immediate_retry(&InMemoryLibraryStore::new());
    }

    #[test]
    fn sqlite_cascade_delete_clears_analysis_rows() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory");
        let t = track(1, "a.flac");
        store.save_track(t.clone()).expect("save");
        store
            .record_analysis(
                t.id,
                &sample_analysis(Some(120.0), Some(MusicalKey::CMajor)),
                AnalysisCapabilities::all(),
                ctx(1_700_000_000),
            )
            .expect("record");
        assert!(matches!(store.load_waveform(t.id), Ok(Some(_))));

        store.delete_track(t.id).expect("delete");
        assert_eq!(store.load_waveform(t.id), Ok(None));
        // Re-saving the same id should not trip a unique-violation
        // because the cascade dropped the analysis/waveform rows too.
        store.save_track(t.clone()).expect("re-save");
        store
            .record_analysis(
                t.id,
                &sample_analysis(Some(130.0), Some(MusicalKey::EMinor)),
                AnalysisCapabilities::all(),
                ctx(1_700_000_000),
            )
            .expect("record again");
        assert!(matches!(store.load_waveform(t.id), Ok(Some(_))));
    }

    fn run_partial_capability_record_preserves_other_attempts(store: &dyn LibraryStore) {
        let t = track(1, "a.flac");
        store.save_track(t.clone()).expect("save");

        // First, record only BPM.
        store
            .record_analysis_attempt_failure(
                t.id,
                AnalysisCapabilities {
                    bpm: true,
                    key: false,
                    audio: false,
                },
                ctx(1_000),
            )
            .expect("record bpm");

        // Then, separately, record only waveform.
        store
            .record_analysis(
                t.id,
                &sample_analysis(None, None),
                AnalysisCapabilities {
                    bpm: false,
                    key: false,
                    audio: true,
                },
                ctx(2_000),
            )
            .expect("record waveform");

        // Both capabilities should now be marked attempted; only key
        // remains pending.
        let needs_bpm = store
            .tracks_needing_analysis(
                AnalysisCapabilities {
                    bpm: true,
                    key: false,
                    audio: false,
                },
                1,
                10,
            )
            .expect("q bpm");
        let needs_wave = store
            .tracks_needing_analysis(
                AnalysisCapabilities {
                    bpm: false,
                    key: false,
                    audio: true,
                },
                1,
                10,
            )
            .expect("q wave");
        let needs_key = store
            .tracks_needing_analysis(
                AnalysisCapabilities {
                    bpm: false,
                    key: true,
                    audio: false,
                },
                1,
                10,
            )
            .expect("q key");
        assert!(needs_bpm.is_empty(), "bpm should be marked attempted");
        assert!(needs_wave.is_empty(), "waveform should be marked attempted");
        assert_eq!(needs_key, vec![t.id], "key still pending");
    }

    #[test]
    fn sqlite_partial_capability_record_preserves_other_attempts() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory");
        run_partial_capability_record_preserves_other_attempts(&store);
    }

    #[test]
    fn in_memory_partial_capability_record_preserves_other_attempts() {
        run_partial_capability_record_preserves_other_attempts(&InMemoryLibraryStore::new());
    }

    #[test]
    fn empty_capabilities_record_is_no_op() {
        let store = SqliteLibraryStore::open_in_memory().expect("open");
        let t = track(1, "a.flac");
        store.save_track(t.clone()).expect("save");

        store
            .record_analysis(
                t.id,
                &sample_analysis(Some(120.0), Some(MusicalKey::CMajor)),
                AnalysisCapabilities::none(),
                ctx(1_000),
            )
            .expect("no-op");
        // No bookkeeping row → track is still listed as needing analysis.
        let needs = store
            .tracks_needing_analysis(AnalysisCapabilities::all(), 1, 10)
            .expect("query");
        assert_eq!(needs, vec![t.id]);
        assert_eq!(store.load_waveform(t.id), Ok(None));
    }

    #[test]
    fn stored_waveform_equality_is_well_defined() {
        // Sanity check that the public StoredWaveform exposes PartialEq
        // so call sites can do simple assertions in tests.
        let stored = StoredWaveform {
            preview: WaveformSegments {
                segment_duration_ms: 25.0,
                segments: vec![WaveformSegment::silent()],
            },
            detail: WaveformSegments {
                segment_duration_ms: 6.0,
                segments: vec![WaveformSegment::silent()],
            },
        };
        assert_eq!(stored.clone(), stored);
    }

    // -------- end analysis storage --------

    // -------- synced lyrics storage --------

    fn sample_synced_lyrics() -> SyncedLyrics {
        SyncedLyrics {
            lines: vec![
                SyncedLyricsLine {
                    at_ms: 1_000,
                    text: "Hello".to_owned(),
                },
                SyncedLyricsLine {
                    at_ms: 3_500,
                    text: "World".to_owned(),
                },
            ],
        }
    }

    fn run_record_and_load_synced_lyrics_round_trips(store: &dyn LibraryStore) {
        let t = track(1, "a.flac");
        store.save_track(t.clone()).expect("save");
        let lyrics = sample_synced_lyrics();
        store
            .record_synced_lyrics(t.id, &lyrics, "lrclib")
            .expect("record");

        let loaded = store
            .load_synced_lyrics(t.id)
            .expect("load")
            .expect("present");
        assert_eq!(loaded.lyrics, lyrics);
        assert_eq!(loaded.source, "lrclib");
    }

    #[test]
    fn sqlite_record_and_load_synced_lyrics_round_trips() {
        let store = SqliteLibraryStore::open_in_memory().expect("open");
        run_record_and_load_synced_lyrics_round_trips(&store);
    }

    #[test]
    fn in_memory_record_and_load_synced_lyrics_round_trips() {
        run_record_and_load_synced_lyrics_round_trips(&InMemoryLibraryStore::new());
    }

    fn run_record_synced_lyrics_replaces_previous(store: &dyn LibraryStore) {
        let t = track(1, "a.flac");
        store.save_track(t.clone()).expect("save");
        store
            .record_synced_lyrics(t.id, &sample_synced_lyrics(), "lrclib")
            .expect("first");
        let replacement = SyncedLyrics {
            lines: vec![SyncedLyricsLine {
                at_ms: 500,
                text: "Only".to_owned(),
            }],
        };
        store
            .record_synced_lyrics(t.id, &replacement, "user")
            .expect("second");

        let loaded = store
            .load_synced_lyrics(t.id)
            .expect("load")
            .expect("present");
        assert_eq!(loaded.lyrics, replacement);
        assert_eq!(loaded.source, "user");
    }

    #[test]
    fn sqlite_record_synced_lyrics_replaces_previous() {
        let store = SqliteLibraryStore::open_in_memory().expect("open");
        run_record_synced_lyrics_replaces_previous(&store);
    }

    #[test]
    fn in_memory_record_synced_lyrics_replaces_previous() {
        run_record_synced_lyrics_replaces_previous(&InMemoryLibraryStore::new());
    }

    fn run_record_synced_lyrics_empty_is_no_op(store: &dyn LibraryStore) {
        let t = track(1, "a.flac");
        store.save_track(t.clone()).expect("save");
        store
            .record_synced_lyrics(t.id, &sample_synced_lyrics(), "lrclib")
            .expect("seed");
        store
            .record_synced_lyrics(t.id, &SyncedLyrics::default(), "noop")
            .expect("no-op write");

        let loaded = store
            .load_synced_lyrics(t.id)
            .expect("load")
            .expect("present");
        assert_eq!(loaded.source, "lrclib");
        assert_eq!(loaded.lyrics, sample_synced_lyrics());
    }

    #[test]
    fn sqlite_record_synced_lyrics_empty_is_no_op() {
        let store = SqliteLibraryStore::open_in_memory().expect("open");
        run_record_synced_lyrics_empty_is_no_op(&store);
    }

    #[test]
    fn in_memory_record_synced_lyrics_empty_is_no_op() {
        run_record_synced_lyrics_empty_is_no_op(&InMemoryLibraryStore::new());
    }

    fn run_clear_synced_lyrics_removes_row(store: &dyn LibraryStore) {
        let t = track(1, "a.flac");
        store.save_track(t.clone()).expect("save");
        store
            .record_synced_lyrics(t.id, &sample_synced_lyrics(), "lrclib")
            .expect("seed");
        store.clear_synced_lyrics(t.id).expect("clear");
        assert_eq!(store.load_synced_lyrics(t.id), Ok(None));
        // Clearing again is a no-op.
        store.clear_synced_lyrics(t.id).expect("clear again");
    }

    #[test]
    fn sqlite_clear_synced_lyrics_removes_row() {
        let store = SqliteLibraryStore::open_in_memory().expect("open");
        run_clear_synced_lyrics_removes_row(&store);
    }

    #[test]
    fn in_memory_clear_synced_lyrics_removes_row() {
        run_clear_synced_lyrics_removes_row(&InMemoryLibraryStore::new());
    }

    #[test]
    fn sqlite_cascade_delete_clears_synced_lyrics() {
        let store = SqliteLibraryStore::open_in_memory().expect("open");
        let t = track(1, "a.flac");
        store.save_track(t.clone()).expect("save");
        store
            .record_synced_lyrics(t.id, &sample_synced_lyrics(), "lrclib")
            .expect("seed");
        store.delete_track(t.id).expect("delete");
        assert_eq!(store.load_synced_lyrics(t.id), Ok(None));
    }

    #[test]
    fn stored_synced_lyrics_equality_is_well_defined() {
        let s = StoredSyncedLyrics {
            lyrics: sample_synced_lyrics(),
            source: "lrclib".to_owned(),
        };
        assert_eq!(s.clone(), s);
    }

    // -------- end synced lyrics storage --------

    // -------- online status storage --------

    fn online_ctx(now_unix: i64) -> OnlineContext {
        OnlineContext {
            provider_version: 1,
            now_unix,
        }
    }

    fn run_record_online_attempt_marks_only_requested_capabilities(store: &dyn LibraryStore) {
        let t = track(1, "a.flac");
        store.save_track(t.clone()).expect("save");

        // Lyrics-only attempt.
        store
            .record_online_attempt(
                t.id,
                OnlineCapabilities {
                    artwork: false,
                    tags: false,
                    lyrics: true,
                },
                online_ctx(1_700_000_000),
            )
            .expect("record lyrics");

        // Should drop out of "needs lyrics" but still appear in
        // "needs artwork" — artwork was never stamped.
        let needs_lyrics = store
            .tracks_needing_online(
                OnlineCapabilities {
                    artwork: false,
                    tags: false,
                    lyrics: true,
                },
                1,
                10,
            )
            .expect("query");
        assert!(needs_lyrics.is_empty(), "lyrics attempt should be recorded");

        let needs_artwork = store
            .tracks_needing_online(
                OnlineCapabilities {
                    artwork: true,
                    tags: false,
                    lyrics: false,
                },
                1,
                10,
            )
            .expect("query");
        assert_eq!(
            needs_artwork,
            vec![t.id],
            "artwork attempt was not requested"
        );
    }

    #[test]
    fn sqlite_record_online_attempt_marks_only_requested_capabilities() {
        let store = SqliteLibraryStore::open_in_memory().expect("open");
        run_record_online_attempt_marks_only_requested_capabilities(&store);
    }

    #[test]
    fn in_memory_record_online_attempt_marks_only_requested_capabilities() {
        run_record_online_attempt_marks_only_requested_capabilities(&InMemoryLibraryStore::new());
    }

    fn run_filter_tracks_needing_online_drops_attempted_missing_and_embedded(
        store: &dyn LibraryStore,
    ) {
        // 4 tracks:
        //   alpha   - never attempted, no embedded artwork
        //   beta    - missing on disk
        //   gamma   - has embedded artwork (artwork branch must skip)
        //   delta   - already lyrics-attempted at v1
        let alpha = track(1, "alpha.flac");
        let mut beta = track(2, "beta.flac");
        beta.location = beta
            .location
            .with_availability(sustain_domain::TrackAvailability::Missing);
        let mut gamma = track(3, "gamma.flac");
        gamma.has_embedded_artwork = Some(true);
        let delta = track(4, "delta.flac");
        for t in [&alpha, &beta, &gamma, &delta] {
            store.save_track(t.clone()).expect("save");
        }

        store
            .record_online_attempt(
                delta.id,
                OnlineCapabilities {
                    artwork: false,
                    tags: false,
                    lyrics: true,
                },
                online_ctx(1_000),
            )
            .expect("record lyrics for delta");

        let all_ids = vec![alpha.id, beta.id, gamma.id, delta.id];

        // Lyrics only: alpha + gamma still need it (delta was
        // attempted, beta is missing).
        let needs_lyrics = store
            .filter_tracks_needing_online(
                &all_ids,
                OnlineCapabilities {
                    artwork: false,
                    tags: false,
                    lyrics: true,
                },
                1,
            )
            .expect("filter");
        assert_eq!(needs_lyrics, vec![alpha.id, gamma.id]);

        // Artwork only: gamma is excluded by the embedded-artwork
        // guard; beta by the missing guard; delta never attempted
        // artwork so it still needs it. Result preserves input order.
        let needs_artwork = store
            .filter_tracks_needing_online(
                &all_ids,
                OnlineCapabilities {
                    artwork: true,
                    tags: false,
                    lyrics: false,
                },
                1,
            )
            .expect("filter");
        assert_eq!(needs_artwork, vec![alpha.id, delta.id]);

        // Empty capability mask -> empty result.
        let none = store
            .filter_tracks_needing_online(&all_ids, OnlineCapabilities::default(), 1)
            .expect("filter");
        assert!(none.is_empty());

        // Empty input -> empty result.
        let none = store
            .filter_tracks_needing_online(&[], OnlineCapabilities::all(), 1)
            .expect("filter");
        assert!(none.is_empty());
    }

    #[test]
    fn sqlite_filter_tracks_needing_online_drops_attempted_missing_and_embedded() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory");
        run_filter_tracks_needing_online_drops_attempted_missing_and_embedded(&store);
    }

    #[test]
    fn in_memory_filter_tracks_needing_online_drops_attempted_missing_and_embedded() {
        run_filter_tracks_needing_online_drops_attempted_missing_and_embedded(
            &InMemoryLibraryStore::new(),
        );
    }

    fn run_online_attempts_partial_capability_preserves_other(store: &dyn LibraryStore) {
        let t = track(1, "a.flac");
        store.save_track(t.clone()).expect("save");

        store
            .record_online_attempt(
                t.id,
                OnlineCapabilities {
                    artwork: true,
                    tags: false,
                    lyrics: false,
                },
                online_ctx(1_000),
            )
            .expect("record artwork");
        store
            .record_online_attempt(
                t.id,
                OnlineCapabilities {
                    artwork: false,
                    tags: false,
                    lyrics: true,
                },
                online_ctx(2_000),
            )
            .expect("record lyrics");

        // Both artwork + lyrics attempts must be recorded; tags is
        // still pending.
        let needs = store
            .tracks_needing_online(OnlineCapabilities::all(), 1, 10)
            .expect("query");
        assert_eq!(needs, vec![t.id], "tags is still un-attempted");

        let needs_artwork = store
            .tracks_needing_online(
                OnlineCapabilities {
                    artwork: true,
                    tags: false,
                    lyrics: false,
                },
                1,
                10,
            )
            .expect("query");
        let needs_lyrics = store
            .tracks_needing_online(
                OnlineCapabilities {
                    artwork: false,
                    tags: false,
                    lyrics: true,
                },
                1,
                10,
            )
            .expect("query");
        assert!(needs_artwork.is_empty());
        assert!(needs_lyrics.is_empty());
    }

    #[test]
    fn sqlite_online_attempts_partial_capability_preserves_other() {
        let store = SqliteLibraryStore::open_in_memory().expect("open");
        run_online_attempts_partial_capability_preserves_other(&store);
    }

    #[test]
    fn in_memory_online_attempts_partial_capability_preserves_other() {
        run_online_attempts_partial_capability_preserves_other(&InMemoryLibraryStore::new());
    }

    fn run_online_query_skips_missing_tracks(store: &dyn LibraryStore) {
        let present = track(1, "present.flac");
        let mut missing = track(2, "missing.flac");
        missing.location = missing
            .location
            .with_availability(sustain_domain::TrackAvailability::Missing);
        for t in [&present, &missing] {
            store.save_track(t.clone()).expect("save");
        }
        let needs = store
            .tracks_needing_online(OnlineCapabilities::all(), 1, 10)
            .expect("query");
        assert_eq!(needs, vec![present.id]);
    }

    #[test]
    fn sqlite_online_query_skips_missing_tracks() {
        let store = SqliteLibraryStore::open_in_memory().expect("open");
        run_online_query_skips_missing_tracks(&store);
    }

    #[test]
    fn in_memory_online_query_skips_missing_tracks() {
        run_online_query_skips_missing_tracks(&InMemoryLibraryStore::new());
    }

    fn run_online_query_invalidates_stale_provider_version(store: &dyn LibraryStore) {
        let t = track(1, "a.flac");
        store.save_track(t.clone()).expect("save");
        store
            .record_online_attempt(
                t.id,
                OnlineCapabilities {
                    artwork: false,
                    tags: false,
                    lyrics: true,
                },
                OnlineContext {
                    provider_version: 1,
                    now_unix: 1_000,
                },
            )
            .expect("record");

        // Same version: track is satisfied.
        assert!(
            store
                .tracks_needing_online(
                    OnlineCapabilities {
                        artwork: false,
                        tags: false,
                        lyrics: true,
                    },
                    1,
                    10,
                )
                .expect("query")
                .is_empty()
        );

        // Newer version: track re-qualifies.
        assert_eq!(
            store
                .tracks_needing_online(
                    OnlineCapabilities {
                        artwork: false,
                        tags: false,
                        lyrics: true,
                    },
                    2,
                    10,
                )
                .expect("query"),
            vec![t.id]
        );
    }

    #[test]
    fn sqlite_online_query_invalidates_stale_provider_version() {
        let store = SqliteLibraryStore::open_in_memory().expect("open");
        run_online_query_invalidates_stale_provider_version(&store);
    }

    #[test]
    fn in_memory_online_query_invalidates_stale_provider_version() {
        run_online_query_invalidates_stale_provider_version(&InMemoryLibraryStore::new());
    }

    fn run_online_query_excludes_tracks_with_embedded_artwork(store: &dyn LibraryStore) {
        let mut with_art = track(1, "with_art.flac");
        with_art.has_embedded_artwork = Some(true);
        let mut without_art = track(2, "without_art.flac");
        without_art.has_embedded_artwork = Some(false);
        let unknown = track(3, "unknown.flac"); // has_embedded_artwork = None
        for t in [&with_art, &without_art, &unknown] {
            store.save_track(t.clone()).expect("save");
        }
        // Artwork-only request: the seeded picture excludes id 1; ids 2 and 3
        // (false and "never scanned") remain candidates.
        let mut needs = store
            .tracks_needing_online(
                OnlineCapabilities {
                    artwork: true,
                    tags: false,
                    lyrics: false,
                },
                1,
                10,
            )
            .expect("query");
        needs.sort();
        assert_eq!(needs, vec![without_art.id, unknown.id]);

        // Lyrics-only request: the artwork bit is irrelevant, so all three
        // tracks remain candidates.
        let mut needs_lyrics = store
            .tracks_needing_online(
                OnlineCapabilities {
                    artwork: false,
                    tags: false,
                    lyrics: true,
                },
                1,
                10,
            )
            .expect("query");
        needs_lyrics.sort();
        assert_eq!(needs_lyrics, vec![with_art.id, without_art.id, unknown.id]);
    }

    #[test]
    fn sqlite_online_query_excludes_tracks_with_embedded_artwork() {
        let store = SqliteLibraryStore::open_in_memory().expect("open");
        run_online_query_excludes_tracks_with_embedded_artwork(&store);
    }

    #[test]
    fn in_memory_online_query_excludes_tracks_with_embedded_artwork() {
        run_online_query_excludes_tracks_with_embedded_artwork(&InMemoryLibraryStore::new());
    }

    #[test]
    fn empty_online_capabilities_record_is_no_op() {
        let store = SqliteLibraryStore::open_in_memory().expect("open");
        let t = track(1, "a.flac");
        store.save_track(t.clone()).expect("save");
        store
            .record_online_attempt(t.id, OnlineCapabilities::none(), online_ctx(1_000))
            .expect("no-op");
        // No row → track still appears in any non-empty query.
        let needs = store
            .tracks_needing_online(OnlineCapabilities::all(), 1, 10)
            .expect("query");
        assert_eq!(needs, vec![t.id]);
    }

    #[test]
    fn sqlite_cascade_delete_clears_online_status() {
        let store = SqliteLibraryStore::open_in_memory().expect("open");
        let t = track(1, "a.flac");
        store.save_track(t.clone()).expect("save");
        store
            .record_online_attempt(
                t.id,
                OnlineCapabilities {
                    artwork: false,
                    tags: false,
                    lyrics: true,
                },
                online_ctx(1_000),
            )
            .expect("record");
        store.delete_track(t.id).expect("delete");
        // Re-add and query — the cascading delete must have dropped
        // the bookkeeping row, so the track qualifies again.
        store.save_track(t.clone()).expect("re-save");
        let needs = store
            .tracks_needing_online(OnlineCapabilities::all(), 1, 10)
            .expect("query");
        assert_eq!(needs, vec![t.id]);
    }

    // -------- end online status storage --------

    fn track(id: i64, path: &str) -> Track {
        Track {
            id: track_id(id),
            location: TrackLocation::available(relative_path(path)),
            content_hash: None,
            metadata: TrackMetadata::default(),
            rating: Rating::unrated(),
            statistics: PlayStatistics::default(),
            file_size_bytes: None,
            has_embedded_artwork: None,
        }
    }

    fn relative_path(path: &str) -> TrackRelativePath {
        TrackRelativePath::new(PathBuf::from(path)).expect("test path is relative")
    }

    fn test_hash(seed: u8) -> TrackContentHash {
        TrackContentHash::new(format!("{seed:064x}")).expect("valid test hash")
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

    fn sample_layout() -> TrackColumnLayout {
        TrackColumnLayout::new(vec![
            TrackColumnEntry {
                column_id: "track_name".to_owned(),
                visible: true,
                width_px: 240,
            },
            TrackColumnEntry {
                column_id: "artist".to_owned(),
                visible: false,
                width_px: 160,
            },
            TrackColumnEntry {
                column_id: "rating".to_owned(),
                visible: true,
                width_px: 100,
            },
        ])
    }

    #[test]
    fn in_memory_store_layout_round_trips_for_each_scope() {
        let store = InMemoryLibraryStore::new();
        let layout = sample_layout();

        for scope in [
            TrackColumnLayoutScope::Default,
            TrackColumnLayoutScope::Playlist(playlist_id(1)),
            TrackColumnLayoutScope::SmartPlaylist(smart_id(2)),
        ] {
            assert_eq!(store.load_track_column_layout(scope), Ok(None));
            assert_eq!(store.save_track_column_layout(scope, &layout), Ok(()));
            assert_eq!(
                store.load_track_column_layout(scope),
                Ok(Some(layout.clone()))
            );
            assert_eq!(store.delete_track_column_layout(scope), Ok(()));
            assert_eq!(store.load_track_column_layout(scope), Ok(None));
        }
    }

    #[test]
    fn sqlite_store_layout_round_trips_for_each_scope() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let playlist = playlist(1, "Favorites", Vec::new());
        assert_eq!(store.save_playlist(playlist.clone()), Ok(()));
        let smart = smart_playlist_with_rules(7, "Top Rated", None, 0, simple_text_rule_set());
        assert_eq!(store.save_smart_playlist(smart.clone()), Ok(()));

        let layout = sample_layout();
        for scope in [
            TrackColumnLayoutScope::Default,
            TrackColumnLayoutScope::Playlist(playlist.id),
            TrackColumnLayoutScope::SmartPlaylist(smart.id),
        ] {
            assert_eq!(store.load_track_column_layout(scope), Ok(None));
            assert_eq!(store.save_track_column_layout(scope, &layout), Ok(()));
            assert_eq!(
                store.load_track_column_layout(scope),
                Ok(Some(layout.clone()))
            );
            assert_eq!(store.delete_track_column_layout(scope), Ok(()));
            assert_eq!(store.load_track_column_layout(scope), Ok(None));
        }
    }

    #[test]
    fn sqlite_store_layout_save_replaces_existing_rows() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let scope = TrackColumnLayoutScope::Default;
        let initial = sample_layout();
        assert_eq!(store.save_track_column_layout(scope, &initial), Ok(()));

        let replacement = TrackColumnLayout::new(vec![TrackColumnEntry {
            column_id: "album".to_owned(),
            visible: true,
            width_px: 200,
        }]);
        assert_eq!(store.save_track_column_layout(scope, &replacement), Ok(()));

        assert_eq!(store.load_track_column_layout(scope), Ok(Some(replacement)));
    }

    #[test]
    fn sqlite_store_playlist_layout_cascades_on_playlist_delete() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let playlist = playlist(1, "Favorites", Vec::new());
        assert_eq!(store.save_playlist(playlist.clone()), Ok(()));
        let scope = TrackColumnLayoutScope::Playlist(playlist.id);
        assert_eq!(
            store.save_track_column_layout(scope, &sample_layout()),
            Ok(())
        );

        assert_eq!(store.delete_playlist(playlist.id), Ok(()));

        assert_eq!(store.load_track_column_layout(scope), Ok(None));
    }

    #[test]
    fn sqlite_store_smart_playlist_layout_cascades_on_smart_playlist_delete() {
        let store = SqliteLibraryStore::open_in_memory().expect("open in-memory sqlite store");
        let smart = smart_playlist_with_rules(3, "Recent", None, 0, simple_text_rule_set());
        assert_eq!(store.save_smart_playlist(smart.clone()), Ok(()));
        let scope = TrackColumnLayoutScope::SmartPlaylist(smart.id);
        assert_eq!(
            store.save_track_column_layout(scope, &sample_layout()),
            Ok(())
        );

        assert_eq!(store.delete_smart_playlist(smart.id), Ok(()));

        assert_eq!(store.load_track_column_layout(scope), Ok(None));
    }

    #[test]
    fn in_memory_store_playlist_layout_cleared_on_playlist_delete() {
        let store = InMemoryLibraryStore::new();
        let playlist = playlist(1, "Favorites", Vec::new());
        assert_eq!(store.save_playlist(playlist.clone()), Ok(()));
        let scope = TrackColumnLayoutScope::Playlist(playlist.id);
        assert_eq!(
            store.save_track_column_layout(scope, &sample_layout()),
            Ok(())
        );

        assert_eq!(store.delete_playlist(playlist.id), Ok(()));

        assert_eq!(store.load_track_column_layout(scope), Ok(None));
    }
}
