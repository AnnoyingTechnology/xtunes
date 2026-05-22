// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::path::{Component, Path, PathBuf};

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
    pub relative_path: TrackRelativePath,
    pub availability: TrackAvailability,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TrackRelativePath(PathBuf);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TrackAvailability {
    #[default]
    Available,
    Missing,
}

impl TrackLocation {
    pub const fn available(relative_path: TrackRelativePath) -> Self {
        Self {
            relative_path,
            availability: TrackAvailability::Available,
        }
    }

    pub const fn missing(relative_path: TrackRelativePath) -> Self {
        Self {
            relative_path,
            availability: TrackAvailability::Missing,
        }
    }

    pub fn is_missing(&self) -> bool {
        self.availability == TrackAvailability::Missing
    }

    pub fn absolute_path(&self, library_root: &Path) -> PathBuf {
        self.relative_path.resolve(library_root)
    }
}

impl TrackRelativePath {
    pub fn new(path: impl Into<PathBuf>) -> Option<Self> {
        let path = normalize_relative_path(path.into())?;
        Some(Self(path))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn resolve(&self, library_root: &Path) -> PathBuf {
        library_root.join(&self.0)
    }

    pub fn to_path_buf(&self) -> PathBuf {
        self.0.clone()
    }
}

fn normalize_relative_path(path: PathBuf) -> Option<PathBuf> {
    if path.is_absolute() {
        return None;
    }

    let mut normalized = PathBuf::new();
    let mut has_file_component = false;
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                normalized.push(value);
                has_file_component = true;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    has_file_component.then_some(normalized)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::TrackRelativePath;

    #[test]
    fn track_relative_paths_reject_absolute_paths() {
        assert_eq!(TrackRelativePath::new("/music/track.flac"), None);
    }

    #[test]
    fn track_relative_paths_reject_parent_components() {
        assert_eq!(TrackRelativePath::new("../track.flac"), None);
        assert_eq!(TrackRelativePath::new("artist/../track.flac"), None);
    }

    #[test]
    fn track_relative_paths_normalize_current_directory_components() {
        assert_eq!(
            TrackRelativePath::new("./artist/album/track.flac").map(|path| path.to_path_buf()),
            Some(PathBuf::from("artist/album/track.flac"))
        );
    }
}
