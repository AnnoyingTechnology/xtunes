// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use gtk::prelude::*;

use super::{ALBUMS_VIEW, PLAYLISTS_VIEW, SONGS_VIEW};

pub(crate) fn build_content_stack(
    songs_view: gtk::ScrolledWindow,
    albums_view: gtk::ScrolledWindow,
    playlists_view: gtk::ScrolledWindow,
) -> gtk::Stack {
    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);

    stack.add_named(&songs_view, Some(SONGS_VIEW));
    stack.add_named(&albums_view, Some(ALBUMS_VIEW));
    stack.add_named(&playlists_view, Some(PLAYLISTS_VIEW));
    stack.set_visible_child_name(SONGS_VIEW);

    stack
}
