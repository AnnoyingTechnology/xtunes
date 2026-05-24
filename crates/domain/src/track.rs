// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::path::{Component, Path, PathBuf};

use crate::{PlayStatistics, Rating, TrackId, TrackMetadata};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Track {
    pub id: TrackId,
    pub location: TrackLocation,
    pub content_hash: Option<TrackContentHash>,
    pub metadata: TrackMetadata,
    pub rating: Rating,
    pub statistics: PlayStatistics,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackLocation {
    pub file_path: TrackFilePath,
    pub availability: TrackAvailability,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TrackRelativePath(PathBuf);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TrackFilePath {
    LibraryRelative(TrackRelativePath),
    External(PathBuf),
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TrackContentHash(String);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TrackAvailability {
    #[default]
    Available,
    Missing,
}

impl TrackLocation {
    pub const fn available(relative_path: TrackRelativePath) -> Self {
        Self {
            file_path: TrackFilePath::LibraryRelative(relative_path),
            availability: TrackAvailability::Available,
        }
    }

    pub const fn missing(relative_path: TrackRelativePath) -> Self {
        Self {
            file_path: TrackFilePath::LibraryRelative(relative_path),
            availability: TrackAvailability::Missing,
        }
    }

    pub fn available_external(path: impl Into<PathBuf>) -> Option<Self> {
        Some(Self {
            file_path: TrackFilePath::External(normalize_absolute_path(path.into())?),
            availability: TrackAvailability::Available,
        })
    }

    pub fn missing_external(path: impl Into<PathBuf>) -> Option<Self> {
        Some(Self {
            file_path: TrackFilePath::External(normalize_absolute_path(path.into())?),
            availability: TrackAvailability::Missing,
        })
    }

    pub fn is_missing(&self) -> bool {
        self.availability == TrackAvailability::Missing
    }

    pub fn is_library_relative(&self) -> bool {
        matches!(self.file_path, TrackFilePath::LibraryRelative(_))
    }

    pub fn library_relative_path(&self) -> Option<&TrackRelativePath> {
        match &self.file_path {
            TrackFilePath::LibraryRelative(relative_path) => Some(relative_path),
            TrackFilePath::External(_) => None,
        }
    }

    pub fn path(&self) -> &Path {
        self.file_path.as_path()
    }

    pub fn absolute_path(&self, library_root: Option<&Path>) -> Option<PathBuf> {
        match &self.file_path {
            TrackFilePath::LibraryRelative(relative_path) => {
                library_root.map(|library_root| relative_path.resolve(library_root))
            }
            TrackFilePath::External(path) => Some(path.clone()),
        }
    }

    pub fn with_availability(self, availability: TrackAvailability) -> Self {
        Self {
            file_path: self.file_path,
            availability,
        }
    }
}

impl TrackFilePath {
    pub fn as_path(&self) -> &Path {
        match self {
            Self::LibraryRelative(relative_path) => relative_path.as_path(),
            Self::External(path) => path.as_path(),
        }
    }

    pub fn storage_kind(&self) -> &'static str {
        match self {
            Self::LibraryRelative(_) => "library_relative",
            Self::External(_) => "external",
        }
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

impl TrackContentHash {
    pub fn new(value: impl AsRef<str>) -> Option<Self> {
        let value = value.as_ref().trim();
        if value.len() != 64 || !value.chars().all(|character| character.is_ascii_hexdigit()) {
            return None;
        }
        Some(Self(value.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
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

fn normalize_absolute_path(path: PathBuf) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }

    let mut normalized = PathBuf::new();
    let mut has_file_component = false;
    for component in path.components() {
        match component {
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(value) => {
                normalized.push(value);
                has_file_component = true;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => return None,
        }
    }

    has_file_component.then_some(normalized)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{TrackContentHash, TrackLocation, TrackRelativePath};

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

    #[test]
    fn external_locations_require_absolute_paths() {
        assert_eq!(TrackLocation::available_external("track.flac"), None);
        assert!(
            TrackLocation::available_external("/home/user/Music/track.flac")
                .expect("absolute path")
                .path()
                .is_absolute()
        );
    }

    #[test]
    fn external_locations_reject_parent_components() {
        assert_eq!(
            TrackLocation::available_external("/home/user/../track.flac"),
            None
        );
    }

    #[test]
    fn content_hash_accepts_sha256_hex_values() {
        let hash = TrackContentHash::new(
            "ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
        )
        .expect("valid hash");

        assert_eq!(
            hash.as_str(),
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        );
    }

    #[test]
    fn content_hash_rejects_invalid_values() {
        assert_eq!(TrackContentHash::new("abc"), None);
        assert_eq!(
            TrackContentHash::new(
                "xyzdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
            ),
            None
        );
    }
}
