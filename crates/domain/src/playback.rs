use std::{path::PathBuf, time::Duration};

use crate::TrackId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlaybackCommand {
    PlayTrack(TrackId),
    Pause,
    Resume,
    TogglePlayPause,
    Stop,
    Seek(Duration),
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackPlaybackSource {
    pub track_id: TrackId,
    pub path: PathBuf,
}

impl TrackPlaybackSource {
    pub fn new(track_id: TrackId, path: impl Into<PathBuf>) -> Self {
        Self {
            track_id,
            path: path.into(),
        }
    }
}
