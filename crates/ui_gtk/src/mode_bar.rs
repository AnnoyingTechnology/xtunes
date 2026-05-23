// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::rc::Rc;

use gtk::prelude::*;

use super::{
    ALBUMS_VIEW, MODE_BAR_HEIGHT, MODE_BUTTON_HEIGHT, PLAYLISTS_VIEW, SONGS_VIEW,
    command_controller::SharedCommandController, library_scan::LibraryScanRequestedCallback,
    preferences::settings_button,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MainViewMode {
    Songs,
    Albums,
    Playlists,
}

pub(crate) type ViewModeChangedCallback = Rc<dyn Fn()>;

pub(crate) fn build_mode_bar(
    window: &gtk::ApplicationWindow,
    sidebar: &gtk::Box,
    content_stack: &gtk::Stack,
    command_controller: SharedCommandController,
    scan_requested: LibraryScanRequestedCallback,
    on_view_mode_changed: ViewModeChangedCallback,
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

    connect_mode_button(
        &songs,
        MainViewMode::Songs,
        sidebar,
        content_stack,
        on_view_mode_changed.clone(),
    );
    connect_mode_button(
        &albums,
        MainViewMode::Albums,
        sidebar,
        content_stack,
        on_view_mode_changed.clone(),
    );
    connect_mode_button(
        &playlists,
        MainViewMode::Playlists,
        sidebar,
        content_stack,
        on_view_mode_changed,
    );

    let mode_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    mode_buttons.set_valign(gtk::Align::Center);
    mode_buttons.append(&songs);
    mode_buttons.append(&albums);
    mode_buttons.append(&playlists);
    mode_bar.set_center_widget(Some(&mode_buttons));

    let settings = settings_button(window, command_controller, scan_requested);
    mode_bar.set_end_widget(Some(&settings));
    mode_bar
}

fn connect_mode_button(
    button: &gtk::ToggleButton,
    mode: MainViewMode,
    sidebar: &gtk::Box,
    content_stack: &gtk::Stack,
    on_view_mode_changed: ViewModeChangedCallback,
) {
    let sidebar = sidebar.clone();
    let content_stack = content_stack.clone();
    button.connect_toggled(move |button| {
        if button.is_active() {
            apply_view_mode(mode, &sidebar, &content_stack);
            on_view_mode_changed();
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

fn set_mode_button_height(control: &gtk::ToggleButton) {
    control.set_height_request(MODE_BUTTON_HEIGHT);
    control.set_valign(gtk::Align::Center);
    control.add_css_class("mode-button");
}
