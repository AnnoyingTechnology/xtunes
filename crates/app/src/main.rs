#![forbid(unsafe_code)]

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
            Box::new(library_store),
            Box::new(xtunes_metadata::LoftyMetadataService),
        );
    }

    if let Ok(playback_service) = xtunes_playback::GStreamerPlaybackService::new() {
        runtime = runtime.with_playback_service(Box::new(playback_service));
    }

    xtunes_ui_gtk::run(runtime);
}
