// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Window-level keyboard shortcuts: Space toggles or starts playback and
//! Ctrl+L reveals the playing track in the active view, both suppressed while
//! a text field has focus.

use super::*;

pub(super) struct KeyboardShortcutContext {
    pub(super) toggle_or_start_playback: Rc<dyn Fn()>,
    pub(super) runtime: SharedRuntime,
    pub(super) songs_table: TrackTable,
    pub(super) playlists_table: TrackTable,
    pub(super) albums_view: AlbumsView,
    pub(super) content_stack: gtk::Stack,
    pub(super) sidebar: PlaylistSidebar,
}

pub(super) fn install_keyboard_shortcuts(
    window: &gtk::ApplicationWindow,
    context: KeyboardShortcutContext,
) {
    let KeyboardShortcutContext {
        toggle_or_start_playback,
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
            // Same surface as the top-bar Play button: toggle when a track
            // is loaded, cold-start the visible view otherwise (issue #60).
            toggle_or_start_playback();
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
