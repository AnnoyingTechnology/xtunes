// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{path::PathBuf, time::Duration};

use crate::{PlaylistId, TrackId};

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
    pub repeat_mode: RepeatMode,
}

impl PlaybackOptions {
    pub const fn with_shuffle_toggled(self) -> Self {
        Self {
            shuffle_enabled: !self.shuffle_enabled,
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum PlaybackQueueSource {
    #[default]
    Library,
    Album,
    Playlist(PlaylistId),
    SearchResults,
    Selection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaybackQueue {
    source: PlaybackQueueSource,
    ordered_track_ids: Vec<TrackId>,
    play_order_track_ids: Vec<TrackId>,
    current_track_id: Option<TrackId>,
    options: PlaybackOptions,
}

impl PlaybackQueue {
    pub fn new(
        source: PlaybackQueueSource,
        ordered_track_ids: Vec<TrackId>,
        current_track_id: TrackId,
        options: PlaybackOptions,
        shuffle_seed: u64,
    ) -> Self {
        let current_track_id = ordered_track_ids
            .contains(&current_track_id)
            .then_some(current_track_id);
        let play_order_track_ids =
            play_order_track_ids(&ordered_track_ids, current_track_id, options, shuffle_seed);

        Self {
            source,
            ordered_track_ids,
            play_order_track_ids,
            current_track_id,
            options,
        }
    }

    pub fn empty(options: PlaybackOptions) -> Self {
        Self {
            source: PlaybackQueueSource::Library,
            ordered_track_ids: Vec::new(),
            play_order_track_ids: Vec::new(),
            current_track_id: None,
            options,
        }
    }

    pub fn source(&self) -> &PlaybackQueueSource {
        &self.source
    }

    pub fn ordered_track_ids(&self) -> &[TrackId] {
        &self.ordered_track_ids
    }

    pub fn play_order_track_ids(&self) -> &[TrackId] {
        &self.play_order_track_ids
    }

    pub fn current_track_id(&self) -> Option<TrackId> {
        self.current_track_id
    }

    pub fn options(&self) -> PlaybackOptions {
        self.options
    }

    pub fn toggle_shuffle(&mut self, shuffle_seed: u64) {
        self.options = self.options.with_shuffle_toggled();
        self.rebuild_play_order(shuffle_seed);
    }

    pub fn toggle_repeat_mode(&mut self) {
        self.options = self.options.with_repeat_toggled();
    }

    pub fn set_repeat_mode(&mut self, repeat_mode: RepeatMode) {
        self.options.repeat_mode = repeat_mode;
    }

    pub fn next_track_id(&self) -> Option<TrackId> {
        self.adjacent_track_id(TrackStep::Next)
    }

    pub fn previous_track_id(&self) -> Option<TrackId> {
        self.adjacent_track_id(TrackStep::Previous)
    }

    pub fn move_to_track(&mut self, track_id: TrackId) -> bool {
        if !self.ordered_track_ids.contains(&track_id) {
            return false;
        }

        self.current_track_id = Some(track_id);
        true
    }

    pub fn replace_ordered_track_ids(
        &mut self,
        ordered_track_ids: Vec<TrackId>,
        shuffle_seed: u64,
    ) {
        let current_track_id = self
            .current_track_id
            .filter(|track_id| ordered_track_ids.contains(track_id));

        self.ordered_track_ids = ordered_track_ids;
        self.current_track_id = current_track_id;
        self.rebuild_play_order(shuffle_seed);
    }

    pub fn remove_track(&mut self, track_id: TrackId, shuffle_seed: u64) {
        let ordered_track_ids = self
            .ordered_track_ids
            .iter()
            .copied()
            .filter(|candidate| *candidate != track_id)
            .collect();
        self.replace_ordered_track_ids(ordered_track_ids, shuffle_seed);
    }

    fn adjacent_track_id(&self, step: TrackStep) -> Option<TrackId> {
        let current_track_id = self.current_track_id?;
        if self.options.repeat_mode == RepeatMode::One {
            return Some(current_track_id);
        }

        let current_index = self
            .play_order_track_ids
            .iter()
            .position(|track_id| *track_id == current_track_id)?;
        let adjacent_index = match step {
            TrackStep::Previous => current_index.checked_sub(1),
            TrackStep::Next => current_index.checked_add(1),
        };

        match adjacent_index.and_then(|index| self.play_order_track_ids.get(index).copied()) {
            Some(track_id) => Some(track_id),
            None if self.options.repeat_mode == RepeatMode::All => match step {
                TrackStep::Previous => self.play_order_track_ids.last().copied(),
                TrackStep::Next => self.play_order_track_ids.first().copied(),
            },
            None => None,
        }
    }

    fn rebuild_play_order(&mut self, shuffle_seed: u64) {
        self.play_order_track_ids = play_order_track_ids(
            &self.ordered_track_ids,
            self.current_track_id,
            self.options,
            shuffle_seed,
        );
    }
}

impl Default for PlaybackQueue {
    fn default() -> Self {
        Self::empty(PlaybackOptions::default())
    }
}

#[derive(Clone, Copy)]
enum TrackStep {
    Previous,
    Next,
}

fn play_order_track_ids(
    ordered_track_ids: &[TrackId],
    current_track_id: Option<TrackId>,
    options: PlaybackOptions,
    shuffle_seed: u64,
) -> Vec<TrackId> {
    let mut track_ids = if options.shuffle_enabled {
        shuffled_track_ids(ordered_track_ids, shuffle_seed)
    } else {
        ordered_track_ids.to_vec()
    };

    if options.shuffle_enabled
        && let Some(current_track_id) = current_track_id
    {
        if let Some(current_index) = track_ids
            .iter()
            .position(|track_id| *track_id == current_track_id)
        {
            track_ids.rotate_left(current_index);
        }
    }

    track_ids
}

fn shuffled_track_ids(track_ids: &[TrackId], shuffle_seed: u64) -> Vec<TrackId> {
    let mut shuffled = track_ids.to_vec();
    let mut random = SplitMix64::new(shuffle_seed);

    for index in (1..shuffled.len()).rev() {
        let swap_index = random.next_index(index + 1);
        shuffled.swap(index, swap_index);
    }

    shuffled
}

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D049BB133111EB);
        value ^ (value >> 31)
    }

    fn next_index(&mut self, upper_bound: usize) -> usize {
        let upper_bound = upper_bound as u64;
        let rejection_threshold = u64::MAX - (u64::MAX % upper_bound);

        loop {
            let value = self.next_u64();
            if value < rejection_threshold {
                return (value % upper_bound) as usize;
            }
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
    use crate::TrackId;

    use super::{PlaybackOptions, PlaybackQueue, PlaybackQueueSource, RepeatMode, VolumePercent};

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

    #[test]
    fn playback_queue_advances_in_normal_order() {
        let queue = queue_with_options(track_id(2), PlaybackOptions::default());

        assert_eq!(queue.previous_track_id(), Some(track_id(1)));
        assert_eq!(queue.next_track_id(), Some(track_id(3)));
    }

    #[test]
    fn playback_queue_stops_at_edges_when_repeat_is_off() {
        let first_queue = queue_with_options(track_id(1), PlaybackOptions::default());
        let last_queue = queue_with_options(track_id(3), PlaybackOptions::default());

        assert_eq!(first_queue.previous_track_id(), None);
        assert_eq!(last_queue.next_track_id(), None);
    }

    #[test]
    fn playback_queue_wraps_at_edges_when_repeat_all_is_enabled() {
        let options = PlaybackOptions {
            shuffle_enabled: false,
            repeat_mode: RepeatMode::All,
        };
        let first_queue = queue_with_options(track_id(1), options);
        let last_queue = queue_with_options(track_id(3), options);

        assert_eq!(first_queue.previous_track_id(), Some(track_id(3)));
        assert_eq!(last_queue.next_track_id(), Some(track_id(1)));
    }

    #[test]
    fn playback_queue_repeats_current_track_when_repeat_one_is_enabled() {
        let queue = queue_with_options(
            track_id(2),
            PlaybackOptions {
                shuffle_enabled: false,
                repeat_mode: RepeatMode::One,
            },
        );

        assert_eq!(queue.previous_track_id(), Some(track_id(2)));
        assert_eq!(queue.next_track_id(), Some(track_id(2)));
    }

    #[test]
    fn playback_queue_uses_shuffle_order_with_current_track_first() {
        let queue = queue_with_options(
            track_id(3),
            PlaybackOptions {
                shuffle_enabled: true,
                repeat_mode: RepeatMode::Off,
            },
        );

        assert_eq!(queue.play_order_track_ids().first(), Some(&track_id(3)));
        assert_ne!(queue.play_order_track_ids(), queue.ordered_track_ids());
        assert_eq!(
            queue.next_track_id(),
            queue.play_order_track_ids().get(1).copied()
        );
    }

    #[test]
    fn playback_queue_drops_missing_track_ids_when_order_is_replaced() {
        let mut queue = queue_with_options(track_id(2), PlaybackOptions::default());

        queue.replace_ordered_track_ids(vec![track_id(1), track_id(3)], 10);

        assert_eq!(queue.current_track_id(), None);
        assert_eq!(queue.next_track_id(), None);
        assert_eq!(queue.ordered_track_ids(), &[track_id(1), track_id(3)]);
    }

    fn queue_with_options(current_track_id: TrackId, options: PlaybackOptions) -> PlaybackQueue {
        PlaybackQueue::new(
            PlaybackQueueSource::Library,
            vec![track_id(1), track_id(2), track_id(3)],
            current_track_id,
            options,
            10,
        )
    }

    fn track_id(value: i64) -> TrackId {
        TrackId::new(value).expect("valid test track id")
    }
}
