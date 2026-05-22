// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use crate::{PlaylistId, TrackId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Playlist {
    pub id: PlaylistId,
    pub name: String,
    pub entries: Vec<PlaylistEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaylistEntry {
    pub playlist_id: PlaylistId,
    pub track_id: TrackId,
    pub position: u32,
}
