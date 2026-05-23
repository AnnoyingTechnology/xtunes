// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TrackId(i64);

impl TrackId {
    pub const fn new(value: i64) -> Option<Self> {
        if value > 0 { Some(Self(value)) } else { None }
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PlaylistId(i64);

impl PlaylistId {
    pub const fn new(value: i64) -> Option<Self> {
        if value > 0 { Some(Self(value)) } else { None }
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SmartPlaylistId(i64);

impl SmartPlaylistId {
    pub const fn new(value: i64) -> Option<Self> {
        if value > 0 { Some(Self(value)) } else { None }
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{PlaylistId, SmartPlaylistId, TrackId};

    #[test]
    fn track_ids_must_be_positive() {
        assert_eq!(TrackId::new(-1), None);
        assert_eq!(TrackId::new(0), None);
        assert_eq!(TrackId::new(1).map(TrackId::get), Some(1));
    }

    #[test]
    fn playlist_ids_must_be_positive() {
        assert_eq!(PlaylistId::new(-1), None);
        assert_eq!(PlaylistId::new(0), None);
        assert_eq!(PlaylistId::new(1).map(PlaylistId::get), Some(1));
    }

    #[test]
    fn smart_playlist_ids_must_be_positive() {
        assert_eq!(SmartPlaylistId::new(-1), None);
        assert_eq!(SmartPlaylistId::new(0), None);
        assert_eq!(SmartPlaylistId::new(1).map(SmartPlaylistId::get), Some(1));
    }
}
