// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::time::Duration;

use crate::TrackId;

/// Hard ceiling on the play threshold for long-form tracks (podcasts,
/// DJ mixes, audiobook chapters). Once the listener has spent this long
/// on a track, the play counts even if the track is longer than 20
/// minutes (in which case the duration/2 rule would otherwise require
/// >10 min of listening).
const PLAY_THRESHOLD_CEILING: Duration = Duration::from_secs(10 * 60);

/// Tracks how much of the currently playing track the listener has
/// actually heard, and whether a play has been registered for this
/// listening session.
///
/// A "play" is registered exactly once per session, the first time the
/// cumulative listened time crosses the threshold returned by
/// [`PlaybackSession::play_threshold`]. After that, further listening
/// in the same session does NOT increment the play count again. A new
/// session begins each time a different track starts playing.
///
/// Cumulative listened time is intentionally distinct from raw playback
/// position: seeking forward does NOT count as listening, pausing and
/// resuming preserves accumulated time, and replaying a section adds to
/// the cumulative total.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackSession {
    track_id: TrackId,
    duration: Duration,
    listened: Duration,
    play_registered: bool,
}

impl PlaybackSession {
    /// Begin a new session for the given track. A fresh session has
    /// zero listened time and has not yet registered a play.
    pub const fn new(track_id: TrackId, duration: Duration) -> Self {
        Self {
            track_id,
            duration,
            listened: Duration::ZERO,
            play_registered: false,
        }
    }

    /// Threshold of cumulative listened time at which a play registers.
    /// Returns `min(duration / 2, 10 minutes)`: short tracks must be
    /// heard halfway through, long tracks need only 10 minutes of
    /// actual listening.
    pub fn play_threshold(duration: Duration) -> Duration {
        let half = duration / 2;
        half.min(PLAY_THRESHOLD_CEILING)
    }

    pub fn track_id(&self) -> TrackId {
        self.track_id
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }

    pub fn listened(&self) -> Duration {
        self.listened
    }

    pub fn is_play_registered(&self) -> bool {
        self.play_registered
    }

    /// Add elapsed wall-clock listening to the session. Callers must
    /// only call this when the track is actually playing (not paused,
    /// not loading, not stopped). The session itself does not know the
    /// transport state and trusts the caller to gate accumulation.
    pub fn accumulate_listening(&mut self, elapsed: Duration) {
        self.listened = self.listened.saturating_add(elapsed);
    }

    /// Returns true when the cumulative listened time has reached the
    /// play threshold and a play has not yet been registered for this
    /// session. Repeated calls return `false` after the play has been
    /// registered via [`PlaybackSession::register_play`].
    pub fn should_register_play(&self) -> bool {
        if self.play_registered {
            return false;
        }
        if self.duration.is_zero() {
            return false;
        }
        self.listened >= Self::play_threshold(self.duration)
    }

    /// Mark the play as registered. Idempotent — calling twice does
    /// not produce two plays.
    pub fn register_play(&mut self) {
        self.play_registered = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> TrackId {
        TrackId::new(1).expect("valid test track id")
    }

    #[test]
    fn play_threshold_is_half_duration_for_short_tracks() {
        let three_minutes = Duration::from_secs(180);
        assert_eq!(
            PlaybackSession::play_threshold(three_minutes),
            Duration::from_secs(90)
        );
    }

    #[test]
    fn play_threshold_caps_at_ten_minutes_for_long_tracks() {
        let one_hour = Duration::from_secs(3600);
        assert_eq!(
            PlaybackSession::play_threshold(one_hour),
            Duration::from_secs(600)
        );
    }

    #[test]
    fn play_threshold_for_twenty_minute_track_is_ten_minutes() {
        let twenty_minutes = Duration::from_secs(20 * 60);
        assert_eq!(
            PlaybackSession::play_threshold(twenty_minutes),
            Duration::from_secs(600)
        );
    }

    #[test]
    fn fresh_session_does_not_register_play() {
        let session = PlaybackSession::new(track(), Duration::from_secs(180));
        assert!(!session.should_register_play());
        assert!(!session.is_play_registered());
    }

    #[test]
    fn play_registers_once_threshold_crossed() {
        let mut session = PlaybackSession::new(track(), Duration::from_secs(180));
        session.accumulate_listening(Duration::from_secs(89));
        assert!(!session.should_register_play());
        session.accumulate_listening(Duration::from_secs(1));
        assert!(session.should_register_play());
    }

    #[test]
    fn play_does_not_re_register_within_same_session() {
        let mut session = PlaybackSession::new(track(), Duration::from_secs(180));
        session.accumulate_listening(Duration::from_secs(120));
        assert!(session.should_register_play());
        session.register_play();
        assert!(!session.should_register_play());
        session.accumulate_listening(Duration::from_secs(60));
        assert!(!session.should_register_play());
    }

    #[test]
    fn long_track_registers_at_ten_minute_ceiling() {
        let mut session = PlaybackSession::new(track(), Duration::from_secs(3600));
        session.accumulate_listening(Duration::from_secs(599));
        assert!(!session.should_register_play());
        session.accumulate_listening(Duration::from_secs(1));
        assert!(session.should_register_play());
    }

    #[test]
    fn zero_duration_track_never_registers_play() {
        let mut session = PlaybackSession::new(track(), Duration::ZERO);
        session.accumulate_listening(Duration::from_secs(60));
        assert!(!session.should_register_play());
    }
}
