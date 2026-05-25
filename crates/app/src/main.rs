// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

#![forbid(unsafe_code)]

use std::{process, sync::Arc};

fn main() {
    let settings_store = match sustain_settings::TomlSettingsStore::open_default() {
        Ok(store) => store,
        Err(error) => {
            eprintln!(
                "Sustain: config directory unavailable ({error:?}). Cannot persist settings."
            );
            process::exit(1);
        }
    };
    let settings_path = settings_store.path().to_path_buf();
    let mut runtime = match sustain_app_runtime::ApplicationRuntime::with_settings_store(Box::new(
        settings_store,
    )) {
        Ok(runtime) => runtime,
        Err(_) => {
            // Sustain is pre-release and ships no migration code. Any load
            // failure on an existing file means the on-disk format is from a
            // previous development iteration. The fix is to delete it.
            eprintln!(
                "Sustain: settings file at {} could not be loaded.",
                settings_path.display()
            );
            eprintln!(
                "The file is in an incompatible/outdated format. Delete it and restart Sustain."
            );
            process::exit(1);
        }
    };

    match sustain_library_store::SqliteLibraryStore::open_default() {
        Ok(library_store) => {
            if let Err(error) = runtime.set_library_services(
                Arc::new(library_store),
                Arc::new(sustain_metadata::LoftyMetadataService),
            ) {
                eprintln!("Sustain: library services failed to initialize ({error:?}).");
                process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("Sustain: library database is unavailable ({error:?}).");
            process::exit(1);
        }
    }

    if let Ok(playback_service) = sustain_playback::GStreamerPlaybackService::new() {
        runtime = runtime.with_playback_service(Box::new(playback_service));
    }

    // Known GTK/GDK runtime warning on some Wayland/Vulkan setups:
    // `vkAcquireNextImageKHR(): ... VK_SUBOPTIMAL_KHR`.
    // This is emitted below Sustain by GTK's Vulkan renderer when the swapchain
    // becomes suboptimal, commonly around resize/scale/surface changes. Rendering
    // can still present successfully, so we intentionally do not filter the log or
    // force `GSK_RENDERER` here. If it becomes visually broken, prefer documenting
    // `GSK_RENDERER=ngl` / `GSK_RENDERER=gl` as a user workaround before changing
    // the app default.
    sustain_ui_gtk::run(runtime);
}
