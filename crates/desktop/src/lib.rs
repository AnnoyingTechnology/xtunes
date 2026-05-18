#![forbid(unsafe_code)]

use std::sync::{Mutex, MutexGuard};

pub use xtunes_app_runtime::{ApplicationCommand, ApplicationQuery, PlaybackState, TrackId};

pub type DesktopResult<T> = Result<T, DesktopError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesktopError {
    IntegrationUnavailable,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NowPlayingMetadata {
    pub track_id: Option<TrackId>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}

pub trait DesktopIntegration {
    fn publish_playback_state(&self, state: PlaybackState) -> DesktopResult<()>;
    fn publish_now_playing(&self, metadata: NowPlayingMetadata) -> DesktopResult<()>;
}

#[derive(Debug, Default)]
pub struct NullDesktopIntegration {
    playback_state: Mutex<PlaybackState>,
    now_playing: Mutex<NowPlayingMetadata>,
}

impl NullDesktopIntegration {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn playback_state(&self) -> DesktopResult<PlaybackState> {
        Ok(self.playback_state_guard()?.clone())
    }

    pub fn now_playing(&self) -> DesktopResult<NowPlayingMetadata> {
        Ok(self.now_playing_guard()?.clone())
    }

    fn playback_state_guard(&self) -> DesktopResult<MutexGuard<'_, PlaybackState>> {
        self.playback_state
            .lock()
            .map_err(|_| DesktopError::IntegrationUnavailable)
    }

    fn now_playing_guard(&self) -> DesktopResult<MutexGuard<'_, NowPlayingMetadata>> {
        self.now_playing
            .lock()
            .map_err(|_| DesktopError::IntegrationUnavailable)
    }
}

impl DesktopIntegration for NullDesktopIntegration {
    fn publish_playback_state(&self, state: PlaybackState) -> DesktopResult<()> {
        *self.playback_state_guard()? = state;
        Ok(())
    }

    fn publish_now_playing(&self, metadata: NowPlayingMetadata) -> DesktopResult<()> {
        *self.now_playing_guard()? = metadata;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{DesktopIntegration, NowPlayingMetadata, NullDesktopIntegration, PlaybackState};

    #[test]
    fn null_desktop_integration_starts_stopped() {
        let desktop = NullDesktopIntegration::new();

        assert_eq!(desktop.playback_state(), Ok(PlaybackState::Stopped));
        assert_eq!(desktop.now_playing(), Ok(NowPlayingMetadata::default()));
    }

    #[test]
    fn null_desktop_integration_records_published_state() {
        let desktop = NullDesktopIntegration::new();
        let metadata = NowPlayingMetadata {
            title: Some("Angel".to_owned()),
            artist: Some("Massive Attack".to_owned()),
            ..NowPlayingMetadata::default()
        };

        assert_eq!(
            desktop.publish_playback_state(PlaybackState::Stopped),
            Ok(())
        );
        assert_eq!(desktop.publish_now_playing(metadata.clone()), Ok(()));

        assert_eq!(desktop.playback_state(), Ok(PlaybackState::Stopped));
        assert_eq!(desktop.now_playing(), Ok(metadata));
    }
}
