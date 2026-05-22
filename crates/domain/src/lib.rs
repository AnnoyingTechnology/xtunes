// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

#![forbid(unsafe_code)]

mod command;
mod id;
mod metadata;
mod playback;
mod playlist;
mod query;
mod rating;
mod settings;
mod statistics;
mod track;

pub use command::{ApplicationCommand, ApplicationQuery};
pub use id::{PlaylistId, TrackId};
pub use metadata::{FieldChange, MetadataChange, TrackMetadata};
pub use playback::{
    PlaybackCommand, PlaybackOptions, PlaybackQueue, PlaybackQueueSource, PlaybackState,
    RepeatMode, TrackPlaybackSource, VolumePercent,
};
pub use playlist::{Playlist, PlaylistEntry};
pub use query::{LibraryQuery, SortDirection, TrackSort, TrackSortColumn};
pub use rating::Rating;
pub use settings::{LibrarySettings, UserSettings};
pub use statistics::PlayStatistics;
pub use track::{Track, TrackAvailability, TrackLocation, TrackRelativePath};
