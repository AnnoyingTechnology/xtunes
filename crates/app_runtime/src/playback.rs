// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::time::{Duration, SystemTime};

use sustain_domain::{
    PlaybackCommand, PlaybackQueue, PlaybackQueueSource, PlaybackSession, PlaybackState, TrackId,
    TrackPlaybackSource,
};
use sustain_playback::PlaybackService;

use crate::{ApplicationRuntime, ApplicationRuntimeError, ApplicationRuntimeResult};

impl ApplicationRuntime {
    pub(super) fn handle_playback_command(
        &mut self,
        command: PlaybackCommand,
    ) -> ApplicationRuntimeResult<()> {
        match command {
            PlaybackCommand::ToggleShuffle => {
                self.playback_queue.toggle_shuffle(playback_shuffle_seed());
                Ok(())
            }
            PlaybackCommand::ToggleRepeat => {
                self.playback_queue.toggle_repeat_mode();
                Ok(())
            }
            PlaybackCommand::PlayTrack(track_id) => {
                let queue = self.library_playback_queue(track_id)?;
                self.play_track(track_id)?;
                self.playback_queue = queue;
                Ok(())
            }
            PlaybackCommand::PlayPreviousTrack => self.play_previous_track(),
            PlaybackCommand::PlayNextTrack => self.play_next_track(),
            PlaybackCommand::SkipCurrentTrack => self.skip_current_track(),
            PlaybackCommand::EnqueueNext(track_ids) => {
                self.playback_queue.enqueue_after_current(&track_ids);
                Ok(())
            }
            PlaybackCommand::Pause => self
                .playback_service()?
                .pause()
                .map_err(|_| ApplicationRuntimeError::PlaybackFailed),
            PlaybackCommand::Resume => self
                .playback_service()?
                .resume()
                .map_err(|_| ApplicationRuntimeError::PlaybackFailed),
            PlaybackCommand::TogglePlayPause => match self.playback_service()?.state() {
                PlaybackState::Playing { .. } => self
                    .playback_service()?
                    .pause()
                    .map_err(|_| ApplicationRuntimeError::PlaybackFailed),
                PlaybackState::Paused { .. } => self
                    .playback_service()?
                    .resume()
                    .map_err(|_| ApplicationRuntimeError::PlaybackFailed),
                PlaybackState::Stopped | PlaybackState::Loading { .. } => Ok(()),
            },
            PlaybackCommand::Stop => {
                self.playback_session = None;
                self.playback_service()?
                    .stop()
                    .map_err(|_| ApplicationRuntimeError::PlaybackFailed)
            }
            PlaybackCommand::Seek(position) => self
                .playback_service()?
                .seek(position)
                .map_err(|_| ApplicationRuntimeError::PlaybackFailed),
            PlaybackCommand::SetVolume(volume) => self
                .playback_service()?
                .set_volume(volume)
                .map_err(|_| ApplicationRuntimeError::PlaybackFailed),
        }
    }

    fn playback_service(&self) -> ApplicationRuntimeResult<&dyn PlaybackService> {
        self.playback_service
            .as_deref()
            .ok_or(ApplicationRuntimeError::PlaybackServiceUnavailable)
    }

    fn library_playback_queue(&self, track_id: TrackId) -> ApplicationRuntimeResult<PlaybackQueue> {
        let _source = self.track_playback_source(track_id)?;
        Ok(PlaybackQueue::new(
            PlaybackQueueSource::Library,
            self.playable_track_ids(),
            track_id,
            self.playback_queue.options(),
            playback_shuffle_seed(),
        ))
    }

    fn play_track(&mut self, track_id: TrackId) -> ApplicationRuntimeResult<()> {
        let source = self.track_playback_source(track_id)?;
        self.playback_service()?
            .play_track(source)
            .map_err(|_| ApplicationRuntimeError::PlaybackFailed)?;
        // Every new playback starts a fresh session: any unfinished
        // listening on the previous track ends here without registering
        // either a play or a skip (unless the caller — see
        // `skip_current_track` — committed one first). Capturing the
        // duration up front means immediate Next clicks still see a
        // session and can decide skip eligibility correctly, instead
        // of racing the 1 Hz tick that would otherwise create it.
        let duration = self
            .library_tracks
            .iter()
            .find(|track| track.id == track_id)
            .and_then(|track| track.metadata.duration)
            .unwrap_or(Duration::ZERO);
        self.playback_session = Some(PlaybackSession::new(track_id, duration));
        Ok(())
    }

    fn track_playback_source(
        &self,
        track_id: TrackId,
    ) -> ApplicationRuntimeResult<TrackPlaybackSource> {
        let track = self
            .library_tracks
            .iter()
            .find(|track| track.id == track_id && !track.location.is_missing())
            .ok_or(ApplicationRuntimeError::TrackUnavailable)?;
        let path = self
            .absolute_track_path(track)
            .ok_or(ApplicationRuntimeError::TrackUnavailable)?;
        Ok(TrackPlaybackSource::new(track_id, path))
    }

    fn play_previous_track(&mut self) -> ApplicationRuntimeResult<()> {
        self.play_adjacent_track(self.playback_queue.previous_track_id())
    }

    fn play_next_track(&mut self) -> ApplicationRuntimeResult<()> {
        self.play_adjacent_track(self.playback_queue.next_track_id())
    }

    fn play_adjacent_track(&mut self, track_id: Option<TrackId>) -> ApplicationRuntimeResult<()> {
        let Some(track_id) = track_id else {
            // End of queue (or no neighbour in the current direction). Stop
            // the backend so its state stops reporting the previous track as
            // still playing — otherwise the auto-advance triggered by EOS
            // would leave the UI showing the last track at a stale position.
            if let Some(service) = self.playback_service.as_deref() {
                let _ = service.stop();
            }
            self.playback_session = None;
            return Ok(());
        };

        self.play_track(track_id)?;
        let _moved = self.playback_queue.move_to_track(track_id);
        Ok(())
    }

    fn skip_current_track(&mut self) -> ApplicationRuntimeResult<()> {
        // Register a skip on the current session only when one exists
        // AND the play threshold has not already been reached. After
        // the threshold there is no skip — the play already counted.
        // This is the only entry point that ever increments skip_count;
        // EOS auto-advance and Previous never do.
        let pending_skip = self
            .playback_session
            .as_ref()
            .and_then(|session| (!session.is_play_registered()).then_some(session.track_id()));
        if let Some(track_id) = pending_skip {
            let now = self.clock.now();
            self.commit_skip_increment(track_id, now)?;
        }
        self.play_next_track()
    }

    /// Drive the play-statistics accounting forward by `elapsed` of
    /// wall-clock time. The UI calls this on a steady cadence
    /// (currently 1 Hz, see [`super::lib`] now-playing refresh timer).
    /// Accumulation only happens while the playback service reports
    /// the [`PlaybackState::Playing`] state, and only against the
    /// track currently associated with the session.
    ///
    /// When the cumulative listened time crosses the play threshold
    /// (see [`PlaybackSession::play_threshold`]), the play count is
    /// incremented exactly once, `last_played_at` is updated, and the
    /// new statistics are flushed to SQLite. No file-tag write is
    /// emitted — listening statistics live exclusively in the
    /// library, per the persistence policy in AGENTS.md.
    pub fn on_playback_tick(&mut self, elapsed: Duration) -> ApplicationRuntimeResult<()> {
        let state = self.playback_state();
        let playing_track_id = match state {
            PlaybackState::Playing { track_id, .. } => track_id,
            _ => return Ok(()),
        };

        self.ensure_session_for_track(playing_track_id);

        let crossed_threshold = match self.playback_session.as_mut() {
            Some(session) if !session.is_play_registered() => {
                session.accumulate_listening(elapsed);
                if session.should_register_play() {
                    session.register_play();
                    true
                } else {
                    false
                }
            }
            _ => false,
        };

        if crossed_threshold {
            let now = self.clock.now();
            self.commit_play_increment(playing_track_id, now)?;
        }
        Ok(())
    }

    fn ensure_session_for_track(&mut self, track_id: TrackId) {
        if let Some(session) = self.playback_session.as_ref()
            && session.track_id() == track_id
        {
            return;
        }
        let duration = self
            .library_tracks
            .iter()
            .find(|track| track.id == track_id)
            .and_then(|track| track.metadata.duration)
            .unwrap_or(Duration::ZERO);
        self.playback_session = Some(PlaybackSession::new(track_id, duration));
    }

    fn commit_play_increment(
        &mut self,
        track_id: TrackId,
        at: SystemTime,
    ) -> ApplicationRuntimeResult<()> {
        self.mutate_track_statistics(track_id, |statistics| {
            statistics.play_count = statistics.play_count.saturating_add(1);
            statistics.last_played_at = Some(at);
        })
    }

    fn commit_skip_increment(
        &mut self,
        track_id: TrackId,
        at: SystemTime,
    ) -> ApplicationRuntimeResult<()> {
        self.mutate_track_statistics(track_id, |statistics| {
            statistics.skip_count = statistics.skip_count.saturating_add(1);
            statistics.last_skipped_at = Some(at);
        })
    }

    // Applies the given mutation to a track's in-memory statistics and
    // persists the updated track row. When no library store is
    // installed — for instance in headless tests — only the in-memory
    // copy is updated; the SQLite write is a no-op so the same code
    // path stays callable.
    fn mutate_track_statistics<F>(
        &mut self,
        track_id: TrackId,
        mutate: F,
    ) -> ApplicationRuntimeResult<()>
    where
        F: FnOnce(&mut sustain_domain::PlayStatistics),
    {
        let Some(track_index) = self
            .library_tracks
            .iter()
            .position(|track| track.id == track_id)
        else {
            return Ok(());
        };
        let mut updated = self.library_tracks[track_index].clone();
        mutate(&mut updated.statistics);
        if let Some(store) = self.library_store.as_ref() {
            store
                .save_track(updated.clone())
                .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        }
        self.library_tracks[track_index] = updated;
        Ok(())
    }

    fn playable_track_ids(&self) -> Vec<TrackId> {
        self.library_tracks
            .iter()
            .filter(|track| !track.location.is_missing())
            .map(|track| track.id)
            .collect()
    }

    pub(super) fn refresh_playback_queue_track_ids(&mut self) {
        let track_ids = self.playable_track_ids();
        self.playback_queue
            .replace_ordered_track_ids(track_ids, playback_shuffle_seed());
    }
}

pub(super) fn playback_track_id(state: &PlaybackState) -> Option<TrackId> {
    match state {
        PlaybackState::Loading { track_id }
        | PlaybackState::Playing { track_id, .. }
        | PlaybackState::Paused { track_id, .. } => Some(*track_id),
        PlaybackState::Stopped => None,
    }
}

pub(super) fn playback_shuffle_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0)
}
