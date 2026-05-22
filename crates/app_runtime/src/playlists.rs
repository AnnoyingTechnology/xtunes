use std::collections::BTreeSet;

use xtunes_domain::{Playlist, PlaylistEntry, PlaylistId, TrackId};
use xtunes_library_store::LibraryStore;

use crate::{ApplicationRuntime, ApplicationRuntimeError, ApplicationRuntimeResult};

impl ApplicationRuntime {
    pub(super) fn create_playlist(&mut self, name: String) -> ApplicationRuntimeResult<()> {
        let name = normalized_playlist_name(name)?;
        let library_store = self.library_store()?;
        let playlists = library_store
            .playlists()
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        let playlist = Playlist {
            id: next_playlist_id(&playlists)?,
            name,
            entries: Vec::new(),
        };
        library_store
            .save_playlist(playlist)
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        self.reload_playlists()
    }

    pub(super) fn rename_playlist(
        &mut self,
        playlist_id: PlaylistId,
        name: String,
    ) -> ApplicationRuntimeResult<()> {
        let name = normalized_playlist_name(name)?;
        let library_store = self.library_store()?;
        let Some(mut playlist) = library_store
            .playlist(playlist_id)
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?
        else {
            return Err(ApplicationRuntimeError::PlaylistNotFound);
        };

        playlist.name = name;
        library_store
            .save_playlist(playlist)
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        self.reload_playlists()
    }

    pub(super) fn delete_playlist(
        &mut self,
        playlist_id: PlaylistId,
    ) -> ApplicationRuntimeResult<()> {
        let library_store = self.library_store()?;
        if library_store
            .playlist(playlist_id)
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?
            .is_none()
        {
            return Err(ApplicationRuntimeError::PlaylistNotFound);
        }

        library_store
            .delete_playlist(playlist_id)
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        self.reload_playlists()
    }

    pub(super) fn add_tracks_to_playlist(
        &mut self,
        playlist_id: PlaylistId,
        track_ids: Vec<TrackId>,
    ) -> ApplicationRuntimeResult<()> {
        self.ensure_tracks_are_in_library(&track_ids)?;
        let library_store = self.library_store()?;
        let Some(mut playlist) = library_store
            .playlist(playlist_id)
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?
        else {
            return Err(ApplicationRuntimeError::PlaylistNotFound);
        };

        let mut existing_track_ids = playlist
            .entries
            .iter()
            .map(|entry| entry.track_id)
            .collect::<BTreeSet<_>>();
        for track_id in unique_track_ids(track_ids) {
            if existing_track_ids.insert(track_id) {
                playlist.entries.push(PlaylistEntry {
                    playlist_id,
                    track_id,
                    position: playlist.entries.len() as u32,
                });
            }
        }
        reindex_playlist_entries(&mut playlist);

        library_store
            .save_playlist(playlist)
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        self.reload_playlists()
    }

    pub(super) fn remove_tracks_from_playlist(
        &mut self,
        playlist_id: PlaylistId,
        track_ids: Vec<TrackId>,
    ) -> ApplicationRuntimeResult<()> {
        let library_store = self.library_store()?;
        let Some(mut playlist) = library_store
            .playlist(playlist_id)
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?
        else {
            return Err(ApplicationRuntimeError::PlaylistNotFound);
        };

        let removed_track_ids = track_ids.into_iter().collect::<BTreeSet<_>>();
        playlist
            .entries
            .retain(|entry| !removed_track_ids.contains(&entry.track_id));
        reindex_playlist_entries(&mut playlist);

        library_store
            .save_playlist(playlist)
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        self.reload_playlists()
    }

    pub(super) fn move_playlist_entry(
        &mut self,
        playlist_id: PlaylistId,
        track_id: TrackId,
        new_position: u32,
    ) -> ApplicationRuntimeResult<()> {
        let library_store = self.library_store()?;
        let Some(mut playlist) = library_store
            .playlist(playlist_id)
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?
        else {
            return Err(ApplicationRuntimeError::PlaylistNotFound);
        };
        let Some(current_index) = playlist
            .entries
            .iter()
            .position(|entry| entry.track_id == track_id)
        else {
            return Err(ApplicationRuntimeError::PlaylistEntryNotFound);
        };

        let entry = playlist.entries.remove(current_index);
        let target_index = usize::try_from(new_position)
            .unwrap_or(usize::MAX)
            .min(playlist.entries.len());
        playlist.entries.insert(target_index, entry);
        reindex_playlist_entries(&mut playlist);

        library_store
            .save_playlist(playlist)
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        self.reload_playlists()
    }

    fn library_store(&self) -> ApplicationRuntimeResult<&dyn LibraryStore> {
        self.library_store
            .as_deref()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)
    }

    fn reload_playlists(&mut self) -> ApplicationRuntimeResult<()> {
        let playlists = self
            .library_store()?
            .playlists()
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        self.playlists = playlists;
        Ok(())
    }

    fn ensure_tracks_are_in_library(&self, track_ids: &[TrackId]) -> ApplicationRuntimeResult<()> {
        let library_track_ids = self
            .library_tracks
            .iter()
            .map(|track| track.id)
            .collect::<BTreeSet<_>>();
        if track_ids
            .iter()
            .all(|track_id| library_track_ids.contains(track_id))
        {
            Ok(())
        } else {
            Err(ApplicationRuntimeError::TrackUnavailable)
        }
    }
}

fn normalized_playlist_name(name: String) -> ApplicationRuntimeResult<String> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        Err(ApplicationRuntimeError::InvalidPlaylistName)
    } else {
        Ok(name)
    }
}

fn next_playlist_id(playlists: &[Playlist]) -> ApplicationRuntimeResult<PlaylistId> {
    let next_id = playlists
        .iter()
        .map(|playlist| playlist.id.get())
        .max()
        .unwrap_or_default()
        .checked_add(1)
        .and_then(PlaylistId::new)
        .ok_or(ApplicationRuntimeError::LibraryStoreFailed)?;
    Ok(next_id)
}

fn unique_track_ids(track_ids: Vec<TrackId>) -> Vec<TrackId> {
    let mut seen = BTreeSet::new();
    let mut unique = Vec::new();
    for track_id in track_ids {
        if seen.insert(track_id) {
            unique.push(track_id);
        }
    }
    unique
}

fn reindex_playlist_entries(playlist: &mut Playlist) {
    for (position, entry) in playlist.entries.iter_mut().enumerate() {
        entry.playlist_id = playlist.id;
        entry.position = position as u32;
    }
}
