// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{
    collections::BTreeMap,
    sync::{Mutex, MutexGuard},
};

use crate::{
    LibraryStore, Playlist, PlaylistFolder, PlaylistFolderId, PlaylistId, SmartPlaylist,
    SmartPlaylistId, StoreError, StoreResult, Track, TrackColumnLayout, TrackColumnLayoutScope,
    TrackId,
};

#[derive(Debug, Default)]
pub struct InMemoryLibraryStore {
    tracks: Mutex<BTreeMap<TrackId, Track>>,
    playlists: Mutex<BTreeMap<PlaylistId, Playlist>>,
    folders: Mutex<BTreeMap<PlaylistFolderId, PlaylistFolder>>,
    smart_playlists: Mutex<BTreeMap<SmartPlaylistId, SmartPlaylist>>,
    default_layout: Mutex<Option<TrackColumnLayout>>,
    playlist_layouts: Mutex<BTreeMap<PlaylistId, TrackColumnLayout>>,
    smart_playlist_layouts: Mutex<BTreeMap<SmartPlaylistId, TrackColumnLayout>>,
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

    fn save_tracks(&self, tracks: &[Track]) -> StoreResult<()> {
        let mut stored_tracks = self.tracks_guard()?;
        for track in tracks {
            stored_tracks.insert(track.id, track.clone());
        }
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

    fn track_by_content_hash(
        &self,
        content_hash: &sustain_domain::TrackContentHash,
    ) -> StoreResult<Option<Track>> {
        Ok(self
            .tracks_guard()?
            .values()
            .find(|track| track.content_hash.as_ref() == Some(content_hash))
            .cloned())
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
        self.playlist_layouts
            .lock()
            .map_err(|_| StoreError::StoreUnavailable)?
            .remove(&playlist_id);
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

        let mut playlists = self.playlists_guard()?;
        let surviving_playlists: std::collections::BTreeSet<PlaylistId> = playlists
            .iter()
            .filter_map(|(id, playlist)| match playlist.parent_folder_id {
                Some(parent) if deleted.contains(&parent) => None,
                _ => Some(*id),
            })
            .collect();
        playlists.retain(|id, _| surviving_playlists.contains(id));
        drop(playlists);

        let mut smart_playlists = self.smart_playlists_guard()?;
        let surviving_smart_playlists: std::collections::BTreeSet<SmartPlaylistId> =
            smart_playlists
                .iter()
                .filter_map(|(id, smart)| match smart.parent_folder_id {
                    Some(parent) if deleted.contains(&parent) => None,
                    _ => Some(*id),
                })
                .collect();
        smart_playlists.retain(|id, _| surviving_smart_playlists.contains(id));
        drop(smart_playlists);

        self.playlist_layouts
            .lock()
            .map_err(|_| StoreError::StoreUnavailable)?
            .retain(|id, _| surviving_playlists.contains(id));
        self.smart_playlist_layouts
            .lock()
            .map_err(|_| StoreError::StoreUnavailable)?
            .retain(|id, _| surviving_smart_playlists.contains(id));
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
        self.smart_playlist_layouts
            .lock()
            .map_err(|_| StoreError::StoreUnavailable)?
            .remove(&smart_playlist_id);
        Ok(())
    }

    fn load_track_column_layout(
        &self,
        scope: TrackColumnLayoutScope,
    ) -> StoreResult<Option<TrackColumnLayout>> {
        match scope {
            TrackColumnLayoutScope::Default => Ok(self
                .default_layout
                .lock()
                .map_err(|_| StoreError::StoreUnavailable)?
                .clone()),
            TrackColumnLayoutScope::Playlist(playlist_id) => Ok(self
                .playlist_layouts
                .lock()
                .map_err(|_| StoreError::StoreUnavailable)?
                .get(&playlist_id)
                .cloned()),
            TrackColumnLayoutScope::SmartPlaylist(smart_playlist_id) => Ok(self
                .smart_playlist_layouts
                .lock()
                .map_err(|_| StoreError::StoreUnavailable)?
                .get(&smart_playlist_id)
                .cloned()),
        }
    }

    fn save_track_column_layout(
        &self,
        scope: TrackColumnLayoutScope,
        layout: &TrackColumnLayout,
    ) -> StoreResult<()> {
        match scope {
            TrackColumnLayoutScope::Default => {
                *self
                    .default_layout
                    .lock()
                    .map_err(|_| StoreError::StoreUnavailable)? = Some(layout.clone());
            }
            TrackColumnLayoutScope::Playlist(playlist_id) => {
                self.playlist_layouts
                    .lock()
                    .map_err(|_| StoreError::StoreUnavailable)?
                    .insert(playlist_id, layout.clone());
            }
            TrackColumnLayoutScope::SmartPlaylist(smart_playlist_id) => {
                self.smart_playlist_layouts
                    .lock()
                    .map_err(|_| StoreError::StoreUnavailable)?
                    .insert(smart_playlist_id, layout.clone());
            }
        }
        Ok(())
    }

    fn delete_track_column_layout(&self, scope: TrackColumnLayoutScope) -> StoreResult<()> {
        match scope {
            TrackColumnLayoutScope::Default => {
                *self
                    .default_layout
                    .lock()
                    .map_err(|_| StoreError::StoreUnavailable)? = None;
            }
            TrackColumnLayoutScope::Playlist(playlist_id) => {
                self.playlist_layouts
                    .lock()
                    .map_err(|_| StoreError::StoreUnavailable)?
                    .remove(&playlist_id);
            }
            TrackColumnLayoutScope::SmartPlaylist(smart_playlist_id) => {
                self.smart_playlist_layouts
                    .lock()
                    .map_err(|_| StoreError::StoreUnavailable)?
                    .remove(&smart_playlist_id);
            }
        }
        Ok(())
    }
}
