// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

#![forbid(unsafe_code)]

use std::{process, sync::Arc};

fn main() {
    let settings_store = match xtunes_settings::TomlSettingsStore::open_default() {
        Ok(store) => store,
        Err(error) => {
            eprintln!("xTunes: config directory unavailable ({error:?}). Cannot persist settings.");
            process::exit(1);
        }
    };
    let settings_path = settings_store.path().to_path_buf();
    let mut runtime = match xtunes_app_runtime::ApplicationRuntime::with_settings_store(Box::new(
        settings_store,
    )) {
        Ok(runtime) => runtime,
        Err(_) => {
            // xTunes is pre-release and ships no migration code. Any load
            // failure on an existing file means the on-disk format is from a
            // previous development iteration. The fix is to delete it.
            eprintln!(
                "xTunes: settings file at {} could not be loaded.",
                settings_path.display()
            );
            eprintln!(
                "The file is in an incompatible/outdated format. Delete it and restart xTunes."
            );
            process::exit(1);
        }
    };

    if let Ok(library_store) = xtunes_library_store::SqliteLibraryStore::open_default() {
        if let Err(error) = runtime.set_library_services(
            Arc::new(library_store),
            Arc::new(xtunes_metadata::LoftyMetadataService),
        ) {
            eprintln!("Failed to initialize xTunes library services: {error:?}");
        }
    }

    if let Ok(playback_service) = xtunes_playback::GStreamerPlaybackService::new() {
        runtime = runtime.with_playback_service(Box::new(playback_service));
    }

    xtunes_ui_gtk::run(runtime);
}
