// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::path::PathBuf;

use crate::TrackId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackPlaybackSource {
    pub track_id: TrackId,
    pub path: PathBuf,
}

impl TrackPlaybackSource {
    pub fn new(track_id: TrackId, path: impl Into<PathBuf>) -> Self {
        Self {
            track_id,
            path: path.into(),
        }
    }
}
