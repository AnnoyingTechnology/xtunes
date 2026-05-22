// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

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

impl TrackMetadata {
    pub fn apply_change(&mut self, change: &MetadataChange) {
        apply_field_change(&mut self.title, &change.title);
        apply_field_change(&mut self.artist, &change.artist);
        apply_field_change(&mut self.album, &change.album);
        apply_field_change(&mut self.album_artist, &change.album_artist);
        apply_field_change(&mut self.composer, &change.composer);
        apply_field_change(&mut self.genre, &change.genre);
        apply_field_change(&mut self.track_number, &change.track_number);
        apply_field_change(&mut self.disc_number, &change.disc_number);
        apply_field_change(&mut self.year, &change.year);
    }
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

fn apply_field_change<T: Clone>(target: &mut Option<T>, change: &FieldChange<T>) {
    match change {
        FieldChange::Unchanged => {}
        FieldChange::Set(value) => {
            *target = Some(value.clone());
        }
        FieldChange::Clear => {
            *target = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FieldChange, MetadataChange, TrackMetadata};

    #[test]
    fn metadata_changes_default_to_unchanged() {
        let change = MetadataChange::default();

        assert_eq!(change.title, FieldChange::Unchanged);
        assert_eq!(change.artist, FieldChange::Unchanged);
        assert_eq!(change.track_number, FieldChange::Unchanged);
    }

    #[test]
    fn track_metadata_applies_field_changes() {
        let mut metadata = TrackMetadata {
            title: Some("Old".to_owned()),
            artist: Some("Artist".to_owned()),
            track_number: Some(1),
            ..TrackMetadata::default()
        };
        let change = MetadataChange {
            title: FieldChange::Set("New".to_owned()),
            artist: FieldChange::Clear,
            track_number: FieldChange::Unchanged,
            year: FieldChange::Set(1998),
            ..MetadataChange::default()
        };

        metadata.apply_change(&change);

        assert_eq!(metadata.title.as_deref(), Some("New"));
        assert_eq!(metadata.artist, None);
        assert_eq!(metadata.track_number, Some(1));
        assert_eq!(metadata.year, Some(1998));
    }
}
