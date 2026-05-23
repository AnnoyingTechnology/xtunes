// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    time::SystemTime,
};

use gtk::prelude::*;
use gtk::{gdk, glib};
use xtunes_app_runtime::{
    PlaybackCommand, Playlist, PlaylistEntry, PlaylistFolder, PlaylistFolderId, PlaylistId,
    PlaylistItem, Rating, Track, TrackId,
};

use super::{
    APP_ID, ApplicationCommand, ApplicationRuntime, LibraryChangedCallback, LibraryChangedHolder,
    PLAYLISTS_VIEW, PlaybackChangedCallback, SharedRuntime, ShowAlbumAction, ShowAlbumHolder,
    accent::install_accent_css,
    albums::AlbumsView,
    app_css::install_app_css,
    command_controller::{SharedCommandController, UiCommandController},
    content_stack::build_content_stack,
    library_scan::library_scan_requested_callback,
    mode_bar::{ViewModeChangedCallback, build_mode_bar},
    now_playing::NowPlayingView,
    preferences::install_preferences_action,
    sidebar::{PlaylistSidebar, SidebarSelection, build_content_area},
    sidebar_context::{
        NEW_PLAYLIST_DEFAULT_NAME, NEW_PLAYLIST_FOLDER_DEFAULT_NAME,
        NEW_SMART_PLAYLIST_DEFAULT_NAME, SidebarActionCallback, SidebarContextAction,
        SidebarContextMenu, unique_default_name,
    },
    smart_playlist_editor::{SmartPlaylistEditorMode, open_smart_playlist_editor},
    status_bar::StatusBar,
    titlebar::{
        Titlebar, build_titlebar, connect_titlebar_playback_controls, sync_play_pause_icon,
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
    install_track_ended_callback(&runtime, &command_controller, &playback_changed);

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
        rating_changed_callback(&command_controller, library_changed_holder.clone());
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
    );
    let playlists_table = build_track_table(
        Vec::new(),
        Some(track_activated.clone()),
        Some(playlist_context_menu),
        Some(rating_changed),
    );
    playback_changed();
    let content_stack = build_content_stack(
        songs_table.widget(),
        albums_view.widget(),
        playlists_table.widget(),
    );
    let visible_summary_refresh =
        visible_summary_refresh_callback(&runtime, &content_stack, &sidebar, &status_bar);
    let library_changed = library_changed_callback(
        &runtime,
        &songs_table,
        &albums_view,
        &playlists_table,
        &sidebar,
        visible_summary_refresh.clone(),
    );
    sidebar.set_selection_changed(sidebar_selection_changed_callback(
        &runtime,
        &playlists_table,
        visible_summary_refresh.clone(),
    ));
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
    install_preferences_action(
        app,
        &window,
        command_controller.clone(),
        scan_requested.clone(),
    );

    let main_content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main_content.set_hexpand(true);
    main_content.set_vexpand(true);
    let mode_bar = build_mode_bar(
        &window,
        &sidebar_widget,
        &content_stack,
        command_controller,
        scan_requested,
        visible_summary_refresh,
    );
    let albums_view_for_reveal = albums_view.clone();
    let show_albums_view = mode_bar.show_albums.clone();
    let show_album_action: ShowAlbumAction = Rc::new(move |track_id| {
        show_albums_view();
        albums_view_for_reveal.reveal_album_for_track(track_id);
    });
    show_album_holder.replace(Some(show_album_action));
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

    window
}

fn library_changed_callback(
    runtime: &SharedRuntime,
    songs_table: &TrackTable,
    albums_view: &AlbumsView,
    playlists_table: &TrackTable,
    sidebar: &PlaylistSidebar,
    visible_summary_refresh: ViewModeChangedCallback,
) -> LibraryChangedCallback {
    let runtime = runtime.clone();
    let songs_table = songs_table.clone();
    let albums_view = albums_view.clone();
    let playlists_table = playlists_table.clone();
    let sidebar = sidebar.clone();

    Rc::new(move || {
        let rows = runtime_library_table_rows(&runtime.borrow());
        songs_table.replace_rows(rows);
        albums_view.replace_tracks(runtime.borrow().library_tracks().to_vec());
        sidebar.refresh();
        let playlist_rows = playlist_table_rows_for(&runtime.borrow(), sidebar.current_selection());
        playlists_table.replace_rows(playlist_rows);
        visible_summary_refresh();
    })
}

fn visible_summary_refresh_callback(
    runtime: &SharedRuntime,
    content_stack: &gtk::Stack,
    sidebar: &PlaylistSidebar,
    status_bar: &StatusBar,
) -> ViewModeChangedCallback {
    let runtime = runtime.clone();
    let content_stack = content_stack.clone();
    let sidebar = sidebar.clone();
    let status_bar = status_bar.clone();

    Rc::new(move || {
        let rows = visible_view_rows(
            &runtime.borrow(),
            &content_stack,
            sidebar.current_selection(),
        );
        status_bar.update_summary(&rows);
    })
}

fn visible_view_rows(
    runtime: &ApplicationRuntime,
    content_stack: &gtk::Stack,
    sidebar_selection: Option<SidebarSelection>,
) -> Vec<TrackTableRow> {
    if content_stack.visible_child_name().as_deref() == Some(PLAYLISTS_VIEW) {
        playlist_table_rows_for(runtime, sidebar_selection)
    } else {
        runtime_library_table_rows(runtime)
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
        SidebarContextAction::NewPlaylist => {
            let (existing_ids, existing_names): (HashSet<PlaylistId>, Vec<String>) = {
                let runtime = runtime.borrow();
                let ids = runtime.playlists().iter().map(|p| p.id).collect();
                let names = runtime
                    .playlists()
                    .iter()
                    .map(|playlist| playlist.name.clone())
                    .collect();
                (ids, names)
            };
            let name = unique_default_name(existing_names, NEW_PLAYLIST_DEFAULT_NAME);
            if command_controller.dispatch_succeeded(ApplicationCommand::CreatePlaylist {
                name,
                parent_folder_id: None,
            }) {
                let new_id = runtime
                    .borrow()
                    .playlists()
                    .iter()
                    .map(|playlist| playlist.id)
                    .find(|id| !existing_ids.contains(id));
                if let Some(id) = new_id {
                    sidebar.arm_pending_rename(PlaylistItem::Playlist(id));
                }
                sidebar.refresh();
            }
        }
        SidebarContextAction::NewPlaylistFolder => {
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
        SidebarContextAction::NewSmartPlaylist => {
            let existing_names: Vec<String> = runtime
                .borrow()
                .smart_playlists()
                .iter()
                .map(|smart| smart.name.clone())
                .collect();
            let name = unique_default_name(existing_names, NEW_SMART_PLAYLIST_DEFAULT_NAME);
            let sidebar_for_created = sidebar.clone();
            open_smart_playlist_editor(
                &parent,
                command_controller.clone(),
                Rc::new(move || sidebar_for_created.refresh()),
                SmartPlaylistEditorMode::Create { name },
            );
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
) -> Option<(Option<xtunes_app_runtime::PlaylistFolderId>, u32)> {
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
) -> super::sidebar::SidebarSelectionChangedCallback {
    let runtime = runtime.clone();
    let playlists_table = playlists_table.clone();

    Rc::new(move |selection| {
        let rows = playlist_table_rows_for(&runtime.borrow(), selection);
        playlists_table.replace_rows(rows);
        visible_summary_refresh();
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

fn playlist_table_rows_for(
    runtime: &ApplicationRuntime,
    selection: Option<SidebarSelection>,
) -> Vec<TrackTableRow> {
    match selection {
        Some(SidebarSelection::Library) => runtime_library_table_rows(runtime),
        Some(SidebarSelection::Item(PlaylistItem::Playlist(playlist_id))) => {
            let Some(playlist) = runtime
                .playlists()
                .iter()
                .find(|playlist| playlist.id == playlist_id)
            else {
                return Vec::new();
            };
            let library_root = runtime.settings().library_path();
            let tracks_by_id: HashMap<TrackId, &Track> = runtime
                .library_tracks()
                .iter()
                .map(|track| (track.id, track))
                .collect();
            playlist_entries_in_order(playlist)
                .filter_map(|track_id| tracks_by_id.get(&track_id).copied())
                .map(|track| TrackTableRow::from_track(track, library_root))
                .collect()
        }
        Some(SidebarSelection::Item(PlaylistItem::SmartPlaylist(smart_playlist_id))) => {
            let library_root = runtime.settings().library_path();
            runtime
                .smart_playlist_matching_tracks(smart_playlist_id, SystemTime::now())
                .into_iter()
                .map(|track| TrackTableRow::from_track(track, library_root))
                .collect()
        }
        _ => Vec::new(),
    }
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
