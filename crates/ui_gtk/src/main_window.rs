// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use gtk::prelude::*;
use gtk::{gdk, glib};
use sustain_app_runtime::{
    PlaybackCommand, PlaybackQueueRequest, PlaybackQueueSource, Playlist, PlaylistEntry,
    PlaylistFolder, PlaylistFolderId, PlaylistItem, Rating, ShuffleMode, Track, TrackColumnLayout,
    TrackColumnLayoutScope, TrackId, UiSettings, UiSidebarSelection, track_matches_search_text,
};

use super::{
    ALBUMS_VIEW, APP_ID, AnalysisProgressReceiver, ApplicationCommand, ApplicationRuntime,
    ArtworkFetchResultReceiver, AvailabilityChangedCallback, LibraryChangedCallback,
    LibraryChangedHolder, MetadataWriteResultReceiver, MprisCommandReceiver,
    OnlineProgressReceiver, PLAYLISTS_VIEW, PlaybackChangedCallback, SIDEBAR_DEFAULT_WIDTH,
    SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH, SONGS_VIEW, SharedMprisService, SharedRuntime,
    ShowAlbumAction, ShowAlbumHolder, SmartPlaylistTrackStatus, SmartShuffleRebuildResultReceiver,
    TrackRowChangedCallback, TrackRowChangedHolder, TrackUpdatedReceiver,
    accent::install_accent_css,
    albums::AlbumsView,
    app_css::install_app_css,
    artwork_loader::ArtworkLoader,
    command_controller::{SharedCommandController, UiCommandController},
    content_stack::build_content_stack,
    library_consolidation::{
        library_consolidation_requested_callback, maybe_auto_resume_library_consolidation,
    },
    library_import::{
        LIBRARY_DROP_INDICATOR_CLASS, install_file_drop_target, library_import_requested_callback,
    },
    library_scan::library_scan_requested_callback,
    now_playing::NowPlayingView,
    playlists_header::{PlaylistsHeader, PlaylistsHeaderState},
    preferences::{install_preferences_action, settings_button},
    shortcuts::{
        GlobalShortcutContext, create_new_playlist, install_global_shortcuts,
        open_new_smart_playlist_editor,
    },
    sidebar::{PlaylistSidebar, SidebarSelection, build_content_area},
    sidebar_context::{
        NEW_PLAYLIST_FOLDER_DEFAULT_NAME, SidebarActionCallback, SidebarContextAction,
        SidebarContextMenu, unique_default_name,
    },
    smart_playlist_editor::{SmartPlaylistEditorMode, open_smart_playlist_editor},
    status_bar::StatusBar,
    titlebar::{
        Titlebar, build_titlebar, connect_titlebar_playback_controls, connect_titlebar_search,
        sync_play_pause_icon,
    },
    track_context::{
        AddToPlaylistCallback, AddToPlaylistEntry, AddToPlaylistProvider, TrackActionCallback,
        TrackActionVisibility, TrackAnalyzeEnabledQuery, TrackAnalyzeRunCallback,
        TrackContextAction, TrackContextActionSet, TrackRetrieveEnabledQuery,
        TrackRetrieveRunCallback, TrackRowContextMenu,
    },
    track_context_ops::{
        add_to_queue_callback, copy_files_callback, get_info_callback, play_next_callback,
        playback_has_current_track_visibility, show_album_callback, show_in_folder_callback,
        track_has_album_visibility,
    },
    track_table::{
        RatingChangedCallback, RowDropPosition, RowReorderCallback, RowReorderDrop,
        TrackActivatedCallback, TrackTable, TrackTableRow, build_track_table,
    },
    window_chrome::{install_resize_handles, install_window_state_chrome},
};

/// Recompute the status-bar summary (track count, total duration) for
/// whichever view is currently visible. Fired after sidebar-driven
/// view switches, library mutations, and search keystrokes.
pub(crate) type VisibleSummaryRefreshCallback = Rc<dyn Fn()>;

/// Channel receivers the main window installs as glib consumers on
/// the GTK main loop. Bundled into a struct rather than passed as
/// individual `build_main_window` parameters so the function signature
/// stays under clippy's argument-count threshold and so adding the
/// next background worker is a one-line struct extension instead of
/// touching every call site.
pub(crate) struct MainWindowAsyncReceivers {
    pub mpris_command_rx: Option<MprisCommandReceiver>,
    pub metadata_write_result_rx: Option<MetadataWriteResultReceiver>,
    pub artwork_fetch_result_rx: Option<ArtworkFetchResultReceiver>,
    pub analysis_progress_rx: Option<AnalysisProgressReceiver>,
    pub online_progress_rx: Option<OnlineProgressReceiver>,
    pub track_updated_rx: Option<TrackUpdatedReceiver>,
    pub smart_shuffle_rebuild_result_rx: Option<SmartShuffleRebuildResultReceiver>,
}

pub(crate) fn build_main_window(
    app: &gtk::Application,
    runtime: SharedRuntime,
    mpris_service: Option<SharedMprisService>,
    receivers: MainWindowAsyncReceivers,
) -> BuiltMainWindow {
    let MainWindowAsyncReceivers {
        mpris_command_rx,
        metadata_write_result_rx,
        artwork_fetch_result_rx,
        analysis_progress_rx,
        online_progress_rx,
        track_updated_rx,
        smart_shuffle_rebuild_result_rx,
    } = receivers;
    let tbw = std::time::Instant::now();
    macro_rules! tlog {
        ($label:expr) => {
            eprintln!(
                "[TIMING]     build_main_window+{:>7.1}ms {}",
                tbw.elapsed().as_secs_f64() * 1000.0,
                $label
            );
        };
    }
    tlog!("entered");
    // Coarse timing landmarks live in this function (and in `main` /
    // `ui_gtk::run`) so a launch regression shows up the first time
    // anyone runs the app from a terminal. Keep them sparse: only
    // phases that can plausibly grow with library size or new
    // features warrant a print. Per-callback timings inside hot
    // paths are intentionally absent.
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .decorated(false)
        .title("Sustain")
        .default_width(1100)
        .default_height(720)
        .build();
    window.add_css_class("app-window");
    window.set_resizable(true);
    install_app_icon();
    window.set_icon_name(Some(APP_ID));
    install_app_css();
    install_accent_css();

    let initial_ui_settings = runtime.borrow().settings().ui.clone();
    let initial_search_text = initial_ui_settings.search_text.trim().to_owned();

    // Shared current-search-text state. Captured by all view-rebuild paths
    // and persisted on normal shutdown with the rest of the UI session.
    let current_search_text: Rc<RefCell<String>> =
        Rc::new(RefCell::new(initial_search_text.clone()));

    let library_tracks = runtime_library_table_rows(&runtime.borrow(), &initial_search_text);
    tlog!("library rows materialised");
    let status_bar = {
        let runtime_for_cancel = runtime.clone();
        StatusBar::new(
            &library_tracks,
            Rc::new(move || {
                runtime_for_cancel
                    .borrow()
                    .request_background_task_cancellation();
            }),
        )
    };
    let command_controller: SharedCommandController =
        Rc::new(UiCommandController::new(runtime.clone()));
    // Wire the lane to observe runtime notifications before any
    // callback can push a notification — otherwise an early ephemeral
    // would land in the queue without a renderer attached.
    status_bar.attach_to_runtime(&runtime);

    let songs_table_holder: Rc<RefCell<Option<TrackTable>>> = Rc::new(RefCell::new(None));
    let albums_view_holder: Rc<RefCell<Option<AlbumsView>>> = Rc::new(RefCell::new(None));
    let playlists_table_holder: Rc<RefCell<Option<TrackTable>>> = Rc::new(RefCell::new(None));

    // One artwork loader for the whole window. Sharing it across views
    // means the on-disk cache, in-memory cache, and worker pool are all
    // single-instance — a track resolved by the Albums grid is
    // immediately available to the now-playing tile and vice versa.
    // Construction launches worker threads, so do it once after the
    // metadata service is installed and before any view subscribes.
    let metadata_service = runtime
        .borrow()
        .metadata_service()
        .expect("metadata service must be installed before building the main window");
    let artwork_loader = ArtworkLoader::new(metadata_service);

    let now_playing = NowPlayingView::new(
        runtime.clone(),
        command_controller.clone(),
        artwork_loader.clone(),
    );
    let initial_volume = runtime.borrow().settings().playback.volume;
    let titlebar = build_titlebar(now_playing.widget(), initial_volume);
    titlebar.set_search_text(&initial_search_text);
    let playback_changed = playback_changed_callback(
        &runtime,
        &now_playing,
        &titlebar,
        songs_table_holder.clone(),
        albums_view_holder.clone(),
        playlists_table_holder.clone(),
        mpris_service.clone(),
    );
    connect_titlebar_playback_controls(
        &titlebar,
        &runtime,
        command_controller.clone(),
        playback_changed.clone(),
    );
    install_track_ended_callback(&runtime, &command_controller, &playback_changed);
    install_mpris_command_consumer(
        mpris_command_rx,
        command_controller.clone(),
        playback_changed.clone(),
        app.clone(),
        window.clone(),
    );

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("app-shell");
    root.set_hexpand(true);
    root.set_vexpand(true);
    root.set_overflow(gtk::Overflow::Hidden);

    let sidebar = PlaylistSidebar::new(runtime.clone());
    let sidebar_widget = sidebar.widget();

    let library_track_activated = library_track_activated_callback(
        &command_controller,
        &runtime,
        playback_changed.clone(),
        &current_search_text,
    );
    let library_changed_holder: LibraryChangedHolder = Rc::new(RefCell::new(None));
    let track_row_changed_holder: TrackRowChangedHolder = Rc::new(RefCell::new(None));
    let parent_window = window.clone().upcast::<gtk::Window>();
    let show_album_holder: ShowAlbumHolder = Rc::new(RefCell::new(None));
    let context_actions = track_context_actions(
        &runtime,
        &parent_window,
        &show_album_holder,
        &command_controller,
        playback_changed.clone(),
        library_changed_holder.clone(),
        track_row_changed_holder.clone(),
    );
    let add_to_playlist_provider = add_to_playlist_provider(&runtime);
    let add_to_playlist_callback =
        add_to_playlist_callback(&command_controller, &runtime, &library_changed_holder);
    let context_menu = TrackRowContextMenu::new(context_actions, parent_window.clone())
        .with_add_to_playlist(
            add_to_playlist_provider.clone(),
            add_to_playlist_callback.clone(),
        )
        .with_analyze_menu(
            track_analyze_run_callback(&runtime),
            analysis_enabled_query(&runtime),
        )
        .with_retrieve_menu(
            track_retrieve_run_callback(&runtime),
            online_enabled_query(&runtime),
        );
    let playlist_context_actions = playlist_track_context_actions(
        &runtime,
        &parent_window,
        &show_album_holder,
        &command_controller,
        playback_changed.clone(),
        library_changed_holder.clone(),
        track_row_changed_holder.clone(),
        &sidebar,
    );
    let playlist_context_menu = TrackRowContextMenu::new(playlist_context_actions, parent_window)
        .with_add_to_playlist(add_to_playlist_provider, add_to_playlist_callback)
        .with_analyze_menu(
            track_analyze_run_callback(&runtime),
            analysis_enabled_query(&runtime),
        )
        .with_retrieve_menu(
            track_retrieve_run_callback(&runtime),
            online_enabled_query(&runtime),
        );
    let rating_changed =
        rating_changed_callback(&command_controller, track_row_changed_holder.clone());
    let songs_table = build_track_table(
        library_tracks.clone(),
        Some(library_track_activated.clone()),
        Some(context_menu.clone()),
        Some(rating_changed.clone()),
        None,
    );
    tlog!("songs table populated");
    songs_table_holder.replace(Some(songs_table.clone()));
    let albums_view = AlbumsView::new(
        runtime.clone(),
        command_controller.clone(),
        playback_changed.clone(),
        context_menu,
        artwork_loader.clone(),
    );
    albums_view_holder.replace(Some(albums_view.clone()));
    let playlist_row_reorder = playlist_row_reorder_callback(
        &command_controller,
        &runtime,
        &sidebar,
        &playlists_table_holder,
        &current_search_text,
    );
    let playlist_track_activated = playlist_track_activated_callback(
        &command_controller,
        &runtime,
        &sidebar,
        playback_changed.clone(),
        &current_search_text,
    );
    let playlists_table = build_track_table(
        Vec::new(),
        Some(playlist_track_activated),
        Some(playlist_context_menu),
        Some(rating_changed),
        Some(playlist_row_reorder),
    );
    playlists_table_holder.replace(Some(playlists_table.clone()));
    install_track_column_layout_persistence(&runtime, &songs_table, &playlists_table, &sidebar);
    playback_changed();
    tlog!("tables + playback wired");
    let playlists_header = PlaylistsHeader::new();
    let playlists_view = gtk::Box::new(gtk::Orientation::Vertical, 0);
    playlists_view.set_hexpand(true);
    playlists_view.set_vexpand(true);
    playlists_view.append(playlists_header.widget());
    playlists_view.append(&playlists_table.widget());
    install_playlists_header_playback(
        &playlists_header,
        &command_controller,
        &runtime,
        &sidebar,
        &current_search_text,
        &playback_changed,
    );
    let songs_drop_indicator = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    songs_drop_indicator.add_css_class(LIBRARY_DROP_INDICATOR_CLASS);
    songs_drop_indicator.set_can_target(false);
    songs_drop_indicator.set_hexpand(true);
    songs_drop_indicator.set_vexpand(true);

    let songs_drop_overlay = gtk::Overlay::new();
    songs_drop_overlay.set_hexpand(true);
    songs_drop_overlay.set_vexpand(true);
    songs_drop_overlay.set_child(Some(&songs_table.widget()));
    songs_drop_overlay.add_overlay(&songs_drop_indicator);

    let content_stack =
        build_content_stack(&songs_drop_overlay, &albums_view.widget(), &playlists_view);
    install_albums_view_activator(&content_stack, &albums_view);
    // The playlists table is built empty. It only needs to be populated
    // when the user actually opens the Playlists view; rebuilding it on
    // every library_changed / selection change while Songs is visible
    // is wasted work and dominates startup time on large libraries
    // (measured: ~672ms for 8890 rows in `replace_rows`).
    let playlists_dirty: Rc<Cell<bool>> = Rc::new(Cell::new(true));
    install_playlists_view_activator(
        &content_stack,
        &runtime,
        &playlists_table,
        &playlists_header,
        &sidebar,
        &current_search_text,
        &playlists_dirty,
    );
    tlog!("content stack + activators installed");
    let visible_summary_refresh = visible_summary_refresh_callback(
        &runtime,
        &content_stack,
        &sidebar,
        &status_bar,
        &current_search_text,
    );
    let library_changed = library_changed_callback(
        &runtime,
        &songs_table,
        &albums_view,
        &sidebar,
        visible_summary_refresh.clone(),
        &current_search_text,
    );
    install_track_availability_observer(&runtime, &songs_table, &playlists_table);
    let track_row_changed = track_row_changed_callback(TrackRowChangedContext {
        runtime: &runtime,
        songs_table: &songs_table,
        albums_view: &albums_view,
        playlists_table: &playlists_table,
        playlists_header: &playlists_header,
        sidebar: &sidebar,
        content_stack: &content_stack,
        playlists_dirty: &playlists_dirty,
        visible_summary_refresh: visible_summary_refresh.clone(),
        current_search_text: &current_search_text,
    });
    track_row_changed_holder.replace(Some(track_row_changed));
    install_metadata_write_result_consumer(
        metadata_write_result_rx,
        runtime.clone(),
        track_row_changed_holder.clone(),
    );
    install_artwork_fetch_result_consumer(ArtworkFetchResultConsumerContext {
        receiver: artwork_fetch_result_rx,
        runtime: runtime.clone(),
        command_controller: command_controller.clone(),
        artwork_loader: artwork_loader.clone(),
        now_playing: now_playing.clone(),
        playback_changed: playback_changed.clone(),
        track_row_changed_holder: track_row_changed_holder.clone(),
    });
    install_analysis_progress_consumer(analysis_progress_rx, runtime.clone());
    install_online_progress_consumer(online_progress_rx, runtime.clone());
    install_track_data_observer(&runtime, track_row_changed_holder.clone());
    install_track_updated_consumer(track_updated_rx, runtime.clone());
    install_smart_shuffle_rebuild_result_consumer(smart_shuffle_rebuild_result_rx, runtime.clone());
    install_smart_shuffle_periodic_rebuild(&runtime);
    // The sidebar is now the sole navigation surface: its selection
    // chooses which content-stack page is visible (Music → SONGS_VIEW,
    // Albums → ALBUMS_VIEW, an Item → PLAYLISTS_VIEW). The non-default
    // selections are applied AFTER first-frame by
    // [`DeferredStartup`] so the cold-start budget covers only the
    // cheap Music page.
    sidebar.set_selection_changed(sidebar_selection_changed_callback(
        &runtime,
        &playlists_table,
        &playlists_header,
        &content_stack,
        &playlists_dirty,
        visible_summary_refresh.clone(),
        &current_search_text,
    ));
    let deferred_startup =
        DeferredStartup::new(initial_ui_settings.sidebar_selection, sidebar.clone());
    install_search_wiring(
        &titlebar,
        SearchWiringContext {
            current_search_text: current_search_text.clone(),
            runtime: runtime.clone(),
            songs_table: songs_table.clone(),
            albums_view: albums_view.clone(),
            playlists_table: playlists_table.clone(),
            playlists_header: playlists_header.clone(),
            sidebar: sidebar.clone(),
            content_stack: content_stack.clone(),
            playlists_dirty: playlists_dirty.clone(),
            visible_summary_refresh: visible_summary_refresh.clone(),
        },
    );
    sidebar.install_context_menu(SidebarContextMenu::new(sidebar_action_callback(
        &window,
        &command_controller,
        &runtime,
        &sidebar,
    )));
    sidebar.set_move_callback(sidebar_move_callback(
        &command_controller,
        &runtime,
        &sidebar,
    ));
    sidebar.set_rename_callback(sidebar_rename_callback(
        &command_controller,
        &runtime,
        &sidebar,
    ));
    sidebar.set_delete_callback(sidebar_delete_callback(&command_controller, &sidebar));
    sidebar.set_tracks_drop_callback(sidebar_tracks_drop_callback(
        &command_controller,
        &library_changed_holder,
    ));
    sidebar.set_edit_smart_playlist_callback(sidebar_edit_smart_playlist_callback(
        &window,
        &command_controller,
        &runtime,
        &sidebar,
    ));
    sidebar.set_analysis_run_callback(sidebar_analysis_run_callback(&runtime));
    sidebar.set_online_run_callback(sidebar_online_run_callback(&runtime));
    sidebar.set_analysis_enabled_query(sidebar_analysis_enabled_query(&runtime));
    sidebar.set_online_enabled_query(sidebar_online_enabled_query(&runtime));
    library_changed_holder.replace(Some(library_changed.clone()));
    let scan_requested = library_scan_requested_callback(&runtime, library_changed.clone());
    let consolidation_requested = library_consolidation_requested_callback(&runtime);
    let import_requested = library_import_requested_callback(&runtime, library_changed.clone());
    install_file_drop_target(&songs_drop_overlay, &songs_drop_indicator, import_requested);
    install_preferences_action(
        app,
        &window,
        command_controller.clone(),
        scan_requested.clone(),
        consolidation_requested.clone(),
    );

    let main_content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main_content.set_hexpand(true);
    main_content.set_vexpand(true);
    let command_controller_for_shortcuts = command_controller.clone();
    let command_controller_for_global_shortcuts = command_controller.clone();

    // Sidebar footer: the [cog] Settings button is the visual
    // entry-point to Preferences (the Ctrl+, accelerator is the power-
    // user path, registered separately by `install_preferences_action`).
    sidebar.footer().append(&settings_button(
        &window,
        command_controller,
        scan_requested,
        consolidation_requested.clone(),
    ));

    main_content.append(&content_stack);

    // gtk::Paned keeps drag-resize between SIDEBAR_MIN_WIDTH and
    // SIDEBAR_MAX_WIDTH, with the user's manually-set width preserved
    // for the next launch via the sidebar collapse controller. The
    // collapse animation tweens the Paned position rather than
    // hiding the sidebar widget, so the existing min/max clamp and
    // drag handle survive untouched. Construct the controller before
    // wiring shortcuts so Ctrl+N / Ctrl+Alt+N can re-expand a
    // collapsed sidebar before arming a row rename.
    let content_area = build_content_area(&sidebar_widget, &main_content);
    let collapse_controller = SidebarCollapseController::new(
        content_area.clone(),
        initial_ui_settings.sidebar_collapsed,
        initial_ui_settings.sidebar_width,
    );
    status_bar.install_sidebar_collapse_toggle(collapse_controller.toggle_widget());

    let albums_view_for_reveal = albums_view.clone();
    let sidebar_for_show_album = sidebar.clone();
    let show_album_action: ShowAlbumAction = Rc::new(move |track_id| {
        sidebar_for_show_album.select_albums();
        albums_view_for_reveal.reveal_album_for_track(track_id);
    });
    show_album_holder.replace(Some(show_album_action));
    install_keyboard_shortcuts(
        &window,
        KeyboardShortcutContext {
            command_controller: command_controller_for_shortcuts,
            playback_changed: playback_changed.clone(),
            runtime: runtime.clone(),
            songs_table: songs_table.clone(),
            playlists_table: playlists_table.clone(),
            albums_view: albums_view.clone(),
            content_stack: content_stack.clone(),
            sidebar: sidebar.clone(),
        },
    );
    install_global_shortcuts(GlobalShortcutContext {
        app: app.clone(),
        window: window.clone(),
        command_controller: command_controller_for_global_shortcuts,
        runtime: runtime.clone(),
        sidebar: sidebar.clone(),
        sidebar_collapse: collapse_controller.clone(),
        titlebar: titlebar.clone(),
        songs_table: songs_table.clone(),
        playlists_table: playlists_table.clone(),
        content_stack: content_stack.clone(),
        library_changed_holder: library_changed_holder.clone(),
        track_row_changed_holder: track_row_changed_holder.clone(),
    });

    root.append(&titlebar.widget);
    root.append(&content_area);
    root.append(&status_bar.widget());

    // `window_frame` is the visible window: it carries `.window-frame`
    // (shadow + rounded corners) and hosts the resize-handle overlays so the
    // handles snap to the actual visible edges. `shadow_gutter` is the outer
    // box whose only job is to provide the inset where the shadow renders.
    let window_frame = gtk::Overlay::new();
    window_frame.set_child(Some(&root));
    install_resize_handles(&window_frame, &window);
    install_window_state_chrome(&window, &window_frame);

    let shadow_gutter = gtk::Box::new(gtk::Orientation::Vertical, 0);
    shadow_gutter.add_css_class("csd");
    shadow_gutter.set_hexpand(true);
    shadow_gutter.set_vexpand(true);
    shadow_gutter.append(&window_frame);
    window.set_child(Some(&shadow_gutter));

    // Any debounced save scheduled within the debounce window of shutdown
    // would otherwise be lost: the timer's main loop never gets to fire.
    let songs_table_for_close = songs_table.clone();
    let playlists_table_for_close = playlists_table.clone();
    let titlebar_for_close = titlebar.clone();
    let runtime_for_close = runtime.clone();
    let sidebar_for_close = sidebar.clone();
    let collapse_controller_for_close = collapse_controller.clone();
    window.connect_close_request(move |_window| {
        songs_table_for_close.flush_pending_layout_save();
        playlists_table_for_close.flush_pending_layout_save();
        titlebar_for_close.flush_pending_volume_save();
        let _ = runtime_for_close
            .borrow_mut()
            .save_ui_settings(ui_settings_from_widgets(
                &titlebar_for_close,
                &sidebar_for_close,
                collapse_controller_for_close.is_collapsed(),
                collapse_controller_for_close.expanded_width(),
            ));
        glib::Propagation::Proceed
    });

    tlog!("widgets assembled");
    // If "Keep my library organized" is on, schedule consolidation
    // immediately. Idempotent (empty plan when already organized) and
    // the natural resume point for a previous run interrupted by a
    // kill or crash.
    maybe_auto_resume_library_consolidation(&runtime, &consolidation_requested);
    tlog!("auto-resume kicked off");

    BuiltMainWindow {
        window,
        deferred_startup,
    }
}

pub(crate) struct BuiltMainWindow {
    pub(crate) window: gtk::ApplicationWindow,
    deferred_startup: DeferredStartup,
}

impl BuiltMainWindow {
    pub(crate) fn run_deferred_startup(self) {
        self.deferred_startup.run();
    }
}

/// Post-first-frame work scheduled to keep the cold-start budget tight.
///
/// The Music view is the cheap default and is already built into the
/// content stack by the time `present()` returns. Restoring Albums or a
/// specific playlist as the persisted selection would otherwise drag
/// album-grouping or playlist-table population into the startup
/// critical path — both can run on the first idle instead, after the
/// window has had a chance to paint.
struct DeferredStartup {
    restore_selection: Option<Box<dyn FnOnce()>>,
}

impl DeferredStartup {
    fn new(selection: UiSidebarSelection, sidebar: PlaylistSidebar) -> Self {
        let restore_selection: Option<Box<dyn FnOnce()>> = match selection {
            UiSidebarSelection::Music => None,
            UiSidebarSelection::Albums => Some(Box::new(move || sidebar.select_albums())),
            UiSidebarSelection::Playlist(item) => Some(Box::new(move || sidebar.select_item(item))),
        };
        Self { restore_selection }
    }

    fn run(self) {
        if let Some(restore) = self.restore_selection {
            restore();
        }
    }
}

/// Defer the cost of populating the Albums view until the user
/// actually switches to it. Activation groups the current library into
/// album rows and lets the virtualized Albums view bind only visible
/// rows; doing that at startup provides no benefit while Music is the
/// initial visible page. Hooking into the content stack's
/// visible-child notification keeps the activation trigger in one
/// place — any caller that flips the stack to `ALBUMS_VIEW`
/// automatically picks it up. `activate()` is idempotent, so the
/// notification firing on every later switch is harmless.
fn install_albums_view_activator(content_stack: &gtk::Stack, albums_view: &AlbumsView) {
    let albums_view = albums_view.clone();
    content_stack.connect_visible_child_name_notify(move |stack| {
        if stack.visible_child_name().as_deref() == Some(ALBUMS_VIEW) {
            albums_view.activate();
        }
    });
}

/// Mirror of `install_albums_view_activator` for the Playlists view.
/// The table is built empty and stays empty while another page is
/// visible; `library_changed` / selection-changed / search rebuilds
/// flip a `dirty` flag instead of running `replace_rows`. When the
/// user picks a playlist row in the sidebar, the activator pays the
/// rebuild cost once with the current state and clears the flag.
/// Music is the default landing page, so in the common case the
/// playlists table is never populated for a session that does not
/// visit a playlist.
fn install_playlists_view_activator(
    content_stack: &gtk::Stack,
    runtime: &SharedRuntime,
    playlists_table: &TrackTable,
    playlists_header: &PlaylistsHeader,
    sidebar: &PlaylistSidebar,
    current_search_text: &Rc<RefCell<String>>,
    playlists_dirty: &Rc<Cell<bool>>,
) {
    let runtime = runtime.clone();
    let playlists_table = playlists_table.clone();
    let playlists_header = playlists_header.clone();
    let sidebar = sidebar.clone();
    let current_search_text = current_search_text.clone();
    let playlists_dirty = playlists_dirty.clone();
    content_stack.connect_visible_child_name_notify(move |stack| {
        if stack.visible_child_name().as_deref() != Some(PLAYLISTS_VIEW) {
            return;
        }
        if !playlists_dirty.get() {
            return;
        }
        let search_text = current_search_text.borrow().clone();
        rebuild_playlists_view(
            &runtime.borrow(),
            &playlists_table,
            &playlists_header,
            sidebar.current_selection(),
            &search_text,
        );
        playlists_dirty.set(false);
    });
}

fn ui_settings_from_widgets(
    titlebar: &Titlebar,
    sidebar: &PlaylistSidebar,
    sidebar_collapsed: bool,
    sidebar_width: u32,
) -> UiSettings {
    UiSettings {
        search_text: titlebar.search_text(),
        sidebar_selection: match sidebar.current_selection() {
            Some(SidebarSelection::Music) | None => UiSidebarSelection::Music,
            Some(SidebarSelection::Albums) => UiSidebarSelection::Albums,
            Some(SidebarSelection::Item(item)) => UiSidebarSelection::Playlist(item),
        },
        sidebar_collapsed,
        sidebar_width: Some(sidebar_width),
    }
}

/// Rebuild the playlists table only when the user is actually looking
/// at it. Triggers that fire while another view is visible (library
/// scan completion, search keystrokes, sidebar selection change) just
/// flip the dirty flag; `install_playlists_view_activator` runs the
/// rebuild on the next visit. See its doc-comment for the rationale.
fn refresh_playlists_view_if_visible(
    runtime: &ApplicationRuntime,
    content_stack: &gtk::Stack,
    playlists_table: &TrackTable,
    playlists_header: &PlaylistsHeader,
    sidebar_selection: Option<SidebarSelection>,
    search_text: &str,
    playlists_dirty: &Cell<bool>,
) {
    if content_stack.visible_child_name().as_deref() == Some(PLAYLISTS_VIEW) {
        rebuild_playlists_view(
            runtime,
            playlists_table,
            playlists_header,
            sidebar_selection,
            search_text,
        );
        playlists_dirty.set(false);
    } else {
        playlists_dirty.set(true);
    }
}

/// Unconditional rebuild of the playlists view (header + track table)
/// from the current selection + search filter. Header summary is derived
/// from the same row set fed to the table, so the visible "N songs, X
/// duration" text always matches what's drawn below it.
fn rebuild_playlists_view(
    runtime: &ApplicationRuntime,
    playlists_table: &TrackTable,
    playlists_header: &PlaylistsHeader,
    sidebar_selection: Option<SidebarSelection>,
    search_text: &str,
) {
    let rows = playlist_table_rows_for(runtime, sidebar_selection, search_text);
    playlists_header.set_state(playlists_header_state_for(
        runtime,
        sidebar_selection,
        &rows,
    ));
    playlists_table.replace_rows(rows);
}

fn playlists_header_state_for(
    runtime: &ApplicationRuntime,
    selection: Option<SidebarSelection>,
    rows: &[TrackTableRow],
) -> Option<PlaylistsHeaderState> {
    let title = match selection {
        Some(SidebarSelection::Item(PlaylistItem::Playlist(id))) => runtime
            .playlists()
            .iter()
            .find(|playlist| playlist.id == id)?
            .name
            .clone(),
        Some(SidebarSelection::Item(PlaylistItem::SmartPlaylist(id))) => runtime
            .smart_playlists()
            .iter()
            .find(|playlist| playlist.id == id)?
            .name
            .clone(),
        // Folders aggregate their children in the sidebar but are not
        // themselves a playable track set, so the header has nothing
        // meaningful to show. Music / Albums selections do not render
        // the playlists header at all (the stack shows a different
        // child).
        Some(SidebarSelection::Item(PlaylistItem::Folder(_)))
        | Some(SidebarSelection::Music)
        | Some(SidebarSelection::Albums)
        | None => return None,
    };
    Some(PlaylistsHeaderState {
        title,
        track_count: rows.len(),
        duration_seconds: rows.iter().map(|row| row.duration_seconds).sum(),
    })
}

fn library_changed_callback(
    runtime: &SharedRuntime,
    songs_table: &TrackTable,
    albums_view: &AlbumsView,
    sidebar: &PlaylistSidebar,
    visible_summary_refresh: VisibleSummaryRefreshCallback,
    current_search_text: &Rc<RefCell<String>>,
) -> LibraryChangedCallback {
    let runtime = runtime.clone();
    let songs_table = songs_table.clone();
    let albums_view = albums_view.clone();
    let sidebar = sidebar.clone();
    let current_search_text = current_search_text.clone();

    Rc::new(move || {
        let search_text = current_search_text.borrow().clone();
        let rows = runtime_library_table_rows(&runtime.borrow(), &search_text);
        songs_table.replace_rows(rows);
        // AlbumsView's internal apply_search() re-derives the visible album
        // set from the new track list using the search text it already
        // holds, so we don't need to call set_search_text here.
        albums_view.replace_tracks(runtime.borrow().library_tracks().to_vec());
        // sidebar.refresh() rebuilds the sidebar tree-model and fires the
        // selection callback exactly once. That callback owns the
        // playlists view — it runs `refresh_playlists_view_if_visible`,
        // so library_changed never needs to touch the playlists table
        // directly.
        sidebar.refresh();
        visible_summary_refresh();
    })
}

/// Wires the runtime's `track_availability_observer` to a narrow
/// per-row refresh on both track tables. The runtime fires this
/// observer after every lazy `is_missing` flip (failed-play
/// detection, library-path re-stat, consolidation source miss).
/// The deferred closure snapshots `(track_id, is_missing)` from the
/// runtime and asks each loaded table to patch matching rows in
/// place; never a `replace_rows`, so scroll/focus/selection survive
/// — see the design note on [`AvailabilityChangedCallback`].
fn install_track_availability_observer(
    runtime: &SharedRuntime,
    songs_table: &TrackTable,
    playlists_table: &TrackTable,
) {
    let runtime_for_observer = runtime.clone();
    let songs_table = songs_table.clone();
    let playlists_table = playlists_table.clone();
    let refresh: AvailabilityChangedCallback = Rc::new(move || {
        let availability: HashMap<TrackId, bool> = runtime_for_observer
            .borrow()
            .library_tracks()
            .iter()
            .map(|track| (track.id, track.location.is_missing()))
            .collect();
        let lookup = |id: TrackId| availability.get(&id).copied();
        songs_table.refresh_missing_flags(&lookup);
        playlists_table.refresh_missing_flags(&lookup);
    });
    runtime
        .borrow_mut()
        .set_track_availability_observer(Box::new(move || {
            // The runtime is mid-borrow when this fires — defer
            // the refresh onto the GLib main loop so the closure
            // can re-borrow the runtime read-only without panicking.
            let refresh = refresh.clone();
            glib::idle_add_local_once(move || refresh());
        }));
}

fn visible_summary_refresh_callback(
    runtime: &SharedRuntime,
    content_stack: &gtk::Stack,
    sidebar: &PlaylistSidebar,
    status_bar: &StatusBar,
    current_search_text: &Rc<RefCell<String>>,
) -> VisibleSummaryRefreshCallback {
    let runtime = runtime.clone();
    let content_stack = content_stack.clone();
    let sidebar = sidebar.clone();
    let status_bar = status_bar.clone();
    let current_search_text = current_search_text.clone();

    Rc::new(move || {
        let search_text = current_search_text.borrow().clone();
        let rows = visible_view_rows(
            &runtime.borrow(),
            &content_stack,
            sidebar.current_selection(),
            &search_text,
        );
        status_bar.update_summary(&rows);
    })
}

fn visible_view_rows(
    runtime: &ApplicationRuntime,
    content_stack: &gtk::Stack,
    sidebar_selection: Option<SidebarSelection>,
    search_text: &str,
) -> Vec<TrackTableRow> {
    if content_stack.visible_child_name().as_deref() == Some(PLAYLISTS_VIEW) {
        playlist_table_rows_for(runtime, sidebar_selection, search_text)
    } else {
        runtime_library_table_rows(runtime, search_text)
    }
}

fn sidebar_action_callback(
    parent: &gtk::ApplicationWindow,
    command_controller: &SharedCommandController,
    runtime: &SharedRuntime,
    sidebar: &PlaylistSidebar,
) -> SidebarActionCallback {
    let parent = parent.clone();
    let command_controller = command_controller.clone();
    let runtime = runtime.clone();
    let sidebar = sidebar.clone();

    Rc::new(move |action| match action {
        SidebarContextAction::Playlist => {
            create_new_playlist(&command_controller, &runtime, &sidebar);
        }
        SidebarContextAction::PlaylistFolder => {
            let existing_names: Vec<String> = runtime
                .borrow()
                .playlist_folders()
                .iter()
                .map(|folder| folder.name.clone())
                .collect();
            let name = unique_default_name(existing_names, NEW_PLAYLIST_FOLDER_DEFAULT_NAME);
            if command_controller.dispatch_succeeded(ApplicationCommand::CreatePlaylistFolder {
                name,
                parent_folder_id: None,
            }) {
                sidebar.refresh();
            }
        }
        SidebarContextAction::SmartPlaylist => {
            open_new_smart_playlist_editor(&parent, command_controller.clone(), &runtime, &sidebar);
        }
    })
}

fn sidebar_rename_callback(
    command_controller: &SharedCommandController,
    runtime: &SharedRuntime,
    sidebar: &PlaylistSidebar,
) -> super::sidebar::SidebarRenameCallback {
    let command_controller = command_controller.clone();
    let runtime = runtime.clone();
    let sidebar = sidebar.clone();

    Rc::new(move |item, new_name| {
        let dispatched = match item {
            PlaylistItem::Playlist(playlist_id) => {
                command_controller.dispatch_succeeded(ApplicationCommand::RenamePlaylist {
                    playlist_id,
                    name: new_name,
                })
            }
            PlaylistItem::Folder(folder_id) => {
                command_controller.dispatch_succeeded(ApplicationCommand::RenamePlaylistFolder {
                    folder_id,
                    name: new_name,
                })
            }
            PlaylistItem::SmartPlaylist(smart_playlist_id) => {
                let Some(rules) = runtime
                    .borrow()
                    .smart_playlists()
                    .iter()
                    .find(|smart| smart.id == smart_playlist_id)
                    .map(|smart| smart.rules.clone())
                else {
                    return;
                };
                command_controller.dispatch_succeeded(ApplicationCommand::UpdateSmartPlaylist {
                    smart_playlist_id,
                    name: new_name,
                    rules,
                })
            }
        };
        if dispatched {
            sidebar.refresh();
        }
    })
}

fn sidebar_edit_smart_playlist_callback(
    parent: &gtk::ApplicationWindow,
    command_controller: &SharedCommandController,
    runtime: &SharedRuntime,
    sidebar: &PlaylistSidebar,
) -> super::sidebar::SidebarEditSmartPlaylistCallback {
    let parent = parent.clone();
    let command_controller = command_controller.clone();
    let runtime = runtime.clone();
    let sidebar = sidebar.clone();

    Rc::new(move |smart_playlist_id| {
        let snapshot = runtime
            .borrow()
            .smart_playlists()
            .iter()
            .find(|smart| smart.id == smart_playlist_id)
            .map(|smart| (smart.name.clone(), smart.rules.clone()));
        let Some((name, rules)) = snapshot else {
            return;
        };
        let sidebar_for_saved = sidebar.clone();
        open_smart_playlist_editor(
            &parent,
            command_controller.clone(),
            Rc::new(move || sidebar_for_saved.refresh()),
            SmartPlaylistEditorMode::Edit {
                smart_playlist_id,
                name,
                rules,
            },
        );
    })
}

fn sidebar_delete_callback(
    command_controller: &SharedCommandController,
    sidebar: &PlaylistSidebar,
) -> super::sidebar::SidebarDeleteCallback {
    let command_controller = command_controller.clone();
    let sidebar = sidebar.clone();

    Rc::new(move |item| {
        let dispatched = match item {
            PlaylistItem::Playlist(playlist_id) => command_controller
                .dispatch_succeeded(ApplicationCommand::DeletePlaylist { playlist_id }),
            PlaylistItem::Folder(folder_id) => command_controller
                .dispatch_succeeded(ApplicationCommand::DeletePlaylistFolder { folder_id }),
            PlaylistItem::SmartPlaylist(smart_playlist_id) => command_controller
                .dispatch_succeeded(ApplicationCommand::DeleteSmartPlaylist { smart_playlist_id }),
        };
        if dispatched {
            sidebar.refresh();
        }
    })
}

fn sidebar_analysis_run_callback(
    runtime: &SharedRuntime,
) -> super::sidebar::SidebarAnalysisRunCallback {
    let runtime = runtime.clone();
    Rc::new(move |item, request| {
        // The runtime decides accept vs deny (based on the global
        // setting) and pushes the matching ephemeral notification; we
        // don't act on the return value.
        let _ = runtime
            .borrow_mut()
            .request_playlist_analysis_run(item, request);
    })
}

fn sidebar_online_run_callback(
    runtime: &SharedRuntime,
) -> super::sidebar::SidebarOnlineRunCallback {
    let runtime = runtime.clone();
    Rc::new(move |item, request| {
        let _ = runtime
            .borrow_mut()
            .request_playlist_online_run(item, request);
    })
}

fn sidebar_analysis_enabled_query(
    runtime: &SharedRuntime,
) -> super::sidebar::SidebarAnalysisEnabledQuery {
    let runtime = runtime.clone();
    Rc::new(move |capability| analysis_capability_enabled(&runtime, capability))
}

fn sidebar_online_enabled_query(
    runtime: &SharedRuntime,
) -> super::sidebar::SidebarOnlineEnabledQuery {
    let runtime = runtime.clone();
    Rc::new(move |capability| online_capability_enabled(&runtime, capability))
}

fn track_analyze_run_callback(runtime: &SharedRuntime) -> TrackAnalyzeRunCallback {
    let runtime = runtime.clone();
    Rc::new(move |track_ids, request| {
        let _ = runtime
            .borrow_mut()
            .request_tracks_analysis_run(track_ids, request);
    })
}

fn track_retrieve_run_callback(runtime: &SharedRuntime) -> TrackRetrieveRunCallback {
    let runtime = runtime.clone();
    Rc::new(move |track_ids, request| {
        let _ = runtime
            .borrow_mut()
            .request_tracks_online_run(track_ids, request);
    })
}

fn analysis_enabled_query(runtime: &SharedRuntime) -> TrackAnalyzeEnabledQuery {
    let runtime = runtime.clone();
    Rc::new(move |capability| analysis_capability_enabled(&runtime, capability))
}

fn online_enabled_query(runtime: &SharedRuntime) -> TrackRetrieveEnabledQuery {
    let runtime = runtime.clone();
    Rc::new(move |capability| online_capability_enabled(&runtime, capability))
}

/// Read the global analysis-capability toggle from the live settings.
/// Shared by the sidebar's per-playlist submenu and the track-table's
/// per-track submenu so both see the exact same "is the background
/// sweep covering this capability?" answer.
fn analysis_capability_enabled(
    runtime: &SharedRuntime,
    capability: sustain_app_runtime::AnalysisCapability,
) -> bool {
    let runtime = runtime.borrow();
    let analysis = runtime.settings().analysis;
    match capability {
        sustain_app_runtime::AnalysisCapability::Bpm => analysis.bpm,
        sustain_app_runtime::AnalysisCapability::Key => analysis.key,
        sustain_app_runtime::AnalysisCapability::Waveform => analysis.waveform,
    }
}

fn online_capability_enabled(
    runtime: &SharedRuntime,
    capability: sustain_app_runtime::OnlineCapability,
) -> bool {
    let runtime = runtime.borrow();
    let online = runtime.settings().online;
    match capability {
        sustain_app_runtime::OnlineCapability::Lyrics => online.lyrics,
        sustain_app_runtime::OnlineCapability::Artwork => online.artwork,
        sustain_app_runtime::OnlineCapability::Tags => online.tags,
    }
}

fn sidebar_move_callback(
    command_controller: &SharedCommandController,
    runtime: &SharedRuntime,
    sidebar: &PlaylistSidebar,
) -> super::sidebar::SidebarMoveCallback {
    let command_controller = command_controller.clone();
    let runtime = runtime.clone();
    let sidebar = sidebar.clone();

    Rc::new(move |source, target, position| {
        let Some((target_parent_folder_id, target_position)) =
            resolve_move_target(&runtime.borrow(), source, target, position)
        else {
            return;
        };
        if command_controller.dispatch_succeeded(ApplicationCommand::MovePlaylistItem {
            item: source,
            target_parent_folder_id,
            position: target_position,
        }) {
            sidebar.refresh();
        }
    })
}

fn resolve_move_target(
    runtime: &ApplicationRuntime,
    source: PlaylistItem,
    target: PlaylistItem,
    drop_position: super::sidebar::DropPosition,
) -> Option<(Option<sustain_app_runtime::PlaylistFolderId>, u32)> {
    use super::sidebar::DropPosition;
    if source == target {
        return None;
    }
    let (target_parent, target_index) = match target {
        PlaylistItem::Folder(folder_id) => {
            if matches!(drop_position, DropPosition::Into) {
                let child_count = runtime
                    .playlist_folders()
                    .iter()
                    .filter(|folder| folder.parent_folder_id == Some(folder_id))
                    .count()
                    + runtime
                        .playlists()
                        .iter()
                        .filter(|playlist| playlist.parent_folder_id == Some(folder_id))
                        .count()
                    + runtime
                        .smart_playlists()
                        .iter()
                        .filter(|smart| smart.parent_folder_id == Some(folder_id))
                        .count();
                return Some((Some(folder_id), child_count as u32));
            }
            let folder = runtime
                .playlist_folders()
                .iter()
                .find(|folder| folder.id == folder_id)?;
            (folder.parent_folder_id, folder.position)
        }
        PlaylistItem::Playlist(target_id) => {
            let playlist = runtime
                .playlists()
                .iter()
                .find(|playlist| playlist.id == target_id)?;
            (playlist.parent_folder_id, playlist.position)
        }
        PlaylistItem::SmartPlaylist(target_id) => {
            let smart = runtime
                .smart_playlists()
                .iter()
                .find(|smart| smart.id == target_id)?;
            (smart.parent_folder_id, smart.position)
        }
    };

    let position = match drop_position {
        DropPosition::Above => target_index,
        DropPosition::Below => target_index.saturating_add(1),
        DropPosition::Into => target_index,
    };
    Some((target_parent, position))
}

#[allow(clippy::too_many_arguments)]
fn sidebar_selection_changed_callback(
    runtime: &SharedRuntime,
    playlists_table: &TrackTable,
    playlists_header: &PlaylistsHeader,
    content_stack: &gtk::Stack,
    playlists_dirty: &Rc<Cell<bool>>,
    visible_summary_refresh: VisibleSummaryRefreshCallback,
    current_search_text: &Rc<RefCell<String>>,
) -> super::sidebar::SidebarSelectionChangedCallback {
    let runtime = runtime.clone();
    let playlists_table = playlists_table.clone();
    let playlists_header = playlists_header.clone();
    let content_stack = content_stack.clone();
    let playlists_dirty = playlists_dirty.clone();
    let current_search_text = current_search_text.clone();

    Rc::new(move |selection| {
        // Layout + default sort are cheap and harmless even when the
        // playlists view is not visible — they only set widget state
        // that any future visit will rely on.
        if let Some(layout) = layout_for_selection(&runtime.borrow(), selection) {
            playlists_table.apply_layout(&layout);
        }
        if matches!(
            selection,
            Some(SidebarSelection::Item(PlaylistItem::Playlist(_)))
        ) {
            playlists_table.apply_playlist_default_sort();
        }
        // The sidebar selection is the sole driver of the content
        // stack: Music → SONGS_VIEW, Albums → ALBUMS_VIEW, a playlist
        // item → PLAYLISTS_VIEW. A null selection means nothing is
        // active and we fall back to the cheap Songs page.
        let target = match selection {
            Some(SidebarSelection::Music) | None => SONGS_VIEW,
            Some(SidebarSelection::Albums) => ALBUMS_VIEW,
            Some(SidebarSelection::Item(_)) => PLAYLISTS_VIEW,
        };
        if content_stack.visible_child_name().as_deref() != Some(target) {
            content_stack.set_visible_child_name(target);
        }
        let search_text = current_search_text.borrow().clone();
        refresh_playlists_view_if_visible(
            &runtime.borrow(),
            &content_stack,
            &playlists_table,
            &playlists_header,
            selection,
            &search_text,
            &playlists_dirty,
        );
        visible_summary_refresh();
    })
}

/// Wires the topbar SearchEntry to a debounced callback that re-filters
/// all three content pages (Music, Albums, Playlists) plus the
/// status-bar summary against the new query. All three are rebuilt on
/// each fire so that switching pages mid-query never shows stale
/// unfiltered content.
///
/// Filtering follows the agreed product semantics:
/// - Music view filters across the 7 track-level fields covered by
///   [`track_matches_search_text`].
/// - Albums view filters by album-level fields only (title, artist,
///   year) via [`AlbumsView::set_search_text`].
/// - Playlist view filters within the currently selected playlist /
///   smart playlist, again on track fields.
///
/// Debouncing: rebuilding the visible track table on every keystroke is
/// expensive — not because of the in-memory filter (microseconds) but
/// because [`TrackTable::replace_rows`] rewrites the underlying
/// `gio::ListStore`, which fires GTK list-model events that the sorter
/// and selection model both have to process. The same effect shows up
/// on the album grid. We therefore cancel any in-flight rebuild and
/// schedule a fresh one [`SEARCH_REBUILD_DEBOUNCE`] in the future on
/// every keystroke, collapsing a typing burst into one rebuild when
/// the user pauses. The raw SearchEntry text is saved on close as part of
/// the UI session, so closing inside the debounce window preserves the query
/// even if the last rebuild never runs.
struct SearchWiringContext {
    current_search_text: Rc<RefCell<String>>,
    runtime: SharedRuntime,
    songs_table: TrackTable,
    albums_view: AlbumsView,
    playlists_table: TrackTable,
    playlists_header: PlaylistsHeader,
    sidebar: PlaylistSidebar,
    content_stack: gtk::Stack,
    playlists_dirty: Rc<Cell<bool>>,
    visible_summary_refresh: VisibleSummaryRefreshCallback,
}

fn install_search_wiring(titlebar: &Titlebar, context: SearchWiringContext) {
    let SearchWiringContext {
        current_search_text,
        runtime,
        songs_table,
        albums_view,
        playlists_table,
        playlists_header,
        sidebar,
        content_stack,
        playlists_dirty,
        visible_summary_refresh,
    } = context;
    let pending_rebuild: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));

    connect_titlebar_search(
        titlebar,
        Rc::new(move |new_text| {
            if *current_search_text.borrow() == new_text {
                return;
            }
            *current_search_text.borrow_mut() = new_text.clone();

            // Cancel any pending rebuild scheduled for the previous
            // keystroke; only the most recent query should run.
            if let Some(previous) = pending_rebuild.borrow_mut().take() {
                previous.remove();
            }

            let runtime = runtime.clone();
            let songs_table = songs_table.clone();
            let albums_view = albums_view.clone();
            let playlists_table = playlists_table.clone();
            let playlists_header = playlists_header.clone();
            let sidebar = sidebar.clone();
            let content_stack = content_stack.clone();
            let playlists_dirty = playlists_dirty.clone();
            let visible_summary_refresh = visible_summary_refresh.clone();
            let pending_rebuild_clear = pending_rebuild.clone();
            let source_id = glib::timeout_add_local_once(SEARCH_REBUILD_DEBOUNCE, move || {
                pending_rebuild_clear.borrow_mut().take();

                let songs_rows = runtime_library_table_rows(&runtime.borrow(), &new_text);
                songs_table.replace_rows(songs_rows);

                albums_view.set_search_text(new_text.clone());

                refresh_playlists_view_if_visible(
                    &runtime.borrow(),
                    &content_stack,
                    &playlists_table,
                    &playlists_header,
                    sidebar.current_selection(),
                    &new_text,
                    &playlists_dirty,
                );

                visible_summary_refresh();
            });
            *pending_rebuild.borrow_mut() = Some(source_id);
        }),
    );
}

/// Debounce window for search-driven view rebuilds. 100ms is short enough
/// that a single keystroke followed by a pause feels instantaneous, and
/// long enough to swallow a burst of typing at any realistic speed
/// (40ms per keystroke at 25 WPM, ~20ms at very fast typing) into one
/// rebuild when the user stops.
const SEARCH_REBUILD_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(100);

/// Wires the persisted-layout machinery for both track tables.
///
/// - The Songs view always writes to the [`Default`] scope — it *is* the
///   "general song list view" the user asked for.
/// - The Playlists view writes to a per-playlist override only when a real
///   playlist or smart playlist is selected. Library / Folder / empty
///   selections are transient and never produce override rows (matches the
///   "user owns their changes; we don't fabricate them" semantics).
/// - The Songs view's initial layout is applied here. The Playlists view's
///   initial layout is applied by the synthetic first call that
///   [`PlaylistSidebar::set_selection_changed`] makes on its handler.
fn install_track_column_layout_persistence(
    runtime: &SharedRuntime,
    songs_table: &TrackTable,
    playlists_table: &TrackTable,
    sidebar: &PlaylistSidebar,
) {
    let runtime_for_songs = runtime.clone();
    songs_table.set_layout_changed_callback(Rc::new(move |layout| {
        let _ = runtime_for_songs
            .borrow()
            .save_track_column_layout(TrackColumnLayoutScope::Default, &layout);
    }));

    let runtime_for_playlists = runtime.clone();
    let sidebar_for_playlists = sidebar.clone();
    playlists_table.set_layout_changed_callback(Rc::new(move |layout| {
        let scope = match sidebar_for_playlists.current_selection() {
            Some(SidebarSelection::Item(PlaylistItem::Playlist(id))) => {
                TrackColumnLayoutScope::Playlist(id)
            }
            Some(SidebarSelection::Item(PlaylistItem::SmartPlaylist(id))) => {
                TrackColumnLayoutScope::SmartPlaylist(id)
            }
            _ => return,
        };
        let _ = runtime_for_playlists
            .borrow()
            .save_track_column_layout(scope, &layout);
    }));

    if let Ok(Some(default)) = runtime
        .borrow()
        .load_track_column_layout(TrackColumnLayoutScope::Default)
    {
        songs_table.apply_layout(&default);
    }
}

fn layout_for_selection(
    runtime: &ApplicationRuntime,
    selection: Option<SidebarSelection>,
) -> Option<TrackColumnLayout> {
    let override_scope = match selection {
        Some(SidebarSelection::Item(PlaylistItem::Playlist(id))) => {
            Some(TrackColumnLayoutScope::Playlist(id))
        }
        Some(SidebarSelection::Item(PlaylistItem::SmartPlaylist(id))) => {
            Some(TrackColumnLayoutScope::SmartPlaylist(id))
        }
        _ => None,
    };
    if let Some(scope) = override_scope {
        if let Ok(Some(layout)) = runtime.load_track_column_layout(scope) {
            return Some(layout);
        }
    }
    runtime
        .load_track_column_layout(TrackColumnLayoutScope::Default)
        .ok()
        .flatten()
}

fn runtime_library_table_rows(
    runtime: &ApplicationRuntime,
    search_text: &str,
) -> Vec<TrackTableRow> {
    runtime
        .library_tracks()
        .iter()
        .filter(|track| search_text.is_empty() || track_matches_search_text(track, search_text))
        .map(TrackTableRow::from_track)
        .collect()
}

fn playlist_table_rows_for(
    runtime: &ApplicationRuntime,
    selection: Option<SidebarSelection>,
    search_text: &str,
) -> Vec<TrackTableRow> {
    // Carry the playlist_position alongside each Track so the row built
    // below mirrors PlaylistEntry::position one-to-one for the regular-
    // playlist branch. Library / Smart Playlist selections never have an
    // authoritative play-order, so their pairs hold None — those rows
    // collate equal under the status column sorter and are unaffected by
    // the play-order sort.
    let candidates: Vec<(Track, Option<u32>)> = match selection {
        // The playlists table mirrors the Music view's rows when the
        // Music entry is selected — same library track set, no
        // play-position. (PLAYLISTS_VIEW is not actually shown for
        // Music / Albums, but the table-rebuild path is shared.)
        Some(SidebarSelection::Music) => runtime
            .library_tracks()
            .iter()
            .map(|track| (track.clone(), None))
            .collect(),
        Some(SidebarSelection::Item(PlaylistItem::Playlist(playlist_id))) => {
            let Some(playlist) = runtime
                .playlists()
                .iter()
                .find(|playlist| playlist.id == playlist_id)
            else {
                return Vec::new();
            };
            let tracks_by_id: HashMap<TrackId, &Track> = runtime
                .library_tracks()
                .iter()
                .map(|track| (track.id, track))
                .collect();
            let mut entries: Vec<&PlaylistEntry> = playlist.entries.iter().collect();
            entries.sort_by_key(|entry| entry.position);
            entries
                .into_iter()
                .filter_map(|entry| {
                    tracks_by_id
                        .get(&entry.track_id)
                        .copied()
                        .cloned()
                        .map(|track| (track, Some(entry.position)))
                })
                .collect()
        }
        Some(SidebarSelection::Item(PlaylistItem::SmartPlaylist(smart_playlist_id))) => runtime
            .smart_playlist_matching_tracks(smart_playlist_id)
            .into_iter()
            .map(|track| (track.clone(), None))
            .collect(),
        _ => return Vec::new(),
    };

    candidates
        .into_iter()
        .filter(|(track, _)| {
            search_text.is_empty() || track_matches_search_text(track, search_text)
        })
        .map(|(track, position)| TrackTableRow::from_track(&track).with_playlist_position(position))
        .collect()
}

fn sidebar_tracks_drop_callback(
    command_controller: &SharedCommandController,
    library_changed_holder: &LibraryChangedHolder,
) -> super::sidebar::SidebarTracksDropCallback {
    let command_controller = command_controller.clone();
    let library_changed_holder = library_changed_holder.clone();

    Rc::new(move |target, track_ids| {
        let PlaylistItem::Playlist(playlist_id) = target else {
            return;
        };
        if track_ids.is_empty() {
            return;
        }
        let dispatched =
            command_controller.dispatch_succeeded(ApplicationCommand::AddTracksToPlaylist {
                playlist_id,
                track_ids,
            });
        if !dispatched {
            return;
        }
        if let Some(callback) = library_changed_holder.borrow().as_ref() {
            callback();
        }
    })
}

fn add_to_playlist_provider(runtime: &SharedRuntime) -> AddToPlaylistProvider {
    let runtime = runtime.clone();
    Rc::new(move || {
        let runtime = runtime.borrow();
        let folders: HashMap<PlaylistFolderId, &PlaylistFolder> = runtime
            .playlist_folders()
            .iter()
            .map(|folder| (folder.id, folder))
            .collect();
        let mut entries: Vec<AddToPlaylistEntry> = runtime
            .playlists()
            .iter()
            .map(|playlist| AddToPlaylistEntry {
                playlist_id: playlist.id,
                display_path: playlist_display_path(playlist, &folders),
            })
            .collect();
        entries.sort_by(|left, right| {
            left.display_path
                .to_lowercase()
                .cmp(&right.display_path.to_lowercase())
        });
        entries
    })
}

fn add_to_playlist_callback(
    command_controller: &SharedCommandController,
    runtime: &SharedRuntime,
    library_changed_holder: &LibraryChangedHolder,
) -> AddToPlaylistCallback {
    let command_controller = command_controller.clone();
    let runtime = runtime.clone();
    let library_changed_holder = library_changed_holder.clone();

    Rc::new(move |playlist_id, track_ids| {
        if track_ids.is_empty() {
            return;
        }
        let dispatched =
            command_controller.dispatch_succeeded(ApplicationCommand::AddTracksToPlaylist {
                playlist_id,
                track_ids,
            });
        if !dispatched {
            return;
        }
        // Library state itself is unchanged, but the currently-displayed
        // playlist may now be longer — re-fire library_changed so the table
        // and sidebar refresh.
        let _ = runtime.borrow();
        if let Some(callback) = library_changed_holder.borrow().as_ref() {
            callback();
        }
    })
}

fn playlist_display_path(
    playlist: &Playlist,
    folders: &HashMap<PlaylistFolderId, &PlaylistFolder>,
) -> String {
    let mut segments: Vec<String> = Vec::new();
    let mut current = playlist.parent_folder_id;
    while let Some(folder_id) = current {
        let Some(folder) = folders.get(&folder_id) else {
            break;
        };
        segments.push(folder.name.clone());
        current = folder.parent_folder_id;
    }
    segments.reverse();
    segments.push(playlist.name.clone());
    segments.join(" / ")
}

fn playback_changed_callback(
    runtime: &SharedRuntime,
    now_playing: &NowPlayingView,
    titlebar: &Titlebar,
    songs_table_holder: Rc<RefCell<Option<TrackTable>>>,
    albums_view_holder: Rc<RefCell<Option<AlbumsView>>>,
    playlists_table_holder: Rc<RefCell<Option<TrackTable>>>,
    mpris_service: Option<SharedMprisService>,
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
        if let Some(albums_view) = albums_view_holder.borrow().as_ref() {
            albums_view.set_playing_track_id(playing_track_id);
        }
        if let Some(playlists_table) = playlists_table_holder.borrow().as_ref() {
            playlists_table.set_playing_track_id(playing_track_id);
        }
        now_playing.refresh(&now_playing_state);
        if let Some(service) = mpris_service.as_deref() {
            service.publish_playback_state(now_playing_state.state.clone());
            service.publish_now_playing(now_playing_to_mpris_metadata(&now_playing_state));
        }
    })
}

fn now_playing_to_mpris_metadata(
    now_playing: &sustain_app_runtime::NowPlaying,
) -> sustain_desktop::NowPlayingMetadata {
    let Some(track) = now_playing.track.as_ref() else {
        return sustain_desktop::NowPlayingMetadata::default();
    };
    sustain_desktop::NowPlayingMetadata {
        track_id: Some(track.id),
        title: track.metadata.title.clone(),
        artist: track.metadata.artist.clone(),
        album: track.metadata.album.clone(),
        album_artist: track.metadata.album_artist.clone(),
        genre: track.metadata.genre.clone(),
        track_number: track.metadata.track_number,
        disc_number: track.metadata.disc_number,
        duration: track.metadata.duration,
    }
}

fn install_mpris_command_consumer(
    receiver: Option<MprisCommandReceiver>,
    command_controller: SharedCommandController,
    playback_changed: PlaybackChangedCallback,
    app: gtk::Application,
    window: gtk::ApplicationWindow,
) {
    // No receiver means MPRIS startup failed; the UI just runs without
    // a media-key bridge, and the dropped sender on the desktop side
    // means future try_send calls would silently no-op anyway.
    let Some(receiver) = receiver else {
        return;
    };
    glib::MainContext::default().spawn_local(async move {
        while let Ok(command) = receiver.recv().await {
            handle_mpris_command(
                command,
                &command_controller,
                &playback_changed,
                &app,
                &window,
            );
        }
    });
}

/// Drains [`sustain_app_runtime::MetadataWriteResult`]s posted by the async metadata writer
/// and surfaces failures to the user.
///
/// The runtime applies the optimistic in-memory + SQLite update
/// synchronously and returns immediately; the disk-side tag write
/// completes later on the worker thread. When it fails (read-only file,
/// permission denied, file gone), we post a status-bar message and
/// re-refresh the affected row so any state derived from disk (e.g. the
/// missing icon, if a follow-up stage starts marking missing on touch
/// failure) becomes visible. We do not roll back the in-memory state in
/// this stage — that is a separate, careful piece of work; the next
/// library scan reconciles the SQLite cache against what is actually on
/// disk.
fn install_metadata_write_result_consumer(
    receiver: Option<MetadataWriteResultReceiver>,
    runtime: SharedRuntime,
    track_row_changed_holder: TrackRowChangedHolder,
) {
    let Some(receiver) = receiver else {
        return;
    };
    glib::MainContext::default().spawn_local(async move {
        while let Ok(result) = receiver.recv().await {
            if matches!(
                result.outcome,
                sustain_app_runtime::MetadataWriteOutcome::Succeeded
            ) {
                continue;
            }
            let message = match result.kind {
                sustain_app_runtime::MetadataWriteKind::Rating => {
                    "Could not save the rating to the audio file."
                }
                sustain_app_runtime::MetadataWriteKind::Metadata => {
                    "Could not save the metadata change to the audio file."
                }
                sustain_app_runtime::MetadataWriteKind::Artwork => {
                    "Could not save the artwork change to the audio file."
                }
            };
            runtime.borrow_mut().push_ephemeral_notification(
                sustain_app_runtime::NotificationCategory::MetadataWrite,
                sustain_app_runtime::NotificationSeverity::Error,
                message.to_owned(),
            );
            if let Some(callback) = track_row_changed_holder.borrow().as_ref() {
                callback(result.track_id);
            }
        }
    });
}

/// Drains [`AnalysisProgress`](sustain_app_runtime::AnalysisProgress)
/// events posted by the background analysis scheduler. Each event is
/// applied to the runtime's notification center on the GTK main thread
/// via [`ApplicationRuntime::apply_analysis_progress`] — that's where
/// the persistent "Analyzing N/total..." notification is created,
/// updated in place per tick, and dismissed on Idle (with an
/// ephemeral summary toast when work actually happened).
fn install_analysis_progress_consumer(
    receiver: Option<AnalysisProgressReceiver>,
    runtime: SharedRuntime,
) {
    let Some(receiver) = receiver else {
        return;
    };
    glib::MainContext::default().spawn_local(async move {
        while let Ok(progress) = receiver.recv().await {
            runtime.borrow_mut().apply_analysis_progress(progress);
        }
    });
}

/// Symmetric to [`install_analysis_progress_consumer`] but for the
/// online scheduler. Each event lands in
/// [`ApplicationRuntime::apply_online_progress`] which owns the
/// matching persistent/ephemeral notification surface.
fn install_online_progress_consumer(
    receiver: Option<OnlineProgressReceiver>,
    runtime: SharedRuntime,
) {
    let Some(receiver) = receiver else {
        return;
    };
    glib::MainContext::default().spawn_local(async move {
        while let Ok(progress) = receiver.recv().await {
            runtime.borrow_mut().apply_online_progress(progress);
        }
    });
}

/// Wires the runtime's `track_data_observer` to the shared
/// per-row refresh callback. The runtime fires this observer after
/// every `apply_track_updated` — i.e. after a background worker has
/// mutated a single track in the library store. The deferred
/// closure invokes the standard row-refresh path so Songs/Albums/
/// Playlists views all repaint the touched row without rebuilding
/// the table.
fn install_track_data_observer(
    runtime: &SharedRuntime,
    track_row_changed_holder: TrackRowChangedHolder,
) {
    runtime
        .borrow_mut()
        .set_track_data_observer(Box::new(move |track_id| {
            // The runtime is mid-borrow when this fires — defer the
            // refresh onto the GLib main loop so the closure can
            // re-borrow read-only without panicking.
            let track_row_changed_holder = track_row_changed_holder.clone();
            glib::idle_add_local_once(move || {
                if let Some(callback) = track_row_changed_holder.borrow().clone() {
                    callback(track_id);
                }
            });
        }));
}

/// Drains track-id events emitted by the analysis and online
/// schedulers. Each id is fed into
/// [`ApplicationRuntime::apply_track_updated`], which reloads the
/// row from the library store and fires the
/// `track_data_observer` so the UI repaints.
fn install_track_updated_consumer(receiver: Option<TrackUpdatedReceiver>, runtime: SharedRuntime) {
    let Some(receiver) = receiver else {
        return;
    };
    glib::MainContext::default().spawn_local(async move {
        while let Ok(track_id) = receiver.recv().await {
            runtime.borrow_mut().apply_track_updated(track_id);
        }
    });
}

/// Drains
/// [`SmartShuffleRebuildResult`](sustain_app_runtime::SmartShuffleRebuildResult)s
/// posted by the background Smart Shuffle rebuild thread and feeds
/// them into
/// [`ApplicationRuntime::apply_smart_shuffle_rebuild_result`], which
/// adopts the new index in memory and persists its blob through the
/// library store. Without this drain, completed rebuilds would queue
/// forever in the `async_channel` and a freshly-rebuilt index would
/// never be picked up.
fn install_smart_shuffle_rebuild_result_consumer(
    receiver: Option<SmartShuffleRebuildResultReceiver>,
    runtime: SharedRuntime,
) {
    let Some(receiver) = receiver else {
        return;
    };
    glib::MainContext::default().spawn_local(async move {
        while let Ok(result) = receiver.recv().await {
            runtime
                .borrow_mut()
                .apply_smart_shuffle_rebuild_result(result);
        }
    });
}

/// Tick interval, in seconds, for the periodic Smart Shuffle rebuild
/// check. 60 s is the documented cadence in
/// `smart_shuffle_scheduler.rs`: fine-grained enough that the
/// "Hourly" preset triggers within a minute of becoming due, coarse
/// enough to be free at the system level (a glib timer at this
/// cadence is in the noise next to background analysis work).
const SMART_SHUFFLE_PERIODIC_TICK_SECS: u32 = 60;

/// Install the timer that drives Smart Shuffle's
/// `SmartShuffleRebuildInterval` setting. Each tick compares
/// `now() - smart_shuffle_metadata().built_at` against the configured
/// interval and requests an index rebuild when due. Runs for the
/// lifetime of the application — there is no "stop" path because the
/// timer is cheap, and the runtime guards re-entrant requests
/// internally.
///
/// Intentionally a no-op when:
///   * the interval setting is `Off`,
///   * a rebuild is already in flight (the scheduler would reject the
///     request anyway, but checking here avoids the redundant borrow),
///   * the index has never been built — the very first build is
///     triggered by an explicit "Rebuild index" click or by the
///     enable path inside `on_shuffle_mode_changed`, not by this
///     timer. Auto-rebuild is for refreshing an index that has gone
///     stale as the library grows, not for nudging a user who has not
///     yet engaged with Smart Shuffle.
fn install_smart_shuffle_periodic_rebuild(runtime: &SharedRuntime) {
    let runtime = runtime.clone();
    glib::timeout_add_seconds_local(SMART_SHUFFLE_PERIODIC_TICK_SECS, move || {
        maybe_request_smart_shuffle_rebuild(&runtime);
        glib::ControlFlow::Continue
    });
}

fn maybe_request_smart_shuffle_rebuild(runtime: &SharedRuntime) {
    // Read-only borrow asks the runtime if its policy gates are open
    // right now. The policy itself (interval, last-built, worker-busy)
    // lives next to the data in `ApplicationRuntime` so it can be
    // unit-tested through the injected clock without a GTK main loop.
    // We only re-borrow mutably to issue the request — which has its
    // own re-entrancy guard in the scheduler — so a worker that flips
    // `is_rebuilding` between the two borrows still ends up consistent.
    if !runtime.borrow().should_run_periodic_smart_shuffle_rebuild() {
        return;
    }
    runtime.borrow_mut().request_smart_shuffle_rebuild();
}

/// Drains [`ArtworkFetchResult`](sustain_app_runtime::ArtworkFetchResult)s
/// posted by the background artwork fetcher.
///
/// On a successful fetch, the cache is invalidated, the freshly-
/// decoded bytes are primed into the loader's in-memory cache (so
/// the imminent now-playing refresh paints the new cover without
/// waiting for the async tag write), and a follow-up `SetArtwork`
/// command persists the bytes through the standard tag-writing
/// path. Failure modes surface a non-modal status-bar message.
/// Every outcome clears the now-playing tile's pending-fetch state
/// and triggers a `playback_changed` refresh so the tile and every
/// downstream view (Albums grid, track-table cover columns) settles
/// on the new visual state.
struct ArtworkFetchResultConsumerContext {
    receiver: Option<ArtworkFetchResultReceiver>,
    runtime: SharedRuntime,
    command_controller: SharedCommandController,
    artwork_loader: ArtworkLoader,
    now_playing: crate::now_playing::NowPlayingView,
    playback_changed: PlaybackChangedCallback,
    track_row_changed_holder: TrackRowChangedHolder,
}

fn install_artwork_fetch_result_consumer(context: ArtworkFetchResultConsumerContext) {
    let ArtworkFetchResultConsumerContext {
        receiver,
        runtime,
        command_controller,
        artwork_loader,
        now_playing,
        playback_changed,
        track_row_changed_holder,
    } = context;
    let Some(receiver) = receiver else {
        return;
    };
    glib::MainContext::default().spawn_local(async move {
        while let Ok(result) = receiver.recv().await {
            use sustain_app_runtime::ArtworkFetchOutcome;
            let (severity, body) = match &result.outcome {
                ArtworkFetchOutcome::Fetched(bytes) => {
                    if let Some(source) = artwork_source_for_track(&runtime, result.track_id) {
                        // Drop the existing in-memory + disk-cache
                        // entry, then prime the in-memory entry with
                        // the freshly fetched bytes. The disk-cache
                        // row is left dropped: the next miss after
                        // the metadata writer lands the tag write
                        // will rebuild it from the file, with the
                        // correct post-write fingerprint.
                        artwork_loader.invalidate(&source);
                        artwork_loader.prime(source, bytes.clone());
                    }
                    let _ = command_controller.dispatch(
                        sustain_app_runtime::ApplicationCommand::SetArtwork {
                            track_id: result.track_id,
                            artwork: Some(bytes.clone()),
                        },
                    );
                    (
                        sustain_app_runtime::NotificationSeverity::Info,
                        "Artwork updated.".to_owned(),
                    )
                }
                ArtworkFetchOutcome::NoMatch => (
                    sustain_app_runtime::NotificationSeverity::Info,
                    "No cover art found for this track.".to_owned(),
                ),
                ArtworkFetchOutcome::Failed => (
                    sustain_app_runtime::NotificationSeverity::Error,
                    "Could not fetch cover art.".to_owned(),
                ),
            };
            // The corresponding "Fetching artwork…" persistent is
            // dismissed by the now-playing tile (it owns the
            // persistent id it pushed). Here we only publish the
            // outcome ephemeral.
            now_playing.notify_artwork_fetch_complete(result.track_id);
            runtime.borrow_mut().push_ephemeral_notification(
                sustain_app_runtime::NotificationCategory::ArtworkFetch,
                severity,
                body,
            );
            playback_changed();
            if let Some(callback) = track_row_changed_holder.borrow().as_ref() {
                callback(result.track_id);
            }
        }
    });
}

/// Resolve the [`ArtworkSource`](crate::artwork_loader::ArtworkSource)
/// for a track in the current library. Returns `None` when the track
/// no longer exists (removed mid-flight) or no library root is
/// configured — both safe states for the caller to treat as
/// "nothing to invalidate".
fn artwork_source_for_track(
    runtime: &SharedRuntime,
    track_id: TrackId,
) -> Option<crate::artwork_loader::ArtworkSource> {
    let runtime = runtime.borrow();
    let track = runtime
        .library_tracks()
        .iter()
        .find(|track| track.id == track_id)?;
    let absolute = runtime.absolute_track_path(track)?;
    let cache_path = track.location.path().to_path_buf();
    Some(crate::artwork_loader::ArtworkSource::embedded_track(
        cache_path, absolute,
    ))
}

fn handle_mpris_command(
    command: sustain_desktop::MprisCommand,
    command_controller: &SharedCommandController,
    playback_changed: &PlaybackChangedCallback,
    app: &gtk::Application,
    window: &gtk::ApplicationWindow,
) {
    use sustain_desktop::MprisCommand;

    // Mapping MPRIS semantics into Sustain's PlaybackCommand surface:
    //
    // * `Play` is "resume from paused/stopped"; MPRIS clients use it as a
    //   distinct verb from `Pause`/`PlayPause` for state machines that
    //   want to be explicit. Map to `Resume`, which the runtime no-ops
    //   when there is no track loaded — the equivalent of MPRIS's
    //   "Play has no effect" clause for an empty queue.
    // * `Stop` is a hard stop with position reset; map to the existing
    //   `Stop` command.
    // * `Raise` / `Quit` are not playback at all; they are routed to GTK
    //   window/application actions directly.
    let playback_command = match command {
        MprisCommand::Raise => {
            window.present();
            return;
        }
        MprisCommand::Quit => {
            app.quit();
            return;
        }
        MprisCommand::PlayPause => PlaybackCommand::TogglePlayPause,
        MprisCommand::Play => PlaybackCommand::Resume,
        MprisCommand::Pause => PlaybackCommand::Pause,
        MprisCommand::Stop => PlaybackCommand::Stop,
        // Treat the desktop-integration Next (media keys, MPRIS clients,
        // GNOME shortcuts) as a user-initiated skip, matching the
        // titlebar Next button. Only the EOS auto-advance path stays on
        // PlayNextTrack so natural track endings never inflate
        // skip_count.
        MprisCommand::Next => PlaybackCommand::SkipCurrentTrack,
        MprisCommand::Previous => PlaybackCommand::PlayPreviousTrack,
    };
    if command_controller.dispatch_succeeded(ApplicationCommand::Playback(playback_command)) {
        playback_changed();
    }
}

/// Activation handler for the Songs view: the queue is the whole library,
/// so auto-advance and Next/Previous walk all playable tracks in library
/// order. Matches the iTunes 11 "Music" library default.
fn library_track_activated_callback(
    command_controller: &SharedCommandController,
    runtime: &SharedRuntime,
    playback_changed: PlaybackChangedCallback,
    current_search_text: &Rc<RefCell<String>>,
) -> TrackActivatedCallback {
    let command_controller = command_controller.clone();
    let runtime = runtime.clone();
    let current_search_text = current_search_text.clone();

    Rc::new(move |track_id: TrackId| {
        let queue = {
            let search_text = current_search_text.borrow().clone();
            queue_request_for_library(&runtime.borrow(), &search_text)
        };
        if command_controller.dispatch_succeeded(ApplicationCommand::Playback(
            PlaybackCommand::PlayTrack { track_id, queue },
        )) {
            playback_changed();
        }
    })
}

/// Activation handler for the Playlists view: the queue is whatever the
/// sidebar currently has selected.
///
/// - Regular playlist: queue is the playlist's entries in their
///   authoritative position order, so auto-advance stays inside the
///   playlist and replays it in the user-defined sequence.
/// - Smart playlist: queue is the smart playlist's current matching
///   tracks. The runtime's `PlaybackQueueSource::Selection` is used as
///   the source label (we don't yet model a smart-playlist source
///   variant) but the play order is the smart playlist's order.
/// - Library pseudo-entry: queue is the full (search-filtered)
///   library, matching the Songs view's behavior.
///
/// Any other selection (folders, no selection) falls back to the Library
/// queue — those targets don't activate tracks in normal use, but a
/// fallback keeps playback predictable if a future code path ever
/// double-clicks a row in that state.
fn playlist_track_activated_callback(
    command_controller: &SharedCommandController,
    runtime: &SharedRuntime,
    sidebar: &PlaylistSidebar,
    playback_changed: PlaybackChangedCallback,
    current_search_text: &Rc<RefCell<String>>,
) -> TrackActivatedCallback {
    let command_controller = command_controller.clone();
    let runtime = runtime.clone();
    let sidebar = sidebar.clone();
    let current_search_text = current_search_text.clone();

    Rc::new(move |track_id: TrackId| {
        let queue = {
            let search_text = current_search_text.borrow().clone();
            let runtime_borrow = runtime.borrow();
            queue_request_for_playlist_selection(
                &runtime_borrow,
                sidebar.current_selection(),
                &search_text,
            )
        };
        if command_controller.dispatch_succeeded(ApplicationCommand::Playback(
            PlaybackCommand::PlayTrack { track_id, queue },
        )) {
            playback_changed();
        }
    })
}

fn queue_request_for_playlist_selection(
    runtime: &ApplicationRuntime,
    selection: Option<SidebarSelection>,
    search_text: &str,
) -> PlaybackQueueRequest {
    if matches!(selection, Some(SidebarSelection::Music)) {
        return queue_request_for_library(runtime, search_text);
    }

    let candidates: Vec<(Track, PlaybackQueueSource)> = match selection {
        Some(SidebarSelection::Item(PlaylistItem::Playlist(playlist_id))) => {
            let Some(playlist) = runtime
                .playlists()
                .iter()
                .find(|playlist| playlist.id == playlist_id)
            else {
                return PlaybackQueueRequest::Library;
            };
            let tracks_by_id: HashMap<TrackId, &Track> = runtime
                .library_tracks()
                .iter()
                .map(|track| (track.id, track))
                .collect();
            let mut entries: Vec<&PlaylistEntry> = playlist.entries.iter().collect();
            entries.sort_by_key(|entry| entry.position);
            let source = PlaybackQueueSource::Playlist(playlist_id);
            entries
                .into_iter()
                .filter_map(|entry| {
                    tracks_by_id
                        .get(&entry.track_id)
                        .copied()
                        .cloned()
                        .map(|track| (track, source.clone()))
                })
                .collect()
        }
        Some(SidebarSelection::Item(PlaylistItem::SmartPlaylist(smart_playlist_id))) => runtime
            .smart_playlist_matching_tracks(smart_playlist_id)
            .into_iter()
            .map(|track| (track.clone(), PlaybackQueueSource::Selection))
            .collect(),
        _ => return queue_request_for_library(runtime, search_text),
    };

    let source = match candidates.first() {
        Some((_, source)) => source.clone(),
        None => return PlaybackQueueRequest::Library,
    };
    let ordered_track_ids: Vec<TrackId> = candidates
        .into_iter()
        .filter(|(track, _)| {
            search_text.is_empty() || track_matches_search_text(track, search_text)
        })
        .map(|(track, _)| track.id)
        .collect();
    PlaybackQueueRequest::Explicit {
        source,
        ordered_track_ids,
    }
}

fn queue_request_for_library(
    runtime: &ApplicationRuntime,
    search_text: &str,
) -> PlaybackQueueRequest {
    if search_text.trim().is_empty() {
        return PlaybackQueueRequest::Library;
    }
    let ordered_track_ids = runtime
        .library_tracks()
        .iter()
        .filter(|track| track_matches_search_text(track, search_text))
        .map(|track| track.id)
        .collect();
    PlaybackQueueRequest::Explicit {
        source: PlaybackQueueSource::SearchResults,
        ordered_track_ids,
    }
}

/// Wires the Playlists header's play and shuffle buttons to start
/// playback from the sidebar's current selection, matching the album
/// detail header's play/shuffle behaviour: the first non-missing track
/// in the queue's display order is the one PlayTrack anchors on; the
/// shuffle toggle decides what comes after.
fn install_playlists_header_playback(
    playlists_header: &PlaylistsHeader,
    command_controller: &SharedCommandController,
    runtime: &SharedRuntime,
    sidebar: &PlaylistSidebar,
    current_search_text: &Rc<RefCell<String>>,
    playback_changed: &PlaybackChangedCallback,
) {
    playlists_header.connect_play(make_playlists_header_play_callback(
        false,
        command_controller,
        runtime,
        sidebar,
        current_search_text,
        playback_changed,
    ));
    playlists_header.connect_shuffle(make_playlists_header_play_callback(
        true,
        command_controller,
        runtime,
        sidebar,
        current_search_text,
        playback_changed,
    ));
}

fn make_playlists_header_play_callback(
    shuffle: bool,
    command_controller: &SharedCommandController,
    runtime: &SharedRuntime,
    sidebar: &PlaylistSidebar,
    current_search_text: &Rc<RefCell<String>>,
    playback_changed: &PlaybackChangedCallback,
) -> Rc<dyn Fn()> {
    let command_controller = command_controller.clone();
    let runtime = runtime.clone();
    let sidebar = sidebar.clone();
    let current_search_text = current_search_text.clone();
    let playback_changed = playback_changed.clone();
    Rc::new(move || {
        // Set shuffle first so subsequent `PlayTrack` builds its queue
        // with the right ordering. Both dispatches are independent —
        // the runtime does not coalesce them. The playlist header's
        // Shuffle button pins the queue to Pure random regardless of
        // the transport setting — Smart's library-wide signals are
        // not the right fit for "shuffle this playlist's tracks".
        let shuffle_mode = if shuffle {
            ShuffleMode::Pure
        } else {
            ShuffleMode::Off
        };
        let _ = command_controller.dispatch(ApplicationCommand::Playback(
            PlaybackCommand::SetShuffleMode(shuffle_mode),
        ));
        let (queue, first_track) = {
            let runtime_borrow = runtime.borrow();
            let search_text = current_search_text.borrow().clone();
            let queue = queue_request_for_playlist_selection(
                &runtime_borrow,
                sidebar.current_selection(),
                &search_text,
            );
            let first_track = first_playable_track_for_queue(&runtime_borrow, &queue);
            (queue, first_track)
        };
        let Some(track_id) = first_track else {
            return;
        };
        if command_controller.dispatch_succeeded(ApplicationCommand::Playback(
            PlaybackCommand::PlayTrack { track_id, queue },
        )) {
            playback_changed();
        }
    })
}

fn first_playable_track_for_queue(
    runtime: &ApplicationRuntime,
    queue: &PlaybackQueueRequest,
) -> Option<TrackId> {
    let library = runtime.library_tracks();
    match queue {
        PlaybackQueueRequest::Library => library
            .iter()
            .find(|track| !track.location.is_missing())
            .map(|track| track.id),
        PlaybackQueueRequest::Explicit {
            ordered_track_ids, ..
        } => {
            let missing: HashMap<TrackId, bool> = library
                .iter()
                .map(|track| (track.id, track.location.is_missing()))
                .collect();
            ordered_track_ids
                .iter()
                .copied()
                .find(|id| matches!(missing.get(id), Some(false)))
        }
    }
}

fn install_track_ended_callback(
    runtime: &SharedRuntime,
    command_controller: &SharedCommandController,
    playback_changed: &PlaybackChangedCallback,
) {
    let command_controller = command_controller.clone();
    let playback_changed = playback_changed.clone();
    // The bus watch fires from glib's main context, the same thread that
    // services GTK events. Dispatching PlayNextTrack therefore happens at a
    // quiescent point, so no other borrow of the runtime can be in flight.
    runtime.borrow().set_track_ended_callback(Box::new(move || {
        if command_controller
            .dispatch_succeeded(ApplicationCommand::Playback(PlaybackCommand::PlayNextTrack))
        {
            playback_changed();
        }
    }));
}

fn rating_changed_callback(
    command_controller: &SharedCommandController,
    track_row_changed_holder: TrackRowChangedHolder,
) -> RatingChangedCallback {
    let command_controller = command_controller.clone();

    Rc::new(move |track_id: TrackId, rating: Rating| {
        if !command_controller
            .dispatch_succeeded(ApplicationCommand::SetRating { track_id, rating })
        {
            return false;
        }
        if let Some(callback) = track_row_changed_holder.borrow().as_ref() {
            callback(track_id);
        }
        true
    })
}

/// Targeted refresh path for single-track mutations (rating, play count).
/// Updates only the affected row in the visible tables, refreshes the
/// AlbumsView model without touching the Songs table's store, and refreshes
/// the status-bar summary. Skips the sidebar tree because row-field mutations
/// do not alter playlist/folder structure.
///
/// When a smart playlist is selected, the Playlists table falls back to a
/// full reflow because the mutation may add/remove the track from the
/// playlist's filtered set — an in-place row update would lie.
struct TrackRowChangedContext<'a> {
    runtime: &'a SharedRuntime,
    songs_table: &'a TrackTable,
    albums_view: &'a AlbumsView,
    playlists_table: &'a TrackTable,
    playlists_header: &'a PlaylistsHeader,
    sidebar: &'a PlaylistSidebar,
    content_stack: &'a gtk::Stack,
    playlists_dirty: &'a Rc<Cell<bool>>,
    visible_summary_refresh: VisibleSummaryRefreshCallback,
    current_search_text: &'a Rc<RefCell<String>>,
}

fn track_row_changed_callback(ctx: TrackRowChangedContext<'_>) -> TrackRowChangedCallback {
    let runtime = ctx.runtime.clone();
    let songs_table = ctx.songs_table.clone();
    let albums_view = ctx.albums_view.clone();
    let playlists_table = ctx.playlists_table.clone();
    let playlists_header = ctx.playlists_header.clone();
    let sidebar = ctx.sidebar.clone();
    let content_stack = ctx.content_stack.clone();
    let playlists_dirty = ctx.playlists_dirty.clone();
    let current_search_text = ctx.current_search_text.clone();
    let visible_summary_refresh = ctx.visible_summary_refresh;

    Rc::new(move |track_id: TrackId| {
        let row = {
            let runtime_borrow = runtime.borrow();
            runtime_borrow
                .library_tracks()
                .iter()
                .find(|track| track.id == track_id)
                .map(TrackTableRow::from_track)
        };
        let Some(row) = row else {
            return;
        };

        songs_table.update_row(track_id, row.clone());
        // In-place per-track refresh — never `replace_tracks`. A single
        // background completion (Lyrics/Tags/Artwork/BPM/Key/Waveform,
        // metadata write, rating change) must not collapse the
        // currently-expanded album or scroll the grid back to the top.
        albums_view.update_track(track_id);

        match sidebar.current_selection() {
            Some(SidebarSelection::Item(PlaylistItem::SmartPlaylist(smart_id))) => {
                // Smart-playlist *membership* may change with the
                // edit — but in the overwhelmingly common case
                // (BPM/key/waveform scan updating a track that
                // either already matches or already doesn't) the
                // set is unchanged and only the row's data needs to
                // repaint. Use the runtime's per-track status check
                // to tell the two apart and avoid the
                // `replace_rows` that would scroll the user back to
                // the top of a large library on every track update.
                let (status, was_in_table) = {
                    let runtime_borrow = runtime.borrow();
                    let status = runtime_borrow.smart_playlist_track_status(smart_id, track_id);
                    let was_in_table = playlists_table.contains_track(track_id);
                    (status, was_in_table)
                };
                let membership_changed = matches!(
                    (status, was_in_table),
                    (SmartPlaylistTrackStatus::Included, false)
                        | (SmartPlaylistTrackStatus::Excluded, true)
                        | (SmartPlaylistTrackStatus::RequiresFullRebuild, _)
                );
                if membership_changed {
                    let search_text = current_search_text.borrow().clone();
                    refresh_playlists_view_if_visible(
                        &runtime.borrow(),
                        &content_stack,
                        &playlists_table,
                        &playlists_header,
                        sidebar.current_selection(),
                        &search_text,
                        &playlists_dirty,
                    );
                } else if was_in_table {
                    // Membership unchanged and the row is visible —
                    // refresh the row's data in place.
                    playlists_table.update_row(track_id, row);
                }
                // else: track doesn't match the smart playlist and
                // isn't on screen anyway; no work needed.
            }
            _ => {
                // In-place row update is cheap (one row) and idempotent
                // for a hidden table; no visibility gating needed.
                playlists_table.update_row(track_id, row);
            }
        }

        visible_summary_refresh();
    })
}

fn track_context_actions(
    runtime: &SharedRuntime,
    window: &gtk::Window,
    show_album_holder: &ShowAlbumHolder,
    command_controller: &SharedCommandController,
    playback_changed: PlaybackChangedCallback,
    library_changed_holder: LibraryChangedHolder,
    track_row_changed_holder: TrackRowChangedHolder,
) -> TrackContextActionSet {
    TrackContextActionSet::new(vec![
        TrackContextAction::play_next(
            play_next_callback(command_controller),
            playback_has_current_track_visibility(runtime),
        ),
        TrackContextAction::add_to_queue(
            add_to_queue_callback(command_controller),
            playback_has_current_track_visibility(runtime),
        ),
        TrackContextAction::get_info(get_info_callback(
            window,
            runtime,
            command_controller,
            &library_changed_holder,
            &track_row_changed_holder,
        )),
        TrackContextAction::show_album(
            show_album_callback(show_album_holder),
            track_has_album_visibility(runtime),
        ),
        TrackContextAction::copy_files(copy_files_callback(runtime, window)),
        TrackContextAction::show_in_folder(show_in_folder_callback(runtime, window)),
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

#[allow(clippy::too_many_arguments)]
fn playlist_track_context_actions(
    runtime: &SharedRuntime,
    window: &gtk::Window,
    show_album_holder: &ShowAlbumHolder,
    command_controller: &SharedCommandController,
    playback_changed: PlaybackChangedCallback,
    library_changed_holder: LibraryChangedHolder,
    track_row_changed_holder: TrackRowChangedHolder,
    sidebar: &PlaylistSidebar,
) -> TrackContextActionSet {
    TrackContextActionSet::new(vec![
        TrackContextAction::play_next(
            play_next_callback(command_controller),
            playback_has_current_track_visibility(runtime),
        ),
        TrackContextAction::add_to_queue(
            add_to_queue_callback(command_controller),
            playback_has_current_track_visibility(runtime),
        ),
        TrackContextAction::get_info(get_info_callback(
            window,
            runtime,
            command_controller,
            &library_changed_holder,
            &track_row_changed_holder,
        )),
        TrackContextAction::show_album(
            show_album_callback(show_album_holder),
            track_has_album_visibility(runtime),
        ),
        TrackContextAction::copy_files(copy_files_callback(runtime, window)),
        TrackContextAction::show_in_folder(show_in_folder_callback(runtime, window)),
        TrackContextAction::remove_from_playlist(
            remove_from_playlist_callback(
                command_controller,
                sidebar,
                library_changed_holder.clone(),
            ),
            current_selection_is_regular_playlist(sidebar),
        ),
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

/// Build the drag-reorder callback for the playlist track table. The callback
/// only acts when a *regular* playlist is selected in the sidebar — smart
/// playlists and the Library pseudo-entry are derived/dynamic and have no
/// authoritative entry order to mutate. No GTK-only row reorder path:
/// this dispatches `MovePlaylistEntries` so the runtime/SQLite are the
/// source of truth.
///
/// Post-dispatch the callback rebuilds **only** the playlists table —
/// nothing in the library, the album set, or the sidebar tree changes
/// when a playlist's internal order is shuffled. Calling the global
/// `library_changed` here (the previous approach) re-built the songs
/// table's entire `gio::ListStore` (10k rows + re-sort), the albums
/// view's groupings, and the sidebar — visible as a 1–2 s freeze after
/// every drop. The narrow refresh below touches only the rows the user
/// is looking at, so the new order appears in the next frame.
///
/// `new_position` is the insertion index in the playlist's *post-removal*
/// entries list (see `ApplicationCommand::MovePlaylistEntries`), so the
/// caller pre-shifts by the count of dragged tracks that currently sit
/// before the target row.
fn playlist_row_reorder_callback(
    command_controller: &SharedCommandController,
    runtime: &SharedRuntime,
    sidebar: &PlaylistSidebar,
    playlists_table_holder: &Rc<RefCell<Option<TrackTable>>>,
    current_search_text: &Rc<RefCell<String>>,
) -> RowReorderCallback {
    let command_controller = command_controller.clone();
    let runtime = runtime.clone();
    let sidebar = sidebar.clone();
    let playlists_table_holder = playlists_table_holder.clone();
    let current_search_text = current_search_text.clone();

    Rc::new(move |drop: RowReorderDrop| -> bool {
        let Some(SidebarSelection::Item(PlaylistItem::Playlist(playlist_id))) =
            sidebar.current_selection()
        else {
            // Drops on smart-playlist / library views are silently
            // ignored; the indicator was already cleared when GTK fired
            // the drop signal, so there is no visual residue.
            return false;
        };

        let new_position = {
            let runtime_borrow = runtime.borrow();
            let Some(playlist) = runtime_borrow
                .playlists()
                .iter()
                .find(|playlist| playlist.id == playlist_id)
            else {
                return false;
            };
            let Some(new_position) = compute_playlist_reorder_position(&playlist.entries, &drop)
            else {
                return false;
            };
            new_position
        };

        let dispatched =
            command_controller.dispatch_succeeded(ApplicationCommand::MovePlaylistEntries {
                playlist_id,
                track_ids: drop.dragged_track_ids,
                new_position,
            });
        if !dispatched {
            return false;
        }

        // Targeted rebuild — only the playlist view. Library / albums /
        // sidebar are untouched because a reorder doesn't mutate any of
        // the state those views derive from.
        if let Some(playlists_table) = playlists_table_holder.borrow().as_ref() {
            let search_text = current_search_text.borrow().clone();
            let rows = playlist_table_rows_for(
                &runtime.borrow(),
                sidebar.current_selection(),
                &search_text,
            );
            playlists_table.replace_rows(rows);
        }
        true
    })
}

/// Resolve the (`Above`/`Below`, target-row-track-id) pair from a drop into a
/// post-removal insertion index for `MovePlaylistEntries`.
///
/// Returns `None` when the target row is not in the playlist (shouldn't
/// happen for in-table drops, but the row id is opaque to the cell-level
/// drop target and is worth validating before dispatching), or when every
/// dragged track is the target row itself.
fn compute_playlist_reorder_position(
    entries: &[sustain_app_runtime::PlaylistEntry],
    drop: &RowReorderDrop,
) -> Option<u32> {
    let target_index = entries
        .iter()
        .position(|entry| entry.track_id == drop.target_track_id)?;
    let moving: std::collections::BTreeSet<sustain_app_runtime::TrackId> =
        drop.dragged_track_ids.iter().copied().collect();
    if moving.is_empty() {
        return None;
    }
    // Count source tracks that currently sit before the target row; they
    // will be removed first, so the target row's post-removal index drops
    // by that count.
    let source_tracks_before_target = entries
        .iter()
        .take(target_index)
        .filter(|entry| moving.contains(&entry.track_id))
        .count();
    let target_post_removal_index = target_index - source_tracks_before_target;
    let new_position = match drop.position {
        RowDropPosition::Above => target_post_removal_index,
        RowDropPosition::Below => target_post_removal_index + 1,
    };
    u32::try_from(new_position).ok()
}

fn remove_from_playlist_callback(
    command_controller: &SharedCommandController,
    sidebar: &PlaylistSidebar,
    library_changed_holder: LibraryChangedHolder,
) -> TrackActionCallback {
    let command_controller = command_controller.clone();
    let sidebar = sidebar.clone();

    Rc::new(move |track_ids: Vec<TrackId>| {
        let Some(SidebarSelection::Item(PlaylistItem::Playlist(playlist_id))) =
            sidebar.current_selection()
        else {
            return;
        };
        if command_controller.dispatch_succeeded(ApplicationCommand::RemoveTracksFromPlaylist {
            playlist_id,
            track_ids,
        }) && let Some(callback) = library_changed_holder.borrow().as_ref()
        {
            callback();
        }
    })
}

fn current_selection_is_regular_playlist(sidebar: &PlaylistSidebar) -> TrackActionVisibility {
    let sidebar = sidebar.clone();
    Rc::new(move |_track_ids| {
        matches!(
            sidebar.current_selection(),
            Some(SidebarSelection::Item(PlaylistItem::Playlist(_)))
        )
    })
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
            .map(&command_builder)
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

struct KeyboardShortcutContext {
    command_controller: SharedCommandController,
    playback_changed: PlaybackChangedCallback,
    runtime: SharedRuntime,
    songs_table: TrackTable,
    playlists_table: TrackTable,
    albums_view: AlbumsView,
    content_stack: gtk::Stack,
    sidebar: PlaylistSidebar,
}

fn install_keyboard_shortcuts(window: &gtk::ApplicationWindow, context: KeyboardShortcutContext) {
    let KeyboardShortcutContext {
        command_controller,
        playback_changed,
        runtime,
        songs_table,
        playlists_table,
        albums_view,
        content_stack,
        sidebar,
    } = context;

    let key_controller = gtk::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);

    let window_for_focus = window.clone();
    key_controller.connect_key_pressed(move |_controller, key, _keycode, state| {
        let typing = focus_accepts_text(&window_for_focus);

        if key == gdk::Key::space && !typing {
            if command_controller.dispatch_succeeded(ApplicationCommand::Playback(
                PlaybackCommand::TogglePlayPause,
            )) {
                playback_changed();
            }
            return glib::Propagation::Stop;
        }

        if matches!(key, gdk::Key::l | gdk::Key::L)
            && state.contains(gdk::ModifierType::CONTROL_MASK)
            && !typing
        {
            jump_to_current_track(
                &runtime,
                &songs_table,
                &playlists_table,
                &albums_view,
                &content_stack,
                &sidebar,
            );
            return glib::Propagation::Stop;
        }

        glib::Propagation::Proceed
    });
    window.add_controller(key_controller);
}

/// Reveal the currently playing track in the active view, or fall back
/// to Music if the active view cannot show it. Does nothing when
/// nothing has ever played (no current `now_playing.track`). Paused
/// tracks still qualify — they remain the current track until
/// something else loads.
///
/// The fallback path picks the Music entry in the sidebar so the
/// content stack flips to `SONGS_VIEW` and the songs table receives
/// the reveal. The per-playlist table only receives the reveal when a
/// real playlist or smart playlist is the current selection.
fn jump_to_current_track(
    runtime: &SharedRuntime,
    songs_table: &TrackTable,
    playlists_table: &TrackTable,
    albums_view: &AlbumsView,
    content_stack: &gtk::Stack,
    sidebar: &PlaylistSidebar,
) {
    let Some(track_id) = runtime
        .borrow()
        .now_playing()
        .track
        .as_ref()
        .map(|track| track.id)
    else {
        return;
    };

    let active_view = content_stack.visible_child_name();
    let revealed_in_active = match active_view.as_deref() {
        Some(ALBUMS_VIEW) => albums_view.reveal_album_for_track(track_id),
        Some(PLAYLISTS_VIEW) => playlists_table.reveal_track(track_id),
        Some(SONGS_VIEW) => songs_table.reveal_track(track_id),
        _ => false,
    };

    if revealed_in_active {
        return;
    }

    sidebar.select_music();
    songs_table.reveal_track(track_id);
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

/// Owns the sidebar collapse / expand state, the toggle button that
/// drives it, and the last manually-set expanded width so a user who
/// drag-resized the sidebar keeps that width on re-expand.
///
/// State transitions snap the [`gtk::Paned`]'s position instantly —
/// the right-hand content column hosts views (Albums grid, track
/// table virtualisation) whose layout cost makes a continuously-
/// resizing animation visibly choppy. An instant flip is also closer
/// to the iTunes 11 sidebar toggle, which had no slide animation.
#[derive(Clone)]
pub(crate) struct SidebarCollapseController {
    inner: Rc<SidebarCollapseControllerInner>,
}

struct SidebarCollapseControllerInner {
    paned: gtk::Paned,
    toggle: gtk::Button,
    collapsed: Cell<bool>,
    last_expanded_position: Cell<i32>,
}

impl SidebarCollapseController {
    fn new(paned: gtk::Paned, initial_collapsed: bool, initial_width: Option<u32>) -> Self {
        let toggle = gtk::Button::new();
        toggle.add_css_class("flat");
        toggle.add_css_class("sidebar-collapse-toggle");
        toggle.set_focus_on_click(false);
        toggle.set_can_focus(false);

        // Clamp the persisted width back into the legal band. The
        // domain stores whatever the user last set; the UI is the
        // authority on min/max, so out-of-band values are silently
        // pulled into range rather than rejected.
        let expanded_width = initial_width
            .map(|width| (width as i32).clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH))
            .unwrap_or(SIDEBAR_DEFAULT_WIDTH);

        let inner = Rc::new(SidebarCollapseControllerInner {
            paned: paned.clone(),
            toggle: toggle.clone(),
            collapsed: Cell::new(initial_collapsed),
            last_expanded_position: Cell::new(expanded_width),
        });

        // Apply the persisted collapsed state.
        if initial_collapsed {
            inner.paned.set_position(0);
        } else {
            inner.paned.set_position(expanded_width);
        }
        sync_collapse_toggle_icon(&toggle, initial_collapsed);

        // Track user-driven drag-resizes so we restore the chosen
        // width on the next expand.
        let inner_for_position = inner.clone();
        inner.paned.connect_position_notify(move |content_area| {
            if inner_for_position.collapsed.get() {
                return;
            }
            let position = content_area.position();
            if position > 0 {
                inner_for_position.last_expanded_position.set(position);
            }
        });

        let inner_for_click = inner.clone();
        toggle.connect_clicked(move |_| {
            let controller = SidebarCollapseController {
                inner: inner_for_click.clone(),
            };
            controller.toggle();
        });

        Self { inner }
    }

    fn toggle_widget(&self) -> gtk::Button {
        self.inner.toggle.clone()
    }

    fn is_collapsed(&self) -> bool {
        self.inner.collapsed.get()
    }

    /// The last manually-set expanded width, in pixels. Used at
    /// shutdown to persist the user's preferred sidebar width. Always
    /// the expanded width — collapsing does not zero this out, so
    /// re-expanding restores the same value on next launch.
    fn expanded_width(&self) -> u32 {
        self.inner.last_expanded_position.get().max(0) as u32
    }

    /// No-op when the sidebar is already visible. Used by shortcuts
    /// that need the sidebar on-screen for their UI affordance to be
    /// visible (e.g. Ctrl+N's armed inline rename of a new playlist
    /// row).
    pub(crate) fn expand_if_collapsed(&self) {
        if self.inner.collapsed.get() {
            self.toggle();
        }
    }

    fn toggle(&self) {
        let collapse = !self.inner.collapsed.get();
        self.inner.collapsed.set(collapse);
        sync_collapse_toggle_icon(&self.inner.toggle, collapse);

        let target = if collapse {
            0
        } else {
            self.inner.last_expanded_position.get().max(1)
        };
        self.inner.paned.set_position(target);
    }
}

/// Repaint the toggle's icon and tooltip to advertise the action the
/// next click performs.
///
/// - When the sidebar is visible, the click collapses it — show a
///   left-pointing arrow ("Collapse sidebar").
/// - When the sidebar is hidden, the click brings it back — show a
///   right-pointing arrow ("Show sidebar").
fn sync_collapse_toggle_icon(button: &gtk::Button, collapsed: bool) {
    let (icon_name, tooltip) = if collapsed {
        ("go-next-symbolic", "Show sidebar")
    } else {
        ("go-previous-symbolic", "Collapse sidebar")
    };
    button.set_icon_name(icon_name);
    button.set_tooltip_text(Some(tooltip));
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

#[cfg(test)]
mod tests {
    use super::*;
    use sustain_app_runtime::{PlaylistEntry, PlaylistId};

    #[test]
    fn drop_above_target_post_removal_collapses_source_tracks_before_target() {
        // Playlist: [1, 2, 3, 4, 5]. Drag [3, 4] (which sit before the
        // target row), drop above row 5. The post-removal list is
        // [1, 2, 5] (len 3); row 5's post-removal index is 2, and "above"
        // resolves to insertion at 2 — landing the [3, 4] block right
        // before 5 in the final order: [1, 2, 3, 4, 5] (no visual change
        // because the block was already contiguous and ends just before
        // the target).
        let entries = playlist_entries(&[1, 2, 3, 4, 5]);
        let drop = RowReorderDrop {
            dragged_track_ids: vec![track_id(3), track_id(4)],
            target_track_id: track_id(5),
            position: RowDropPosition::Above,
        };
        assert_eq!(compute_playlist_reorder_position(&entries, &drop), Some(2));
    }

    #[test]
    fn drop_below_target_adds_one_to_post_removal_index() {
        // Playlist: [1, 2, 3, 4, 5]. Drag [3], drop below row 5.
        // Post-removal list: [1, 2, 4, 5] (len 4); row 5's post-removal
        // index is 3; "below" → insertion at 4, which clamps to len and
        // lands the track at the tail: [1, 2, 4, 5, 3].
        let entries = playlist_entries(&[1, 2, 3, 4, 5]);
        let drop = RowReorderDrop {
            dragged_track_ids: vec![track_id(3)],
            target_track_id: track_id(5),
            position: RowDropPosition::Below,
        };
        assert_eq!(compute_playlist_reorder_position(&entries, &drop), Some(4));
    }

    #[test]
    fn drop_above_target_when_no_sources_precede_it_keeps_index_unchanged() {
        // Playlist: [1, 2, 3, 4]. Drag [4], drop above row 2.
        // No source tracks before row 2; row 2 is at index 1, stays at
        // post-removal index 1. "Above" → insertion at 1 — final order:
        // [1, 4, 2, 3].
        let entries = playlist_entries(&[1, 2, 3, 4]);
        let drop = RowReorderDrop {
            dragged_track_ids: vec![track_id(4)],
            target_track_id: track_id(2),
            position: RowDropPosition::Above,
        };
        assert_eq!(compute_playlist_reorder_position(&entries, &drop), Some(1));
    }

    #[test]
    fn missing_target_rejects_the_move() {
        let entries = playlist_entries(&[1, 2, 3]);
        let drop = RowReorderDrop {
            dragged_track_ids: vec![track_id(1)],
            target_track_id: track_id(99),
            position: RowDropPosition::Above,
        };
        assert_eq!(compute_playlist_reorder_position(&entries, &drop), None);
    }

    #[test]
    fn empty_dragged_set_rejects_the_move() {
        let entries = playlist_entries(&[1, 2, 3]);
        let drop = RowReorderDrop {
            dragged_track_ids: Vec::new(),
            target_track_id: track_id(2),
            position: RowDropPosition::Above,
        };
        assert_eq!(compute_playlist_reorder_position(&entries, &drop), None);
    }

    fn playlist_entries(track_ids: &[i64]) -> Vec<PlaylistEntry> {
        let playlist_id = PlaylistId::new(1).expect("positive id");
        track_ids
            .iter()
            .enumerate()
            .map(|(position, id)| PlaylistEntry {
                playlist_id,
                track_id: track_id(*id),
                position: u32::try_from(position).expect("position fits in u32"),
            })
            .collect()
    }

    fn track_id(value: i64) -> TrackId {
        TrackId::new(value).expect("positive track id")
    }
}
