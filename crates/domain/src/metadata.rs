use std::time::Duration;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TrackMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub composer: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<i32>,
    pub duration: Option<Duration>,
    pub bitrate_kbps: Option<u32>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum FieldChange<T> {
    #[default]
    Unchanged,
    Set(T),
    Clear,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MetadataChange {
    pub title: FieldChange<String>,
    pub artist: FieldChange<String>,
    pub album: FieldChange<String>,
    pub album_artist: FieldChange<String>,
    pub composer: FieldChange<String>,
    pub genre: FieldChange<String>,
    pub track_number: FieldChange<u32>,
    pub disc_number: FieldChange<u32>,
    pub year: FieldChange<i32>,
}

#[cfg(test)]
mod tests {
    use super::{FieldChange, MetadataChange};

    #[test]
    fn metadata_changes_default_to_unchanged() {
        let change = MetadataChange::default();

        assert_eq!(change.title, FieldChange::Unchanged);
        assert_eq!(change.artist, FieldChange::Unchanged);
        assert_eq!(change.track_number, FieldChange::Unchanged);
    }
}
