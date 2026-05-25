// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gtk::prelude::*;
use gtk::{gdk, glib};
use sustain_app_runtime::{
    PlaybackCommand, Playlist, PlaylistEntry, PlaylistFolder, PlaylistFolderId, PlaylistItem,
    Rating, Track, TrackColumnLayout, TrackColumnLayoutScope, TrackId,
    filter_tracks_by_search_text, track_matches_search_text,
};

use super::{
    ALBUMS_VIEW, APP_ID, ApplicationCommand, ApplicationRuntime, ArtworkFetchResultReceiver,
    LibraryChangedCallback, LibraryChangedHolder, MetadataWriteResultReceiver,
    MprisCommandReceiver, PLAYLISTS_VIEW, PlaybackChangedCallback, SONGS_VIEW, SharedMprisService,
    SharedRuntime, ShowAlbumAction, ShowAlbumHolder, TrackRowChangedCallback,
    TrackRowChangedHolder,
    accent::install_accent_css,
    albums::AlbumsView,
    app_css::install_app_css,
    artwork_loader::ArtworkLoader,
    command_controller::{SharedCommandController, UiCommandController},
    content_stack::build_content_stack,
    library_consolidation::library_consolidation_requested_callback,
    library_import::{
        LIBRARY_DROP_INDICATOR_CLASS, install_file_drop_target, library_import_requested_callback,
    },
    library_scan::library_scan_requested_callback,
    mode_bar::{ShowSongsViewCallback, ViewModeChangedCallback, build_mode_bar},
    now_playing::NowPlayingView,
    preferences::install_preferences_action,
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
        TrackActionVisibility, TrackContextAction, TrackContextActionSet, TrackRowContextMenu,
    },
    track_context_ops::{
        copy_files_callback, get_info_callback, play_next_callback,
        playback_has_current_track_visibility, show_album_callback, show_in_folder_callback,
        track_has_album_visibility,
    },
    track_table::{
        RatingChangedCallback, TrackActivatedCallback, TrackTable, TrackTableRow, build_track_table,
    },
    window_chrome::{install_resize_handles, install_window_state_chrome},
};

pub(crate) fn build_main_window(
    app: &gtk::Application,
    runtime: SharedRuntime,
    mpris_service: Option<SharedMprisService>,
    mpris_command_rx: Option<MprisCommandReceiver>,
    metadata_write_result_rx: Option<MetadataWriteResultReceiver>,
    artwork_fetch_result_rx: Option<ArtworkFetchResultReceiver>,
) -> gtk::ApplicationWindow {
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

    // Shared current-search-text state. Lives only in memory: per the
    // agreed product decision, the query never persists across restarts —
    // a populated search field on launch would silently hide most of the
    // library and confuse the user. Captured by all view-rebuild paths.
    let current_search_text: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

    let library_tracks = runtime_library_table_rows(&runtime.borrow(), "");
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
    let command_controller: SharedCommandController = Rc::new(UiCommandController::new(
        runtime.clone(),
        status_bar.clone(),
    ));

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
        status_bar.clone(),
    );
    let initial_volume = runtime.borrow().settings().playback.volume;
    let titlebar = build_titlebar(now_playing.widget(), initial_volume);
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
    sidebar_widget.set_visible(false);

    let track_activated = track_activated_callback(&command_controller, playback_changed.clone());
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
    );
    let add_to_playlist_provider = add_to_playlist_provider(&runtime);
    let add_to_playlist_callback =
        add_to_playlist_callback(&command_controller, &runtime, &library_changed_holder);
    let context_menu = TrackRowContextMenu::new(context_actions, parent_window.clone())
        .with_add_to_playlist(
            add_to_playlist_provider.clone(),
            add_to_playlist_callback.clone(),
        );
    let playlist_context_actions = playlist_track_context_actions(
        &runtime,
        &parent_window,
        &show_album_holder,
        &command_controller,
        playback_changed.clone(),
        library_changed_holder.clone(),
        &sidebar,
    );
    let playlist_context_menu = TrackRowContextMenu::new(playlist_context_actions, parent_window)
        .with_add_to_playlist(add_to_playlist_provider, add_to_playlist_callback);
    let rating_changed =
        rating_changed_callback(&command_controller, track_row_changed_holder.clone());
    let songs_table = build_track_table(
        library_tracks.clone(),
        Some(track_activated.clone()),
        Some(context_menu.clone()),
        Some(rating_changed.clone()),
    );
    songs_table_holder.replace(Some(songs_table.clone()));
    let albums_view = AlbumsView::new(
        runtime.clone(),
        command_controller.clone(),
        playback_changed.clone(),
        context_menu,
        artwork_loader.clone(),
    );
    albums_view_holder.replace(Some(albums_view.clone()));
    let playlists_table = build_track_table(
        Vec::new(),
        Some(track_activated.clone()),
        Some(playlist_context_menu),
        Some(rating_changed),
    );
    playlists_table_holder.replace(Some(playlists_table.clone()));
    install_track_column_layout_persistence(&runtime, &songs_table, &playlists_table, &sidebar);
    playback_changed();
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

    let content_stack = build_content_stack(
        &songs_drop_overlay,
        &albums_view.widget(),
        &playlists_table.widget(),
    );
    install_albums_view_activator(&content_stack, &albums_view);
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
        &playlists_table,
        &sidebar,
        visible_summary_refresh.clone(),
        &current_search_text,
    );
    let track_row_changed = track_row_changed_callback(
        &runtime,
        &songs_table,
        &playlists_table,
        &sidebar,
        visible_summary_refresh.clone(),
        &current_search_text,
    );
    track_row_changed_holder.replace(Some(track_row_changed));
    install_metadata_write_result_consumer(
        metadata_write_result_rx,
        status_bar.clone(),
        track_row_changed_holder.clone(),
    );
    install_artwork_fetch_result_consumer(ArtworkFetchResultConsumerContext {
        receiver: artwork_fetch_result_rx,
        runtime: runtime.clone(),
        command_controller: command_controller.clone(),
        artwork_loader: artwork_loader.clone(),
        now_playing: now_playing.clone(),
        playback_changed: playback_changed.clone(),
        status_bar: status_bar.clone(),
        track_row_changed_holder: track_row_changed_holder.clone(),
    });
    sidebar.set_selection_changed(sidebar_selection_changed_callback(
        &runtime,
        &playlists_table,
        visible_summary_refresh.clone(),
        &current_search_text,
    ));
    install_search_wiring(
        &titlebar,
        SearchWiringContext {
            current_search_text: current_search_text.clone(),
            runtime: runtime.clone(),
            songs_table: songs_table.clone(),
            albums_view: albums_view.clone(),
            playlists_table: playlists_table.clone(),
            sidebar: sidebar.clone(),
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
    library_changed_holder.replace(Some(library_changed.clone()));
    let scan_requested =
        library_scan_requested_callback(&runtime, library_changed.clone(), &status_bar);
    let consolidation_requested =
        library_consolidation_requested_callback(&runtime, library_changed.clone(), &status_bar);
    let import_requested =
        library_import_requested_callback(&runtime, library_changed.clone(), &status_bar);
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
    let mode_bar = build_mode_bar(
        &window,
        &sidebar_widget,
        &content_stack,
        command_controller,
        scan_requested,
        consolidation_requested,
        visible_summary_refresh,
    );
    let albums_view_for_reveal = albums_view.clone();
    let show_albums_view = mode_bar.show_albums.clone();
    let show_album_action: ShowAlbumAction = Rc::new(move |track_id| {
        show_albums_view();
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
            show_songs: mode_bar.show_songs.clone(),
        },
    );
    install_global_shortcuts(GlobalShortcutContext {
        app: app.clone(),
        window: window.clone(),
        command_controller: command_controller_for_global_shortcuts,
        runtime: runtime.clone(),
        sidebar: sidebar.clone(),
        titlebar: titlebar.clone(),
        songs_table: songs_table.clone(),
        playlists_table: playlists_table.clone(),
        content_stack: content_stack.clone(),
        show_playlists: mode_bar.show_playlists.clone(),
        library_changed_holder: library_changed_holder.clone(),
    });
    main_content.append(&mode_bar.widget);
    main_content.append(&content_stack);

    let content_area = build_content_area(&sidebar_widget, &main_content);

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

    // Any debounced save scheduled within the debounce window of shutdown
    // would otherwise be lost: the timer's main loop never gets to fire.
    let songs_table_for_close = songs_table.clone();
    let playlists_table_for_close = playlists_table.clone();
    let titlebar_for_close = titlebar.clone();
    window.connect_close_request(move |_window| {
        songs_table_for_close.flush_pending_layout_save();
        playlists_table_for_close.flush_pending_layout_save();
        titlebar_for_close.flush_pending_volume_save();
        glib::Propagation::Proceed
    });

    window
}

/// Defer the cost of populating the Albums view until the user
/// actually switches to it. Activation groups the current library into
/// album rows and lets the virtualized Albums view bind only visible
/// rows; doing that at startup still provides no benefit while Songs is
/// the initial mode. Hooking into the content stack's visible-child
/// notification keeps the activation trigger in one place — any caller
/// that flips the stack to ALBUMS_VIEW
/// (the mode-bar toggle, the reveal-album action, future shortcuts)
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

fn library_changed_callback(
    runtime: &SharedRuntime,
    songs_table: &TrackTable,
    albums_view: &AlbumsView,
    playlists_table: &TrackTable,
    sidebar: &PlaylistSidebar,
    visible_summary_refresh: ViewModeChangedCallback,
    current_search_text: &Rc<RefCell<String>>,
) -> LibraryChangedCallback {
    let runtime = runtime.clone();
    let songs_table = songs_table.clone();
    let albums_view = albums_view.clone();
    let playlists_table = playlists_table.clone();
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
        sidebar.refresh();
        let playlist_rows =
            playlist_table_rows_for(&runtime.borrow(), sidebar.current_selection(), &search_text);
        playlists_table.replace_rows(playlist_rows);
        visible_summary_refresh();
    })
}

fn visible_summary_refresh_callback(
    runtime: &SharedRuntime,
    content_stack: &gtk::Stack,
    sidebar: &PlaylistSidebar,
    status_bar: &StatusBar,
    current_search_text: &Rc<RefCell<String>>,
) -> ViewModeChangedCallback {
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

fn sidebar_selection_changed_callback(
    runtime: &SharedRuntime,
    playlists_table: &TrackTable,
    visible_summary_refresh: ViewModeChangedCallback,
    current_search_text: &Rc<RefCell<String>>,
) -> super::sidebar::SidebarSelectionChangedCallback {
    let runtime = runtime.clone();
    let playlists_table = playlists_table.clone();
    let current_search_text = current_search_text.clone();

    Rc::new(move |selection| {
        if let Some(layout) = layout_for_selection(&runtime.borrow(), selection) {
            playlists_table.apply_layout(&layout);
        }
        let search_text = current_search_text.borrow().clone();
        let rows = playlist_table_rows_for(&runtime.borrow(), selection, &search_text);
        playlists_table.replace_rows(rows);
        visible_summary_refresh();
    })
}

/// Wires the topbar SearchEntry to a debounced callback that re-filters
/// all three view modes (Songs, Albums, Playlists) plus the status-bar
/// summary against the new query. All three views are rebuilt on each
/// fire so that switching modes mid-query never shows stale unfiltered
/// content.
///
/// Filtering follows the agreed product semantics:
/// - Songs view filters across the 7 track-level fields covered by
///   [`track_matches_search_text`].
/// - Albums view filters by album-level fields only (title, artist,
///   year) via [`AlbumsView::set_search_text`].
/// - Playlists view filters within the currently selected playlist /
///   smart playlist / Library pseudo-entry, again on track fields.
///
/// Debouncing: rebuilding the visible track table on every keystroke is
/// expensive — not because of the in-memory filter (microseconds) but
/// because [`TrackTable::replace_rows`] rewrites the underlying
/// `gio::ListStore`, which fires GTK list-model events that the sorter
/// and selection model both have to process. The same effect shows up
/// on the album grid. We therefore cancel any in-flight rebuild and
/// schedule a fresh one [`SEARCH_REBUILD_DEBOUNCE`] in the future on
/// every keystroke, collapsing a typing burst into one rebuild when
/// the user pauses. No flush-on-close: search state is purely
/// in-memory and never persisted, so dropping a pending rebuild at
/// shutdown loses nothing.
struct SearchWiringContext {
    current_search_text: Rc<RefCell<String>>,
    runtime: SharedRuntime,
    songs_table: TrackTable,
    albums_view: AlbumsView,
    playlists_table: TrackTable,
    sidebar: PlaylistSidebar,
    visible_summary_refresh: ViewModeChangedCallback,
}

fn install_search_wiring(titlebar: &Titlebar, context: SearchWiringContext) {
    let SearchWiringContext {
        current_search_text,
        runtime,
        songs_table,
        albums_view,
        playlists_table,
        sidebar,
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
            let sidebar = sidebar.clone();
            let visible_summary_refresh = visible_summary_refresh.clone();
            let pending_rebuild_clear = pending_rebuild.clone();
            let source_id = glib::timeout_add_local_once(SEARCH_REBUILD_DEBOUNCE, move || {
                pending_rebuild_clear.borrow_mut().take();

                let songs_rows = runtime_library_table_rows(&runtime.borrow(), &new_text);
                songs_table.replace_rows(songs_rows);

                albums_view.set_search_text(new_text.clone());

                let playlist_rows = playlist_table_rows_for(
                    &runtime.borrow(),
                    sidebar.current_selection(),
                    &new_text,
                );
                playlists_table.replace_rows(playlist_rows);

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
    let candidate_tracks: Vec<Track> = match selection {
        Some(SidebarSelection::Library) => runtime.library_tracks().to_vec(),
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
            playlist_entries_in_order(playlist)
                .filter_map(|track_id| tracks_by_id.get(&track_id).copied().cloned())
                .collect()
        }
        Some(SidebarSelection::Item(PlaylistItem::SmartPlaylist(smart_playlist_id))) => runtime
            .smart_playlist_matching_tracks(smart_playlist_id)
            .into_iter()
            .cloned()
            .collect(),
        _ => return Vec::new(),
    };

    let filtered = if search_text.is_empty() {
        candidate_tracks
    } else {
        filter_tracks_by_search_text(&candidate_tracks, search_text)
    };
    filtered.iter().map(TrackTableRow::from_track).collect()
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

fn playlist_entries_in_order(playlist: &Playlist) -> impl Iterator<Item = TrackId> + '_ {
    let mut ordered: Vec<&PlaylistEntry> = playlist.entries.iter().collect();
    ordered.sort_by_key(|entry| entry.position);
    ordered.into_iter().map(|entry| entry.track_id)
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

/// Drains [`MetadataWriteResult`]s posted by the async metadata writer
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
    status_bar: StatusBar,
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
            status_bar.show_command_message(message);
            if let Some(callback) = track_row_changed_holder.borrow().as_ref() {
                callback(result.track_id);
            }
        }
    });
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
    status_bar: StatusBar,
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
        status_bar,
        track_row_changed_holder,
    } = context;
    let Some(receiver) = receiver else {
        return;
    };
    glib::MainContext::default().spawn_local(async move {
        while let Ok(result) = receiver.recv().await {
            use sustain_app_runtime::ArtworkFetchOutcome;
            match result.outcome {
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
                            artwork: Some(bytes),
                        },
                    );
                    status_bar.show_task_message("Artwork updated.", false);
                }
                ArtworkFetchOutcome::NoMatch => {
                    status_bar.show_task_message("No cover art found for this track.", false);
                }
                ArtworkFetchOutcome::Failed => {
                    status_bar.show_task_message("Could not fetch cover art.", false);
                }
            }
            now_playing.notify_artwork_fetch_complete(result.track_id);
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
        MprisCommand::Next => PlaybackCommand::PlayNextTrack,
        MprisCommand::Previous => PlaybackCommand::PlayPreviousTrack,
    };
    if command_controller.dispatch_succeeded(ApplicationCommand::Playback(playback_command)) {
        playback_changed();
    }
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
/// Updates only the affected row in the visible tables and refreshes the
/// status-bar summary. Skips the AlbumsView grid (rating/play-count don't
/// affect album grouping) and the sidebar tree (row-field mutations do not
/// alter playlist/folder structure).
///
/// When a smart playlist is selected, the Playlists table falls back to a
/// full reflow because the mutation may add/remove the track from the
/// playlist's filtered set — an in-place row update would lie.
fn track_row_changed_callback(
    runtime: &SharedRuntime,
    songs_table: &TrackTable,
    playlists_table: &TrackTable,
    sidebar: &PlaylistSidebar,
    visible_summary_refresh: ViewModeChangedCallback,
    current_search_text: &Rc<RefCell<String>>,
) -> TrackRowChangedCallback {
    let runtime = runtime.clone();
    let songs_table = songs_table.clone();
    let playlists_table = playlists_table.clone();
    let sidebar = sidebar.clone();
    let current_search_text = current_search_text.clone();

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

        match sidebar.current_selection() {
            Some(SidebarSelection::Item(PlaylistItem::SmartPlaylist(_))) => {
                let search_text = current_search_text.borrow().clone();
                let playlist_rows = playlist_table_rows_for(
                    &runtime.borrow(),
                    sidebar.current_selection(),
                    &search_text,
                );
                playlists_table.replace_rows(playlist_rows);
            }
            _ => {
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
) -> TrackContextActionSet {
    TrackContextActionSet::new(vec![
        TrackContextAction::play_next(
            play_next_callback(command_controller),
            playback_has_current_track_visibility(runtime),
        ),
        TrackContextAction::get_info(get_info_callback(
            window,
            runtime,
            command_controller,
            &library_changed_holder,
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

fn playlist_track_context_actions(
    runtime: &SharedRuntime,
    window: &gtk::Window,
    show_album_holder: &ShowAlbumHolder,
    command_controller: &SharedCommandController,
    playback_changed: PlaybackChangedCallback,
    library_changed_holder: LibraryChangedHolder,
    sidebar: &PlaylistSidebar,
) -> TrackContextActionSet {
    TrackContextActionSet::new(vec![
        TrackContextAction::play_next(
            play_next_callback(command_controller),
            playback_has_current_track_visibility(runtime),
        ),
        TrackContextAction::get_info(get_info_callback(
            window,
            runtime,
            command_controller,
            &library_changed_holder,
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
    show_songs: ShowSongsViewCallback,
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
        show_songs,
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
                &show_songs,
            );
            return glib::Propagation::Stop;
        }

        glib::Propagation::Proceed
    });
    window.add_controller(key_controller);
}

/// Reveal the currently playing track in the active view, or fall back to
/// Songs if the active view cannot show it. Does nothing when nothing has
/// ever played (no current `now_playing.track`). Paused tracks still
/// qualify — they remain the current track until something else loads.
///
/// Ctrl-L reveal currently has one deliberate product ambiguity:
///
/// **"Wrong view" intent is ambiguous.** Today this stays in the active
///    view when its model contains the playing track. In Playlists view
///    with the "Library" sidebar entry selected, that means the song is
///    revealed in the Playlists table (since Library mirrors the full
///    library), which the user has flagged as wrong — they expect a switch
///    to Songs. A real fix needs the Playlists branch to check whether a
///    *real* playlist is selected (vs. the Library pseudo-entry), or to
///    abandon the per-view reveal entirely and always route through Songs.
///    Not done here because the right product behavior is unclear.
fn jump_to_current_track(
    runtime: &SharedRuntime,
    songs_table: &TrackTable,
    playlists_table: &TrackTable,
    albums_view: &AlbumsView,
    content_stack: &gtk::Stack,
    show_songs: &ShowSongsViewCallback,
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

    show_songs();
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
