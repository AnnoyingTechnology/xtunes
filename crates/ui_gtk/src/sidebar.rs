// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use gtk::prelude::*;

use super::{SIDEBAR_DEFAULT_WIDTH, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH};

pub(crate) fn build_sidebar() -> gtk::Box {
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

pub(crate) fn build_content_area(sidebar: &gtk::Box, main_content: &gtk::Box) -> gtk::Paned {
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
