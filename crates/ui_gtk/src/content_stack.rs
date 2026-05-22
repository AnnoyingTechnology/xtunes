// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use gtk::prelude::*;

use super::{
    ALBUMS_VIEW, PLAYLISTS_VIEW, SONGS_VIEW,
    track_context::TrackRowContextMenu,
    track_table::{TrackActivatedCallback, build_track_table},
};

pub(crate) fn build_content_stack(
    songs_view: gtk::ScrolledWindow,
    albums_view: gtk::ScrolledWindow,
    track_activated: TrackActivatedCallback,
    context_menu: TrackRowContextMenu,
) -> gtk::Stack {
    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);

    let playlists_view =
        build_track_table(Vec::new(), Some(track_activated), Some(context_menu), None).widget();

    stack.add_named(&songs_view, Some(SONGS_VIEW));
    stack.add_named(&albums_view, Some(ALBUMS_VIEW));
    stack.add_named(&playlists_view, Some(PLAYLISTS_VIEW));
    stack.set_visible_child_name(SONGS_VIEW);

    stack
}
