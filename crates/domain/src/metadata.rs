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
    pub grouping: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<u32>,
    pub track_total: Option<u32>,
    pub disc_number: Option<u32>,
    pub disc_total: Option<u32>,
    pub year: Option<i32>,
    pub compilation: Option<bool>,
    pub bpm: Option<u32>,
    pub key: Option<String>,
    pub comments: Option<String>,
    pub duration: Option<Duration>,
    pub bitrate_kbps: Option<u32>,
    pub sample_rate_hz: Option<u32>,
    pub channels: Option<u8>,
}

impl TrackMetadata {
    pub fn apply_change(&mut self, change: &MetadataChange) {
        apply_field_change(&mut self.title, &change.title);
        apply_field_change(&mut self.artist, &change.artist);
        apply_field_change(&mut self.album, &change.album);
        apply_field_change(&mut self.album_artist, &change.album_artist);
        apply_field_change(&mut self.composer, &change.composer);
        apply_field_change(&mut self.grouping, &change.grouping);
        apply_field_change(&mut self.genre, &change.genre);
        apply_field_change(&mut self.track_number, &change.track_number);
        apply_field_change(&mut self.track_total, &change.track_total);
        apply_field_change(&mut self.disc_number, &change.disc_number);
        apply_field_change(&mut self.disc_total, &change.disc_total);
        apply_field_change(&mut self.year, &change.year);
        apply_field_change(&mut self.compilation, &change.compilation);
        apply_field_change(&mut self.bpm, &change.bpm);
        apply_field_change(&mut self.key, &change.key);
        apply_field_change(&mut self.comments, &change.comments);
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
    pub grouping: FieldChange<String>,
    pub genre: FieldChange<String>,
    pub track_number: FieldChange<u32>,
    pub track_total: FieldChange<u32>,
    pub disc_number: FieldChange<u32>,
    pub disc_total: FieldChange<u32>,
    pub year: FieldChange<i32>,
    pub compilation: FieldChange<bool>,
    pub bpm: FieldChange<u32>,
    pub key: FieldChange<String>,
    pub comments: FieldChange<String>,
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

    #[test]
    fn track_metadata_applies_extended_field_changes() {
        let mut metadata = TrackMetadata::default();
        let change = MetadataChange {
            grouping: FieldChange::Set("Workout".to_owned()),
            track_total: FieldChange::Set(12),
            disc_total: FieldChange::Set(2),
            compilation: FieldChange::Set(true),
            bpm: FieldChange::Set(128),
            key: FieldChange::Set("Am".to_owned()),
            comments: FieldChange::Set("Note".to_owned()),
            ..MetadataChange::default()
        };

        metadata.apply_change(&change);

        assert_eq!(metadata.grouping.as_deref(), Some("Workout"));
        assert_eq!(metadata.track_total, Some(12));
        assert_eq!(metadata.disc_total, Some(2));
        assert_eq!(metadata.compilation, Some(true));
        assert_eq!(metadata.bpm, Some(128));
        assert_eq!(metadata.key.as_deref(), Some("Am"));
        assert_eq!(metadata.comments.as_deref(), Some("Note"));
    }

    #[test]
    fn track_metadata_clears_extended_field_changes() {
        let mut metadata = TrackMetadata {
            grouping: Some("Old group".to_owned()),
            compilation: Some(true),
            bpm: Some(100),
            ..TrackMetadata::default()
        };
        let change = MetadataChange {
            grouping: FieldChange::Clear,
            compilation: FieldChange::Clear,
            bpm: FieldChange::Clear,
            ..MetadataChange::default()
        };

        metadata.apply_change(&change);

        assert_eq!(metadata.grouping, None);
        assert_eq!(metadata.compilation, None);
        assert_eq!(metadata.bpm, None);
    }
}
