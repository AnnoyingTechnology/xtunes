use std::path::PathBuf;

use crate::{PlayStatistics, Rating, TrackId, TrackMetadata};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Track {
    pub id: TrackId,
    pub location: TrackLocation,
    pub metadata: TrackMetadata,
    pub rating: Rating,
    pub statistics: PlayStatistics,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackLocation {
    pub path: PathBuf,
}

impl TrackLocation {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}
