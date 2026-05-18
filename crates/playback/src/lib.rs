#![forbid(unsafe_code)]

use std::{cell::RefCell, time::Duration};

pub use xtunes_domain::{PlaybackCommand, PlaybackState, TrackPlaybackSource};

pub type PlaybackResult<T> = Result<T, PlaybackError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlaybackError {
    MissingSourcePath,
}

pub trait PlaybackService {
    fn play_track(&self, source: TrackPlaybackSource) -> PlaybackResult<()>;
    fn pause(&self) -> PlaybackResult<()>;
    fn resume(&self) -> PlaybackResult<()>;
    fn stop(&self) -> PlaybackResult<()>;
    fn seek(&self, position: Duration) -> PlaybackResult<()>;
    fn state(&self) -> PlaybackState;
}

#[derive(Debug, Default)]
pub struct NullPlaybackService {
    state: RefCell<PlaybackState>,
}

impl NullPlaybackService {
    pub fn new() -> Self {
        Self::default()
    }
}

impl PlaybackService for NullPlaybackService {
    fn play_track(&self, source: TrackPlaybackSource) -> PlaybackResult<()> {
        if source.path.as_os_str().is_empty() {
            return Err(PlaybackError::MissingSourcePath);
        }

        self.state.replace(PlaybackState::Playing {
            track_id: source.track_id,
            position: Duration::ZERO,
        });
        Ok(())
    }

    fn pause(&self) -> PlaybackResult<()> {
        let current = self.state();
        if let PlaybackState::Playing { track_id, position } = current {
            self.state
                .replace(PlaybackState::Paused { track_id, position });
        }
        Ok(())
    }

    fn resume(&self) -> PlaybackResult<()> {
        let current = self.state();
        if let PlaybackState::Paused { track_id, position } = current {
            self.state
                .replace(PlaybackState::Playing { track_id, position });
        }
        Ok(())
    }

    fn stop(&self) -> PlaybackResult<()> {
        self.state.replace(PlaybackState::Stopped);
        Ok(())
    }

    fn seek(&self, position: Duration) -> PlaybackResult<()> {
        let next = match self.state() {
            PlaybackState::Playing { track_id, .. } => {
                PlaybackState::Playing { track_id, position }
            }
            PlaybackState::Paused { track_id, .. } => PlaybackState::Paused { track_id, position },
            other => other,
        };
        self.state.replace(next);
        Ok(())
    }

    fn state(&self) -> PlaybackState {
        self.state.borrow().clone()
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use xtunes_domain::TrackId;

    use super::{NullPlaybackService, PlaybackError, PlaybackService, PlaybackState};
    use crate::TrackPlaybackSource;

    #[test]
    fn null_service_starts_stopped() {
        let playback = NullPlaybackService::new();

        assert_eq!(playback.state(), PlaybackState::Stopped);
    }

    #[test]
    fn null_service_tracks_basic_state_transitions() {
        let playback = NullPlaybackService::new();
        let track_id = positive_track_id();

        assert_eq!(
            playback.play_track(TrackPlaybackSource::new(
                track_id,
                PathBuf::from("/music/track.flac")
            )),
            Ok(())
        );
        assert_eq!(
            playback.state(),
            PlaybackState::Playing {
                track_id,
                position: Duration::ZERO
            }
        );

        assert_eq!(playback.seek(Duration::from_secs(42)), Ok(()));
        assert_eq!(
            playback.state(),
            PlaybackState::Playing {
                track_id,
                position: Duration::from_secs(42)
            }
        );

        assert_eq!(playback.pause(), Ok(()));
        assert_eq!(
            playback.state(),
            PlaybackState::Paused {
                track_id,
                position: Duration::from_secs(42)
            }
        );

        assert_eq!(playback.resume(), Ok(()));
        assert_eq!(
            playback.state(),
            PlaybackState::Playing {
                track_id,
                position: Duration::from_secs(42)
            }
        );

        assert_eq!(playback.stop(), Ok(()));
        assert_eq!(playback.state(), PlaybackState::Stopped);
    }

    #[test]
    fn null_service_rejects_missing_source_path() {
        let playback = NullPlaybackService::new();

        assert_eq!(
            playback.play_track(TrackPlaybackSource::new(
                positive_track_id(),
                PathBuf::new()
            )),
            Err(PlaybackError::MissingSourcePath)
        );
    }

    fn positive_track_id() -> TrackId {
        match TrackId::new(1) {
            Some(track_id) => track_id,
            None => unreachable!("hard-coded positive track id should be valid"),
        }
    }
}
