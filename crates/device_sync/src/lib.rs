// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

#![forbid(unsafe_code)]

//! Sync playlists from the library to external devices (`sustain-device-sync`).
//!
//! Implements the shared device-sync spine of issues #23/#24: device
//! identity and discovery ([`identity`]), an incremental content-aware
//! differ and copy [`engine`], and three on-drive layouts —
//! deduplicated `.m3u8`, one-folder-per-playlist, and Pioneer's
//! `export.pdb` + ANLZ format. The library's database and DSP pipeline
//! are not reached directly: the caller resolves a device's ticked
//! playlists (smart playlists re-evaluated each sync) into the neutral
//! [`model`] inputs and hands them here.

pub mod engine;
pub mod identity;
mod layout;
pub mod model;
mod sanitize;

pub use engine::{plan, sync};
pub use identity::{
    ConnectedDevice, MARKER_FILE, discover, generate_device_id, read_marker, write_marker,
};
pub use model::{
    Placement, SyncError, SyncInputPlaylist, SyncInputTrack, SyncOutcome, SyncPlan, SyncProgress,
    SyncRequest, SyncStage,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use sustain_domain::{
        DeviceKind, DeviceLayout, FilesPerFolderCap, SyncDevice, SyncDeviceId, TrackId,
    };

    struct Fixture {
        _src: tempfile::TempDir,
        dest: tempfile::TempDir,
        tracks: Vec<SyncInputTrack>,
    }

    fn fixture(count: usize) -> Fixture {
        let src = tempfile::tempdir().expect("src dir");
        let dest = tempfile::tempdir().expect("dest dir");
        let mut tracks = Vec::new();
        for i in 1..=count {
            let path = src.path().join(format!("song{i}.mp3"));
            std::fs::write(&path, format!("audio-data-{i}").repeat(4)).expect("write src");
            tracks.push(SyncInputTrack {
                track_id: TrackId::new(i as i64).expect("id"),
                source_path: path,
                title: format!("Title {i}"),
                artist: format!("Artist {}", (i % 2) + 1),
                album: format!("Album {}", (i % 2) + 1),
                genre: Some("House".into()),
                track_number: Some(i as u32),
                year: Some(2020),
                duration_ms: 200_000,
                rating: 3,
                bpm: Some(128.0),
                key: Some(sustain_domain::MusicalKey::AMinor),
                bitrate_kbps: Some(320),
                sample_rate_hz: 44_100,
                bit_depth: 16,
                file_size: 0,
                date_added: Some("2026-01-01".into()),
                extension: "mp3".into(),
                fingerprint: format!("fp-{i}"),
                waveform_preview: None,
                waveform_detail: None,
            });
        }
        Fixture {
            _src: src,
            dest,
            tracks,
        }
    }

    fn device(layout: DeviceLayout) -> SyncDevice {
        SyncDevice {
            id: SyncDeviceId::new("test-device").expect("id"),
            label: "Test".into(),
            kind: DeviceKind::UsbDrive,
            layout,
            sub_path: String::new(),
            files_per_folder_cap: FilesPerFolderCap::Unlimited,
            volume_id: None,
        }
    }

    fn request(
        fx: &Fixture,
        layout: DeviceLayout,
        prev: Vec<sustain_domain::SyncManifestEntry>,
        remove: bool,
    ) -> SyncRequest {
        SyncRequest {
            device: device(layout),
            mount_path: fx.dest.path().to_path_buf(),
            tracks: fx.tracks.clone(),
            playlists: vec![SyncInputPlaylist {
                name: "My Set".into(),
                track_indices: (0..fx.tracks.len()).collect(),
            }],
            previous_manifest: prev,
            remove_stale: remove,
            export_date: "2026-01-01".into(),
        }
    }

    fn run(req: &SyncRequest) -> SyncOutcome {
        sync(req, &mut |_| {}, &|| false).expect("sync ok")
    }

    #[test]
    fn m3u_layout_writes_tree_and_playlist() {
        let fx = fixture(3);
        let req = request(&fx, DeviceLayout::M3u, Vec::new(), false);
        let outcome = run(&req);
        assert_eq!(outcome.copied, 3);
        assert!(fx.dest.path().join("My Set.m3u8").exists());
        let m3u = std::fs::read_to_string(fx.dest.path().join("My Set.m3u8")).expect("read m3u");
        assert!(m3u.starts_with("#EXTM3U"));
        assert!(m3u.contains("Music/"));
        // Audio tree exists and is deduplicated (3 files).
        let count = walk_files(fx.dest.path().join("Music"));
        assert_eq!(count, 3);
    }

    #[test]
    fn folder_layout_copies_per_playlist_and_is_stable() {
        let fx = fixture(2);
        let req = request(&fx, DeviceLayout::FolderPerPlaylist, Vec::new(), false);
        let first = run(&req);
        assert_eq!(first.copied, 2);
        assert!(fx.dest.path().join("My Set").is_dir());

        // Re-sync with the prior manifest: nothing should be recopied.
        let req2 = request(
            &fx,
            DeviceLayout::FolderPerPlaylist,
            first.manifest.clone(),
            false,
        );
        let second = run(&req2);
        assert_eq!(second.copied, 0);
        assert_eq!(second.updated, 0);
        assert_eq!(second.unchanged, 2);
        // The on-device paths are identical across syncs.
        let mut a: Vec<_> = first.manifest.iter().map(|m| &m.on_device_path).collect();
        let mut b: Vec<_> = second.manifest.iter().map(|m| &m.on_device_path).collect();
        a.sort();
        b.sort();
        assert_eq!(a, b);
    }

    #[test]
    fn pioneer_layout_writes_pdb_and_anlz() {
        let fx = fixture(2);
        let req = request(&fx, DeviceLayout::Pioneer, Vec::new(), false);
        let outcome = run(&req);
        assert_eq!(outcome.copied, 2);
        assert!(fx.dest.path().join("PIONEER/rekordbox/export.pdb").exists());
        assert!(fx.dest.path().join("Contents").is_dir());
        // At least one ANLZ .EXT was written under USBANLZ.
        let exts = walk_files(fx.dest.path().join("PIONEER/USBANLZ"));
        assert!(exts >= 2, "expected per-track ANLZ files, found {exts}");
    }

    #[test]
    fn incremental_resync_copies_nothing_when_unchanged() {
        let fx = fixture(3);
        let req = request(&fx, DeviceLayout::M3u, Vec::new(), false);
        let first = run(&req);
        let req2 = request(&fx, DeviceLayout::M3u, first.manifest.clone(), false);
        let second = run(&req2);
        assert_eq!(second.copied, 0);
        assert_eq!(second.unchanged, 3);
    }

    #[test]
    fn removal_only_with_confirmation() {
        let fx = fixture(3);
        // First sync all three.
        let first = run(&request(&fx, DeviceLayout::M3u, Vec::new(), false));

        // Shrink the resolved selection to the first two tracks (the
        // runtime passes only selected tracks as `req.tracks`).
        let shrink = |remove: bool| SyncRequest {
            device: device(DeviceLayout::M3u),
            mount_path: fx.dest.path().to_path_buf(),
            tracks: fx.tracks[..2].to_vec(),
            playlists: vec![SyncInputPlaylist {
                name: "My Set".into(),
                track_indices: vec![0, 1],
            }],
            previous_manifest: first.manifest.clone(),
            remove_stale: remove,
            export_date: "2026-01-01".into(),
        };

        // Without confirmation, the third file stays and remains tracked.
        let kept = sync(&shrink(false), &mut |_| {}, &|| false).expect("sync");
        assert_eq!(kept.removed, 0);
        assert_eq!(kept.manifest.len(), 3);

        // With confirmation, the stale file is removed.
        let removed = sync(&shrink(true), &mut |_| {}, &|| false).expect("sync");
        assert_eq!(removed.removed, 1);
        assert_eq!(removed.manifest.len(), 2);
    }

    #[test]
    fn marker_is_written_on_sync() {
        let fx = fixture(1);
        let req = request(&fx, DeviceLayout::M3u, Vec::new(), false);
        run(&req);
        assert_eq!(
            read_marker(fx.dest.path()).map(SyncDeviceId::into_string),
            Some("test-device".to_owned())
        );
    }

    #[test]
    fn empty_selection_is_rejected() {
        let fx = fixture(0);
        let req = SyncRequest {
            device: device(DeviceLayout::M3u),
            mount_path: fx.dest.path().to_path_buf(),
            tracks: Vec::new(),
            playlists: Vec::new(),
            previous_manifest: Vec::new(),
            remove_stale: false,
            export_date: "2026-01-01".into(),
        };
        assert!(matches!(
            sync(&req, &mut |_| {}, &|| false),
            Err(SyncError::Empty)
        ));
    }

    fn walk_files(dir: PathBuf) -> usize {
        let mut count = 0;
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    count += walk_files(path);
                } else {
                    count += 1;
                }
            }
        }
        count
    }
}
