// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use crate::TrackId;

use super::{
    PlaybackOptions, PlaybackQueue, PlaybackQueueSource, RepeatMode, ShuffleMode,
    effective_shuffle_mode,
};

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
        shuffle_mode: ShuffleMode::Off,
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
            shuffle_mode: ShuffleMode::Off,
            repeat_mode: RepeatMode::One,
        },
    );

    assert_eq!(queue.previous_track_id(), Some(track_id(2)));
    assert_eq!(queue.next_track_id(), Some(track_id(2)));
}

#[test]
fn playback_queue_uses_pure_shuffle_order_with_current_track_first() {
    let queue = queue_with_options(
        track_id(3),
        PlaybackOptions {
            shuffle_mode: ShuffleMode::Pure,
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
fn enqueue_at_end_appends_in_pure_shuffle_play_order_too() {
    let mut queue = queue_with_options(
        track_id(2),
        PlaybackOptions {
            shuffle_mode: ShuffleMode::Pure,
            repeat_mode: RepeatMode::Off,
        },
    );

    assert!(queue.enqueue_at_end(&[track_id(9)]));

    let play_order = queue.play_order_track_ids();
    assert_eq!(play_order.first().copied(), Some(track_id(2)));
    assert_eq!(play_order.last().copied(), Some(track_id(9)));
}

#[test]
fn enqueue_after_current_inserts_in_pure_shuffle_play_order_too() {
    let mut queue = queue_with_options(
        track_id(2),
        PlaybackOptions {
            shuffle_mode: ShuffleMode::Pure,
            repeat_mode: RepeatMode::Off,
        },
    );

    assert!(queue.enqueue_after_current(&[track_id(9)]));

    let play_order = queue.play_order_track_ids();
    assert_eq!(play_order.first().copied(), Some(track_id(2)));
    assert_eq!(play_order.get(1).copied(), Some(track_id(9)));
    assert_eq!(queue.next_track_id(), Some(track_id(9)));
}

#[test]
fn lazy_queue_starts_with_current_track_in_history() {
    let queue = queue_with_options(
        track_id(2),
        PlaybackOptions {
            shuffle_mode: ShuffleMode::Smart,
            repeat_mode: RepeatMode::Off,
        },
    );

    assert_eq!(queue.current_track_id(), Some(track_id(2)));
    assert_eq!(queue.play_order_track_ids(), &[track_id(2)]);
    assert_eq!(queue.next_track_id(), None);
    assert!(queue.needs_lazy_pick());
}

#[test]
fn lazy_queue_appends_pick_and_walks_history_back_and_forth() {
    let mut queue = queue_with_options(
        track_id(1),
        PlaybackOptions {
            shuffle_mode: ShuffleMode::Smart,
            repeat_mode: RepeatMode::Off,
        },
    );
    assert!(queue.lazy_append_pick(track_id(3)));
    assert_eq!(queue.next_track_id(), Some(track_id(3)));

    // Advance: cursor moves onto the appended pick.
    assert!(queue.move_to_track(track_id(3)));
    assert_eq!(queue.current_track_id(), Some(track_id(3)));
    assert_eq!(queue.previous_track_id(), Some(track_id(1)));
    assert_eq!(queue.next_track_id(), None);
    assert!(queue.needs_lazy_pick());

    // Step back: cursor moves onto seed, next now revisits the
    // already-picked track 3 (no fresh pick).
    assert!(queue.move_to_track(track_id(1)));
    assert!(!queue.needs_lazy_pick());
    assert_eq!(queue.next_track_id(), Some(track_id(3)));
}

#[test]
fn lazy_queue_move_to_track_outside_history_resets_branch() {
    // move_to_track is the auto-advance hook; in normal flow the
    // target is always already in history (the picker put it
    // there). The defensive branch — target not in history —
    // truncates speculative picks past the current cursor and
    // pushes the new track as the head of a fresh sub-sequence.
    let mut queue = queue_with_options(
        track_id(1),
        PlaybackOptions {
            shuffle_mode: ShuffleMode::Smart,
            repeat_mode: RepeatMode::Off,
        },
    );
    assert!(queue.lazy_append_pick(track_id(3)));
    assert!(queue.move_to_track(track_id(3)));
    // History is now [1, 3] with cursor at 1. Step back to track 1.
    assert!(queue.move_to_track(track_id(1)));
    assert_eq!(queue.current_track_id(), Some(track_id(1)));
    // From cursor=0 jump to a track that is not in history. That
    // truncates the speculative tail and pushes the new track.
    assert!(queue.move_to_track(track_id(2)));
    assert_eq!(queue.play_order_track_ids(), &[track_id(1), track_id(2)]);
    assert_eq!(queue.current_track_id(), Some(track_id(2)));
    assert_eq!(queue.next_track_id(), None);
}

#[test]
fn lazy_queue_move_to_track_within_history_walks_cursor() {
    // The everyday Previous → Next pattern. After picking a few
    // tracks the user steps back to a mid-history entry; the
    // cursor walks rather than truncating, so a subsequent Next
    // replays the already-chosen successor.
    let mut queue = queue_with_options(
        track_id(1),
        PlaybackOptions {
            shuffle_mode: ShuffleMode::Smart,
            repeat_mode: RepeatMode::Off,
        },
    );
    assert!(queue.lazy_append_pick(track_id(3)));
    assert!(queue.move_to_track(track_id(3)));
    assert!(queue.lazy_append_pick(track_id(2)));
    assert!(queue.move_to_track(track_id(2)));
    // history = [1, 3, 2], cursor = 2. Step back to track 3.
    assert!(queue.move_to_track(track_id(3)));
    assert_eq!(queue.current_track_id(), Some(track_id(3)));
    // Next now replays the already-picked track 2 instead of
    // consulting the picker.
    assert!(!queue.needs_lazy_pick());
    assert_eq!(queue.next_track_id(), Some(track_id(2)));
    assert_eq!(
        queue.play_order_track_ids(),
        &[track_id(1), track_id(3), track_id(2)]
    );
}

#[test]
fn smart_shuffle_downgrades_to_pure_for_ad_hoc_sources() {
    // Album / SearchResults / Selection are explicit listening
    // contexts; Smart's discovery-oriented signals would be
    // inappropriate there. The user's setting is preserved (the
    // `shuffle_mode` option stays `Smart`), but the layout is
    // built as if Pure was requested.
    for source in [
        PlaybackQueueSource::Album,
        PlaybackQueueSource::SearchResults,
        PlaybackQueueSource::Selection,
    ] {
        assert_eq!(
            effective_shuffle_mode(ShuffleMode::Smart, &source),
            ShuffleMode::Pure
        );
    }
    // Library and Playlist contexts still honour Smart.
    assert_eq!(
        effective_shuffle_mode(ShuffleMode::Smart, &PlaybackQueueSource::Library),
        ShuffleMode::Smart
    );
}

#[test]
fn lazy_enqueue_after_current_inserts_between_cursor_and_picked_next() {
    let mut queue = queue_with_options(
        track_id(1),
        PlaybackOptions {
            shuffle_mode: ShuffleMode::Smart,
            repeat_mode: RepeatMode::Off,
        },
    );
    assert!(queue.lazy_append_pick(track_id(3)));

    // Enqueue Next track 2: should sit before the picked successor.
    assert!(queue.enqueue_after_current(&[track_id(2)]));
    assert_eq!(
        queue.play_order_track_ids(),
        &[track_id(1), track_id(2), track_id(3)]
    );
    assert_eq!(queue.next_track_id(), Some(track_id(2)));
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
