// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use crate::{PlaylistId, TrackId};

use super::{PlaybackOptions, RepeatMode, shuffle::shuffled_track_ids};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum PlaybackQueueSource {
    #[default]
    Library,
    Album,
    Playlist(PlaylistId),
    SearchResults,
    Selection,
}

/// Describes the queue the runtime should build when starting playback at a
/// specific track. The activation source (UI view, MPRIS, ...) decides:
/// the runtime never reaches for "all library tracks" by default; it does
/// only what the request asks for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlaybackQueueRequest {
    /// Build the queue from every playable library track. Used by Songs view
    /// and other surfaces that don't pin the queue to a narrower context.
    Library,
    /// Build the queue from this explicit ordered list, labelled with the
    /// given source for downstream UI / MPRIS reporting. Track ids that
    /// don't resolve to a playable library track are silently dropped so
    /// the queue never tries to play missing entries.
    Explicit {
        source: PlaybackQueueSource,
        ordered_track_ids: Vec<TrackId>,
    },
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

    pub fn set_shuffle_enabled(&mut self, enabled: bool, shuffle_seed: u64) {
        if self.options.shuffle_enabled == enabled {
            return;
        }
        self.options = self.options.with_shuffle_enabled(enabled);
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

    /// Inserts the given tracks so they play immediately after the currently
    /// playing track, in both the ordered queue and the play order. Tracks
    /// already present elsewhere in the queue are moved to the new position;
    /// the currently playing track is left in place and skipped if present
    /// in `track_ids`. Returns `false` when there is no current track to
    /// anchor against or when the candidate list reduces to nothing.
    pub fn enqueue_after_current(&mut self, track_ids: &[TrackId]) -> bool {
        let Some(current_track_id) = self.current_track_id else {
            return false;
        };

        let mut to_insert: Vec<TrackId> = Vec::with_capacity(track_ids.len());
        for candidate in track_ids {
            if *candidate != current_track_id && !to_insert.contains(candidate) {
                to_insert.push(*candidate);
            }
        }
        if to_insert.is_empty() {
            return false;
        }

        self.ordered_track_ids.retain(|id| !to_insert.contains(id));
        self.play_order_track_ids
            .retain(|id| !to_insert.contains(id));

        if let Some(index) = self
            .ordered_track_ids
            .iter()
            .position(|id| *id == current_track_id)
        {
            for (offset, track_id) in to_insert.iter().enumerate() {
                self.ordered_track_ids.insert(index + 1 + offset, *track_id);
            }
        }
        if let Some(index) = self
            .play_order_track_ids
            .iter()
            .position(|id| *id == current_track_id)
        {
            for (offset, track_id) in to_insert.iter().enumerate() {
                self.play_order_track_ids
                    .insert(index + 1 + offset, *track_id);
            }
        }

        true
    }

    /// Appends the given tracks at the tail of the play queue, behind every
    /// already-queued track in both the ordered queue and the play order.
    /// Tracks already present elsewhere in the queue are moved to the new
    /// position; the currently playing track is left in place and skipped if
    /// present in `track_ids`. Returns `false` when there is no current track
    /// to anchor against or when the candidate list reduces to nothing.
    pub fn enqueue_at_end(&mut self, track_ids: &[TrackId]) -> bool {
        let Some(current_track_id) = self.current_track_id else {
            return false;
        };

        let mut to_append: Vec<TrackId> = Vec::with_capacity(track_ids.len());
        for candidate in track_ids {
            if *candidate != current_track_id && !to_append.contains(candidate) {
                to_append.push(*candidate);
            }
        }
        if to_append.is_empty() {
            return false;
        }

        self.ordered_track_ids.retain(|id| !to_append.contains(id));
        self.play_order_track_ids
            .retain(|id| !to_append.contains(id));

        self.ordered_track_ids.extend(to_append.iter().copied());
        self.play_order_track_ids.extend(to_append.iter().copied());

        true
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
        && let Some(current_index) = track_ids
            .iter()
            .position(|track_id| *track_id == current_track_id)
    {
        track_ids.rotate_left(current_index);
    }

    track_ids
}

#[cfg(test)]
mod tests {
    use crate::TrackId;

    use super::{PlaybackOptions, PlaybackQueue, PlaybackQueueSource, RepeatMode};

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

    #[test]
    fn enqueue_after_current_inserts_at_current_plus_one() {
        let mut queue = queue_with_options(track_id(2), PlaybackOptions::default());

        assert!(queue.enqueue_after_current(&[track_id(9)]));

        assert_eq!(
            queue.ordered_track_ids(),
            &[track_id(1), track_id(2), track_id(9), track_id(3)]
        );
        assert_eq!(queue.next_track_id(), Some(track_id(9)));
    }

    #[test]
    fn enqueue_after_current_moves_track_already_later_in_queue() {
        let mut queue = queue_with_options(track_id(1), PlaybackOptions::default());

        assert!(queue.enqueue_after_current(&[track_id(3)]));

        assert_eq!(
            queue.ordered_track_ids(),
            &[track_id(1), track_id(3), track_id(2)]
        );
        assert_eq!(queue.next_track_id(), Some(track_id(3)));
    }

    #[test]
    fn enqueue_after_current_inserts_multiple_in_order() {
        let mut queue = queue_with_options(track_id(1), PlaybackOptions::default());

        assert!(queue.enqueue_after_current(&[track_id(9), track_id(8)]));

        assert_eq!(
            queue.ordered_track_ids(),
            &[
                track_id(1),
                track_id(9),
                track_id(8),
                track_id(2),
                track_id(3),
            ]
        );
    }

    #[test]
    fn enqueue_after_current_dedupes_repeated_candidates() {
        let mut queue = queue_with_options(track_id(1), PlaybackOptions::default());

        assert!(queue.enqueue_after_current(&[track_id(9), track_id(9), track_id(8)]));

        assert_eq!(
            queue.ordered_track_ids(),
            &[
                track_id(1),
                track_id(9),
                track_id(8),
                track_id(2),
                track_id(3)
            ]
        );
    }

    #[test]
    fn enqueue_after_current_skips_currently_playing_track() {
        let mut queue = queue_with_options(track_id(2), PlaybackOptions::default());

        assert!(!queue.enqueue_after_current(&[track_id(2)]));
        assert_eq!(
            queue.ordered_track_ids(),
            &[track_id(1), track_id(2), track_id(3)]
        );
    }

    #[test]
    fn enqueue_after_current_returns_false_with_no_current_track() {
        let mut queue = PlaybackQueue::empty(PlaybackOptions::default());

        assert!(!queue.enqueue_after_current(&[track_id(1)]));
        assert!(queue.ordered_track_ids().is_empty());
    }

    #[test]
    fn enqueue_at_end_appends_after_every_queued_track() {
        let mut queue = queue_with_options(track_id(2), PlaybackOptions::default());

        assert!(queue.enqueue_at_end(&[track_id(9)]));

        assert_eq!(
            queue.ordered_track_ids(),
            &[track_id(1), track_id(2), track_id(3), track_id(9)]
        );
        assert_eq!(queue.next_track_id(), Some(track_id(3)));
    }

    #[test]
    fn enqueue_at_end_moves_track_already_in_queue_to_the_tail() {
        let mut queue = queue_with_options(track_id(1), PlaybackOptions::default());

        assert!(queue.enqueue_at_end(&[track_id(2)]));

        assert_eq!(
            queue.ordered_track_ids(),
            &[track_id(1), track_id(3), track_id(2)]
        );
        assert_eq!(queue.next_track_id(), Some(track_id(3)));
    }

    #[test]
    fn enqueue_at_end_appends_multiple_in_order() {
        let mut queue = queue_with_options(track_id(1), PlaybackOptions::default());

        assert!(queue.enqueue_at_end(&[track_id(9), track_id(8)]));

        assert_eq!(
            queue.ordered_track_ids(),
            &[
                track_id(1),
                track_id(2),
                track_id(3),
                track_id(9),
                track_id(8),
            ]
        );
    }

    #[test]
    fn enqueue_at_end_dedupes_repeated_candidates() {
        let mut queue = queue_with_options(track_id(1), PlaybackOptions::default());

        assert!(queue.enqueue_at_end(&[track_id(9), track_id(9), track_id(8)]));

        assert_eq!(
            queue.ordered_track_ids(),
            &[
                track_id(1),
                track_id(2),
                track_id(3),
                track_id(9),
                track_id(8),
            ]
        );
    }

    #[test]
    fn enqueue_at_end_skips_currently_playing_track() {
        let mut queue = queue_with_options(track_id(2), PlaybackOptions::default());

        assert!(!queue.enqueue_at_end(&[track_id(2)]));
        assert_eq!(
            queue.ordered_track_ids(),
            &[track_id(1), track_id(2), track_id(3)]
        );
    }

    #[test]
    fn enqueue_at_end_returns_false_with_no_current_track() {
        let mut queue = PlaybackQueue::empty(PlaybackOptions::default());

        assert!(!queue.enqueue_at_end(&[track_id(1)]));
        assert!(queue.ordered_track_ids().is_empty());
    }

    #[test]
    fn enqueue_at_end_appends_in_shuffle_play_order_too() {
        let mut queue = queue_with_options(
            track_id(2),
            PlaybackOptions {
                shuffle_enabled: true,
                repeat_mode: RepeatMode::Off,
            },
        );

        assert!(queue.enqueue_at_end(&[track_id(9)]));

        let play_order = queue.play_order_track_ids();
        assert_eq!(play_order.first().copied(), Some(track_id(2)));
        assert_eq!(play_order.last().copied(), Some(track_id(9)));
    }

    #[test]
    fn enqueue_after_current_inserts_in_shuffle_play_order_too() {
        let mut queue = queue_with_options(
            track_id(2),
            PlaybackOptions {
                shuffle_enabled: true,
                repeat_mode: RepeatMode::Off,
            },
        );

        assert!(queue.enqueue_after_current(&[track_id(9)]));

        let play_order = queue.play_order_track_ids();
        assert_eq!(play_order.first().copied(), Some(track_id(2)));
        assert_eq!(play_order.get(1).copied(), Some(track_id(9)));
        assert_eq!(queue.next_track_id(), Some(track_id(9)));
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
