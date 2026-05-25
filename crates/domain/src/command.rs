// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{path::PathBuf, time::Duration};

use crate::{
    LibraryQuery, MetadataChange, PlaybackCommand, PlaylistFolderId, PlaylistId, PlaylistItem,
    Rating, SmartPlaylistId, SmartPlaylistRuleSet, TrackId, UserSettings,
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
        parent_folder_id: Option<PlaylistFolderId>,
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
    /// Reorder one or more tracks within an existing playlist.
    ///
    /// The moved tracks are extracted from the playlist (preserving the
    /// authoritative entry order among themselves), then re-inserted as a
    /// single contiguous block at `new_position`. `new_position` is the
    /// insertion index in the *post-removal* entries list and is clamped to
    /// the list's length, so the saturating `u32::MAX` value lands the
    /// block at the tail.
    MovePlaylistEntries {
        playlist_id: PlaylistId,
        track_ids: Vec<TrackId>,
        new_position: u32,
    },
    CreatePlaylistFolder {
        name: String,
        parent_folder_id: Option<PlaylistFolderId>,
    },
    RenamePlaylistFolder {
        folder_id: PlaylistFolderId,
        name: String,
    },
    DeletePlaylistFolder {
        folder_id: PlaylistFolderId,
    },
    CreateSmartPlaylist {
        name: String,
        parent_folder_id: Option<PlaylistFolderId>,
        rules: SmartPlaylistRuleSet,
    },
    UpdateSmartPlaylist {
        smart_playlist_id: SmartPlaylistId,
        name: String,
        rules: SmartPlaylistRuleSet,
    },
    DeleteSmartPlaylist {
        smart_playlist_id: SmartPlaylistId,
    },
    MovePlaylistItem {
        item: PlaylistItem,
        target_parent_folder_id: Option<PlaylistFolderId>,
        position: u32,
    },
    UpdateMetadata {
        track_id: TrackId,
        change: Box<MetadataChange>,
    },
    ResetPlayCount {
        track_id: TrackId,
    },
    SetArtwork {
        track_id: TrackId,
        artwork: Option<Vec<u8>>,
    },
    /// Trigger an explicit remote artwork lookup for `track_id`.
    /// The runtime enqueues the work on the background fetcher and
    /// returns immediately; the outcome is delivered later through
    /// the runtime's artwork-fetch result sink.
    FetchArtwork {
        track_id: TrackId,
    },
    RemoveTrackFromLibrary {
        track_id: TrackId,
    },
    MoveTrackToTrash {
        track_id: TrackId,
    },
    AddExternalLibraryItems {
        paths: Vec<PathBuf>,
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
