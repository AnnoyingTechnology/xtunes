#![forbid(unsafe_code)]

use std::path::PathBuf;

pub use xtunes_domain::{PlayStatistics, Rating, TrackMetadata};

pub type ImportResult<T> = Result<T, ImportError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImportSource {
    RhythmboxLibraryXml(PathBuf),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImportError {
    SourceUnavailable,
    InvalidSource,
    UnsupportedSource,
}

pub trait LibraryImporter {
    fn import_library(&self, source: ImportSource) -> ImportResult<ImportedLibrary>;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImportedLibrary {
    pub tracks: Vec<ImportedTrack>,
    pub playlists: Vec<ImportedPlaylist>,
}

impl ImportedLibrary {
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty() && self.playlists.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportedTrack {
    pub source_id: ImportedTrackId,
    pub path: PathBuf,
    pub metadata: TrackMetadata,
    pub rating: Rating,
    pub statistics: PlayStatistics,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ImportedTrackId(String);

impl ImportedTrackId {
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into().trim().to_owned();
        if value.is_empty() {
            None
        } else {
            Some(Self(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportedPlaylist {
    pub name: String,
    pub entries: Vec<ImportedPlaylistEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportedPlaylistEntry {
    pub track_source_id: ImportedTrackId,
    pub position: u32,
}

#[cfg(test)]
mod tests {
    use super::{ImportedLibrary, ImportedTrackId};

    #[test]
    fn imported_library_defaults_to_empty() {
        assert!(ImportedLibrary::default().is_empty());
    }

    #[test]
    fn imported_track_id_rejects_blank_values() {
        assert_eq!(ImportedTrackId::new(""), None);
        assert_eq!(ImportedTrackId::new("   "), None);
    }

    #[test]
    fn imported_track_id_trims_values() {
        let id = ImportedTrackId::new("  rhythmdb:42  ");

        assert_eq!(
            id.as_ref().map(ImportedTrackId::as_str),
            Some("rhythmdb:42")
        );
    }
}
