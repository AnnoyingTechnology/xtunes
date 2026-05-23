// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{
    cell::Cell,
    fs,
    path::Path,
    rc::Rc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gtk::prelude::*;
use gtk::{gdk, glib};
use xtunes_app_runtime::{
    ApplicationCommand, FieldChange, MetadataChange, Rating, Track, TrackId, TrackMetadata,
};

use super::{LibraryChangedHolder, SharedRuntime, command_controller::SharedCommandController};

const DIALOG_WIDTH: i32 = 540;
const DIALOG_HEIGHT: i32 = 700;
const COVER_THUMB_SIZE: i32 = 64;
const ARTWORK_PREVIEW_SIZE: i32 = 320;
const NUMBER_ENTRY_WIDTH_CHARS: i32 = 5;
const PAIR_ENTRY_WIDTH_CHARS: i32 = 4;
const READONLY_VALUE_MAX_WIDTH_CHARS: i32 = 60;

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
        .default_height(DIALOG_HEIGHT)
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
    let header = build_header(&track, &artwork_bytes);
    outer.append(&header);

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(120);
    stack.set_hexpand(true);
    stack.set_margin_top(12);

    let details = DetailsPage::new(&initial_metadata, initial_rating, initial_play_count);
    stack.add_titled(&details.widget, Some("details"), "Details");

    let artwork = build_artwork_page(&artwork_bytes);
    stack.add_titled(&artwork, Some("artwork"), "Artwork");

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

        let mut any_succeeded = false;
        if change != MetadataChange::default()
            && command_controller
                .dispatch_succeeded(ApplicationCommand::UpdateMetadata { track_id, change })
        {
            any_succeeded = true;
        }
        if new_rating != initial_rating
            && command_controller.dispatch_succeeded(ApplicationCommand::SetRating {
                track_id,
                rating: new_rating,
            })
        {
            any_succeeded = true;
        }
        if reset_clicked
            && command_controller
                .dispatch_succeeded(ApplicationCommand::ResetPlayCount { track_id })
        {
            any_succeeded = true;
        }
        if any_succeeded && let Some(callback) = library_changed_holder.borrow().as_ref() {
            callback();
        }
        window_for_ok.close();
    });

    window.present();
}

fn build_header(track: &Track, artwork_bytes: &Option<Vec<u8>>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("track-info-header");

    let cover_frame = gtk::Frame::new(None);
    cover_frame.add_css_class("track-info-cover");
    cover_frame.set_size_request(COVER_THUMB_SIZE, COVER_THUMB_SIZE);
    if let Some(texture) = artwork_texture(artwork_bytes) {
        let image = gtk::Image::from_paintable(Some(&texture));
        image.set_pixel_size(COVER_THUMB_SIZE);
        cover_frame.set_child(Some(&image));
    } else {
        let placeholder = gtk::Image::from_icon_name("image-missing-symbolic");
        placeholder.set_pixel_size(COVER_THUMB_SIZE / 2);
        cover_frame.set_child(Some(&placeholder));
    }
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
    row
}

#[derive(Clone)]
struct DetailsPage {
    widget: gtk::Box,
    title: gtk::Entry,
    artist: gtk::Entry,
    album: gtk::Entry,
    album_artist: gtk::Entry,
    composer: gtk::Entry,
    grouping: gtk::Entry,
    genre: gtk::Entry,
    year: gtk::Entry,
    track_number: gtk::Entry,
    track_total: gtk::Entry,
    disc_number: gtk::Entry,
    disc_total: gtk::Entry,
    compilation: gtk::CheckButton,
    bpm: gtk::Entry,
    key: gtk::Entry,
    comments: gtk::TextView,
    rating: Rc<Cell<u8>>,
    play_count_reset: Rc<Cell<bool>>,
}

impl DetailsPage {
    fn new(initial: &TrackMetadata, initial_rating: Rating, initial_play_count: u64) -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 8);
        widget.add_css_class("track-info-details");
        widget.set_margin_top(10);

        let grid = gtk::Grid::new();
        grid.set_row_spacing(6);
        grid.set_column_spacing(10);
        grid.set_hexpand(true);

        let mut row: i32 = 0;
        let title = text_entry(initial.title.as_deref());
        attach_field(&grid, row, "Title", &title);
        row += 1;
        let artist = text_entry(initial.artist.as_deref());
        attach_field(&grid, row, "Artist", &artist);
        row += 1;
        let album = text_entry(initial.album.as_deref());
        attach_field(&grid, row, "Album", &album);
        row += 1;
        let album_artist = text_entry(initial.album_artist.as_deref());
        attach_field(&grid, row, "Album artist", &album_artist);
        row += 1;
        let composer = text_entry(initial.composer.as_deref());
        attach_field(&grid, row, "Composer", &composer);
        row += 1;
        let grouping = text_entry(initial.grouping.as_deref());
        attach_field(&grid, row, "Grouping", &grouping);
        row += 1;
        let genre = text_entry(initial.genre.as_deref());
        attach_field(&grid, row, "Genre", &genre);
        row += 1;

        let year = number_entry(initial.year, NUMBER_ENTRY_WIDTH_CHARS);
        attach_field(&grid, row, "Year", &year);
        row += 1;

        let track_number = number_entry(initial.track_number, PAIR_ENTRY_WIDTH_CHARS);
        let track_total = number_entry(initial.track_total, PAIR_ENTRY_WIDTH_CHARS);
        attach_paired_field(&grid, row, "Track", &track_number, &track_total);
        row += 1;

        let disc_number = number_entry(initial.disc_number, PAIR_ENTRY_WIDTH_CHARS);
        let disc_total = number_entry(initial.disc_total, PAIR_ENTRY_WIDTH_CHARS);
        attach_paired_field(&grid, row, "Disc", &disc_number, &disc_total);
        row += 1;

        let compilation =
            gtk::CheckButton::with_label("Album is a compilation of songs by various artists");
        compilation.set_active(initial.compilation.unwrap_or(false));
        grid.attach(&compilation, 1, row, 3, 1);
        row += 1;

        let bpm = number_entry(initial.bpm, NUMBER_ENTRY_WIDTH_CHARS);
        attach_field(&grid, row, "BPM", &bpm);
        row += 1;

        let key = text_entry(initial.key.as_deref());
        key.set_width_chars(8);
        key.set_hexpand(false);
        attach_field(&grid, row, "Key", &key);
        row += 1;

        let rating_widget = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        let rating = Rc::new(Cell::new(initial_rating.stars()));
        for star in 1u8..=5 {
            let button = rating_star_button(star);
            sync_rating_button(&button, star, rating.get());
            let rating_state = rating.clone();
            let parent_box = rating_widget.clone();
            button.connect_clicked(move |_| {
                let next = next_rating(rating_state.get(), star);
                rating_state.set(next);
                refresh_rating_buttons(&parent_box, next);
            });
            rating_widget.append(&button);
        }
        attach_field(&grid, row, "Rating", &rating_widget);
        row += 1;

        let play_count_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let play_count_label = gtk::Label::new(None);
        play_count_label.set_xalign(0.0);
        play_count_label.set_hexpand(true);
        play_count_label.set_text(&initial_play_count.to_string());
        // The label snapshots the count at dialog open. The Reset button stages
        // a reset that is dispatched on OK; the displayed number doesn't change
        // until the dialog is reopened.
        let play_count_reset = Rc::new(Cell::new(false));
        let reset_button = gtk::Button::with_label("Reset");
        let reset_state = play_count_reset.clone();
        let label_for_reset = play_count_label.clone();
        reset_button.connect_clicked(move |button| {
            reset_state.set(true);
            button.set_sensitive(false);
            label_for_reset.set_text("0 (will reset on OK)");
        });
        play_count_row.append(&play_count_label);
        play_count_row.append(&reset_button);
        attach_field(&grid, row, "Play count", &play_count_row);
        row += 1;

        let comments = gtk::TextView::new();
        comments.set_wrap_mode(gtk::WrapMode::WordChar);
        comments.set_accepts_tab(false);
        if let Some(text) = initial.comments.as_deref() {
            comments.buffer().set_text(text);
        }
        let comments_scroll = gtk::ScrolledWindow::new();
        // Without these caps the ScrolledWindow propagates the TextView's
        // natural height, which scales with content length (audio
        // fingerprints in comments can be thousands of pixels tall and
        // overflow the dialog). Lock the row to a predictable height and
        // let the SW handle scrolling internally.
        comments_scroll.set_min_content_height(70);
        comments_scroll.set_max_content_height(90);
        comments_scroll.set_propagate_natural_height(false);
        comments_scroll.set_hexpand(true);
        comments_scroll.set_vexpand(false);
        comments_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        comments_scroll.set_child(Some(&comments));
        let comments_label = gtk::Label::new(Some("Comments"));
        comments_label.set_xalign(0.0);
        comments_label.set_valign(gtk::Align::Start);
        comments_label.add_css_class("track-info-field-label");
        grid.attach(&comments_label, 0, row, 1, 1);
        grid.attach(&comments_scroll, 1, row, 3, 1);

        widget.append(&grid);

        Self {
            widget,
            title,
            artist,
            album,
            album_artist,
            composer,
            grouping,
            genre,
            year,
            track_number,
            track_total,
            disc_number,
            disc_total,
            compilation,
            bpm,
            key,
            comments,
            rating,
            play_count_reset,
        }
    }

    fn metadata_diff(&self, initial: &TrackMetadata) -> MetadataChange {
        MetadataChange {
            title: text_diff(initial.title.as_deref(), &self.title.text()),
            artist: text_diff(initial.artist.as_deref(), &self.artist.text()),
            album: text_diff(initial.album.as_deref(), &self.album.text()),
            album_artist: text_diff(initial.album_artist.as_deref(), &self.album_artist.text()),
            composer: text_diff(initial.composer.as_deref(), &self.composer.text()),
            grouping: text_diff(initial.grouping.as_deref(), &self.grouping.text()),
            genre: text_diff(initial.genre.as_deref(), &self.genre.text()),
            track_number: number_diff(initial.track_number, &self.track_number.text()),
            track_total: number_diff(initial.track_total, &self.track_total.text()),
            disc_number: number_diff(initial.disc_number, &self.disc_number.text()),
            disc_total: number_diff(initial.disc_total, &self.disc_total.text()),
            year: signed_number_diff(initial.year, &self.year.text()),
            compilation: bool_diff(initial.compilation, self.compilation.is_active()),
            bpm: number_diff(initial.bpm, &self.bpm.text()),
            key: text_diff(initial.key.as_deref(), &self.key.text()),
            comments: text_diff(
                initial.comments.as_deref(),
                &self.text_view_text(&self.comments),
            ),
            lyrics: FieldChange::Unchanged,
        }
    }

    fn current_rating(&self) -> Rating {
        Rating::new(self.rating.get()).unwrap_or_else(Rating::unrated)
    }

    fn play_count_reset_requested(&self) -> bool {
        self.play_count_reset.get()
    }

    fn text_view_text(&self, view: &gtk::TextView) -> glib::GString {
        let buffer = view.buffer();
        buffer.text(&buffer.start_iter(), &buffer.end_iter(), false)
    }
}

#[derive(Clone)]
struct LyricsPage {
    widget: gtk::ScrolledWindow,
    view: gtk::TextView,
}

impl LyricsPage {
    fn new(initial: &TrackMetadata) -> Self {
        let view = gtk::TextView::new();
        view.add_css_class("track-info-lyrics-view");
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_accepts_tab(false);
        view.set_top_margin(8);
        view.set_bottom_margin(8);
        view.set_left_margin(8);
        view.set_right_margin(8);
        if let Some(text) = initial.lyrics.as_deref() {
            view.buffer().set_text(text);
        }

        let widget = gtk::ScrolledWindow::new();
        widget.add_css_class("track-info-lyrics");
        widget.set_margin_top(10);
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.set_min_content_height(280);
        widget.set_propagate_natural_height(false);
        widget.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        widget.set_child(Some(&view));

        Self { widget, view }
    }

    fn lyrics_diff(&self, initial: &TrackMetadata) -> FieldChange<String> {
        let buffer = self.view.buffer();
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        text_diff_preserve_newlines(initial.lyrics.as_deref(), &text)
    }
}

fn build_artwork_page(artwork_bytes: &Option<Vec<u8>>) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    page.add_css_class("track-info-artwork");
    page.set_margin_top(10);
    page.set_halign(gtk::Align::Center);

    let frame = gtk::Frame::new(None);
    frame.add_css_class("track-info-artwork-frame");
    frame.set_size_request(ARTWORK_PREVIEW_SIZE, ARTWORK_PREVIEW_SIZE);

    if let Some(texture) = artwork_texture(artwork_bytes) {
        let image = gtk::Image::from_paintable(Some(&texture));
        image.set_pixel_size(ARTWORK_PREVIEW_SIZE);
        frame.set_child(Some(&image));
    } else {
        let placeholder = gtk::Image::from_icon_name("image-missing-symbolic");
        placeholder.set_pixel_size(ARTWORK_PREVIEW_SIZE / 3);
        frame.set_child(Some(&placeholder));
    }
    page.append(&frame);

    let note = gtk::Label::new(Some(if artwork_bytes.is_some() {
        "Artwork is embedded in the audio file."
    } else {
        "This track has no embedded artwork."
    }));
    note.add_css_class("dim-label");
    note.set_margin_top(4);
    page.append(&note);

    page
}

fn build_file_page(track: &Track, absolute_path: Option<&Path>) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 6);
    page.add_css_class("track-info-file");
    page.set_margin_top(10);

    let grid = gtk::Grid::new();
    grid.set_row_spacing(4);
    grid.set_column_spacing(12);
    grid.set_hexpand(true);

    let file_metadata = absolute_path.and_then(|path| fs::metadata(path).ok());

    let mut row: i32 = 0;
    attach_readonly_field(&grid, row, "Kind", &format_kind(absolute_path));
    row += 1;
    attach_readonly_field(
        &grid,
        row,
        "Duration",
        &format_duration_label(track.metadata.duration),
    );
    row += 1;
    attach_readonly_field(
        &grid,
        row,
        "Size",
        &format_size_label(file_metadata.as_ref().map(|metadata| metadata.len())),
    );
    row += 1;
    attach_readonly_field(
        &grid,
        row,
        "Bit rate",
        &format_optional_unit(track.metadata.bitrate_kbps, "kbps"),
    );
    row += 1;
    attach_readonly_field(
        &grid,
        row,
        "Sample rate",
        &format_sample_rate(track.metadata.sample_rate_hz),
    );
    row += 1;
    attach_readonly_field(
        &grid,
        row,
        "Channels",
        &format_channels(track.metadata.channels),
    );
    row += 1;
    attach_readonly_field(&grid, row, "Format", &format_kind(absolute_path));
    row += 1;
    attach_readonly_field(
        &grid,
        row,
        "Date modified",
        &format_modified(
            file_metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok()),
        ),
    );
    row += 1;
    let location_text = absolute_path
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| String::from("\u{2014}"));
    attach_readonly_field(&grid, row, "Location", &location_text);

    page.append(&grid);
    page
}

fn attach_field(grid: &gtk::Grid, row: i32, label_text: &str, field: &impl IsA<gtk::Widget>) {
    let label = gtk::Label::new(Some(label_text));
    label.set_xalign(1.0);
    label.set_valign(gtk::Align::Center);
    label.add_css_class("track-info-field-label");
    grid.attach(&label, 0, row, 1, 1);
    grid.attach(field, 1, row, 3, 1);
}

fn attach_paired_field(
    grid: &gtk::Grid,
    row: i32,
    label_text: &str,
    first: &gtk::Entry,
    second: &gtk::Entry,
) {
    let label = gtk::Label::new(Some(label_text));
    label.set_xalign(1.0);
    label.set_valign(gtk::Align::Center);
    label.add_css_class("track-info-field-label");
    grid.attach(&label, 0, row, 1, 1);
    grid.attach(first, 1, row, 1, 1);

    let separator = gtk::Label::new(Some("of"));
    separator.add_css_class("dim-label");
    grid.attach(&separator, 2, row, 1, 1);
    grid.attach(second, 3, row, 1, 1);
}

fn attach_readonly_field(grid: &gtk::Grid, row: i32, label_text: &str, value: &str) {
    let label = gtk::Label::new(Some(label_text));
    label.set_xalign(1.0);
    label.set_valign(gtk::Align::Start);
    label.add_css_class("track-info-field-label");
    grid.attach(&label, 0, row, 1, 1);

    let value_label = gtk::Label::new(Some(value));
    value_label.set_xalign(0.0);
    value_label.set_valign(gtk::Align::Start);
    value_label.set_wrap(true);
    value_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    value_label.set_max_width_chars(READONLY_VALUE_MAX_WIDTH_CHARS);
    value_label.set_selectable(true);
    value_label.set_hexpand(true);
    grid.attach(&value_label, 1, row, 1, 1);
}

fn text_entry(initial: Option<&str>) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_hexpand(true);
    if let Some(text) = initial {
        entry.set_text(text);
    }
    entry
}

fn number_entry<T: ToString>(initial: Option<T>, width_chars: i32) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_width_chars(width_chars);
    entry.set_max_width_chars(width_chars);
    entry.set_hexpand(false);
    entry.set_halign(gtk::Align::Start);
    if let Some(value) = initial {
        entry.set_text(&value.to_string());
    }
    entry
}

fn rating_star_button(star: u8) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("track-info-rating-star");
    button.set_focusable(false);
    button.set_tooltip_text(Some(&format!(
        "{star} star{}",
        if star == 1 { "" } else { "s" }
    )));
    button
}

fn sync_rating_button(button: &gtk::Button, star: u8, rating: u8) {
    button.set_label(if star <= rating {
        "\u{2605}"
    } else {
        "\u{2606}"
    });
}

fn refresh_rating_buttons(parent: &gtk::Box, rating: u8) {
    let mut child = parent.first_child();
    let mut star: u8 = 1;
    while let Some(widget) = child {
        if let Some(button) = widget.downcast_ref::<gtk::Button>() {
            sync_rating_button(button, star, rating);
        }
        child = widget.next_sibling();
        star += 1;
    }
}

fn next_rating(current: u8, clicked: u8) -> u8 {
    if current == clicked { 0 } else { clicked }
}

fn text_diff(initial: Option<&str>, current: &str) -> FieldChange<String> {
    let trimmed_current = current.trim();
    match (initial, trimmed_current) {
        (Some(value), candidate) if value == candidate => FieldChange::Unchanged,
        (None, "") => FieldChange::Unchanged,
        (_, "") => FieldChange::Clear,
        (_, candidate) => FieldChange::Set(candidate.to_owned()),
    }
}

/// Like `text_diff` but preserves internal whitespace (newlines, indentation).
/// Used for free-form prose fields like lyrics where formatting matters.
/// Empty/whitespace-only buffers still clear the tag.
fn text_diff_preserve_newlines(initial: Option<&str>, current: &str) -> FieldChange<String> {
    let trimmed = current.trim();
    match (initial, trimmed) {
        (None, "") => FieldChange::Unchanged,
        (_, "") => FieldChange::Clear,
        (Some(value), _) if value == current => FieldChange::Unchanged,
        _ => FieldChange::Set(current.to_owned()),
    }
}

fn number_diff(initial: Option<u32>, current: &str) -> FieldChange<u32> {
    let trimmed = current.trim();
    if trimmed.is_empty() {
        return if initial.is_some() {
            FieldChange::Clear
        } else {
            FieldChange::Unchanged
        };
    }
    let Ok(parsed) = trimmed.parse::<u32>() else {
        return FieldChange::Unchanged;
    };
    if Some(parsed) == initial {
        FieldChange::Unchanged
    } else {
        FieldChange::Set(parsed)
    }
}

fn signed_number_diff(initial: Option<i32>, current: &str) -> FieldChange<i32> {
    let trimmed = current.trim();
    if trimmed.is_empty() {
        return if initial.is_some() {
            FieldChange::Clear
        } else {
            FieldChange::Unchanged
        };
    }
    let Ok(parsed) = trimmed.parse::<i32>() else {
        return FieldChange::Unchanged;
    };
    if Some(parsed) == initial {
        FieldChange::Unchanged
    } else {
        FieldChange::Set(parsed)
    }
}

fn bool_diff(initial: Option<bool>, current: bool) -> FieldChange<bool> {
    if initial.unwrap_or(false) == current {
        FieldChange::Unchanged
    } else {
        FieldChange::Set(current)
    }
}

fn artwork_texture(artwork_bytes: &Option<Vec<u8>>) -> Option<gdk::Texture> {
    let bytes = artwork_bytes.as_ref()?;
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_read(std::io::Cursor::new(bytes.clone())).ok()?;
    Some(gdk::Texture::for_pixbuf(&pixbuf))
}

fn format_kind(path: Option<&Path>) -> String {
    let Some(path) = path else {
        return String::from("\u{2014}");
    };
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("mp3") => "MPEG audio file".to_owned(),
        Some("flac") => "FLAC audio file".to_owned(),
        Some("ogg" | "oga") => "Ogg Vorbis audio file".to_owned(),
        Some("opus") => "Opus audio file".to_owned(),
        Some("m4a" | "m4b" | "mp4") => "MPEG-4 audio file".to_owned(),
        Some(other) => format!("{} audio file", other.to_ascii_uppercase()),
        None => "Audio file".to_owned(),
    }
}

fn format_duration_label(duration: Option<Duration>) -> String {
    let Some(duration) = duration else {
        return String::from("\u{2014}");
    };
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn format_size_label(size: Option<u64>) -> String {
    let Some(size) = size else {
        return String::from("\u{2014}");
    };
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let size_f = size as f64;
    if size_f >= GIB {
        format!("{:.2} GB", size_f / GIB)
    } else if size_f >= MIB {
        format!("{:.2} MB", size_f / MIB)
    } else if size_f >= KIB {
        format!("{:.2} KB", size_f / KIB)
    } else {
        format!("{size} B")
    }
}

fn format_optional_unit<T: std::fmt::Display>(value: Option<T>, unit: &str) -> String {
    match value {
        Some(value) => format!("{value} {unit}"),
        None => String::from("\u{2014}"),
    }
}

fn format_sample_rate(sample_rate_hz: Option<u32>) -> String {
    match sample_rate_hz {
        Some(hz) => {
            let khz = f64::from(hz) / 1000.0;
            if (khz.fract() - 0.0).abs() < f64::EPSILON {
                format!("{khz:.0} kHz")
            } else {
                format!("{khz:.1} kHz")
            }
        }
        None => String::from("\u{2014}"),
    }
}

fn format_channels(channels: Option<u8>) -> String {
    match channels {
        Some(1) => "Mono".to_owned(),
        Some(2) => "Stereo".to_owned(),
        Some(count) => format!("{count} channels"),
        None => String::from("\u{2014}"),
    }
}

fn format_modified(modified: Option<SystemTime>) -> String {
    let Some(modified) = modified else {
        return String::from("\u{2014}");
    };
    let Ok(since_epoch) = modified.duration_since(UNIX_EPOCH) else {
        return String::from("\u{2014}");
    };
    format_unix_seconds(since_epoch.as_secs())
}

fn format_unix_seconds(unix_seconds: u64) -> String {
    let (year, month, day, hour, minute) = unix_seconds_to_ymdhm(unix_seconds);
    format!("{day:02}/{month:02}/{year:04} {hour:02}:{minute:02}")
}

fn unix_seconds_to_ymdhm(unix_seconds: u64) -> (i32, u32, u32, u32, u32) {
    // Convert from days-since-1970 to a civil date using Howard Hinnant's
    // chrono algorithm. This avoids pulling in a date crate purely for a
    // file-modified timestamp.
    let total_minutes = unix_seconds / 60;
    let minute = (total_minutes % 60) as u32;
    let total_hours = total_minutes / 60;
    let hour = (total_hours % 24) as u32;
    let days_since_epoch = (total_hours / 24) as i64;

    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = (y + i64::from(month <= 2)) as i32;

    (year, month, day, hour, minute)
}

#[cfg(test)]
mod tests {
    use super::{
        bool_diff, format_channels, format_duration_label, format_kind, format_sample_rate,
        format_size_label, next_rating, number_diff, signed_number_diff, text_diff,
        text_diff_preserve_newlines, unix_seconds_to_ymdhm,
    };
    use std::path::Path;
    use std::time::Duration;
    use xtunes_app_runtime::FieldChange;

    #[test]
    fn text_diff_preserves_unchanged_value() {
        assert_eq!(text_diff(Some("hello"), "hello"), FieldChange::Unchanged);
        assert_eq!(text_diff(None, ""), FieldChange::Unchanged);
    }

    #[test]
    fn text_diff_clears_when_field_emptied() {
        assert_eq!(text_diff(Some("hello"), ""), FieldChange::Clear);
        assert_eq!(text_diff(Some("hello"), "   "), FieldChange::Clear);
    }

    #[test]
    fn text_diff_sets_when_value_changes() {
        assert_eq!(text_diff(Some("a"), "b"), FieldChange::Set("b".to_owned()));
        assert_eq!(text_diff(None, "b"), FieldChange::Set("b".to_owned()));
    }

    #[test]
    fn number_diff_handles_empty_and_invalid_inputs() {
        assert_eq!(number_diff(None, ""), FieldChange::Unchanged);
        assert_eq!(number_diff(Some(3), ""), FieldChange::Clear);
        assert_eq!(number_diff(Some(3), "abc"), FieldChange::Unchanged);
        assert_eq!(number_diff(Some(3), "3"), FieldChange::Unchanged);
        assert_eq!(number_diff(Some(3), "4"), FieldChange::Set(4));
    }

    #[test]
    fn signed_number_diff_handles_negatives() {
        assert_eq!(signed_number_diff(Some(2000), "-1"), FieldChange::Set(-1));
        assert_eq!(signed_number_diff(None, "1998"), FieldChange::Set(1998));
        assert_eq!(signed_number_diff(Some(1998), ""), FieldChange::Clear);
    }

    #[test]
    fn bool_diff_treats_none_as_false_baseline() {
        assert_eq!(bool_diff(None, false), FieldChange::Unchanged);
        assert_eq!(bool_diff(None, true), FieldChange::Set(true));
        assert_eq!(bool_diff(Some(true), false), FieldChange::Set(false));
        assert_eq!(bool_diff(Some(true), true), FieldChange::Unchanged);
    }

    #[test]
    fn next_rating_toggles_off_when_clicking_current_star() {
        assert_eq!(next_rating(3, 3), 0);
        assert_eq!(next_rating(3, 4), 4);
        assert_eq!(next_rating(0, 2), 2);
    }

    #[test]
    fn format_kind_recognises_common_extensions() {
        assert_eq!(format_kind(Some(Path::new("song.mp3"))), "MPEG audio file");
        assert_eq!(format_kind(Some(Path::new("song.flac"))), "FLAC audio file");
        assert_eq!(format_kind(None), "\u{2014}");
    }

    #[test]
    fn format_size_label_uses_binary_prefixes() {
        assert_eq!(format_size_label(Some(512)), "512 B");
        assert_eq!(format_size_label(Some(2_048)), "2.00 KB");
        assert_eq!(format_size_label(Some(5_242_880)), "5.00 MB");
        assert_eq!(format_size_label(None), "\u{2014}");
    }

    #[test]
    fn format_duration_label_includes_hours_when_needed() {
        assert_eq!(
            format_duration_label(Some(Duration::from_secs(245))),
            "4:05"
        );
        assert_eq!(
            format_duration_label(Some(Duration::from_secs(3_904))),
            "1:05:04"
        );
        assert_eq!(format_duration_label(None), "\u{2014}");
    }

    #[test]
    fn format_sample_rate_handles_common_rates() {
        assert_eq!(format_sample_rate(Some(44_100)), "44.1 kHz");
        assert_eq!(format_sample_rate(Some(48_000)), "48 kHz");
        assert_eq!(format_sample_rate(None), "\u{2014}");
    }

    #[test]
    fn format_channels_uses_human_labels() {
        assert_eq!(format_channels(Some(1)), "Mono");
        assert_eq!(format_channels(Some(2)), "Stereo");
        assert_eq!(format_channels(Some(6)), "6 channels");
        assert_eq!(format_channels(None), "\u{2014}");
    }

    #[test]
    fn text_diff_preserve_newlines_keeps_internal_whitespace() {
        assert_eq!(
            text_diff_preserve_newlines(None, "line one\n\nline two"),
            FieldChange::Set("line one\n\nline two".to_owned())
        );
        assert_eq!(
            text_diff_preserve_newlines(Some("a\nb"), "a\nb"),
            FieldChange::Unchanged
        );
        assert_eq!(
            text_diff_preserve_newlines(Some("a\nb"), "  \n  \n"),
            FieldChange::Clear
        );
        assert_eq!(
            text_diff_preserve_newlines(None, ""),
            FieldChange::Unchanged
        );
    }

    #[test]
    fn unix_seconds_to_ymdhm_matches_known_dates() {
        assert_eq!(unix_seconds_to_ymdhm(0), (1970, 1, 1, 0, 0));
        // 2000-01-01 00:00:00 UTC
        assert_eq!(unix_seconds_to_ymdhm(946_684_800), (2000, 1, 1, 0, 0));
        // 2020-01-01 12:45:00 UTC
        assert_eq!(unix_seconds_to_ymdhm(1_577_882_700), (2020, 1, 1, 12, 45));
    }
}
