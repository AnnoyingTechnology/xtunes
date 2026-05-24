// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use crate::{PlaylistId, SmartPlaylistId};

/// Persisted layout for the track table: which columns are shown, in what
/// order, and at what pixel width. The domain treats `column_id` as opaque
/// so the UI can evolve its own column set without dragging the domain along.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TrackColumnLayout {
    /// Ordered left-to-right. The vector order *is* the column order; no
    /// separate position field is exposed at the domain level.
    pub entries: Vec<TrackColumnEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackColumnEntry {
    pub column_id: String,
    pub visible: bool,
    pub width_px: u32,
}

/// Scope for a stored layout. `Default` is the user-wide fallback applied to
/// the main library view and to any playlist that has not overridden it;
/// `Playlist` and `SmartPlaylist` overrides take precedence when set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackColumnLayoutScope {
    Default,
    Playlist(PlaylistId),
    SmartPlaylist(SmartPlaylistId),
}

impl TrackColumnLayout {
    pub fn new(entries: Vec<TrackColumnEntry>) -> Self {
        Self { entries }
    }
}
