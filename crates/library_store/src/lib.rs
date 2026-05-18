#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    sync::{Mutex, MutexGuard},
};

pub use xtunes_domain::{LibraryQuery, Playlist, PlaylistId, Rating, Track, TrackId};

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreError {
    StoreUnavailable,
}

pub trait LibraryStore {
    fn save_track(&self, track: Track) -> StoreResult<()>;
    fn track(&self, track_id: TrackId) -> StoreResult<Option<Track>>;
    fn tracks(&self) -> StoreResult<Vec<Track>>;
    fn save_playlist(&self, playlist: Playlist) -> StoreResult<()>;
    fn playlist(&self, playlist_id: PlaylistId) -> StoreResult<Option<Playlist>>;
    fn playlists(&self) -> StoreResult<Vec<Playlist>>;
}

#[derive(Debug, Default)]
pub struct InMemoryLibraryStore {
    tracks: Mutex<BTreeMap<TrackId, Track>>,
    playlists: Mutex<BTreeMap<PlaylistId, Playlist>>,
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

    use super::{InMemoryLibraryStore, LibraryQuery, LibraryStore, Playlist, Track};
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
