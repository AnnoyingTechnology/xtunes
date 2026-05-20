use std::{path::PathBuf, time::Duration};

use crate::TrackId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlaybackCommand {
    PlayTrack(TrackId),
    PlayPreviousTrack,
    PlayNextTrack,
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PlaybackOptions {
    pub shuffle_enabled: bool,
    pub repeat_enabled: bool,
}

impl PlaybackOptions {
    pub const fn with_shuffle_toggled(self) -> Self {
        Self {
            shuffle_enabled: !self.shuffle_enabled,
            repeat_enabled: self.repeat_enabled,
        }
    }

    pub const fn with_repeat_toggled(self) -> Self {
        Self {
            shuffle_enabled: self.shuffle_enabled,
            repeat_enabled: !self.repeat_enabled,
        }
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VolumePercent(u8);

impl VolumePercent {
    pub const MAX: u8 = 100;

    pub const fn new(value: u8) -> Option<Self> {
        if value <= Self::MAX {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn from_clamped(value: u8) -> Self {
        if value > Self::MAX {
            Self(Self::MAX)
        } else {
            Self(value)
        }
    }

    pub fn from_scalar(value: f64) -> Self {
        if !value.is_finite() {
            return Self(0);
        }

        let percent = (value.clamp(0.0, 1.0) * f64::from(Self::MAX)).round();
        Self(percent as u8)
    }

    pub const fn get(self) -> u8 {
        self.0
    }

    pub fn as_scalar(self) -> f64 {
        f64::from(self.0) / f64::from(Self::MAX)
    }
}

impl Default for VolumePercent {
    fn default() -> Self {
        Self(Self::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::{PlaybackOptions, VolumePercent};

    #[test]
    fn volume_percent_accepts_only_percent_range() {
        assert_eq!(VolumePercent::new(100).map(VolumePercent::get), Some(100));
        assert_eq!(VolumePercent::new(101), None);
    }

    #[test]
    fn volume_percent_converts_from_scalar() {
        assert_eq!(VolumePercent::from_scalar(0.425).get(), 43);
        assert_eq!(VolumePercent::from_scalar(2.0).get(), 100);
        assert_eq!(VolumePercent::from_scalar(f64::NAN).get(), 0);
    }

    #[test]
    fn playback_options_toggle_shuffle_without_affecting_repeat() {
        let options = PlaybackOptions {
            shuffle_enabled: false,
            repeat_enabled: true,
        };

        assert_eq!(
            options.with_shuffle_toggled(),
            PlaybackOptions {
                shuffle_enabled: true,
                repeat_enabled: true,
            }
        );
    }

    #[test]
    fn playback_options_toggle_repeat_without_affecting_shuffle() {
        let options = PlaybackOptions {
            shuffle_enabled: true,
            repeat_enabled: false,
        };

        assert_eq!(
            options.with_repeat_toggled(),
            PlaybackOptions {
                shuffle_enabled: true,
                repeat_enabled: true,
            }
        );
    }
}
