// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! SQLite `LibraryStore` operations for the cached smart-shuffle index.

use super::*;

pub(super) fn save_smart_shuffle_index(
    connection: &Connection,
    index: &StoredSmartShuffleIndex,
) -> StoreResult<()> {
    connection
        .execute(
            UPSERT_SMART_SHUFFLE_INDEX_SQL,
            params![index.index_blob, i64::from(index.schema_version)],
        )
        .map(|_| ())
        .map_err(StoreError::from)
}

pub(super) fn load_smart_shuffle_index(
    connection: &Connection,
) -> StoreResult<Option<StoredSmartShuffleIndex>> {
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

pub(super) fn clear_smart_shuffle_index(connection: &Connection) -> StoreResult<()> {
    connection
        .execute(DELETE_SMART_SHUFFLE_INDEX_SQL, [])
        .map(|_| ())
        .map_err(StoreError::from)
}
