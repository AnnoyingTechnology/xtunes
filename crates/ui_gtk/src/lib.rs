#![forbid(unsafe_code)]

use gtk::gdk::prelude::ToplevelExt;
use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango};
use track_table::{TrackTableRow, build_track_table, mock_library_tracks, mock_playlist_tracks};

pub use xtunes_app_runtime::{ApplicationCommand, ApplicationQuery, ApplicationRuntime};

mod track_table;

const TITLEBAR_HEIGHT: i32 = 72;
const TITLEBAR_LEFT_PADDING: i32 = 48;
const TITLEBAR_RIGHT_PADDING: i32 = 0;
const TITLEBAR_CONTROL_HEIGHT: i32 = 42;
const MEDIA_ICON_SIZE: i32 = 32;
const MODE_BAR_HEIGHT: i32 = 34;
const MODE_BUTTON_HEIGHT: i32 = 22;
const NOW_PLAYING_HORIZONTAL_MARGIN: i32 = TITLEBAR_HEIGHT / 2;
const NOW_PLAYING_ICON_SIZE: i32 = 16;
const NOW_PLAYING_SIDE_WIDTH: i32 = 58;
const NOW_PLAYING_WIDTH: i32 = 600;
const PREFERENCES_HEIGHT: i32 = 190;
const PREFERENCES_WIDTH: i32 = 520;
const RESIZE_CORNER_SIZE: i32 = 18;
const RESIZE_EDGE_THICKNESS: i32 = 6;
const SIDEBAR_DEFAULT_WIDTH: i32 = 220;
const SIDEBAR_MIN_WIDTH: i32 = 150;
const SIDEBAR_MAX_WIDTH: i32 = 300;
const STATUS_BAR_HEIGHT: i32 = 28;
const VOLUME_WIDTH: i32 = 192;
const WINDOW_SHADOW_MARGIN: i32 = 14;
const SONGS_VIEW: &str = "songs";
const ALBUMS_VIEW: &str = "albums";
const PLAYLISTS_VIEW: &str = "playlists";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MainViewMode {
    Songs,
    Albums,
    Playlists,
}

pub fn run(_runtime: ApplicationRuntime) {
    let app = gtk::Application::builder()
        .application_id("io.github.AnnoyingTechnology.xtunes")
        .build();

    app.connect_activate(move |app| {
        let window = build_main_window(app);
        window.present();
    });

    app.run();
}

fn build_main_window(app: &gtk::Application) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .decorated(false)
        .title("xTunes")
        .default_width(1100)
        .default_height(720)
        .build();
    window.add_css_class("app-window");
    window.set_resizable(true);
    install_app_css();
    install_preferences_action(app, &window);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("app-shell");
    root.set_hexpand(true);
    root.set_vexpand(true);
    root.set_overflow(gtk::Overflow::Hidden);

    let sidebar = build_sidebar();
    sidebar.set_visible(false);

    let library_tracks = mock_library_tracks();
    let content_stack = build_content_stack(&library_tracks);

    let main_content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main_content.set_hexpand(true);
    main_content.set_vexpand(true);
    main_content.append(&build_mode_bar(&window, &sidebar, &content_stack));
    main_content.append(&content_stack);

    let content_area = build_content_area(&sidebar, &main_content);

    root.append(&build_titlebar());
    root.append(&content_area);
    root.append(&build_status_bar(&library_tracks));

    let window_frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    window_frame.add_css_class("csd");
    window_frame.set_hexpand(true);
    window_frame.set_vexpand(true);
    window_frame.append(&root);
    install_window_state_chrome(&window, &window_frame);

    let shell = gtk::Overlay::new();
    shell.set_child(Some(&window_frame));
    install_resize_handles(&shell, &window);
    window.set_child(Some(&shell));

    window
}

fn install_window_state_chrome(window: &gtk::ApplicationWindow, window_frame: &gtk::Box) {
    update_window_state_chrome(window, window_frame);

    let window_frame_for_fullscreen = window_frame.clone();
    window.connect_fullscreened_notify(move |window| {
        update_window_state_chrome(window, &window_frame_for_fullscreen);
    });

    let window_frame_for_maximize = window_frame.clone();
    window.connect_maximized_notify(move |window| {
        update_window_state_chrome(window, &window_frame_for_maximize);
    });
}

fn update_window_state_chrome(window: &gtk::ApplicationWindow, window_frame: &gtk::Box) {
    let is_floating = !window.is_fullscreen() && !window.is_maximized();
    let margin = if is_floating {
        window_frame.add_css_class("window-frame");
        WINDOW_SHADOW_MARGIN
    } else {
        window_frame.remove_css_class("window-frame");
        0
    };

    window_frame.set_margin_top(margin);
    window_frame.set_margin_end(margin);
    window_frame.set_margin_bottom(margin);
    window_frame.set_margin_start(margin);
}

fn install_preferences_action(app: &gtk::Application, window: &gtk::ApplicationWindow) {
    if app.lookup_action("preferences").is_some() {
        return;
    }

    let preferences = gio::SimpleAction::new("preferences", None);
    let window = window.clone();
    preferences.connect_activate(move |_action, _parameter| {
        open_preferences_window(&window);
    });
    app.add_action(&preferences);
    app.set_accels_for_action("app.preferences", &["<Primary>comma"]);
}

fn open_preferences_window(parent: &gtk::ApplicationWindow) {
    let window = gtk::Window::builder()
        .title("Library Path")
        .decorated(false)
        .transient_for(parent)
        .modal(false)
        .default_width(PREFERENCES_WIDTH + WINDOW_SHADOW_MARGIN * 2)
        .default_height(PREFERENCES_HEIGHT + WINDOW_SHADOW_MARGIN * 2)
        .resizable(false)
        .build();
    window.add_css_class("app-window");

    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("preferences-frame");
    frame.set_margin_top(WINDOW_SHADOW_MARGIN);
    frame.set_margin_end(WINDOW_SHADOW_MARGIN);
    frame.set_margin_bottom(WINDOW_SHADOW_MARGIN);
    frame.set_margin_start(WINDOW_SHADOW_MARGIN);
    frame.set_size_request(PREFERENCES_WIDTH, PREFERENCES_HEIGHT);

    let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    panel.add_css_class("preferences-panel");
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

    let library_path_help = gtk::Label::new(Some(
        "Files in this folder are automatically added to your library.",
    ));
    library_path_help.add_css_class("preference-helper");
    library_path_help.set_xalign(0.0);
    library_path_help.set_wrap(true);

    label_group.append(&library_path_label);
    label_group.append(&library_path_help);

    let path_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);

    let path_entry = gtk::Entry::new();
    path_entry.set_hexpand(true);
    path_entry.set_placeholder_text(Some("/home/julien/Music"));

    let folder_icon = gtk::Image::from_icon_name("folder-open-symbolic");
    let folder_button = gtk::Button::new();
    folder_button.add_css_class("flat");
    folder_button.add_css_class("settings-button");
    folder_button.set_child(Some(&folder_icon));
    folder_button.set_tooltip_text(Some("Choose folder"));

    path_row.append(&path_entry);
    path_row.append(&folder_button);

    content.append(&label_group);
    content.append(&path_row);
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

fn build_content_area(sidebar: &gtk::Box, main_content: &gtk::Box) -> gtk::Paned {
    let content_area = gtk::Paned::new(gtk::Orientation::Horizontal);
    content_area.set_hexpand(true);
    content_area.set_vexpand(true);
    content_area.set_wide_handle(false);
    content_area.set_resize_start_child(false);
    content_area.set_shrink_start_child(false);
    content_area.set_resize_end_child(true);
    content_area.set_shrink_end_child(false);
    content_area.set_start_child(Some(sidebar));
    content_area.set_end_child(Some(main_content));
    content_area.set_position(SIDEBAR_DEFAULT_WIDTH);
    content_area.connect_position_notify(clamp_sidebar_width);
    content_area
}

fn clamp_sidebar_width(content_area: &gtk::Paned) {
    let current_width = content_area.position();
    let clamped_width = current_width.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
    if clamped_width != current_width {
        content_area.set_position(clamped_width);
    }
}

fn install_app_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(
        r#"
        .app-window {
            background-color: transparent;
        }

        .window-frame {
            border-radius: 10px;
            box-shadow:
                0 14px 36px 0 alpha(black, 0.30),
                0 2px 10px 0 alpha(black, 0.22);
        }

        .preferences-frame {
            border-radius: 10px;
            box-shadow:
                0 14px 36px 0 alpha(black, 0.30),
                0 2px 10px 0 alpha(black, 0.22);
        }

        .app-shell {
            background-color: @theme_bg_color;
        }

        .window-frame .app-shell {
            border-radius: 10px;
        }

        .preferences-panel {
            background-color: @theme_bg_color;
            border-radius: 10px;
        }

        .media-control {
            opacity: 0.90;
        }

        .now-playing-area {
            background-color: alpha(@theme_fg_color, 0.035);
            min-height: 72px;
            min-width: 600px;
        }

        .now-playing-artwork {
            background-color: alpha(@theme_fg_color, 0.12);
            min-height: 72px;
            min-width: 72px;
        }

        .now-playing-title {
            font-weight: bold;
        }

        .now-playing-artist,
        .now-playing-time {
            color: alpha(@theme_fg_color, 0.58);
        }

        .now-playing-time {
            font-size: 0.85em;
        }

        .now-playing-side-icon {
            opacity: 0.64;
        }

        .song-progress trough {
            background-color: alpha(@theme_fg_color, 0.18);
            border: none;
            border-radius: 0;
            min-height: 6px;
            min-width: 6px;
        }

        .song-progress progress {
            background-color: alpha(@theme_fg_color, 0.48);
            border: none;
            border-radius: 0;
            min-height: 6px;
            min-width: 6px;
        }

        .volume-slider trough {
            background-color: alpha(@theme_fg_color, 0.22);
            border: none;
            border-radius: 999px;
            min-height: 6px;
        }

        .volume-slider highlight {
            background-color: alpha(@theme_fg_color, 0.55);
            border-radius: 999px;
            min-height: 6px;
        }

        .volume-slider slider {
            background-color: @theme_bg_color;
            border: none;
            box-shadow: none;
        }

        .settings-button {
            background: transparent;
            border: none;
            box-shadow: none;
            margin-right: 8px;
            min-height: 28px;
            min-width: 28px;
            opacity: 0.72;
            padding: 0;
        }

        .settings-button:hover,
        .settings-button:active,
        .settings-button:focus {
            background: transparent;
            border: none;
            box-shadow: none;
            opacity: 0.90;
        }

        .preference-helper {
            color: alpha(@theme_fg_color, 0.58);
            font-size: 0.9em;
        }

        .preference-close-button {
            background: transparent;
            border: none;
            box-shadow: none;
            min-height: 24px;
            min-width: 24px;
            opacity: 0.62;
            padding: 0;
        }

        .preference-close-button:hover,
        .preference-close-button:active,
        .preference-close-button:focus {
            background: transparent;
            border: none;
            box-shadow: none;
            opacity: 0.90;
        }

        .mode-button {
            min-height: 22px;
            border-radius: 999px;
            padding-top: 0;
            padding-bottom: 0;
        }

        .mode-button label {
            padding-top: 0;
            padding-bottom: 0;
        }

        .mode-bar,
        .status-bar {
            background-color: alpha(@theme_fg_color, 0.04);
        }

        .mode-bar {
            border-bottom: 1px solid alpha(@theme_fg_color, 0.08);
        }

        .topbar-search,
        .topbar-search entry,
        .topbar-search text {
            border-radius: 999px;
        }

        .status-bar {
            border-top: 1px solid alpha(@theme_fg_color, 0.08);
        }

        .playlist-sidebar {
            background-color: mix(@theme_bg_color, black, 0.10);
            border-right: 1px solid alpha(@theme_fg_color, 0.12);
        }

        .track-table header {
            background-color: alpha(@theme_fg_color, 0.08);
        }

        .track-table header button {
            background: transparent;
            border: none;
            border-right: 1px solid alpha(@theme_fg_color, 0.12);
            border-radius: 0;
            box-shadow: none;
            padding-top: 4px;
            padding-bottom: 4px;
        }

        .track-table header button:hover {
            background-color: alpha(@theme_fg_color, 0.05);
        }

        .track-table listview row,
        .track-table listview row cell {
            border: none;
            margin: 0;
            padding: 0;
        }

        .track-table-cell {
            margin: 0;
            min-height: 28px;
            padding: 0;
        }

        .track-table-row-even {
            background-color: alpha(@theme_fg_color, 0.025);
        }

        .track-table-row-odd {
            background-color: transparent;
        }

        columnview.track-table listview row:not(:selected),
        columnview.track-table listview row:not(:selected):hover,
        columnview.track-table listview row:not(:selected):active,
        columnview.track-table listview row:not(:selected):focus {
            background-color: transparent;
            background-image: none;
            color: @theme_fg_color;
        }

        columnview.track-table listview row:not(:selected) cell,
        columnview.track-table listview row:not(:selected):hover cell,
        columnview.track-table listview row:not(:selected):active cell,
        columnview.track-table listview row:not(:selected):focus cell {
            background-color: transparent;
            background-image: none;
            color: @theme_fg_color;
        }

        columnview.track-table listview row:not(:selected):hover .track-table-row-even,
        columnview.track-table listview row:not(:selected):active .track-table-row-even,
        columnview.track-table listview row:not(:selected):focus .track-table-row-even {
            background-color: alpha(@theme_fg_color, 0.025);
        }

        columnview.track-table listview row:not(:selected):hover .track-table-row-odd,
        columnview.track-table listview row:not(:selected):active .track-table-row-odd,
        columnview.track-table listview row:not(:selected):focus .track-table-row-odd {
            background-color: transparent;
        }

        .track-table listview row:selected,
        .track-table listview row:selected .track-table-cell {
            background-color: @theme_selected_bg_color;
            color: @theme_selected_fg_color;
        }

        .rating-stars {
            min-height: 28px;
        }

        button.rating-star {
            background: transparent;
            border: none;
            border-radius: 4px;
            box-shadow: none;
            color: alpha(@theme_fg_color, 0.70);
            margin: 0;
            min-height: 20px;
            min-width: 16px;
            padding: 0;
        }

        button.rating-star:hover,
        button.rating-star:active,
        button.rating-star:focus {
            background: transparent;
            border: none;
            box-shadow: none;
        }

        button.rating-star label {
            margin: 0;
            padding: 0;
        }

        button.rating-star-filled {
            color: alpha(@theme_fg_color, 0.86);
        }

        button.rating-star-empty {
            color: alpha(@theme_fg_color, 0.35);
        }
        "#,
    );

    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn install_resize_handles(shell: &gtk::Overlay, window: &gtk::ApplicationWindow) {
    for (edge, halign, valign, width, height, cursor) in [
        (
            gdk::SurfaceEdge::North,
            gtk::Align::Fill,
            gtk::Align::Start,
            -1,
            RESIZE_EDGE_THICKNESS,
            "n-resize",
        ),
        (
            gdk::SurfaceEdge::East,
            gtk::Align::End,
            gtk::Align::Fill,
            RESIZE_EDGE_THICKNESS,
            -1,
            "e-resize",
        ),
        (
            gdk::SurfaceEdge::South,
            gtk::Align::Fill,
            gtk::Align::End,
            -1,
            RESIZE_EDGE_THICKNESS,
            "s-resize",
        ),
        (
            gdk::SurfaceEdge::West,
            gtk::Align::Start,
            gtk::Align::Fill,
            RESIZE_EDGE_THICKNESS,
            -1,
            "w-resize",
        ),
        (
            gdk::SurfaceEdge::NorthWest,
            gtk::Align::Start,
            gtk::Align::Start,
            RESIZE_CORNER_SIZE,
            RESIZE_CORNER_SIZE,
            "nw-resize",
        ),
        (
            gdk::SurfaceEdge::NorthEast,
            gtk::Align::End,
            gtk::Align::Start,
            RESIZE_CORNER_SIZE,
            RESIZE_CORNER_SIZE,
            "ne-resize",
        ),
        (
            gdk::SurfaceEdge::SouthEast,
            gtk::Align::End,
            gtk::Align::End,
            RESIZE_CORNER_SIZE,
            RESIZE_CORNER_SIZE,
            "se-resize",
        ),
        (
            gdk::SurfaceEdge::SouthWest,
            gtk::Align::Start,
            gtk::Align::End,
            RESIZE_CORNER_SIZE,
            RESIZE_CORNER_SIZE,
            "sw-resize",
        ),
    ] {
        let handle = resize_handle(edge, window, cursor);
        handle.set_halign(halign);
        handle.set_valign(valign);
        handle.set_size_request(width, height);
        shell.add_overlay(&handle);
        shell.set_measure_overlay(&handle, false);
    }
}

fn resize_handle(
    edge: gdk::SurfaceEdge,
    window: &gtk::ApplicationWindow,
    cursor_name: &str,
) -> gtk::Box {
    let handle = gtk::Box::new(gtk::Orientation::Vertical, 0);
    handle.set_cursor_from_name(Some(cursor_name));

    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    let window = window.clone();
    let handle_widget = handle.clone();
    click.connect_pressed(move |click, _press_count, x, y| {
        let Some(surface) = window.surface() else {
            return;
        };
        let Ok(toplevel) = surface.downcast::<gdk::Toplevel>() else {
            return;
        };
        let Some(device) = click.current_event_device() else {
            return;
        };
        let (surface_x, surface_y) = handle_widget
            .translate_coordinates(&window, x, y)
            .unwrap_or((x, y));

        toplevel.begin_resize(
            edge,
            Some(&device),
            click.current_button() as i32,
            surface_x,
            surface_y,
            click.current_event_time(),
        );
    });
    handle.add_controller(click);

    handle
}

fn build_titlebar() -> gtk::WindowHandle {
    let topbar = gtk::CenterBox::new();
    topbar.add_css_class("titlebar");
    topbar.set_hexpand(true);
    topbar.set_height_request(TITLEBAR_HEIGHT);

    let previous = media_icon_button("media-skip-backward-symbolic", "Previous");
    let play_pause = media_icon_button("media-playback-start-symbolic", "Play");
    let next = media_icon_button("media-skip-forward-symbolic", "Next");
    set_titlebar_control_height(&previous);
    set_titlebar_control_height(&play_pause);
    set_titlebar_control_height(&next);

    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
    volume.add_css_class("volume-slider");
    volume.set_value(0.8);
    volume.set_width_request(VOLUME_WIDTH);
    volume.set_height_request(TITLEBAR_CONTROL_HEIGHT);
    volume.set_draw_value(false);
    volume.set_tooltip_text(Some("Volume"));

    let search = gtk::SearchEntry::new();
    search.add_css_class("topbar-search");
    search.set_placeholder_text(Some("Search"));
    search.set_width_chars(24);
    search.set_valign(gtk::Align::Center);

    let window_controls = gtk::WindowControls::new(gtk::PackType::End);
    window_controls.set_valign(gtk::Align::Center);

    let left_controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    left_controls.set_valign(gtk::Align::Center);
    left_controls.append(&horizontal_spacer(TITLEBAR_LEFT_PADDING));
    left_controls.append(&previous);
    left_controls.append(&play_pause);
    left_controls.append(&next);
    left_controls.append(&volume);

    let right_controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    right_controls.set_valign(gtk::Align::Center);
    right_controls.append(&search);
    right_controls.append(&horizontal_spacer(TITLEBAR_RIGHT_PADDING));
    right_controls.append(&window_controls);

    topbar.set_start_widget(Some(&left_controls));
    topbar.set_center_widget(Some(&build_now_playing_area()));
    topbar.set_end_widget(Some(&right_controls));

    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&topbar));
    handle
}

fn build_now_playing_area() -> gtk::Box {
    let area = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    area.add_css_class("now-playing-area");
    area.set_width_request(NOW_PLAYING_WIDTH);
    area.set_height_request(TITLEBAR_HEIGHT);
    area.set_hexpand(false);
    area.set_halign(gtk::Align::Center);
    area.set_margin_start(NOW_PLAYING_HORIZONTAL_MARGIN);
    area.set_margin_end(NOW_PLAYING_HORIZONTAL_MARGIN);
    area.set_valign(gtk::Align::Fill);

    let artwork = gtk::Box::new(gtk::Orientation::Vertical, 0);
    artwork.add_css_class("now-playing-artwork");
    artwork.set_size_request(TITLEBAR_HEIGHT, TITLEBAR_HEIGHT);

    let details = gtk::Box::new(gtk::Orientation::Vertical, 0);
    details.set_hexpand(true);
    details.set_vexpand(true);

    let detail_content = gtk::CenterBox::new();
    detail_content.set_hexpand(true);
    detail_content.set_vexpand(true);
    detail_content.set_valign(gtk::Align::Fill);
    detail_content.set_start_widget(Some(&now_playing_side_status(
        "media-playlist-shuffle-symbolic",
        "Shuffle",
        "1:24",
    )));
    detail_content.set_center_widget(Some(&now_playing_metadata()));
    detail_content.set_end_widget(Some(&now_playing_side_status(
        "media-playlist-repeat-symbolic",
        "Repeat",
        "-2:40",
    )));

    let progress = gtk::ProgressBar::new();
    progress.add_css_class("song-progress");
    progress.set_fraction(0.35);
    progress.set_hexpand(true);
    progress.set_halign(gtk::Align::Fill);
    progress.set_valign(gtk::Align::End);

    details.append(&detail_content);
    details.append(&progress);

    area.append(&artwork);
    area.append(&details);
    area
}

fn now_playing_metadata() -> gtk::Box {
    let metadata = gtk::Box::new(gtk::Orientation::Vertical, 2);
    metadata.set_halign(gtk::Align::Center);
    metadata.set_valign(gtk::Align::Center);
    metadata.set_hexpand(true);

    let title = gtk::Label::new(Some("Midnight City"));
    title.add_css_class("now-playing-title");
    title.set_ellipsize(pango::EllipsizeMode::End);
    title.set_max_width_chars(32);
    title.set_xalign(0.5);

    let artist = gtk::Label::new(Some("M83"));
    artist.add_css_class("now-playing-artist");
    artist.set_ellipsize(pango::EllipsizeMode::End);
    artist.set_max_width_chars(36);
    artist.set_xalign(0.5);

    metadata.append(&title);
    metadata.append(&artist);
    metadata
}

fn now_playing_side_status(icon_name: &str, tooltip: &str, time_text: &str) -> gtk::Box {
    let status = gtk::Box::new(gtk::Orientation::Vertical, 2);
    status.set_width_request(NOW_PLAYING_SIDE_WIDTH);
    status.set_halign(gtk::Align::Center);
    status.set_valign(gtk::Align::Center);

    let icon = gtk::Image::from_icon_name(icon_name);
    icon.add_css_class("now-playing-side-icon");
    icon.set_pixel_size(NOW_PLAYING_ICON_SIZE);
    icon.set_tooltip_text(Some(tooltip));
    icon.set_halign(gtk::Align::Center);

    let time = gtk::Label::new(Some(time_text));
    time.add_css_class("now-playing-time");
    time.set_halign(gtk::Align::Center);
    time.set_xalign(0.5);

    status.append(&icon);
    status.append(&time);
    status
}

fn build_mode_bar(
    window: &gtk::ApplicationWindow,
    sidebar: &gtk::Box,
    content_stack: &gtk::Stack,
) -> gtk::CenterBox {
    let mode_bar = gtk::CenterBox::new();
    mode_bar.add_css_class("mode-bar");
    mode_bar.set_height_request(MODE_BAR_HEIGHT);
    mode_bar.set_hexpand(true);

    let songs = gtk::ToggleButton::with_label("Songs");
    let albums = gtk::ToggleButton::with_label("Albums");
    let playlists = gtk::ToggleButton::with_label("Playlists");
    set_mode_button_height(&songs);
    set_mode_button_height(&albums);
    set_mode_button_height(&playlists);
    albums.set_group(Some(&songs));
    playlists.set_group(Some(&songs));
    songs.set_active(true);

    connect_mode_button(&songs, MainViewMode::Songs, sidebar, content_stack);
    connect_mode_button(&albums, MainViewMode::Albums, sidebar, content_stack);
    connect_mode_button(&playlists, MainViewMode::Playlists, sidebar, content_stack);

    let mode_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    mode_buttons.set_valign(gtk::Align::Center);
    mode_buttons.append(&songs);
    mode_buttons.append(&albums);
    mode_buttons.append(&playlists);
    mode_bar.set_center_widget(Some(&mode_buttons));

    let settings = settings_button(window);
    mode_bar.set_end_widget(Some(&settings));
    mode_bar
}

fn settings_button(window: &gtk::ApplicationWindow) -> gtk::Button {
    let icon = gtk::Image::from_icon_name("preferences-system-symbolic");
    icon.set_pixel_size(18);

    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("settings-button");
    button.set_child(Some(&icon));
    button.set_tooltip_text(Some("Preferences"));
    button.set_valign(gtk::Align::Center);

    let window = window.clone();
    button.connect_clicked(move |_| {
        open_preferences_window(&window);
    });

    button
}

fn connect_mode_button(
    button: &gtk::ToggleButton,
    mode: MainViewMode,
    sidebar: &gtk::Box,
    content_stack: &gtk::Stack,
) {
    let sidebar = sidebar.clone();
    let content_stack = content_stack.clone();
    button.connect_toggled(move |button| {
        if button.is_active() {
            apply_view_mode(mode, &sidebar, &content_stack);
        }
    });
}

fn apply_view_mode(mode: MainViewMode, sidebar: &gtk::Box, content_stack: &gtk::Stack) {
    match mode {
        MainViewMode::Songs => {
            sidebar.set_visible(false);
            content_stack.set_visible_child_name(SONGS_VIEW);
        }
        MainViewMode::Albums => {
            sidebar.set_visible(false);
            content_stack.set_visible_child_name(ALBUMS_VIEW);
        }
        MainViewMode::Playlists => {
            sidebar.set_visible(true);
            content_stack.set_visible_child_name(PLAYLISTS_VIEW);
        }
    }
}

fn horizontal_spacer(width: i32) -> gtk::Box {
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_width_request(width);
    spacer
}

fn media_icon_button(icon_name: &str, tooltip: &str) -> gtk::Button {
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(MEDIA_ICON_SIZE);

    let button = gtk::Button::new();
    button.set_child(Some(&icon));
    button.set_tooltip_text(Some(tooltip));
    button.add_css_class("flat");
    button.add_css_class("media-control");
    set_titlebar_control_height(&button);
    button
}

fn set_titlebar_control_height(control: &gtk::Button) {
    control.set_height_request(TITLEBAR_CONTROL_HEIGHT);
}

fn set_mode_button_height(control: &gtk::ToggleButton) {
    control.set_height_request(MODE_BUTTON_HEIGHT);
    control.set_valign(gtk::Align::Center);
    control.add_css_class("mode-button");
}

fn build_sidebar() -> gtk::Box {
    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
    sidebar.add_css_class("playlist-sidebar");
    sidebar.set_vexpand(true);
    sidebar.set_size_request(SIDEBAR_MIN_WIDTH, -1);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
    content.set_vexpand(true);

    let title = gtk::Label::new(Some("Playlists"));
    title.set_margin_top(8);
    title.set_margin_end(8);
    title.set_margin_bottom(4);
    title.set_margin_start(8);
    title.set_xalign(0.0);
    content.append(&title);

    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let empty_state = gtk::Label::new(Some("No playlists imported yet"));
    empty_state.set_margin_top(8);
    empty_state.set_margin_end(8);
    empty_state.set_margin_bottom(8);
    empty_state.set_margin_start(8);
    empty_state.set_xalign(0.0);
    content.append(&empty_state);

    sidebar.append(&content);

    sidebar
}

fn build_content_stack(library_tracks: &[TrackTableRow]) -> gtk::Stack {
    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);

    let songs_view = build_track_table(library_tracks.to_vec());
    let albums_view = build_album_area();
    let playlists_view = build_track_table(mock_playlist_tracks(library_tracks));

    stack.add_named(&songs_view, Some(SONGS_VIEW));
    stack.add_named(&albums_view, Some(ALBUMS_VIEW));
    stack.add_named(&playlists_view, Some(PLAYLISTS_VIEW));
    stack.set_visible_child_name(SONGS_VIEW);

    stack
}

fn build_album_area() -> gtk::ScrolledWindow {
    let flow = gtk::FlowBox::new();
    flow.set_margin_top(12);
    flow.set_margin_end(12);
    flow.set_margin_bottom(12);
    flow.set_margin_start(12);
    flow.set_max_children_per_line(8);
    flow.set_selection_mode(gtk::SelectionMode::None);

    for index in 1..=12 {
        let item = gtk::Box::new(gtk::Orientation::Vertical, 6);
        item.set_margin_top(6);
        item.set_margin_end(6);
        item.set_margin_bottom(6);
        item.set_margin_start(6);

        let cover = gtk::Box::new(gtk::Orientation::Vertical, 0);
        cover.add_css_class("card");
        cover.set_size_request(120, 120);

        let title = gtk::Label::new(Some(&format!("Album {index}")));
        title.set_xalign(0.0);

        item.append(&cover);
        item.append(&title);
        flow.insert(&item, -1);
    }

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_vexpand(true);
    scroller.set_hexpand(true);
    scroller.set_child(Some(&flow));
    scroller
}

fn build_status_bar(library_tracks: &[TrackTableRow]) -> gtk::CenterBox {
    let status = gtk::CenterBox::new();
    status.add_css_class("status-bar");
    status.set_height_request(STATUS_BAR_HEIGHT);
    status.set_hexpand(true);

    let duration_seconds = library_tracks
        .iter()
        .map(|track| track.duration_seconds)
        .sum();
    let size_bytes = library_tracks
        .iter()
        .map(|track| track.file_size_bytes)
        .sum();
    let summary = gtk::Label::new(Some(&library_status_text(
        library_tracks.len(),
        duration_seconds,
        size_bytes,
    )));
    summary.set_xalign(0.5);
    status.set_center_widget(Some(&summary));
    status
}

fn library_status_text(track_count: usize, duration_seconds: u64, size_bytes: u64) -> String {
    format!(
        "{} {}, {}, {}",
        track_count,
        pluralize(track_count, "song", "songs"),
        duration_text(duration_seconds),
        file_size_text(size_bytes),
    )
}

fn duration_text(duration_seconds: u64) -> String {
    let hours = duration_seconds / 3_600;
    if hours >= 24 {
        let days = hours / 24;
        format!("{} {}", days, pluralize(days as usize, "day", "days"))
    } else {
        format!("{} {}", hours, pluralize(hours as usize, "hour", "hours"))
    }
}

fn file_size_text(size_bytes: u64) -> String {
    const MB: u64 = 1_000_000;
    const GB: u64 = 1_000_000_000;

    if size_bytes >= GB {
        format!("{} GB", size_bytes / GB)
    } else {
        format!("{} MB", size_bytes / MB)
    }
}

fn pluralize(count: usize, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 { singular } else { plural }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_status_uses_hours_and_megabytes_for_small_libraries() {
        assert_eq!(
            library_status_text(2, 7_200, 250_000_000),
            "2 songs, 2 hours, 250 MB"
        );
    }

    #[test]
    fn library_status_uses_days_and_gigabytes_for_large_libraries() {
        assert_eq!(
            library_status_text(1, 172_800, 3_000_000_000),
            "1 song, 2 days, 3 GB"
        );
    }
}
