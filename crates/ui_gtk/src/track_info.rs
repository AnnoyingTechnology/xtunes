// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use gtk::prelude::*;
use gtk::{gdk, glib};
use sustain_app_runtime::{ApplicationCommand, MetadataChange, Track, TrackId};

use super::{LibraryChangedHolder, SharedRuntime, command_controller::SharedCommandController};

mod artwork;
mod details;
mod diff;
mod file_page;
mod form;
mod format;
mod lyrics;

use artwork::{ArtworkPage, update_artwork_frame};
use details::DetailsPage;
use file_page::build_file_page;
use lyrics::LyricsPage;

const DIALOG_WIDTH: i32 = 540;
const COVER_THUMB_SIZE: i32 = 64;
const ARTWORK_PREVIEW_SIZE: i32 = 320;
const NUMBER_ENTRY_WIDTH_CHARS: i32 = 5;
const PAIR_ENTRY_WIDTH_CHARS: i32 = 4;

pub(crate) fn open_track_info_dialog(
    parent: &gtk::Window,
    runtime: &SharedRuntime,
    command_controller: &SharedCommandController,
    library_changed_holder: &LibraryChangedHolder,
    track_id: TrackId,
) {
    let (track, absolute_path) = {
        let runtime_borrow = runtime.borrow();
        let Some(track) = runtime_borrow
            .library_tracks()
            .iter()
            .find(|track| track.id == track_id)
            .cloned()
        else {
            return;
        };
        let absolute_path = runtime_borrow.absolute_track_path(&track);
        (track, absolute_path)
    };

    let initial_metadata = track.metadata.clone();
    let initial_rating = track.rating;
    let initial_play_count = track.statistics.play_count;

    let window = gtk::Window::builder()
        .title("Get Info")
        .transient_for(parent)
        .modal(true)
        .resizable(false)
        .default_width(DIALOG_WIDTH)
        .build();
    window.add_css_class("track-info-window");

    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.set_margin_top(16);
    outer.set_margin_bottom(16);
    outer.set_margin_start(18);
    outer.set_margin_end(18);

    let artwork_bytes = absolute_path
        .as_deref()
        .and_then(|path| runtime.borrow().read_artwork(path));
    let header = build_header(&track, artwork_bytes.as_deref());
    outer.append(&header.widget);

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(120);
    stack.set_hexpand(true);
    stack.set_margin_top(12);

    let details = DetailsPage::new(&initial_metadata, initial_rating, initial_play_count);
    stack.add_titled(&details.widget, Some("details"), "Details");

    let artwork = ArtworkPage::new(
        parent,
        command_controller,
        library_changed_holder,
        track_id,
        header.cover_frame.clone(),
        artwork_bytes.as_deref(),
    );
    stack.add_titled(&artwork.widget, Some("artwork"), "Artwork");

    let lyrics = LyricsPage::new(&initial_metadata);
    stack.add_titled(&lyrics.widget, Some("lyrics"), "Lyrics");

    let file_page = build_file_page(&track, absolute_path.as_deref());
    stack.add_titled(&file_page, Some("file"), "File");

    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.set_halign(gtk::Align::Center);
    outer.append(&switcher);
    outer.append(&stack);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(14);
    let cancel = gtk::Button::with_label("Cancel");
    let ok = gtk::Button::with_label("OK");
    ok.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&ok);
    outer.append(&buttons);

    window.set_child(Some(&outer));

    let window_for_cancel = window.clone();
    cancel.connect_clicked(move |_| {
        window_for_cancel.close();
    });

    let key_controller = gtk::EventControllerKey::new();
    let window_for_escape = window.clone();
    key_controller.connect_key_pressed(move |_controller, key, _keycode, _state| {
        if key == gdk::Key::Escape {
            window_for_escape.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key_controller);

    let command_controller = command_controller.clone();
    let library_changed_holder = library_changed_holder.clone();
    let window_for_ok = window.clone();
    let details_for_ok = details.clone();
    let lyrics_for_ok = lyrics.clone();
    ok.connect_clicked(move |_| {
        let mut change = details_for_ok.metadata_diff(&initial_metadata);
        change.lyrics = lyrics_for_ok.lyrics_diff(&initial_metadata);
        let new_rating = details_for_ok.current_rating();
        let reset_clicked = details_for_ok.play_count_reset_requested();

        let mut attempted = false;
        let mut any_succeeded = false;
        let mut any_failed = false;
        if change != MetadataChange::default() {
            attempted = true;
            match command_controller.dispatch(ApplicationCommand::UpdateMetadata {
                track_id,
                change: Box::new(change),
            }) {
                Ok(()) => any_succeeded = true,
                Err(_) => any_failed = true,
            }
        }
        if new_rating != initial_rating {
            attempted = true;
            match command_controller.dispatch(ApplicationCommand::SetRating {
                track_id,
                rating: new_rating,
            }) {
                Ok(()) => any_succeeded = true,
                Err(_) => any_failed = true,
            }
        }
        if reset_clicked {
            attempted = true;
            match command_controller.dispatch(ApplicationCommand::ResetPlayCount { track_id }) {
                Ok(()) => any_succeeded = true,
                Err(_) => any_failed = true,
            }
        }
        if any_succeeded && let Some(callback) = library_changed_holder.borrow().as_ref() {
            callback();
        }
        if !attempted || !any_failed {
            window_for_ok.close();
        }
    });

    window.present();
}

struct Header {
    widget: gtk::Box,
    cover_frame: gtk::Frame,
}

fn build_header(track: &Track, artwork_bytes: Option<&[u8]>) -> Header {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("track-info-header");

    let cover_frame = gtk::Frame::new(None);
    cover_frame.add_css_class("track-info-cover");
    cover_frame.set_size_request(COVER_THUMB_SIZE, COVER_THUMB_SIZE);
    update_artwork_frame(&cover_frame, artwork_bytes, COVER_THUMB_SIZE);
    row.append(&cover_frame);

    let info = gtk::Box::new(gtk::Orientation::Vertical, 2);
    info.set_valign(gtk::Align::Center);
    info.set_hexpand(true);

    let title = gtk::Label::new(Some(track.metadata.title.as_deref().unwrap_or("Untitled")));
    title.add_css_class("track-info-title");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    info.append(&title);

    let artist = gtk::Label::new(Some(
        track.metadata.artist.as_deref().unwrap_or("Unknown Artist"),
    ));
    artist.add_css_class("track-info-subtitle");
    artist.set_xalign(0.0);
    artist.set_ellipsize(gtk::pango::EllipsizeMode::End);
    info.append(&artist);

    if let Some(album) = track.metadata.album.as_deref() {
        let album_label = gtk::Label::new(Some(album));
        album_label.add_css_class("track-info-subtitle");
        album_label.set_xalign(0.0);
        album_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        info.append(&album_label);
    }

    row.append(&info);
    Header {
        widget: row,
        cover_frame,
    }
}
