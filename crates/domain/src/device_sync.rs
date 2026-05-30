// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Durable value types for syncing playlists to external devices
//! (USB sticks, SD cards, and — once its transport lands — Android).
//!
//! These are the facts Sustain owns and persists about a device: its
//! stable identity, the on-drive layout to write, and which playlists
//! were ticked for it. The device only carries half the story (which
//! files are present); the selection lives here, keyed by a Sustain-
//! generated id stored in a `.sustain-device-id` marker on the device.
//! The sync engine, identity probing, and on-drive writers live in the
//! `sustain-device-sync` crate; this module is pure data so the storage
//! layer can persist it without pulling in that machinery.

use crate::PlaylistItem;

/// Stable, transport-agnostic identifier Sustain assigns to a device on
/// first sync. Written into a `.sustain-device-id` marker on the device
/// and used as the SQLite key for the device's saved selection, options,
/// and manifest. Survives remounts and moving between machines.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SyncDeviceId(String);

impl SyncDeviceId {
    /// Wrap a non-empty id string (e.g. a generated UUID). Returns
    /// `None` for an empty or whitespace-only value.
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        if value.trim().is_empty() {
            None
        } else {
            Some(Self(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

/// The on-drive layout written for a device. A per-device choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceLayout {
    /// One canonical `Music/Artist/Album/NN Title.ext` tree (each track
    /// stored once) plus one UTF-8 `.m3u8` per playlist with relative
    /// paths. For phones and players that read playlists.
    M3u,
    /// One folder per playlist holding real audio copies (a track in
    /// three playlists is copied three times). For folder-navigating car
    /// stereos and dumb players.
    FolderPerPlaylist,
    /// Pioneer's on-device database format (`export.pdb` + ANLZ
    /// waveforms), consumable by Pioneer XDJ/CDJ hardware and Rekordbox.
    Pioneer,
}

impl DeviceLayout {
    pub const ALL: [Self; 3] = [Self::M3u, Self::FolderPerPlaylist, Self::Pioneer];

    pub const fn as_db(self) -> i64 {
        match self {
            Self::M3u => 0,
            Self::FolderPerPlaylist => 1,
            Self::Pioneer => 2,
        }
    }

    pub const fn from_db(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::M3u),
            1 => Some(Self::FolderPerPlaylist),
            2 => Some(Self::Pioneer),
            _ => None,
        }
    }

    /// Short label for the UI layout chooser.
    pub const fn label(self) -> &'static str {
        match self {
            Self::M3u => "Playlists as .m3u8",
            Self::FolderPerPlaylist => "One folder per playlist",
            Self::Pioneer => "Pioneer (Rekordbox / XDJ)",
        }
    }
}

/// Optional per-folder file count cap for [`DeviceLayout::FolderPerPlaylist`].
/// Off by default; opt in for memory-limited players that choke on large
/// directories. When a playlist exceeds the cap it is split into numbered
/// subfolders (`01/`, `02/`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilesPerFolderCap {
    Unlimited,
    N64,
    N128,
    N256,
    N512,
}

impl FilesPerFolderCap {
    pub const ALL: [Self; 5] = [
        Self::Unlimited,
        Self::N64,
        Self::N128,
        Self::N256,
        Self::N512,
    ];

    /// The numeric cap, or `None` for unlimited.
    pub const fn limit(self) -> Option<u32> {
        match self {
            Self::Unlimited => None,
            Self::N64 => Some(64),
            Self::N128 => Some(128),
            Self::N256 => Some(256),
            Self::N512 => Some(512),
        }
    }

    /// Persist as the numeric value (0 = unlimited).
    pub const fn as_db(self) -> i64 {
        match self.limit() {
            None => 0,
            Some(n) => n as i64,
        }
    }

    pub const fn from_db(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::Unlimited),
            64 => Some(Self::N64),
            128 => Some(Self::N128),
            256 => Some(Self::N256),
            512 => Some(Self::N512),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Unlimited => "Unlimited",
            Self::N64 => "64",
            Self::N128 => "128",
            Self::N256 => "256",
            Self::N512 => "512",
        }
    }
}

/// What kind of device this is — drives the sidebar icon and the default
/// sub-path. Android's MTP transport is not yet implemented; the variant
/// exists so a recognised phone can be represented and configured.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceKind {
    /// A mounted block device: USB stick, SD card, external SSD.
    UsbDrive,
    /// An Android phone or tablet (MTP transport pending).
    Android,
}

impl DeviceKind {
    pub const fn as_db(self) -> i64 {
        match self {
            Self::UsbDrive => 0,
            Self::Android => 1,
        }
    }

    pub const fn from_db(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::UsbDrive),
            1 => Some(Self::Android),
            _ => None,
        }
    }

    /// Default sub-path under the device root to sync into. Android
    /// targets its `Music` folder; a plain drive targets the root.
    pub const fn default_sub_path(self) -> &'static str {
        match self {
            Self::UsbDrive => "",
            Self::Android => "Music",
        }
    }
}

/// A device Sustain knows about: its identity plus the saved per-device
/// configuration. The ticked playlists are stored separately (see
/// [`crate::PlaylistItem`]); this is everything else.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncDevice {
    pub id: SyncDeviceId,
    /// Human-readable name shown in the sidebar.
    pub label: String,
    pub kind: DeviceKind,
    pub layout: DeviceLayout,
    /// Sub-path under the device root to sync into. Empty = device root.
    pub sub_path: String,
    pub files_per_folder_cap: FilesPerFolderCap,
    /// Filesystem volume id, used only to re-recognise a device whose
    /// marker file was deleted. `None` until first observed.
    pub volume_id: Option<String>,
}

/// One row of a device's sync manifest: a track Sustain last wrote to
/// the device, where it put it, and a fingerprint of the source content
/// at the time. On re-sync the engine diffs the resolved track set
/// against these rows and copies only what changed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncManifestEntry {
    pub track_id: crate::TrackId,
    /// Path on the device, relative to the device root.
    pub on_device_path: String,
    /// Fingerprint of the source file when it was last written (content
    /// hash when known, else a size-based token). A change means the
    /// on-device copy is stale and must be rewritten.
    pub fingerprint: String,
}

/// A device's saved playlist selection: the ticked playlists and smart
/// playlists, in display order. Folders are not selectable for sync.
pub type DeviceSelection = Vec<PlaylistItem>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_rejects_blank() {
        assert!(SyncDeviceId::new("  ").is_none());
        assert_eq!(
            SyncDeviceId::new("abc").map(SyncDeviceId::into_string),
            Some("abc".to_owned())
        );
    }

    #[test]
    fn layout_round_trips_through_db() {
        for layout in DeviceLayout::ALL {
            assert_eq!(DeviceLayout::from_db(layout.as_db()), Some(layout));
        }
        assert_eq!(DeviceLayout::from_db(99), None);
    }

    #[test]
    fn cap_round_trips_and_reports_limit() {
        for cap in FilesPerFolderCap::ALL {
            assert_eq!(FilesPerFolderCap::from_db(cap.as_db()), Some(cap));
        }
        assert_eq!(FilesPerFolderCap::Unlimited.limit(), None);
        assert_eq!(FilesPerFolderCap::N128.limit(), Some(128));
    }

    #[test]
    fn kind_round_trips_and_has_default_sub_path() {
        for kind in [DeviceKind::UsbDrive, DeviceKind::Android] {
            assert_eq!(DeviceKind::from_db(kind.as_db()), Some(kind));
        }
        assert_eq!(DeviceKind::Android.default_sub_path(), "Music");
        assert_eq!(DeviceKind::UsbDrive.default_sub_path(), "");
    }
}
