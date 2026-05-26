// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::time::Duration;

use crate::TrackId;

mod options;
mod queue;
mod shuffle;
mod source;
mod volume;

pub use options::{PlaybackOptions, RepeatMode};
pub use queue::{PlaybackQueue, PlaybackQueueRequest, PlaybackQueueSource};
pub use source::TrackPlaybackSource;
pub use volume::VolumePercent;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlaybackCommand {
    /// Start playback at `track_id` and set the play queue from `queue`.
    /// The queue request is part of the command — not derived inside the
    /// runtime — so the caller (UI / MPRIS / test) decides what context
    /// the activation runs in. Activating a track from the Songs view
    /// passes [`PlaybackQueueRequest::Library`]; activating from a
    /// playlist passes [`PlaybackQueueRequest::Explicit`] with the
    /// playlist's track ids so auto-advance stays within the playlist.
    PlayTrack {
        track_id: TrackId,
        queue: PlaybackQueueRequest,
    },
    PlayPreviousTrack,
    /// Auto-advance to the next track. Used by the GStreamer EOS callback
    /// when the current track ends naturally. NOT a user-initiated skip;
    /// does not affect skip statistics.
    PlayNextTrack,
    /// User-initiated skip of the currently playing track. Counts as a
    /// skip (increments `skip_count`, sets `last_skipped_at`) when the
    /// play threshold has not yet been reached, then advances to the
    /// next track. Dispatched by the titlebar Next button and any other
    /// surface where the user is explicitly choosing to abandon the
    /// current track in favor of the next one (e.g. media-key Next).
    SkipCurrentTrack,
    EnqueueNext(Vec<TrackId>),
    /// Append the given tracks to the tail of the play queue, behind every
    /// already-queued track. Counterpart to [`Self::EnqueueNext`], which
    /// inserts at the head right after the currently playing track.
    EnqueueLast(Vec<TrackId>),
    ToggleShuffle,
    ToggleRepeat,
    Pause,
    Resume,
    TogglePlayPause,
    Stop,
    Seek(Duration),
    SetVolume(VolumePercent),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum PlaybackState {
    #[default]
    Stopped,
    Loading {
        track_id: TrackId,
    },
    Playing {
        track_id: TrackId,
        position: Duration,
    },
    Paused {
        track_id: TrackId,
        position: Duration,
    },
}
