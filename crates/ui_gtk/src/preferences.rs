// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use super::{
    ApplicationCommand, ApplicationRuntimeError, PREFERENCES_HEIGHT, PREFERENCES_WIDTH,
    UserSettings, WINDOW_SHADOW_MARGIN, command_controller::SharedCommandController,
    library_scan::LibraryScanRequestedCallback,
};

pub(crate) fn install_preferences_action(
    app: &gtk::Application,
    window: &gtk::ApplicationWindow,
    command_controller: SharedCommandController,
    scan_requested: LibraryScanRequestedCallback,
) {
    if app.lookup_action("preferences").is_some() {
        return;
    }

    let preferences = gio::SimpleAction::new("preferences", None);
    let window = window.clone();
    let command_controller = command_controller.clone();
    let scan_requested = scan_requested.clone();
    preferences.connect_activate(move |_action, _parameter| {
        open_preferences_window(&window, command_controller.clone(), scan_requested.clone());
    });
    app.add_action(&preferences);
    app.set_accels_for_action("app.preferences", &["<Primary>comma"]);
}

pub(crate) fn settings_button(
    window: &gtk::ApplicationWindow,
    command_controller: SharedCommandController,
    scan_requested: LibraryScanRequestedCallback,
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
    let command_controller = command_controller.clone();
    let scan_requested = scan_requested.clone();
    button.connect_clicked(move |_| {
        open_preferences_window(&window, command_controller.clone(), scan_requested.clone());
    });

    button
}

fn open_preferences_window(
    parent: &gtk::ApplicationWindow,
    command_controller: SharedCommandController,
    scan_requested: LibraryScanRequestedCallback,
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
    if let Some(library_path) = command_controller
        .runtime()
        .borrow()
        .settings()
        .library_path()
    {
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

    let command_controller_for_entry = command_controller.clone();
    path_entry.connect_activate(move |entry| {
        let _result = save_library_path_from_entry(&command_controller_for_entry, entry);
    });

    let window_for_folder = window.clone();
    let command_controller_for_folder = command_controller.clone();
    let path_entry_for_folder = path_entry.clone();
    folder_button.connect_clicked(move |_| {
        open_library_folder_chooser(
            &window_for_folder,
            &command_controller_for_folder,
            &path_entry_for_folder,
        );
    });

    let command_controller_for_scan = command_controller.clone();
    let path_entry_for_scan = path_entry.clone();
    let scan_status_for_scan = scan_status.clone();
    let scan_requested_for_scan = scan_requested.clone();
    scan_button.connect_clicked(move |_| {
        if path_entry_for_scan.text().trim().is_empty() {
            scan_status_for_scan.set_text("Choose a library folder before scanning.");
            return;
        }

        let scan_message = match save_library_path_from_entry(
            &command_controller_for_scan,
            &path_entry_for_scan,
        ) {
            Ok(()) => match request_library_scan_from_entry(
                &path_entry_for_scan,
                &scan_requested_for_scan,
            ) {
                Ok(()) => "Scan started. Progress is shown in the status bar.".to_owned(),
                Err(error) => scan_error_text(error).to_owned(),
            },
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
    command_controller: &SharedCommandController,
    path_entry: &gtk::Entry,
) {
    let dialog = gtk::FileDialog::builder()
        .title("Choose Library Folder")
        .accept_label("Choose")
        .modal(true)
        .build();

    let current_path = path_entry.text().trim().to_owned();
    if !current_path.is_empty() {
        let current_folder = gio::File::for_path(current_path);
        dialog.set_initial_folder(Some(&current_folder));
    }

    let command_controller = command_controller.clone();
    let path_entry = path_entry.clone();
    dialog.select_folder(Some(parent), None::<&gio::Cancellable>, move |result| {
        let Ok(file) = result else {
            return;
        };
        let Some(folder_path) = file.path() else {
            return;
        };
        path_entry.set_text(&folder_path.to_string_lossy());
        let _result = save_library_path_from_entry(&command_controller, &path_entry);
    });
}

fn save_library_path_from_entry(
    command_controller: &SharedCommandController,
    path_entry: &gtk::Entry,
) -> Result<(), ApplicationRuntimeError> {
    let path_text = path_entry.text().trim().to_owned();
    let library_path = if path_text.is_empty() {
        None
    } else {
        Some(PathBuf::from(path_text))
    };
    let settings = UserSettings::with_library_path(library_path);

    command_controller.dispatch(ApplicationCommand::UpdateSettings(settings))
}

fn request_library_scan_from_entry(
    path_entry: &gtk::Entry,
    scan_requested: &LibraryScanRequestedCallback,
) -> Result<(), ApplicationRuntimeError> {
    let path_text = path_entry.text().trim().to_owned();
    if path_text.is_empty() {
        return Err(ApplicationRuntimeError::LibraryScanFailed);
    }

    scan_requested(PathBuf::from(path_text))
}

fn scan_error_text(error: ApplicationRuntimeError) -> &'static str {
    match error {
        ApplicationRuntimeError::LibraryScanFailed => "The selected folder could not be scanned.",
        ApplicationRuntimeError::LibraryServicesUnavailable => {
            "Library scanning is not available in this build."
        }
        ApplicationRuntimeError::LibraryStoreFailed => "The library database could not be updated.",
        ApplicationRuntimeError::MetadataWriteFailed => "The track metadata could not be updated.",
        ApplicationRuntimeError::InvalidPlaylistName => "The playlist name is not valid.",
        ApplicationRuntimeError::PlaylistEntryNotFound
        | ApplicationRuntimeError::PlaylistNotFound => "The playlist could not be updated.",
        ApplicationRuntimeError::BackgroundTaskRunning => "A library scan is already running.",
        ApplicationRuntimeError::PlaybackFailed
        | ApplicationRuntimeError::PlaybackServiceUnavailable
        | ApplicationRuntimeError::TrackUnavailable => "Playback is not available.",
        ApplicationRuntimeError::SettingsLoadFailed
        | ApplicationRuntimeError::SettingsSaveFailed => "The library path could not be saved.",
        ApplicationRuntimeError::TrackTrashFailed => "The track could not be moved to trash.",
        ApplicationRuntimeError::UnsupportedCommand(_) => "This action is not available yet.",
    }
}
