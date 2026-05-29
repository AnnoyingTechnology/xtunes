// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! SQLite `LibraryStore` operations for track column layouts.

use super::*;

pub(super) fn load_track_column_layout(
    connection: &Connection,
    scope: TrackColumnLayoutScope,
) -> StoreResult<Option<TrackColumnLayout>> {
    let entries = match scope {
        TrackColumnLayoutScope::Default => load_layout_rows(
            connection,
            "SELECT column_id, visible, width_px \
                 FROM track_column_layout_default \
                 ORDER BY position",
            params![],
        )?,
        TrackColumnLayoutScope::Playlist(playlist_id) => load_layout_rows(
            connection,
            "SELECT column_id, visible, width_px \
                 FROM track_column_layout_playlist_override \
                 WHERE playlist_id = ?1 \
                 ORDER BY position",
            params![playlist_id.get()],
        )?,
        TrackColumnLayoutScope::SmartPlaylist(smart_playlist_id) => load_layout_rows(
            connection,
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

pub(super) fn save_track_column_layout(
    connection: &mut Connection,
    scope: TrackColumnLayoutScope,
    layout: &TrackColumnLayout,
) -> StoreResult<()> {
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

pub(super) fn delete_track_column_layout(
    connection: &Connection,
    scope: TrackColumnLayoutScope,
) -> StoreResult<()> {
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
