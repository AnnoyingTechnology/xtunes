// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

#![forbid(unsafe_code)]

//! Pioneer DJ on-drive format writer (`sustain-pioneer`).
//!
//! A self-contained re-derivation of the Pioneer DeviceSQL export
//! format consumed by CDJ/XDJ hardware and Rekordbox: the `export.pdb`
//! track database, the `ANLZ0000.DAT`/`.EXT` analysis files, the
//! waveform encodings, the cover-art thumbnails, and the path-hash that
//! addresses a track's analysis directory. Beyond Sustain's neutral
//! value types ([`sustain_domain`]) its only weight is image decoding
//! for the [`artwork`] thumbnails — no DSP, no audio decoding, no
//! storage — so the format lives in isolation and is driven by callers
//! that translate Sustain's library model into the flat [`model`]
//! inputs.
//!
//! The byte-level layout, the reverse-engineered page-header formulas,
//! and the constant field values are preserved from the maintainer's
//! hardware-validated reference exporter; the code is a clean
//! restructure, not a verbatim lift. See [`pdb`] and [`path_hash`] for
//! the format details and provenance.

pub mod anlz;
pub mod artwork;
pub mod device_sql;
pub mod key;
pub mod model;
pub mod path_hash;
pub mod pdb;
pub mod waveform;

pub use artwork::{ArtworkError, ArtworkSet};
pub use model::{AnlzInput, PioneerArtwork, PioneerFileType, PioneerPlaylist, PioneerTrack};
// The `path_hash` *module* is public; the free functions are reached as
// `path_hash::anlz_dir` etc. (re-exporting the `path_hash` function here
// too would collide with the module name).
pub use pdb::PdbError;

/// File name of the track database within `PIONEER/rekordbox/`.
pub const PDB_RELATIVE_PATH: &str = "PIONEER/rekordbox/export.pdb";

/// Root directory of the audio files on the drive (each track stored
/// once under `Contents/Artist/Album/`).
pub const CONTENTS_DIR: &str = "Contents";
