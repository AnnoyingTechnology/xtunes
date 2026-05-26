// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use sustain_domain::{ApplicationCommand, LibraryManagementMode};

use crate::{ApplicationRuntime, ApplicationRuntimeError, ApplicationRuntimeResult};

impl ApplicationRuntime {
    pub fn handle_command(&mut self, command: ApplicationCommand) -> ApplicationRuntimeResult<()> {
        match command {
            ApplicationCommand::Playback(command) => {
                self.handle_playback_command(command)?;
            }
            ApplicationCommand::UpdateSettings(settings) => {
                if self.background_task_status.is_running()
                    && settings.library != self.settings.library
                {
                    let cancellation_allowed = self
                        .background_task_status
                        .is_library_consolidation_running()
                        && self.settings.library.path == settings.library.path
                        && self.settings.library.management_mode
                            == LibraryManagementMode::CopyAddedFilesIntoLibrary
                        && settings.library.management_mode
                            == LibraryManagementMode::ReferenceFilesInPlace;

                    if cancellation_allowed {
                        self.request_library_consolidation_cancellation();
                    } else {
                        return Err(ApplicationRuntimeError::BackgroundTaskRunning);
                    }
                }
                if let Some(settings_store) = &self.settings_store {
                    settings_store
                        .save_settings(settings.clone())
                        .map_err(|_| ApplicationRuntimeError::SettingsSaveFailed)?;
                }
                self.settings = settings;
                // Per the documented lazy-availability contract on
                // load_library_tracks, settings changes do not stat
                // tracks. Reconciliation is the scan's job (or lazy
                // detection on touch).
            }
            ApplicationCommand::ScanLibrary { library_path } => {
                self.scan_library(library_path)?;
            }
            ApplicationCommand::RemoveTrackFromLibrary { track_id } => {
                self.remove_track_from_library(track_id)?;
            }
            ApplicationCommand::MoveTrackToTrash { track_id } => {
                self.move_track_to_trash(track_id)?;
            }
            ApplicationCommand::SetRating { track_id, rating } => {
                self.set_rating(track_id, rating)?;
            }
            ApplicationCommand::CreatePlaylist {
                name,
                parent_folder_id,
            } => {
                self.create_playlist(name, parent_folder_id)?;
            }
            ApplicationCommand::RenamePlaylist { playlist_id, name } => {
                self.rename_playlist(playlist_id, name)?;
            }
            ApplicationCommand::DeletePlaylist { playlist_id } => {
                self.delete_playlist(playlist_id)?;
            }
            ApplicationCommand::AddTracksToPlaylist {
                playlist_id,
                track_ids,
            } => {
                self.add_tracks_to_playlist(playlist_id, track_ids)?;
            }
            ApplicationCommand::RemoveTracksFromPlaylist {
                playlist_id,
                track_ids,
            } => {
                self.remove_tracks_from_playlist(playlist_id, track_ids)?;
            }
            ApplicationCommand::MovePlaylistEntries {
                playlist_id,
                track_ids,
                new_position,
            } => {
                self.move_playlist_entries(playlist_id, track_ids, new_position)?;
            }
            ApplicationCommand::CreatePlaylistFolder {
                name,
                parent_folder_id,
            } => {
                self.create_playlist_folder(name, parent_folder_id)?;
            }
            ApplicationCommand::RenamePlaylistFolder { folder_id, name } => {
                self.rename_playlist_folder(folder_id, name)?;
            }
            ApplicationCommand::DeletePlaylistFolder { folder_id } => {
                self.delete_playlist_folder(folder_id)?;
            }
            ApplicationCommand::CreateSmartPlaylist {
                name,
                parent_folder_id,
                rules,
            } => {
                self.create_smart_playlist(name, parent_folder_id, rules)?;
            }
            ApplicationCommand::UpdateSmartPlaylist {
                smart_playlist_id,
                name,
                rules,
            } => {
                self.update_smart_playlist(smart_playlist_id, name, rules)?;
            }
            ApplicationCommand::DeleteSmartPlaylist { smart_playlist_id } => {
                self.delete_smart_playlist(smart_playlist_id)?;
            }
            ApplicationCommand::MovePlaylistItem {
                item,
                target_parent_folder_id,
                position,
            } => {
                self.move_playlist_item(item, target_parent_folder_id, position)?;
            }
            ApplicationCommand::UpdateMetadata { track_id, change } => {
                self.update_metadata(track_id, *change)?;
            }
            ApplicationCommand::ResetPlayCount { track_id } => {
                self.reset_play_count(track_id)?;
            }
            ApplicationCommand::SetArtwork { track_id, artwork } => {
                self.set_artwork(track_id, artwork)?;
            }
            ApplicationCommand::FetchArtwork { track_id } => {
                self.fetch_artwork(track_id)?;
            }
            ApplicationCommand::AddExternalLibraryItems { paths } => {
                self.add_external_library_items(paths)?;
            }
        }

        Ok(())
    }
}
