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
    pub availability: TrackAvailability,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TrackAvailability {
    #[default]
    Available,
    Missing,
}

impl TrackLocation {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            availability: TrackAvailability::Available,
        }
    }

    pub fn missing(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            availability: TrackAvailability::Missing,
        }
    }

    pub fn is_missing(&self) -> bool {
        self.availability == TrackAvailability::Missing
    }
}
