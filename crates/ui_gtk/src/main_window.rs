use std::{cell::RefCell, rc::Rc};

use gtk::prelude::*;
use gtk::{gdk, glib};
use xtunes_app_runtime::{PlaybackCommand, Rating, TrackId};

use super::{
    APP_ID, ApplicationCommand, ApplicationRuntime, LibraryChangedCallback, LibraryChangedHolder,
    PlaybackChangedCallback, SharedRuntime,
    accent::install_accent_css,
    albums::AlbumsView,
    app_css::install_app_css,
    command_controller::{SharedCommandController, UiCommandController},
    content_stack::build_content_stack,
    library_scan::library_scan_requested_callback,
    mode_bar::build_mode_bar,
    now_playing::NowPlayingView,
    preferences::install_preferences_action,
    sidebar::{build_content_area, build_sidebar},
    status_bar::StatusBar,
    titlebar::{
        Titlebar, build_titlebar, connect_titlebar_playback_controls, sync_play_pause_icon,
    },
    track_context::{
        TrackActionCallback, TrackContextAction, TrackContextActionSet, TrackRowContextMenu,
    },
    track_table::{
        RatingChangedCallback, TrackActivatedCallback, TrackTable, TrackTableRow, build_track_table,
    },
    window_chrome::{install_resize_handles, install_window_state_chrome},
};

pub(crate) fn build_main_window(
    app: &gtk::Application,
    runtime: SharedRuntime,
) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .decorated(false)
        .title("xTunes")
        .default_width(1100)
        .default_height(720)
        .build();
    window.add_css_class("app-window");
    window.set_resizable(true);
    install_app_icon();
    window.set_icon_name(Some(APP_ID));
    install_app_css();
    install_accent_css();

    let library_tracks = runtime_library_table_rows(&runtime.borrow());
    let status_bar = StatusBar::new(&library_tracks);
    let command_controller: SharedCommandController = Rc::new(UiCommandController::new(
        runtime.clone(),
        status_bar.clone(),
    ));

    let songs_table_holder: Rc<RefCell<Option<TrackTable>>> = Rc::new(RefCell::new(None));

    let now_playing = NowPlayingView::new(runtime.clone(), command_controller.clone());
    let titlebar = build_titlebar(now_playing.widget());
    let playback_changed = playback_changed_callback(
        &runtime,
        &now_playing,
        &titlebar,
        songs_table_holder.clone(),
    );
    connect_titlebar_playback_controls(
        &titlebar,
        command_controller.clone(),
        playback_changed.clone(),
    );
    install_keyboard_shortcuts(
        &window,
        command_controller.clone(),
        playback_changed.clone(),
    );

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("app-shell");
    root.set_hexpand(true);
    root.set_vexpand(true);
    root.set_overflow(gtk::Overflow::Hidden);

    let sidebar = build_sidebar();
    sidebar.set_visible(false);

    let track_activated = track_activated_callback(&command_controller, playback_changed.clone());
    let library_changed_holder: LibraryChangedHolder = Rc::new(RefCell::new(None));
    let context_actions = track_context_actions(
        &command_controller,
        playback_changed.clone(),
        library_changed_holder.clone(),
    );
    let context_menu =
        TrackRowContextMenu::new(context_actions, window.clone().upcast::<gtk::Window>());
    let rating_changed =
        rating_changed_callback(&command_controller, library_changed_holder.clone());
    let songs_table = build_track_table(
        library_tracks.clone(),
        Some(track_activated.clone()),
        Some(context_menu.clone()),
        Some(rating_changed),
    );
    songs_table_holder.replace(Some(songs_table.clone()));
    let albums_view = AlbumsView::new(
        runtime.clone(),
        command_controller.clone(),
        playback_changed.clone(),
        context_menu.clone(),
    );
    playback_changed();
    let content_stack = build_content_stack(
        songs_table.widget(),
        albums_view.widget(),
        track_activated,
        context_menu,
    );
    let library_changed =
        library_changed_callback(&runtime, &songs_table, &albums_view, &status_bar);
    library_changed_holder.replace(Some(library_changed.clone()));
    let scan_requested =
        library_scan_requested_callback(&runtime, library_changed.clone(), &status_bar);
    install_preferences_action(
        app,
        &window,
        command_controller.clone(),
        scan_requested.clone(),
    );

    let main_content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main_content.set_hexpand(true);
    main_content.set_vexpand(true);
    main_content.append(&build_mode_bar(
        &window,
        &sidebar,
        &content_stack,
        command_controller,
        scan_requested,
    ));
    main_content.append(&content_stack);

    let content_area = build_content_area(&sidebar, &main_content);

    root.append(&titlebar.widget);
    root.append(&content_area);
    root.append(&status_bar.widget());

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

fn library_changed_callback(
    runtime: &SharedRuntime,
    songs_table: &TrackTable,
    albums_view: &AlbumsView,
    status_bar: &StatusBar,
) -> LibraryChangedCallback {
    let runtime = runtime.clone();
    let songs_table = songs_table.clone();
    let albums_view = albums_view.clone();
    let status_bar = status_bar.clone();

    Rc::new(move || {
        let rows = runtime_library_table_rows(&runtime.borrow());
        songs_table.replace_rows(rows.clone());
        albums_view.replace_tracks(runtime.borrow().library_tracks().to_vec());
        status_bar.update_summary(&rows);
    })
}

fn runtime_library_table_rows(runtime: &ApplicationRuntime) -> Vec<TrackTableRow> {
    let library_root = runtime.settings().library_path();
    runtime
        .library_tracks()
        .iter()
        .map(|track| TrackTableRow::from_track(track, library_root))
        .collect()
}

fn playback_changed_callback(
    runtime: &SharedRuntime,
    now_playing: &NowPlayingView,
    titlebar: &Titlebar,
    songs_table_holder: Rc<RefCell<Option<TrackTable>>>,
) -> PlaybackChangedCallback {
    let runtime = runtime.clone();
    let now_playing = now_playing.clone();
    let play_pause_icon = titlebar.play_pause_icon.clone();

    Rc::new(move || {
        let now_playing_state = runtime.borrow().now_playing();
        sync_play_pause_icon(&play_pause_icon, &now_playing_state.state);
        let playing_track_id = now_playing_state.track.as_ref().map(|track| track.id);
        if let Some(songs_table) = songs_table_holder.borrow().as_ref() {
            songs_table.set_playing_track_id(playing_track_id);
        }
        now_playing.refresh(&now_playing_state);
    })
}

fn track_activated_callback(
    command_controller: &SharedCommandController,
    playback_changed: PlaybackChangedCallback,
) -> TrackActivatedCallback {
    let command_controller = command_controller.clone();

    Rc::new(move |track_id: TrackId| {
        if command_controller.dispatch_succeeded(ApplicationCommand::Playback(
            PlaybackCommand::PlayTrack(track_id),
        )) {
            playback_changed();
        }
    })
}

fn rating_changed_callback(
    command_controller: &SharedCommandController,
    library_changed_holder: LibraryChangedHolder,
) -> RatingChangedCallback {
    let command_controller = command_controller.clone();

    Rc::new(move |track_id: TrackId, rating: Rating| {
        if !command_controller
            .dispatch_succeeded(ApplicationCommand::SetRating { track_id, rating })
        {
            return false;
        }
        if let Some(callback) = library_changed_holder.borrow().as_ref() {
            callback();
        }
        true
    })
}

fn track_context_actions(
    command_controller: &SharedCommandController,
    playback_changed: PlaybackChangedCallback,
    library_changed_holder: LibraryChangedHolder,
) -> TrackContextActionSet {
    TrackContextActionSet::new(vec![
        TrackContextAction::remove_from_library(track_mutation_callback(
            command_controller,
            playback_changed.clone(),
            library_changed_holder.clone(),
            |track_id| ApplicationCommand::RemoveTrackFromLibrary { track_id },
        )),
        TrackContextAction::move_to_trash(track_mutation_callback(
            command_controller,
            playback_changed,
            library_changed_holder,
            |track_id| ApplicationCommand::MoveTrackToTrash { track_id },
        )),
    ])
}

fn track_mutation_callback(
    command_controller: &SharedCommandController,
    playback_changed: PlaybackChangedCallback,
    library_changed_holder: LibraryChangedHolder,
    command_builder: impl Fn(TrackId) -> ApplicationCommand + 'static,
) -> TrackActionCallback {
    let command_controller = command_controller.clone();

    Rc::new(move |track_ids: Vec<TrackId>| {
        let commands = track_ids
            .into_iter()
            .map(|track_id| command_builder(track_id))
            .collect::<Vec<_>>();
        let result = command_controller.dispatch_batch(commands);
        if result.succeeded == 0 {
            return;
        }
        playback_changed();
        if let Some(callback) = library_changed_holder.borrow().as_ref() {
            callback();
        }
    })
}

fn install_keyboard_shortcuts(
    window: &gtk::ApplicationWindow,
    command_controller: SharedCommandController,
    playback_changed: PlaybackChangedCallback,
) {
    let key_controller = gtk::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);

    let window_for_focus = window.clone();
    key_controller.connect_key_pressed(move |_controller, key, _keycode, _state| {
        if key != gdk::Key::space || focus_accepts_text(&window_for_focus) {
            return glib::Propagation::Proceed;
        }

        if command_controller.dispatch_succeeded(ApplicationCommand::Playback(
            PlaybackCommand::TogglePlayPause,
        )) {
            playback_changed();
        }
        glib::Propagation::Stop
    });
    window.add_controller(key_controller);
}

fn focus_accepts_text(window: &gtk::ApplicationWindow) -> bool {
    let Some(mut focus) = gtk::prelude::RootExt::focus(window) else {
        return false;
    };

    loop {
        if focus.is::<gtk::Editable>() {
            return true;
        }

        let Some(parent) = focus.parent() else {
            return false;
        };
        focus = parent;
    }
}

fn install_app_icon() {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let theme = gtk::IconTheme::for_display(&display);

    // During development (cargo run), icons live under data/icons in the project tree.
    // At compile time, CARGO_MANIFEST_DIR points to crates/ui_gtk/.
    let dev_icons = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/icons");
    if dev_icons.exists() {
        theme.add_search_path(dev_icons);
    }
}
