use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::mpsc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use glib::ControlFlow;

use super::{
    ApplicationCommand, ApplicationRuntimeError, LibraryChangedCallback, LibraryScanSummary,
    PREFERENCES_HEIGHT, PREFERENCES_WIDTH, SharedRuntime, UserSettings, WINDOW_SHADOW_MARGIN,
};

/// Spawn a library scan on a background thread. Uses `mpsc::channel` to send incremental
/// track discoveries and a final completion message back, then polls them on the main loop
/// via `idle_add_local` — which does not require `Send`, so GTK widgets and `Rc<RefCell<T>>`
/// can be captured safely.
///
/// `append_row` is called for each discovered track during scanning (append-only, O(1)).
/// `refresh_status` updates the status bar summary once at completion without table rebuild.
fn spawn_scan_on_thread(
    params: xtunes_app_runtime::ScanParameters,
    status_label: gtk::Label,
    button: gtk::Button,
    runtime: SharedRuntime,
    refresh_status: LibraryChangedCallback,
    append_row: super::AppendRowCallback,
) {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        xtunes_app_runtime::run_incremental_scan(params, tx);
    });

    // Poll the channel on the main loop. `idle_add_local` does not require `Send`,
    // so we can capture GTK widgets and Rc<RefCell<T>> directly.
    let rx_cell = RefCell::new(Some(rx));
    glib::idle_add_local(move || {
        let mut rx_opt = rx_cell.borrow_mut();
        let Some(rx) = rx_opt.as_ref() else {
            return ControlFlow::Break;
        };

        // Drain all available messages this tick for responsive UI
        loop {
            match rx.try_recv() {
                Ok(xtunes_app_runtime::ScanMessage::TrackFound(track)) => {
                    runtime.borrow_mut().apply_scanned_track(track.clone());
                    append_row(&track);
                }
                Ok(xtunes_app_runtime::ScanMessage::Done(Ok(summary))) => {
                    *rx_opt = None;
                    drop(rx_opt);
                    runtime.borrow_mut().finalize_scan(summary.clone());
                    drop(runtime.borrow_mut());
                    refresh_status();
                    status_label.set_text(&scan_summary_text(&summary));
                    button.set_sensitive(true);
                    return ControlFlow::Break;
                }
                Ok(xtunes_app_runtime::ScanMessage::Done(Err(error))) => {
                    *rx_opt = None;
                    drop(rx_opt);
                    status_label.set_text(scan_error_text(error));
                    button.set_sensitive(true);
                    return ControlFlow::Break;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    break;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    *rx_opt = None;
                    drop(rx_opt);
                    status_label.set_text("Scan was cancelled.");
                    button.set_sensitive(true);
                    return ControlFlow::Break;
                }
            }
        }

        // Update progress label only — rows appended individually, no table rebuild needed
        let track_count = runtime.borrow().library_tracks().len();
        status_label.set_text(&format!("Scanning… {} tracks found", track_count));
        ControlFlow::Continue
    });
}

pub(crate) fn install_preferences_action(
    app: &gtk::Application,
    window: &gtk::ApplicationWindow,
    runtime: SharedRuntime,
    append_row: super::AppendRowCallback,
    refresh_status: LibraryChangedCallback,
) {
    if app.lookup_action("preferences").is_some() {
        return;
    }

    let preferences = gio::SimpleAction::new("preferences", None);
    let window = window.clone();
    let runtime = runtime.clone();
    let append_row = append_row.clone();
    let refresh_status = refresh_status.clone();
    preferences.connect_activate(move |_action, _parameter| {
        open_preferences_window(
            &window,
            runtime.clone(),
            append_row.clone(),
            refresh_status.clone(),
        );
    });
    app.add_action(&preferences);
    app.set_accels_for_action("app.preferences", &["<Primary>comma"]);
}

pub(crate) fn settings_button(
    window: &gtk::ApplicationWindow,
    runtime: SharedRuntime,
    append_row: super::AppendRowCallback,
    refresh_status: LibraryChangedCallback,
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
    let append_row = append_row.clone();
    let refresh_status = refresh_status.clone();
    button.connect_clicked(move |_| {
        open_preferences_window(
            &window,
            runtime.clone(),
            append_row.clone(),
            refresh_status.clone(),
        );
    });

    button
}

fn open_preferences_window(
    parent: &gtk::ApplicationWindow,
    runtime: SharedRuntime,
    append_row: super::AppendRowCallback,
    refresh_status: LibraryChangedCallback,
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
    let refresh_status_for_scan = refresh_status.clone();
    let append_row_for_scan = append_row.clone();
    let scan_button_for_scan = scan_button.clone();
    scan_button.connect_clicked(move |_| {
        scan_button_for_scan.set_sensitive(false);
        scan_status_for_scan.set_text("Scanning...");

        let path_text = path_entry_for_scan.text().trim().to_owned();
        if path_text.is_empty() {
            scan_status_for_scan.set_text("Choose a library folder before scanning.");
            scan_button_for_scan.set_sensitive(true);
            return;
        }

        let save_result = save_library_path_from_entry(&runtime_for_scan, &path_entry_for_scan);
        if save_result.is_err() {
            scan_status_for_scan.set_text(scan_error_text(
                save_result.unwrap_err(),
            ));
            scan_button_for_scan.set_sensitive(true);
            return;
        }

        let library_path = PathBuf::from(path_text);
        let scan_params = runtime_for_scan.borrow().take_scan_parameters(library_path);

        match scan_params {
            Ok(params) => {
                spawn_scan_on_thread(
                    params,
                    scan_status_for_scan.clone(),
                    scan_button_for_scan.clone(),
                    runtime_for_scan.clone(),
                    refresh_status_for_scan.clone(),
                    append_row_for_scan.clone(),
                );
            }
            Err(error) => {
                scan_status_for_scan.set_text(scan_error_text(error));
                scan_button_for_scan.set_sensitive(true);
            }
        }
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
