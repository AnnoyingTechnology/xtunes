use std::{path::PathBuf, time::Duration};

use crate::{
    LibraryQuery, MetadataChange, PlaybackCommand, PlaylistId, Rating, TrackId, UserSettings,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationCommand {
    Playback(PlaybackCommand),
    SetRating {
        track_id: TrackId,
        rating: Rating,
    },
    CreatePlaylist {
        name: String,
    },
    RenamePlaylist {
        playlist_id: PlaylistId,
        name: String,
    },
    DeletePlaylist {
        playlist_id: PlaylistId,
    },
    AddTracksToPlaylist {
        playlist_id: PlaylistId,
        track_ids: Vec<TrackId>,
    },
    RemoveTracksFromPlaylist {
        playlist_id: PlaylistId,
        track_ids: Vec<TrackId>,
    },
    MovePlaylistEntry {
        playlist_id: PlaylistId,
        track_id: TrackId,
        new_position: u32,
    },
    UpdateMetadata {
        track_id: TrackId,
        change: MetadataChange,
    },
    RemoveTrackFromLibrary {
        track_id: TrackId,
    },
    MoveTrackToTrash {
        track_id: TrackId,
    },
    UpdateSettings(UserSettings),
    ScanLibrary {
        library_path: PathBuf,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationQuery {
    ListTracks(LibraryQuery),
    ListPlaylists,
    TrackDetails(TrackId),
    SearchTracks {
        search_text: String,
    },
    PlayStatistics(TrackId),
    CurrentPlaybackState,
    Settings,
    TotalDuration(LibraryQuery),
    SelectionDuration {
        track_ids: Vec<TrackId>,
    },
    PlaybackPosition {
        track_id: TrackId,
        position: Duration,
    },
}
