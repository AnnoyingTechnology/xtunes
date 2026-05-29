// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! SQLite `LibraryStore` operations for synced lyrics.

use super::*;

pub(super) fn record_synced_lyrics(
    connection: &Connection,
    track_id: TrackId,
    lyrics: &SyncedLyrics,
    source: &str,
) -> StoreResult<()> {
    if lyrics.is_empty() {
        return Ok(());
    }
    let json = serde_json::to_string(&lyrics.lines)
        .map_err(|error| StoreError::Database(error.to_string()))?;
    connection
        .execute(
            UPSERT_TRACK_SYNCED_LYRICS_SQL,
            params![track_id.get(), json, source],
        )
        .map(|_| ())
        .map_err(StoreError::from)
}

pub(super) fn load_synced_lyrics(
    connection: &Connection,
    track_id: TrackId,
) -> StoreResult<Option<StoredSyncedLyrics>> {
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

pub(super) fn clear_synced_lyrics(connection: &Connection, track_id: TrackId) -> StoreResult<()> {
    connection
        .execute(DELETE_TRACK_SYNCED_LYRICS_SQL, params![track_id.get()])
        .map(|_| ())
        .map_err(StoreError::from)
}
