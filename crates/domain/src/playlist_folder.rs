// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use crate::{PlaylistFolderId, PlaylistId, SmartPlaylistId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaylistFolder {
    pub id: PlaylistFolderId,
    pub name: String,
    pub parent_folder_id: Option<PlaylistFolderId>,
    pub position: u32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PlaylistItem {
    Playlist(PlaylistId),
    SmartPlaylist(SmartPlaylistId),
    Folder(PlaylistFolderId),
}
