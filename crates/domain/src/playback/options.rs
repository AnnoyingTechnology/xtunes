// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RepeatMode {
    #[default]
    Off,
    One,
    All,
}

impl RepeatMode {
    pub const fn is_enabled(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub const fn toggled_for_single_button(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::One | Self::All => Self::Off,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PlaybackOptions {
    pub shuffle_enabled: bool,
    pub repeat_mode: RepeatMode,
}

impl PlaybackOptions {
    pub const fn with_shuffle_toggled(self) -> Self {
        Self {
            shuffle_enabled: !self.shuffle_enabled,
            repeat_mode: self.repeat_mode,
        }
    }

    pub const fn with_shuffle_enabled(self, shuffle_enabled: bool) -> Self {
        Self {
            shuffle_enabled,
            repeat_mode: self.repeat_mode,
        }
    }

    pub const fn with_repeat_toggled(self) -> Self {
        Self {
            shuffle_enabled: self.shuffle_enabled,
            repeat_mode: self.repeat_mode.toggled_for_single_button(),
        }
    }

    pub const fn repeat_enabled(self) -> bool {
        self.repeat_mode.is_enabled()
    }
}

#[cfg(test)]
mod tests {
    use super::{PlaybackOptions, RepeatMode};

    #[test]
    fn playback_options_toggle_shuffle_without_affecting_repeat() {
        let options = PlaybackOptions {
            shuffle_enabled: false,
            repeat_mode: RepeatMode::All,
        };

        assert_eq!(
            options.with_shuffle_toggled(),
            PlaybackOptions {
                shuffle_enabled: true,
                repeat_mode: RepeatMode::All,
            }
        );
    }

    #[test]
    fn playback_options_toggle_repeat_without_affecting_shuffle() {
        let options = PlaybackOptions {
            shuffle_enabled: true,
            repeat_mode: RepeatMode::Off,
        };

        assert_eq!(
            options.with_repeat_toggled(),
            PlaybackOptions {
                shuffle_enabled: true,
                repeat_mode: RepeatMode::All,
            }
        );
    }

    #[test]
    fn playback_options_sets_shuffle_without_affecting_repeat() {
        let options = PlaybackOptions {
            shuffle_enabled: true,
            repeat_mode: RepeatMode::All,
        };

        assert_eq!(
            options.with_shuffle_enabled(false),
            PlaybackOptions {
                shuffle_enabled: false,
                repeat_mode: RepeatMode::All,
            }
        );
    }

    #[test]
    fn playback_options_toggle_repeat_disables_repeat_one() {
        let options = PlaybackOptions {
            shuffle_enabled: false,
            repeat_mode: RepeatMode::One,
        };

        assert_eq!(
            options.with_repeat_toggled(),
            PlaybackOptions {
                shuffle_enabled: false,
                repeat_mode: RepeatMode::Off,
            }
        );
    }
}
