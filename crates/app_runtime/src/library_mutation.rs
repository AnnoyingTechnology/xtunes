// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use sustain_domain::{LibraryManagementMode, MetadataChange, Rating, TrackId};

use crate::{
    ApplicationRuntime, ApplicationRuntimeError, ApplicationRuntimeResult,
    managed_library::{metadata_change_affects_managed_path, save_managed_metadata_update},
    playback::{playback_shuffle_seed, playback_track_id},
};

impl ApplicationRuntime {
    pub(super) fn set_rating(
        &mut self,
        track_id: TrackId,
        rating: Rating,
    ) -> ApplicationRuntimeResult<()> {
        self.ensure_no_background_library_task()?;
        let library_store = self
            .library_store
            .clone()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;
        let metadata_service = self
            .metadata_service
            .clone()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;
        let track_index = self
            .library_tracks
            .iter()
            .position(|track| track.id == track_id && !track.location.is_missing())
            .ok_or(ApplicationRuntimeError::TrackUnavailable)?;
        let path = self
            .absolute_track_path(&self.library_tracks[track_index])
            .ok_or(ApplicationRuntimeError::TrackUnavailable)?;

        metadata_service
            .write_rating(&path, rating)
            .map_err(|_| ApplicationRuntimeError::MetadataWriteFailed)?;

        let mut track = self.library_tracks[track_index].clone();
        track.rating = rating;
        library_store
            .save_track(track.clone())
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        self.library_tracks[track_index] = track;

        Ok(())
    }

    pub(super) fn update_metadata(
        &mut self,
        track_id: TrackId,
        change: MetadataChange,
    ) -> ApplicationRuntimeResult<()> {
        self.ensure_no_background_library_task()?;
        let library_store = self
            .library_store
            .clone()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;
        let metadata_service = self
            .metadata_service
            .clone()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;
        let track_index = self
            .library_tracks
            .iter()
            .position(|track| track.id == track_id && !track.location.is_missing())
            .ok_or(ApplicationRuntimeError::TrackUnavailable)?;
        let path = self
            .absolute_track_path(&self.library_tracks[track_index])
            .ok_or(ApplicationRuntimeError::TrackUnavailable)?;

        metadata_service
            .write_metadata(&path, change.clone())
            .map_err(|_| ApplicationRuntimeError::MetadataWriteFailed)?;

        let mut track = self.library_tracks[track_index].clone();
        track.metadata.apply_change(&change);
        let track = if self.settings.library.management_mode
            == LibraryManagementMode::CopyAddedFilesIntoLibrary
            && metadata_change_affects_managed_path(&change)
        {
            let library_path = self
                .settings
                .library_path()
                .ok_or(ApplicationRuntimeError::LibraryPathUnavailable)?;
            save_managed_metadata_update(
                library_path,
                library_store.as_ref(),
                &self.library_tracks,
                track,
            )?
        } else {
            library_store
                .save_track(track.clone())
                .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
            track
        };
        self.library_tracks[track_index] = track;

        Ok(())
    }

    pub(super) fn set_artwork(
        &mut self,
        track_id: TrackId,
        artwork: Option<Vec<u8>>,
    ) -> ApplicationRuntimeResult<()> {
        self.ensure_no_background_library_task()?;
        let metadata_service = self
            .metadata_service
            .clone()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;
        let track = self
            .library_tracks
            .iter()
            .find(|track| track.id == track_id && !track.location.is_missing())
            .cloned()
            .ok_or(ApplicationRuntimeError::TrackUnavailable)?;
        let path = self
            .absolute_track_path(&track)
            .ok_or(ApplicationRuntimeError::TrackUnavailable)?;
        metadata_service
            .write_artwork(&path, artwork)
            .map_err(|_| ApplicationRuntimeError::MetadataWriteFailed)?;
        Ok(())
    }

    pub(super) fn reset_play_count(&mut self, track_id: TrackId) -> ApplicationRuntimeResult<()> {
        self.ensure_no_background_library_task()?;
        let library_store = self
            .library_store
            .clone()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;
        let track_index = self
            .library_tracks
            .iter()
            .position(|track| track.id == track_id)
            .ok_or(ApplicationRuntimeError::TrackUnavailable)?;

        let mut track = self.library_tracks[track_index].clone();
        track.statistics.play_count = 0;
        track.statistics.skip_count = 0;
        track.statistics.last_played_at = None;
        track.statistics.last_skipped_at = None;
        library_store
            .save_track(track.clone())
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        self.library_tracks[track_index] = track;

        Ok(())
    }

    pub(super) fn remove_track_from_library(
        &mut self,
        track_id: TrackId,
    ) -> ApplicationRuntimeResult<()> {
        self.ensure_no_background_library_task()?;
        self.stop_playback_if_playing(track_id);
        let library_store = self
            .library_store
            .as_ref()
            .ok_or(ApplicationRuntimeError::LibraryServicesUnavailable)?;
        library_store
            .delete_track(track_id)
            .map_err(|_| ApplicationRuntimeError::LibraryStoreFailed)?;
        self.library_tracks.retain(|track| track.id != track_id);
        self.playback_queue
            .remove_track(track_id, playback_shuffle_seed());
        Ok(())
    }

    pub(super) fn move_track_to_trash(
        &mut self,
        track_id: TrackId,
    ) -> ApplicationRuntimeResult<()> {
        self.ensure_no_background_library_task()?;
        let track = self
            .library_tracks
            .iter()
            .find(|track| track.id == track_id)
            .cloned()
            .ok_or(ApplicationRuntimeError::TrackUnavailable)?;

        self.stop_playback_if_playing(track_id);

        if let Some(path) = self.absolute_track_path(&track) {
            if path.exists() {
                trash::delete(&path).map_err(|_| ApplicationRuntimeError::TrackTrashFailed)?;
            }
        }

        self.remove_track_from_library(track_id)
    }

    fn stop_playback_if_playing(&self, track_id: TrackId) {
        let Some(service) = self.playback_service.as_deref() else {
            return;
        };
        if playback_track_id(&service.state()) == Some(track_id) {
            let _ = service.stop();
        }
    }

    fn ensure_no_background_library_task(&self) -> ApplicationRuntimeResult<()> {
        if self.background_task_status.is_running() {
            return Err(ApplicationRuntimeError::BackgroundTaskRunning);
        }

        Ok(())
    }
}
