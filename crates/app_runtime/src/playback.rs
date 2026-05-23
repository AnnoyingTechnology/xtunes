// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use sustain_domain::{
    PlaybackCommand, PlaybackQueue, PlaybackQueueSource, PlaybackState, TrackId,
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
            PlaybackCommand::Stop => self
                .playback_service()?
                .stop()
                .map_err(|_| ApplicationRuntimeError::PlaybackFailed),
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

    fn play_track(&self, track_id: TrackId) -> ApplicationRuntimeResult<()> {
        let source = self.track_playback_source(track_id)?;
        self.playback_service()?
            .play_track(source)
            .map_err(|_| ApplicationRuntimeError::PlaybackFailed)
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
            return Ok(());
        };

        self.play_track(track_id)?;
        let _moved = self.playback_queue.move_to_track(track_id);
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
