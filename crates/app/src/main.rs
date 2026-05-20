#![forbid(unsafe_code)]

use std::sync::Arc;

fn main() {
    let mut runtime = xtunes_settings::TomlSettingsStore::open_default()
        .ok()
        .and_then(|settings_store| {
            xtunes_app_runtime::ApplicationRuntime::with_settings_store(Box::new(settings_store))
                .ok()
        })
        .unwrap_or_default();

    if let Ok(library_store) = xtunes_library_store::SqliteLibraryStore::open_default() {
        runtime = runtime.with_library_services(
            Arc::new(library_store),
            Arc::new(xtunes_metadata::LoftyMetadataService),
        );
    }

    if let Ok(playback_service) = xtunes_playback::GStreamerPlaybackService::new() {
        runtime = runtime.with_playback_service(Box::new(playback_service));
    }

    xtunes_ui_gtk::run(runtime);
}
