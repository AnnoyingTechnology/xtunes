// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{cell::Cell, rc::Rc};

use gtk::prelude::*;
use sustain_app_runtime::UiViewMode;

use super::{
    ALBUMS_VIEW, MODE_BAR_HEIGHT, MODE_BUTTON_HEIGHT, PLAYLISTS_VIEW, SONGS_VIEW,
    command_controller::SharedCommandController,
    library_consolidation::LibraryConsolidationRequestedCallback,
    library_scan::LibraryScanRequestedCallback,
    preferences::settings_button,
    sidebar::{PlaylistSidebar, SidebarSelection},
};

pub(crate) type ViewModeChangedCallback = Rc<dyn Fn()>;
pub(crate) type ShowAlbumsViewCallback = Rc<dyn Fn()>;
pub(crate) type ShowSongsViewCallback = Rc<dyn Fn()>;
pub(crate) type ShowPlaylistsViewCallback = Rc<dyn Fn()>;

pub(crate) struct ModeBar {
    pub(crate) widget: gtk::CenterBox,
    pub(crate) show_albums: ShowAlbumsViewCallback,
    pub(crate) show_songs: ShowSongsViewCallback,
    pub(crate) show_playlists: ShowPlaylistsViewCallback,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_mode_bar(
    window: &gtk::ApplicationWindow,
    sidebar: &PlaylistSidebar,
    content_stack: &gtk::Stack,
    current_view_mode: &Rc<Cell<UiViewMode>>,
    command_controller: SharedCommandController,
    scan_requested: LibraryScanRequestedCallback,
    consolidation_requested: LibraryConsolidationRequestedCallback,
    initial_mode: UiViewMode,
    on_view_mode_changed: ViewModeChangedCallback,
) -> ModeBar {
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

    connect_mode_button(
        &songs,
        UiViewMode::Songs,
        sidebar,
        content_stack,
        current_view_mode,
        on_view_mode_changed.clone(),
    );
    connect_mode_button(
        &albums,
        UiViewMode::Albums,
        sidebar,
        content_stack,
        current_view_mode,
        on_view_mode_changed.clone(),
    );
    connect_mode_button(
        &playlists,
        UiViewMode::Playlists,
        sidebar,
        content_stack,
        current_view_mode,
        on_view_mode_changed,
    );

    match initial_mode {
        UiViewMode::Songs => songs.set_active(true),
        UiViewMode::Albums => albums.set_active(true),
        UiViewMode::Playlists => playlists.set_active(true),
    }

    let mode_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    mode_buttons.set_valign(gtk::Align::Center);
    mode_buttons.append(&songs);
    mode_buttons.append(&albums);
    mode_buttons.append(&playlists);
    mode_bar.set_center_widget(Some(&mode_buttons));

    let settings = settings_button(
        window,
        command_controller,
        scan_requested,
        consolidation_requested,
    );
    mode_bar.set_end_widget(Some(&settings));

    let albums_for_callback = albums.clone();
    let show_albums: ShowAlbumsViewCallback = Rc::new(move || {
        albums_for_callback.set_active(true);
    });

    let songs_for_callback = songs.clone();
    let show_songs: ShowSongsViewCallback = Rc::new(move || {
        songs_for_callback.set_active(true);
    });

    let playlists_for_callback = playlists.clone();
    let show_playlists: ShowPlaylistsViewCallback = Rc::new(move || {
        playlists_for_callback.set_active(true);
    });

    ModeBar {
        widget: mode_bar,
        show_albums,
        show_songs,
        show_playlists,
    }
}

fn connect_mode_button(
    button: &gtk::ToggleButton,
    mode: UiViewMode,
    sidebar: &PlaylistSidebar,
    content_stack: &gtk::Stack,
    current_view_mode: &Rc<Cell<UiViewMode>>,
    on_view_mode_changed: ViewModeChangedCallback,
) {
    let sidebar = sidebar.clone();
    let content_stack = content_stack.clone();
    let current_view_mode = current_view_mode.clone();
    button.connect_toggled(move |button| {
        if button.is_active() {
            apply_view_mode(mode, &sidebar, &content_stack, &current_view_mode);
            on_view_mode_changed();
        }
    });
}

/// Apply a top-bar mode change.
///
/// The "mode" the user picks (Songs / Albums / Playlists) is one input;
/// the sidebar's current selection is the other. Together they decide
/// which page of `content_stack` is visible. In Playlists mode the choice
/// is split:
///
/// - selection is the Library entry (UI label: "Music") → `SONGS_VIEW`.
///   The Library row is conceptually the whole-library access point, and
///   we display it via the already-populated songs table rather than
///   rebuilding the same rows into the playlist-detail table. This keeps
///   the click cost at zero and avoids showing a playlist-style header
///   above what is really the songs list.
/// - selection is a playlist / smart playlist → `PLAYLISTS_VIEW`, which
///   carries the playlist-detail header and the per-selection table.
pub(crate) fn apply_view_mode(
    mode: UiViewMode,
    sidebar: &PlaylistSidebar,
    content_stack: &gtk::Stack,
    current_view_mode: &Rc<Cell<UiViewMode>>,
) {
    current_view_mode.set(mode);
    match mode {
        UiViewMode::Songs => {
            sidebar.widget().set_visible(false);
            content_stack.set_visible_child_name(SONGS_VIEW);
        }
        UiViewMode::Albums => {
            sidebar.widget().set_visible(false);
            content_stack.set_visible_child_name(ALBUMS_VIEW);
        }
        UiViewMode::Playlists => {
            sidebar.widget().set_visible(true);
            content_stack.set_visible_child_name(stack_child_for_playlists_mode(
                sidebar.current_selection(),
            ));
        }
    }
}

/// Picks the content-stack child appropriate for the Playlists mode given
/// the sidebar's current selection. Exposed so the sidebar-selection
/// callback can resync the stack when the user clicks between Music and
/// a playlist *without* changing the top-bar mode.
pub(crate) fn stack_child_for_playlists_mode(selection: Option<SidebarSelection>) -> &'static str {
    match selection {
        Some(SidebarSelection::Item(_)) => PLAYLISTS_VIEW,
        Some(SidebarSelection::Library) | None => SONGS_VIEW,
    }
}

fn set_mode_button_height(control: &gtk::ToggleButton) {
    control.set_height_request(MODE_BUTTON_HEIGHT);
    control.set_valign(gtk::Align::Center);
    control.add_css_class("mode-button");
}
