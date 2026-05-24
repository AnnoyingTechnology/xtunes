// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

#![forbid(unsafe_code)]

mod command;
mod id;
mod managed_library;
mod metadata;
mod playback;
mod playlist;
mod playlist_folder;
mod query;
mod rating;
mod settings;
mod smart_playlist;
mod smart_playlist_evaluation;
mod statistics;
mod track;

pub use command::{ApplicationCommand, ApplicationQuery};
pub use id::{PlaylistFolderId, PlaylistId, SmartPlaylistId, TrackId};
pub use managed_library::{
    ManagedTrackPathError, ManagedTrackPathInput, ManagedTrackPathPlan, ManagedTrackPathPlanner,
};
pub use metadata::{FieldChange, MetadataChange, TrackMetadata};
pub use playback::{
    PlaybackCommand, PlaybackOptions, PlaybackQueue, PlaybackQueueSource, PlaybackState,
    RepeatMode, TrackPlaybackSource, VolumePercent,
};
pub use playlist::{Playlist, PlaylistEntry};
pub use playlist_folder::{PlaylistFolder, PlaylistItem};
pub use query::{LibraryQuery, SortDirection, TrackSort, TrackSortColumn};
pub use rating::Rating;
pub use settings::{LibraryManagementMode, LibrarySettings, UserSettings};
pub use smart_playlist::{
    SmartPlaylist, SmartPlaylistDateField, SmartPlaylistLimit, SmartPlaylistLimitSelection,
    SmartPlaylistMatchKind, SmartPlaylistNumberField, SmartPlaylistNumberOperator,
    SmartPlaylistRule, SmartPlaylistRuleSet, SmartPlaylistTextField, SmartPlaylistTextOperator,
};
pub use smart_playlist_evaluation::{matching_tracks, track_matches_rule, track_matches_rule_set};
pub use statistics::PlayStatistics;
pub use track::{Track, TrackAvailability, TrackContentHash, TrackLocation, TrackRelativePath};
