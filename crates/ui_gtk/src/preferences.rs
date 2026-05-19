use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use super::{
    ApplicationCommand, ApplicationRuntimeError, LibraryChangedCallback, LibraryScanSummary,
    PREFERENCES_HEIGHT, PREFERENCES_WIDTH, SharedRuntime, UserSettings, WINDOW_SHADOW_MARGIN,
};

pub(crate) fn install_preferences_action(
    app: &gtk::Application,
    window: &gtk::ApplicationWindow,
    runtime: SharedRuntime,
    library_changed: LibraryChangedCallback,
) {
    if app.lookup_action("preferences").is_some() {
        return;
    }

    let preferences = gio::SimpleAction::new("preferences", None);
    let window = window.clone();
    let runtime = runtime.clone();
    let library_changed = library_changed.clone();
    preferences.connect_activate(move |_action, _parameter| {
        open_preferences_window(&window, runtime.clone(), library_changed.clone());
    });
    app.add_action(&preferences);
    app.set_accels_for_action("app.preferences", &["<Primary>comma"]);
}

pub(crate) fn settings_button(
    window: &gtk::ApplicationWindow,
    runtime: SharedRuntime,
    library_changed: LibraryChangedCallback,
) -> gtk::Button {
    let icon = gtk::Image::from_icon_name("preferences-system-symbolic");
    icon.set_pixel_size(18);

    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("settings-button");
    button.set_child(Some(&icon));
    button.set_tooltip_text(Some("Preferences"));
    button.set_valign(gtk::Align::Center);

    let window = window.clone();
    let runtime = runtime.clone();
    let library_changed = library_changed.clone();
    button.connect_clicked(move |_| {
        open_preferences_window(&window, runtime.clone(), library_changed.clone());
    });

    button
}

fn open_preferences_window(
    parent: &gtk::ApplicationWindow,
    runtime: SharedRuntime,
    library_changed: LibraryChangedCallback,
) {
    let window = gtk::Window::builder()
        .title("Library Path")
        .decorated(false)
        .transient_for(parent)
        .modal(true)
        .default_width(PREFERENCES_WIDTH + WINDOW_SHADOW_MARGIN * 2)
        .default_height(PREFERENCES_HEIGHT + WINDOW_SHADOW_MARGIN * 2)
        .resizable(false)
        .build();
    window.add_css_class("app-window");

    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("preferences-frame");
    frame.set_hexpand(true);
    frame.set_vexpand(true);
    frame.set_margin_top(WINDOW_SHADOW_MARGIN);
    frame.set_margin_end(WINDOW_SHADOW_MARGIN);
    frame.set_margin_bottom(WINDOW_SHADOW_MARGIN);
    frame.set_margin_start(WINDOW_SHADOW_MARGIN);
    frame.set_size_request(PREFERENCES_WIDTH, PREFERENCES_HEIGHT);

    let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    panel.add_css_class("preferences-panel");
    panel.set_hexpand(true);
    panel.set_vexpand(true);
    panel.set_overflow(gtk::Overflow::Hidden);

    let close_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    close_row.set_margin_top(8);
    close_row.set_margin_end(8);
    close_row.set_margin_start(8);

    let close_icon = gtk::Image::from_icon_name("window-close-symbolic");
    close_icon.set_pixel_size(14);

    let close_button = gtk::Button::new();
    close_button.add_css_class("flat");
    close_button.add_css_class("preference-close-button");
    close_button.set_child(Some(&close_icon));
    close_button.set_tooltip_text(Some("Close"));
    close_button.set_halign(gtk::Align::End);
    close_button.set_valign(gtk::Align::Center);
    close_button.set_hexpand(true);

    let window_for_close = window.clone();
    close_button.connect_clicked(move |_| {
        window_for_close.close();
    });
    close_row.append(&close_button);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.set_margin_top(24);
    content.set_margin_end(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);

    let label_group = gtk::Box::new(gtk::Orientation::Vertical, 3);

    let library_path_label = gtk::Label::new(Some("Library path"));
    library_path_label.set_xalign(0.0);

    let library_path_help =
        gtk::Label::new(Some("Files in this folder are scanned into your library."));
    library_path_help.add_css_class("preference-helper");
    library_path_help.set_xalign(0.0);
    library_path_help.set_wrap(true);

    label_group.append(&library_path_label);
    label_group.append(&library_path_help);

    let path_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);

    let path_entry = gtk::Entry::new();
    path_entry.set_hexpand(true);
    path_entry.set_placeholder_text(Some("/home/julien/Music"));
    if let Some(library_path) = &runtime.borrow().settings().library_path {
        path_entry.set_text(&library_path.to_string_lossy());
    }

    let folder_icon = gtk::Image::from_icon_name("folder-open-symbolic");
    let folder_button = gtk::Button::new();
    folder_button.add_css_class("flat");
    folder_button.add_css_class("settings-button");
    folder_button.set_child(Some(&folder_icon));
    folder_button.set_tooltip_text(Some("Choose folder"));

    let scan_button = gtk::Button::with_label("Scan");
    scan_button.set_tooltip_text(Some("Save library path and scan now"));

    let scan_status = gtk::Label::new(None);
    scan_status.add_css_class("preference-helper");
    scan_status.set_xalign(0.0);
    scan_status.set_wrap(true);

    let runtime_for_entry = runtime.clone();
    path_entry.connect_activate(move |entry| {
        let _result = save_library_path_from_entry(&runtime_for_entry, entry);
    });

    let window_for_folder = window.clone();
    let runtime_for_folder = runtime.clone();
    let path_entry_for_folder = path_entry.clone();
    folder_button.connect_clicked(move |_| {
        open_library_folder_chooser(
            &window_for_folder,
            &runtime_for_folder,
            &path_entry_for_folder,
        );
    });

    let runtime_for_scan = runtime.clone();
    let path_entry_for_scan = path_entry.clone();
    let scan_status_for_scan = scan_status.clone();
    let library_changed_for_scan = library_changed.clone();
    scan_button.connect_clicked(move |_| {
        scan_status_for_scan.set_text("Scanning...");
        let scan_message =
            match save_library_path_from_entry(&runtime_for_scan, &path_entry_for_scan) {
                Ok(()) => {
                    match request_library_scan_from_entry(&runtime_for_scan, &path_entry_for_scan) {
                        Ok(Some(summary)) => {
                            library_changed_for_scan();
                            scan_summary_text(&summary)
                        }
                        Ok(None) => "Choose a library folder before scanning.".to_owned(),
                        Err(error) => scan_error_text(error).to_owned(),
                    }
                }
                Err(error) => scan_error_text(error).to_owned(),
            };
        scan_status_for_scan.set_text(&scan_message);
    });

    path_row.append(&path_entry);
    path_row.append(&folder_button);
    path_row.append(&scan_button);

    content.append(&label_group);
    content.append(&path_row);
    content.append(&scan_status);
    panel.append(&close_row);
    panel.append(&content);
    frame.append(&panel);
    window.set_child(Some(&frame));

    let key_controller = gtk::EventControllerKey::new();
    let window_for_escape = window.clone();
    key_controller.connect_key_pressed(move |_controller, key, _keycode, _state| {
        if key == gdk::Key::Escape {
            window_for_escape.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key_controller);

    window.present();
}

fn open_library_folder_chooser(
    parent: &gtk::Window,
    runtime: &SharedRuntime,
    path_entry: &gtk::Entry,
) {
    let dialog = gtk::FileChooserNative::builder()
        .title("Choose Library Folder")
        .action(gtk::FileChooserAction::SelectFolder)
        .accept_label("Choose")
        .cancel_label("Cancel")
        .modal(true)
        .transient_for(parent)
        .build();

    let current_path = path_entry.text().trim().to_owned();
    if !current_path.is_empty() {
        let current_folder = gio::File::for_path(current_path);
        let _result = dialog.set_current_folder(Some(&current_folder));
    }

    let runtime = runtime.clone();
    let path_entry = path_entry.clone();
    dialog.run_async(move |dialog, response| {
        if response == gtk::ResponseType::Accept {
            if let Some(folder_path) = dialog.file().and_then(|file| file.path()) {
                path_entry.set_text(&folder_path.to_string_lossy());
                let _result = save_library_path_from_entry(&runtime, &path_entry);
            }
        }
        dialog.destroy();
    });
}

fn save_library_path_from_entry(
    runtime: &SharedRuntime,
    path_entry: &gtk::Entry,
) -> Result<(), ApplicationRuntimeError> {
    let path_text = path_entry.text().trim().to_owned();
    let library_path = if path_text.is_empty() {
        None
    } else {
        Some(PathBuf::from(path_text))
    };
    let settings = UserSettings { library_path };

    runtime
        .borrow_mut()
        .handle_command(ApplicationCommand::UpdateSettings(settings))
}

fn request_library_scan_from_entry(
    runtime: &SharedRuntime,
    path_entry: &gtk::Entry,
) -> Result<Option<LibraryScanSummary>, ApplicationRuntimeError> {
    let path_text = path_entry.text().trim().to_owned();
    if path_text.is_empty() {
        return Ok(None);
    }

    let mut runtime = runtime.borrow_mut();
    runtime.handle_command(ApplicationCommand::ScanLibrary {
        library_path: PathBuf::from(path_text),
    })?;

    Ok(runtime.last_scan_summary().cloned())
}

fn scan_summary_text(summary: &LibraryScanSummary) -> String {
    format!(
        "Scanned {} tracks. Skipped {} unsupported files. {} files failed.",
        summary.scanned_tracks, summary.skipped_unsupported_files, summary.failed_files
    )
}

fn scan_error_text(error: ApplicationRuntimeError) -> &'static str {
    match error {
        ApplicationRuntimeError::LibraryScanFailed => "The selected folder could not be scanned.",
        ApplicationRuntimeError::LibraryServicesUnavailable => {
            "Library scanning is not available in this build."
        }
        ApplicationRuntimeError::LibraryStoreFailed => "The library database could not be updated.",
        ApplicationRuntimeError::PlaybackFailed
        | ApplicationRuntimeError::PlaybackServiceUnavailable
        | ApplicationRuntimeError::TrackUnavailable => "Playback is not available.",
        ApplicationRuntimeError::SettingsLoadFailed
        | ApplicationRuntimeError::SettingsSaveFailed => "The library path could not be saved.",
    }
}
